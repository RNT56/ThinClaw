//! LLM input/output hooks.
//!
//! Wraps the LLM call pipeline with before/after hooks for:
//! - Content inspection and modification
//! - Token usage tracking
//! - Latency monitoring
//! - Content filtering

use std::time::Instant;

use serde::{Deserialize, Serialize};

/// LLM hook event — emitted before and after LLM calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmHookEvent {
    /// Hook point (before_input / after_output).
    pub hook_point: LlmHookPoint,
    /// Model being used.
    pub model: String,
    /// Message content (may be modified by hooks).
    pub content: String,
    /// Token count (estimated or actual).
    pub token_count: Option<u32>,
    /// Provider name.
    pub provider: Option<String>,
    /// Session ID.
    pub session_id: Option<String>,
}

/// Where in the pipeline this hook fires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LlmHookPoint {
    /// Before sending to the LLM.
    BeforeInput,
    /// After receiving from the LLM.
    AfterOutput,
}

/// Result of processing a hook.
#[derive(Debug, Clone)]
pub enum HookResult {
    /// Continue with potentially modified content.
    Continue(String),
    /// Block the request/response with a reason.
    Block(String),
    /// Pass through unchanged.
    PassThrough,
}

/// A registered LLM hook.
pub struct LlmHook {
    /// Hook name.
    pub name: String,
    /// Which point this hook fires at.
    pub hook_point: LlmHookPoint,
    /// Priority (lower = earlier execution).
    pub priority: i32,
    /// The hook function.
    handler: Box<dyn Fn(&LlmHookEvent) -> HookResult + Send + Sync>,
}

impl LlmHook {
    /// Create a new hook.
    pub fn new(
        name: impl Into<String>,
        point: LlmHookPoint,
        priority: i32,
        handler: impl Fn(&LlmHookEvent) -> HookResult + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            hook_point: point,
            priority,
            handler: Box::new(handler),
        }
    }

    /// Execute the hook.
    pub fn execute(&self, event: &LlmHookEvent) -> HookResult {
        (self.handler)(event)
    }
}

/// LLM hook registry.
pub struct LlmHookRegistry {
    hooks: Vec<LlmHook>,
}

impl LlmHookRegistry {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Register a hook.
    pub fn register(&mut self, hook: LlmHook) {
        self.hooks.push(hook);
        self.hooks.sort_by_key(|h| h.priority);
    }

    /// Execute all hooks for a given point.
    pub fn execute(&self, event: &mut LlmHookEvent) -> Result<(), String> {
        for hook in &self.hooks {
            if hook.hook_point != event.hook_point {
                continue;
            }

            match hook.execute(event) {
                HookResult::Continue(modified) => {
                    event.content = modified;
                }
                HookResult::Block(reason) => {
                    return Err(format!("Blocked by hook '{}': {}", hook.name, reason));
                }
                HookResult::PassThrough => {}
            }
        }
        Ok(())
    }

    /// Number of registered hooks.
    pub fn hook_count(&self) -> usize {
        self.hooks.len()
    }

    /// List hook names.
    pub fn hook_names(&self) -> Vec<&str> {
        self.hooks.iter().map(|h| h.name.as_str()).collect()
    }
}

impl Default for LlmHookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// LLM call metrics collected around hooks.
#[derive(Debug, Clone, Serialize)]
pub struct LlmCallMetrics {
    pub model: String,
    pub provider: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub latency_ms: u64,
    pub cached: bool,
}

/// Timer for measuring LLM call latency.
pub struct LlmCallTimer {
    model: String,
    provider: String,
    start: Instant,
}

impl LlmCallTimer {
    pub fn start(model: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            provider: provider.into(),
            start: Instant::now(),
        }
    }

    pub fn finish(self, input_tokens: u32, output_tokens: u32, cached: bool) -> LlmCallMetrics {
        LlmCallMetrics {
            model: self.model,
            provider: self.provider,
            input_tokens,
            output_tokens,
            latency_ms: self.start.elapsed().as_millis() as u64,
            cached,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event(point: LlmHookPoint, content: &str) -> LlmHookEvent {
        LlmHookEvent {
            hook_point: point,
            model: "gpt-4o".to_string(),
            content: content.to_string(),
            token_count: None,
            provider: None,
            session_id: None,
        }
    }

    #[test]
    fn test_passthrough_hook() {
        let mut registry = LlmHookRegistry::new();
        registry.register(LlmHook::new("noop", LlmHookPoint::BeforeInput, 0, |_| {
            HookResult::PassThrough
        }));

        let mut event = make_event(LlmHookPoint::BeforeInput, "hello");
        assert!(registry.execute(&mut event).is_ok());
        assert_eq!(event.content, "hello");
    }

    #[test]
    fn test_modify_hook() {
        let mut registry = LlmHookRegistry::new();
        registry.register(LlmHook::new(
            "uppercase",
            LlmHookPoint::BeforeInput,
            0,
            |e| HookResult::Continue(e.content.to_uppercase()),
        ));

        let mut event = make_event(LlmHookPoint::BeforeInput, "hello");
        registry.execute(&mut event).unwrap();
        assert_eq!(event.content, "HELLO");
    }

    #[test]
    fn test_block_hook() {
        let mut registry = LlmHookRegistry::new();
        registry.register(LlmHook::new(
            "blocker",
            LlmHookPoint::BeforeInput,
            0,
            |_| HookResult::Block("unsafe content".to_string()),
        ));

        let mut event = make_event(LlmHookPoint::BeforeInput, "bad content");
        let result = registry.execute(&mut event);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unsafe content"));
    }

    #[test]
    fn test_hook_point_filtering() {
        let mut registry = LlmHookRegistry::new();
        registry.register(LlmHook::new(
            "input-only",
            LlmHookPoint::BeforeInput,
            0,
            |_| HookResult::Continue("MODIFIED".to_string()),
        ));

        // Should NOT fire on AfterOutput
        let mut event = make_event(LlmHookPoint::AfterOutput, "hello");
        registry.execute(&mut event).unwrap();
        assert_eq!(event.content, "hello"); // Unchanged
    }

    #[test]
    fn test_priority_ordering() {
        let mut registry = LlmHookRegistry::new();
        registry.register(LlmHook::new("second", LlmHookPoint::BeforeInput, 10, |e| {
            HookResult::Continue(format!("{}-second", e.content))
        }));
        registry.register(LlmHook::new("first", LlmHookPoint::BeforeInput, 1, |e| {
            HookResult::Continue(format!("{}-first", e.content))
        }));

        let mut event = make_event(LlmHookPoint::BeforeInput, "start");
        registry.execute(&mut event).unwrap();
        assert_eq!(event.content, "start-first-second");
    }

    #[test]
    fn test_hook_names() {
        let mut registry = LlmHookRegistry::new();
        registry.register(LlmHook::new("a", LlmHookPoint::BeforeInput, 0, |_| {
            HookResult::PassThrough
        }));
        registry.register(LlmHook::new("b", LlmHookPoint::AfterOutput, 0, |_| {
            HookResult::PassThrough
        }));

        assert_eq!(registry.hook_count(), 2);
        let names = registry.hook_names();
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
    }

    #[test]
    fn test_call_timer() {
        let timer = LlmCallTimer::start("gpt-4o", "openai");
        std::thread::sleep(std::time::Duration::from_millis(10));
        let metrics = timer.finish(100, 50, false);

        assert_eq!(metrics.model, "gpt-4o");
        assert!(metrics.latency_ms >= 10);
        assert_eq!(metrics.input_tokens, 100);
    }
}
