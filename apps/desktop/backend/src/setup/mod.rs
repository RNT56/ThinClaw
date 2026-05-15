/// App setup modules — extracted from lib.rs for code organization.
///
/// Each sub-module handles a specific aspect of Tauri app initialization:
/// - `commands` — IPC command registry (all `collect_commands!` registrations)
/// - `tray` — System tray icon, menu, and animation state
/// - `shortcuts` — Global keyboard shortcut registration
pub mod commands;
pub mod shortcuts;
pub mod tray;
