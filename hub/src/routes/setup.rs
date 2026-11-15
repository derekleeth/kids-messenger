//! Getting into the family: the first parent sets the hub up with its setup code, and every other device joins with
//! a one-time code a parent made for a member.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use kit_protocol::{EnrollRequest, Enrolled, Hello, PROTOCOL_VERSION, Role, SetupRequest};
use sqlx::SqliteConnection;

use super::{check_id, clean_name};
use crate::error::{ApiError, ApiResult};
use crate::{Hub, db, now_ms, random};

pub async fn hello(State(hub): State<Arc<Hub>>) -> ApiResult<Json<Hello>> {
    let name = db::family_name(&hub.pool).await?;
    Ok(Json(Hello {
        app: "kitchat-hub".into(),
        protocol: PROTOCOL_VERSION,
        needs_setup: name.is_none(),
        family_name: name.unwrap_or_default(),
    }))
}

/// Adds a device for `member_id` and returns its new token.
async fn add_device(conn: &mut SqliteConnection, member_id: &str, device_name: &str) -> ApiResult<(String, String)> {
    let device_name = clean_name(device_name, 60, "device name")?;
    let id = ulid::Ulid::new().to_string();
    let token = random::token();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO devices (id, member_id, name, token_hash, created_at, last_seen) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(member_id)
    .bind(device_name)
    .bind(random::hash_token(&token))
    .bind(now)
    .bind(now)
    .execute(conn)
    .await?;
    Ok((id, token))
}

pub async fn setup(State(hub): State<Arc<Hub>>, Json(req): Json<SetupRequest>) -> ApiResult<Json<Enrolled>> {
    hub.guard.check()?;
    let mut setup_code = hub.setup_code.lock().await;
    let Some(expected) = setup_code.as_deref() else {
        return Err(ApiError::Conflict("This family hub is already set up. Ask a parent for a join code.".into()));
    };
    if !random::same(&random::clean_code(&req.setup_code), expected) {
        hub.guard.failed();
        return Err(ApiError::bad("That setup code isn't right. It's shown where the hub was started."));
    }
    let family_name = clean_name(&req.family_name, 40, "family name")?;
    let name = clean_name(&req.name, 40, "name")?;

    let mut tx = hub.pool.begin().await?;
    let now = now_ms();
    sqlx::query("INSERT INTO family (id, name, rev, created_at) VALUES (1, ?, 0, ?)")
        .bind(&family_name)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    let rev = db::next_rev(&mut tx).await?;
    let member_id = ulid::Ulid::new().to_string();
    sqlx::query("INSERT INTO members (id, name, role, color, avatar, created_at, rev) VALUES (?, ?, 'parent', 'sky', '🦁', ?, ?)")
        .bind(&member_id)
        .bind(&name)
        .bind(now)
        .bind(rev)
        .execute(&mut *tx)
        .await?;
    let chat_id = ulid::Ulid::new().to_string();
    sqlx::query("INSERT INTO chats (id, kind, created_at, rev) VALUES (?, 'family', ?, ?)")
        .bind(&chat_id)
        .bind(now)
        .bind(rev)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO chat_members (chat_id, member_id) VALUES (?, ?)")
        .bind(&chat_id)
        .bind(&member_id)
        .execute(&mut *tx)
        .await?;
    let (device_id, token) = add_device(&mut tx, &member_id, &req.device_name).await?;
    let member = db::member(&mut tx, &member_id).await?.ok_or_else(|| ApiError::Internal("member vanished".into()))?;
    tx.commit().await?;

    *setup_code = None;
    let _ = std::fs::remove_file(crate::setup_code_file(&hub.data_dir));
    log::info!("family \"{family_name}\" set up by {name}");
    debug_assert_eq!(member.role, Role::Parent);
    Ok(Json(Enrolled { token, device_id, member, family_name }))
}

pub async fn enroll(State(hub): State<Arc<Hub>>, Json(req): Json<EnrollRequest>) -> ApiResult<Json<Enrolled>> {
    hub.guard.check()?;
    let code = random::clean_code(&req.code);
    let mut tx = hub.pool.begin().await?;
    let member_id: Option<String> = sqlx::query_scalar(
        "UPDATE enroll_codes SET used = 1 WHERE code = ? AND used = 0 AND expires_at > ? RETURNING member_id",
    )
    .bind(&code)
    .bind(now_ms())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(member_id) = member_id else {
        hub.guard.failed();
        return Err(ApiError::bad(
            "That code didn't work. Codes work once, for 10 minutes: ask a parent for a new one.",
        ));
    };
    check_id(&member_id)?;
    let (device_id, token) = add_device(&mut tx, &member_id, &req.device_name).await?;
    let member = db::member(&mut tx, &member_id).await?.ok_or_else(|| ApiError::Internal("member vanished".into()))?;
    tx.commit().await?;
    let family_name = db::family_name(&hub.pool).await?.unwrap_or_default();
    log::info!("{} added a device", member.name);
    Ok(Json(Enrolled { token, device_id, member, family_name }))
}
