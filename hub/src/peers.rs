//! Pairing a family's hubs and keeping them in step over the network (docs/multi_hub_plan.md).
//!
//! A parent makes a hub code (`POST /hubs/codes`, or `kitchat-hub pair-code` on the home hub). On the travel hub,
//! `kitchat-hub join <home address> <code>` pairs the two: the home hub makes a secret they share, which only the
//! travel hub keeps as it is (it has to send it; home keeps its hash), and the travel hub copies the family.
//!
//! From then on the travel hub calls home whenever it can; home can't call it, behind the campground's network. It asks
//! for home's changes and home holds the question until there are some (a long poll), so they arrive within moments;
//! and it sends its own as soon as they happen. Only the travel hub keeps track of how far it has got each way, so
//! either hub can stop and start again at any time, and taking in the same changes twice does nothing
//! ([`crate::replica`]).
//!
//! An unencrypted (`http`) address could be a stranger's machine on another network, so before sending anything there
//! the travel hub asks it to show it knows the secret, the way devices check their hub ([`kit_protocol::proof`]).

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use kit_protocol::hubs::{Applied, Changes, HubCode, HubProveRequest, PairRequest, Paired, PeerInfo};
use kit_protocol::proof::{HubProof, hub_proof};
use kit_protocol::{ErrorBody, Hello};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::broadcast;

use crate::auth::Authed;
use crate::error::{ApiError, ApiResult};
use crate::limits::{ClientIp, is_home_or_tailnet};
use crate::{Hub, db, hub_files, now_ms, random, replica};

/// How long a hub code works.
const CODE_LIFETIME_MS: i64 = 10 * 60 * 1000;
/// How long home holds a request for changes when it has none. Well under the hub's request time limit.
pub const WAIT: Duration = Duration::from_secs(20);
/// How often the travel hub sends its changes even when it hasn't noticed any.
const PUSH_AT_LEAST_EVERY: Duration = Duration::from_secs(60);
/// How often the travel hub looks for pictures and videos to move, when there were none last time.
const FILES_EVERY: Duration = Duration::from_secs(10);
const FIRST_RETRY: Duration = Duration::from_secs(2);
const LONGEST_RETRY: Duration = Duration::from_secs(60);

// ---- On the hub that's joined (home) ----------------------------------------------------------------------------

/// A new hub code, for a parent's app or `kitchat-hub pair-code`.
pub async fn make_code(hub: &Hub) -> ApiResult<HubCode> {
    if db::family_id(&hub.pool).await?.is_none() {
        return Err(ApiError::Conflict("Set the family up on this hub first.".into()));
    }
    let now = now_ms();
    sqlx::query("DELETE FROM hub_codes WHERE expires_at < ? OR used = 1").bind(now).execute(&hub.pool).await?;
    let expires_at = now + CODE_LIFETIME_MS;
    for _ in 0..5 {
        let code = random::enroll_code();
        let inserted = sqlx::query("INSERT OR IGNORE INTO hub_codes (code, expires_at) VALUES (?, ?)")
            .bind(&code)
            .bind(expires_at)
            .execute(&hub.pool)
            .await?;
        if inserted.rows_affected() == 1 {
            return Ok(HubCode { code, expires_at });
        }
    }
    Err(ApiError::Internal("couldn't make a unique hub code".into()))
}

/// `POST /hubs/codes`
pub async fn new_code(State(hub): State<Arc<Hub>>, authed: Authed) -> ApiResult<Json<HubCode>> {
    authed.require_parent()?;
    let code = make_code(&hub).await?;
    log::info!("{} made a code for pairing another hub", authed.member.name);
    Ok(Json(code))
}

/// `GET /hubs`: the family's other hubs this one knows.
pub async fn list(State(hub): State<Arc<Hub>>, authed: Authed) -> ApiResult<Json<Vec<PeerInfo>>> {
    authed.require_parent()?;
    let rows: Vec<(String, String, Option<String>, i64)> =
        sqlx::query_as("SELECT hub_id, name, url, last_synced FROM peers ORDER BY created_at")
            .fetch_all(&hub.pool)
            .await?;
    Ok(Json(
        rows.into_iter().map(|(hub_id, name, url, last_synced)| PeerInfo { hub_id, name, url, last_synced }).collect(),
    ))
}

/// `DELETE /hubs/{id}`: stops keeping in step with that hub. What it already has stays there.
pub async fn remove(State(hub): State<Arc<Hub>>, authed: Authed, Path(id): Path<String>) -> ApiResult<Json<()>> {
    authed.require_parent()?;
    let done = sqlx::query("DELETE FROM peers WHERE hub_id = ?").bind(&id).execute(&hub.pool).await?;
    sqlx::query("DELETE FROM peer_files WHERE hub_id = ?").bind(&id).execute(&hub.pool).await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound("This hub doesn't know that hub.".into()));
    }
    log::info!("{} stopped keeping this hub in step with hub {id}", authed.member.name);
    Ok(Json(()))
}

/// `POST /hubs/pair`: another hub joins with a code. Limited like joining codes for devices.
pub async fn pair(
    State(hub): State<Arc<Hub>>,
    ClientIp(ip): ClientIp,
    Json(req): Json<PairRequest>,
) -> ApiResult<Json<Paired>> {
    hub.limits.check_code(ip)?;
    hub.guard.check()?;
    let family_id =
        db::family_id(&hub.pool).await?.ok_or_else(|| ApiError::Conflict("This hub isn't set up yet.".into()))?;
    ulid::Ulid::from_string(&req.hub_id).map_err(|_| ApiError::bad("That hub ID isn't valid."))?;
    if req.hub_id == hub.hub_id {
        return Err(ApiError::bad("A hub can't pair with itself."));
    }
    let name = crate::routes::clean_name(&req.name, 40, "hub name")?;
    let used: Option<String> = sqlx::query_scalar(
        "UPDATE hub_codes SET used = 1 WHERE code = ? AND used = 0 AND expires_at > ? RETURNING code",
    )
    .bind(random::clean_code(&req.code))
    .bind(now_ms())
    .fetch_optional(&hub.pool)
    .await?;
    if used.is_none() {
        hub.guard.failed();
        hub.limits.wrong_code(ip);
        return Err(ApiError::bad("That hub code didn't work. Codes work once, for 10 minutes: make a new one."));
    }
    let secret = random::token();
    // Pairing again (after a reinstall, say) replaces the secret, and the other hub starts its positions over.
    sqlx::query(
        "INSERT INTO peers (hub_id, name, url, secret, secret_hash, created_at) VALUES (?, ?, NULL, NULL, ?, ?)
         ON CONFLICT (hub_id) DO UPDATE SET name = excluded.name, secret_hash = excluded.secret_hash",
    )
    .bind(&req.hub_id)
    .bind(&name)
    .bind(random::hash_token(&secret))
    .bind(now_ms())
    .execute(&hub.pool)
    .await?;
    log::info!("hub \"{name}\" ({}) is now one of the family's hubs", req.hub_id);
    Ok(Json(Paired { hub_id: hub.hub_id.clone(), family_id, secret }))
}

/// `POST /hubs/prove`: shows a paired hub that this is the hub it paired with. Only for hubs on the home network or
/// Tailscale, the only places an unencrypted address is used, for the reason `/prove` gives.
pub async fn prove(
    State(hub): State<Arc<Hub>>,
    ClientIp(ip): ClientIp,
    Json(req): Json<HubProveRequest>,
) -> ApiResult<Json<HubProof>> {
    if !is_home_or_tailnet(ip) {
        return Err(ApiError::Forbidden("This only works from the home network or Tailscale.".into()));
    }
    if !(32..=128).contains(&req.nonce.len()) {
        return Err(ApiError::bad("That nonce isn't valid."));
    }
    let secret_hash: Option<String> = sqlx::query_scalar("SELECT secret_hash FROM peers WHERE hub_id = ?")
        .bind(&req.hub_id)
        .fetch_optional(&hub.pool)
        .await?;
    let secret_hash = secret_hash.ok_or_else(|| ApiError::NotFound("This hub isn't paired with that hub.".into()))?;
    Ok(Json(HubProof { proof: hub_proof(&secret_hash, &req.hub_id, &req.nonce) }))
}

/// A paired hub, from `Authorization: Hub <hub ID>:<secret>`.
pub struct Peer {
    pub hub_id: String,
}

impl FromRequestParts<Arc<Hub>> for Peer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, hub: &Arc<Hub>) -> Result<Self, Self::Rejection> {
        let given = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Hub "))
            .and_then(|v| v.trim().split_once(':'));
        let peer = match given {
            Some((hub_id, secret)) => {
                let stored: Option<String> = sqlx::query_scalar("SELECT secret_hash FROM peers WHERE hub_id = ?")
                    .bind(hub_id)
                    .fetch_optional(&hub.pool)
                    .await?;
                stored.filter(|hash| random::same(hash, &random::hash_token(secret))).map(|_| hub_id.to_string())
            }
            None => None,
        };
        let Some(hub_id) = peer else {
            if let Some(ClientIp(ip)) = parts.extensions.get::<ClientIp>() {
                hub.limits.bad_token(*ip);
            }
            return Err(ApiError::Unauthorized);
        };
        hub.limits.device_request(&format!("hub:{hub_id}"))?;
        sqlx::query("UPDATE peers SET last_synced = ? WHERE hub_id = ?")
            .bind(now_ms())
            .bind(&hub_id)
            .execute(&hub.pool)
            .await?;
        Ok(Peer { hub_id })
    }
}

#[derive(Deserialize)]
pub struct ChangesQuery {
    #[serde(default)]
    since: i64,
    /// Hold the request (up to [`WAIT`]) until there's something new.
    #[serde(default)]
    wait: bool,
}

/// `GET /hubs/changes?since=N[&wait=true]`: what changed here, for a paired hub.
pub async fn changes(
    State(hub): State<Arc<Hub>>,
    _peer: Peer,
    Query(q): Query<ChangesQuery>,
) -> ApiResult<Json<Changes>> {
    if q.wait {
        // Listening before looking, so a change made in between isn't slept through.
        let mut live = hub.live.subscribe();
        let mut stopping = hub.stopping.subscribe();
        let deadline = tokio::time::Instant::now() + WAIT;
        // Not every change is announced on /live (a new device, a picture), so also look now and then.
        while db::current_rev(&hub.pool).await? <= q.since {
            tokio::select! {
                _ = live.recv() => {}
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                _ = tokio::time::sleep_until(deadline) => break,
                _ = stopping.wait_for(|s| *s) => break,
            }
        }
    }
    Ok(Json(replica::changes_since(&hub, q.since, replica::BATCH).await?))
}

/// `POST /hubs/changes`: a paired hub's changes.
pub async fn take_changes(
    State(hub): State<Arc<Hub>>,
    peer: Peer,
    Json(changes): Json<Changes>,
) -> ApiResult<Json<Applied>> {
    if changes.hub_id != peer.hub_id {
        return Err(ApiError::Forbidden("Those changes are from another hub.".into()));
    }
    let changed = replica::apply(&hub, changes).await?;
    Ok(Json(Applied { changed }))
}

// ---- On the hub that joins (travel) -----------------------------------------------------------------------------

/// Checks and tidies the address of the hub to call, as the app does: `http` only on the home network, inside
/// Tailscale or on this computer; without a scheme, `http` there and `https` elsewhere.
pub fn normalize_url(raw: &str) -> ApiResult<String> {
    let raw = raw.trim().trim_end_matches('/');
    let bad = || ApiError::bad(format!("{raw} doesn't look like a hub's address."));
    let with_scheme = if raw.contains("://") {
        raw.to_string()
    } else {
        let host = url::Url::parse(&format!("http://{raw}")).map_err(|_| bad())?;
        let local = host.host_str().is_some_and(looks_local);
        format!("{}://{raw}", if local { "http" } else { "https" })
    };
    let url = url::Url::parse(&with_scheme).map_err(|_| bad())?;
    let host = url.host_str().ok_or_else(bad)?;
    match url.scheme() {
        "https" => {}
        "http" if looks_local(host) => {}
        "http" => return Err(ApiError::bad("A hub outside the home network needs an https:// address.")),
        _ => return Err(bad()),
    }
    Ok(with_scheme)
}

fn looks_local(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    match host.parse::<IpAddr>() {
        Ok(ip) => ip.is_loopback() || is_home_or_tailnet(ip),
        Err(_) => {
            let host = host.to_ascii_lowercase();
            host == "localhost" || !host.contains('.') || host.ends_with(".local") || host.ends_with(".ts.net")
        }
    }
}

/// Plain http to anywhere but this computer: nothing shows who is really answering.
fn needs_proof(base: &str) -> bool {
    let Ok(url) = url::Url::parse(base) else { return true };
    let host = url.host_str().unwrap_or_default().trim_start_matches('[').trim_end_matches(']');
    let on_this_computer =
        host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
    url.scheme() == "http" && !on_this_computer
}

fn http() -> ApiResult<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        // Longer than home holds a request for changes.
        .timeout(WAIT + Duration::from_secs(20))
        .user_agent(concat!("KitChatHub/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| ApiError::Internal(format!("http client: {e}")))
}

/// The other hub, as seen from the hub that calls it.
pub(crate) struct Remote {
    http: reqwest::Client,
    pub(crate) base: String,
    /// "Hub <this hub's ID>:<secret>", or nothing before pairing.
    auth: Option<String>,
}

impl Remote {
    /// A request to `path` on the other hub, signed in as this hub once paired.
    pub(crate) fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let req = self.http.request(method, format!("{}{path}", self.base));
        match &self.auth {
            Some(auth) => req.header(reqwest::header::AUTHORIZATION, auth),
            None => req,
        }
    }

    pub(crate) async fn send_raw(&self, req: reqwest::RequestBuilder) -> ApiResult<reqwest::Response> {
        req.send().await.map_err(|e| ApiError::Internal(format!("couldn't reach {}: {e}", self.base)))
    }

    /// What went wrong, from an answer that isn't a success: "not here" and "not yet" stay `NotFound` and
    /// `Unavailable`, so the caller can skip a file and carry on.
    pub(crate) async fn error(&self, res: reqwest::Response) -> ApiError {
        let status = res.status();
        let bytes = res.bytes().await.unwrap_or_default();
        let why = serde_json::from_slice::<ErrorBody>(&bytes).map(|b| b.error).unwrap_or_else(|_| status.to_string());
        match status.as_u16() {
            401 => ApiError::Internal(format!("{} doesn't know this hub any more: pair them again ({why})", self.base)),
            404 => ApiError::NotFound(why),
            409 => ApiError::Conflict(why),
            503 => ApiError::Unavailable(why),
            _ => ApiError::Internal(format!("{}: {why}", self.base)),
        }
    }

    pub(crate) async fn send<T: DeserializeOwned>(&self, req: reqwest::RequestBuilder) -> ApiResult<T> {
        let res = self.send_raw(req).await?;
        if !res.status().is_success() {
            return Err(self.error(res).await);
        }
        let bytes = res.bytes().await.map_err(|e| ApiError::Internal(format!("{}: {e}", self.base)))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        self.send(self.request(reqwest::Method::GET, path)).await
    }

    pub(crate) async fn post<T: DeserializeOwned>(&self, path: &str, body: &impl serde::Serialize) -> ApiResult<T> {
        let body = serde_json::to_vec(body)?;
        let req = self.request(reqwest::Method::POST, path).header(reqwest::header::CONTENT_TYPE, "application/json");
        self.send(req.body(body)).await
    }

    /// Makes sure an unencrypted address is the hub this one paired with, before anything is sent there.
    pub(crate) async fn prove(&self, our_hub_id: &str, secret: &str) -> ApiResult<()> {
        if !needs_proof(&self.base) {
            return Ok(());
        }
        let nonce = random::token();
        let proof: HubProof = Remote { auth: None, http: self.http.clone(), base: self.base.clone() }
            .post("/hubs/prove", &HubProveRequest { hub_id: our_hub_id.to_string(), nonce: nonce.clone() })
            .await?;
        let expected = hub_proof(&random::hash_token(secret), our_hub_id, &nonce);
        if !random::same(&proof.proof, &expected) {
            return Err(ApiError::Internal(format!("{} isn't the hub this one paired with", self.base)));
        }
        Ok(())
    }
}

/// A hub this one calls, as stored.
pub(crate) struct Dialed {
    pub(crate) hub_id: String,
    name: String,
    url: String,
    pub(crate) secret: String,
}

pub(crate) async fn dialed(hub: &Hub, hub_id: &str) -> ApiResult<Option<Dialed>> {
    let row: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT hub_id, name, url, secret FROM peers WHERE hub_id = ? AND url IS NOT NULL AND secret IS NOT NULL",
    )
    .bind(hub_id)
    .fetch_optional(&hub.pool)
    .await?;
    Ok(row.map(|(hub_id, name, url, secret)| Dialed { hub_id, name, url, secret }))
}

pub(crate) async fn dialed_ids(hub: &Hub) -> ApiResult<Vec<String>> {
    Ok(sqlx::query_scalar("SELECT hub_id FROM peers WHERE url IS NOT NULL AND secret IS NOT NULL")
        .fetch_all(&hub.pool)
        .await?)
}

pub(crate) fn remote_for(hub: &Hub, peer: &Dialed) -> ApiResult<Remote> {
    Ok(Remote { http: http()?, base: peer.url.clone(), auth: Some(format!("Hub {}:{}", hub.hub_id, peer.secret)) })
}

/// Joins the family's hubs through the hub at `address` with a code a parent made there, and copies the family (or,
/// for a hub of the family that was paired before, catches up). Returns how many rows changed here.
pub async fn join(hub: &Hub, address: &str, code: &str, name: &str) -> ApiResult<usize> {
    let base = normalize_url(address)?;
    let remote = Remote { http: http()?, base: base.clone(), auth: None };
    let hello: Hello = remote.get("/hello").await?;
    if hello.app != "kitchat-hub" {
        return Err(ApiError::bad(format!("{base} doesn't look like a KitChat hub.")));
    }
    let theirs = hello.family_id.ok_or_else(|| ApiError::Conflict(format!("The hub at {base} isn't set up yet.")))?;
    if let Some(ours) = db::family_id(&hub.pool).await?
        && ours != theirs
    {
        return Err(ApiError::Conflict(format!(
            "This hub already has a different family. Start it with an empty --data folder to join {base}'s family."
        )));
    }
    let paired: Paired = remote
        .post("/hubs/pair", &PairRequest { code: code.to_string(), hub_id: hub.hub_id.clone(), name: name.to_string() })
        .await?;
    let host = url::Url::parse(&base).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    sqlx::query(
        "INSERT INTO peers (hub_id, name, url, secret, secret_hash, created_at) VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (hub_id) DO UPDATE SET url = excluded.url, secret = excluded.secret,
           secret_hash = excluded.secret_hash, pulled_rev = 0, pushed_rev = 0",
    )
    .bind(&paired.hub_id)
    .bind(format!("Hub at {host}"))
    .bind(&base)
    .bind(&paired.secret)
    .bind(random::hash_token(&paired.secret))
    .bind(now_ms())
    .execute(&hub.pool)
    .await?;
    let peer = dialed(hub, &paired.hub_id).await?.ok_or_else(|| ApiError::Internal("peer vanished".into()))?;
    let remote = remote_for(hub, &peer)?;
    remote.prove(&hub.hub_id, &peer.secret).await?;
    let changed = pull(hub, &remote, &peer, false).await?;
    push(hub, &remote, &peer).await?;
    Ok(changed)
}

/// Takes in the other hub's changes until there are no more. With `wait`, the first request waits for some.
async fn pull(hub: &Hub, remote: &Remote, peer: &Dialed, wait: bool) -> ApiResult<usize> {
    let mut changed = 0;
    let mut wait = wait;
    loop {
        let since: i64 = sqlx::query_scalar("SELECT pulled_rev FROM peers WHERE hub_id = ?")
            .bind(&peer.hub_id)
            .fetch_one(&hub.pool)
            .await?;
        let changes: Changes = remote.get(&format!("/hubs/changes?since={since}&wait={wait}")).await?;
        if changes.hub_id != peer.hub_id {
            return Err(ApiError::Conflict(format!(
                "{} is a different hub now (its data moved?): pair them again",
                peer.url
            )));
        }
        let (rev, more) = (changes.rev, changes.more);
        changed += replica::apply(hub, changes).await?;
        sqlx::query("UPDATE peers SET pulled_rev = ?, last_synced = ? WHERE hub_id = ?")
            .bind(rev)
            .bind(now_ms())
            .bind(&peer.hub_id)
            .execute(&hub.pool)
            .await?;
        if !more {
            return Ok(changed);
        }
        wait = false;
    }
}

/// Sends this hub's changes the other hasn't taken in yet. Returns how many rows changed there.
async fn push(hub: &Hub, remote: &Remote, peer: &Dialed) -> ApiResult<usize> {
    let mut changed = 0;
    loop {
        let since: i64 = sqlx::query_scalar("SELECT pushed_rev FROM peers WHERE hub_id = ?")
            .bind(&peer.hub_id)
            .fetch_one(&hub.pool)
            .await?;
        let changes = replica::changes_since(hub, since, replica::BATCH).await?;
        if changes.rev <= since {
            return Ok(changed);
        }
        let (rev, more) = (changes.rev, changes.more);
        let applied: Applied = remote.post("/hubs/changes", &changes).await?;
        changed += applied.changed;
        sqlx::query("UPDATE peers SET pushed_rev = ?, last_synced = ? WHERE hub_id = ?")
            .bind(rev)
            .bind(now_ms())
            .bind(&peer.hub_id)
            .execute(&hub.pool)
            .await?;
        if !more {
            return Ok(changed);
        }
    }
}

/// Swaps changes once with every hub this one calls: everything new each way, and the pictures and videos to copy.
/// Returns how many rows and files changed. For tests and `kitchat-hub sync`.
pub async fn sync_once(hub: &Hub) -> ApiResult<usize> {
    let mut changed = 0;
    for id in dialed_ids(hub).await? {
        let Some(peer) = dialed(hub, &id).await? else { continue };
        let remote = remote_for(hub, &peer)?;
        remote.prove(&hub.hub_id, &peer.secret).await?;
        changed += pull(hub, &remote, &peer, false).await?;
        changed += push(hub, &remote, &peer).await?;
        loop {
            let moved = hub_files::copy_files(hub, &remote, &peer).await?;
            if moved == 0 {
                break;
            }
            changed += moved;
        }
    }
    Ok(changed)
}

/// Keeps this hub in step with every hub it calls, for as long as it runs: one task per hub, started for hubs paired
/// while it runs too.
pub fn spawn(hub: Arc<Hub>) {
    tokio::spawn(async move {
        let running: Arc<Mutex<HashSet<String>>> = Arc::default();
        let mut stopping = hub.stopping.subscribe();
        loop {
            match dialed_ids(&hub).await {
                Ok(ids) => {
                    for id in ids {
                        if running.lock().unwrap().insert(id.clone()) {
                            tokio::spawn(keep_in_step(hub.clone(), id, running.clone()));
                        }
                    }
                }
                Err(e) => log::warn!("couldn't read the paired hubs: {e}"),
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(60)) => {}
                _ = stopping.wait_for(|s| *s) => return,
            }
        }
    });
}

/// Calls one hub again and again, waiting longer after each failure, until it's unpaired or this hub stops.
async fn keep_in_step(hub: Arc<Hub>, hub_id: String, running: Arc<Mutex<HashSet<String>>>) {
    let mut retry = FIRST_RETRY;
    let mut stopping = hub.stopping.subscribe();
    loop {
        let failed = match dialed(&hub, &hub_id).await {
            Ok(None) => break,
            Ok(Some(peer)) => {
                let result = tokio::select! {
                    r = session(&hub, &peer, &mut retry) => r,
                    _ = stopping.wait_for(|s| *s) => break,
                };
                result.err().map(|e| format!("hub \"{}\" ({}): {e}", peer.name, peer.url))
            }
            Err(e) => Some(format!("couldn't read paired hub {hub_id}: {e}")),
        };
        if let Some(why) = failed {
            log::warn!("{why}; trying again in {} s", retry.as_secs());
        }
        tokio::select! {
            _ = tokio::time::sleep(retry) => {}
            _ = stopping.wait_for(|s| *s) => break,
        }
        retry = (retry * 2).min(LONGEST_RETRY);
    }
    running.lock().unwrap().remove(&hub_id);
}

/// One stretch of keeping in step: catches up both ways, then takes in the other hub's changes as they come and sends
/// this hub's soon after they happen, until something fails.
async fn session(hub: &Hub, peer: &Dialed, retry: &mut Duration) -> ApiResult<()> {
    let remote = remote_for(hub, peer)?;
    remote.prove(&hub.hub_id, &peer.secret).await?;
    pull(hub, &remote, peer, false).await?;
    push(hub, &remote, peer).await?;
    *retry = FIRST_RETRY;
    log::info!("in step with hub \"{}\" at {}", peer.name, peer.url);
    tokio::try_join!(
        keep_pulling(hub, &remote, peer),
        keep_pushing(hub, &remote, peer),
        keep_copying(hub, &remote, peer)
    )?;
    Ok(())
}

/// Moves pictures and videos one round after another, and looks again now and then once there's nothing to move. A
/// file that fails waits for the next round without holding up messages: pulling and pushing notice a lost connection.
async fn keep_copying(hub: &Hub, remote: &Remote, peer: &Dialed) -> ApiResult<()> {
    loop {
        match hub_files::copy_files(hub, remote, peer).await {
            Ok(0) => tokio::time::sleep(FILES_EVERY).await,
            Ok(_) => {}
            Err(e) => {
                log::warn!("pictures and videos with hub \"{}\": {e}", peer.name);
                tokio::time::sleep(FILES_EVERY).await;
            }
        }
    }
}

async fn keep_pulling(hub: &Hub, remote: &Remote, peer: &Dialed) -> ApiResult<()> {
    loop {
        pull(hub, remote, peer, true).await?;
    }
}

async fn keep_pushing(hub: &Hub, remote: &Remote, peer: &Dialed) -> ApiResult<()> {
    let mut live = hub.live.subscribe();
    loop {
        tokio::select! {
            r = live.recv() => {
                if matches!(r, Err(broadcast::error::RecvError::Closed)) {
                    return Ok(());
                }
                // A moment for the rest of a burst of changes, so they go together.
                tokio::time::sleep(Duration::from_millis(300)).await;
                while live.try_recv().is_ok() {}
            }
            _ = tokio::time::sleep(PUSH_AT_LEAST_EVERY) => {}
        }
        push(hub, remote, peer).await?;
    }
}
