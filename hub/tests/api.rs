//! The hub's REST API, end to end through the router: setting up, joining, chatting, pictures, notes and removing a
//! device.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use kit_protocol::notes::{Block, BlockKind, Note};
use kit_protocol::*;
use kitchat_hub::{Hub, app, open_in_memory};
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
    for _ in 0..10 {
        let req = json!({ "code": "ZZZZZZ", "deviceName": "x" });
        assert_eq!(call(&f.app, "POST", "/enroll", None, Some(req)).await.0, StatusCode::BAD_REQUEST);
    }
    let req = json!({ "code": "ZZZZZZ", "deviceName": "x" });
    assert_eq!(call(&f.app, "POST", "/enroll", None, Some(req)).await.0, StatusCode::TOO_MANY_REQUESTS);
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

    let (status, state) = call(&f.app, "POST", "/media/uploads", Some(&f.kid), Some(json!({ "size": video.len() }))).await;
    assert_eq!(status, StatusCode::OK);
    let id = state["uploadId"].as_str().unwrap().to_string();
    let path = format!("/media/uploads/{id}");
    // Someone else can't see or add to it.
    assert_eq!(call(&f.app, "GET", &path, Some(&f.parent), None).await.0, StatusCode::NOT_FOUND);

    let (first, rest) = video.split_at(20);
    assert_eq!(raw(&f.app, "PUT", &format!("{path}?offset=0"), Some(&f.kid), first.to_vec()).await.0, StatusCode::OK);
    // Finishing early, or a chunk at the wrong place, is refused; the state says where to carry on.
    assert_eq!(call(&f.app, "POST", &format!("{path}/finish"), Some(&f.kid), None).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(raw(&f.app, "PUT", &format!("{path}?offset=5"), Some(&f.kid), rest.to_vec()).await.0, StatusCode::CONFLICT);
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
    let (status, msg) = call(&f.app, "POST", &format!("/chats/{chat}/messages"), Some(&f.kid), Some(json!({ "id": ulid(), "body": body }))).await;
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
