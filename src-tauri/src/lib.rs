//! KitChat: a family messenger with notes and lists, for the kids' tablets, the parents' phones and a desktop.
//!
//! The Rust side keeps this device's copy of the family's chats and notes (`store`), stays connected to the family
//! hub and sends what's waiting (`engine`), and keeps the device's token in the OS keychain (`secret`). The page
//! (src/) shows it all and calls the commands in `commands`.

mod commands;
#[cfg(desktop)]
mod desktop;
mod engine;
mod error;
mod hub;
mod platform;
mod secret;
mod store;

use tauri::Manager;

/// Rotating log files in the app's log folder, plus stdout in development. Tokens and codes are never logged.
fn logger() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    use log::LevelFilter;
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};
    tauri_plugin_log::Builder::new()
        .clear_targets()
        .targets([
            Target::new(TargetKind::LogDir { file_name: Some("kitchat".into()) }),
            Target::new(TargetKind::Stdout),
        ])
        .level(LevelFilter::Info)
        .level_for("sqlx", LevelFilter::Warn)
        .level_for("tao", LevelFilter::Warn)
        .level_for("wry", LevelFilter::Warn)
        .level_for("tungstenite", LevelFilter::Warn)
        .level_for("tokio_tungstenite", LevelFilter::Warn)
        .max_file_size(5 * 1024 * 1024)
        .rotation_strategy(RotationStrategy::KeepSome(2))
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .build()
}

/// `KITCHAT_PROFILE=kid` runs a second, separate copy (its own data and token), for trying parent and kid side by
/// side on one computer.
fn profile() -> String {
    std::env::var("KITCHAT_PROFILE").unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let profile = profile();
    let builder = tauri::Builder::default();
    // A second launch brings the running window forward instead of starting another copy.
    #[cfg(desktop)]
    let builder = if profile.is_empty() {
        builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| desktop::show(app)))
    } else {
        builder
    };
    let builder = builder.plugin(logger()).plugin(tauri_plugin_opener::init()).plugin(platform::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_notification::init()).plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec![desktop::HIDDEN_ARG]),
    ));
    builder
        .setup(move |app| {
            // reqwest and the WebSocket use rustls with ring.
            let _ = rustls::crypto::ring::default_provider().install_default();
            log::info!(
                "KitChat {} starting on {} {}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            let mut data_dir = app.path().app_data_dir()?;
            if !profile.is_empty() {
                data_dir = data_dir.join(format!("profile-{profile}"));
                log::info!("profile {profile}: data in {}", data_dir.display());
            }
            std::fs::create_dir_all(&data_dir)?;
            let store = tauri::async_runtime::block_on(store::Store::open(&data_dir.join("kitchat.db")))
                .inspect_err(|e| log::error!("could not open the database in {}: {e}", data_dir.display()))?;
            let tokens = secret::TokenStore::new(&data_dir, &profile);
            // The page shows photos and videos straight from the media folder (convertFileSrc), so a big video
            // streams from disk instead of crossing the IPC bridge.
            let media_dir = data_dir.join("media");
            std::fs::create_dir_all(&media_dir)?;
            app.asset_protocol_scope().allow_directory(&media_dir, false)?;
            // And the previews of photos and videos picked on Android, shown in the box before they're sent.
            let outgoing = app.path().app_cache_dir()?.join("outgoing");
            std::fs::create_dir_all(&outgoing)?;
            app.asset_protocol_scope().allow_directory(&outgoing, false)?;
            let engine = tauri::async_runtime::block_on(engine::Engine::start(
                app.handle().clone(),
                store,
                tokens,
                media_dir,
            ));
            app.manage(engine);
            // The window starts hidden (tauri.conf.json) and is shown here, unless KitChat started with the computer.
            #[cfg(desktop)]
            desktop::setup(app.handle(), data_dir.clone())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::snapshot,
            commands::hub_hello,
            commands::setup_family,
            commands::join_family,
            commands::sign_out,
            commands::send_message,
            commands::stage_file,
            commands::send_media,
            commands::retry_message,
            commands::discard_message,
            commands::media_file,
            commands::save_media,
            commands::older_messages,
            commands::mark_read,
            commands::react,
            commands::open_direct,
            commands::save_note,
            commands::delete_note,
            commands::add_member,
            commands::update_member,
            commands::join_code,
            commands::devices,
            commands::revoke_device,
            commands::set_foreground,
            commands::take_shared,
            commands::pick_media,
            commands::scan_qr,
            commands::device_name_hint,
            commands::log,
            commands::set_unread,
            commands::autostart_enabled,
            commands::set_autostart,
            commands::chat_shelf,
            commands::set_hub_addresses,
        ])
        .run(tauri::generate_context!())
        .expect("error while running KitChat");
}
