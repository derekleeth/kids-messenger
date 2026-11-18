//! This device's SQLite copy of the family's chats, messages and notes, and its outbox.

use std::path::Path;
use std::str::FromStr;

use kit_protocol::notes::Note;
use kit_protocol::{Chat, Member, Message, MessageBody};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

use crate::error::AppResult;

/// How many messages per chat the page gets at once; older ones load as it scrolls back.
pub const PAGE: i64 = 60;

/// A message as the page shows it: sent, or still waiting in the outbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiMessage {
    #[serde(flatten)]
    pub message: Message,
    pub pending: bool,
    pub error: Option<String>,
}

impl From<Message> for UiMessage {
    fn from(message: Message) -> Self {
        Self { message, pending: false, error: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub hub_url: String,
    pub device_id: String,
    pub me: Member,
    pub family_name: String,
}

pub struct OutboxEntry {
    pub id: String,
    pub chat_id: String,
    pub body: MessageBody,
    pub upload: Option<String>,
    pub created_at: i64,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct Store {
    pub pool: SqlitePool,
}

/// A message the Links & lists panel shows: a link, or a shared list or note.
fn on_shelf(message: &Message) -> bool {
    match &message.body {
        MessageBody::Text { text } => !kit_protocol::links::find_all(text).is_empty(),
        MessageBody::Note { .. } => true,
        _ => false,
    }
}

impl Store {
    pub async fn open(path: &Path) -> AppResult<Self> {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new().max_connections(4).connect_with(options).await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    #[cfg(test)]
    pub async fn memory() -> AppResult<Self> {
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn get(&self, key: &str) -> AppResult<Option<String>> {
        Ok(sqlx::query_scalar("SELECT value FROM kv WHERE key = ?").bind(key).fetch_optional(&self.pool).await?)
    }

    pub async fn set(&self, key: &str, value: &str) -> AppResult<()> {
        sqlx::query("INSERT INTO kv (key, value) VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET value = excluded.value")
            .bind(key)
            .bind(value)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_json<T: for<'de> Deserialize<'de>>(&self, key: &str) -> AppResult<Option<T>> {
        match self.get(key).await? {
            Some(v) => Ok(serde_json::from_str(&v).ok()),
            None => Ok(None),
        }
    }

    pub async fn set_json<T: Serialize>(&self, key: &str, value: &T) -> AppResult<()> {
        self.set(key, &serde_json::to_string(value)?).await
    }

    pub async fn account(&self) -> AppResult<Option<Account>> {
        self.get_json("account").await
    }

    pub async fn rev(&self) -> AppResult<i64> {
        Ok(self.get("rev").await?.and_then(|r| r.parse().ok()).unwrap_or(0))
    }

    /// Moves the sync position forward (never back).
    pub async fn advance_rev(&self, rev: i64) -> AppResult<()> {
        if rev > self.rev().await? {
            self.set("rev", &rev.to_string()).await?;
        }
        Ok(())
    }

    pub async fn members(&self) -> AppResult<Vec<Member>> {
        Ok(self.get_json("members").await?.unwrap_or_default())
    }

    /// Forgets everything: signing out, or the device was removed.
    pub async fn clear(&self) -> AppResult<()> {
        let mut tx = self.pool.begin().await?;
        for table in ["kv", "chats", "messages", "outbox", "notes"] {
            sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}"))).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn put_chat(&self, chat: &Chat) -> AppResult<()> {
        sqlx::query("INSERT INTO chats (id, json) VALUES (?, ?) ON CONFLICT (id) DO UPDATE SET json = excluded.json")
            .bind(&chat.id)
            .bind(serde_json::to_string(chat)?)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn chats(&self) -> AppResult<Vec<Chat>> {
        let rows: Vec<String> = sqlx::query_scalar("SELECT json FROM chats").fetch_all(&self.pool).await?;
        Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    /// Stores a message from the hub. Returns whether it's new to this device (so it may deserve a notification).
    pub async fn put_message(&self, message: &Message) -> AppResult<bool> {
        let existed: Option<i64> = sqlx::query_scalar("SELECT 1 FROM messages WHERE id = ?")
            .bind(&message.id)
            .fetch_optional(&self.pool)
            .await?;
        sqlx::query(
            "INSERT INTO messages (id, chat_id, created_at, json) VALUES (?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET json = excluded.json, created_at = excluded.created_at",
        )
        .bind(&message.id)
        .bind(&message.chat_id)
        .bind(message.created_at)
        .bind(serde_json::to_string(message)?)
        .execute(&self.pool)
        .await?;
        Ok(existed.is_none())
    }

    /// The newest messages of a chat (before `before`, when given), oldest first, outbox included at the end.
    pub async fn messages(&self, chat_id: &str, before: Option<&str>, limit: i64) -> AppResult<Vec<UiMessage>> {
        let cursor: Option<(i64, String)> = match before {
            Some(id) => {
                sqlx::query_as("SELECT created_at, id FROM messages WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await?
            }
            None => None,
        };
        let (at, id) = cursor.unwrap_or((i64::MAX, "\u{10FFFF}".into()));
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT json FROM messages WHERE chat_id = ? AND (created_at < ? OR (created_at = ? AND id < ?))
             ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(chat_id)
        .bind(at)
        .bind(at)
        .bind(id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        let mut out: Vec<UiMessage> =
            rows.iter().rev().filter_map(|j| serde_json::from_str::<Message>(j).ok()).map(UiMessage::from).collect();
        if before.is_none() {
            out.extend(self.outbox_messages(Some(chat_id), "").await?);
        }
        Ok(out)
    }

    /// The links and lists sent in a chat, newest first (before `before`, when given): text messages with a web
    /// link in them, and shared notes. Only what's on this device; older messages arrive as the chat is scrolled.
    pub async fn shelf(&self, chat_id: &str, before: Option<&str>, limit: usize) -> AppResult<Vec<UiMessage>> {
        let cursor: Option<(i64, String)> = match before {
            Some(id) => {
                sqlx::query_as("SELECT created_at, id FROM messages WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await?
            }
            None => None,
        };
        let (at, id) = cursor.unwrap_or((i64::MAX, "\u{10FFFF}".into()));
        // A cheap first pass in SQL; the exact rules (kit_protocol::links) below.
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT json FROM messages WHERE chat_id = ? AND (created_at < ? OR (created_at = ? AND id < ?))
               AND (json LIKE '%http%' OR json LIKE '%www.%' OR json LIKE '%\"note\"%')
             ORDER BY created_at DESC, id DESC",
        )
        .bind(chat_id)
        .bind(at)
        .bind(at)
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .filter_map(|j| serde_json::from_str::<Message>(j).ok())
            .filter(on_shelf)
            .take(limit)
            .map(UiMessage::from)
            .collect())
    }

    pub async fn add_outbox(&self, entry: &OutboxEntry) -> AppResult<()> {
        sqlx::query("INSERT INTO outbox (id, chat_id, body_json, upload, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(&entry.id)
            .bind(&entry.chat_id)
            .bind(serde_json::to_string(&entry.body)?)
            .bind(&entry.upload)
            .bind(entry.created_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn outbox(&self) -> AppResult<Vec<OutboxEntry>> {
        // id, chat_id, body_json, upload, created_at, error
        type Row = (String, String, String, Option<String>, i64, Option<String>);
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT id, chat_id, body_json, upload, created_at, error FROM outbox ORDER BY created_at, id",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|(id, chat_id, body, upload, created_at, error)| {
                Some(OutboxEntry { id, chat_id, body: serde_json::from_str(&body).ok()?, upload, created_at, error })
            })
            .collect())
    }

    /// Outbox entries as messages from `me`, for showing.
    pub async fn outbox_messages(&self, chat_id: Option<&str>, me: &str) -> AppResult<Vec<UiMessage>> {
        let me = if me.is_empty() { self.account().await?.map(|a| a.me.id).unwrap_or_default() } else { me.into() };
        Ok(self
            .outbox()
            .await?
            .into_iter()
            .filter(|e| chat_id.is_none_or(|c| c == e.chat_id))
            .map(|e| outbox_message(&e, &me))
            .collect())
    }

    pub async fn outbox_entry(&self, id: &str) -> AppResult<Option<OutboxEntry>> {
        Ok(self.outbox().await?.into_iter().find(|e| e.id == id))
    }

    pub async fn uploaded(&self, id: &str, body: &MessageBody) -> AppResult<()> {
        sqlx::query("UPDATE outbox SET body_json = ?, upload = NULL WHERE id = ?")
            .bind(serde_json::to_string(body)?)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn outbox_failed(&self, id: &str, error: Option<&str>) -> AppResult<()> {
        sqlx::query("UPDATE outbox SET error = ? WHERE id = ?").bind(error).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn remove_outbox(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM outbox WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn put_note(&self, note: &Note, dirty: bool) -> AppResult<()> {
        sqlx::query(
            "INSERT INTO notes (id, json, dirty, deleted) VALUES (?, ?, ?, 0)
             ON CONFLICT (id) DO UPDATE SET json = excluded.json, dirty = excluded.dirty, deleted = 0",
        )
        .bind(&note.id)
        .bind(serde_json::to_string(note)?)
        .bind(dirty)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// A note from the hub. A local edit not yet sent wins for now; sending it merges the two on the hub.
    pub async fn put_remote_note(&self, note: &Note) -> AppResult<bool> {
        let dirty: Option<i64> = sqlx::query_scalar("SELECT dirty FROM notes WHERE id = ?")
            .bind(&note.id)
            .fetch_optional(&self.pool)
            .await?;
        if dirty == Some(1) {
            return Ok(false);
        }
        self.put_note(note, false).await?;
        Ok(true)
    }

    pub async fn note(&self, id: &str) -> AppResult<Option<(Note, bool, bool)>> {
        let row: Option<(String, i64, i64)> = sqlx::query_as("SELECT json, dirty, deleted FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.and_then(|(j, dirty, deleted)| Some((serde_json::from_str(&j).ok()?, dirty != 0, deleted != 0))))
    }

    pub async fn notes(&self) -> AppResult<Vec<Note>> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT json FROM notes WHERE deleted = 0").fetch_all(&self.pool).await?;
        Ok(rows.iter().filter_map(|j| serde_json::from_str(j).ok()).collect())
    }

    /// Notes changed or deleted here that the hub doesn't have yet: (note, deleted).
    pub async fn dirty_notes(&self) -> AppResult<Vec<(Note, bool)>> {
        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT json, deleted FROM notes WHERE dirty = 1").fetch_all(&self.pool).await?;
        Ok(rows.into_iter().filter_map(|(j, d)| Some((serde_json::from_str(&j).ok()?, d != 0))).collect())
    }

    /// The hub took this version: clean, unless it was edited again meanwhile.
    pub async fn note_saved(&self, sent: &Note, merged: &Note) -> AppResult<()> {
        let current = self.note(&sent.id).await?;
        let edited_since = current.as_ref().is_some_and(|(n, _, _)| n != sent);
        if !edited_since {
            self.put_note(merged, false).await?;
        }
        Ok(())
    }

    pub async fn mark_note_deleted(&self, id: &str) -> AppResult<()> {
        sqlx::query("UPDATE notes SET deleted = 1, dirty = 1 WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn remove_note(&self, id: &str) -> AppResult<()> {
        sqlx::query("DELETE FROM notes WHERE id = ?").bind(id).execute(&self.pool).await?;
        Ok(())
    }
}

pub fn outbox_message(e: &OutboxEntry, me: &str) -> UiMessage {
    UiMessage {
        message: Message {
            id: e.id.clone(),
            chat_id: e.chat_id.clone(),
            sender_id: me.into(),
            body: e.body.clone(),
            created_at: e.created_at,
            reactions: Vec::new(),
            link: None,
            rev: 0,
        },
        pending: true,
        error: e.error.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kit_protocol::notes::{Block, BlockKind};

    fn msg(id: &str, at: i64) -> Message {
        Message {
            id: id.into(),
            chat_id: "c".into(),
            sender_id: "s".into(),
            body: MessageBody::Text { text: id.into() },
            created_at: at,
            reactions: vec![],
            link: None,
            rev: 1,
        }
    }

    #[tokio::test]
    async fn messages_page_backwards_with_the_outbox_last() {
        let store = Store::memory().await.unwrap();
        for (i, id) in ["a", "b", "c", "d"].iter().enumerate() {
            assert!(store.put_message(&msg(id, i as i64)).await.unwrap());
        }
        assert!(!store.put_message(&msg("a", 0)).await.unwrap());
        store
            .add_outbox(&OutboxEntry {
                id: "z".into(),
                chat_id: "c".into(),
                body: MessageBody::Text { text: "hi".into() },
                upload: None,
                created_at: 9,
                error: None,
            })
            .await
            .unwrap();
        let page = store.messages("c", None, 2).await.unwrap();
        let ids: Vec<_> = page.iter().map(|m| m.message.id.as_str()).collect();
        assert_eq!(ids, ["c", "d", "z"]);
        assert!(page[2].pending);
        let older = store.messages("c", Some("c"), 2).await.unwrap();
        let ids: Vec<_> = older.iter().map(|m| m.message.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[tokio::test]
    async fn shelf_has_links_and_lists_newest_first() {
        let store = Store::memory().await.unwrap();
        let text = |id: &str, at: i64, text: &str| Message { body: MessageBody::Text { text: text.into() }, ..msg(id, at) };
        store.put_message(&text("a", 1, "look www.example.com")).await.unwrap();
        store.put_message(&text("b", 2, "http is a protocol")).await.unwrap();
        store.put_message(&text("c", 3, "no links here")).await.unwrap();
        let shared = MessageBody::Note {
            note_id: "n".into(),
            title: "Cool sites".into(),
            emoji: "📝".into(),
            preview: vec![],
            shared: true,
        };
        store.put_message(&Message { body: shared, ..msg("d", 4) }).await.unwrap();
        store.put_message(&text("e", 5, "https://kitchat.example/x and more")).await.unwrap();
        let ids = |v: Vec<UiMessage>| v.into_iter().map(|m| m.message.id).collect::<Vec<_>>();
        assert_eq!(ids(store.shelf("c", None, 10).await.unwrap()), ["e", "d", "a"]);
        assert_eq!(ids(store.shelf("c", None, 2).await.unwrap()), ["e", "d"]);
        assert_eq!(ids(store.shelf("c", Some("d"), 10).await.unwrap()), ["a"]);
        assert!(store.shelf("other", None, 10).await.unwrap().is_empty());
    }

    fn note(text: &str) -> Note {
        Note {
            id: "n".into(),
            owner_id: "o".into(),
            title: "t".into(),
            emoji: "📝".into(),
            color: "sun".into(),
            shared: false,
            blocks: vec![Block {
                id: "b".into(),
                kind: BlockKind::Text,
                text: text.into(),
                checked: false,
                url: String::new(),
                pos: 1.0,
                updated_at: 1,
                deleted: false,
                edited_by: String::new(),
            }],
            created_at: 1,
            meta_updated_at: 1,
            rev: 0,
        }
    }

    #[tokio::test]
    async fn local_edits_are_not_overwritten_before_they_are_sent() {
        let store = Store::memory().await.unwrap();
        store.put_note(&note("mine"), true).await.unwrap();
        assert!(!store.put_remote_note(&note("theirs")).await.unwrap());
        assert_eq!(store.note("n").await.unwrap().unwrap().0.blocks[0].text, "mine");

        // Saved, but edited again meanwhile: still dirty.
        let sent = note("mine");
        store.put_note(&note("mine again"), true).await.unwrap();
        store.note_saved(&sent, &note("merged")).await.unwrap();
        assert_eq!(store.dirty_notes().await.unwrap().len(), 1);

        let sent = note("mine again");
        store.note_saved(&sent, &note("merged")).await.unwrap();
        assert!(store.dirty_notes().await.unwrap().is_empty());
        assert_eq!(store.note("n").await.unwrap().unwrap().0.blocks[0].text, "merged");
    }

    #[tokio::test]
    async fn rev_only_moves_forward() {
        let store = Store::memory().await.unwrap();
        store.advance_rev(5).await.unwrap();
        store.advance_rev(3).await.unwrap();
        assert_eq!(store.rev().await.unwrap(), 5);
    }
}
