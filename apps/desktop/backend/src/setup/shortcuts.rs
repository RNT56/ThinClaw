/// Global shortcut registration for Spotlight and Push-to-Talk.
///
/// Extracted from `lib.rs` to keep the entrypoint focused on app lifecycle.
use std::str::FromStr;
use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

/// Register the configured global shortcuts (Spotlight + PTT).
pub fn register_shortcuts(app: &tauri::App) {
    let config_manager = app.state::<crate::config::ConfigManager>();
    let config = config_manager.get_config();

    // Register spotlight shortcut
    if let Ok(shortcut) = Shortcut::from_str(&config.spotlight_shortcut) {
        let _ = app.global_shortcut().register(shortcut);
    } else {
        let shortcut = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyK);
        let _ = app.global_shortcut().register(shortcut);
    }

    // Register PTT shortcut
    if let Ok(shortcut) = Shortcut::from_str(&config.ptt_shortcut) {
        let _ = app.global_shortcut().register(shortcut);
    } else {
        let shortcut = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::KeyV);
        let _ = app.global_shortcut().register(shortcut);
    }
}
