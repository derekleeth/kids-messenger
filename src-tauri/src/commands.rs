//! The commands the page calls (src/lib/api.ts).

use std::sync::Arc;

use kit_protocol::notes::Note;
use kit_protocol::{Chat, Device, EnrollRequest, Hello, Member, MemberUpdate, MessageBody, NewMember, SetupRequest};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State};

use crate::engine::{Delta, Engine};
use crate::error::{AppError, AppResult};
use crate::hub::{HubClient, looks_local, normalize_url};
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

/// Android 17 blocks the home network until the person allows it; asked before the first contact with a hub there.
async fn allow_home_network(app: &AppHandle, url: &str) -> AppResult<()> {
    let host = url.split("://").nth(1).unwrap_or(url);
    if looks_local(host) && !platform::local_network(app).await {
        return Err(AppError::Invalid(
            "KitChat needs permission to reach devices on your home network to find the family hub. Allow it in              Settings › Apps › KitChat › Permissions."
                .into(),
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn hub_hello(app: AppHandle, url: String) -> AppResult<Hello> {
    let url = normalize_url(&url)?;
    allow_home_network(&app, &url).await?;
    HubClient::new(&url, None)?.hello().await
}

#[tauri::command]
pub async fn setup_family(
    app: AppHandle,
    engine: Eng<'_>,
    url: String,
    setup_code: String,
    family_name: String,
    name: String,
    device_name: String,
) -> AppResult<()> {
    let url = normalize_url(&url)?;
    allow_home_network(&app, &url).await?;
    let client = HubClient::new(&url, None)?;
    let enrolled = client.setup(&SetupRequest { setup_code, family_name, name, device_name }).await?;
    engine.enrolled(&url, enrolled).await
}

#[tauri::command]
pub async fn join_family(
    app: AppHandle,
    engine: Eng<'_>,
    url: String,
    code: String,
    device_name: String,
) -> AppResult<()> {
    let url = normalize_url(&url)?;
    allow_home_network(&app, &url).await?;
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

/// The bytes of a call's body. Desktop sends them as they are; Android's WebView can't hand a request body over, so
/// Tauri sends them through postMessage as JSON instead: an array of numbers.
fn body_bytes(request: &Request<'_>) -> AppResult<Vec<u8>> {
    let bytes = match request.body() {
        InvokeBody::Raw(bytes) => bytes.clone(),
        InvokeBody::Json(value) => picture_from_json(value).unwrap_or_default(),
    };
    if bytes.is_empty() {
        return Err(AppError::Invalid("No picture came through.".into()));
    }
    Ok(bytes)
}

/// Picture bytes sent as JSON: an array of numbers, or an object of index to number (a typed array serialized as is).
fn picture_from_json(value: &serde_json::Value) -> Option<Vec<u8>> {
    let byte = |v: &serde_json::Value| v.as_u64().and_then(|n| u8::try_from(n).ok());
    match value {
        serde_json::Value::Array(items) => items.iter().map(byte).collect(),
        serde_json::Value::Object(map) => {
            let mut bytes = vec![0u8; map.len()];
            for (key, v) in map {
                *bytes.get_mut(key.parse::<usize>().ok()?)? = byte(v)?;
            }
            Some(bytes)
        }
        _ => None,
    }
}

/// Where files on their way out wait: the page's pictures (`stage_file`), and on Android the photos and videos picked
/// or shared in (KitPlugin.kt puts them under the same cache folder).
fn cache_dir(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    use tauri::Manager;
    Ok(app.path().app_cache_dir()?)
}

/// A file in the app's cache, as the page names it. Anything else is refused.
fn cached_file(app: &AppHandle, path: &str) -> AppResult<std::path::PathBuf> {
    let path = std::path::PathBuf::from(path);
    let cache = cache_dir(app)?;
    let inside = path.canonicalize().ok().zip(cache.canonicalize().ok()).is_some_and(|(p, c)| p.starts_with(c));
    if !inside || !path.is_file() {
        return Err(AppError::Invalid("That file isn't one this app made.".into()));
    }
    Ok(path)
}

/// Saves a file the page made or was given (a picture shrunk for the chat, a photo or video picked on desktop) in the
/// cache, for `send_media`. Returns where it is.
#[tauri::command]
pub async fn stage_file(app: AppHandle, request: Request<'_>) -> AppResult<String> {
    let bytes = body_bytes(&request)?;
    if bytes.len() as u64 > kit_protocol::MAX_ORIGINAL_BYTES {
        return Err(AppError::Invalid("That's too big to send.".into()));
    }
    let dir = cache_dir(&app)?.join("outgoing");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(ulid::Ulid::new().to_string());
    tokio::fs::write(&path, &bytes).await?;
    Ok(path.to_string_lossy().into_owned())
}

/// A photo or video to send, with the small picture the chat shows for it.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingMedia {
    /// "image" or "video".
    pub kind: String,
    /// The file as it was taken (in the app's cache).
    pub path: String,
    /// A JPEG at most 1920 pixels across: the photo shrunk, or a still from the video.
    pub preview_path: Option<String>,
    /// The size of the picture or video as it's shown (rotation applied).
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub caption: String,
}

/// Moves a file into the media folder as `local-<ulid>`, ready to upload.
async fn adopt(engine: &Engine, from: &std::path::Path) -> AppResult<(String, std::path::PathBuf)> {
    let local = format!("local-{}", ulid::Ulid::new());
    let to = engine.media_dir.join(&local);
    if tokio::fs::rename(from, &to).await.is_err() {
        tokio::fs::copy(from, &to).await?;
        let _ = tokio::fs::remove_file(from).await;
    }
    Ok((local, to))
}

async fn sniff_file(path: &std::path::Path) -> AppResult<&'static str> {
    use tokio::io::AsyncReadExt;
    let mut head = [0u8; 16];
    let n = tokio::fs::File::open(path).await?.read(&mut head).await?;
    Ok(kit_protocol::media::sniff(&head[..n]).unwrap_or_default())
}

/// Photos and videos at their full size. The hidden data that says where they were taken comes out here, before
/// anything leaves the device. Each becomes a message in the outbox, sent once its files are uploaded.
#[tauri::command]
pub async fn send_media(
    app: AppHandle,
    engine: Eng<'_>,
    chat_id: String,
    items: Vec<OutgoingMedia>,
) -> AppResult<Vec<UiMessage>> {
    use kit_protocol::media;
    let mut sent = Vec::new();
    for item in items {
        let original = cached_file(&app, &item.path)?;
        let preview = item.preview_path.as_deref().map(|p| cached_file(&app, p)).transpose()?;
        let size = tokio::fs::metadata(&original).await?.len();
        if size > kit_protocol::MAX_ORIGINAL_BYTES {
            return Err(AppError::Invalid("That's too big to send (the most is 1 GB). Try a shorter video.".into()));
        }
        let kind = sniff_file(&original).await?;
        let caption = item.caption.trim().to_string();
        let (width, height) = (item.width.max(1), item.height.max(1));
        let body = if item.kind == "video" {
            if !media::is_video(kind) {
                return Err(AppError::Invalid("Only MP4 videos can be sent.".into()));
            }
            let (video_id, video_path) = adopt(&engine, &original).await?;
            let path = video_path.clone();
            let cleaned = tokio::task::spawn_blocking(move || media::clean_video_file(&path))
                .await
                .map_err(|e| AppError::Other(e.to_string()))?;
            if let Err(e) = cleaned {
                let _ = tokio::fs::remove_file(&video_path).await;
                return Err(AppError::Invalid(format!("That video can't be read ({e}).")));
            }
            let poster = match &preview {
                Some(p) => Some(adopt(&engine, p).await?.0),
                None => None,
            };
            MessageBody::Video {
                video_media_id: video_id,
                poster_media_id: poster,
                width,
                height,
                duration_ms: item.duration_ms,
                bytes: size,
                caption,
            }
        } else {
            // A GIF stays as it is (shrinking it would stop it moving), as does any small picture without a preview.
            let as_is = kind == "image/gif" || (preview.is_none() && media::is_shown_picture(kind));
            if as_is && size as usize > kit_protocol::MAX_MEDIA_BYTES {
                return Err(AppError::Invalid("That picture is too big.".into()));
            }
            let shown = match (&preview, as_is) {
                (_, true) => &original,
                (Some(p), false) => p,
                (None, false) => return Err(AppError::Invalid("That picture can't be opened.".into())),
            };
            let bytes = media::clean_photo(&tokio::fs::read(shown).await?);
            let shown_id = format!("local-{}", ulid::Ulid::new());
            tokio::fs::write(engine.media_dir.join(&shown_id), &bytes).await?;
            let _ = tokio::fs::remove_file(shown).await;
            // The full photo too, when the app knows how to take its hidden data out (JPEG, PNG).
            let mut full = None;
            if !as_is && media::is_full_photo(kind) {
                let bytes = media::clean_photo(&tokio::fs::read(&original).await?);
                let full_id = format!("local-{}", ulid::Ulid::new());
                tokio::fs::write(engine.media_dir.join(&full_id), &bytes).await?;
                full = Some((full_id, bytes.len() as u64));
            }
            let _ = tokio::fs::remove_file(&original).await;
            MessageBody::Image {
                media_id: shown_id,
                width,
                height,
                caption,
                full_bytes: full.as_ref().map(|f| f.1).unwrap_or(0),
                full_media_id: full.map(|f| f.0),
            }
        };
        let first = body.clone().media_ids_mut().first().map(|id| id.to_string());
        sent.push(queue(&engine, chat_id.clone(), body, first).await?);
    }
    Ok(sent)
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
    if let Some(mut entry) = engine.store.outbox_entry(&id).await? {
        for local in entry.body.media_ids_mut().into_iter().filter(|id| crate::engine::is_local(id)) {
            let _ = tokio::fs::remove_file(engine.media_dir.join(local.as_str())).await;
        }
    }
    engine.store.remove_outbox(&id).await
}

/// Where a photo or video is on this device (for the page to show through the asset protocol), fetching it from the
/// hub first when it isn't here yet. `kit://download` events say how that's going.
#[tauri::command]
pub async fn media_file(engine: Eng<'_>, id: String) -> AppResult<String> {
    check_media_id(&id)?;
    Ok(engine.media_file(&id).await?.to_string_lossy().into_owned())
}

fn check_media_id(id: &str) -> AppResult<()> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(AppError::Invalid("bad picture ID".into()));
    }
    Ok(())
}

/// The file name a saved photo or video gets: from its ID, so saving the same one twice doesn't make a copy.
fn saved_name(id: &str, kind: &str) -> Option<String> {
    let ext = match kind {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/heic" => "heic",
        "image/avif" => "avif",
        "video/mp4" => "mp4",
        "video/quicktime" => "mov",
        "video/webm" => "webm",
        _ => return None,
    };
    Some(format!("KitChat-{}.{ext}", id.trim_start_matches("local-")))
}

/// Where `save_media` put a photo or video: "downloads" (a computer's Downloads folder) or "gallery" (Android).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub place: &'static str,
    pub name: String,
}

/// Saves a photo or video from a chat where the person can find it outside KitChat: the Downloads folder on a
/// computer, the gallery (Pictures/KitChat or Movies/KitChat) on Android. Downloads it from the hub first if needed.
#[tauri::command]
pub async fn save_media(app: AppHandle, engine: Eng<'_>, id: String) -> AppResult<Saved> {
    check_media_id(&id)?;
    let from = engine.media_file(&id).await?;
    let kind = sniff_file(&from).await?;
    let name = saved_name(&id, kind).ok_or_else(|| AppError::Invalid("That file can't be saved.".into()))?;
    #[cfg(target_os = "android")]
    {
        let _ = &engine;
        platform::save_to_gallery(&app, &from, kind, &name).await.map_err(AppError::Other)?;
        Ok(Saved { place: "gallery", name })
    }
    #[cfg(not(target_os = "android"))]
    {
        use tauri::Manager;
        let dir = app.path().download_dir().map_err(|_| AppError::Other("There's no Downloads folder.".into()))?;
        tokio::fs::create_dir_all(&dir).await?;
        let to = dir.join(&name);
        if !tokio::fs::try_exists(&to).await.unwrap_or(false) {
            // Copied next to it first, so a half-copied video never sits in Downloads under its real name.
            let part = dir.join(format!("{name}.part"));
            if let Err(e) = tokio::fs::copy(&from, &part).await {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(e.into());
            }
            tokio::fs::rename(&part, &to).await?;
        }
        log::info!("saved {id} to {}", to.display());
        Ok(Saved { place: "downloads", name })
    }
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
pub async fn take_shared(app: AppHandle) -> platform::Incoming {
    platform::take_shared(&app).await
}

/// Android's photo picker. `null` on desktop, where the page uses a file input.
#[tauri::command]
pub async fn pick_media(app: AppHandle) -> AppResult<Option<Vec<SharedItem>>> {
    platform::pick_media(&app).await.transpose().map_err(AppError::Other)
}


/// The unread count, for the taskbar button and the tray. Nothing on Android (the notifications say it there).
#[tauri::command]
pub fn set_unread(app: AppHandle, count: u32) {
    #[cfg(desktop)]
    crate::desktop::set_unread(&app, count);
    let _ = (app, count);
}

/// Whether KitChat starts when the person signs in to the computer. Always false on Android.
#[tauri::command]
pub fn autostart_enabled(app: AppHandle) -> bool {
    #[cfg(desktop)]
    return crate::desktop::autostart_enabled(&app);
    #[cfg(not(desktop))]
    {
        let _ = app;
        false
    }
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, on: bool) -> AppResult<bool> {
    #[cfg(desktop)]
    {
        crate::desktop::set_autostart(&app, on).map_err(AppError::Other)?;
        Ok(crate::desktop::autostart_enabled(&app))
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, on);
        Err(AppError::Invalid("Starting with the computer is only for the desktop app.".into()))
    }
}

/// The links and lists sent in a chat, newest first, for the Links & lists panel.
#[tauri::command]
pub async fn chat_shelf(engine: Eng<'_>, chat_id: String, before: Option<String>) -> AppResult<Vec<UiMessage>> {
    engine.store.shelf(&chat_id, before.as_deref(), 100).await
}

/// The family's hub has a new address. Returns the address as it'll be used.
#[tauri::command]
pub async fn change_hub(app: AppHandle, engine: Eng<'_>, url: String) -> AppResult<String> {
    let url = normalize_url(&url)?;
    allow_home_network(&app, &url).await?;
    engine.change_hub(&url).await
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

#[cfg(test)]
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
    fn picture_bytes_from_json() {
        assert_eq!(picture_from_json(&serde_json::json!([255, 216, 0])), Some(vec![255, 216, 0]));
        assert_eq!(picture_from_json(&serde_json::json!({"1": 216, "0": 255})), Some(vec![255, 216]));
        assert_eq!(picture_from_json(&serde_json::json!([256])), None);
        assert_eq!(picture_from_json(&serde_json::json!({"5": 1})), None);
        assert_eq!(picture_from_json(&serde_json::json!("abc")), None);
    }

    #[test]
    fn saved_names() {
        assert_eq!(saved_name("01ABC", "image/jpeg").as_deref(), Some("KitChat-01ABC.jpg"));
        assert_eq!(saved_name("local-01ABC", "video/mp4").as_deref(), Some("KitChat-01ABC.mp4"));
        assert_eq!(saved_name("01ABC", ""), None);
    }

    #[test]
    fn percent_round_trip() {
        let s = "Sam's 🦊 picture: 100%";
        assert_eq!(percent_decode(&percent_encode(s)), s);
        assert_eq!(percent_encode("http://a.b:1"), "http%3A%2F%2Fa.b%3A1");
    }
}
