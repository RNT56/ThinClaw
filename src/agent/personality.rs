//! Agent identity packs and temporary session personality overlays.
//!
//! The durable workspace identity still lives in `IDENTITY.md`, `SOUL.md`,
//! `USER.md`, and `AGENTS.md`. This module defines the operator-facing
//! personality vocabulary used by setup, chat commands, and prompt overlays.

use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersonalityPack {
    pub key: &'static str,
    pub summary: &'static str,
    pub prompt_patch: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPersonalityOverlay {
    pub name: String,
    pub prompt_patch: String,
}

impl SessionPersonalityOverlay {
    pub fn new(name: impl Into<String>, prompt_patch: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            prompt_patch: prompt_patch.into(),
        }
    }
}

const CORE_PERSONALITY_PACKS: &[PersonalityPack] = &[
    PersonalityPack {
        key: "balanced",
        summary: "Balanced, dependable, and humane",
        prompt_patch: "Be balanced, dependable, and humane. Stay warm, grounded, and candid about uncertainty while keeping the user's goals in view.",
    },
    PersonalityPack {
        key: "professional",
        summary: "Polished, reliable, and workplace-ready",
        prompt_patch: "Be polished, reliable, and workplace-ready. Use clear professional language, give structured recommendations, and stay calm under ambiguity.",
    },
    PersonalityPack {
        key: "creative_partner",
        summary: "Curious, imaginative, and exploratory",
        prompt_patch: "Be a creative partner. Think laterally, offer fresh directions, suggest experiments, and keep the collaboration energizing without getting vague.",
    },
    PersonalityPack {
        key: "research_assistant",
        summary: "Methodical, evidence-driven, and careful",
        prompt_patch: "Be methodical, evidence-driven, and careful. Surface assumptions, distinguish facts from inferences, and prioritize trustworthy synthesis.",
    },
    PersonalityPack {
        key: "mentor",
        summary: "Patient, explanatory, and encouraging",
        prompt_patch: "Be patient, explanatory, and encouraging. Teach clearly, pace explanations to the user, and help them feel more capable after each interaction.",
    },
    PersonalityPack {
        key: "minimal",
        summary: "Lean, unobtrusive, and flexible",
        prompt_patch: "Be lean and unobtrusive. Give the minimum useful answer first, avoid ceremony, and expand only when the task needs it.",
    },
];

const SESSION_PERSONALITY_PRESETS: &[PersonalityPack] = &[
    PersonalityPack {
        key: "concise",
        summary: "Short, tightly edited replies",
        prompt_patch: "Be extremely concise. Use short sentences, tight bullets, and avoid filler.",
    },
    PersonalityPack {
        key: "creative",
        summary: "Lateral and experimental",
        prompt_patch: "Think laterally. Offer unusual perspectives, alternative approaches, and lightweight experiments.",
    },
    PersonalityPack {
        key: "technical",
        summary: "Precise and exacting",
        prompt_patch: "Be precise and technical. Use exact terminology, include code or implementation details when helpful, and prioritize correctness.",
    },
    PersonalityPack {
        key: "playful",
        summary: "Warm and lightly playful",
        prompt_patch: "Be lighthearted and fun. Use humor sparingly, keep the tone warm, and stay genuinely helpful.",
    },
    PersonalityPack {
        key: "formal",
        summary: "Professional and restrained",
        prompt_patch: "Use formal, professional language. Keep the structure clear and avoid slang or casual asides.",
    },
    PersonalityPack {
        key: "eli5",
        summary: "Simple and concrete explanations",
        prompt_patch: "Explain things simply. Use everyday language, short sentences, and concrete analogies.",
    },
];

const PERSONALITY_ALIASES: &[(&str, &str)] = &[
    ("default", "balanced"),
    ("natural", "balanced"),
    ("creative-partner", "creative_partner"),
    ("research-assistant", "research_assistant"),
];

fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn canonical_key(value: &str) -> String {
    let normalized = normalize_key(value);
    PERSONALITY_ALIASES
        .iter()
        .find(|(alias, _)| *alias == normalized)
        .map(|(_, canonical)| (*canonical).to_string())
        .unwrap_or(normalized)
}

fn find_personality(key: &str) -> Option<&'static PersonalityPack> {
    let canonical = canonical_key(key);
    CORE_PERSONALITY_PACKS
        .iter()
        .chain(SESSION_PERSONALITY_PRESETS.iter())
        .find(|pack| pack.key == canonical)
}

pub fn available_personality_names() -> impl Iterator<Item = &'static str> {
    CORE_PERSONALITY_PACKS
        .iter()
        .chain(SESSION_PERSONALITY_PRESETS.iter())
        .map(|pack| pack.key)
}

pub fn canonical_personality_pack_name(requested: &str) -> &'static str {
    let canonical = canonical_key(requested);
    CORE_PERSONALITY_PACKS
        .iter()
        .find(|pack| pack.key == canonical)
        .map(|pack| pack.key)
        .unwrap_or("balanced")
}

pub fn personality_pack_seed_markdown(requested: &str) -> &'static str {
    match canonical_personality_pack_name(requested) {
        "professional" => include_str!("../../assets/personality_packs/professional.md"),
        "creative_partner" => include_str!("../../assets/personality_packs/creative_partner.md"),
        "research_assistant" => {
            include_str!("../../assets/personality_packs/research_assistant.md")
        }
        "mentor" => include_str!("../../assets/personality_packs/mentor.md"),
        "minimal" => include_str!("../../assets/personality_packs/minimal.md"),
        _ => include_str!("../../assets/personality_packs/balanced.md"),
    }
}

pub fn resolve_personality(requested: &str) -> SessionPersonalityOverlay {
    let trimmed = requested.trim();
    if let Some(pack) = find_personality(trimmed) {
        return SessionPersonalityOverlay::new(pack.key, pack.prompt_patch);
    }
    SessionPersonalityOverlay::new(
        trimmed,
        format!("Adopt the following personality for this session: {trimmed}"),
    )
}

pub fn preview(overlay: &SessionPersonalityOverlay) -> Cow<'_, str> {
    if let Some(pack) = find_personality(&overlay.name) {
        Cow::Borrowed(pack.summary)
    } else {
        Cow::Owned(overlay.prompt_patch.clone())
    }
}

pub fn format_overlay(overlay: &SessionPersonalityOverlay) -> String {
    format!(
        "## Temporary Personality\n\nYour core identity is unchanged. For this session only, adopt this personality overlay:\n\n{}",
        overlay.prompt_patch
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_pack_maps_legacy_default() {
        assert_eq!(canonical_personality_pack_name("default"), "balanced");
        assert_eq!(canonical_personality_pack_name("MENTOR"), "mentor");
    }

    #[test]
    fn resolve_builtin_personality_is_case_insensitive() {
        let personality = resolve_personality("TeChNiCaL");
        assert_eq!(personality.name, "technical");
        assert!(personality.prompt_patch.contains("precise and technical"));
    }

    #[test]
    fn resolve_freeform_personality_preserves_custom_text() {
        let personality = resolve_personality("noir detective");
        assert_eq!(personality.name, "noir detective");
        assert!(
            personality
                .prompt_patch
                .contains("Adopt the following personality")
        );
    }
}
