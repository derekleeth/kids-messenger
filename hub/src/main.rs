//! `kitchat-hub [--data DIR] [--port N] [--bind ADDR] [--no-previews] [--trust-proxy] [--rate-limit-scale X]`
//!
//! Runs the family hub. Data (the database and pictures) goes in `--data`, by default `hub-data` next to where it's
//! started; the port is 8787. On first start it prints a setup code for the first parent to type into the app.
//! Every option can also be set with an environment variable (handy for a systemd service); a flag wins.

use std::net::SocketAddr;
use std::path::PathBuf;

use kitchat_hub::{Options, app, limits, open, setup_code_file};

struct Args {
    data: PathBuf,
    bind: String,
    port: u16,
    previews: bool,
    trust_proxy: bool,
    rate_limit_scale: f64,
}

const USAGE: &str =
    "kitchat-hub [--data DIR] [--port N] [--bind ADDR] [--no-previews] [--trust-proxy] [--rate-limit-scale X]

  --data DIR              where the database and pictures are kept (default: ./hub-data, or KITCHAT_DATA)
  --port N                the port to listen on (default: 8787, or KITCHAT_PORT)
  --bind ADDR             the address to listen on (default: 0.0.0.0, every network, or KITCHAT_BIND)
  --no-previews           don't fetch link previews (or KITCHAT_NO_PREVIEWS=1)
  --trust-proxy           behind a reverse proxy on this computer (nginx): take the client's address from
                          X-Real-IP / X-Forwarded-For, only when the connection comes from loopback
                          (or KITCHAT_TRUST_PROXY=1); use with --bind 127.0.0.1
  --rate-limit-scale X    multiply the rate limits by X, e.g. 2 for twice as many requests (default: 1, or
                          KITCHAT_RATE_LIMIT_SCALE)";

/// An on/off environment variable: set to anything but empty, `0`, `false`, `no` or `off` is on.
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .is_ok_and(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no" | "off"))
}

fn parse_scale(s: &str) -> Option<f64> {
    s.trim().parse().ok().filter(|x: &f64| x.is_finite() && *x > 0.0)
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        data: std::env::var_os("KITCHAT_DATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("hub-data")),
        bind: std::env::var("KITCHAT_BIND").ok().filter(|b| !b.trim().is_empty()).unwrap_or_else(|| "0.0.0.0".into()),
        port: match std::env::var("KITCHAT_PORT") {
            Ok(p) => p.trim().parse().map_err(|_| format!("KITCHAT_PORT={p} isn't a port number"))?,
            Err(_) => 8787,
        },
        previews: !env_flag("KITCHAT_NO_PREVIEWS"),
        trust_proxy: env_flag("KITCHAT_TRUST_PROXY"),
        rate_limit_scale: match std::env::var("KITCHAT_RATE_LIMIT_SCALE") {
            Ok(s) => parse_scale(&s).ok_or(format!("KITCHAT_RATE_LIMIT_SCALE={s} isn't a number above 0"))?,
            Err(_) => 1.0,
        },
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--data" => args.data = it.next().ok_or("--data needs a folder")?.into(),
            "--port" => args.port = it.next().and_then(|p| p.parse().ok()).ok_or("--port needs a number")?,
            "--bind" => args.bind = it.next().ok_or("--bind needs an address")?,
            "--no-previews" => args.previews = false,
            "--trust-proxy" => args.trust_proxy = true,
            "--rate-limit-scale" => {
                args.rate_limit_scale =
                    it.next().as_deref().and_then(parse_scale).ok_or("--rate-limit-scale needs a number above 0")?
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other => {
                return Err(format!(
                    "unknown option {other}

{USAGE}"
                ));
            }
        }
    }
    Ok(args)
}

/// Waits for Ctrl-C, or SIGTERM (systemd stopping the service).
async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,sqlx=warn")).init();
    let args = match parse_args() {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(2);
        }
    };
    // reqwest (link previews) needs a TLS crypto provider chosen before its first client.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let options = Options {
        data_dir: args.data.clone(),
        previews: args.previews,
        limits: limits::Config { trust_proxy: args.trust_proxy, scale: args.rate_limit_scale },
    };
    let hub = match open(options).await {
        Ok(hub) => hub,
        Err(e) => {
            eprintln!("Couldn't open the hub's data in {}: {e}", args.data.display());
            std::process::exit(1);
        }
    };
    if let Some(code) = hub.setup_code.lock().await.clone() {
        let _ = std::fs::write(setup_code_file(&args.data), format!("{code}\n"));
        println!();
        println!("  ┌──────────────────────────────────────────────┐");
        println!("  │  KitChat hub is new. Setup code:  {code}   │");
        println!("  │  Type it into the app to set up your family.  │");
        println!("  └──────────────────────────────────────────────┘");
        println!();
    }

    let addr: SocketAddr = match format!("{}:{}", args.bind, args.port).parse() {
        Ok(addr) => addr,
        Err(_) => {
            eprintln!("{}:{} isn't an address to listen on", args.bind, args.port);
            std::process::exit(2);
        }
    };
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Couldn't listen on {addr}: {e}");
            std::process::exit(1);
        }
    };
    if args.trust_proxy && !addr.ip().is_loopback() {
        log::warn!("************************************************************************");
        log::warn!("--trust-proxy is on but the hub listens on {addr}, not just this computer.");
        log::warn!("Proxy headers are only believed from 127.0.0.1/::1, but anyone who can reach");
        log::warn!("this port directly skips the proxy. Use --bind 127.0.0.1 behind nginx.");
        log::warn!("************************************************************************");
    }
    log::info!(
        "KitChat hub listening on http://{addr} (bind {}, port {}, data in {})",
        args.bind,
        args.port,
        args.data.display()
    );
    log::info!(
        "client addresses: {}; rate limits x{}; link previews {}",
        if args.trust_proxy {
            "from X-Real-IP/X-Forwarded-For via a loopback proxy"
        } else {
            "the connection's own (no proxy)"
        },
        args.rate_limit_scale,
        if args.previews { "on" } else { "off" },
    );
    let stopping = hub.stopping.clone();
    let shutdown = async move {
        stop_signal().await;
        log::info!("stopping");
        stopping.send_replace(true);
    };
    let service = app(hub).into_make_service_with_connect_info::<SocketAddr>();
    if let Err(e) = axum::serve(listener, service).with_graceful_shutdown(shutdown).await {
        eprintln!("The hub stopped: {e}");
        std::process::exit(1);
    }
}
