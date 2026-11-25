//! Keeping a family's hubs in step (docs/multi_hub_plan.md): what changed on this hub since a change number, for
//! another hub, and taking in another hub's changes.
//!
//! Taking in changes follows rules that give the same result whatever order the changes come in and however many
//! times they come: messages, media, devices and chat members are added if missing; the newer edit of a member, a
//! reaction or a part of a note wins; the later "seen" wins; a removed device or a deleted note stays that way. Only a
//! row that actually changes takes a new change number, so when two hubs keep swapping changes, what one sends back
//! changes nothing and they settle.
//!
//! Each changed row takes a number of its own (not one for the whole batch), so devices catching up with `/sync` in
//! pages get every one of them.

use std::collections::{BTreeMap, BTreeSet};

use kit_protocol::hubs::{
    Changes, ChatMemberRecord, ChatRecord, DeviceRecord, FamilyRecord, MediaRecord, MemberRecord, MessageRecord,
    NoteRecord, ReactionRecord, newer,
};
use kit_protocol::notes::{self, Note};
use kit_protocol::{ChatKind, Event, Member};
use sqlx::SqliteConnection;

use crate::error::{ApiError, ApiResult};
use crate::live::{self, Audience};
use crate::{Hub, db};

/// Most rows of one kind in a batch.
pub const BATCH: i64 = 500;

/// What changed on this hub after change number `since`, at most `limit` rows of each kind. With `since = 0`, everything.
pub async fn changes_since(hub: &Hub, since: i64, limit: i64) -> ApiResult<Changes> {
    // Read first: a change made while this runs is sent next time, never missed.
    let current = db::current_rev(&hub.pool).await?;
    let mut conn = hub.pool.acquire().await?;
    let family = family(&mut conn).await?.ok_or_else(|| ApiError::Conflict("This hub isn't set up yet.".into()))?;

    let members = members(&mut conn).await?;
    let devices = devices_since(&mut conn, since, limit).await?;
    let chats = chats_since(&mut conn, since, limit).await?;
    let messages = messages_since(&mut conn, since, limit).await?;
    let notes = notes_since(&mut conn, since, limit).await?;
    let media = media_since(&mut conn, since, limit).await?;

    // A kind that filled its batch may have more after its last row: the batch only goes up to the lowest such row,
    // where every kind is complete.
    let mut rev = current;
    let mut more = false;
    let lasts = [last(&devices), last(&chats), last(&messages), last(&notes), last(&media)];
    for (count, last_rev) in lasts {
        if count as i64 >= limit
            && let Some(last_rev) = last_rev
        {
            rev = rev.min(last_rev);
            more = true;
        }
    }
    let devices = up_to(devices, rev);
    let mut chats = up_to(chats, rev);
    let messages = up_to(messages, rev);
    let notes = up_to(notes, rev);
    let media = up_to(media, rev);

    // A message's chat comes along even when the chat itself changed later (someone read it since), so the other hub
    // has it to put the message in.
    let have: BTreeSet<String> = chats.iter().map(|c| c.id.clone()).collect();
    let needed: BTreeSet<&str> = messages.iter().map(|m| m.chat_id.as_str()).filter(|id| !have.contains(*id)).collect();
    for id in needed {
        if let Some(chat) = chat(&mut conn, id).await? {
            chats.push(chat);
        }
    }

    Ok(Changes { hub_id: hub.hub_id.clone(), family, rev, more, members, devices, chats, messages, notes, media })
}

fn up_to<T>(rows: Vec<(i64, T)>, rev: i64) -> Vec<T> {
    rows.into_iter().filter(|(r, _)| *r <= rev).map(|(_, row)| row).collect()
}

fn last<T>(rows: &[(i64, T)]) -> (usize, Option<i64>) {
    (rows.len(), rows.last().map(|(r, _)| *r))
}

/// Takes in another hub's changes. A hub that isn't set up yet becomes one of that family's hubs; a hub of another
/// family refuses them. Returns how many rows changed here.
pub async fn apply(hub: &Hub, changes: Changes) -> ApiResult<usize> {
    if changes.hub_id == hub.hub_id {
        return Err(ApiError::bad("A hub can't take in its own changes."));
    }
    // Held while a hub that isn't set up becomes one, so setting it up with its code can't happen at the same time.
    let mut setup_code = hub.setup_code.lock().await;
    let mut tx = hub.pool.begin().await?;
    let mut out = Applier::default();

    match family(&mut tx).await? {
        Some(ours) if ours.family_id != changes.family.family_id => {
            return Err(ApiError::Conflict("That hub belongs to another family.".into()));
        }
        Some(_) => {}
        None => {
            let FamilyRecord { family_id, name, created_at } = &changes.family;
            sqlx::query("INSERT INTO family (id, name, rev, created_at, family_id) VALUES (1, ?, 0, ?, ?)")
                .bind(name)
                .bind(created_at)
                .bind(family_id)
                .execute(&mut *tx)
                .await?;
            log::info!("this hub is now one of the hubs of the family \"{name}\"");
        }
    }

    for member in &changes.members {
        out.member(&mut tx, member).await?;
    }
    for device in &changes.devices {
        out.device(&mut tx, device).await?;
    }
    for media in &changes.media {
        out.media(&mut tx, media).await?;
    }
    for chat in &changes.chats {
        out.chat(&mut tx, chat).await?;
    }
    for message in &changes.messages {
        out.message(&mut tx, message).await?;
    }
    for note in &changes.notes {
        out.note(&mut tx, note).await?;
    }
    let members_rev = out.members_rev;
    let members = if members_rev.is_some() { db::members(&mut tx).await? } else { Vec::new() };
    tx.commit().await?;

    if setup_code.take().is_some() {
        let _ = std::fs::remove_file(crate::setup_code_file(&hub.data_dir));
    }
    drop(setup_code);

    // The devices connected here hear about it as if it had happened here.
    if let Some(rev) = members_rev {
        live::send(hub, Audience::Everyone, Event::Members { rev, members });
    }
    for (audience, event) in out.events {
        live::send(hub, audience, event);
    }
    Ok(out.changed)
}

/// What taking in a batch changed, and what to tell the devices connected here.
#[derive(Default)]
struct Applier {
    changed: usize,
    /// The change number of the last member that changed: everyone gets the new list once.
    members_rev: Option<i64>,
    events: Vec<(Audience, Event)>,
}

impl Applier {
    async fn next_rev(&mut self, conn: &mut SqliteConnection) -> ApiResult<i64> {
        self.changed += 1;
        db::next_rev(conn).await
    }

    async fn member(&mut self, conn: &mut SqliteConnection, record: &MemberRecord) -> ApiResult<()> {
        let MemberRecord { member, created_at, updated_at } = record;
        let stored: Option<(String, String, String, i64)> =
            sqlx::query_as("SELECT name, color, avatar, updated_at FROM members WHERE id = ?")
                .bind(&member.id)
                .fetch_optional(&mut *conn)
                .await?;
        let incoming = (&member.name, &member.color, &member.avatar);
        match stored {
            None => {
                let rev = self.next_rev(conn).await?;
                sqlx::query(
                    "INSERT INTO members (id, name, role, color, avatar, created_at, updated_at, rev)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&member.id)
                .bind(&member.name)
                .bind(db::role_str(member.role))
                .bind(&member.color)
                .bind(&member.avatar)
                .bind(created_at)
                .bind(updated_at)
                .bind(rev)
                .execute(&mut *conn)
                .await?;
                self.members_rev = Some(rev);
            }
            Some((name, color, avatar, stored_at)) => {
                if newer(*updated_at, &incoming, stored_at, &(&name, &color, &avatar)) {
                    let rev = self.next_rev(conn).await?;
                    sqlx::query(
                        "UPDATE members SET name = ?, color = ?, avatar = ?, updated_at = ?, rev = ? WHERE id = ?",
                    )
                    .bind(&member.name)
                    .bind(&member.color)
                    .bind(&member.avatar)
                    .bind(updated_at)
                    .bind(rev)
                    .bind(&member.id)
                    .execute(&mut *conn)
                    .await?;
                    self.members_rev = Some(rev);
                }
            }
        }
        Ok(())
    }

    async fn device(&mut self, conn: &mut SqliteConnection, d: &DeviceRecord) -> ApiResult<()> {
        let stored: Option<(i64, i64)> = sqlx::query_as("SELECT revoked, last_seen FROM devices WHERE id = ?")
            .bind(&d.id)
            .fetch_optional(&mut *conn)
            .await?;
        match stored {
            None => {
                let rev = self.next_rev(conn).await?;
                sqlx::query(
                    "INSERT INTO devices (id, member_id, name, token_hash, created_at, last_seen, revoked, rev)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(&d.id)
                .bind(&d.member_id)
                .bind(&d.name)
                .bind(&d.token_hash)
                .bind(d.created_at)
                .bind(d.last_seen)
                .bind(d.revoked)
                .bind(rev)
                .execute(&mut *conn)
                .await?;
            }
            Some((revoked, last_seen)) => {
                if d.revoked && revoked == 0 {
                    let rev = self.next_rev(conn).await?;
                    sqlx::query("UPDATE devices SET revoked = 1, rev = ? WHERE id = ?")
                        .bind(rev)
                        .bind(&d.id)
                        .execute(&mut *conn)
                        .await?;
                    self.events.push((Audience::Device(d.id.clone()), Event::Revoked));
                }
                // Not a change devices need to hear about, so no change number.
                if d.last_seen > last_seen {
                    sqlx::query("UPDATE devices SET last_seen = ? WHERE id = ?")
                        .bind(d.last_seen)
                        .bind(&d.id)
                        .execute(&mut *conn)
                        .await?;
                }
            }
        }
        Ok(())
    }

    async fn media(&mut self, conn: &mut SqliteConnection, m: &MediaRecord) -> ApiResult<()> {
        let exists: Option<String> =
            sqlx::query_scalar("SELECT id FROM media WHERE id = ?").bind(&m.id).fetch_optional(&mut *conn).await?;
        if exists.is_none() {
            let rev = self.next_rev(conn).await?;
            sqlx::query(
                "INSERT INTO media (id, uploader_id, content_type, size, created_at, rev) VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(&m.id)
            .bind(&m.uploader_id)
            .bind(&m.content_type)
            .bind(m.size as i64)
            .bind(m.created_at)
            .bind(rev)
            .execute(&mut *conn)
            .await?;
        }
        Ok(())
    }

    async fn chat(&mut self, conn: &mut SqliteConnection, c: &ChatRecord) -> ApiResult<()> {
        let mut changed = false;
        let created_at: Option<i64> = sqlx::query_scalar("SELECT created_at FROM chats WHERE id = ?")
            .bind(&c.id)
            .fetch_optional(&mut *conn)
            .await?;
        if let Some(created_at) = created_at {
            // Both hubs may have started the same two people's chat while apart: they settle on the earlier time.
            if c.created_at < created_at {
                sqlx::query("UPDATE chats SET created_at = ? WHERE id = ?")
                    .bind(c.created_at)
                    .bind(&c.id)
                    .execute(&mut *conn)
                    .await?;
                changed = true;
            }
        } else {
            if let Some(key) = &c.direct_key {
                let other: Option<String> = sqlx::query_scalar("SELECT id FROM chats WHERE direct_key = ?")
                    .bind(key)
                    .fetch_optional(&mut *conn)
                    .await?;
                if other.is_some() {
                    // Only possible for two chats started on hubs from before 0.6, which gave them random IDs.
                    return Err(ApiError::Conflict(format!(
                        "Two hubs have different chats for the same two people ({key})."
                    )));
                }
            }
            let kind = if c.kind == ChatKind::Family { "family" } else { "direct" };
            sqlx::query("INSERT INTO chats (id, kind, direct_key, created_at, rev) VALUES (?, ?, ?, ?, 0)")
                .bind(&c.id)
                .bind(kind)
                .bind(&c.direct_key)
                .bind(c.created_at)
                .execute(&mut *conn)
                .await?;
            changed = true;
        }
        for ChatMemberRecord { member_id, read_up_to } in &c.members {
            let stored: Option<(Option<String>,)> =
                sqlx::query_as("SELECT read_up_to FROM chat_members WHERE chat_id = ? AND member_id = ?")
                    .bind(&c.id)
                    .bind(member_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            match stored {
                None => {
                    sqlx::query("INSERT INTO chat_members (chat_id, member_id, read_up_to) VALUES (?, ?, ?)")
                        .bind(&c.id)
                        .bind(member_id)
                        .bind(read_up_to)
                        .execute(&mut *conn)
                        .await?;
                    changed = true;
                }
                Some((stored,)) => {
                    // ULIDs sort by time, and any message is later than none.
                    if read_up_to.as_deref() > stored.as_deref() {
                        sqlx::query("UPDATE chat_members SET read_up_to = ? WHERE chat_id = ? AND member_id = ?")
                            .bind(read_up_to)
                            .bind(&c.id)
                            .bind(member_id)
                            .execute(&mut *conn)
                            .await?;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            let rev = self.next_rev(conn).await?;
            sqlx::query("UPDATE chats SET rev = ? WHERE id = ?").bind(rev).bind(&c.id).execute(&mut *conn).await?;
            let chat = db::chat(conn, &c.id).await?.ok_or_else(|| ApiError::Internal("chat vanished".into()))?;
            self.events.push((Audience::Members(chat.member_ids.clone()), Event::Chat { rev, chat }));
        }
        Ok(())
    }

    async fn message(&mut self, conn: &mut SqliteConnection, m: &MessageRecord) -> ApiResult<()> {
        let stored: Option<(Option<String>, i64)> =
            sqlx::query_as("SELECT link_json, created_at FROM messages WHERE id = ?")
                .bind(&m.id)
                .fetch_optional(&mut *conn)
                .await?;
        let mut changed = false;
        match stored {
            None => {
                sqlx::query(
                    "INSERT INTO messages (id, chat_id, sender_id, body_json, link_json, created_at, rev)
                     VALUES (?, ?, ?, ?, ?, ?, 0)",
                )
                .bind(&m.id)
                .bind(&m.chat_id)
                .bind(&m.sender_id)
                .bind(serde_json::to_string(&m.body)?)
                .bind(m.link.as_ref().map(serde_json::to_string).transpose()?)
                .bind(m.created_at)
                .execute(&mut *conn)
                .await?;
                changed = true;
            }
            Some((link, created_at)) => {
                // Both hubs settle on the earlier time (a resend may have reached each at a different moment).
                if m.created_at < created_at {
                    sqlx::query("UPDATE messages SET created_at = ? WHERE id = ?")
                        .bind(m.created_at)
                        .bind(&m.id)
                        .execute(&mut *conn)
                        .await?;
                    changed = true;
                }
                if link.is_none()
                    && let Some(incoming) = &m.link
                {
                    sqlx::query("UPDATE messages SET link_json = ? WHERE id = ?")
                        .bind(serde_json::to_string(incoming)?)
                        .bind(&m.id)
                        .execute(&mut *conn)
                        .await?;
                    changed = true;
                }
            }
        }
        for ReactionRecord { member_id, emoji, updated_at } in &m.reactions {
            let stored: Option<(String, i64)> =
                sqlx::query_as("SELECT emoji, updated_at FROM reactions WHERE message_id = ? AND member_id = ?")
                    .bind(&m.id)
                    .bind(member_id)
                    .fetch_optional(&mut *conn)
                    .await?;
            let wins = match &stored {
                None => true,
                Some((stored_emoji, stored_at)) => newer(*updated_at, emoji, *stored_at, stored_emoji),
            };
            if wins {
                sqlx::query(
                    "INSERT OR REPLACE INTO reactions (message_id, member_id, emoji, updated_at) VALUES (?, ?, ?, ?)",
                )
                .bind(&m.id)
                .bind(member_id)
                .bind(emoji)
                .bind(updated_at)
                .execute(&mut *conn)
                .await?;
                changed = true;
            }
        }
        if changed {
            let rev = self.next_rev(conn).await?;
            sqlx::query("UPDATE messages SET rev = ? WHERE id = ?").bind(rev).bind(&m.id).execute(&mut *conn).await?;
            let message =
                db::message(conn, &m.id).await?.ok_or_else(|| ApiError::Internal("message vanished".into()))?;
            let chat = db::chat(conn, &m.chat_id).await?.ok_or_else(|| ApiError::Internal("chat vanished".into()))?;
            self.events.push((Audience::Members(chat.member_ids), Event::Message { rev, message }));
        }
        Ok(())
    }

    async fn note(&mut self, conn: &mut SqliteConnection, record: &NoteRecord) -> ApiResult<()> {
        let mut incoming = record.note.clone();
        incoming.rev = 0;
        let stored = db::note(conn, &incoming.id).await?;
        let (note, deleted, was_shared) = match stored {
            None => (incoming, record.deleted, false),
            Some((mut stored, stored_deleted)) => {
                stored.rev = 0;
                let merged = notes::merge_copies(&stored, &incoming);
                let deleted = stored_deleted || record.deleted;
                if merged == stored && deleted == stored_deleted {
                    return Ok(());
                }
                (merged, deleted, stored.shared)
            }
        };
        let rev = self.next_rev(conn).await?;
        sqlx::query(
            "INSERT INTO notes (id, owner_id, json, shared, deleted, rev) VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT (id) DO UPDATE SET json = excluded.json, shared = excluded.shared, deleted = excluded.deleted,
               rev = excluded.rev",
        )
        .bind(&note.id)
        .bind(&note.owner_id)
        .bind(serde_json::to_string(&note)?)
        .bind(note.shared)
        .bind(deleted)
        .bind(rev)
        .execute(&mut *conn)
        .await?;
        let owner = note.owner_id.clone();
        let mut note = note;
        note.rev = rev;
        // As the notes routes tell devices about a save or a deletion.
        if deleted {
            let audience = if note.shared { Audience::Everyone } else { Audience::Members(vec![owner]) };
            self.events.push((audience, Event::NoteDeleted { rev, id: note.id }));
        } else if note.shared {
            self.events.push((Audience::Everyone, Event::Note { rev, note }));
        } else {
            let id = note.id.clone();
            self.events.push((Audience::Members(vec![owner.clone()]), Event::Note { rev, note }));
            if was_shared {
                self.events.push((Audience::AllBut(vec![owner]), Event::NoteDeleted { rev, id }));
            }
        }
        Ok(())
    }
}

// ---- Reading this hub's rows as records -------------------------------------------------------------------------

async fn family(conn: &mut SqliteConnection) -> ApiResult<Option<FamilyRecord>> {
    let row: Option<(Option<String>, String, i64)> =
        sqlx::query_as("SELECT family_id, name, created_at FROM family WHERE id = 1").fetch_optional(conn).await?;
    Ok(row.map(|(family_id, name, created_at)| FamilyRecord {
        family_id: family_id.unwrap_or_default(),
        name,
        created_at,
    }))
}

async fn members(conn: &mut SqliteConnection) -> ApiResult<Vec<MemberRecord>> {
    let rows: Vec<(String, i64, i64)> =
        sqlx::query_as("SELECT id, created_at, updated_at FROM members ORDER BY created_at, id")
            .fetch_all(&mut *conn)
            .await?;
    let times: BTreeMap<String, (i64, i64)> = rows.into_iter().map(|(id, c, u)| (id, (c, u))).collect();
    let members: Vec<Member> = db::members(conn).await?;
    Ok(members
        .into_iter()
        .map(|member| {
            let (created_at, updated_at) = times.get(&member.id).copied().unwrap_or_default();
            MemberRecord { member, created_at, updated_at }
        })
        .collect())
}

type DeviceRow = (String, String, String, String, i64, i64, i64, i64);

async fn devices_since(conn: &mut SqliteConnection, since: i64, limit: i64) -> ApiResult<Vec<(i64, DeviceRecord)>> {
    let rows: Vec<DeviceRow> = sqlx::query_as(
        "SELECT id, member_id, name, token_hash, created_at, last_seen, revoked, rev FROM devices
         WHERE rev > ? ORDER BY rev LIMIT ?",
    )
    .bind(since)
    .bind(limit)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, member_id, name, token_hash, created_at, last_seen, revoked, rev)| {
            (rev, DeviceRecord { id, member_id, name, token_hash, created_at, last_seen, revoked: revoked != 0 })
        })
        .collect())
}

async fn chat(conn: &mut SqliteConnection, id: &str) -> ApiResult<Option<ChatRecord>> {
    let row: Option<(String, String, Option<String>, i64)> =
        sqlx::query_as("SELECT id, kind, direct_key, created_at FROM chats WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some((id, kind, direct_key, created_at)) = row else { return Ok(None) };
    let members: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT member_id, read_up_to FROM chat_members WHERE chat_id = ? ORDER BY member_id")
            .bind(&id)
            .fetch_all(conn)
            .await?;
    let kind = if kind == "family" { ChatKind::Family } else { ChatKind::Direct };
    let members =
        members.into_iter().map(|(member_id, read_up_to)| ChatMemberRecord { member_id, read_up_to }).collect();
    Ok(Some(ChatRecord { id, kind, direct_key, created_at, members }))
}

async fn chats_since(conn: &mut SqliteConnection, since: i64, limit: i64) -> ApiResult<Vec<(i64, ChatRecord)>> {
    let rows: Vec<(String, i64)> = sqlx::query_as("SELECT id, rev FROM chats WHERE rev > ? ORDER BY rev LIMIT ?")
        .bind(since)
        .bind(limit)
        .fetch_all(&mut *conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, rev) in rows {
        if let Some(chat) = chat(conn, &id).await? {
            out.push((rev, chat));
        }
    }
    Ok(out)
}

async fn messages_since(conn: &mut SqliteConnection, since: i64, limit: i64) -> ApiResult<Vec<(i64, MessageRecord)>> {
    let rows: Vec<db::MessageRow> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {} FROM messages WHERE rev > ? ORDER BY rev LIMIT ?",
        db::MESSAGE_COLUMNS
    )))
    .bind(since)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, chat_id, sender_id, body_json, link_json, created_at, rev) in rows {
        let reactions: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT member_id, emoji, updated_at FROM reactions WHERE message_id = ? ORDER BY member_id",
        )
        .bind(&id)
        .fetch_all(&mut *conn)
        .await?;
        let reactions = reactions
            .into_iter()
            .map(|(member_id, emoji, updated_at)| ReactionRecord { member_id, emoji, updated_at })
            .collect();
        let body = serde_json::from_str(&body_json)?;
        let link = link_json.as_deref().map(serde_json::from_str).transpose()?;
        out.push((rev, MessageRecord { id, chat_id, sender_id, body, link, created_at, reactions }));
    }
    Ok(out)
}

async fn notes_since(conn: &mut SqliteConnection, since: i64, limit: i64) -> ApiResult<Vec<(i64, NoteRecord)>> {
    let rows: Vec<(String, i64, i64)> =
        sqlx::query_as("SELECT json, deleted, rev FROM notes WHERE rev > ? ORDER BY rev LIMIT ?")
            .bind(since)
            .bind(limit)
            .fetch_all(conn)
            .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (json, deleted, rev) in rows {
        let mut note: Note = serde_json::from_str(&json)?;
        note.rev = 0;
        out.push((rev, NoteRecord { note, deleted: deleted != 0 }));
    }
    Ok(out)
}

async fn media_since(conn: &mut SqliteConnection, since: i64, limit: i64) -> ApiResult<Vec<(i64, MediaRecord)>> {
    let rows: Vec<(String, Option<String>, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT id, uploader_id, content_type, size, created_at, rev FROM media WHERE rev > ? ORDER BY rev LIMIT ?",
    )
    .bind(since)
    .bind(limit)
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, uploader_id, content_type, size, created_at, rev)| {
            (rev, MediaRecord { id, uploader_id, content_type, size: size as u64, created_at })
        })
        .collect())
}
