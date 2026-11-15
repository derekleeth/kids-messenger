//! What differs between desktop and Android: notifications, staying connected in the background, things shared into
//! the app from other apps, and scanning a QR code.
//!
//! On Android these go through `KitPlugin.kt` (gen/android): a foreground service (`HubService.kt`) keeps the app's
//! process, and so its hub connection, alive while it's in the background, and posts the message notifications.

use serde::{Deserialize, Serialize};
#[cfg(target_os = "android")]
use tauri::Manager;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{AppHandle, Runtime};

#[cfg(target_os = "android")]
pub struct KitPlugin<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Something shared to KitChat from another app: a picture (saved in the app's cache) or text such as a link.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedItem {
    pub kind: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub text: String,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("kit")
        .setup(|_app, _api| {
            #[cfg(target_os = "android")]
            {
                let handle = _api.register_android_plugin("com.kitchat.app", "KitPlugin")?;
                _app.manage(KitPlugin(handle));
            }
            Ok(())
        })
        .build()
}

#[cfg(target_os = "android")]
fn plugin<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::State<'_, KitPlugin<R>>> {
    app.try_state::<KitPlugin<R>>()
}

/// Shows a message notification.
pub fn notify<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str) {
    #[cfg(target_os = "android")]
    if let Some(p) = plugin(app) {
        let args = serde_json::json!({ "title": title, "body": body });
        if let Err(e) = p.0.run_mobile_plugin::<serde_json::Value>("notify", args) {
            log::warn!("notification: {e}");
        }
    }
    #[cfg(desktop)]
    {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app.notification().builder().title(title).body(body).show() {
            log::warn!("notification: {e}");
        }
    }
    let _ = (app, title, body);
}

/// Android: runs the background service while this device is part of a family. Nothing elsewhere.
pub fn keep_alive<R: Runtime>(app: &AppHandle<R>, on: bool) {
    #[cfg(target_os = "android")]
    if let Some(p) = plugin(app)
        && let Err(e) = p.0.run_mobile_plugin::<serde_json::Value>("keepAlive", serde_json::json!({ "on": on }))
    {
        log::warn!("background service: {e}");
    }
    let _ = (app, on);
}

/// Android 17+: asks for the permission to reach devices on the home network (a hub at 192.168.x.x), which Android
/// otherwise blocks. Says whether it's allowed. Elsewhere, always allowed.
pub async fn local_network<R: Runtime>(app: &AppHandle<R>) -> bool {
    #[cfg(target_os = "android")]
    if let Some(p) = plugin(app) {
        #[derive(Deserialize)]
        struct Answer {
            granted: bool,
        }
        return match p.0.run_mobile_plugin_async::<Answer>("localNetwork", serde_json::json!({})).await {
            Ok(a) => a.granted,
            Err(e) => {
                log::warn!("local network permission: {e}");
                true
            }
        };
    }
    let _ = app;
    true
}

/// What's been shared into the app since the last call (Android's share sheet).
pub async fn take_shared<R: Runtime>(app: &AppHandle<R>) -> Vec<SharedItem> {
    #[cfg(target_os = "android")]
    if let Some(p) = plugin(app) {
        #[derive(Deserialize)]
        struct Shared {
            items: Vec<SharedItem>,
        }
        return match p.0.run_mobile_plugin_async::<Shared>("takeShared", serde_json::json!({})).await {
            Ok(shared) => shared.items,
            Err(e) => {
                log::warn!("shared items: {e}");
                Vec::new()
            }
        };
    }
    let _ = app;
    Vec::new()
}

/// Scans a QR code with the camera (Android, through Google's code scanner). `None` when cancelled.
pub async fn scan<R: Runtime>(app: &AppHandle<R>) -> Result<Option<String>, String> {
    #[cfg(target_os = "android")]
    if let Some(p) = plugin(app) {
        #[derive(Deserialize)]
        struct Scanned {
            text: Option<String>,
            #[serde(default)]
            canceled: bool,
        }
        let scanned: Scanned =
            p.0.run_mobile_plugin_async("scan", serde_json::json!({})).await.map_err(|e| e.to_string())?;
        return Ok(if scanned.canceled { None } else { scanned.text });
    }
    let _ = app;
    Err("Scanning needs a camera. Type the code instead.".into())
}
