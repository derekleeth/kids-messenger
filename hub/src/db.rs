//! The hub's SQLite database: opening it, and reading rows back as protocol types.

use std::collections::BTreeMap;
use std::path::Path;
use std::str::FromStr;

use kit_protocol::notes::Note;
use kit_protocol::{Chat, ChatKind, LinkPreview, Member, Message, MessageBody, Reaction, Role};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::{SqliteConnection, SqlitePool};

use crate::error::{ApiError, ApiResult};

pub async fn open(path: &Path) -> ApiResult<SqlitePool> {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new().max_connections(8).connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

/// An in-memory database, for tests.
pub async fn open_memory() -> ApiResult<SqlitePool> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);
    // One connection: each in-memory connection would be its own database.
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}

pub async fn family_name(pool: &SqlitePool) -> ApiResult<Option<String>> {
    Ok(sqlx::query_scalar("SELECT name FROM family WHERE id = 1").fetch_optional(pool).await?)
}

/// Takes the next change number. Call inside the transaction that makes the change.
pub async fn next_rev(conn: &mut SqliteConnection) -> ApiResult<i64> {
    Ok(sqlx::query_scalar("UPDATE family SET rev = rev + 1 WHERE id = 1 RETURNING rev").fetch_one(conn).await?)
}

pub async fn current_rev(pool: &SqlitePool) -> ApiResult<i64> {
    Ok(sqlx::query_scalar("SELECT rev FROM family WHERE id = 1").fetch_optional(pool).await?.unwrap_or(0))
}

pub fn role_str(role: Role) -> &'static str {
    match role {
        Role::Parent => "parent",
        Role::Kid => "kid",
    }
}

fn role_from(s: &str) -> Role {
    if s == "parent" { Role::Parent } else { Role::Kid }
}

type MemberRow = (String, String, String, String, String);

fn member_from((id, name, role, color, avatar): MemberRow) -> Member {
    Member { id, name, role: role_from(&role), color, avatar }
}

pub async fn members(conn: &mut SqliteConnection) -> ApiResult<Vec<Member>> {
    let rows: Vec<MemberRow> =
        sqlx::query_as("SELECT id, name, role, color, avatar FROM members ORDER BY created_at, id")
            .fetch_all(conn)
            .await?;
    Ok(rows.into_iter().map(member_from).collect())
}

pub async fn member(conn: &mut SqliteConnection, id: &str) -> ApiResult<Option<Member>> {
    let row: Option<MemberRow> = sqlx::query_as("SELECT id, name, role, color, avatar FROM members WHERE id = ?")
        .bind(id)
        .fetch_optional(conn)
        .await?;
    Ok(row.map(member_from))
}

pub async fn chat(conn: &mut SqliteConnection, id: &str) -> ApiResult<Option<Chat>> {
    let row: Option<(String, String, i64)> = sqlx::query_as("SELECT id, kind, created_at FROM chats WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    let Some((id, kind, created_at)) = row else { return Ok(None) };
    let rows: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT member_id, read_up_to FROM chat_members WHERE chat_id = ? ORDER BY member_id")
            .bind(&id)
            .fetch_all(conn)
            .await?;
    let member_ids = rows.iter().map(|(m, _)| m.clone()).collect();
    let read_up_to: BTreeMap<String, String> = rows.into_iter().filter_map(|(m, r)| Some((m, r?))).collect();
    let kind = if kind == "family" { ChatKind::Family } else { ChatKind::Direct };
    Ok(Some(Chat { id, kind, member_ids, read_up_to, created_at }))
}

/// The chats a member is in. With `since`, only those that changed after it.
pub async fn chats_of(conn: &mut SqliteConnection, member_id: &str, since: i64) -> ApiResult<Vec<Chat>> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT c.id FROM chats c JOIN chat_members m ON m.chat_id = c.id
         WHERE m.member_id = ? AND c.rev > ? ORDER BY c.created_at",
    )
    .bind(member_id)
    .bind(since)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(chat) = chat(conn, &id).await? {
            out.push(chat);
        }
    }
    Ok(out)
}

pub async fn is_chat_member(conn: &mut SqliteConnection, chat_id: &str, member_id: &str) -> ApiResult<bool> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_members WHERE chat_id = ? AND member_id = ?")
        .bind(chat_id)
        .bind(member_id)
        .fetch_one(conn)
        .await?;
    Ok(n > 0)
}

pub async fn family_chat_id(conn: &mut SqliteConnection) -> ApiResult<String> {
    sqlx::query_scalar("SELECT id FROM chats WHERE kind = 'family'")
        .fetch_optional(conn)
        .await?
        .ok_or_else(|| ApiError::Internal("the family chat is missing".into()))
}

pub type MessageRow = (String, String, String, String, Option<String>, i64, i64);

pub const MESSAGE_COLUMNS: &str = "id, chat_id, sender_id, body_json, link_json, created_at, rev";

pub async fn message_from(conn: &mut SqliteConnection, row: MessageRow) -> ApiResult<Message> {
    let (id, chat_id, sender_id, body_json, link_json, created_at, rev) = row;
    let body: MessageBody = serde_json::from_str(&body_json)?;
    let link: Option<LinkPreview> = link_json.as_deref().map(serde_json::from_str).transpose()?;
    let reactions: Vec<(String, String)> =
        sqlx::query_as("SELECT member_id, emoji FROM reactions WHERE message_id = ? ORDER BY member_id")
            .bind(&id)
            .fetch_all(conn)
            .await?;
    let reactions = reactions.into_iter().map(|(member_id, emoji)| Reaction { member_id, emoji }).collect();
    Ok(Message { id, chat_id, sender_id, body, created_at, reactions, link, rev })
}

pub async fn message(conn: &mut SqliteConnection, id: &str) -> ApiResult<Option<Message>> {
    let row: Option<MessageRow> =
        sqlx::query_as(sqlx::AssertSqlSafe(format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?")))
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    match row {
        Some(row) => Ok(Some(message_from(conn, row).await?)),
        None => Ok(None),
    }
}

pub async fn messages_from(conn: &mut SqliteConnection, rows: Vec<MessageRow>) -> ApiResult<Vec<Message>> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(message_from(conn, row).await?);
    }
    Ok(out)
}

pub async fn note(conn: &mut SqliteConnection, id: &str) -> ApiResult<Option<(Note, bool)>> {
    let row: Option<(String, i64, i64)> =
        sqlx::query_as("SELECT json, deleted, rev FROM notes WHERE id = ?").bind(id).fetch_optional(conn).await?;
    match row {
        Some((json, deleted, rev)) => {
            let mut note: Note = serde_json::from_str(&json)?;
            note.rev = rev;
            Ok(Some((note, deleted != 0)))
        }
        None => Ok(None),
    }
}
