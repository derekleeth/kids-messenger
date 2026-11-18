//! Keeping the hub well-behaved on the open internet: who's asking (the client's address, also behind a reverse
//! proxy), how often they may ask, how long a request may take, and the security headers on every answer.
//!
//! The limits are generous for a family's few devices and tight for anyone guessing codes or tokens:
//! - `/hello`, `/setup`, `/enroll` and `/prove`: 10 a minute per address, 5 at once.
//! - Wrong or missing device tokens: 20 per address in 10 minutes, then that address is refused for 15 minutes.
//! - Everything else: 20 a second per device (100 at once) and 40 a second per address.
//! - Wrong codes: 5 per address in 10 minutes, on top of the hub-wide [`crate::auth::CodeGuard`].
//! - `/live`: 3 sockets per device, 64 in all.
//!
//! `--rate-limit-scale` multiplies the request rates and the token-failure allowance (not the wrong-code limits).

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tokio::sync::Semaphore;

use crate::Hub;
use crate::error::ApiError;

/// How the limits are set up.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    /// Believe `X-Real-IP` / `X-Forwarded-For`, but only from a proxy on this computer (a loopback peer).
    pub trust_proxy: bool,
    /// Multiplies the rate limits; 1 is the default.
    pub scale: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self { trust_proxy: false, scale: 1.0 }
    }
}

/// How long a request may take, except `/live` and pictures and videos going up or down.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Requests handled at once.
pub const MAX_CONCURRENT: usize = 256;
/// The biggest WebSocket message (or frame) a device may send on `/live`.
pub const MAX_WS_MESSAGE: usize = 64 * 1024;
pub const LIVE_PER_DEVICE: usize = 3;
pub const LIVE_TOTAL: usize = 64;

const TOKEN_FAILURES: f64 = 20.0;
const TOKEN_FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);
const TOKEN_LOCKOUT: Duration = Duration::from_secs(15 * 60);
pub const CODE_FAILURES_PER_IP: usize = 5;
pub const CODE_FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);
const PRUNE_EVERY: Duration = Duration::from_secs(60);

/// The address a request came from, as worked out by [`guard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientIp(pub IpAddr);

impl<S: Send + Sync> FromRequestParts<S> for ClientIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<ClientIp>().copied().unwrap_or(ClientIp(IpAddr::V4(Ipv4Addr::UNSPECIFIED))))
    }
}

/// The client's address: the socket's peer, or, with `trust_proxy` and a peer on this computer, what the proxy says
/// (`X-Real-IP`, else the right-most `X-Forwarded-For`, the one the proxy itself added).
pub fn client_ip(peer: Option<SocketAddr>, headers: &HeaderMap, trust_proxy: bool) -> IpAddr {
    let peer_ip = peer.map(|p| p.ip()).unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let peer_ip = match peer_ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    };
    if !trust_proxy || !peer_ip.is_loopback() {
        return peer_ip;
    }
    let header_ip = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    if let Some(ip) = header_ip("x-real-ip").and_then(|v| v.trim().parse().ok()) {
        return ip;
    }
    // Several X-Forwarded-For headers count as one list, in order.
    let forwarded: Vec<String> =
        headers.get_all("x-forwarded-for").iter().filter_map(|v| v.to_str().ok()).map(str::to_owned).collect();
    forwarded.iter().flat_map(|v| v.split(',')).next_back().and_then(|v| v.trim().parse().ok()).unwrap_or(peer_ip)
}

/// A device on the home network or inside Tailscale. Not this computer: behind a proxy or tunnel that's where every
/// request from the internet seems to come from.
pub fn is_home_or_tailnet(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            // 100.64.0.0/10: Tailscale's range.
            ip.is_private() || ip.is_link_local() || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_unicast_link_local(),
    }
}

/// Token buckets, one per key, behind a plain mutex: a family hub sees a handful of keys.
pub struct RateLimiter<K> {
    per_sec: f64,
    burst: f64,
    state: Mutex<Buckets<K>>,
}

struct Buckets<K> {
    map: HashMap<K, Bucket>,
    pruned: Instant,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

impl<K: Hash + Eq + Clone> RateLimiter<K> {
    pub fn new(per_sec: f64, burst: f64) -> Self {
        Self {
            per_sec,
            burst: burst.max(1.0),
            state: Mutex::new(Buckets { map: HashMap::new(), pruned: Instant::now() }),
        }
    }

    /// Takes one token for `key`, or says how long until one is there.
    pub fn check(&self, key: &K) -> Result<(), Duration> {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        if now.duration_since(state.pruned) > PRUNE_EVERY {
            state.pruned = now;
            let (per_sec, burst) = (self.per_sec, self.burst);
            // A bucket that has filled up again is the same as no bucket.
            state.map.retain(|_, b| b.tokens + now.duration_since(b.at).as_secs_f64() * per_sec < burst);
        }
        let bucket = state.map.entry(key.clone()).or_insert(Bucket { tokens: self.burst, at: now });
        bucket.tokens = (bucket.tokens + now.duration_since(bucket.at).as_secs_f64() * self.per_sec).min(self.burst);
        bucket.at = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - bucket.tokens) / self.per_sec))
        }
    }
}

/// Counts failures per address in a window; enough of them and the address is refused for a while.
struct Lockouts {
    max: usize,
    window: Duration,
    lockout: Duration,
    state: Mutex<(HashMap<IpAddr, Failures>, Instant)>,
}

#[derive(Default)]
struct Failures {
    times: Vec<Instant>,
    until: Option<Instant>,
}

impl Lockouts {
    fn new(max: usize, window: Duration, lockout: Duration) -> Self {
        Self { max: max.max(1), window, lockout, state: Mutex::new((HashMap::new(), Instant::now())) }
    }

    /// How long `ip` is still refused for, if it is.
    fn refused(&self, ip: IpAddr) -> Option<Duration> {
        let now = Instant::now();
        let state = self.state.lock().unwrap();
        state.0.get(&ip)?.until.and_then(|until| until.checked_duration_since(now)).filter(|d| !d.is_zero())
    }

    /// Notes a failure; true if this one locked the address out.
    fn failed(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let mut state = self.state.lock().unwrap();
        let (map, pruned) = &mut *state;
        if now.duration_since(*pruned) > PRUNE_EVERY {
            *pruned = now;
            map.retain(|_, f| {
                f.times.retain(|t| now.duration_since(*t) < self.window);
                !f.times.is_empty() || f.until.is_some_and(|u| u > now)
            });
        }
        let f = map.entry(ip).or_default();
        f.times.retain(|t| now.duration_since(*t) < self.window);
        f.times.push(now);
        if f.times.len() >= self.max && f.until.is_none_or(|u| u <= now) {
            f.until = Some(now + self.lockout);
            f.times.clear();
            true
        } else {
            false
        }
    }
}

/// Every limit the hub keeps, shared by all requests.
pub struct Limits {
    pub config: Config,
    open_paths: RateLimiter<IpAddr>,
    per_device: RateLimiter<String>,
    per_ip: RateLimiter<IpAddr>,
    bad_tokens: Lockouts,
    wrong_codes: Lockouts,
    live: Mutex<(HashMap<String, usize>, usize)>,
    concurrent: Arc<Semaphore>,
}

impl Limits {
    pub fn new(config: Config) -> Self {
        let s = if config.scale.is_finite() && config.scale > 0.0 { config.scale } else { 1.0 };
        Self {
            config: Config { scale: s, ..config },
            open_paths: RateLimiter::new(10.0 / 60.0 * s, 5.0 * s),
            per_device: RateLimiter::new(20.0 * s, 100.0 * s),
            per_ip: RateLimiter::new(40.0 * s, 200.0 * s),
            bad_tokens: Lockouts::new((TOKEN_FAILURES * s).round() as usize, TOKEN_FAILURE_WINDOW, TOKEN_LOCKOUT),
            wrong_codes: Lockouts::new(CODE_FAILURES_PER_IP, CODE_FAILURE_WINDOW, CODE_FAILURE_WINDOW),
            live: Mutex::new((HashMap::new(), 0)),
            concurrent: Arc::new(Semaphore::new(MAX_CONCURRENT)),
        }
    }

    /// Called for every request a device makes with its token.
    pub fn device_request(&self, device_id: &str) -> Result<(), ApiError> {
        self.per_device.check(&device_id.to_owned()).map_err(ApiError::RateLimited)
    }

    /// A request came with a wrong or missing token.
    pub fn bad_token(&self, ip: IpAddr) {
        if self.bad_tokens.failed(ip) {
            log::warn!(
                "{ip} sent too many wrong device tokens; refusing it for {} minutes",
                TOKEN_LOCKOUT.as_secs() / 60
            );
        }
    }

    /// Whether `ip` may try another setup or join code.
    pub fn check_code(&self, ip: IpAddr) -> Result<(), ApiError> {
        if self.wrong_codes.refused(ip).is_some() { Err(ApiError::TooManyAttempts) } else { Ok(()) }
    }

    pub fn wrong_code(&self, ip: IpAddr) {
        if self.wrong_codes.failed(ip) {
            log::warn!("{ip} typed too many wrong codes; refusing its codes for a while");
        }
    }

    /// A place for one more `/live` socket for `device_id`, given back when the [`LiveSlot`] is dropped.
    pub fn live_slot(self: &Arc<Self>, device_id: &str) -> Option<LiveSlot> {
        let mut live = self.live.lock().unwrap();
        let (per_device, total) = &mut *live;
        let n = per_device.entry(device_id.to_owned()).or_default();
        if *n >= LIVE_PER_DEVICE || *total >= LIVE_TOTAL {
            if *n == 0 {
                per_device.remove(device_id);
            }
            return None;
        }
        *n += 1;
        *total += 1;
        Some(LiveSlot { limits: self.clone(), device_id: device_id.to_owned() })
    }
}

/// One open `/live` socket, counted until it's dropped.
pub struct LiveSlot {
    limits: Arc<Limits>,
    device_id: String,
}

impl Drop for LiveSlot {
    fn drop(&mut self) {
        let mut live = self.limits.live.lock().unwrap();
        let (per_device, total) = &mut *live;
        *total = total.saturating_sub(1);
        if let Some(n) = per_device.get_mut(&self.device_id) {
            *n -= 1;
            if *n == 0 {
                per_device.remove(&self.device_id);
            }
        }
    }
}

fn too_many(wait: Duration) -> Response {
    ApiError::RateLimited(wait).into_response()
}

/// Paths anyone may call without a device token.
fn is_open_path(path: &str) -> bool {
    matches!(path, "/hello" | "/setup" | "/enroll" | "/prove")
}

/// Paths that may take longer than [`REQUEST_TIMEOUT`]: the live socket and pictures and videos going up or down.
fn is_long_path(path: &str) -> bool {
    path == "/live" || path == "/media" || path.starts_with("/media/")
}

/// The middleware in front of every route: works out the client's address, applies the per-address limits, the
/// concurrency limit and the timeout, and adds the security headers to the answer.
pub async fn guard(State(hub): State<Arc<Hub>>, mut req: Request, next: Next) -> Response {
    let limits = &hub.limits;
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    let ip = client_ip(peer, req.headers(), limits.config.trust_proxy);
    req.extensions_mut().insert(ClientIp(ip));
    let path = req.uri().path().to_owned();

    let mut res = 'res: {
        if let Some(wait) = limits.bad_tokens.refused(ip) {
            break 'res too_many(wait);
        }
        let checked = if is_open_path(&path) { limits.open_paths.check(&ip) } else { limits.per_ip.check(&ip) };
        if let Err(wait) = checked {
            break 'res too_many(wait);
        }
        let Ok(_permit) = limits.concurrent.clone().try_acquire_owned() else {
            break 'res too_many(Duration::from_secs(1));
        };
        if is_long_path(&path) {
            next.run(req).await
        } else {
            match tokio::time::timeout(REQUEST_TIMEOUT, next.run(req)).await {
                Ok(res) => res,
                Err(_) => {
                    log::warn!("a request to {path} from {ip} took too long");
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        axum::Json(kit_protocol::ErrorBody {
                            error: "The family hub took too long. Try again in a moment.".into(),
                        }),
                    )
                        .into_response()
                }
            }
        }
    };
    add_security_headers(res.headers_mut());
    res
}

fn add_security_headers(headers: &mut HeaderMap) {
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    // API answers are never cached; pictures and videos keep whatever caching they were sent with.
    let is_json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    if is_json {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(s: &str) -> Option<SocketAddr> {
        Some(s.parse().unwrap())
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, v.parse().unwrap());
        }
        h
    }

    #[test]
    fn proxy_headers_only_count_from_a_trusted_loopback_peer() {
        let h = headers(&[("x-forwarded-for", "1.1.1.1, 2.2.2.2")]);
        assert_eq!(client_ip(peer("127.0.0.1:5000"), &h, false), "127.0.0.1".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip(peer("127.0.0.1:5000"), &h, true), "2.2.2.2".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip(peer("[::1]:5000"), &h, true), "2.2.2.2".parse::<IpAddr>().unwrap());
        assert_eq!(client_ip(peer("8.8.8.8:5000"), &h, true), "8.8.8.8".parse::<IpAddr>().unwrap());
        let h = headers(&[("x-real-ip", "3.3.3.3"), ("x-forwarded-for", "1.1.1.1")]);
        assert_eq!(client_ip(peer("127.0.0.1:5000"), &h, true), "3.3.3.3".parse::<IpAddr>().unwrap());
        let h = headers(&[("x-real-ip", "junk")]);
        assert_eq!(client_ip(peer("127.0.0.1:5000"), &h, true), "127.0.0.1".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn home_and_tailscale_addresses_are_told_apart_from_the_rest() {
        for ip in
            ["192.168.1.20", "10.0.0.5", "172.16.0.1", "169.254.1.1", "100.101.102.103", "fd7a:115c:a1e0::1", "fe80::1"]
        {
            assert!(is_home_or_tailnet(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "100.128.0.1", "127.0.0.1", "::1", "0.0.0.0", "2001:4860:4860::8888"] {
            assert!(!is_home_or_tailnet(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn buckets_allow_a_burst_then_refill() {
        let limiter = RateLimiter::new(1000.0, 3.0);
        for _ in 0..3 {
            limiter.check(&1).unwrap();
        }
        assert!(limiter.check(&1).is_err());
        assert!(limiter.check(&2).is_ok());
        std::thread::sleep(Duration::from_millis(5));
        assert!(limiter.check(&1).is_ok());
    }

    #[test]
    fn live_sockets_are_counted_per_device() {
        let limits = Arc::new(Limits::new(Config::default()));
        let slots: Vec<_> = (0..LIVE_PER_DEVICE).map(|_| limits.live_slot("a").unwrap()).collect();
        assert!(limits.live_slot("a").is_none());
        assert!(limits.live_slot("b").is_some());
        drop(slots);
        assert!(limits.live_slot("a").is_some());
    }
}
