//! Notes and lists. A note is its owner's alone until they make it a family list; then everyone can see and edit it,
//! and edits are combined block by block (kit_protocol::notes::merge).

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
            (notes::merge(&stored, &incoming, &stored.owner_id == me), was_shared)
        }
        None => {
            let mut note = incoming.clone();
            note.owner_id = me.clone();
            note.created_at = now_ms();
            (note, false)
        }
    };
    if merged.blocks.len() > notes::MAX_BLOCKS {
        return Err(ApiError::bad(format!("A note can have up to {} lines.", notes::MAX_BLOCKS)));
    }
    let rev = db::next_rev(&mut tx).await?;
    let mut merged = merged;
    merged.rev = rev;
    sqlx::query(
        "INSERT INTO notes (id, owner_id, json, shared, rev) VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (id) DO UPDATE SET json = excluded.json, shared = excluded.shared, rev = excluded.rev",
    )
    .bind(&merged.id)
    .bind(&merged.owner_id)
    .bind(serde_json::to_string(&merged)?)
    .bind(merged.shared)
    .bind(rev)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    let owner = merged.owner_id.clone();
    if merged.shared {
        live::send(&hub, Audience::Everyone, Event::Note { rev, note: merged.clone() });
    } else {
        live::send(&hub, Audience::Members(vec![owner.clone()]), Event::Note { rev, note: merged.clone() });
        if was_shared {
            live::send(&hub, Audience::AllBut(vec![owner]), Event::NoteDeleted { rev, id: merged.id.clone() });
        }
    }
    Ok(Json(merged))
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
