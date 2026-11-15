mod chats;
mod media;
mod members;
mod notes;
mod setup;
mod sync;

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, patch, post, put};

use crate::error::{ApiError, ApiResult};
use crate::{Hub, live};

pub fn router() -> Router<Arc<Hub>> {
    Router::new()
        .route("/hello", get(setup::hello))
        .route("/setup", post(setup::setup))
        .route("/enroll", post(setup::enroll))
        .route("/sync", get(sync::sync))
        .route("/live", get(live::handler))
        .route("/members", get(members::list).post(members::create))
        .route("/members/{id}", patch(members::update))
        .route("/members/{id}/codes", post(members::new_code))
        .route("/devices", get(members::devices))
        .route("/devices/{id}", delete(members::revoke))
        .route("/chats/direct", post(chats::direct))
        .route("/chats/{id}/messages", get(chats::history).post(chats::send))
        .route("/chats/{id}/read", post(chats::read))
        .route("/messages/{id}/reactions", post(chats::react))
        .route("/media", post(media::upload))
        .route("/media/{id}", get(media::download))
        .route("/notes/{id}", put(notes::save).delete(notes::remove))
        .layer(DefaultBodyLimit::max(kit_protocol::MAX_MEDIA_BYTES + 64 * 1024))
}

/// A ULID, as devices make them for messages and notes.
fn check_id(id: &str) -> ApiResult<()> {
    ulid::Ulid::from_string(id).map(|_| ()).map_err(|_| ApiError::bad("That ID isn't valid."))
}

/// A name a person typed: trimmed, and between 1 and `max` characters.
fn clean_name(name: &str, max: usize, what: &str) -> ApiResult<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(ApiError::bad(format!("The {what} can't be empty.")));
    }
    if name.chars().count() > max {
        return Err(ApiError::bad(format!("The {what} can be up to {max} characters.")));
    }
    Ok(name)
}

fn check_color(color: &str) -> ApiResult<()> {
    if kit_protocol::COLORS.contains(&color) { Ok(()) } else { Err(ApiError::bad("That colour isn't one of ours.")) }
}

fn check_avatar(avatar: &str) -> ApiResult<()> {
    let n = avatar.chars().count();
    if (1..=24).contains(&n) { Ok(()) } else { Err(ApiError::bad("Pick a picture for this person.")) }
}
