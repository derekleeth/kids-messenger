//! Pairing two of a family's hubs and keeping them in step over a real connection, pictures and videos included
//! (docs/multi_hub_plan.md, phases 3 and 4).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use kit_protocol::*;
use kitchat_hub::hub_files::{Keep, PIECE};
use kitchat_hub::{Hub, app, open_in_memory, peers, random, replica};
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
    let dir = temp_dir();
    let hub = open_in_memory(dir.clone()).await.unwrap();
    serve(hub, dir).await
}

/// A hub that keeps other hubs' files as `keep` says.
async fn new_hub_keeping(keep: Keep) -> TestHub {
    let dir = temp_dir();
    let options =
        kitchat_hub::Options { data_dir: dir.clone(), previews: false, limits: Default::default(), keep_files: keep };
    serve(kitchat_hub::open(options).await.unwrap(), dir).await
}

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kitchat-peers-test-{}", ulid()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn serve(hub: Arc<Hub>, dir: std::path::PathBuf) -> TestHub {
    let _ = rustls::crypto::ring::default_provider().install_default();
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
    paired_with(new_hub().await).await
}

/// Home with a family, and `travel` joined to it.
async fn paired_with(travel: TestHub) -> (TestHub, TestHub, String) {
    let home = new_hub().await;
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

/// A picture the hub takes (a JPEG it can't decode is kept as it is), `len` bytes long.
fn picture(len: usize, seed: u8) -> Vec<u8> {
    let mut bytes = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02];
    bytes.extend((0..len - 8).map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed) % 0xF0));
    bytes.extend([0xFF, 0xD9]);
    bytes
}

fn request(method: &str, path: &str, auth: String, body: Vec<u8>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .extension(ConnectInfo("127.0.0.1:1".parse::<SocketAddr>().unwrap()))
        .header(header::AUTHORIZATION, auth)
        .body(Body::from(body))
        .unwrap()
}

async fn upload(app: &Router, token: &str, bytes: Vec<u8>) -> String {
    let res = app.clone().oneshot(request("POST", "/media", format!("Bearer {token}"), bytes)).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    body["mediaId"].as_str().unwrap().to_string()
}

async fn download(app: &Router, token: &str, id: &str) -> (StatusCode, Vec<u8>) {
    let req = request("GET", &format!("/media/{id}"), format!("Bearer {token}"), Vec::new());
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    (status, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

async fn has_file(h: &TestHub, id: &str) -> bool {
    let have: bool =
        sqlx::query_scalar("SELECT have_file FROM media WHERE id = ?").bind(id).fetch_one(&h.hub.pool).await.unwrap();
    have && h.hub.media_dir().join(id).exists()
}

#[tokio::test]
async fn pictures_and_videos_go_both_ways_in_pieces() {
    let (home, travel, parent) = paired().await;
    let big = picture(2 * PIECE as usize + 12345, 1);
    let small = picture(3000, 2);
    let from_home = upload(&home.app, &parent, big.clone()).await;
    let from_camp = upload(&travel.app, &parent, small.clone()).await;

    assert!(peers::sync_once(&travel.hub).await.unwrap() > 0);
    assert!(has_file(&travel, &from_home).await);
    assert!(has_file(&home, &from_camp).await);
    assert_eq!(download(&travel.app, &parent, &from_home).await, (StatusCode::OK, big));
    assert_eq!(download(&home.app, &parent, &from_camp).await, (StatusCode::OK, small));
    // Nothing more to move, and nothing sent back.
    assert_eq!(peers::sync_once(&travel.hub).await.unwrap(), 0);
}

#[tokio::test]
async fn a_file_cut_off_partway_carries_on_where_it_stopped() {
    let (home, travel, parent) = paired().await;
    let big = picture(2 * PIECE as usize + 99, 3);
    let id = upload(&home.app, &parent, big.clone()).await;
    // Its record arrives, and the first piece of the file, before the connection drops.
    let changes = replica::changes_since(&home.hub, 0, 1000).await.unwrap();
    replica::apply(&travel.hub, changes).await.unwrap();
    let part = travel.hub.uploads_dir().join(format!("hub-{id}"));
    std::fs::write(&part, &big[..PIECE as usize]).unwrap();

    peers::sync_once(&travel.hub).await.unwrap();
    assert!(has_file(&travel, &id).await);
    assert!(!part.exists());
    assert_eq!(download(&travel.app, &parent, &id).await, (StatusCode::OK, big));
}

#[tokio::test]
async fn a_picture_whose_file_is_on_the_other_hub_is_for_later() {
    let (home, travel, parent) = paired().await;
    let id = upload(&travel.app, &parent, picture(3000, 4)).await;
    // Home hears of it, but it can't call the travel hub for the file.
    let changes = replica::changes_since(&travel.hub, 0, 1000).await.unwrap();
    replica::apply(&home.hub, changes).await.unwrap();
    let (status, _) = download(&home.app, &parent, &id).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn a_travel_hub_copies_recent_files_that_fit_and_fetches_the_rest_when_opened() {
    let keep = Keep { days: 30, max_bytes: 25_000 };
    let (home, travel, parent) = paired_with(new_hub_keeping(keep).await).await;
    let (old, older, newer) = (picture(10_000, 5), picture(10_000, 6), picture(10_000, 7));
    let old_id = upload(&home.app, &parent, old.clone()).await;
    let older_id = upload(&home.app, &parent, older.clone()).await;
    let newer_id = upload(&home.app, &parent, newer.clone()).await;
    let days_ago = |days: i64| kitchat_hub::now_ms() - days * 24 * 60 * 60 * 1000;
    for (id, at) in [(&old_id, days_ago(90)), (&older_id, days_ago(2)), (&newer_id, days_ago(1))] {
        sqlx::query("UPDATE media SET created_at = ? WHERE id = ?")
            .bind(at)
            .bind(id)
            .execute(&home.hub.pool)
            .await
            .unwrap();
    }

    peers::sync_once(&travel.hub).await.unwrap();
    // The two recent ones fit; the one from three months ago isn't copied.
    assert!(has_file(&travel, &newer_id).await);
    assert!(has_file(&travel, &older_id).await);
    assert!(!has_file(&travel, &old_id).await);

    // Opening it fetches it from home, and the copy kept longest makes room.
    assert_eq!(download(&travel.app, &parent, &old_id).await, (StatusCode::OK, old));
    assert!(has_file(&travel, &old_id).await);
    assert!(!has_file(&travel, &older_id).await);
    assert!(has_file(&travel, &newer_id).await);

    // With home out of reach, a picture that isn't here is for later.
    home.hub.stopping.send_replace(true);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (status, _) = download(&travel.app, &parent, &older_id).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn only_the_paired_hub_gets_or_sends_files() {
    let (home, travel, parent) = paired().await;
    let id = upload(&home.app, &parent, picture(3000, 8)).await;
    let path = format!("/hubs/files/{id}");
    let refused = [format!("Hub {}:nope", travel.hub.hub_id), format!("Bearer {parent}"), String::new()];
    for auth in refused {
        let res = home.app.clone().oneshot(request("GET", &path, auth.clone(), Vec::new())).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED, "{auth}");
        let res = home.app.clone().oneshot(request("PUT", &format!("{path}?offset=0"), auth, vec![1])).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    }
    let auth = format!("Hub {}:{}", travel.hub.hub_id, secret(&travel).await);
    let res = home.app.clone().oneshot(request("GET", &path, auth, Vec::new())).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

async fn checksum(h: &TestHub, id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT sha256 FROM media WHERE id = ?").bind(id).fetch_one(&h.hub.pool).await.unwrap()
}

/// Spoils a file on disk without changing its size, as a bad disk or connection might.
fn damage(h: &TestHub, id: &str) {
    let path = h.hub.media_dir().join(id);
    let mut bytes = std::fs::read(&path).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x55;
    std::fs::write(path, bytes).unwrap();
}

#[tokio::test]
async fn each_picture_carries_the_checksum_of_its_file() {
    let (home, travel, parent) = paired().await;
    let bytes = picture(2 * PIECE as usize + 7, 9);
    let id = upload(&home.app, &parent, bytes.clone()).await;
    let expected = kitchat_hub::hub_files::sha256(&bytes);
    assert_eq!(checksum(&home, &id).await.as_deref(), Some(expected.as_str()));
    peers::sync_once(&travel.hub).await.unwrap();
    assert_eq!(checksum(&travel, &id).await.as_deref(), Some(expected.as_str()));
    assert!(has_file(&travel, &id).await);
}

#[tokio::test]
async fn a_file_that_arrives_damaged_is_not_kept_or_fetched_again_and_again() {
    let (home, travel, parent) = paired().await;
    let id = upload(&home.app, &parent, picture(PIECE as usize + 500, 10)).await;
    damage(&home, &id);

    peers::sync_once(&travel.hub).await.unwrap();
    assert!(!has_file(&travel, &id).await);
    assert!(!travel.hub.uploads_dir().join(format!("hub-{id}")).exists(), "the damaged copy is thrown away");
    // The background rounds leave it for a while.
    assert_eq!(peers::sync_once(&travel.hub).await.unwrap(), 0);
    // Opening it still tries, and still won't show a damaged file.
    let (status, _) = download(&travel.app, &parent, &id).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn home_refuses_a_damaged_file_from_the_travel_hub() {
    let (home, travel, parent) = paired().await;
    let id = upload(&travel.app, &parent, picture(3000, 11)).await;
    damage(&travel, &id);

    peers::sync_once(&travel.hub).await.unwrap();
    assert!(!has_file(&home, &id).await);
    assert!(!home.hub.uploads_dir().join(format!("hub-{id}")).exists());
    assert_eq!(peers::sync_once(&travel.hub).await.unwrap(), 0, "not sent again straight away");
}

#[tokio::test]
async fn pictures_from_before_checksums_get_theirs_and_pass_them_on() {
    let (home, travel, parent) = paired().await;
    let bytes = picture(3000, 12);
    let id = upload(&home.app, &parent, bytes.clone()).await;
    sqlx::query("UPDATE media SET sha256 = NULL").execute(&home.hub.pool).await.unwrap();
    let expected = kitchat_hub::hub_files::sha256(&bytes);
    peers::sync_once(&travel.hub).await.unwrap();
    // Without one, a copy is still kept (the size is all there is to check), and notes the checksum of what came.
    assert!(has_file(&travel, &id).await);
    assert_eq!(checksum(&travel, &id).await.as_deref(), Some(expected.as_str()));

    assert_eq!(kitchat_hub::hub_files::fill_in_checksums(&home.hub).await.unwrap(), 1);
    assert_eq!(kitchat_hub::hub_files::fill_in_checksums(&home.hub).await.unwrap(), 0);
    assert_eq!(checksum(&home, &id).await.as_deref(), Some(expected.as_str()));
    peers::sync_once(&travel.hub).await.unwrap();
    // And nothing more after that, either way.
    assert_eq!(peers::sync_once(&travel.hub).await.unwrap(), 0);
}
