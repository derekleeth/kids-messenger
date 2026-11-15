//! Chats and messages: starting a chat with one person, sending, reading history, reactions and "seen".

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use kit_protocol::{Chat, Event, MarkRead, Message, MessageBody, React, STICKERS, SendMessage};
use serde::Deserialize;
use sqlx::SqliteConnection;

use super::check_id;
use crate::auth::Authed;
use crate::error::{ApiError, ApiResult};
use crate::live::{self, Audience};
use crate::{Hub, db, now_ms};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectRequest {
    member_id: String,
}

pub async fn direct(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Json(req): Json<DirectRequest>,
) -> ApiResult<Json<Chat>> {
    let me = &authed.member.id;
    if &req.member_id == me {
        return Err(ApiError::bad("That's you!"));
    }
    let mut tx = hub.pool.begin().await?;
    if db::member(&mut tx, &req.member_id).await?.is_none() {
        return Err(ApiError::NotFound("No such person.".into()));
    }
    let mut pair = [me.clone(), req.member_id.clone()];
    pair.sort();
    let key = pair.join(":");
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM chats WHERE direct_key = ?").bind(&key).fetch_optional(&mut *tx).await?;
    if let Some(id) = existing {
        let chat = db::chat(&mut tx, &id).await?.ok_or_else(|| ApiError::Internal("chat vanished".into()))?;
        return Ok(Json(chat));
    }
    let rev = db::next_rev(&mut tx).await?;
    let id = ulid::Ulid::new().to_string();
    sqlx::query("INSERT INTO chats (id, kind, direct_key, created_at, rev) VALUES (?, 'direct', ?, ?, ?)")
        .bind(&id)
        .bind(&key)
        .bind(now_ms())
        .bind(rev)
        .execute(&mut *tx)
        .await?;
    for member in &pair {
        sqlx::query("INSERT INTO chat_members (chat_id, member_id) VALUES (?, ?)")
            .bind(&id)
            .bind(member)
            .execute(&mut *tx)
            .await?;
    }
    let chat = db::chat(&mut tx, &id).await?.ok_or_else(|| ApiError::Internal("chat vanished".into()))?;
    tx.commit().await?;
    live::send(&hub, Audience::Members(pair.to_vec()), Event::Chat { rev, chat: chat.clone() });
    Ok(Json(chat))
}

async fn require_chat(conn: &mut SqliteConnection, chat_id: &str, authed: &Authed) -> ApiResult<Chat> {
    let chat = db::chat(conn, chat_id).await?.ok_or_else(|| ApiError::NotFound("No such chat.".into()))?;
    if !chat.member_ids.contains(&authed.member.id) {
        return Err(ApiError::Forbidden("You're not in that chat.".into()));
    }
    Ok(chat)
}

#[derive(Deserialize)]
pub struct HistoryQuery {
    before: Option<String>,
    limit: Option<i64>,
}

/// Older messages, newest last: up to `limit` before the message `before` (or the newest, without it).
pub async fn history(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(chat_id): Path<String>,
    Query(q): Query<HistoryQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    let mut conn = hub.pool.acquire().await?;
    require_chat(&mut conn, &chat_id, &authed).await?;
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let cursor: Option<(i64, String)> = match &q.before {
        Some(before) => {
            sqlx::query_as("SELECT created_at, id FROM messages WHERE id = ? AND chat_id = ?")
                .bind(before)
                .bind(&chat_id)
                .fetch_optional(&mut *conn)
                .await?
        }
        None => None,
    };
    let (at, id) = cursor.unwrap_or((i64::MAX, String::new()));
    let rows: Vec<db::MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM messages WHERE chat_id = ? AND (created_at < ? OR (created_at = ? AND id < ?))
         ORDER BY created_at DESC, id DESC LIMIT ?",
        db::MESSAGE_COLUMNS
    )))
    .bind(&chat_id)
    .bind(at)
    .bind(at)
    .bind(if id.is_empty() { "\u{10FFFF}".to_string() } else { id })
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    let mut messages = db::messages_from(&mut conn, rows).await?;
    messages.reverse();
    Ok(Json(messages))
}

/// Checks a message before it's stored, and fills in what the hub decides (a shared note's card).
async fn prepare_body(conn: &mut SqliteConnection, authed: &Authed, body: MessageBody) -> ApiResult<MessageBody> {
    match body {
        MessageBody::Text { text } => {
            let text = text.trim().to_string();
            if text.is_empty() {
                return Err(ApiError::bad("Type something first!"));
            }
            if text.chars().count() > kit_protocol::MAX_TEXT_CHARS {
                return Err(ApiError::bad("That message is too long. Try splitting it up."));
            }
            Ok(MessageBody::Text { text })
        }
        MessageBody::Image { media_id, width, height, caption } => {
            let exists: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE id = ?").bind(&media_id).fetch_one(conn).await?;
            if exists == 0 {
                return Err(ApiError::bad("That picture didn't finish uploading. Try again."));
            }
            let caption: String = caption.trim().chars().take(500).collect();
            Ok(MessageBody::Image { media_id, width: width.min(20_000), height: height.min(20_000), caption })
        }
        MessageBody::Sticker { sticker } => {
            if !STICKERS.contains(&sticker.as_str()) {
                return Err(ApiError::bad("We don't have that sticker."));
            }
            Ok(MessageBody::Sticker { sticker })
        }
        MessageBody::Note { note_id, .. } => {
            let (note, deleted) =
                db::note(conn, &note_id).await?.ok_or_else(|| ApiError::NotFound("No such note.".into()))?;
            if deleted || (note.owner_id != authed.member.id && !note.shared) {
                return Err(ApiError::NotFound("No such note.".into()));
            }
            Ok(MessageBody::Note {
                note_id,
                title: note.title.clone(),
                emoji: note.emoji.clone(),
                preview: note.preview(6),
                shared: note.shared,
            })
        }
    }
}

pub async fn send(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(chat_id): Path<String>,
    Json(req): Json<SendMessage>,
) -> ApiResult<Json<Message>> {
    check_id(&req.id)?;
    let mut tx = hub.pool.begin().await?;
    let chat = require_chat(&mut tx, &chat_id, &authed).await?;
    // A resend of a message already stored (the device didn't hear back the first time).
    if let Some(existing) = db::message(&mut tx, &req.id).await? {
        if existing.sender_id == authed.member.id && existing.chat_id == chat_id {
            return Ok(Json(existing));
        }
        return Err(ApiError::Conflict("That message ID is taken.".into()));
    }
    let body = prepare_body(&mut tx, &authed, req.body).await?;
    let rev = db::next_rev(&mut tx).await?;
    sqlx::query("INSERT INTO messages (id, chat_id, sender_id, body_json, created_at, rev) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(&req.id)
        .bind(&chat_id)
        .bind(&authed.member.id)
        .bind(serde_json::to_string(&body)?)
        .bind(now_ms())
        .bind(rev)
        .execute(&mut *tx)
        .await?;
    let message = db::message(&mut tx, &req.id).await?.ok_or_else(|| ApiError::Internal("message vanished".into()))?;
    tx.commit().await?;
    live::send(&hub, Audience::Members(chat.member_ids.clone()), Event::Message { rev, message: message.clone() });

    if let MessageBody::Text { text } = &message.body
        && let Some(url) = kit_protocol::links::first(text)
        && hub.previews.is_some()
    {
        let hub = hub.clone();
        let (id, members) = (message.id.clone(), chat.member_ids);
        tokio::spawn(async move {
            if let Err(e) = add_preview(&hub, &id, &url, members).await {
                log::warn!("link preview failed: {e}");
            }
        });
    }
    Ok(Json(message))
}

async fn add_preview(hub: &Hub, message_id: &str, url: &str, members: Vec<String>) -> ApiResult<()> {
    let Some(fetcher) = &hub.previews else { return Ok(()) };
    let Some(fetched) = fetcher.fetch(url).await else { return Ok(()) };
    let mut preview = fetched.preview;
    if let Some((bytes, kind)) = fetched.image {
        preview.image_media_id = Some(super::media::store(hub, None, bytes, kind).await?);
    }
    let mut tx = hub.pool.begin().await?;
    let rev = db::next_rev(&mut tx).await?;
    sqlx::query("UPDATE messages SET link_json = ?, rev = ? WHERE id = ?")
        .bind(serde_json::to_string(&preview)?)
        .bind(rev)
        .bind(message_id)
        .execute(&mut *tx)
        .await?;
    let message = db::message(&mut tx, message_id).await?;
    tx.commit().await?;
    if let Some(message) = message {
        live::send(hub, Audience::Members(members), Event::Message { rev, message });
    }
    Ok(())
}

pub async fn read(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(chat_id): Path<String>,
    Json(req): Json<MarkRead>,
) -> ApiResult<Json<Chat>> {
    let mut tx = hub.pool.begin().await?;
    let chat = require_chat(&mut tx, &chat_id, &authed).await?;
    let in_chat: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM messages WHERE id = ? AND chat_id = ?")
        .bind(&req.message_id)
        .bind(&chat_id)
        .fetch_one(&mut *tx)
        .await?;
    if in_chat == 0 {
        return Err(ApiError::NotFound("No such message.".into()));
    }
    // Only ever forward: IDs sort by time.
    if chat.read_up_to.get(&authed.member.id).is_some_and(|r| r.as_str() >= req.message_id.as_str()) {
        return Ok(Json(chat));
    }
    let rev = db::next_rev(&mut tx).await?;
    sqlx::query("UPDATE chat_members SET read_up_to = ? WHERE chat_id = ? AND member_id = ?")
        .bind(&req.message_id)
        .bind(&chat_id)
        .bind(&authed.member.id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE chats SET rev = ? WHERE id = ?").bind(rev).bind(&chat_id).execute(&mut *tx).await?;
    let chat = db::chat(&mut tx, &chat_id).await?.ok_or_else(|| ApiError::Internal("chat vanished".into()))?;
    tx.commit().await?;
    live::send(&hub, Audience::Members(chat.member_ids.clone()), Event::Chat { rev, chat: chat.clone() });
    Ok(Json(chat))
}

pub async fn react(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(message_id): Path<String>,
    Json(req): Json<React>,
) -> ApiResult<Json<Message>> {
    let emoji = req.emoji.trim().to_string();
    if emoji.chars().count() > 16 {
        return Err(ApiError::bad("That's not one emoji."));
    }
    let mut tx = hub.pool.begin().await?;
    let message =
        db::message(&mut tx, &message_id).await?.ok_or_else(|| ApiError::NotFound("No such message.".into()))?;
    let chat = require_chat(&mut tx, &message.chat_id, &authed).await?;
    if emoji.is_empty() {
        sqlx::query("DELETE FROM reactions WHERE message_id = ? AND member_id = ?")
            .bind(&message_id)
            .bind(&authed.member.id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("INSERT OR REPLACE INTO reactions (message_id, member_id, emoji) VALUES (?, ?, ?)")
            .bind(&message_id)
            .bind(&authed.member.id)
            .bind(&emoji)
            .execute(&mut *tx)
            .await?;
    }
    let rev = db::next_rev(&mut tx).await?;
    sqlx::query("UPDATE messages SET rev = ? WHERE id = ?").bind(rev).bind(&message_id).execute(&mut *tx).await?;
    let message =
        db::message(&mut tx, &message_id).await?.ok_or_else(|| ApiError::Internal("message vanished".into()))?;
    tx.commit().await?;
    live::send(&hub, Audience::Members(chat.member_ids), Event::Message { rev, message: message.clone() });
    Ok(Json(message))
}
