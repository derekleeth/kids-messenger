//! Photos and videos between a family's hubs (docs/multi_hub_plan.md, phase 4).
//!
//! Records of pictures travel with the other changes ([`crate::replica`]); their files follow here, in pieces of
//! [`PIECE`], so a slow or dropped connection carries on where it stopped. As with the changes, only the travel hub
//! calls: it sends home the files its devices uploaded, and copies home's files to itself.
//!
//! A travel router has little room, so it copies only the files of the last [`Keep::days`] days, newest first, up to
//! [`Keep::max_bytes`]. An older picture is fetched from home when someone opens it, if home can be reached, and when
//! space runs short the copies kept longest go first (home still has them). Files uploaded to the travel hub itself
//! are never deleted.
//!
//! Each picture's record carries a SHA-256 of its file, worked out by the hub it was uploaded to. A hub taking in a
//! copy checks it before keeping it; one that comes out wrong is thrown away, and not tried again in the background
//! for a day.
//!
//! A hub serving a picture whose file it doesn't have yet answers 503, which the app treats as "try again later".

use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
use axum::response::Response;
use kit_protocol::hubs::{FileProgress, FilesWanted, WantedFiles};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::error::{ApiError, ApiResult};
use crate::peers::{Dialed, Peer, Remote, dialed, dialed_ids, remote_for};
use crate::{Hub, now_ms};

/// How much of a file goes in one request.
pub const PIECE: u64 = 2 * 1024 * 1024;
/// How long one piece may take, on a slow campground connection.
const PIECE_TIME: Duration = Duration::from_secs(120);
/// How many files one round looks at.
const ROUND: i64 = 200;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Which of the other hub's files a hub that calls it keeps a copy of.
#[derive(Debug, Clone, Copy)]
pub struct Keep {
    /// Copy pictures and videos from the last this many days.
    pub days: u32,
    /// Keep at most this much of other hubs' files.
    pub max_bytes: u64,
}

impl Default for Keep {
    fn default() -> Self {
        Keep { days: 30, max_bytes: 20 * 1024 * 1024 * 1024 }
    }
}

/// SHA-256 of a picture's bytes, in hex.
pub fn sha256(bytes: &[u8]) -> String {
    kit_protocol::proof::hex(&Sha256::digest(bytes))
}

/// SHA-256 of a file, in hex, read a piece at a time (a video can be a gigabyte).
pub async fn sha256_file(path: &FsPath) -> ApiResult<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || -> std::io::Result<String> {
        use std::io::Read;
        let mut file = std::fs::File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                return Ok(kit_protocol::proof::hex(&hasher.finalize()));
            }
            hasher.update(&buf[..n]);
        }
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
    .map_err(ApiError::from)
}

/// Works out the checksums of pictures from before they had one, for the files this hub has, so they reach the other
/// hubs with the next changes. Returns how many it filled in.
pub async fn fill_in_checksums(hub: &Hub) -> ApiResult<usize> {
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM media WHERE sha256 IS NULL AND have_file = 1").fetch_all(&hub.pool).await?;
    let mut filled = 0;
    for id in ids {
        let sum = match sha256_file(&hub.media_dir().join(&id)).await {
            Ok(sum) => sum,
            Err(e) => {
                log::warn!("couldn't read picture {id} for its checksum: {e}");
                continue;
            }
        };
        let mut tx = hub.pool.begin().await?;
        let rev = crate::db::next_rev(&mut tx).await?;
        sqlx::query("UPDATE media SET sha256 = ?, rev = ? WHERE id = ? AND sha256 IS NULL")
            .bind(&sum)
            .bind(rev)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        filled += 1;
    }
    if filled > 0 {
        log::info!("worked out the checksums of {filled} older pictures and videos");
    }
    Ok(filled)
}

/// A lock for each file being fetched or delivered, by its part file.
type Locks = Mutex<Option<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>>;

/// One fetch or delivery of a file at a time, so two don't write to the same part file.
async fn file_lock(hub: &Hub, id: &str) -> FileLock {
    static LOCKS: Locks = Mutex::new(None);
    let key = part_path(hub, id);
    let lock = LOCKS.lock().unwrap().get_or_insert_with(HashMap::new).entry(key.clone()).or_default().clone();
    let guard = lock.clone().lock_owned().await;
    FileLock { _guard: guard, key, lock, locks: &LOCKS }
}

struct FileLock {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    key: PathBuf,
    lock: Arc<tokio::sync::Mutex<()>>,
    locks: &'static Locks,
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let mut locks = self.locks.lock().unwrap();
        // Ours, the map's, and the guard's: nobody else is waiting.
        if Arc::strong_count(&self.lock) <= 3
            && let Some(map) = locks.as_mut()
        {
            map.remove(&self.key);
        }
    }
}

/// Where a file coming from another hub collects until it's whole.
fn part_path(hub: &Hub, id: &str) -> PathBuf {
    hub.uploads_dir().join(format!("hub-{id}"))
}

async fn part_len(hub: &Hub, id: &str) -> u64 {
    tokio::fs::metadata(part_path(hub, id)).await.map(|m| m.len()).unwrap_or(0)
}

/// A picture's size, and whether this hub has its file.
async fn record(hub: &Hub, id: &str) -> ApiResult<Option<(u64, bool)>> {
    let row: Option<(i64, bool)> =
        sqlx::query_as("SELECT size, have_file FROM media WHERE id = ?").bind(id).fetch_optional(&hub.pool).await?;
    Ok(row.map(|(size, have)| (size as u64, have)))
}

/// Makes a whole part file this picture's file, if it matches the picture's checksum. One that doesn't is thrown
/// away (`BadRequest`) and not tried again in the background for a day.
async fn finish(hub: &Hub, id: &str, copied: bool, kept_at: i64) -> ApiResult<()> {
    let part = part_path(hub, id);
    let expected: Option<String> =
        sqlx::query_scalar("SELECT sha256 FROM media WHERE id = ?").bind(id).fetch_one(&hub.pool).await?;
    let sum = sha256_file(&part).await?;
    if expected.as_ref().is_some_and(|expected| *expected != sum) {
        let _ = tokio::fs::remove_file(&part).await;
        failed(hub, id).await?;
        return Err(ApiError::bad(format!("Picture {id} arrived damaged (its checksum doesn't match).")));
    }
    tokio::fs::rename(&part, hub.media_dir().join(id)).await?;
    sqlx::query(
        "UPDATE media SET have_file = 1, copied = ?, kept_at = ?, sha256 = COALESCE(sha256, ?), copy_failed_at = 0
         WHERE id = ?",
    )
    .bind(copied)
    .bind(kept_at)
    .bind(&sum)
    .bind(id)
    .execute(&hub.pool)
    .await?;
    Ok(())
}

/// Notes that copying a file came out wrong, so the background rounds leave it for a day.
async fn failed(hub: &Hub, id: &str) -> ApiResult<()> {
    sqlx::query("UPDATE media SET copy_failed_at = ? WHERE id = ?").bind(now_ms()).bind(id).execute(&hub.pool).await?;
    Ok(())
}

// ---- On the hub that's called (home) ----------------------------------------------------------------------------

/// `GET /hubs/files/{id}`: a file, for a paired hub. Answers Range requests.
pub async fn get(
    State(hub): State<Arc<Hub>>,
    _peer: Peer,
    Path(id): Path<String>,
    req: Request,
) -> ApiResult<Response> {
    crate::routes::check_id(&id)?;
    match record(&hub, &id).await? {
        Some((_, true)) => crate::routes::serve_media(&hub, &id, req).await,
        _ => Err(ApiError::NotFound("This hub doesn't have that file.".into())),
    }
}

/// `POST /hubs/files/wanted`: which of these files this hub still needs.
pub async fn wanted(
    State(hub): State<Arc<Hub>>,
    _peer: Peer,
    Json(req): Json<FilesWanted>,
) -> ApiResult<Json<WantedFiles>> {
    if req.ids.len() > ROUND as usize {
        return Err(ApiError::bad("Too many files at once."));
    }
    let mut answer = WantedFiles { wanted: Vec::new(), unknown: Vec::new() };
    for id in req.ids {
        crate::routes::check_id(&id)?;
        match record(&hub, &id).await? {
            None => answer.unknown.push(id),
            Some((_, true)) => {}
            Some((_, false)) => {
                let received = part_len(&hub, &id).await;
                answer.wanted.push(FileProgress { id, received });
            }
        }
    }
    Ok(Json(answer))
}

#[derive(Deserialize)]
pub struct PieceAt {
    offset: u64,
}

/// `PUT /hubs/files/{id}?offset=N`: a piece of a file a paired hub has and this one doesn't. A piece at offset 0
/// starts the file over.
pub async fn put(
    State(hub): State<Arc<Hub>>,
    _peer: Peer,
    Path(id): Path<String>,
    Query(at): Query<PieceAt>,
    body: Bytes,
) -> ApiResult<Json<FileProgress>> {
    crate::routes::check_id(&id)?;
    let (size, have) = record(&hub, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound("This hub hasn't heard of that picture yet.".into()))?;
    if have {
        return Ok(Json(FileProgress { id, received: size }));
    }
    let _lock = file_lock(&hub, &id).await;
    let received = if at.offset == 0 { 0 } else { part_len(&hub, &id).await };
    if at.offset != received {
        return Err(ApiError::Conflict(format!("This hub has {received} bytes of that file, not {}.", at.offset)));
    }
    if body.is_empty() || received + body.len() as u64 > size {
        return Err(ApiError::bad("That piece doesn't fit the file."));
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(at.offset == 0)
        .open(part_path(&hub, &id))
        .await?;
    file.seek(std::io::SeekFrom::Start(received)).await?;
    file.write_all(&body).await?;
    file.flush().await?;
    drop(file);
    let received = received + body.len() as u64;
    if received == size {
        let created_at: i64 =
            sqlx::query_scalar("SELECT created_at FROM media WHERE id = ?").bind(&id).fetch_one(&hub.pool).await?;
        finish(&hub, &id, true, created_at).await?;
    }
    Ok(Json(FileProgress { id, received }))
}

// ---- On the hub that calls (travel) -----------------------------------------------------------------------------

/// Sends the other hub one round of the files uploaded here that it lacks, then copies one round of its files here,
/// and makes room if needed. Returns how many files moved.
pub(crate) async fn copy_files(hub: &Hub, remote: &Remote, peer: &Dialed) -> ApiResult<usize> {
    let sent = send_files(hub, remote, peer).await?;
    let fetched = fetch_files(hub, remote).await?;
    make_room(hub).await?;
    Ok(sent + fetched)
}

async fn send_files(hub: &Hub, remote: &Remote, peer: &Dialed) -> ApiResult<usize> {
    let ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM media WHERE have_file = 1 AND copied = 0 AND copy_failed_at < ?
           AND id NOT IN (SELECT media_id FROM peer_files WHERE hub_id = ?)
         ORDER BY created_at LIMIT ?",
    )
    .bind(now_ms() - DAY_MS)
    .bind(&peer.hub_id)
    .bind(ROUND)
    .fetch_all(&hub.pool)
    .await?;
    if ids.is_empty() {
        return Ok(0);
    }
    let answer: WantedFiles = remote.post("/hubs/files/wanted", &FilesWanted { ids: ids.clone() }).await?;
    let mut sent = 0;
    for id in &ids {
        if answer.unknown.contains(id) {
            continue;
        }
        if let Some(want) = answer.wanted.iter().find(|w| &w.id == id) {
            match send_file(hub, remote, id, want.received).await {
                Ok(()) => sent += 1,
                // Its record went missing there, or it was changed meanwhile: next round.
                Err(ApiError::NotFound(_) | ApiError::Conflict(_)) => continue,
                Err(ApiError::BadRequest(why)) => {
                    log::warn!("{} didn't take picture {id}: {why}", remote.base);
                    failed(hub, id).await?;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
        sqlx::query("INSERT OR IGNORE INTO peer_files (hub_id, media_id) VALUES (?, ?)")
            .bind(&peer.hub_id)
            .bind(id)
            .execute(&hub.pool)
            .await?;
    }
    Ok(sent)
}

async fn send_file(hub: &Hub, remote: &Remote, id: &str, received: u64) -> ApiResult<()> {
    let mut file = tokio::fs::File::open(hub.media_dir().join(id)).await?;
    let len = file.metadata().await?.len();
    let mut offset = if received < len { received } else { 0 };
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    while offset < len {
        let mut piece = vec![0u8; PIECE.min(len - offset) as usize];
        file.read_exact(&mut piece).await?;
        let req = remote
            .request(reqwest::Method::PUT, &format!("/hubs/files/{id}?offset={offset}"))
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .timeout(PIECE_TIME)
            .body(piece);
        let progress: FileProgress = remote.send(req).await?;
        if progress.received <= offset {
            return Err(ApiError::Internal(format!("{} didn't take a piece of {id}", remote.base)));
        }
        offset = progress.received;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
    }
    Ok(())
}

async fn fetch_files(hub: &Hub, remote: &Remote) -> ApiResult<usize> {
    let keep = hub.keep_files;
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT id, size, created_at FROM media WHERE have_file = 0 AND created_at >= ? AND copy_failed_at < ?
         ORDER BY created_at DESC LIMIT ?",
    )
    .bind(now_ms() - i64::from(keep.days) * DAY_MS)
    .bind(now_ms() - DAY_MS)
    .bind(ROUND)
    .fetch_all(&hub.pool)
    .await?;
    let mut fetched = 0;
    for (id, size, created_at) in rows {
        if copied_bytes(hub).await? + size as u64 > keep.max_bytes {
            continue;
        }
        match fetch_file(hub, remote, &id, created_at).await {
            Ok(true) => fetched += 1,
            Ok(false) => {}
            // The other hub doesn't have it either, yet.
            Err(ApiError::NotFound(_) | ApiError::Unavailable(_)) => {}
            Err(ApiError::BadRequest(why)) => log::warn!("from {}: {why}", remote.base),
            Err(e) => return Err(e),
        }
    }
    Ok(fetched)
}

async fn copied_bytes(hub: &Hub) -> ApiResult<u64> {
    let n: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(size), 0) FROM media WHERE have_file = 1 AND copied = 1")
        .fetch_one(&hub.pool)
        .await?;
    Ok(n as u64)
}

/// Copies one file from the other hub, carrying on from a part file left last time. False if it arrived meanwhile.
async fn fetch_file(hub: &Hub, remote: &Remote, id: &str, kept_at: i64) -> ApiResult<bool> {
    let _lock = file_lock(hub, id).await;
    let Some((size, false)) = record(hub, id).await? else { return Ok(false) };
    let part = part_path(hub, id);
    let mut have = part_len(hub, id).await;
    if have > size {
        tokio::fs::remove_file(&part).await?;
        have = 0;
    }
    while have < size {
        let last = (have + PIECE).min(size) - 1;
        let req = remote
            .request(reqwest::Method::GET, &format!("/hubs/files/{id}"))
            .header(reqwest::header::RANGE, format!("bytes={have}-{last}"))
            .timeout(PIECE_TIME);
        let res = remote.send_raw(req).await?;
        let status = res.status();
        if !status.is_success() {
            return Err(remote.error(res).await);
        }
        let whole = status != reqwest::StatusCode::PARTIAL_CONTENT;
        let bytes = res.bytes().await.map_err(|e| ApiError::Internal(format!("{}: {e}", remote.base)))?;
        if bytes.is_empty() {
            return Err(ApiError::Internal(format!("{} sent an empty piece of {id}", remote.base)));
        }
        let mut file =
            tokio::fs::OpenOptions::new().create(true).append(!whole).write(true).truncate(whole).open(&part).await?;
        file.write_all(&bytes).await?;
        file.flush().await?;
        have = if whole { bytes.len() as u64 } else { have + bytes.len() as u64 };
        if whole && have != size {
            break;
        }
    }
    if have != size {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(ApiError::Internal(format!("{id} from {} isn't the size it should be", remote.base)));
    }
    finish(hub, id, true, kept_at).await?;
    Ok(true)
}

/// Deletes the copies kept longest until other hubs' files fit in [`Keep::max_bytes`] again.
pub(crate) async fn make_room(hub: &Hub) -> ApiResult<()> {
    let mut used = copied_bytes(hub).await?;
    if used <= hub.keep_files.max_bytes {
        return Ok(());
    }
    let oldest: Vec<(String, i64)> =
        sqlx::query_as("SELECT id, size FROM media WHERE have_file = 1 AND copied = 1 ORDER BY kept_at, id")
            .fetch_all(&hub.pool)
            .await?;
    for (id, size) in oldest {
        if used <= hub.keep_files.max_bytes {
            break;
        }
        // A file being sent to a device can't be deleted on Windows: skip it this time.
        match tokio::fs::remove_file(hub.media_dir().join(&id)).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => continue,
        }
        sqlx::query("UPDATE media SET have_file = 0 WHERE id = ?").bind(&id).execute(&hub.pool).await?;
        used = used.saturating_sub(size as u64);
    }
    Ok(())
}

/// Fetches a picture someone opened from the hubs this one calls. False if none of them could give it.
pub(crate) async fn fetch_now(hub: &Hub, id: &str) -> ApiResult<bool> {
    for hub_id in dialed_ids(hub).await? {
        let Some(peer) = dialed(hub, &hub_id).await? else { continue };
        let remote = remote_for(hub, &peer)?;
        let fetched = async {
            remote.prove(&hub.hub_id, &peer.secret).await?;
            fetch_file(hub, &remote, id, now_ms()).await
        }
        .await;
        match fetched {
            Ok(_) => {
                make_room(hub).await?;
                if matches!(record(hub, id).await?, Some((_, true))) {
                    return Ok(true);
                }
            }
            Err(e) => log::info!("couldn't fetch picture {id} from {}: {e}", remote.base),
        }
    }
    Ok(false)
}
