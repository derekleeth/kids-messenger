//! `/sync?since=N`: everything a device's member can see that changed after change number `N`.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use kit_protocol::SyncResponse;
use kit_protocol::notes::Note;
use serde::Deserialize;

use crate::auth::Authed;
use crate::error::ApiResult;
use crate::{Hub, db};

/// Newest messages per chat on a first sync; older ones load as the chat is scrolled back.
const FIRST_SYNC_PER_CHAT: i64 = 100;
/// Most changed messages in one catch-up. A device that was away for longer than this gets the rest next time.
const CATCH_UP_LIMIT: i64 = 2000;

#[derive(Deserialize)]
pub struct SyncQuery {
    #[serde(default)]
    since: i64,
}

pub async fn sync(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Query(q): Query<SyncQuery>,
) -> ApiResult<Json<SyncResponse>> {
    let me = authed.member.id.clone();
    // Read first: a change made while this runs may be sent twice, never missed.
    let mut rev = db::current_rev(&hub.pool).await?;
    let family_name = db::family_name(&hub.pool).await?.unwrap_or_default();
    let mut conn = hub.pool.acquire().await?;
    let members = db::members(&mut conn).await?;
    let all_chats = db::chats_of(&mut conn, &me, 0).await?;
    let chats = if q.since == 0 { all_chats.clone() } else { db::chats_of(&mut conn, &me, q.since).await? };

    let mut messages = Vec::new();
    if q.since == 0 {
        for chat in &all_chats {
            let rows: Vec<db::MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
                "SELECT {} FROM messages WHERE chat_id = ? ORDER BY created_at DESC, id DESC LIMIT ?",
                db::MESSAGE_COLUMNS
            )))
            .bind(&chat.id)
            .bind(FIRST_SYNC_PER_CHAT)
            .fetch_all(&mut *conn)
            .await?;
            let mut batch = db::messages_from(&mut conn, rows).await?;
            batch.reverse();
            messages.extend(batch);
        }
    } else {
        let rows: Vec<db::MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {} FROM messages WHERE rev > ? AND chat_id IN
               (SELECT chat_id FROM chat_members WHERE member_id = ?)
             ORDER BY rev LIMIT ?",
            db::MESSAGE_COLUMNS
        )))
        .bind(q.since)
        .bind(&me)
        .bind(CATCH_UP_LIMIT)
        .fetch_all(&mut *conn)
        .await?;
        if rows.len() as i64 == CATCH_UP_LIMIT {
            // Only claim to be caught up to the last message actually sent.
            rev = rows.last().map(|r| r.6).unwrap_or(rev);
        }
        messages = db::messages_from(&mut conn, rows).await?;
    }

    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT json, rev FROM notes WHERE rev > ? AND deleted = 0 AND (owner_id = ? OR shared = 1) ORDER BY rev",
    )
    .bind(q.since)
    .bind(&me)
    .fetch_all(&mut *conn)
    .await?;
    let mut notes = Vec::with_capacity(rows.len());
    for (json, note_rev) in rows {
        let mut note: Note = serde_json::from_str(&json)?;
        note.rev = note_rev;
        notes.push(note);
    }
    let deleted_notes: Vec<String> = if q.since == 0 {
        Vec::new()
    } else {
        // Deleted ones this member could see, and lists that stopped being shared with them.
        sqlx::query_scalar(
            "SELECT id FROM notes WHERE rev > ? AND
               ((deleted = 1 AND (owner_id = ? OR shared = 1)) OR (deleted = 0 AND shared = 0 AND owner_id != ?))",
        )
        .bind(q.since)
        .bind(&me)
        .bind(&me)
        .fetch_all(&mut *conn)
        .await?
    };

    Ok(Json(SyncResponse {
        rev,
        hub_id: Some(hub.hub_id.clone()),
        me: Some(authed.member),
        family_name,
        members,
        chats,
        messages,
        notes,
        deleted_notes,
    }))
}
