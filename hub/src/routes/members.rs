//! The family's members and devices. Parents add members, make join codes and remove devices; anyone can change
//! their own name, colour and picture.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use kit_protocol::{Device, EnrollCode, Event, Member, MemberUpdate, NewMember};

use super::{check_avatar, check_color, clean_name};
use crate::auth::Authed;
use crate::error::{ApiError, ApiResult};
use crate::live::{self, Audience};
use crate::{Hub, db, now_ms, random};

/// How long a join code works.
const CODE_LIFETIME_MS: i64 = 10 * 60 * 1000;

pub async fn list(State(hub): State<Arc<Hub>>, _authed: Authed) -> ApiResult<Json<Vec<Member>>> {
    let mut conn = hub.pool.acquire().await?;
    Ok(Json(db::members(&mut conn).await?))
}

pub async fn create(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Json(req): Json<NewMember>,
) -> ApiResult<Json<Member>> {
    authed.require_parent()?;
    let name = clean_name(&req.name, 40, "name")?;
    check_color(&req.color)?;
    check_avatar(&req.avatar)?;
    let mut tx = hub.pool.begin().await?;
    let rev = db::next_rev(&mut tx).await?;
    let id = ulid::Ulid::new().to_string();
    sqlx::query("INSERT INTO members (id, name, role, color, avatar, created_at, rev) VALUES (?, ?, ?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(&name)
        .bind(db::role_str(req.role))
        .bind(&req.color)
        .bind(&req.avatar)
        .bind(now_ms())
        .bind(rev)
        .execute(&mut *tx)
        .await?;
    // Everyone is in the family chat.
    let family_chat = db::family_chat_id(&mut tx).await?;
    sqlx::query("INSERT INTO chat_members (chat_id, member_id) VALUES (?, ?)")
        .bind(&family_chat)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE chats SET rev = ? WHERE id = ?").bind(rev).bind(&family_chat).execute(&mut *tx).await?;
    let member = db::member(&mut tx, &id).await?.ok_or_else(|| ApiError::Internal("member vanished".into()))?;
    let members = db::members(&mut tx).await?;
    let chat = db::chat(&mut tx, &family_chat).await?;
    tx.commit().await?;
    live::send(&hub, Audience::Everyone, Event::Members { rev, members });
    if let Some(chat) = chat {
        live::send(&hub, Audience::Everyone, Event::Chat { rev, chat });
    }
    log::info!("{} added {name} to the family", authed.member.name);
    Ok(Json(member))
}

pub async fn update(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(id): Path<String>,
    Json(req): Json<MemberUpdate>,
) -> ApiResult<Json<Member>> {
    if id != authed.member.id {
        authed.require_parent()?;
    }
    let mut tx = hub.pool.begin().await?;
    let mut member = db::member(&mut tx, &id).await?.ok_or_else(|| ApiError::NotFound("No such person.".into()))?;
    if let Some(name) = &req.name {
        member.name = clean_name(name, 40, "name")?;
    }
    if let Some(color) = req.color {
        check_color(&color)?;
        member.color = color;
    }
    if let Some(avatar) = req.avatar {
        check_avatar(&avatar)?;
        member.avatar = avatar;
    }
    let rev = db::next_rev(&mut tx).await?;
    sqlx::query("UPDATE members SET name = ?, color = ?, avatar = ?, rev = ? WHERE id = ?")
        .bind(&member.name)
        .bind(&member.color)
        .bind(&member.avatar)
        .bind(rev)
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let members = db::members(&mut tx).await?;
    tx.commit().await?;
    live::send(&hub, Audience::Everyone, Event::Members { rev, members });
    Ok(Json(member))
}

pub async fn new_code(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(id): Path<String>,
) -> ApiResult<Json<EnrollCode>> {
    authed.require_parent()?;
    let mut conn = hub.pool.acquire().await?;
    if db::member(&mut conn, &id).await?.is_none() {
        return Err(ApiError::NotFound("No such person.".into()));
    }
    let now = now_ms();
    sqlx::query("DELETE FROM enroll_codes WHERE expires_at < ? OR used = 1").bind(now).execute(&mut *conn).await?;
    let expires_at = now + CODE_LIFETIME_MS;
    // A clash with a live code is astronomically unlikely; try again rather than fail.
    for _ in 0..5 {
        let code = random::enroll_code();
        let inserted = sqlx::query("INSERT OR IGNORE INTO enroll_codes (code, member_id, expires_at) VALUES (?, ?, ?)")
            .bind(&code)
            .bind(&id)
            .bind(expires_at)
            .execute(&mut *conn)
            .await?;
        if inserted.rows_affected() == 1 {
            return Ok(Json(EnrollCode { code, member_id: id, expires_at }));
        }
    }
    Err(ApiError::Internal("couldn't make a unique join code".into()))
}

pub async fn devices(State(hub): State<Arc<Hub>>, authed: Authed) -> ApiResult<Json<Vec<Device>>> {
    authed.require_parent()?;
    let rows: Vec<(String, String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, member_id, name, created_at, last_seen FROM devices WHERE revoked = 0 ORDER BY created_at",
    )
    .fetch_all(&hub.pool)
    .await?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, member_id, name, created_at, last_seen)| Device { id, member_id, name, created_at, last_seen })
            .collect(),
    ))
}

pub async fn revoke(State(hub): State<Arc<Hub>>, authed: Authed, Path(id): Path<String>) -> ApiResult<Json<()>> {
    authed.require_parent()?;
    if id == authed.device_id {
        return Err(ApiError::bad("You can't remove the device you're using. Do it from another parent's device."));
    }
    let done =
        sqlx::query("UPDATE devices SET revoked = 1 WHERE id = ? AND revoked = 0").bind(&id).execute(&hub.pool).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("No such device.".into()));
    }
    live::send(&hub, Audience::Device(id.clone()), Event::Revoked);
    log::info!("{} removed device {id}", authed.member.name);
    Ok(Json(()))
}
