//! Who's asking: every request but `/hello`, `/setup` and `/enroll` carries a device token
//! (`Authorization: Bearer <token>`), and the hub looks up the device and its member from the token's hash.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use kit_protocol::{Member, Role};

use crate::error::{ApiError, ApiResult};
use crate::{Hub, db, now_ms, random};

pub struct Authed {
    pub device_id: String,
    pub member: Member,
}

impl Authed {
    pub fn is_parent(&self) -> bool {
        self.member.role == Role::Parent
    }

    pub fn require_parent(&self) -> ApiResult<()> {
        if self.is_parent() { Ok(()) } else { Err(ApiError::Forbidden("Only a parent can do that.".into())) }
    }
}

/// How often a device's "last seen" time is written, at most.
const LAST_SEEN_EVERY_MS: i64 = 60_000;

pub fn bearer(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(str::trim).filter(|t| !t.is_empty())
}

pub async fn device_for_token(hub: &Hub, token: &str) -> ApiResult<Authed> {
    let row: Option<(String, String, i64)> =
        sqlx::query_as("SELECT id, member_id, last_seen FROM devices WHERE token_hash = ? AND revoked = 0")
            .bind(random::hash_token(token))
            .fetch_optional(&hub.pool)
            .await?;
    let Some((device_id, member_id, last_seen)) = row else { return Err(ApiError::Unauthorized) };
    let mut conn = hub.pool.acquire().await?;
    let member = db::member(&mut conn, &member_id).await?.ok_or(ApiError::Unauthorized)?;
    let now = now_ms();
    if now - last_seen > LAST_SEEN_EVERY_MS {
        sqlx::query("UPDATE devices SET last_seen = ? WHERE id = ?")
            .bind(now)
            .bind(&device_id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(Authed { device_id, member })
}

impl FromRequestParts<Arc<Hub>> for Authed {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, hub: &Arc<Hub>) -> Result<Self, Self::Rejection> {
        let token = bearer(parts).ok_or(ApiError::Unauthorized)?;
        device_for_token(hub, token).await
    }
}

/// Slows down guessing setup and enrollment codes: after `MAX_FAILURES` wrong codes within `WINDOW`, every code is
/// refused until the window has passed. One family, a handful of devices: a global limit is enough, and it means a
/// guesser can't get around it by changing address.
#[derive(Default)]
pub struct CodeGuard {
    failures: Mutex<Vec<Instant>>,
}

const MAX_FAILURES: usize = 10;
const WINDOW: Duration = Duration::from_secs(10 * 60);

impl CodeGuard {
    pub fn check(&self) -> ApiResult<()> {
        let mut failures = self.failures.lock().unwrap();
        failures.retain(|t| t.elapsed() < WINDOW);
        if failures.len() >= MAX_FAILURES { Err(ApiError::TooManyAttempts) } else { Ok(()) }
    }

    pub fn failed(&self) {
        self.failures.lock().unwrap().push(Instant::now());
    }
}
