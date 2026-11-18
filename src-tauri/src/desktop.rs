//! Desktop only: KitChat lives in the system tray so messages keep arriving with the window closed, can start when
//! the computer does, and shows how many messages are unread on its taskbar button.

use std::path::PathBuf;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewWindow, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt;

/// Passed when KitChat starts with the computer: it goes straight to the tray.
pub const HIDDEN_ARG: &str = "--hidden";

pub struct Tray {
    icon: TrayIcon,
    autostart: CheckMenuItem<Wry>,
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

/// Brings the window back from the tray (or from behind other windows).
pub fn show(app: &AppHandle) {
    if let Some(w) = window(app) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn setup(app: &AppHandle, data_dir: PathBuf) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open KitChat", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Start when I sign in",
        true,
        app.autolaunch().is_enabled().unwrap_or(false),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit KitChat", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &autostart, &PredefinedMenuItem::separator(app)?, &quit])?;

    let mut builder = TrayIconBuilder::with_id("kitchat")
        .tooltip("KitChat")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show(app),
            "autostart" => {
                let on = !app.autolaunch().is_enabled().unwrap_or(false);
                if let Err(e) = set_autostart(app, on) {
                    log::warn!("start at sign-in: {e}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let icon = builder.build(app)?;
    app.manage(Tray { icon, autostart: autostart.clone() });

    if let Some(w) = window(app) {
        // Closing the window tucks KitChat into the tray; "Quit" in the tray's menu really quits.
        let handle = app.clone();
        w.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Some(w) = window(&handle) {
                    let _ = w.hide();
                }
                let hint = data_dir.join("tray-hint-shown");
                if !hint.exists() {
                    let _ = std::fs::write(&hint, b"1");
                    crate::platform::notify(
                        &handle,
                        "KitChat is still here",
                        "It keeps running in the tray so messages still arrive. Right-click Kit there to quit.",
                    );
                }
            }
        });
        if !std::env::args().any(|a| a == HIDDEN_ARG) {
            let _ = w.show();
        }
    }
    Ok(())
}

pub fn set_autostart(app: &AppHandle, on: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    let result = if on { launcher.enable() } else { launcher.disable() };
    result.map_err(|e| e.to_string())?;
    let enabled = launcher.is_enabled().unwrap_or(on);
    if let Some(tray) = app.try_state::<Tray>() {
        let _ = tray.autostart.set_checked(enabled);
    }
    Ok(())
}

pub fn autostart_enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Shows the unread count: on the taskbar button (a red badge on Windows, the dock or launcher elsewhere), in the
/// tray's tooltip and in the window title.
pub fn set_unread(app: &AppHandle, count: u32) {
    let text = if count == 0 { "KitChat".to_string() } else { format!("KitChat ({count} unread)") };
    if let Some(tray) = app.try_state::<Tray>() {
        let _ = tray.icon.set_tooltip(Some(&text));
    }
    let Some(w) = window(app) else { return };
    let _ = w.set_title(if count == 0 { "KitChat".to_string() } else { format!("KitChat ({count})") }.as_str());
    #[cfg(target_os = "windows")]
    {
        let icon = (count > 0).then(|| tauri::image::Image::new_owned(badge(count), BADGE, BADGE));
        let _ = w.set_overlay_icon(icon);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = w.set_badge_count((count > 0).then_some(count as i64));
    }
}

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
const BADGE: u32 = 32;

#[cfg_attr(not(any(target_os = "windows", test)), allow(dead_code))]
/// A red dot with the count in white (9+ above nine), as RGBA pixels, for the taskbar button's overlay.
fn badge(count: u32) -> Vec<u8> {
    // 3x5 digits, one row per byte, high bit on the left.
    const DIGITS: [[u8; 5]; 10] = [
        [0b111, 0b101, 0b101, 0b101, 0b111],
        [0b010, 0b110, 0b010, 0b010, 0b111],
        [0b111, 0b001, 0b111, 0b100, 0b111],
        [0b111, 0b001, 0b111, 0b001, 0b111],
        [0b101, 0b101, 0b111, 0b001, 0b001],
        [0b111, 0b100, 0b111, 0b001, 0b111],
        [0b111, 0b100, 0b111, 0b101, 0b111],
        [0b111, 0b001, 0b010, 0b010, 0b010],
        [0b111, 0b101, 0b111, 0b101, 0b111],
        [0b111, 0b101, 0b111, 0b001, 0b111],
    ];
    const PLUS: [u8; 5] = [0b000, 0b010, 0b111, 0b010, 0b000];
    let n = BADGE as i32;
    let mut px = vec![0u8; (BADGE * BADGE * 4) as usize];
    let mut put = |x: i32, y: i32, rgba: [u8; 4]| {
        if (0..n).contains(&x) && (0..n).contains(&y) {
            let i = ((y * n + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&rgba);
        }
    };
    let c = (n as f32 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
            if d <= c + 0.5 {
                put(x, y, [0xE5, 0x39, 0x35, 0xFF]);
            }
        }
    }
    let glyphs: Vec<[u8; 5]> = if count > 9 { vec![DIGITS[9], PLUS] } else { vec![DIGITS[count as usize]] };
    let scale = if glyphs.len() == 1 { 4 } else { 3 };
    let width = glyphs.len() as i32 * 3 * scale + (glyphs.len() as i32 - 1) * scale;
    let (left, top) = ((n - width) / 2, (n - 5 * scale) / 2);
    for (g, glyph) in glyphs.iter().enumerate() {
        let gx = left + g as i32 * 4 * scale;
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..3 {
                if bits & (0b100 >> col) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            put(gx + col * scale + dx, top + row as i32 * scale + dy, [0xFF; 4]);
                        }
                    }
                }
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_is_a_red_dot_with_white_digits() {
        for count in [1, 7, 42] {
            let px = badge(count);
            assert_eq!(px.len(), (BADGE * BADGE * 4) as usize);
            let at = |x: u32, y: u32| &px[((y * BADGE + x) * 4) as usize..][..4];
            assert_eq!(at(0, 0)[3], 0, "corners stay clear");
            assert!(px.chunks(4).any(|p| p == [0xFF; 4]), "has white pixels");
            assert!(px.chunks(4).any(|p| p == [0xE5, 0x39, 0x35, 0xFF]), "has red pixels");
        }
    }
}
