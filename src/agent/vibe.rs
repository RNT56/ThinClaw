//! Backward-compatible shim for the older `/vibe` vocabulary.
//!
//! New code should prefer `agent::personality`, but we keep this module so
//! existing imports and slash-command habits continue to work during migration.

use std::borrow::Cow;

pub use crate::agent::personality::SessionPersonalityOverlay as VibeOverlay;

pub fn resolve_vibe(name: &str) -> VibeOverlay {
    crate::agent::personality::resolve_personality(name)
}

pub fn format_overlay(vibe: &VibeOverlay) -> String {
    crate::agent::personality::format_overlay(vibe)
}

pub fn builtin_vibe_names() -> impl Iterator<Item = &'static str> {
    crate::agent::personality::available_personality_names()
}

pub fn preview(vibe: &VibeOverlay) -> Cow<'_, str> {
    crate::agent::personality::preview(vibe)
}
