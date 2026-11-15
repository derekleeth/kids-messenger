//! Pictures: uploaded once, then fetched by ID by anyone in the family.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::Response;
use kit_protocol::Uploaded;

use crate::auth::Authed;
use crate::error::{ApiError, ApiResult};
use crate::{Hub, images, now_ms};

/// Saves a picture and returns its ID. JPEGs lose their EXIF (location) data on the way in.
pub async fn store(hub: &Hub, uploader: Option<&str>, bytes: Vec<u8>, kind: &str) -> ApiResult<String> {
    let bytes = if kind == "image/jpeg" { images::strip_jpeg_metadata(&bytes) } else { bytes };
    let id = ulid::Ulid::new().to_string();
    let path = hub.media_dir().join(&id);
    tokio::fs::write(&path, &bytes).await?;
    sqlx::query("INSERT INTO media (id, uploader_id, content_type, size, created_at) VALUES (?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(uploader)
        .bind(kind)
        .bind(bytes.len() as i64)
        .bind(now_ms())
        .execute(&hub.pool)
        .await?;
    Ok(id)
}

pub async fn upload(State(hub): State<Arc<Hub>>, authed: Authed, body: Bytes) -> ApiResult<axum::Json<Uploaded>> {
    if body.is_empty() {
        return Err(ApiError::bad("That picture is empty."));
    }
    if body.len() > kit_protocol::MAX_MEDIA_BYTES {
        return Err(ApiError::bad("That picture is too big."));
    }
    let kind =
        images::sniff(&body).ok_or_else(|| ApiError::bad("Only pictures can be sent (JPEG, PNG, WebP or GIF)."))?;
    let media_id = store(&hub, Some(&authed.member.id), body.to_vec(), kind).await?;
    Ok(axum::Json(Uploaded { media_id }))
}

pub async fn download(State(hub): State<Arc<Hub>>, _authed: Authed, Path(id): Path<String>) -> ApiResult<Response> {
    super::check_id(&id)?;
    let kind: Option<String> =
        sqlx::query_scalar("SELECT content_type FROM media WHERE id = ?").bind(&id).fetch_optional(&hub.pool).await?;
    let kind = kind.ok_or_else(|| ApiError::NotFound("That picture is gone.".into()))?;
    let bytes = tokio::fs::read(hub.media_dir().join(&id)).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ApiError::NotFound("That picture is gone.".into())
        } else {
            e.into()
        }
    })?;
    Response::builder()
        .header(header::CONTENT_TYPE, kind)
        // A picture never changes once uploaded.
        .header(header::CACHE_CONTROL, "private, max-age=31536000, immutable")
        .header("X-Content-Type-Options", "nosniff")
        .body(Body::from(bytes))
        .map_err(|e| ApiError::Internal(e.to_string()))
}
