//! Transport-neutral smart routing classification types.

/// Classification of a request's complexity, determining which model handles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskComplexity {
    /// Short, simple queries -> cheap model
    Simple,
    /// Ambiguous complexity -> cheap model first, cascade to primary if uncertain
    Moderate,
    /// Code generation, analysis, multi-step reasoning -> primary model
    Complex,
}

/// Configuration for smart routing classification.
#[derive(Debug, Clone)]
pub struct SmartRoutingConfig {
    /// Enable cascade mode: retry with primary if cheap model response seems uncertain.
    pub cascade_enabled: bool,
    /// Message length threshold below which a message may be classified as Simple.
    pub simple_max_chars: usize,
    /// Message length threshold above which a message is classified as Complex.
    pub complex_min_chars: usize,
}

impl Default for SmartRoutingConfig {
    fn default() -> Self {
        Self {
            cascade_enabled: true,
            simple_max_chars: 200,
            complex_min_chars: 1000,
        }
    }
}

/// Classify a message's complexity based on content patterns and length.
pub fn classify_message(msg: &str, config: &SmartRoutingConfig) -> TaskComplexity {
    let trimmed = msg.trim();
    let len = trimmed.len();

    if len == 0 {
        return TaskComplexity::Simple;
    }

    if trimmed.contains("```") {
        return TaskComplexity::Complex;
    }

    let lower = trimmed.to_lowercase();

    const COMPLEX_KEYWORDS: &[&str] = &[
        "implement",
        "refactor",
        "analyze",
        "debug",
        "create a",
        "build a",
        "design",
        "fix the",
        "fix this",
        "write a",
        "write the",
        "explain how",
        "explain why",
        "explain the",
        "compare",
        "optimize",
        "review",
        "rewrite",
        "migrate",
        "architect",
        "integrate",
    ];

    if COMPLEX_KEYWORDS
        .iter()
        .any(|keyword| lower.contains(keyword))
    {
        return TaskComplexity::Complex;
    }

    if len >= config.complex_min_chars {
        return TaskComplexity::Complex;
    }

    const SIMPLE_KEYWORDS: &[&str] = &[
        "list",
        "show",
        "what is",
        "what's",
        "status",
        "help",
        "yes",
        "no",
        "ok",
        "thanks",
        "thank you",
        "hello",
        "hi",
        "hey",
        "ping",
        "version",
        "how many",
        "when",
        "where is",
        "who",
    ];

    if len <= config.simple_max_chars
        && SIMPLE_KEYWORDS
            .iter()
            .any(|keyword| lower.contains(keyword))
    {
        return TaskComplexity::Simple;
    }

    if len <= 10 {
        return TaskComplexity::Simple;
    }

    TaskComplexity::Moderate
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> SmartRoutingConfig {
        SmartRoutingConfig::default()
    }

    #[test]
    fn classifies_simple_messages() {
        assert_eq!(
            classify_message("", &default_config()),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_message("hello", &default_config()),
            TaskComplexity::Simple
        );
        assert_eq!(
            classify_message("what is the status?", &default_config()),
            TaskComplexity::Simple
        );
    }

    #[test]
    fn classifies_complex_messages() {
        assert_eq!(
            classify_message("implement a binary search function", &default_config()),
            TaskComplexity::Complex
        );
        assert_eq!(
            classify_message("```rust\nfn main() {}\n```", &default_config()),
            TaskComplexity::Complex
        );
    }

    #[test]
    fn classifies_moderate_messages() {
        assert_eq!(
            classify_message("Tell me more about that idea.", &default_config()),
            TaskComplexity::Moderate
        );
    }
}
