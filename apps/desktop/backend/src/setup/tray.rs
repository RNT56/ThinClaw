/// Tray icon setup for the system tray.
///
/// Extracted from `lib.rs` to keep the entrypoint focused on app lifecycle.
use std::sync::Arc;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager,
};

/// Managed state for tray icon animation.
pub(crate) struct TrayState {
    pub tray: tauri::tray::TrayIcon,
    pub idle_icon: tauri::image::Image<'static>,
    pub active_icon: tauri::image::Image<'static>,
    pub reset_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// Set up the macOS/Windows/Linux system tray icon with menu items.
pub fn setup_tray(app: &tauri::App) {
    let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>);
    let show_i = MenuItem::with_id(app, "show", "Show OpenClaw", true, None::<&str>);

    if let (Ok(quit_i), Ok(show_i)) = (quit_i, show_i) {
        let menu = Menu::with_items(app, &[&show_i, &quit_i]);
        if let Ok(menu) = menu {
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!(
                "../../icons/tray-iconTemplate.png"
            ))
            .expect("failed to load tray icon");

            let tray_result = TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => app.exit(0),
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app);

            // Store tray handle for animated icon switching
            if let Ok(tray) = &tray_result {
                let active_icon = tauri::image::Image::from_bytes(include_bytes!(
                    "../../icons/tray-icon-activeTemplate.png"
                ))
                .expect("failed to load active tray icon");

                let idle_icon_copy = tauri::image::Image::from_bytes(include_bytes!(
                    "../../icons/tray-iconTemplate.png"
                ))
                .expect("failed to load idle tray icon");

                let tray_state = TrayState {
                    tray: tray.clone(),
                    idle_icon: idle_icon_copy,
                    active_icon,
                    reset_handle: tokio::sync::Mutex::new(None),
                };
                app.manage(Arc::new(tray_state));
            }
        }
    }
}
