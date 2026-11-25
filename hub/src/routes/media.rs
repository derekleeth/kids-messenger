//! Photos and videos: uploaded once, then fetched by ID by anyone in the family.
//!
//! Small pictures (what the chat shows, and link previews) come in one request to `POST /media`. Full-size photos and
//! videos come in chunks: `POST /media/uploads` starts one, `PUT /media/uploads/{id}?offset=` adds each chunk, and
//! `POST /media/uploads/{id}/finish` checks the file and makes it media. A device that loses its connection asks
//! `GET /media/uploads/{id}` how much arrived and carries on from there. Downloads answer Range requests, so a video
//! plays (or resumes downloading) from the middle.
//!
//! Every file loses its hidden data (where it was taken) on the way in: see `kit_protocol::media`.

use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderValue, header};
use axum::response::Response;
use kit_protocol::media;
use kit_protocol::{MAX_ORIGINAL_BYTES, NewUpload, UploadState, Uploaded};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tower::ServiceExt;

use crate::auth::Authed;
use crate::error::{ApiError, ApiResult};
use crate::{Hub, now_ms};

/// An upload nobody finished is thrown away after this long.
const UPLOAD_LIFETIME_MS: i64 = 3 * 24 * 60 * 60 * 1000;

async fn add_media(hub: &Hub, id: &str, uploader: Option<&str>, kind: &str, size: u64) -> ApiResult<()> {
    let mut tx = hub.pool.begin().await?;
    let rev = crate::db::next_rev(&mut tx).await?;
    sqlx::query("INSERT INTO media (id, uploader_id, content_type, size, created_at, rev) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(id)
        .bind(uploader)
        .bind(kind)
        .bind(size as i64)
        .bind(now_ms())
        .bind(rev)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Saves a small picture and returns its ID.
pub async fn store(hub: &Hub, uploader: Option<&str>, bytes: Vec<u8>, kind: &str) -> ApiResult<String> {
    let bytes = media::clean_photo(&bytes);
    let id = ulid::Ulid::new().to_string();
    tokio::fs::write(hub.media_dir().join(&id), &bytes).await?;
    add_media(hub, &id, uploader, kind, bytes.len() as u64).await?;
    Ok(id)
}

pub async fn upload(State(hub): State<Arc<Hub>>, authed: Authed, body: Bytes) -> ApiResult<axum::Json<Uploaded>> {
    if body.is_empty() {
        return Err(ApiError::bad("That picture is empty."));
    }
    if body.len() > kit_protocol::MAX_MEDIA_BYTES {
        return Err(ApiError::bad("That picture is too big."));
    }
    let kind = media::sniff(&body)
        .filter(|k| media::is_shown_picture(k))
        .ok_or_else(|| ApiError::bad("Only pictures can be sent (JPEG, PNG, WebP or GIF)."))?;
    let media_id = store(&hub, Some(&authed.member.id), body.to_vec(), kind).await?;
    Ok(axum::Json(Uploaded { media_id }))
}

// ---- Big uploads ------------------------------------------------------------------------------------------------

async fn upload_row(hub: &Hub, authed: &Authed, id: &str) -> ApiResult<UploadState> {
    super::check_id(id)?;
    let row: Option<(i64, i64)> = sqlx::query_as("SELECT size, received FROM uploads WHERE id = ? AND uploader_id = ?")
        .bind(id)
        .bind(&authed.member.id)
        .fetch_optional(&hub.pool)
        .await?;
    let (size, received) = row.ok_or_else(|| ApiError::NotFound("That upload is gone. Send it again.".into()))?;
    Ok(UploadState { upload_id: id.to_string(), size: size as u64, received: received as u64 })
}

pub async fn start_upload(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    axum::Json(req): axum::Json<NewUpload>,
) -> ApiResult<axum::Json<UploadState>> {
    if req.size == 0 {
        return Err(ApiError::bad("That file is empty."));
    }
    if req.size > MAX_ORIGINAL_BYTES {
        return Err(ApiError::bad(format!(
            "That's too big to send (the most is {} GB). Try a shorter video.",
            MAX_ORIGINAL_BYTES / (1024 * 1024 * 1024)
        )));
    }
    forget_old_uploads(&hub).await?;
    let id = ulid::Ulid::new().to_string();
    tokio::fs::write(hub.uploads_dir().join(&id), b"").await?;
    sqlx::query("INSERT INTO uploads (id, uploader_id, size, received, created_at) VALUES (?, ?, ?, 0, ?)")
        .bind(&id)
        .bind(&authed.member.id)
        .bind(req.size as i64)
        .bind(now_ms())
        .execute(&hub.pool)
        .await?;
    Ok(axum::Json(UploadState { upload_id: id, size: req.size, received: 0 }))
}

async fn forget_old_uploads(hub: &Hub) -> ApiResult<()> {
    let old: Vec<String> = sqlx::query_scalar("SELECT id FROM uploads WHERE created_at < ?")
        .bind(now_ms() - UPLOAD_LIFETIME_MS)
        .fetch_all(&hub.pool)
        .await?;
    for id in old {
        let _ = tokio::fs::remove_file(hub.uploads_dir().join(&id)).await;
        sqlx::query("DELETE FROM uploads WHERE id = ?").bind(&id).execute(&hub.pool).await?;
    }
    Ok(())
}

pub async fn upload_state(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<UploadState>> {
    Ok(axum::Json(upload_row(&hub, &authed, &id).await?))
}

#[derive(Deserialize)]
pub struct ChunkAt {
    offset: u64,
}

pub async fn add_chunk(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(id): Path<String>,
    Query(at): Query<ChunkAt>,
    body: Bytes,
) -> ApiResult<axum::Json<UploadState>> {
    let mut state = upload_row(&hub, &authed, &id).await?;
    if at.offset != state.received {
        return Err(ApiError::Conflict(format!(
            "The hub has {} bytes of this upload, not {}.",
            state.received, at.offset
        )));
    }
    if state.received + body.len() as u64 > state.size {
        return Err(ApiError::bad("That's more than the file's size."));
    }
    let mut file = tokio::fs::OpenOptions::new().write(true).open(hub.uploads_dir().join(&id)).await?;
    // Anything past what was counted is a chunk that didn't finish arriving last time.
    file.set_len(state.received).await?;
    file.seek(std::io::SeekFrom::End(0)).await?;
    file.write_all(&body).await?;
    file.flush().await?;
    state.received += body.len() as u64;
    sqlx::query("UPDATE uploads SET received = ? WHERE id = ?")
        .bind(state.received as i64)
        .bind(&id)
        .execute(&hub.pool)
        .await?;
    Ok(axum::Json(state))
}

pub async fn finish_upload(
    State(hub): State<Arc<Hub>>,
    authed: Authed,
    Path(id): Path<String>,
) -> ApiResult<axum::Json<Uploaded>> {
    let state = upload_row(&hub, &authed, &id).await?;
    if state.received != state.size {
        return Err(ApiError::bad("That upload isn't finished yet."));
    }
    let part = hub.uploads_dir().join(&id);
    let mut head = vec![0u8; 16];
    {
        let mut f = tokio::fs::File::open(&part).await?;
        let n = f.read(&mut head).await?;
        head.truncate(n);
    }
    let kind = media::sniff(&head).unwrap_or_default();
    let size = if media::is_video(kind) {
        let path = part.clone();
        tokio::task::spawn_blocking(move || media::clean_video_file(&path))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?
            .map_err(|e| ApiError::bad(format!("That video can't be read ({e}).")))?;
        state.size
    } else if media::is_shown_picture(kind) {
        let bytes = media::clean_photo(&tokio::fs::read(&part).await?);
        tokio::fs::write(&part, &bytes).await?;
        bytes.len() as u64
    } else {
        let _ = tokio::fs::remove_file(&part).await;
        sqlx::query("DELETE FROM uploads WHERE id = ?").bind(&id).execute(&hub.pool).await?;
        return Err(ApiError::bad("Only photos (JPEG, PNG, WebP, GIF) and MP4 videos can be sent."));
    };
    let media_id = ulid::Ulid::new().to_string();
    tokio::fs::rename(&part, hub.media_dir().join(&media_id)).await?;
    add_media(&hub, &media_id, Some(&authed.member.id), kind, size).await?;
    sqlx::query("DELETE FROM uploads WHERE id = ?").bind(&id).execute(&hub.pool).await?;
    Ok(axum::Json(Uploaded { media_id }))
}

// ---- Downloads --------------------------------------------------------------------------------------------------

pub async fn download(
    State(hub): State<Arc<Hub>>,
    _authed: Authed,
    Path(id): Path<String>,
    req: Request,
) -> ApiResult<Response> {
    super::check_id(&id)?;
    let kind: Option<String> =
        sqlx::query_scalar("SELECT content_type FROM media WHERE id = ?").bind(&id).fetch_optional(&hub.pool).await?;
    let gone = || ApiError::NotFound("That picture is gone.".into());
    let kind = kind.ok_or_else(gone)?;
    let path = hub.media_dir().join(&id);
    if !tokio::fs::try_exists(&path).await? {
        return Err(gone());
    }
    let kind = HeaderValue::from_str(&kind).map_err(|e| ApiError::Internal(e.to_string()))?;
    // ServeFile answers Range and If-Range, and streams from disk.
    let res = tower_http::services::ServeFile::new(&path)
        .oneshot(req)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let (mut parts, body) = res.into_parts();
    parts.headers.insert(header::CONTENT_TYPE, kind);
    // A file never changes once uploaded.
    parts.headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
    parts.headers.insert("X-Content-Type-Options", HeaderValue::from_static("nosniff"));
    Ok(Response::from_parts(parts, Body::new(body)))
}
