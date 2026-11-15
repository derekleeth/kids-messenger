//! `kitchat-hub [--data DIR] [--port N] [--bind ADDR] [--no-previews]`
//!
//! Runs the family hub. Data (the database and pictures) goes in `--data`, by default `hub-data` next to where it's
//! started; the port is 8787. On first start it prints a setup code for the first parent to type into the app.

use std::net::SocketAddr;
use std::path::PathBuf;

use kitchat_hub::{Options, app, open, setup_code_file};

struct Args {
    data: PathBuf,
    bind: String,
    port: u16,
    previews: bool,
}

const USAGE: &str = "kitchat-hub [--data DIR] [--port N] [--bind ADDR] [--no-previews]

  --data DIR      where the database and pictures are kept (default: ./hub-data, or KITCHAT_DATA)
  --port N        the port to listen on (default: 8787, or KITCHAT_PORT)
  --bind ADDR     the address to listen on (default: 0.0.0.0, every network)
  --no-previews   don't fetch link previews";

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        data: std::env::var_os("KITCHAT_DATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("hub-data")),
        bind: "0.0.0.0".into(),
        port: std::env::var("KITCHAT_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(8787),
        previews: true,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--data" => args.data = it.next().ok_or("--data needs a folder")?.into(),
            "--port" => args.port = it.next().and_then(|p| p.parse().ok()).ok_or("--port needs a number")?,
            "--bind" => args.bind = it.next().ok_or("--bind needs an address")?,
            "--no-previews" => args.previews = false,
            "-h" | "--help" => return Err(USAGE.into()),
            other => return Err(format!("unknown option {other}\n\n{USAGE}")),
        }
    }
    Ok(args)
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

    let hub = match open(Options { data_dir: args.data.clone(), previews: args.previews }).await {
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
    log::info!("KitChat hub listening on http://{addr} (data in {})", args.data.display());
    let shutdown = async {
        let _ = tokio::signal::ctrl_c().await;
        log::info!("stopping");
    };
    if let Err(e) = axum::serve(listener, app(hub)).with_graceful_shutdown(shutdown).await {
        eprintln!("The hub stopped: {e}");
        std::process::exit(1);
    }
}
