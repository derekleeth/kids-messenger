//! `kitchat-hub [--data DIR] [--port N] [--bind ADDR] [--no-previews] [--trust-proxy] [--rate-limit-scale X]
//! [--keep-days N] [--keep-gb N]`
//!
//! Runs the family hub. Data (the database and pictures) goes in `--data`, by default `hub-data` next to where it's
//! started; the port is 8787. On first start it prints a setup code for the first parent to type into the app.
//! Every option can also be set with an environment variable (handy for a systemd service); a flag wins.
//!
//! A second hub for the family (a travel router) joins it with `kitchat-hub join <home hub's address> <code>`, with a
//! code from `kitchat-hub pair-code` on the home hub; from then on it keeps in step with home whenever it runs and can
//! reach it (docs/multi_hub_plan.md).

use std::net::SocketAddr;
use std::path::PathBuf;

use kitchat_hub::{Options, app, hub_files, limits, open, peers, setup_code_file};

/// Something to do instead of running the hub.
enum Command {
    /// Join the family's hubs through the hub at this address, with a code made there.
    Join { address: String, code: String, name: String },
    /// Print a code another hub can join with.
    PairCode,
    /// Swap changes once with every hub this one calls, and stop.
    Sync,
}

struct Args {
    command: Option<Command>,
    data: PathBuf,
    bind: String,
    port: u16,
    previews: bool,
    trust_proxy: bool,
    rate_limit_scale: f64,
    keep_files: hub_files::Keep,
}

const USAGE: &str =
    "kitchat-hub [--data DIR] [--port N] [--bind ADDR] [--no-previews] [--trust-proxy] [--rate-limit-scale X]
            [--keep-days N] [--keep-gb N]
kitchat-hub pair-code [--data DIR]
kitchat-hub join ADDRESS CODE [--name NAME] [--data DIR]
kitchat-hub sync [--data DIR]

  pair-code               print a code (good for 10 minutes) another of the family's hubs can join with
  join ADDRESS CODE       join the family's hubs through the hub at ADDRESS (e.g. https://hub.example.com, or
                          192.168.1.20:8787 at home), with a code from pair-code there, and copy the family;
                          the running hub then keeps in step with it whenever it can reach it
  --name NAME             what the other hub calls this one (default: this computer's name)
  sync                    swap changes once with the hubs this one joined, and stop

  --data DIR              where the database and pictures are kept (default: ./hub-data, or KITCHAT_DATA)
  --port N                the port to listen on (default: 8787, or KITCHAT_PORT)
  --bind ADDR             the address to listen on (default: 0.0.0.0, every network, or KITCHAT_BIND)
  --no-previews           don't fetch link previews (or KITCHAT_NO_PREVIEWS=1)
  --trust-proxy           behind a reverse proxy on this computer (nginx): take the client's address from
                          X-Real-IP / X-Forwarded-For, only when the connection comes from loopback
                          (or KITCHAT_TRUST_PROXY=1); use with --bind 127.0.0.1
  --rate-limit-scale X    multiply the rate limits by X, e.g. 2 for twice as many requests (default: 1, or
                          KITCHAT_RATE_LIMIT_SCALE)
  --keep-days N           on a hub that joined another: copy that hub's photos and videos from the last N days
                          (default: 30, or KITCHAT_KEEP_DAYS); older ones are fetched when opened
  --keep-gb N             ... and keep at most N GB of them (default: 20, or KITCHAT_KEEP_GB)";

/// An on/off environment variable: set to anything but empty, `0`, `false`, `no` or `off` is on.
fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .is_ok_and(|v| !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no" | "off"))
}

fn parse_days(s: &str) -> Option<u32> {
    s.trim().parse().ok()
}

/// Gigabytes, as bytes.
fn parse_gb(s: &str) -> Option<u64> {
    s.trim().parse().ok().filter(|x: &f64| x.is_finite() && *x >= 0.0).map(|gb| (gb * 1024.0 * 1024.0 * 1024.0) as u64)
}

fn parse_scale(s: &str) -> Option<f64> {
    s.trim().parse().ok().filter(|x: &f64| x.is_finite() && *x > 0.0)
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        command: None,
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
        keep_files: hub_files::Keep {
            days: match std::env::var("KITCHAT_KEEP_DAYS") {
                Ok(d) => parse_days(&d).ok_or(format!("KITCHAT_KEEP_DAYS={d} isn't a number of days"))?,
                Err(_) => hub_files::Keep::default().days,
            },
            max_bytes: match std::env::var("KITCHAT_KEEP_GB") {
                Ok(g) => parse_gb(&g).ok_or(format!("KITCHAT_KEEP_GB={g} isn't a number of GB"))?,
                Err(_) => hub_files::Keep::default().max_bytes,
            },
        },
    };
    let mut it = std::env::args().skip(1).peekable();
    let mut positional = Vec::new();
    let mut name = None;
    let command = it.next_if(|a| !a.starts_with('-'));
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--name" if command.as_deref() == Some("join") => name = Some(it.next().ok_or("--name needs a name")?),
            "--data" => args.data = it.next().ok_or("--data needs a folder")?.into(),
            "--port" => args.port = it.next().and_then(|p| p.parse().ok()).ok_or("--port needs a number")?,
            "--bind" => args.bind = it.next().ok_or("--bind needs an address")?,
            "--no-previews" => args.previews = false,
            "--trust-proxy" => args.trust_proxy = true,
            "--rate-limit-scale" => {
                args.rate_limit_scale =
                    it.next().as_deref().and_then(parse_scale).ok_or("--rate-limit-scale needs a number above 0")?
            }
            "--keep-days" => {
                args.keep_files.days = it.next().as_deref().and_then(parse_days).ok_or("--keep-days needs a number")?
            }
            "--keep-gb" => {
                args.keep_files.max_bytes =
                    it.next().as_deref().and_then(parse_gb).ok_or("--keep-gb needs a number of GB")?
            }
            "-h" | "--help" => return Err(USAGE.into()),
            other if !other.starts_with('-') && command.is_some() => positional.push(other.to_string()),
            other => {
                return Err(format!(
                    "unknown option {other}

{USAGE}"
                ));
            }
        }
    }
    args.command = match (command.as_deref(), positional.as_slice()) {
        (None, []) => None,
        (Some("pair-code"), []) => Some(Command::PairCode),
        (Some("sync"), []) => Some(Command::Sync),
        (Some("join"), [address, code]) => Some(Command::Join {
            address: address.clone(),
            code: code.clone(),
            name: name.unwrap_or_else(computer_name),
        }),
        (Some("join"), _) => {
            return Err(format!(
                "join needs the other hub's address and a code

{USAGE}"
            ));
        }
        _ => return Err(USAGE.into()),
    };
    Ok(args)
}

fn computer_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .chain(std::fs::read_to_string("/etc/hostname").ok())
        .map(|n| n.trim().chars().take(40).collect::<String>())
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "Travel hub".into())
}

/// Runs a command instead of the hub, and exits.
async fn run_command(hub: &kitchat_hub::Hub, command: Command) -> ! {
    let result = match command {
        Command::PairCode => peers::make_code(hub).await.map(|code| {
            println!();
            println!("  Hub code: {}", code.code);
            println!("  On the other hub, within 10 minutes:  kitchat-hub join <this hub's address> {}", code.code);
            println!();
        }),
        Command::Join { address, code, name } => peers::join(hub, &address, &code, &name).await.map(|changed| {
            println!("Joined the family's hubs through {address} ({changed} things copied).");
            println!("Start the hub (or restart it) and it keeps in step whenever it can reach that hub.");
        }),
        Command::Sync => peers::sync_once(hub).await.map(|changed| println!("In step ({changed} things changed).")),
    };
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
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
        keep_files: args.keep_files,
    };
    let hub = match open(options).await {
        Ok(hub) => hub,
        Err(e) => {
            eprintln!("Couldn't open the hub's data in {}: {e}", args.data.display());
            std::process::exit(1);
        }
    };
    if let Some(command) = args.command {
        run_command(&hub, command).await;
    }
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
    {
        let hub = hub.clone();
        tokio::spawn(async move {
            if let Err(e) = hub_files::fill_in_checksums(&hub).await {
                log::warn!("couldn't work out the checksums of older pictures: {e}");
            }
        });
    }
    peers::spawn(hub.clone());
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
