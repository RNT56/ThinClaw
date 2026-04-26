//! Deterministic skill prefilter for two-phase selection.
//!
//! The first phase of skill selection is entirely deterministic -- no LLM involvement,
//! no skill content in context. This prevents circular manipulation where a loaded
//! skill could influence which skills get loaded.
//!
//! Scoring:
//! - Keyword exact match: 10 points (capped at 30 total)
//! - Keyword substring match: 5 points (capped at 30 total)
//! - Tag match: 3 points (capped at 15 total)
//! - Regex pattern match: 20 points (capped at 40 total)

use crate::skills::LoadedSkill;

/// Default maximum context tokens allocated to skills.
pub const MAX_SKILL_CONTEXT_TOKENS: usize = 4000;

/// Maximum keyword score cap per skill to prevent gaming via keyword stuffing.
/// Even if a skill has 20 keywords, it can earn at most this many keyword points.
const MAX_KEYWORD_SCORE: u32 = 30;

/// Maximum tag score cap per skill (parallel to keyword cap).
const MAX_TAG_SCORE: u32 = 15;

/// Maximum regex pattern score cap per skill. Without a cap, 5 patterns at
/// 20 points each could yield 100 points, dominating keyword+tag scores.
const MAX_REGEX_SCORE: u32 = 40;

/// Maximum description-word match score. Low weight (2 pts/word) with a
/// conservative cap so description matches are a last-resort signal, not a
/// dominant factor that overrides explicit keyword configuration.
const MAX_DESCRIPTION_SCORE: u32 = 10;

/// Result of prefiltering with score information.
#[derive(Debug)]
pub struct ScoredSkill<'a> {
    pub skill: &'a LoadedSkill,
    pub score: u32,
}

/// Select candidate skills for a given message using deterministic scoring.
///
/// Returns skills sorted by score (highest first), limited by `max_candidates`
/// and total context budget. No LLM is involved in this selection.
pub fn prefilter_skills<'a>(
    message: &str,
    available_skills: &'a [LoadedSkill],
    max_candidates: usize,
    max_context_tokens: usize,
) -> Vec<&'a LoadedSkill> {
    if available_skills.is_empty() || message.is_empty() {
        return vec![];
    }

    let message_lower = message.to_lowercase();

    let mut scored: Vec<ScoredSkill<'a>> = available_skills
        .iter()
        .filter(|skill| {
            // Routing block: dont_use_when excludes the skill
            if skill.manifest.activation.is_excluded_by_routing(message) {
                tracing::debug!(
                    skill = skill.name(),
                    "Excluded by dont_use_when routing block"
                );
                return false;
            }
            // Routing block: use_when must match if specified
            if !skill.manifest.activation.matches_use_when(message) {
                tracing::debug!(
                    skill = skill.name(),
                    "Skipped: no use_when condition matched"
                );
                return false;
            }
            true
        })
        .filter_map(|skill| {
            let score = score_skill(skill, &message_lower, message);
            if score > 0 {
                Some(ScoredSkill { skill, score })
            } else {
                None
            }
        })
        .collect();

    // Sort by score descending
    scored.sort_by_key(|b| std::cmp::Reverse(b.score));

    // Apply candidate limit and context budget
    let mut result = Vec::new();
    let mut budget_remaining = max_context_tokens;

    for entry in scored {
        if result.len() >= max_candidates {
            break;
        }
        let declared_tokens = entry.skill.manifest.activation.max_context_tokens;
        // Rough token estimate: ~0.25 tokens per byte (~4 bytes per token for English prose)
        let approx_tokens = (entry.skill.prompt_content.len() as f64 * 0.25) as usize;
        let raw_cost = if approx_tokens > declared_tokens * 2 {
            tracing::warn!(
                "Skill '{}' declares max_context_tokens={} but prompt is ~{} tokens; using actual estimate",
                entry.skill.name(),
                declared_tokens,
                approx_tokens,
            );
            approx_tokens
        } else {
            declared_tokens
        };
        // Enforce a minimum token cost so max_context_tokens=0 can't bypass budgeting
        let token_cost = raw_cost.max(1);
        if token_cost <= budget_remaining {
            budget_remaining -= token_cost;
            result.push(entry.skill);
        }
    }

    result
}

/// Score a skill against a user message.
fn score_skill(skill: &LoadedSkill, message_lower: &str, message_original: &str) -> u32 {
    let mut score: u32 = 0;

    // Keyword scoring with cap to prevent gaming via keyword stuffing
    let mut keyword_score: u32 = 0;
    for kw_lower in &skill.lowercased_keywords {
        // Exact word match (surrounded by word boundaries)
        if message_lower
            .split_whitespace()
            .any(|word| word.trim_matches(|c: char| !c.is_alphanumeric()) == kw_lower.as_str())
        {
            keyword_score += 10;
        } else if message_lower.contains(kw_lower.as_str()) {
            // Substring match
            keyword_score += 5;
        }
    }
    score += keyword_score.min(MAX_KEYWORD_SCORE);

    // Tag scoring from activation.tags
    let mut tag_score: u32 = 0;
    for tag_lower in &skill.lowercased_tags {
        if message_lower.contains(tag_lower.as_str()) {
            tag_score += 3;
        }
    }
    score += tag_score.min(MAX_TAG_SCORE);

    // Regex pattern scoring using pre-compiled patterns (cached at load time), with cap
    let mut regex_score: u32 = 0;
    for re in &skill.compiled_patterns {
        if re.is_match(message_original) {
            regex_score += 20;
        }
    }
    score += regex_score.min(MAX_REGEX_SCORE);

    // Description-word scoring: broad semantic fallback.
    // Scores 2 points per description word that appears as an exact word in the
    // message. This catches near-misses where the user's vocabulary doesn't
    // overlap with the skill's keywords but does overlap with its description.
    // Example: user says "kubernetes", skill description mentions "kubernetes"
    // but keywords only list ["docker", "container"].
    let mut desc_score: u32 = 0;
    for desc_word in &skill.lowercased_description_words {
        if message_lower
            .split_whitespace()
            .any(|word| word.trim_matches(|c: char| !c.is_alphanumeric()) == desc_word.as_str())
        {
            desc_score += 2;
        }
    }
    score += desc_score.min(MAX_DESCRIPTION_SCORE);

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{
        ActivationCriteria, LoadedSkill, SkillManifest, SkillSource, SkillSourceTier, SkillTrust,
    };
    use std::path::PathBuf;

    fn make_skill(name: &str, keywords: &[&str], tags: &[&str], patterns: &[&str]) -> LoadedSkill {
        let pattern_strings: Vec<String> = patterns.iter().map(|s| s.to_string()).collect();
        let compiled = LoadedSkill::compile_patterns(&pattern_strings);
        let kw_vec: Vec<String> = keywords.iter().map(|s| s.to_string()).collect();
        let tag_vec: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
        let lowercased_keywords = kw_vec.iter().map(|k| k.to_lowercase()).collect();
        let lowercased_tags = tag_vec.iter().map(|t| t.to_lowercase()).collect();
        let description = format!("{} skill", name);
        let lowercased_description_words: Vec<String> = description
            .split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|w| w.len() >= 3)
            .collect();
        LoadedSkill {
            manifest: SkillManifest {
                name: name.to_string(),
                version: "1.0.0".to_string(),
                description,
                activation: ActivationCriteria {
                    keywords: kw_vec,
                    patterns: pattern_strings,
                    tags: tag_vec,
                    max_context_tokens: 1000,
                    use_when: vec![],
                    dont_use_when: vec![],
                },
                metadata: None,
            },
            prompt_content: "Test prompt".to_string(),
            trust: SkillTrust::Trusted,
            source: SkillSource::User(PathBuf::from("/tmp/test")),
            source_tier: SkillSourceTier::Trusted,
            content_hash: "sha256:000".to_string(),
            compiled_patterns: compiled,
            lowercased_keywords,
            lowercased_tags,
            lowercased_description_words,
        }
    }

    fn make_skill_with_routing(
        name: &str,
        keywords: &[&str],
        use_when: &[&str],
        dont_use_when: &[&str],
    ) -> LoadedSkill {
        let mut skill = make_skill(name, keywords, &[], &[]);
        skill.manifest.activation.use_when = use_when.iter().map(|s| s.to_string()).collect();
        skill.manifest.activation.dont_use_when =
            dont_use_when.iter().map(|s| s.to_string()).collect();
        skill
    }

    #[test]
    fn test_empty_message_returns_nothing() {
        let skills = vec![make_skill("test", &["write"], &[], &[])];
        let result = prefilter_skills("", &skills, 3, MAX_SKILL_CONTEXT_TOKENS);
        assert!(result.is_empty());
    }

    #[test]
    fn test_no_matching_skills() {
        let skills = vec![make_skill("cooking", &["recipe", "cook", "bake"], &[], &[])];
        let result = prefilter_skills(
            "Help me write an email",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_keyword_exact_match() {
        let skills = vec![make_skill("writing", &["write", "edit"], &[], &[])];
        let result = prefilter_skills(
            "Please write an email",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name(), "writing");
    }

    #[test]
    fn test_keyword_substring_match() {
        let skills = vec![make_skill("writing", &["writing"], &[], &[])];
        let result = prefilter_skills(
            "I need help with rewriting this text",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_tag_match() {
        let skills = vec![make_skill("writing", &[], &["prose", "email"], &[])];
        let result = prefilter_skills(
            "Draft an email for me",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_regex_pattern_match() {
        let skills = vec![make_skill(
            "writing",
            &[],
            &[],
            &[r"(?i)\b(write|draft)\b.*\b(email|letter)\b"],
        )];
        let result = prefilter_skills(
            "Please draft an email to my boss",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_scoring_priority() {
        let skills = vec![
            make_skill("cooking", &["cook"], &[], &[]),
            make_skill(
                "writing",
                &["write", "draft"],
                &["email"],
                &[r"(?i)\b(write|draft)\b.*\bemail\b"],
            ),
        ];
        let result = prefilter_skills(
            "Write and draft an email",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name(), "writing");
    }

    #[test]
    fn test_max_candidates_limit() {
        let skills = vec![
            make_skill("a", &["test"], &[], &[]),
            make_skill("b", &["test"], &[], &[]),
            make_skill("c", &["test"], &[], &[]),
        ];
        let result = prefilter_skills("test", &skills, 2, MAX_SKILL_CONTEXT_TOKENS);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_context_budget_limit() {
        let mut skill = make_skill("big", &["test"], &[], &[]);
        skill.manifest.activation.max_context_tokens = 3000;
        let mut skill2 = make_skill("also_big", &["test"], &[], &[]);
        skill2.manifest.activation.max_context_tokens = 3000;

        let skills = vec![skill, skill2];
        // Budget of 4000 can only fit one 3000-token skill
        let result = prefilter_skills("test", &skills, 5, 4000);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_invalid_regex_handled_gracefully() {
        let skills = vec![make_skill("bad", &["test"], &[], &["[invalid regex"])];
        let result = prefilter_skills("test", &skills, 3, MAX_SKILL_CONTEXT_TOKENS);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_keyword_score_capped() {
        let many_keywords: Vec<&str> = vec![
            "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p",
        ];
        let skill = make_skill("spammer", &many_keywords, &[], &[]);
        let skills = vec![skill];
        let result = prefilter_skills(
            "a b c d e f g h i j k l m n o p",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_tag_score_capped() {
        let many_tags: Vec<&str> = vec![
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel",
        ];
        let skill = make_skill("tag-spammer", &[], &many_tags, &[]);
        let skills = vec![skill];
        let result = prefilter_skills(
            "alpha bravo charlie delta echo foxtrot golf hotel",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_regex_score_capped() {
        let skill = make_skill(
            "regex-spammer",
            &[],
            &[],
            &[
                r"(?i)\bwrite\b",
                r"(?i)\bdraft\b",
                r"(?i)\bedit\b",
                r"(?i)\bcompose\b",
                r"(?i)\bauthor\b",
            ],
        );
        let skills = vec![skill];
        let result = prefilter_skills(
            "write draft edit compose author",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_zero_context_tokens_still_costs_budget() {
        let mut skill = make_skill("free", &["test"], &[], &[]);
        skill.manifest.activation.max_context_tokens = 0;
        skill.prompt_content = String::new();
        let mut skill2 = make_skill("also_free", &["test"], &[], &[]);
        skill2.manifest.activation.max_context_tokens = 0;
        skill2.prompt_content = String::new();

        let skills = vec![skill, skill2];
        let result = prefilter_skills("test", &skills, 5, 1);
        assert_eq!(result.len(), 1);
    }

    // ── Routing block tests ──────────────────────────────────────────

    #[test]
    fn test_dont_use_when_excludes_skill() {
        let skill = make_skill_with_routing("writing", &["write"], &[], &["code", "programming"]);
        let skills = vec![skill];
        // Message contains "code" → skill should be excluded
        let result = prefilter_skills(
            "write some code for me",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_dont_use_when_allows_when_no_match() {
        let skill = make_skill_with_routing("writing", &["write"], &[], &["code", "programming"]);
        let skills = vec![skill];
        // No exclusion match → skill should be included
        let result = prefilter_skills(
            "write an email to my boss",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_use_when_requires_match() {
        let skill = make_skill_with_routing("writing", &["write"], &["formal", "business"], &[]);
        let skills = vec![skill];
        // Message doesn't contain "formal" or "business" → excluded
        let result = prefilter_skills("write a joke for me", &skills, 3, MAX_SKILL_CONTEXT_TOKENS);
        assert!(result.is_empty());
    }

    #[test]
    fn test_use_when_matches() {
        let skill = make_skill_with_routing("writing", &["write"], &["formal", "business"], &[]);
        let skills = vec![skill];
        // Message contains "business" → included
        let result = prefilter_skills(
            "write a business proposal",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_both_routing_blocks() {
        let skill = make_skill_with_routing("writing", &["write"], &["email"], &["spam"]);
        let skills = vec![skill];
        // Has "email" (use_when match) but also "spam" (dont_use_when match) → excluded
        let result = prefilter_skills(
            "write email about spam filtering",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert!(result.is_empty());
    }

    // ── Description-word scoring tests ─────────────────────────────────

    #[test]
    fn test_description_word_match_activates_skill() {
        // Skill has NO keywords matching the message, but its description does.
        // The flagship case: "kubernetes" in message, description mentions "kubernetes"
        // but keywords only have ["docker", "container"].
        let mut skill = make_skill("docker-deploy", &["docker", "container"], &[], &[]);
        // Override description to include "kubernetes"
        skill.manifest.description =
            "Assists with Docker container deployments and Kubernetes orchestration".to_string();
        skill.lowercased_description_words = skill
            .manifest
            .description
            .split_whitespace()
            .map(|w| {
                w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|w| w.len() >= 3)
            .collect();

        let skills = vec![skill];
        let result = prefilter_skills(
            "help me set up kubernetes",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(
            result.len(),
            1,
            "skill should activate via description match"
        );
        assert_eq!(result[0].name(), "docker-deploy");
    }

    #[test]
    fn test_description_score_capped() {
        // Skill description has many words matching the message — score should be capped
        let mut skill = make_skill("broad", &[], &[], &[]);
        // Force a description with many matchable words
        skill.manifest.description =
            "assists deploy build push pull run test check validate lint format audit review"
                .to_string();
        skill.lowercased_description_words = skill
            .manifest
            .description
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .filter(|w| w.len() >= 3)
            .collect();

        let skills = vec![skill];
        // Message hits many description words — should still activate (score ≤ MAX_DESCRIPTION_SCORE)
        let result = prefilter_skills(
            "assists deploy build push pull run test check validate lint format audit review",
            &skills,
            3,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_description_match_does_not_beat_explicit_keywords() {
        // A skill with explicit keyword match should outscore one with only description match
        let kw_skill = make_skill("kw-skill", &["kubernetes"], &[], &[]);
        let mut desc_skill = make_skill("desc-skill", &[], &[], &[]);
        desc_skill.manifest.description = "Kubernetes orchestration tool".to_string();
        desc_skill.lowercased_description_words = vec![
            "kubernetes".to_string(),
            "orchestration".to_string(),
            "tool".to_string(),
        ];

        let skills = vec![desc_skill, kw_skill];
        let result = prefilter_skills(
            "kubernetes cluster setup",
            &skills,
            2,
            MAX_SKILL_CONTEXT_TOKENS,
        );
        // Both should match; kw-skill should sort first (higher score)
        assert_eq!(result.len(), 2);
        assert_eq!(
            result[0].name(),
            "kw-skill",
            "explicit keyword match should rank higher"
        );
    }
}
