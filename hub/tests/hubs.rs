//! Two hubs of one family side by side (docs/multi_hub_plan.md): a new hub gets the whole family, hubs that were apart
//! combine what happened on each, and hubs that keep swapping changes settle.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use kit_protocol::hubs::Changes;
use kit_protocol::notes::{Block, BlockKind, Note};
use kit_protocol::*;
use kitchat_hub::{Hub, app, open_in_memory, replica};
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
    _dir: TempDir,
}

async fn new_hub() -> TestHub {
    let dir = std::env::temp_dir().join(format!("kitchat-hubs-test-{}", ulid()));
    std::fs::create_dir_all(&dir).unwrap();
    let hub = open_in_memory(dir.clone()).await.unwrap();
    TestHub { app: app(hub.clone()), hub, _dir: TempDir(dir) }
}

fn ulid() -> String {
    ulid::Ulid::new().to_string()
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

async fn ok(app: &Router, method: &str, path: &str, token: &str, body: Value) -> Value {
    let (status, body) = call(app, method, path, Some(token), Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{method} {path}: {body}");
    body
}

/// A family on `h`: a parent, and a kid with two devices. Returns (parent token, kid token, kid's second token).
async fn set_up(h: &TestHub) -> (String, String, String) {
    let code = h.hub.setup_code.lock().await.clone().unwrap();
    let (_, body) = call(
        &h.app,
        "POST",
        "/setup",
        None,
        Some(json!({ "setupCode": code, "familyName": "The Lynx Family", "name": "Dad", "deviceName": "Desktop" })),
    )
    .await;
    let parent = serde_json::from_value::<Enrolled>(body).unwrap().token;
    let kid = ok(
        &h.app,
        "POST",
        "/members",
        &parent,
        json!({ "name": "Sam", "role": "kid", "color": "mint", "avatar": "🦊" }),
    )
    .await;
    let kid_id = kid["id"].as_str().unwrap().to_string();
    let kid = join(&h.app, &parent, &kid_id, "Sam's tablet").await;
    let kid_phone = join(&h.app, &parent, &kid_id, "Sam's old phone").await;
    (parent, kid, kid_phone)
}

async fn join(app: &Router, parent: &str, member_id: &str, device: &str) -> String {
    let code = ok(app, "POST", &format!("/members/{member_id}/codes"), parent, Value::Null).await;
    let (_, body) =
        call(app, "POST", "/enroll", None, Some(json!({ "code": code["code"], "deviceName": device }))).await;
    serde_json::from_value::<Enrolled>(body).unwrap().token
}

async fn sync(app: &Router, token: &str) -> SyncResponse {
    let (status, body) = call(app, "GET", "/sync?since=0", Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).unwrap()
}

async fn send(app: &Router, token: &str, chat: &str, text: &str) -> String {
    let id = ulid();
    ok(
        app,
        "POST",
        &format!("/chats/{chat}/messages"),
        token,
        json!({ "id": id, "body": { "kind": "text", "text": text } }),
    )
    .await;
    id
}

/// Sends everything `from` has after `since` to `to`, in batches of `limit`, and moves `since` on. Returns how many
/// rows changed on `to`.
async fn swap(from: &Hub, to: &Hub, since: &mut i64, limit: i64) -> usize {
    let mut changed = 0;
    loop {
        let changes = replica::changes_since(from, *since, limit).await.unwrap();
        *since = changes.rev;
        let more = changes.more;
        changed += replica::apply(to, changes).await.unwrap();
        if !more {
            return changed;
        }
    }
}

/// Everything a hub has, in an order and form two hubs with the same family should agree on exactly.
async fn everything(hub: &Hub) -> Changes {
    let mut all = replica::changes_since(hub, 0, 100_000).await.unwrap();
    all.hub_id.clear();
    all.rev = 0;
    all.members.sort_by(|a, b| a.member.id.cmp(&b.member.id));
    all.devices.sort_by(|a, b| a.id.cmp(&b.id));
    for device in &mut all.devices {
        // Each hub notes when a device last used it.
        device.last_seen = 0;
    }
    all.chats.sort_by(|a, b| a.id.cmp(&b.id));
    all.messages.sort_by(|a, b| a.id.cmp(&b.id));
    all.notes.sort_by(|a, b| a.note.id.cmp(&b.note.id));
    for note in &mut all.notes {
        note.note.blocks.sort_by(|a, b| a.id.cmp(&b.id));
    }
    all.media.sort_by(|a, b| a.id.cmp(&b.id));
    all
}

fn block(id: &str, text: &str, at: i64) -> Block {
    Block {
        id: id.into(),
        kind: BlockKind::Check,
        text: text.into(),
        checked: false,
        url: String::new(),
        pos: 1.0,
        updated_at: at,
        deleted: false,
        edited_by: String::new(),
    }
}

fn list(id: &str, blocks: Vec<Block>) -> Value {
    serde_json::to_value(Note {
        id: id.into(),
        owner_id: String::new(),
        title: "Camping".into(),
        emoji: "⛺".into(),
        color: "mint".into(),
        shared: true,
        blocks,
        created_at: 0,
        meta_updated_at: 1,
        rev: 0,
    })
    .unwrap()
}

async fn add_media(hub: &Hub, id: &str) {
    sqlx::query("INSERT INTO media (id, uploader_id, content_type, size, created_at, rev) VALUES (?, NULL, 'image/jpeg', 9, 1, 1)")
        .bind(id)
        .execute(&hub.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_new_hub_gets_the_whole_family_and_its_devices_sign_in_there() {
    let (home, travel) = (new_hub().await, new_hub().await);
    let (parent, kid, _) = set_up(&home).await;
    let family_chat = sync(&home.app, &kid).await.chats[0].id.clone();
    let mut sent = Vec::new();
    for n in 0..12 {
        sent.push(send(&home.app, &kid, &family_chat, &format!("message {n}")).await);
    }
    ok(&home.app, "POST", &format!("/messages/{}/reactions", sent[0]), &parent, json!({ "emoji": "❤️" })).await;
    // "Seen" changes the chat after its messages, so the chat has to come along with the messages' batch.
    ok(&home.app, "POST", &format!("/chats/{family_chat}/read"), &parent, json!({ "messageId": sent[11] })).await;
    let note = ulid();
    ok(&home.app, "PUT", &format!("/notes/{note}"), &kid, list(&note, vec![block("a", "tent", 1)])).await;
    add_media(&home.hub, &ulid()).await;

    // Small batches, so it takes several.
    let mut since = 0;
    assert!(swap(&home.hub, &travel.hub, &mut since, 3).await > 0);
    assert_eq!(everything(&home.hub).await, everything(&travel.hub).await);

    // The travel hub is set up now, as the same family, and the kid's tablet can use it as it is.
    assert!(travel.hub.setup_code.lock().await.is_none());
    let (_, hello) = call(&travel.app, "GET", "/hello", None, None).await;
    let (_, home_hello) = call(&home.app, "GET", "/hello", None, None).await;
    assert_eq!(hello["familyName"], "The Lynx Family");
    assert_eq!(hello["familyId"], home_hello["familyId"]);
    assert_ne!(hello["hubId"], home_hello["hubId"]);
    let there = sync(&travel.app, &kid).await;
    assert_eq!(there.messages.len(), 12);
    assert_eq!(there.notes.len(), 1);
    assert_eq!(there.chats[0].read_up_to.values().next(), Some(&sent[11]));
    let (status, _) = call(
        &travel.app,
        "POST",
        "/setup",
        None,
        Some(json!({ "setupCode": "ANYTHING", "familyName": "F", "name": "X", "deviceName": "Y" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn hubs_apart_combine_what_happened_on_each_and_settle() {
    let (home, travel) = (new_hub().await, new_hub().await);
    let (parent, kid, kid_phone) = set_up(&home).await;
    let first = sync(&home.app, &kid).await;
    let family_chat = first.chats[0].id.clone();
    let kid_id = first.me.unwrap().id;
    let before = send(&home.app, &kid, &family_chat, "packing!").await;
    let note = ulid();
    ok(
        &home.app,
        "PUT",
        &format!("/notes/{note}"),
        &kid,
        list(&note, vec![block("a", "tent", 1), block("b", "torch", 1)]),
    )
    .await;

    let (mut home_seen, mut travel_seen) = (0, 0);
    swap(&home.hub, &travel.hub, &mut home_seen, 50).await;
    // What the travel hub took in changed numbers there; sending it back changes nothing at home.
    assert_eq!(swap(&travel.hub, &home.hub, &mut travel_seen, 50).await, 0);

    // Apart. At home:
    let at_home = send(&home.app, &kid, &family_chat, "from home").await;
    ok(&home.app, "POST", &format!("/messages/{before}/reactions"), &parent, json!({ "emoji": "👍" })).await;
    let mut home_list = list(&note, vec![block("a", "big tent", 5), block("b", "torch", 1)]);
    home_list["metaUpdatedAt"] = json!(1);
    ok(&home.app, "PUT", &format!("/notes/{note}"), &kid, home_list).await;
    let (_, devices) = call(&home.app, "GET", "/devices", Some(&parent), None).await;
    let phone = devices.as_array().unwrap().iter().find(|d| d["name"] == "Sam's old phone").unwrap()["id"].clone();
    assert_eq!(
        call(&home.app, "DELETE", &format!("/devices/{}", phone.as_str().unwrap()), Some(&parent), None).await.0,
        StatusCode::OK
    );
    let home_direct = ok(&home.app, "POST", "/chats/direct", &kid, json!({ "memberId": first.members[0].id })).await;
    send(&home.app, &kid, home_direct["id"].as_str().unwrap(), "psst (home)").await;

    // On the trip:
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let on_trip = send(&travel.app, &parent, &family_chat, "from the lake").await;
    ok(&travel.app, "POST", &format!("/messages/{before}/reactions"), &parent, json!({ "emoji": "❤️" })).await;
    ok(
        &travel.app,
        "PUT",
        &format!("/notes/{note}"),
        &kid,
        list(&note, vec![block("a", "tent", 1), block("b", "torch ✓", 6)]),
    )
    .await;
    let ava = ok(
        &travel.app,
        "POST",
        "/members",
        &parent,
        json!({ "name": "Ava", "role": "kid", "color": "grape", "avatar": "🐰" }),
    )
    .await;
    ok(&travel.app, "PATCH", &format!("/members/{kid_id}"), &kid, json!({ "avatar": "🐯" })).await;
    ok(&travel.app, "POST", &format!("/chats/{family_chat}/read"), &kid, json!({ "messageId": on_trip })).await;
    let trip_direct = ok(&travel.app, "POST", "/chats/direct", &kid, json!({ "memberId": first.members[0].id })).await;
    send(&travel.app, &kid, trip_direct["id"].as_str().unwrap(), "psst (trip)").await;
    assert_eq!(home_direct["id"], trip_direct["id"], "the same two people's chat has the same ID on both hubs");

    // Back in touch. Devices connected to the travel hub hear about what happened at home.
    let mut live = travel.hub.live.subscribe();
    assert!(swap(&home.hub, &travel.hub, &mut home_seen, 4).await > 0);
    assert!(swap(&travel.hub, &home.hub, &mut travel_seen, 4).await > 0);
    let mut heard = Vec::new();
    while let Ok(delivery) = live.try_recv() {
        heard.push(delivery.event.clone());
    }
    assert!(heard.iter().any(|e| matches!(e, Event::Message { message, .. } if message.id == at_home)));
    assert!(heard.iter().any(|e| matches!(e, Event::Revoked)));

    // Settled: another round changes nothing, either way.
    for _ in 0..2 {
        swap(&home.hub, &travel.hub, &mut home_seen, 50).await;
        swap(&travel.hub, &home.hub, &mut travel_seen, 50).await;
    }
    assert_eq!(swap(&home.hub, &travel.hub, &mut home_seen, 50).await, 0);
    assert_eq!(swap(&travel.hub, &home.hub, &mut travel_seen, 50).await, 0);
    assert_eq!(everything(&home.hub).await, everything(&travel.hub).await);

    // And what they agree on is right.
    let view = sync(&home.app, &kid).await;
    let texts: Vec<_> = view
        .messages
        .iter()
        .filter_map(|m| match &m.body {
            MessageBody::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    for text in ["packing!", "from home", "from the lake", "psst (home)", "psst (trip)"] {
        assert!(texts.contains(&text), "{text} is missing from {texts:?}");
    }
    let packing = view.messages.iter().find(|m| m.id == before).unwrap();
    assert_eq!(
        packing.reactions,
        vec![Reaction { member_id: first.members[0].id.clone(), emoji: "❤️".into() }],
        "the newer reaction wins"
    );
    let camping = &view.notes[0];
    let text = |id: &str| camping.blocks.iter().find(|b| b.id == id).unwrap().text.clone();
    assert_eq!((text("a"), text("b")), ("big tent".into(), "torch ✓".into()), "both edits are kept");
    assert!(view.members.iter().any(|m| m.id == ava["id"].as_str().unwrap()), "Ava, added on the trip, is at home too");
    assert_eq!(view.members.iter().find(|m| m.id == kid_id).unwrap().avatar, "🐯");
    assert_eq!(view.chats.iter().filter(|c| c.kind == ChatKind::Direct).count(), 1);
    let family = view.chats.iter().find(|c| c.id == family_chat).unwrap();
    assert_eq!(family.read_up_to.get(&kid_id), Some(&on_trip));
    // The phone removed at home is removed on the trip too.
    assert_eq!(call(&travel.app, "GET", "/sync", Some(&kid_phone), None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&travel.app, "GET", "/sync", Some(&kid), None).await.0, StatusCode::OK);
}

#[tokio::test]
async fn hubs_of_different_families_refuse_each_other() {
    let (ours, theirs) = (new_hub().await, new_hub().await);
    set_up(&ours).await;
    set_up(&theirs).await;
    let changes = replica::changes_since(&theirs.hub, 0, 100).await.unwrap();
    assert!(matches!(replica::apply(&ours.hub, changes).await, Err(kitchat_hub::error::ApiError::Conflict(_))));
    // Nor can a hub take in its own changes.
    let own = replica::changes_since(&ours.hub, 0, 100).await.unwrap();
    assert!(replica::apply(&ours.hub, own).await.is_err());
    // A hub that isn't set up has nothing to send.
    let fresh = new_hub().await;
    assert!(replica::changes_since(&fresh.hub, 0, 100).await.is_err());
}
