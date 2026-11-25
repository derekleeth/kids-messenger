//! The KitChat family hub.
//!
//! One small always-on program, on a home computer or a little server, that keeps the family's messages, pictures and
//! notes and passes them to every device. Devices talk to it over HTTPS (REST plus the `/live` WebSocket); the hub
//! is reached from outside the house through a tunnel (docs/hub_setup.md). Only the family's own devices can use it:
//! each joins with a one-time code a parent makes, and gets a token the hub checks on every request.

pub mod auth;
pub mod db;
pub mod error;
pub mod limits;
pub mod live;
pub mod preview;
pub mod random;
pub mod replica;
mod routes;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, broadcast, watch};

pub use routes::{direct_chat_id, router};

pub struct Hub {
    pub pool: SqlitePool,
    /// This hub, out of the family's hubs ([`kit_protocol::Hello::hub_id`]).
    pub hub_id: String,
    pub data_dir: PathBuf,
    pub live: broadcast::Sender<Arc<live::Delivery>>,
    /// Printed at startup until the family is set up; the first parent types it into the app.
    pub setup_code: Mutex<Option<String>>,
    pub guard: auth::CodeGuard,
    /// Fetches link previews. `None` turns previews off (tests, or `--no-previews`).
    pub previews: Option<preview::Fetcher>,
    /// Link previews fetched lately, for the links on lists.
    pub recent_previews: preview::Recent,
    /// Rate limits, lockouts and the client-address rules ([`limits`]).
    pub limits: Arc<limits::Limits>,
    /// Set to true when the hub is shutting down, so `/live` sockets close.
    pub stopping: watch::Sender<bool>,
}

impl Hub {
    pub fn media_dir(&self) -> PathBuf {
        self.data_dir.join("media")
    }

    pub fn uploads_dir(&self) -> PathBuf {
        self.data_dir.join("uploads")
    }
}

pub struct Options {
    pub data_dir: PathBuf,
    pub previews: bool,
    pub limits: limits::Config,
}

/// Opens (or creates) the hub's database in `data_dir` and gets it ready to serve.
pub async fn open(options: Options) -> Result<Arc<Hub>, error::ApiError> {
    std::fs::create_dir_all(options.data_dir.join("media"))?;
    std::fs::create_dir_all(options.data_dir.join("uploads"))?;
    let pool = db::open(&options.data_dir.join("kitchat-hub.db")).await?;
    let hub_id = db::hub_id(&pool, &origin(&options.data_dir)).await?;
    let (live, _) = broadcast::channel(512);
    let setup_code = if db::family_name(&pool).await?.is_none() { Some(random::setup_code()) } else { None };
    let previews = if options.previews { Some(preview::Fetcher::new()?) } else { None };
    Ok(Arc::new(Hub {
        pool,
        hub_id,
        data_dir: options.data_dir,
        live,
        setup_code: Mutex::new(setup_code),
        guard: auth::CodeGuard::default(),
        previews,
        recent_previews: preview::Recent::default(),
        limits: Arc::new(limits::Limits::new(options.limits)),
        stopping: watch::Sender::new(false),
    }))
}

/// The router with the hub attached, behind the limits and security headers ([`limits::guard`]).
pub fn app(hub: Arc<Hub>) -> Router {
    router().layer(axum::middleware::from_fn_with_state(hub.clone(), limits::guard)).with_state(hub)
}

/// Where this hub's data is: its folder and the computer's name. Data copied to another computer or folder (a travel
/// hub made from a copy of the home hub's) gets a hub ID of its own, so devices don't take one hub's change numbers
/// for the other's.
fn origin(data_dir: &Path) -> String {
    let dir = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    format!("{}|{}", computer_name(), dir.display())
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::fs::read_to_string("/proc/sys/kernel/hostname"))
        .or_else(|_| std::fs::read_to_string("/etc/hostname"))
        .or_else(|_| std::env::var("HOSTNAME"))
        .map(|name| name.trim().to_string())
        .unwrap_or_default()
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or_default()
}

/// Where the setup code is also written, for a hub started as a service with nobody watching its output.
pub fn setup_code_file(data_dir: &Path) -> PathBuf {
    data_dir.join("SETUP-CODE.txt")
}

/// A hub with an in-memory database and no link previews, for tests.
pub async fn open_in_memory(data_dir: PathBuf) -> Result<Arc<Hub>, error::ApiError> {
    open_in_memory_with(data_dir, limits::Config::default()).await
}

/// [`open_in_memory`] with other limits.
pub async fn open_in_memory_with(data_dir: PathBuf, limits: limits::Config) -> Result<Arc<Hub>, error::ApiError> {
    std::fs::create_dir_all(data_dir.join("media"))?;
    std::fs::create_dir_all(data_dir.join("uploads"))?;
    let pool = db::open_memory().await?;
    let hub_id = db::hub_id(&pool, &origin(&data_dir)).await?;
    let (live, _) = broadcast::channel(512);
    Ok(Arc::new(Hub {
        pool,
        hub_id,
        data_dir,
        live,
        setup_code: Mutex::new(Some(random::setup_code())),
        guard: auth::CodeGuard::default(),
        previews: None,
        recent_previews: preview::Recent::default(),
        limits: Arc::new(limits::Limits::new(limits)),
        stopping: watch::Sender::new(false),
    }))
}
