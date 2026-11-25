//! Notes and lists. A note is its owner's alone until they make it a family list; then everyone can see and edit it,
//! and edits are combined block by block (kit_protocol::notes::merge).
//!
//! A line with a web link gets a preview card, like a message does: after a save the hub fetches the pages it hasn't
//! seen lately and sends the note again with them.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use kit_protocol::Event;
use kit_protocol::notes::{self, Note};

use super::check_id;
use crate::auth::Authed;
use crate::error::{ApiError, ApiResult};
use crate::live::{self, Audience};
use crate::{Hub, db, now_ms};

pub async fn save(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(id): Path<String>,
    Json(incoming): Json<Note>,
) -> ApiResult<Json<Note>> {
    check_id(&id)?;
    if incoming.id != id {
        return Err(ApiError::bad("The note's ID doesn't match."));
    }
    notes::validate(&incoming).map_err(ApiError::BadRequest)?;
    super::check_color(&incoming.color)?;
    let me = &authed.member.id;

    let mut tx = hub.pool.begin().await?;
    let stored = db::note(&mut tx, &id).await?;
    let (merged, was_shared) = match stored {
        Some((_, true)) => return Err(ApiError::NotFound("That note was deleted.".into())),
        Some((stored, false)) => {
            if &stored.owner_id != me && !stored.shared {
                return Err(ApiError::NotFound("No such note.".into()));
            }
            let was_shared = stored.shared;
            (notes::merge(&stored, &incoming, &stored.owner_id == me, me), was_shared)
        }
        None => {
            let mut note = incoming.clone();
            note.owner_id = me.clone();
            note.created_at = now_ms();
            for block in &mut note.blocks {
                block.edited_by = me.clone();
                block.link = None;
            }
            (note, false)
        }
    };
    if merged.blocks.len() > notes::MAX_BLOCKS {
        return Err(ApiError::bad(format!("A note can have up to {} lines.", notes::MAX_BLOCKS)));
    }
    let mut merged = merged;
    let to_fetch = previews_to_fetch(&hub, &mut merged);
    let rev = put(&mut tx, &mut merged).await?;
    tx.commit().await?;

    announce(&hub, &merged);
    if !merged.shared && was_shared {
        live::send(&hub, Audience::AllBut(vec![merged.owner_id.clone()]), Event::NoteDeleted { rev, id: id.clone() });
    }
    for url in to_fetch {
        let (hub, id) = (hub.clone(), id.clone());
        tokio::spawn(async move {
            if let Err(e) = add_preview(&hub, &id, &url).await {
                log::warn!("link preview failed: {e}");
            }
        });
    }
    Ok(Json(merged))
}

/// The most links one save fetches; the rest wait for the next save.
const FETCHES_PER_SAVE: usize = 12;

/// Fills in the previews the hub fetched lately, and returns the links it still has to fetch.
fn previews_to_fetch(hub: &Hub, note: &mut Note) -> Vec<String> {
    if hub.previews.is_none() {
        return Vec::new();
    }
    let mut to_fetch: Vec<String> = Vec::new();
    for block in &mut note.blocks {
        let Some((_, url)) = block.wants_preview() else { continue };
        match hub.recent_previews.get(&url) {
            Some(Some(preview)) => block.link = Some(preview),
            Some(None) => {}
            None if to_fetch.len() < FETCHES_PER_SAVE && !to_fetch.contains(&url) => to_fetch.push(url),
            None => {}
        }
    }
    // Marked as being fetched, so the saves made while it is don't fetch it again.
    for url in &to_fetch {
        hub.recent_previews.put(url, None);
    }
    to_fetch
}

/// Fetches a link's preview and adds it to every line of the note that still links there.
async fn add_preview(hub: &Hub, note_id: &str, url: &str) -> ApiResult<()> {
    let preview = super::chats::fetch_preview(hub, url).await?;
    hub.recent_previews.put(url, preview.clone());
    let Some(preview) = preview else { return Ok(()) };
    let mut tx = hub.pool.begin().await?;
    let Some((mut note, false)) = db::note(&mut tx, note_id).await? else { return Ok(()) };
    let mut changed = false;
    for block in &mut note.blocks {
        if block.wants_preview().is_some_and(|(_, wanted)| wanted == url) {
            block.link = Some(preview.clone());
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }
    put(&mut tx, &mut note).await?;
    tx.commit().await?;
    announce(hub, &note);
    Ok(())
}

/// Stores a note with a new change counter, and returns the counter.
async fn put(tx: &mut sqlx::SqliteConnection, note: &mut Note) -> ApiResult<i64> {
    let rev = db::next_rev(&mut *tx).await?;
    note.rev = rev;
    sqlx::query(
        "INSERT INTO notes (id, owner_id, json, shared, rev) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET json = excluded.json, shared = excluded.shared, rev = excluded.rev",
    )
    .bind(&note.id)
    .bind(&note.owner_id)
    .bind(serde_json::to_string(&note)?)
    .bind(note.shared)
    .bind(rev)
    .execute(&mut *tx)
    .await?;
    Ok(rev)
}

/// Sends a note to everyone who can see it.
fn announce(hub: &Hub, note: &Note) {
    let audience = if note.shared { Audience::Everyone } else { Audience::Members(vec![note.owner_id.clone()]) };
    live::send(hub, audience, Event::Note { rev: note.rev, note: note.clone() });
}

pub async fn remove(State(hub): State<Arc<Hub>>, authed: Authed, Path(id): Path<String>) -> ApiResult<Json<()>> {
    let mut tx = hub.pool.begin().await?;
    let (note, deleted) = db::note(&mut tx, &id).await?.ok_or_else(|| ApiError::NotFound("No such note.".into()))?;
    if deleted {
        return Ok(Json(()));
    }
    if note.owner_id != authed.member.id && !note.shared {
        return Err(ApiError::NotFound("No such note.".into()));
    }
    if note.owner_id != authed.member.id {
        return Err(ApiError::Forbidden("Only the person who made this list can delete it.".into()));
    }
    let rev = db::next_rev(&mut tx).await?;
    sqlx::query("UPDATE notes SET deleted = 1, rev = ? WHERE id = ?").bind(rev).bind(&id).execute(&mut *tx).await?;
    tx.commit().await?;
    let audience = if note.shared { Audience::Everyone } else { Audience::Members(vec![note.owner_id]) };
    live::send(&hub, audience, Event::NoteDeleted { rev, id });
    Ok(Json(()))
}
