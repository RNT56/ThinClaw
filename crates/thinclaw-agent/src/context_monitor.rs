//! Context window monitoring and compaction triggers.
//!
//! Monitors the size of the conversation context and triggers
//! compaction when approaching the limit.

use serde::{Deserialize, Serialize};

use thinclaw_llm_core::{ChatMessage, Role};

/// Default context window limit (conservative estimate).
const DEFAULT_CONTEXT_LIMIT: usize = 100_000;

/// Compaction threshold as a percentage of the limit.
const COMPACTION_THRESHOLD: f64 = 0.8;

/// Approximate tokens per word (rough estimate for English).
const TOKENS_PER_WORD: f64 = 1.3;

/// Context pressure levels derived from approximate context usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ContextPressure {
    /// Context is comfortably below warning thresholds.
    #[default]
    None,
    /// Context is approaching the limit.
    Warning,
    /// Context is critically full.
    Critical,
}

impl ContextPressure {
    /// Convert a usage percentage into a pressure level.
    pub fn from_usage_percent(usage_percent: f32) -> Self {
        if !usage_percent.is_finite() {
            return Self::None;
        }

        if usage_percent >= 95.0 {
            Self::Critical
        } else if usage_percent >= 85.0 {
            Self::Warning
        } else {
            Self::None
        }
    }
}

/// Strategy for context compaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionStrategy {
    /// Summarize old messages and keep recent ones.
    Summarize {
        /// Number of recent turns to keep intact.
        keep_recent: usize,
    },
    /// Truncate old messages without summarization.
    Truncate {
        /// Number of recent turns to keep.
        keep_recent: usize,
    },
    /// Move context to workspace memory.
    MoveToWorkspace,
}

impl Default for CompactionStrategy {
    fn default() -> Self {
        Self::Summarize { keep_recent: 5 }
    }
}

/// Monitors context size and suggests compaction.
pub struct ContextMonitor {
    /// Maximum tokens allowed in context.
    context_limit: usize,
    /// Threshold ratio for triggering compaction.
    threshold_ratio: f64,
}

impl ContextMonitor {
    /// Create a new context monitor with default settings.
    pub fn new() -> Self {
        Self {
            context_limit: DEFAULT_CONTEXT_LIMIT,
            threshold_ratio: COMPACTION_THRESHOLD,
        }
    }

    /// Create with a custom context limit.
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.context_limit = limit;
        self
    }

    /// Create with a custom threshold ratio.
    pub fn with_threshold(mut self, ratio: f64) -> Self {
        self.threshold_ratio = ratio.clamp(0.5, 0.95);
        self
    }

    /// Estimate the token count for a list of messages.
    pub fn estimate_tokens(&self, messages: &[ChatMessage]) -> usize {
        messages.iter().map(estimate_message_tokens).sum()
    }

    /// Check if compaction is needed.
    pub fn needs_compaction(&self, messages: &[ChatMessage]) -> bool {
        let tokens = self.estimate_tokens(messages);
        let threshold = (self.context_limit as f64 * self.threshold_ratio) as usize;
        tokens >= threshold
    }

    /// Get the current usage percentage.
    pub fn usage_percent(&self, messages: &[ChatMessage]) -> f64 {
        let tokens = self.estimate_tokens(messages);
        (tokens as f64 / self.context_limit as f64) * 100.0
    }

    /// Check the current pressure level from a usage percentage.
    pub fn check_pressure(&self, usage_percent: f32) -> ContextPressure {
        ContextPressure::from_usage_percent(usage_percent)
    }

    /// Suggest a compaction strategy based on current context.
    pub fn suggest_compaction(&self, messages: &[ChatMessage]) -> Option<CompactionStrategy> {
        if !self.needs_compaction(messages) {
            return None;
        }

        let tokens = self.estimate_tokens(messages);
        let overage = tokens as f64 / self.context_limit as f64;

        if overage > 0.95 {
            // Critical: aggressive truncation
            Some(CompactionStrategy::Truncate { keep_recent: 3 })
        } else if overage > 0.85 {
            // High: summarize and keep fewer
            Some(CompactionStrategy::Summarize { keep_recent: 5 })
        } else {
            // Moderate: move to workspace
            Some(CompactionStrategy::MoveToWorkspace)
        }
    }

    /// Get the context limit.
    pub fn limit(&self) -> usize {
        self.context_limit
    }

    /// Get the current threshold in tokens.
    pub fn threshold(&self) -> usize {
        (self.context_limit as f64 * self.threshold_ratio) as usize
    }
}

/// Get the warning message for a given pressure level.
pub fn pressure_message(pressure: ContextPressure) -> Option<String> {
    match pressure {
        ContextPressure::None => None,
        ContextPressure::Warning => Some(
            "⚠ Context window 85% full — consider /compress (/compact) or starting a /new thread"
                .to_string(),
        ),
        ContextPressure::Critical => {
            Some("🔴 Context window 95% full — auto-compaction imminent".to_string())
        }
    }
}

/// Return the pressure level that should trigger a warning when transitioning
/// from the previous persisted level to the current one.
pub fn pressure_transition(
    previous: Option<ContextPressure>,
    current: ContextPressure,
) -> Option<ContextPressure> {
    if current == ContextPressure::None {
        return None;
    }

    if previous != Some(current) {
        Some(current)
    } else {
        None
    }
}

impl Default for ContextMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Estimate tokens for a single message.
fn estimate_message_tokens(message: &ChatMessage) -> usize {
    // Use word-based estimation as it's more accurate for varied content
    let word_count = message.content.split_whitespace().count();

    // Add overhead for role and structure
    let overhead = 4; // ~4 tokens for role and message structure

    let text_tokens = (word_count as f64 * TOKENS_PER_WORD) as usize + overhead;

    // Estimate tokens for multimodal attachments.
    // Image tokens vary by resolution, but roughly:
    //   - LLM APIs base64-encode images (~4/3x inflation)
    //   - Tokenizers average ~1 token per 4 characters of base64
    //   - So ~1 token per 3 bytes of raw image data
    //   - Plus fixed overhead per image (~85 tokens for metadata/framing)
    let attachment_tokens: usize = message
        .attachments
        .iter()
        .map(|att| {
            let raw_bytes = att.size();
            let base64_chars = raw_bytes * 4 / 3;
            let image_tokens = base64_chars / 4; // ~1 token per 4 chars
            image_tokens + 85 // per-image overhead
        })
        .sum();

    text_tokens + attachment_tokens
}

/// Estimate tokens for raw text.
pub fn estimate_text_tokens(text: &str) -> usize {
    let word_count = text.split_whitespace().count();
    (word_count as f64 * TOKENS_PER_WORD) as usize
}

/// Context size breakdown for reporting.
#[derive(Debug, Clone)]
pub struct ContextBreakdown {
    /// Total estimated tokens.
    pub total_tokens: usize,
    /// System message tokens.
    pub system_tokens: usize,
    /// User message tokens.
    pub user_tokens: usize,
    /// Assistant message tokens.
    pub assistant_tokens: usize,
    /// Tool result tokens.
    pub tool_tokens: usize,
    /// Number of messages.
    pub message_count: usize,
}

impl ContextBreakdown {
    /// Analyze a list of messages.
    pub fn analyze(messages: &[ChatMessage]) -> Self {
        let mut breakdown = Self {
            total_tokens: 0,
            system_tokens: 0,
            user_tokens: 0,
            assistant_tokens: 0,
            tool_tokens: 0,
            message_count: messages.len(),
        };

        for message in messages {
            let tokens = estimate_message_tokens(message);
            breakdown.total_tokens += tokens;

            match message.role {
                Role::System => breakdown.system_tokens += tokens,
                Role::User => breakdown.user_tokens += tokens,
                Role::Assistant => breakdown.assistant_tokens += tokens,
                Role::Tool => breakdown.tool_tokens += tokens,
            }
        }

        breakdown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_estimation() {
        let msg = ChatMessage::user("Hello, how are you today?");
        let tokens = estimate_message_tokens(&msg);
        // 5 words * 1.3 + 4 overhead = ~10-11 tokens
        assert!(tokens > 0);
        assert!(tokens < 20);
    }

    #[test]
    fn test_needs_compaction() {
        let monitor = ContextMonitor::new().with_limit(100);

        // Small context - no compaction needed
        let small: Vec<ChatMessage> = vec![ChatMessage::user("Hello")];
        assert!(!monitor.needs_compaction(&small));

        // Large context - compaction needed
        let large_content = "word ".repeat(1000);
        let large: Vec<ChatMessage> = vec![ChatMessage::user(&large_content)];
        assert!(monitor.needs_compaction(&large));
    }

    #[test]
    fn test_suggest_compaction() {
        let monitor = ContextMonitor::new().with_limit(100);

        let small: Vec<ChatMessage> = vec![ChatMessage::user("Hello")];
        assert!(monitor.suggest_compaction(&small).is_none());
    }

    #[test]
    fn test_context_breakdown() {
        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
        ];

        let breakdown = ContextBreakdown::analyze(&messages);
        assert_eq!(breakdown.message_count, 3);
        assert!(breakdown.system_tokens > 0);
        assert!(breakdown.user_tokens > 0);
        assert!(breakdown.assistant_tokens > 0);
    }

    #[test]
    fn test_pressure_thresholds() {
        let monitor = ContextMonitor::new();
        assert_eq!(monitor.check_pressure(84.9), ContextPressure::None);
        assert_eq!(monitor.check_pressure(85.0), ContextPressure::Warning);
        assert_eq!(monitor.check_pressure(94.9), ContextPressure::Warning);
        assert_eq!(monitor.check_pressure(95.0), ContextPressure::Critical);
        assert_eq!(monitor.check_pressure(f32::NAN), ContextPressure::None);
    }

    #[test]
    fn test_pressure_message_and_transition() {
        assert!(pressure_message(ContextPressure::None).is_none());
        assert!(
            pressure_message(ContextPressure::Warning)
                .unwrap()
                .contains("85% full")
        );
        assert!(
            pressure_message(ContextPressure::Critical)
                .unwrap()
                .contains("95% full")
        );

        assert_eq!(
            pressure_transition(None, ContextPressure::Warning),
            Some(ContextPressure::Warning)
        );
        assert_eq!(
            pressure_transition(Some(ContextPressure::Warning), ContextPressure::Warning),
            None
        );
        assert_eq!(
            pressure_transition(Some(ContextPressure::Critical), ContextPressure::Warning),
            Some(ContextPressure::Warning)
        );
        assert_eq!(
            pressure_transition(Some(ContextPressure::Critical), ContextPressure::None),
            None
        );
    }
}
