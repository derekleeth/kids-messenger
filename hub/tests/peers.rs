//! Pairing two of a family's hubs and keeping them in step over a real connection (docs/multi_hub_plan.md, phase 3).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use kit_protocol::*;
use kitchat_hub::{Hub, app, open_in_memory, peers, random};
use serde_json::{Value, json};
use tower::ServiceExt;

/// A folder deleted when the test ends.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TestHub {
    hub: Arc<Hub>,
    app: Router,
    /// Where it listens, on this computer.
    url: String,
    _dir: TempDir,
}

impl Drop for TestHub {
    fn drop(&mut self) {
        self.hub.stopping.send_replace(true);
    }
}

/// A hub listening on a port of its own, as a hub runs.
async fn new_hub() -> TestHub {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dir = std::env::temp_dir().join(format!("kitchat-peers-test-{}", ulid()));
    std::fs::create_dir_all(&dir).unwrap();
    let hub = open_in_memory(dir.clone()).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let service = app(hub.clone()).into_make_service_with_connect_info::<SocketAddr>();
    let mut stopping = hub.stopping.subscribe();
    tokio::spawn(async move {
        let _ = axum::serve(listener, service)
            .with_graceful_shutdown(async move {
                let _ = stopping.wait_for(|s| *s).await;
            })
            .await;
    });
    TestHub { app: app(hub.clone()), hub, url, _dir: TempDir(dir) }
}

fn ulid() -> String {
    ulid::Ulid::new().to_string()
}

async fn call_from(
    app: &Router,
    peer: &str,
    method: &str,
    path: &str,
    auth: Option<String>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let peer: SocketAddr = peer.parse().unwrap();
    let mut req = Request::builder().method(method).uri(path).extension(ConnectInfo(peer));
    if let Some(auth) = auth {
        req = req.header(header::AUTHORIZATION, auth);
    }
    let req = match body {
        Some(body) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let value = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap_or(Value::Null) };
    (status, value)
}

async fn call(app: &Router, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    call_from(app, "127.0.0.1:1", method, path, token.map(|t| format!("Bearer {t}")), body).await
}

async fn ok(app: &Router, method: &str, path: &str, token: &str, body: Value) -> Value {
    let (status, body) = call(app, method, path, Some(token), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{method} {path}: {body}");
    body
}

/// Sets a family up on `h`. Returns the parent's token.
async fn set_up(h: &TestHub, family: &str) -> String {
    let code = h.hub.setup_code.lock().await.clone().unwrap();
    let (status, body) = call(
        &h.app,
        "POST",
        "/setup",
        None,
        Some(json!({ "setupCode": code, "familyName": family, "name": "Dad", "deviceName": "Desktop" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value::<Enrolled>(body).unwrap().token
}

async fn hub_code(h: &TestHub, parent: &str) -> String {
    ok(&h.app, "POST", "/hubs/codes", parent, Value::Null).await["code"].as_str().unwrap().to_string()
}

async fn sync(app: &Router, token: &str) -> SyncResponse {
    let (status, body) = call(app, "GET", "/sync?since=0", Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

async fn send(app: &Router, token: &str, chat: &str, text: &str) -> String {
    let id = ulid();
    let body = json!({ "id": id, "body": { "kind": "text", "text": text } });
    ok(app, "POST", &format!("/chats/{chat}/messages"), token, body).await;
    id
}

async fn has_message(app: &Router, token: &str, id: &str) -> bool {
    sync(app, token).await.messages.iter().any(|m| m.id == id)
}

/// Home with a family, and a new travel hub joined to it. Returns (home, travel, the parent's token).
async fn paired() -> (TestHub, TestHub, String) {
    let (home, travel) = (new_hub().await, new_hub().await);
    let parent = set_up(&home, "The Lynx Family").await;
    let code = hub_code(&home, &parent).await;
    peers::join(&travel.hub, &home.url, &code, "Beryl").await.unwrap();
    (home, travel, parent)
}

/// The secret the travel hub keeps for home.
async fn secret(travel: &TestHub) -> String {
    sqlx::query_scalar("SELECT secret FROM peers").fetch_one(&travel.hub.pool).await.unwrap()
}

#[tokio::test]
async fn a_joined_hub_copies_the_family_and_keeps_in_step_both_ways() {
    let (home, travel, parent) = paired().await;
    assert!(travel.hub.setup_code.lock().await.is_none(), "the travel hub is set up by joining");
    // The parent's device signs in at the travel hub with the token it already has.
    let family_chat = sync(&travel.app, &parent).await.chats[0].id.clone();

    let at_camp = send(&travel.app, &parent, &family_chat, "Tent's up").await;
    let at_home = send(&home.app, &parent, &family_chat, "Fed the cat").await;
    assert!(peers::sync_once(&travel.hub).await.unwrap() > 0);
    for h in [&home, &travel] {
        assert!(has_message(&h.app, &parent, &at_camp).await);
        assert!(has_message(&h.app, &parent, &at_home).await);
    }
    // Nothing new: nothing changes, on either hub.
    assert_eq!(peers::sync_once(&travel.hub).await.unwrap(), 0);

    // Each hub lists the other.
    let listed = ok(&home.app, "GET", "/hubs", &parent, Value::Null).await;
    assert_eq!(listed[0]["hubId"], travel.hub.hub_id);
    assert_eq!(listed[0]["name"], "Beryl");
    let listed = ok(&travel.app, "GET", "/hubs", &parent, Value::Null).await;
    assert_eq!(listed[0]["hubId"], home.hub.hub_id);
    assert_eq!(listed[0]["url"], home.url);
}

#[tokio::test]
async fn hub_codes_work_once_and_only_for_parents() {
    let (home, _travel, parent) = paired().await;
    let other = new_hub().await;
    // The code the travel hub used.
    let used: String = sqlx::query_scalar("SELECT code FROM hub_codes").fetch_one(&home.hub.pool).await.unwrap();
    assert!(peers::join(&other.hub, &home.url, &used, "Other").await.is_err());
    assert!(peers::join(&other.hub, &home.url, "WRONG-CODE", "Other").await.is_err());
    assert!(other.hub.setup_code.lock().await.is_some(), "nothing was copied");

    let kid = ok(
        &home.app,
        "POST",
        "/members",
        &parent,
        json!({ "name": "Sam", "role": "kid", "color": "mint", "avatar": "🦊" }),
    )
    .await;
    let code =
        ok(&home.app, "POST", &format!("/members/{}/codes", kid["id"].as_str().unwrap()), &parent, Value::Null).await;
    let (_, body) =
        // Not from 127.0.0.1, which used up its tries at open routes on the joins above.
        call_from(&home.app, "10.0.0.5:1", "POST", "/enroll", None, Some(json!({ "code": code["code"], "deviceName": "Tablet" })))
            .await;
    let kid = serde_json::from_value::<Enrolled>(body).unwrap().token;
    let (status, _) = call(&home.app, "POST", "/hubs/codes", Some(&kid), None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn only_the_paired_hub_gets_or_sends_changes() {
    let (home, travel, parent) = paired().await;
    let travel_id = travel.hub.hub_id.clone();
    let secret = secret(&travel).await;

    let (status, _) =
        call_from(&home.app, "127.0.0.1:1", "GET", "/hubs/changes", Some(format!("Hub {travel_id}:nope")), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) =
        call_from(&home.app, "127.0.0.1:1", "GET", "/hubs/changes", Some(format!("Bearer {secret}")), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let auth = format!("Hub {travel_id}:{secret}");
    let (status, changes) = call_from(&home.app, "127.0.0.1:1", "GET", "/hubs/changes", Some(auth.clone()), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(changes["hubId"], home.hub.hub_id);

    // Changes that say they're from another hub.
    let mut theirs = kitchat_hub::replica::changes_since(&travel.hub, 0, 10).await.unwrap();
    theirs.hub_id = ulid();
    let body = serde_json::to_value(&theirs).unwrap();
    let (status, _) =
        call_from(&home.app, "127.0.0.1:1", "POST", "/hubs/changes", Some(auth.clone()), Some(body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Unpaired: the secret stops working.
    ok(&home.app, "DELETE", &format!("/hubs/{travel_id}"), &parent, Value::Null).await;
    let (status, _) = call_from(&home.app, "127.0.0.1:1", "GET", "/hubs/changes", Some(auth), None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(peers::sync_once(&travel.hub).await.is_err());
}

#[tokio::test]
async fn a_hub_asking_for_changes_hears_as_soon_as_there_are_some() {
    let (home, travel, parent) = paired().await;
    let auth = format!("Hub {}:{}", travel.hub.hub_id, secret(&travel).await);
    let since = db_rev(&home.hub).await;
    let app = home.app.clone();
    let asked = Instant::now();
    let waiting = tokio::spawn(async move {
        call_from(&app, "127.0.0.1:1", "GET", &format!("/hubs/changes?since={since}&wait=true"), Some(auth), None).await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiting.is_finished(), "it waits while there's nothing new");
    let family_chat = sync(&home.app, &parent).await.chats[0].id.clone();
    let id = send(&home.app, &parent, &family_chat, "Dinner!").await;
    let (status, changes) = waiting.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert!(asked.elapsed() < Duration::from_secs(5), "answered as soon as the message came");
    assert_eq!(changes["messages"][0]["id"], id);
}

async fn db_rev(hub: &Hub) -> i64 {
    sqlx::query_scalar("SELECT rev FROM family").fetch_one(&hub.pool).await.unwrap()
}

#[tokio::test]
async fn a_hub_with_another_family_cant_join() {
    let (home, travel) = (new_hub().await, new_hub().await);
    let parent = set_up(&home, "The Lynx Family").await;
    set_up(&travel, "Someone Else").await;
    let code = hub_code(&home, &parent).await;
    let err = peers::join(&travel.hub, &home.url, &code, "Beryl").await.unwrap_err();
    assert!(err.to_string().contains("different family"), "{err}");
    // The code wasn't spent on it.
    let used: i64 = sqlx::query_scalar("SELECT used FROM hub_codes").fetch_one(&home.hub.pool).await.unwrap();
    assert_eq!(used, 0);
}

#[tokio::test]
async fn home_proves_itself_only_to_hubs_nearby() {
    let (home, travel, _) = paired().await;
    let travel_id = travel.hub.hub_id.clone();
    let nonce = random::token();
    let body = json!({ "hubId": travel_id, "nonce": nonce });

    let (status, proof) =
        call_from(&home.app, "192.168.8.20:5000", "POST", "/hubs/prove", None, Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK);
    let expected = proof::hub_proof(&random::hash_token(&secret(&travel).await), &travel_id, &nonce);
    assert_eq!(proof["proof"], expected);

    let (status, _) = call_from(&home.app, "8.8.8.8:5000", "POST", "/hubs/prove", None, Some(body)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_running_hub_keeps_in_step_by_itself() {
    let (home, travel, parent) = paired().await;
    peers::spawn(travel.hub.clone());
    let family_chat = sync(&home.app, &parent).await.chats[0].id.clone();
    let from_home = send(&home.app, &parent, &family_chat, "Back by 6?").await;
    let from_camp = send(&travel.app, &parent, &family_chat, "Yes!").await;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if has_message(&travel.app, &parent, &from_home).await && has_message(&home.app, &parent, &from_camp).await {
            break;
        }
        assert!(Instant::now() < deadline, "the hubs didn't get in step");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
