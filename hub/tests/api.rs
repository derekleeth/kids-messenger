//! The hub's REST API, end to end through the router: setting up, joining, chatting, pictures, notes and removing a
//! device.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt;
use kit_protocol::notes::{Block, BlockKind, Note};
use kit_protocol::proof;
use kit_protocol::*;
use kitchat_hub::limits::Config;
use kitchat_hub::{Hub, app, open_in_memory, open_in_memory_with};
use serde_json::{Value, json};
use tower::ServiceExt;

struct Family {
    hub: Arc<Hub>,
    app: Router,
    parent: String,
    parent_member: Member,
    kid: String,
    kid_member: Member,
    _dir: TempDir,
}

/// A folder deleted when the test ends.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir() -> TempDir {
    let dir = std::env::temp_dir().join(format!("kitchat-hub-test-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

async fn call(app: &Router, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
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

/// A request from `peer` (a socket address, as the server would see it) with extra headers.
async fn call_from(
    app: &Router,
    peer: &str,
    headers: &[(&str, &str)],
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, HeaderMap) {
    let peer: std::net::SocketAddr = peer.parse().unwrap();
    let mut req = Request::builder().method(method).uri(path).extension(ConnectInfo(peer));
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    if let Some(token) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let req = match body {
        Some(body) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    (res.status(), res.headers().clone())
}

async fn raw(app: &Router, method: &str, path: &str, token: Option<&str>, body: Vec<u8>) -> (StatusCode, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let res = app.clone().oneshot(req.body(Body::from(body)).unwrap()).await.unwrap();
    let status = res.status();
    (status, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

fn ulid() -> String {
    ulid::Ulid::new().to_string()
}

/// A hub with a parent (set up with the setup code) and a kid (joined with a code).
async fn family() -> Family {
    let dir = temp_dir();
    let hub = open_in_memory(dir.0.clone()).await.unwrap();
    let app = app(hub.clone());
    let code = hub.setup_code.lock().await.clone().unwrap();
    let (status, body) = call(
        &app,
        "POST",
        "/setup",
        None,
        Some(json!({ "setupCode": code.to_lowercase(), "familyName": "The Lynx Family", "name": "Dad", "deviceName": "Desktop" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let enrolled: Enrolled = serde_json::from_value(body).unwrap();
    assert_eq!(enrolled.member.role, Role::Parent);

    let (status, body) = call(
        &app,
        "POST",
        "/members",
        Some(&enrolled.token),
        Some(json!({ "name": "Sam", "role": "kid", "color": "mint", "avatar": "🦊" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let kid_member: Member = serde_json::from_value(body).unwrap();
    let kid = join(&app, &enrolled.token, &kid_member.id, "Sam's tablet").await;
    Family { hub, app, parent: enrolled.token, parent_member: enrolled.member, kid, kid_member, _dir: dir }
}

async fn join(app: &Router, parent: &str, member_id: &str, device: &str) -> String {
    let (status, body) = call(app, "POST", &format!("/members/{member_id}/codes"), Some(parent), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let code: EnrollCode = serde_json::from_value(body).unwrap();
    let (status, body) =
        call(app, "POST", "/enroll", None, Some(json!({ "code": code.code, "deviceName": device }))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value::<Enrolled>(body).unwrap().token
}

async fn sync(app: &Router, token: &str, since: i64) -> SyncResponse {
    let (status, body) = call(app, "GET", &format!("/sync?since={since}"), Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

#[tokio::test]
async fn setup_happens_once_with_the_right_code() {
    let dir = temp_dir();
    let hub = open_in_memory(dir.0.clone()).await.unwrap();
    let app = app(hub.clone());
    let (_, hello) = call(&app, "GET", "/hello", None, None).await;
    assert_eq!(hello["needsSetup"], true);

    let bad = json!({ "setupCode": "WRONG123", "familyName": "F", "name": "Dad", "deviceName": "PC" });
    assert_eq!(call(&app, "POST", "/setup", None, Some(bad)).await.0, StatusCode::BAD_REQUEST);

    let code = hub.setup_code.lock().await.clone().unwrap();
    let good = json!({ "setupCode": code, "familyName": "F", "name": "Dad", "deviceName": "PC" });
    assert_eq!(call(&app, "POST", "/setup", None, Some(good.clone())).await.0, StatusCode::OK);
    assert_eq!(call(&app, "POST", "/setup", None, Some(good)).await.0, StatusCode::CONFLICT);
    let (_, hello) = call(&app, "GET", "/hello", None, None).await;
    assert_eq!(hello["needsSetup"], false);
    assert_eq!(hello["familyName"], "F");
}

#[tokio::test]
async fn requests_need_a_device_token() {
    let f = family().await;
    assert_eq!(call(&f.app, "GET", "/sync", None, None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&f.app, "GET", "/sync", Some("nope"), None).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn join_codes_work_once_and_only_parents_manage_the_family() {
    let f = family().await;
    let (_, body) = call(&f.app, "POST", &format!("/members/{}/codes", f.kid_member.id), Some(&f.parent), None).await;
    let code = body["code"].as_str().unwrap().to_string();
    let req = json!({ "code": code, "deviceName": "Phone" });
    assert_eq!(call(&f.app, "POST", "/enroll", None, Some(req.clone())).await.0, StatusCode::OK);
    assert_eq!(call(&f.app, "POST", "/enroll", None, Some(req)).await.0, StatusCode::BAD_REQUEST);

    let new_member = json!({ "name": "Stranger", "role": "parent", "color": "sun", "avatar": "x" });
    assert_eq!(call(&f.app, "POST", "/members", Some(&f.kid), Some(new_member)).await.0, StatusCode::FORBIDDEN);
    let path = format!("/members/{}/codes", f.kid_member.id);
    assert_eq!(call(&f.app, "POST", &path, Some(&f.kid), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&f.app, "GET", "/devices", Some(&f.kid), None).await.0, StatusCode::FORBIDDEN);

    // A kid can change their own picture.
    let path = format!("/members/{}", f.kid_member.id);
    let (status, body) = call(&f.app, "PATCH", &path, Some(&f.kid), Some(json!({ "avatar": "🐱" }))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["avatar"], "🐱");
}

#[tokio::test]
async fn too_many_wrong_codes_lock_joining_for_a_while() {
    let f = family().await;
    // From ten different addresses: the hub-wide guard still counts them all.
    for i in 0..10 {
        let req = json!({ "code": "ZZZZZZ", "deviceName": "x" });
        let peer = format!("203.0.113.{i}:4000");
        let status = call_from(&f.app, &peer, &[], "POST", "/enroll", None, Some(req)).await.0;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let req = json!({ "code": "ZZZZZZ", "deviceName": "x" });
    let (status, headers) = call_from(&f.app, "203.0.113.99:4000", &[], "POST", "/enroll", None, Some(req)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(headers.contains_key(header::RETRY_AFTER));
}

#[tokio::test]
async fn one_address_gets_five_wrong_codes() {
    let dir = temp_dir();
    // A higher scale loosens the request rate but not the wrong-code limit, so this tests the latter alone.
    let hub = open_in_memory_with(dir.0.clone(), Config { trust_proxy: false, scale: 10.0 }).await.unwrap();
    let app = app(hub);
    let req = json!({ "code": "ZZZZZZ", "deviceName": "x" });
    for _ in 0..5 {
        let status = call_from(&app, "198.51.100.1:1", &[], "POST", "/enroll", None, Some(req.clone())).await.0;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let status = call_from(&app, "198.51.100.1:1", &[], "POST", "/enroll", None, Some(req.clone())).await.0;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let status = call_from(&app, "198.51.100.2:1", &[], "POST", "/enroll", None, Some(req)).await.0;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// `/enroll` with a body it can't read: refused before any code is checked, so only the request rate counts.
async fn enroll_junk(app: &Router, peer: &str, headers: &[(&str, &str)]) -> (StatusCode, HeaderMap) {
    call_from(app, peer, headers, "POST", "/enroll", None, Some(json!({}))).await
}

#[tokio::test]
async fn joining_is_rate_limited_per_address() {
    let dir = temp_dir();
    let app = app(open_in_memory(dir.0.clone()).await.unwrap());
    for _ in 0..5 {
        assert_eq!(enroll_junk(&app, "198.51.100.1:1", &[]).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let (status, headers) = enroll_junk(&app, "198.51.100.1:1", &[]).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = headers[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
    assert!((1..=6).contains(&wait), "{wait}");
    assert_eq!(enroll_junk(&app, "198.51.100.2:1", &[]).await.0, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn wrong_tokens_shut_an_address_out() {
    let f = family().await;
    let from = async |peer: &str, token: Option<&str>| call_from(&f.app, peer, &[], "GET", "/sync", token, None).await;
    for _ in 0..20 {
        assert_eq!(from("198.51.100.7:1", Some("nope")).await.0, StatusCode::UNAUTHORIZED);
    }
    // Even a good token is refused from that address now, and so are the open paths.
    let kid = f.kid.as_str();
    let (status, headers) = from("198.51.100.7:1", Some(kid)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = headers[header::RETRY_AFTER].to_str().unwrap().parse().unwrap();
    assert!(wait > 14 * 60, "{wait}");
    let status = call_from(&f.app, "198.51.100.7:1", &[], "GET", "/hello", None, None).await.0;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // Missing tokens count too.
    for _ in 0..20 {
        assert_eq!(from("198.51.100.8:1", None).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(from("198.51.100.8:1", Some(kid)).await.0, StatusCode::TOO_MANY_REQUESTS);
    // Other addresses are fine.
    assert_eq!(from("198.51.100.9:1", Some(kid)).await.0, StatusCode::OK);
}

#[tokio::test]
async fn proxy_headers_are_ignored_unless_trusted() {
    // Without --trust-proxy, X-Forwarded-For can't spread one client over many addresses.
    let dir = temp_dir();
    let plain = app(open_in_memory(dir.0.clone()).await.unwrap());
    for i in 0..5 {
        let xff = format!("192.0.2.{i}");
        let status = enroll_junk(&plain, "127.0.0.1:1", &[("x-forwarded-for", &xff)]).await.0;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }
    let status = enroll_junk(&plain, "127.0.0.1:1", &[("x-forwarded-for", "192.0.2.200")]).await.0;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);

    // With it, from a proxy on loopback, each forwarded client gets its own limit...
    let dir2 = temp_dir();
    let trusted = app(open_in_memory_with(dir2.0.clone(), Config { trust_proxy: true, scale: 1.0 }).await.unwrap());
    for i in 0..10 {
        let xff = format!("10.9.9.9, 192.0.2.{i}");
        let status = enroll_junk(&trusted, "127.0.0.1:1", &[("x-forwarded-for", &xff)]).await.0;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{xff}");
    }
    // ...the right-most X-Forwarded-For entry (added by the proxy) is the one that counts, not what the client sent...
    for _ in 0..5 {
        enroll_junk(&trusted, "127.0.0.1:1", &[("x-forwarded-for", "192.0.2.50")]).await;
    }
    let status = enroll_junk(&trusted, "[::1]:1", &[("x-forwarded-for", "1.2.3.4, 192.0.2.50")]).await.0;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // ...X-Real-IP comes first...
    let headers = [("x-real-ip", "192.0.2.51"), ("x-forwarded-for", "192.0.2.50")];
    assert_eq!(enroll_junk(&trusted, "127.0.0.1:1", &headers).await.0, StatusCode::UNPROCESSABLE_ENTITY);
    // ...but a client connecting directly can't use the headers.
    for i in 0..5 {
        let fake = format!("192.0.2.{}", 100 + i);
        enroll_junk(&trusted, "198.51.100.3:1", &[("x-forwarded-for", &fake), ("x-real-ip", &fake)]).await;
    }
    let status = enroll_junk(&trusted, "198.51.100.3:1", &[("x-real-ip", "192.0.2.199")]).await.0;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn every_answer_has_the_security_headers() {
    let f = family().await;
    let check = |headers: &HeaderMap, api: bool| {
        assert_eq!(headers["x-content-type-options"], "nosniff");
        assert_eq!(headers["x-frame-options"], "DENY");
        assert_eq!(headers["referrer-policy"], "no-referrer");
        assert_eq!(headers["content-security-policy"], "default-src 'none'; frame-ancestors 'none'");
        let no_store = headers.get(header::CACHE_CONTROL).is_some_and(|v| v == "no-store");
        assert_eq!(no_store, api, "{headers:?}");
    };
    let peer = "198.51.100.1:1";
    check(&call_from(&f.app, peer, &[], "GET", "/hello", None, None).await.1, true);
    check(&call_from(&f.app, peer, &[], "GET", "/sync", Some(&f.kid), None).await.1, true);
    check(&call_from(&f.app, peer, &[], "GET", "/sync", None, None).await.1, true);
    check(&call_from(&f.app, peer, &[], "GET", "/nowhere", None, None).await.1, false);

    let jpeg = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x02, 1, 2, 3, 0xFF, 0xD9];
    let (status, body) = raw(&f.app, "POST", "/media", Some(&f.kid), jpeg).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let media_id = serde_json::from_slice::<Value>(&body).unwrap()["mediaId"].as_str().unwrap().to_string();
    let (status, headers) =
        call_from(&f.app, peer, &[], "GET", &format!("/media/{media_id}"), Some(&f.kid), None).await;
    assert_eq!(status, StatusCode::OK);
    check(&headers, false);
}

#[tokio::test]
async fn messages_reach_the_family_and_resends_are_stored_once() {
    let f = family().await;
    let first = sync(&f.app, &f.kid, 0).await;
    assert_eq!(first.members.len(), 2);
    assert_eq!(first.chats.len(), 1);
    let family_chat = first.chats[0].id.clone();
    assert_eq!(first.chats[0].kind, ChatKind::Family);

    let id = ulid();
    let msg = json!({ "id": id, "body": { "kind": "text", "text": " look https://lego.com " } });
    let path = format!("/chats/{family_chat}/messages");
    let (status, body) = call(&f.app, "POST", &path, Some(&f.kid), Some(msg.clone())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["body"]["text"], "look https://lego.com");
    let (status, _) = call(&f.app, "POST", &path, Some(&f.kid), Some(msg)).await;
    assert_eq!(status, StatusCode::OK);

    let parent = sync(&f.app, &f.parent, 0).await;
    assert_eq!(parent.messages.len(), 1);
    assert_eq!(parent.messages[0].sender_id, f.kid_member.id);

    // Catching up only brings what changed.
    let since = parent.rev;
    assert!(sync(&f.app, &f.parent, since).await.messages.is_empty());
    let msg = json!({ "id": ulid(), "body": { "kind": "sticker", "sticker": "wave" } });
    assert_eq!(call(&f.app, "POST", &path, Some(&f.kid), Some(msg)).await.0, StatusCode::OK);
    let later = sync(&f.app, &f.parent, since).await;
    assert_eq!(later.messages.len(), 1);

    // Reactions and seen.
    let react = format!("/messages/{id}/reactions");
    let (_, body) = call(&f.app, "POST", &react, Some(&f.parent), Some(json!({ "emoji": "❤️" }))).await;
    assert_eq!(body["reactions"][0]["emoji"], "❤️");
    let read = format!("/chats/{family_chat}/read");
    let (_, body) = call(&f.app, "POST", &read, Some(&f.parent), Some(json!({ "messageId": id }))).await;
    assert_eq!(body["readUpTo"][&f.parent_member.id], id);

    // Bad messages.
    let empty = json!({ "id": ulid(), "body": { "kind": "text", "text": "   " } });
    assert_eq!(call(&f.app, "POST", &path, Some(&f.kid), Some(empty)).await.0, StatusCode::BAD_REQUEST);
    let sticker = json!({ "id": ulid(), "body": { "kind": "sticker", "sticker": "nope" } });
    assert_eq!(call(&f.app, "POST", &path, Some(&f.kid), Some(sticker)).await.0, StatusCode::BAD_REQUEST);
    let bad_id = json!({ "id": "1", "body": { "kind": "text", "text": "hi" } });
    assert_eq!(call(&f.app, "POST", &path, Some(&f.kid), Some(bad_id)).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn direct_chats_are_private_to_the_two() {
    let f = family().await;
    let (_, body) = call(
        &f.app,
        "POST",
        "/members",
        Some(&f.parent),
        Some(json!({ "name": "Ava", "role": "kid", "color": "bubblegum", "avatar": "🐰" })),
    )
    .await;
    let ava: Member = serde_json::from_value(body).unwrap();
    let ava_token = join(&f.app, &f.parent, &ava.id, "Ava's tablet").await;

    let req = json!({ "memberId": f.parent_member.id });
    let (status, chat) = call(&f.app, "POST", "/chats/direct", Some(&f.kid), Some(req.clone())).await;
    assert_eq!(status, StatusCode::OK);
    // Asking again gives the same chat.
    let (_, again) = call(&f.app, "POST", "/chats/direct", Some(&f.kid), Some(req)).await;
    assert_eq!(chat["id"], again["id"]);

    let path = format!("/chats/{}/messages", chat["id"].as_str().unwrap());
    let msg = json!({ "id": ulid(), "body": { "kind": "text", "text": "secret plan" } });
    assert_eq!(call(&f.app, "POST", &path, Some(&f.kid), Some(msg)).await.0, StatusCode::OK);
    assert_eq!(call(&f.app, "GET", &path, Some(&ava_token), None).await.0, StatusCode::FORBIDDEN);
    assert!(sync(&f.app, &ava_token, 0).await.messages.is_empty());
    let (_, history) = call(&f.app, "GET", &path, Some(&f.parent), None).await;
    assert_eq!(history.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn history_pages_backwards() {
    let f = family().await;
    let chat = sync(&f.app, &f.kid, 0).await.chats[0].id.clone();
    let path = format!("/chats/{chat}/messages");
    let mut ids = Vec::new();
    for i in 0..5 {
        let id = ulid();
        let msg = json!({ "id": id, "body": { "kind": "text", "text": format!("m{i}") } });
        call(&f.app, "POST", &path, Some(&f.kid), Some(msg)).await;
        ids.push(id);
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    let (_, page) = call(&f.app, "GET", &format!("{path}?limit=2"), Some(&f.kid), None).await;
    let texts: Vec<_> = page.as_array().unwrap().iter().map(|m| m["body"]["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["m3", "m4"]);
    let (_, page) = call(&f.app, "GET", &format!("{path}?limit=2&before={}", ids[3]), Some(&f.kid), None).await;
    let texts: Vec<_> = page.as_array().unwrap().iter().map(|m| m["body"]["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["m1", "m2"]);
}

#[tokio::test]
async fn pictures_lose_their_location_and_need_a_token() {
    let f = family().await;
    let (status, _) = raw(&f.app, "POST", "/media", Some(&f.kid), b"<html>not a picture</html>".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x0D];
    jpeg.extend_from_slice(b"Exif\0\0GPS!!");
    jpeg.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 1, 2, 3, 0xFF, 0xD9]);
    let (status, body) = raw(&f.app, "POST", "/media", Some(&f.kid), jpeg).await;
    assert_eq!(status, StatusCode::OK);
    let media_id = serde_json::from_slice::<Value>(&body).unwrap()["mediaId"].as_str().unwrap().to_string();

    let path = format!("/media/{media_id}");
    assert_eq!(raw(&f.app, "GET", &path, None, vec![]).await.0, StatusCode::UNAUTHORIZED);
    let (status, bytes) = raw(&f.app, "GET", &path, Some(&f.parent), vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!bytes.windows(5).any(|w| w == b"GPS!!"));

    let chat = sync(&f.app, &f.kid, 0).await.chats[0].id.clone();
    let msg = json!({ "id": ulid(), "body": { "kind": "image", "mediaId": media_id, "width": 10, "height": 10 } });
    let path = format!("/chats/{chat}/messages");
    assert_eq!(call(&f.app, "POST", &path, Some(&f.kid), Some(msg)).await.0, StatusCode::OK);
    let missing = json!({ "id": ulid(), "body": { "kind": "image", "mediaId": ulid(), "width": 1, "height": 1 } });
    assert_eq!(call(&f.app, "POST", &path, Some(&f.kid), Some(missing)).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn big_uploads_come_in_chunks_and_videos_lose_their_location() {
    let f = family().await;
    // A tiny MP4: ftyp, the frames, and an index holding a location box.
    let mut video = Vec::new();
    for (kind, body) in [(&b"ftyp"[..], &b"isom    "[..]), (b"mdat", b"0123456789frames")] {
        video.extend(((body.len() + 8) as u32).to_be_bytes());
        video.extend(kind);
        video.extend(body);
    }
    let xyz = [&[0, 0, 0, 21][..], b"\xA9xyz", b"+40.71-074.00"].concat();
    let udta = [&((xyz.len() + 8) as u32).to_be_bytes()[..], b"udta", &xyz].concat();
    video.extend([&((udta.len() + 8) as u32).to_be_bytes()[..], b"moov", &udta].concat());

    let (status, state) =
        call(&f.app, "POST", "/media/uploads", Some(&f.kid), Some(json!({ "size": video.len() }))).await;
    assert_eq!(status, StatusCode::OK);
    let id = state["uploadId"].as_str().unwrap().to_string();
    let path = format!("/media/uploads/{id}");
    // Someone else can't see or add to it.
    assert_eq!(call(&f.app, "GET", &path, Some(&f.parent), None).await.0, StatusCode::NOT_FOUND);

    let (first, rest) = video.split_at(20);
    assert_eq!(raw(&f.app, "PUT", &format!("{path}?offset=0"), Some(&f.kid), first.to_vec()).await.0, StatusCode::OK);
    // Finishing early, or a chunk at the wrong place, is refused; the state says where to carry on.
    assert_eq!(call(&f.app, "POST", &format!("{path}/finish"), Some(&f.kid), None).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        raw(&f.app, "PUT", &format!("{path}?offset=5"), Some(&f.kid), rest.to_vec()).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(call(&f.app, "GET", &path, Some(&f.kid), None).await.1["received"], 20);
    assert_eq!(raw(&f.app, "PUT", &format!("{path}?offset=20"), Some(&f.kid), rest.to_vec()).await.0, StatusCode::OK);
    let (status, done) = call(&f.app, "POST", &format!("{path}/finish"), Some(&f.kid), None).await;
    assert_eq!(status, StatusCode::OK);
    let media_id = done["mediaId"].as_str().unwrap().to_string();

    let (status, bytes) = raw(&f.app, "GET", &format!("/media/{media_id}"), Some(&f.parent), vec![]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes.len(), video.len());
    assert!(!bytes.windows(4).any(|w| w == b"\xA9xyz"));

    // A part of it, as a video player asks.
    let req = Request::builder()
        .uri(format!("/media/{media_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {}", f.parent))
        .header(header::RANGE, "bytes=24-29")
        .body(Body::empty())
        .unwrap();
    let res = f.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(res.headers()[header::CONTENT_TYPE], "video/mp4");
    assert_eq!(&res.into_body().collect().await.unwrap().to_bytes()[..], b"012345");

    let chat = sync(&f.app, &f.kid, 0).await.chats[0].id.clone();
    let body = json!({ "kind": "video", "videoMediaId": media_id, "width": 1080, "height": 1920, "durationMs": 1500, "bytes": video.len() });
    let (status, msg) = call(
        &f.app,
        "POST",
        &format!("/chats/{chat}/messages"),
        Some(&f.kid),
        Some(json!({ "id": ulid(), "body": body })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(msg["body"]["kind"], "video");

    // Only photos and videos.
    let (_, state) = call(&f.app, "POST", "/media/uploads", Some(&f.kid), Some(json!({ "size": 6 }))).await;
    let path = format!("/media/uploads/{}", state["uploadId"].as_str().unwrap());
    raw(&f.app, "PUT", &format!("{path}?offset=0"), Some(&f.kid), b"<html>".to_vec()).await;
    assert_eq!(call(&f.app, "POST", &format!("{path}/finish"), Some(&f.kid), None).await.0, StatusCode::BAD_REQUEST);
    // Too big to start.
    let huge = json!({ "size": kit_protocol::MAX_ORIGINAL_BYTES + 1 });
    assert_eq!(call(&f.app, "POST", "/media/uploads", Some(&f.kid), Some(huge)).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_removed_device_is_shut_out() {
    let f = family().await;
    let (_, devices) = call(&f.app, "GET", "/devices", Some(&f.parent), None).await;
    let devices: Vec<Device> = serde_json::from_value(devices).unwrap();
    let tablet = devices.iter().find(|d| d.member_id == f.kid_member.id).unwrap();
    let mut live = f.hub.live.subscribe();
    let path = format!("/devices/{}", tablet.id);
    assert_eq!(call(&f.app, "DELETE", &path, Some(&f.parent), None).await.0, StatusCode::OK);
    assert_eq!(call(&f.app, "GET", "/sync", Some(&f.kid), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(live.recv().await.unwrap().event, Event::Revoked);
}

fn block(text: &str, at: i64) -> Block {
    Block {
        id: ulid(),
        kind: BlockKind::Check,
        text: text.into(),
        checked: false,
        url: String::new(),
        pos: at as f64,
        updated_at: at,
        deleted: false,
        edited_by: String::new(),
        link: None,
    }
}

#[tokio::test]
async fn notes_are_private_until_shared_as_a_family_list() {
    let f = family().await;
    let id = ulid();
    let mut note = Note {
        id: id.clone(),
        owner_id: "ignored".into(),
        title: "Cool sites".into(),
        emoji: "🌐".into(),
        color: "sky".into(),
        shared: false,
        blocks: vec![block("minecraft.net", 1), block("lego.com", 2)],
        created_at: 0,
        meta_updated_at: 1,
        rev: 0,
    };
    let path = format!("/notes/{id}");
    let (status, body) = call(&f.app, "PUT", &path, Some(&f.kid), Some(serde_json::to_value(&note).unwrap())).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ownerId"], f.kid_member.id);

    let parent_first = sync(&f.app, &f.parent, 0).await;
    assert!(parent_first.notes.is_empty());
    let as_parent = serde_json::to_value(&note).unwrap();
    assert_eq!(call(&f.app, "PUT", &path, Some(&f.parent), Some(as_parent)).await.0, StatusCode::NOT_FOUND);

    // Shared: the parent sees it and can tick something off.
    note.shared = true;
    note.meta_updated_at = 2;
    call(&f.app, "PUT", &path, Some(&f.kid), Some(serde_json::to_value(&note).unwrap())).await;
    let seen = sync(&f.app, &f.parent, parent_first.rev).await;
    assert_eq!(seen.notes.len(), 1);
    let mut theirs = seen.notes[0].clone();
    theirs.blocks[1].checked = true;
    theirs.blocks[1].updated_at = 10;
    // The parent can't take sharing away.
    theirs.shared = false;
    theirs.meta_updated_at = 10;
    let (status, merged) =
        call(&f.app, "PUT", &path, Some(&f.parent), Some(serde_json::to_value(&theirs).unwrap())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(merged["shared"], true);
    assert_eq!(merged["blocks"][1]["checked"], true);
    assert_eq!(merged["blocks"][0]["editedBy"], f.kid_member.id.as_str(), "the kid wrote this line");
    assert_eq!(merged["blocks"][1]["editedBy"], f.parent_member.id.as_str(), "the parent ticked this one");

    // The kid shares it into the family chat: the card carries the list's lines.
    let chat = sync(&f.app, &f.kid, 0).await.chats[0].id.clone();
    let msg = json!({ "id": ulid(), "body": { "kind": "note", "noteId": id, "title": "x", "emoji": "x", "preview": [], "shared": false } });
    let (_, sent) = call(&f.app, "POST", &format!("/chats/{chat}/messages"), Some(&f.kid), Some(msg)).await;
    assert_eq!(sent["body"]["title"], "Cool sites");
    assert_eq!(sent["body"]["preview"][1], "☑ lego.com");
    assert_eq!(sent["body"]["shared"], true);

    // Unshared again: it disappears for the parent.
    let before = sync(&f.app, &f.parent, 0).await.rev;
    note.shared = false;
    note.meta_updated_at = 20;
    call(&f.app, "PUT", &path, Some(&f.kid), Some(serde_json::to_value(&note).unwrap())).await;
    let after = sync(&f.app, &f.parent, before).await;
    assert!(after.notes.is_empty());
    assert_eq!(after.deleted_notes, vec![id.clone()]);

    // Only the owner deletes.
    assert_eq!(call(&f.app, "DELETE", &path, Some(&f.parent), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&f.app, "DELETE", &path, Some(&f.kid), None).await.0, StatusCode::OK);
    let (status, _) = call(&f.app, "PUT", &path, Some(&f.kid), Some(serde_json::to_value(&note).unwrap())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

async fn prove_from(app: &Router, peer: &str, device_id: &str, nonce: &str) -> (StatusCode, Value) {
    let peer: std::net::SocketAddr = peer.parse().unwrap();
    let body = json!({ "deviceId": device_id, "nonce": nonce }).to_string();
    let req = Request::builder()
        .method("POST")
        .uri("/prove")
        .extension(ConnectInfo(peer))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn the_hub_proves_itself_to_its_own_devices_nearby() {
    let f = family().await;
    let (_, devices) = call(&f.app, "GET", "/devices", Some(&f.parent), None).await;
    let devices: Vec<Device> = serde_json::from_value(devices).unwrap();
    let kid = devices.iter().find(|d| d.member_id == f.kid_member.id).unwrap();
    let nonce = "ab".repeat(32);
    let expected = proof::hub_proof(&proof::token_hash(&f.kid), &kid.id, &nonce);

    for peer in ["192.168.1.30:5000", "100.101.102.103:5000"] {
        let (status, body) = prove_from(&f.app, peer, &kid.id, &nonce).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["proof"], expected.as_str());
    }

    // From the internet, or through a proxy on this computer, it won't answer.
    assert_eq!(prove_from(&f.app, "8.8.8.8:5000", &kid.id, &nonce).await.0, StatusCode::FORBIDDEN);
    assert_eq!(prove_from(&f.app, "127.0.0.1:5000", &kid.id, &nonce).await.0, StatusCode::FORBIDDEN);

    let peer = "192.168.1.31:5000";
    assert_eq!(prove_from(&f.app, peer, &ulid(), &nonce).await.0, StatusCode::NOT_FOUND);
    assert_eq!(prove_from(&f.app, peer, &kid.id, "short").await.0, StatusCode::BAD_REQUEST);

    // A removed device can still tell it's talking to the hub that removed it.
    let path = format!("/devices/{}", kid.id);
    assert_eq!(call(&f.app, "DELETE", &path, Some(&f.parent), None).await.0, StatusCode::OK);
    let (status, body) = prove_from(&f.app, peer, &kid.id, &nonce).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["proof"], expected.as_str());
}

#[tokio::test]
async fn hello_and_sync_say_which_hub_and_family() {
    let f = family().await;
    let (_, hello) = call(&f.app, "GET", "/hello", None, None).await;
    let hello: Hello = serde_json::from_value(hello).unwrap();
    assert_eq!(hello.hub_id.as_deref(), Some(f.hub.hub_id.as_str()));
    assert!(hello.family_id.is_some_and(|id| !id.is_empty()));
    assert_eq!(sync(&f.app, &f.kid, 0).await.hub_id, hello.hub_id);

    // A hub that isn't set up has an ID, but no family yet.
    let dir = temp_dir();
    let new = app(open_in_memory(dir.0.clone()).await.unwrap());
    let (_, hello) = call(&new, "GET", "/hello", None, None).await;
    assert!(hello["hubId"].is_string());
    assert!(hello.get("familyId").is_none());
}

#[tokio::test]
async fn a_hub_keeps_its_id_and_a_copy_of_its_data_gets_its_own() {
    let dir = temp_dir();
    let open = |data_dir| {
        kitchat_hub::open(kitchat_hub::Options {
            data_dir,
            previews: false,
            limits: Config::default(),
            keep_files: Default::default(),
        })
    };
    let (home, copy) = (dir.0.join("home"), dir.0.join("copy"));
    let hub = open(home.clone()).await.unwrap();
    let id = hub.hub_id.clone();
    hub.pool.close().await;
    drop(hub);
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(home.join("kitchat-hub.db"), copy.join("kitchat-hub.db")).unwrap();

    let again = open(home).await.unwrap();
    assert_eq!(again.hub_id, id);
    let copied = open(copy).await.unwrap();
    assert_ne!(copied.hub_id, id);
    again.pool.close().await;
    copied.pool.close().await;
}

#[tokio::test]
async fn a_message_sent_late_keeps_the_time_it_was_written() {
    let f = family().await;
    let family_chat = sync(&f.app, &f.kid, 0).await.chats[0].id.clone();
    let path = format!("/chats/{family_chat}/messages");
    let written = kitchat_hub::now_ms() - 3 * 60 * 60 * 1000;
    let id = ulid::Ulid::from_parts(written as u64, 1).to_string();
    let msg = json!({ "id": id, "body": { "kind": "text", "text": "at the lake" }, "sentAt": written });
    let (status, body) = call(&f.app, "POST", &path, Some(&f.kid), Some(msg)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["createdAt"], written);

    // A time that doesn't match the message's own ID isn't believed.
    let before = kitchat_hub::now_ms();
    let msg = json!({ "id": ulid(), "body": { "kind": "text", "text": "hi" }, "sentAt": written });
    let (_, body) = call(&f.app, "POST", &path, Some(&f.kid), Some(msg)).await;
    assert!(body["createdAt"].as_i64().unwrap() >= before);
}

#[tokio::test]
async fn a_reaction_taken_away_is_gone() {
    let f = family().await;
    let family_chat = sync(&f.app, &f.kid, 0).await.chats[0].id.clone();
    let id = ulid();
    let msg = json!({ "id": id, "body": { "kind": "text", "text": "pizza?" } });
    call(&f.app, "POST", &format!("/chats/{family_chat}/messages"), Some(&f.kid), Some(msg)).await;
    let react = format!("/messages/{id}/reactions");
    let (_, body) = call(&f.app, "POST", &react, Some(&f.parent), Some(json!({ "emoji": "🍕" }))).await;
    assert_eq!(body["reactions"].as_array().unwrap().len(), 1);
    let (_, body) = call(&f.app, "POST", &react, Some(&f.parent), Some(json!({ "emoji": "" }))).await;
    assert!(body["reactions"].as_array().unwrap().is_empty());
    assert!(sync(&f.app, &f.kid, 0).await.messages[0].reactions.is_empty());
}

#[tokio::test]
async fn a_direct_chat_id_comes_from_its_two_members() {
    let f = family().await;
    let req = json!({ "memberId": f.parent_member.id });
    let (_, chat) = call(&f.app, "POST", "/chats/direct", Some(&f.kid), Some(req)).await;
    let mut pair = [f.parent_member.id.clone(), f.kid_member.id.clone()];
    pair.sort();
    let expected = kitchat_hub::direct_chat_id(&pair.join(":"));
    assert_eq!(chat["id"], expected.as_str());
    assert!(ulid::Ulid::from_string(&expected).is_ok());
}
