//! The commands the page calls (src/lib/api.ts).

use std::sync::Arc;

use kit_protocol::notes::Note;
use kit_protocol::{Chat, Device, EnrollRequest, Hello, Member, MemberUpdate, MessageBody, NewMember, SetupRequest};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, State};

use crate::engine::{Delta, Engine};
use crate::error::{AppError, AppResult};
use crate::hub::{HubClient, normalize_url};
use crate::platform::{self, SharedItem};
use crate::store::{OutboxEntry, PAGE, UiMessage, outbox_message};

type Eng<'a> = State<'a, Arc<Engine>>;

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default()
}

#[tauri::command]
pub async fn snapshot(engine: Eng<'_>) -> AppResult<Delta> {
    engine.snapshot().await
}

#[tauri::command]
pub async fn hub_hello(url: String) -> AppResult<Hello> {
    HubClient::new(&url, None)?.hello().await
}

#[tauri::command]
pub async fn setup_family(
    engine: Eng<'_>,
    url: String,
    setup_code: String,
    family_name: String,
    name: String,
    device_name: String,
) -> AppResult<()> {
    let url = normalize_url(&url)?;
    let client = HubClient::new(&url, None)?;
    let enrolled = client.setup(&SetupRequest { setup_code, family_name, name, device_name }).await?;
    engine.enrolled(&url, enrolled).await
}

#[tauri::command]
pub async fn join_family(engine: Eng<'_>, url: String, code: String, device_name: String) -> AppResult<()> {
    let url = normalize_url(&url)?;
    let client = HubClient::new(&url, None)?;
    let enrolled = client.enroll(&EnrollRequest { code, device_name }).await?;
    engine.enrolled(&url, enrolled).await
}

#[tauri::command]
pub async fn sign_out(engine: Eng<'_>) -> AppResult<()> {
    engine.sign_out(None).await
}

async fn queue(engine: &Engine, chat_id: String, body: MessageBody, upload: Option<String>) -> AppResult<UiMessage> {
    let account = engine.store.account().await?.ok_or(AppError::NotEnrolled)?;
    let entry =
        OutboxEntry { id: ulid::Ulid::new().to_string(), chat_id, body, upload, created_at: now_ms(), error: None };
    engine.store.add_outbox(&entry).await?;
    let message = outbox_message(&entry, &account.me.id);
    engine.emit(Delta { messages: vec![message.clone()], ..Default::default() });
    engine.kick();
    Ok(message)
}

#[tauri::command]
pub async fn send_message(engine: Eng<'_>, chat_id: String, body: MessageBody) -> AppResult<UiMessage> {
    if let MessageBody::Text { text } = &body
        && text.trim().is_empty()
    {
        return Err(AppError::Invalid("Type something first!".into()));
    }
    queue(&engine, chat_id, body, None).await
}

/// A picture, already shrunk and re-drawn by the page (which drops its EXIF data), as the raw request body. The
/// chat and size come in headers: `x-chat-id`, `x-width`, `x-height`, and `x-caption` (URL-encoded).
#[tauri::command]
pub async fn send_image(engine: Eng<'_>, request: Request<'_>) -> AppResult<UiMessage> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(AppError::Invalid("No picture came through.".into()));
    };
    if bytes.len() > kit_protocol::MAX_MEDIA_BYTES {
        return Err(AppError::Invalid("That picture is too big.".into()));
    }
    let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let chat_id = header("x-chat-id");
    let width = header("x-width").parse().unwrap_or(0);
    let height = header("x-height").parse().unwrap_or(0);
    let caption = percent_decode(&header("x-caption"));
    let local = format!("local-{}", ulid::Ulid::new());
    tokio::fs::write(engine.media_dir.join(&local), bytes).await?;
    let body = MessageBody::Image { media_id: local.clone(), width, height, caption };
    queue(&engine, chat_id, body, Some(local)).await
}

#[tauri::command]
pub async fn retry_message(engine: Eng<'_>, id: String) -> AppResult<()> {
    engine.store.outbox_failed(&id, None).await?;
    if let Some(entry) = engine.store.outbox_entry(&id).await? {
        let me = engine.store.account().await?.map(|a| a.me.id).unwrap_or_default();
        engine.emit(Delta { messages: vec![outbox_message(&entry, &me)], ..Default::default() });
    }
    engine.kick();
    Ok(())
}

#[tauri::command]
pub async fn discard_message(engine: Eng<'_>, id: String) -> AppResult<()> {
    if let Some(entry) = engine.store.outbox_entry(&id).await?
        && let Some(local) = entry.upload
    {
        let _ = tokio::fs::remove_file(engine.media_dir.join(local)).await;
    }
    engine.store.remove_outbox(&id).await
}

/// A picture's bytes: from this device's cache, or fetched from the hub (and cached).
#[tauri::command]
pub async fn media(engine: Eng<'_>, id: String) -> AppResult<Response> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(AppError::Invalid("bad picture ID".into()));
    }
    let path = engine.media_dir.join(&id);
    if let Ok(bytes) = tokio::fs::read(&path).await {
        return Ok(Response::new(bytes));
    }
    let bytes = engine.client()?.media(&id).await?;
    let _ = tokio::fs::write(&path, &bytes).await;
    Ok(Response::new(bytes))
}

/// Older messages of a chat, before `before`: from the local copy, or from the hub when it has run out.
#[tauri::command]
pub async fn older_messages(engine: Eng<'_>, chat_id: String, before: String) -> AppResult<Vec<UiMessage>> {
    let local = engine.store.messages(&chat_id, Some(&before), PAGE).await?;
    if local.len() as i64 >= PAGE {
        return Ok(local);
    }
    let oldest = local.first().map(|m| m.message.id.clone()).unwrap_or(before);
    match engine.client()?.history(&chat_id, &oldest).await {
        Ok(fetched) => {
            for message in &fetched {
                engine.store.put_message(message).await?;
            }
            let mut all: Vec<UiMessage> = fetched.into_iter().map(UiMessage::from).collect();
            all.extend(local);
            Ok(all)
        }
        Err(AppError::Offline(_)) => Ok(local),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn mark_read(engine: Eng<'_>, chat_id: String, message_id: String) -> AppResult<()> {
    let client = engine.client()?;
    match client.mark_read(&chat_id, &message_id).await {
        Ok(chat) => {
            engine.store.put_chat(&chat).await?;
            engine.emit(Delta { chats: vec![chat], ..Default::default() });
            Ok(())
        }
        // "Seen" isn't worth an error: it's sent again next time the chat opens.
        Err(AppError::Offline(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn react(engine: Eng<'_>, message_id: String, emoji: String) -> AppResult<()> {
    let message = engine.client()?.react(&message_id, &emoji).await?;
    engine.store.put_message(&message).await?;
    engine.emit(Delta { messages: vec![message.into()], ..Default::default() });
    Ok(())
}

#[tauri::command]
pub async fn open_direct(engine: Eng<'_>, member_id: String) -> AppResult<Chat> {
    // Already have it: no need to ask the hub.
    let me = engine.store.account().await?.ok_or(AppError::NotEnrolled)?.me.id;
    if let Some(chat) = engine.store.chats().await?.into_iter().find(|c| {
        c.kind == kit_protocol::ChatKind::Direct && c.member_ids.contains(&member_id) && c.member_ids.contains(&me)
    }) {
        return Ok(chat);
    }
    let chat = engine.client()?.direct(&member_id).await?;
    engine.store.put_chat(&chat).await?;
    engine.emit(Delta { chats: vec![chat.clone()], ..Default::default() });
    Ok(chat)
}

/// Saves a note here at once, and on the hub as soon as it can.
#[tauri::command]
pub async fn save_note(engine: Eng<'_>, note: Note) -> AppResult<()> {
    kit_protocol::notes::validate(&note).map_err(AppError::Invalid)?;
    engine.store.put_note(&note, true).await?;
    engine.emit(Delta { notes: vec![note], ..Default::default() });
    engine.kick();
    Ok(())
}

#[tauri::command]
pub async fn delete_note(engine: Eng<'_>, id: String) -> AppResult<()> {
    engine.store.mark_note_deleted(&id).await?;
    engine.emit(Delta { deleted_notes: vec![id], ..Default::default() });
    engine.kick();
    Ok(())
}

#[tauri::command]
pub async fn add_member(engine: Eng<'_>, member: NewMember) -> AppResult<Member> {
    engine.client()?.add_member(&member).await
}

#[tauri::command]
pub async fn update_member(engine: Eng<'_>, id: String, update: MemberUpdate) -> AppResult<Member> {
    engine.client()?.update_member(&id, &update).await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinCode {
    pub code: String,
    pub expires_at: i64,
    pub hub_url: String,
    /// What the QR code holds: `kitchat://join?hub=…&code=…`.
    pub link: String,
    pub qr_svg: String,
}

#[tauri::command]
pub async fn join_code(engine: Eng<'_>, member_id: String) -> AppResult<JoinCode> {
    let client = engine.client()?;
    let code = client.join_code(&member_id).await?;
    let hub_url = client.base().to_string();
    let link = format!("kitchat://join?hub={}&code={}", percent_encode(&hub_url), code.code);
    let qr = qrcode::QrCode::new(link.as_bytes()).map_err(|e| AppError::Other(format!("QR code: {e}")))?;
    let qr_svg = qr
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(qrcode::render::svg::Color("#1f1147"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build();
    Ok(JoinCode { code: code.code, expires_at: code.expires_at, hub_url, link, qr_svg })
}

#[tauri::command]
pub async fn devices(engine: Eng<'_>) -> AppResult<Vec<Device>> {
    engine.client()?.devices().await
}

#[tauri::command]
pub async fn revoke_device(engine: Eng<'_>, id: String) -> AppResult<()> {
    engine.client()?.revoke(&id).await
}

#[tauri::command]
pub fn set_foreground(engine: Eng<'_>, foreground: bool) {
    engine.set_foreground(foreground);
}

#[tauri::command]
pub async fn take_shared(app: AppHandle) -> Vec<SharedItem> {
    platform::take_shared(&app).await
}

/// A picture shared in from another app, read once from the app's cache and then deleted.
#[tauri::command]
pub async fn read_shared(app: AppHandle, path: String) -> AppResult<Response> {
    use tauri::Manager;
    let cache = app.path().app_cache_dir()?.join("shared");
    let path = std::path::PathBuf::from(path);
    let inside = path.canonicalize().ok().zip(cache.canonicalize().ok()).is_some_and(|(p, c)| p.starts_with(c));
    if !inside {
        return Err(AppError::Invalid("That file isn't one that was shared.".into()));
    }
    let bytes = tokio::fs::read(&path).await?;
    let _ = tokio::fs::remove_file(&path).await;
    Ok(Response::new(bytes))
}

#[tauri::command]
pub async fn scan_qr(app: AppHandle) -> Result<Option<String>, String> {
    platform::scan(&app).await
}

#[tauri::command]
pub fn device_name_hint() -> String {
    if cfg!(target_os = "android") {
        return "Tablet".into();
    }
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "Computer".into())
}

#[tauri::command]
pub fn log(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!("page: {message}"),
        "warn" => log::warn!("page: {message}"),
        _ => log::info!("page: {message}"),
    }
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_round_trip() {
        let s = "Sam's 🦊 picture: 100%";
        assert_eq!(percent_decode(&percent_encode(s)), s);
        assert_eq!(percent_encode("http://a.b:1"), "http%3A%2F%2Fa.b%3A1");
    }
}
