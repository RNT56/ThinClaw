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

/// Approximate tokens per whitespace-delimited word (rough estimate for
/// space-separated prose in Latin scripts).
const TOKENS_PER_WORD: f64 = 1.3;

/// Approximate characters per token for the character-based estimate. This is
/// the widely used "~4 chars per token" rule of thumb for BPE tokenizers on
/// mixed English / code / JSON content.
const CHARS_PER_TOKEN: usize = 4;

/// Default reserve for tokenizer variance, provider framing, and model-specific
/// accounting that is not visible to the provider-neutral estimator.
pub const AUXILIARY_CONTEXT_SAFETY_MARGIN_PERCENT: u8 = 10;

const RECENT_EVIDENCE_TRUNCATION_MARKER: &str =
    "[... older evidence omitted to fit the active model context ...]\n";

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

    /// Stable wire label used by channel and UI event contracts.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Warning => "warning",
            Self::Critical => "critical",
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
#[derive(Debug, Clone, Copy)]
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

/// A typed untrusted-evidence message fitted to a concrete provider request
/// budget. The newest evidence is retained when truncation is necessary.
#[derive(Debug, Clone)]
pub struct BoundedUntrustedContext {
    pub message: ChatMessage,
    pub was_truncated: bool,
    pub retained_chars: usize,
    pub estimated_input_tokens: usize,
    pub input_token_limit: usize,
}

/// Fit one untrusted evidence block beside fixed request messages while
/// reserving output tokens and a percentage of the total model window.
///
/// This is intended for auxiliary LLM paths such as compaction, summaries,
/// search synthesis, and heartbeat checks. Those paths do not run the main
/// dispatcher history cap, so merely limiting the number of messages is not a
/// sufficient context bound: one message may itself be larger than the model
/// window.
///
/// Returns `None` only when the fixed messages plus an empty typed evidence
/// envelope cannot fit. Callers should fail closed rather than sending an
/// inevitably oversized request to the provider.
pub fn bound_recent_untrusted_context(
    monitor: &ContextMonitor,
    fixed_messages: &[ChatMessage],
    segment_id: &str,
    source: &str,
    content: &str,
    output_reserve_tokens: usize,
    safety_margin_percent: u8,
) -> Option<BoundedUntrustedContext> {
    let safety_margin = monitor
        .limit()
        .saturating_mul(safety_margin_percent.min(95) as usize)
        / 100;
    let input_token_limit = monitor
        .limit()
        .saturating_sub(output_reserve_tokens)
        .saturating_sub(safety_margin);
    let fixed_tokens = monitor.estimate_tokens(fixed_messages);

    let full_message = ChatMessage::untrusted_context(segment_id, source, content);
    let full_tokens =
        fixed_tokens.saturating_add(monitor.estimate_tokens(std::slice::from_ref(&full_message)));
    if full_tokens <= input_token_limit {
        return Some(BoundedUntrustedContext {
            message: full_message,
            was_truncated: false,
            retained_chars: content.chars().count(),
            estimated_input_tokens: full_tokens,
            input_token_limit,
        });
    }

    let empty_message = ChatMessage::untrusted_context(segment_id, source, "");
    let empty_tokens =
        fixed_tokens.saturating_add(monitor.estimate_tokens(std::slice::from_ref(&empty_message)));
    if empty_tokens > input_token_limit {
        return None;
    }

    let total_chars = content.chars().count();
    let candidate = |retained_chars: usize| {
        if retained_chars == 0 {
            return String::new();
        }
        let skipped_chars = total_chars.saturating_sub(retained_chars);
        let start = content
            .char_indices()
            .nth(skipped_chars)
            .map_or(content.len(), |(index, _)| index);
        format!("{RECENT_EVIDENCE_TRUNCATION_MARKER}{}", &content[start..])
    };

    // Binary search the largest UTF-8-safe suffix whose fully rendered typed
    // envelope fits. Measuring the rendered message also accounts for JSON
    // escaping and envelope framing rather than guessing from raw bytes.
    let mut low = 0usize;
    let mut high = total_chars;
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let raw = candidate(mid);
        let message = ChatMessage::untrusted_context(segment_id, source, raw);
        let tokens =
            fixed_tokens.saturating_add(monitor.estimate_tokens(std::slice::from_ref(&message)));
        if tokens <= input_token_limit {
            low = mid;
        } else {
            high = mid - 1;
        }
    }

    let raw = candidate(low);
    let message = ChatMessage::untrusted_context(segment_id, source, raw);
    let estimated_input_tokens =
        fixed_tokens.saturating_add(monitor.estimate_tokens(std::slice::from_ref(&message)));
    Some(BoundedUntrustedContext {
        message,
        was_truncated: true,
        retained_chars: low,
        estimated_input_tokens,
        input_token_limit,
    })
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

/// Whether a structured pressure state event should be emitted.
///
/// The initial healthy state is implicit, but every later level change is
/// explicit, including recovery to `None`, so persistent UI indicators clear.
pub fn pressure_state_changed(previous: Option<ContextPressure>, current: ContextPressure) -> bool {
    previous != Some(current) && (previous.is_some() || current != ContextPressure::None)
}

impl Default for ContextMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns true for codepoints that a BPE tokenizer typically encodes at
/// roughly one token per character (CJK ideographs, kana, Hangul, and common
/// full-width forms). Whitespace-word counting badly undercounts these scripts
/// because they are not space-delimited, so we count them per-character.
fn is_dense_script(ch: char) -> bool {
    matches!(ch as u32,
        0x3040..=0x30FF |   // Hiragana + Katakana
        0x3400..=0x4DBF |   // CJK Unified Ideographs Extension A
        0x4E00..=0x9FFF |   // CJK Unified Ideographs
        0xAC00..=0xD7AF |   // Hangul syllables
        0xF900..=0xFAFF |   // CJK Compatibility Ideographs
        0xFF00..=0xFFEF |   // Half/Full-width forms
        0x20000..=0x2FA1F   // CJK Extension B+ / Supplement
    )
}

/// Estimate tokens for raw text using a content-aware heuristic.
///
/// Combines two independent estimates and takes the larger (more conservative)
/// so we never *under*-count relative to a real tokenizer, which is the failure
/// mode that lets a request overflow the provider's context window:
///
/// - **Character-based** (`~4 chars/token`), with dense scripts (CJK/kana/Hangul)
///   counted at ~1 token each. This dominates for code, JSON tool payloads, and
///   non-Latin text, where whitespace-word counting is wildly inaccurate.
/// - **Word-based** (`words * 1.3`), a good fit for ordinary Latin-script prose.
///
/// This is not an exact BPE tokenizer (which would require a per-provider
/// vocabulary and cannot cover Anthropic locally), but it is dramatically closer
/// than pure word counting for the content types agents actually generate.
pub fn estimate_text_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }

    let mut dense_chars = 0usize;
    let mut other_chars = 0usize;
    for ch in text.chars() {
        if is_dense_script(ch) {
            dense_chars += 1;
        } else {
            other_chars += 1;
        }
    }
    // Ceiling division for the Latin/code portion, plus ~1 token per dense char.
    let char_based = dense_chars + other_chars.div_ceil(CHARS_PER_TOKEN);

    let word_count = text.split_whitespace().count();
    let word_based = (word_count as f64 * TOKENS_PER_WORD) as usize;

    char_based.max(word_based)
}

/// Estimate tokens for a single message, including tool-call JSON and media.
fn estimate_message_tokens(message: &ChatMessage) -> usize {
    // Add overhead for role framing and message structure (~4 tokens).
    let overhead = 4;

    let mut text_tokens = estimate_text_tokens(&message.content) + overhead;

    // Tool-call payloads (name + serialized arguments) are real context that
    // the previous word-based estimator ignored entirely — an assistant turn
    // that calls tools with large JSON arguments could be undercounted by
    // thousands of tokens. Account for both the request and result framing.
    if let Some(ref tool_calls) = message.tool_calls {
        for call in tool_calls {
            text_tokens += estimate_text_tokens(&call.name);
            text_tokens += estimate_text_tokens(&call.arguments.to_string());
            text_tokens += 8; // per-tool-call id + JSON framing overhead
        }
    }
    if let Some(ref name) = message.name {
        text_tokens += estimate_text_tokens(name) + 2;
    }
    if message.tool_call_id.is_some() {
        text_tokens += 4; // tool_call_id linkage overhead
    }

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
        assert!(tokens > 0);
        assert!(tokens < 20);
    }

    #[test]
    fn test_dense_json_not_undercounted() {
        // Dense JSON has few whitespace "words" but many tokens. The char-based
        // estimate must dominate so we don't wildly undercount tool payloads.
        let json = r#"{"a":1,"b":2,"c":3,"d":4,"e":5,"f":6,"g":7,"h":8,"i":9}"#;
        let word_based = (json.split_whitespace().count() as f64 * TOKENS_PER_WORD) as usize;
        let estimate = estimate_text_tokens(json);
        assert!(
            estimate > word_based * 3,
            "dense JSON estimate {estimate} should far exceed word-based {word_based}"
        );
    }

    #[test]
    fn test_cjk_counted_per_character() {
        // No whitespace words at all, but ~12 tokens of real content.
        let cjk = "今日はいい天気ですね本当に";
        let word_based = (cjk.split_whitespace().count() as f64 * TOKENS_PER_WORD) as usize;
        let estimate = estimate_text_tokens(cjk);
        assert!(word_based <= 2, "sanity: CJK has ~no whitespace words");
        assert!(
            estimate >= cjk.chars().count(),
            "CJK estimate {estimate} should be ~1 token per char"
        );
    }

    #[test]
    fn test_tool_call_arguments_are_counted() {
        // An assistant message whose tool-call arguments carry a large payload
        // must not be estimated as if it were empty text.
        let big_args = serde_json::json!({ "blob": "x".repeat(4000) });
        let with_tools = ChatMessage::assistant_with_tool_calls(
            None,
            vec![thinclaw_llm_core::ToolCall {
                id: "c0".to_string(),
                name: "write".to_string(),
                arguments: big_args,
            }],
        );
        let bare = ChatMessage::assistant("");
        let tool_tokens = estimate_message_tokens(&with_tools);
        let bare_tokens = estimate_message_tokens(&bare);
        assert!(
            tool_tokens > bare_tokens + 800,
            "tool-call args ({tool_tokens}) must be counted vs bare ({bare_tokens})"
        );
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
    fn auxiliary_evidence_is_bounded_and_retains_recent_utf8_content() {
        let monitor = ContextMonitor::new().with_limit(500);
        let fixed = vec![ChatMessage::system("Summarize the evidence.")];
        let old = "old-日本語-".repeat(800);
        let recent = "RECENT-DECISION-日本語";
        let content = format!("{old}{recent}");

        let bounded = bound_recent_untrusted_context(
            &monitor,
            &fixed,
            "conversation",
            "test",
            &content,
            100,
            AUXILIARY_CONTEXT_SAFETY_MARGIN_PERCENT,
        )
        .expect("fixed prompt should leave room for evidence");

        assert!(bounded.was_truncated);
        assert!(bounded.retained_chars < content.chars().count());
        assert!(bounded.message.content.contains(recent));
        assert!(bounded.message.content.contains("older evidence omitted"));
        assert!(bounded.estimated_input_tokens <= bounded.input_token_limit);
        assert_eq!(
            bounded.message.untrusted_context_identity(),
            Some(("conversation", "test"))
        );
    }

    #[test]
    fn auxiliary_evidence_fails_when_required_request_cannot_fit() {
        let monitor = ContextMonitor::new().with_limit(64);
        let fixed = vec![ChatMessage::system("x".repeat(1_000))];
        assert!(
            bound_recent_untrusted_context(
                &monitor,
                &fixed,
                "conversation",
                "test",
                "evidence",
                16,
                AUXILIARY_CONTEXT_SAFETY_MARGIN_PERCENT,
            )
            .is_none()
        );
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

        assert!(!pressure_state_changed(None, ContextPressure::None));
        assert!(pressure_state_changed(None, ContextPressure::Warning));
        assert!(!pressure_state_changed(
            Some(ContextPressure::Warning),
            ContextPressure::Warning
        ));
        assert!(pressure_state_changed(
            Some(ContextPressure::Warning),
            ContextPressure::Critical
        ));
        assert!(pressure_state_changed(
            Some(ContextPressure::Critical),
            ContextPressure::None
        ));
        assert_eq!(ContextPressure::Critical.as_str(), "critical");
    }
}
