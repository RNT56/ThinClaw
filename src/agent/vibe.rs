//! Session vibe overlays.
//!
//! This module keeps the session-tone surface self-contained so the
//! eventual command wiring can consume a stable API without needing to
//! know about the concrete built-in mappings.

use std::borrow::Cow;

/// A session-level tone overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VibeOverlay {
    /// Canonical vibe name.
    pub name: String,
    /// Prompt patch to inject when the vibe is active.
    pub prompt_patch: String,
}

impl VibeOverlay {
    /// Create a new vibe overlay from canonical parts.
    pub fn new(name: impl Into<String>, prompt_patch: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            prompt_patch: prompt_patch.into(),
        }
    }
}

/// Built-in vibe presets available to the agent.
///
/// The prompts are intentionally short and operational so they can be
/// layered onto a larger system prompt without becoming brittle.
pub const BUILTIN_VIBES: &[(&str, &str)] = &[
    (
        "concise",
        "Be extremely concise. Use short sentences, tight bullets, and avoid filler.",
    ),
    (
        "creative",
        "Think laterally. Offer unusual perspectives, alternative approaches, and experiments.",
    ),
    (
        "technical",
        "Be precise and technical. Use exact terminology, include code examples, and prioritize correctness.",
    ),
    (
        "playful",
        "Be lighthearted and fun. Use humor sparingly, keep the tone warm, and stay helpful.",
    ),
    (
        "formal",
        "Use formal, professional language. Keep structure clear and avoid slang or casual asides.",
    ),
    (
        "eli5",
        "Explain things simply. Use everyday language, short sentences, and concrete analogies.",
    ),
];

/// Resolve a vibe name to a built-in preset or freeform overlay.
///
/// Matching is case-insensitive and trims surrounding whitespace. Unknown
/// names are treated as custom tone instructions.
pub fn resolve_vibe(name: &str) -> VibeOverlay {
    let trimmed = name.trim();
    let lower = trimmed.to_ascii_lowercase();

    if let Some((canonical_name, prompt_patch)) = BUILTIN_VIBES
        .iter()
        .copied()
        .find(|(candidate, _)| *candidate == lower)
    {
        return VibeOverlay::new(canonical_name, prompt_patch);
    }

    VibeOverlay::new(
        trimmed,
        format!("Adopt the following tone for this session: {trimmed}"),
    )
}

/// Render the active vibe as a prompt overlay block.
pub fn format_overlay(vibe: &VibeOverlay) -> String {
    format!(
        "## Temporary Vibe\n\nYour core identity is unchanged. For this session only, adopt this tone:\n\n{}",
        vibe.prompt_patch
    )
}

/// Return the built-in vibe names in display order.
pub fn builtin_vibe_names() -> impl Iterator<Item = &'static str> {
    BUILTIN_VIBES.iter().map(|(name, _)| *name)
}

/// Return the built-in prompt patch for a vibe name, if present.
pub fn builtin_vibe_prompt(name: &str) -> Option<&'static str> {
    let lower = name.trim().to_ascii_lowercase();
    BUILTIN_VIBES
        .iter()
        .copied()
        .find(|(candidate, _)| *candidate == lower)
        .map(|(_, prompt)| prompt)
}

/// Return a display-friendly preview of a vibe.
pub fn preview(vibe: &VibeOverlay) -> Cow<'_, str> {
    if let Some(prompt) = builtin_vibe_prompt(&vibe.name) {
        Cow::Borrowed(prompt)
    } else {
        Cow::Owned(vibe.prompt_patch.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_builtin_vibe_is_case_insensitive() {
        let vibe = resolve_vibe("TeChNiCaL");
        assert_eq!(vibe.name, "technical");
        assert!(vibe.prompt_patch.contains("precise and technical"));
    }

    #[test]
    fn resolve_freeform_vibe_preserves_custom_tone() {
        let vibe = resolve_vibe("noir detective");
        assert_eq!(vibe.name, "noir detective");
        assert!(
            vibe.prompt_patch
                .contains("Adopt the following tone for this session")
        );
    }

    #[test]
    fn format_overlay_wraps_prompt_patch() {
        let vibe = VibeOverlay::new("concise", "Be short.");
        let overlay = format_overlay(&vibe);
        assert!(overlay.contains("## Temporary Vibe"));
        assert!(overlay.contains("Be short."));
    }

    #[test]
    fn builtin_names_include_expected_set() {
        let names: Vec<_> = builtin_vibe_names().collect();
        assert_eq!(
            names,
            vec![
                "concise",
                "creative",
                "technical",
                "playful",
                "formal",
                "eli5"
            ]
        );
    }
}
