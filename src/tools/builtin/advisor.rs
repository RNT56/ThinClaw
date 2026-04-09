//! Advisor consultation tool for the AdvisorExecutor routing mode.
//!
//! When routing mode is AdvisorExecutor, this tool is injected into the
//! executor model's tool set. When the executor calls `consult_advisor`,
//! the dispatcher intercepts the call and routes it to the advisor
//! (primary/expensive) model for guidance.
//!
//! The advisor never calls tools and never produces user-facing output —
//! it returns strategic guidance that the executor uses to continue.
//!
//! Reference: <https://claude.com/blog/the-advisor-strategy>

use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;

use crate::context::JobContext;
use crate::error::LlmError;
use crate::llm::{
    ChatMessage, CompletionRequest, LlmProvider,
};
use crate::llm::route_planner::{AdvisorConfig, ADVISOR_SYSTEM_PROMPT};
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Name of the advisor tool that executors can call.
pub const ADVISOR_TOOL_NAME: &str = "consult_advisor";

/// Built-in tool that lets the executor model consult the advisor.
///
/// The executor calls this when it's stuck, uncertain about approach,
/// or needs frontier-level reasoning for a hard decision.
///
/// The advisor receives:
/// - The executor's question/context
/// - A summary of the conversation so far
///
/// The advisor returns:
/// - A plan, correction, or stop signal
/// - The advisor NEVER calls tools or produces user-facing output
pub struct ConsultAdvisorTool;

#[async_trait]
impl Tool for ConsultAdvisorTool {
    fn name(&self) -> &str {
        ADVISOR_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Consult a more capable model for guidance when you're stuck, uncertain \
         about the best approach, or facing a complex decision point. The advisor \
         will return a plan, correction, or recommendation. Use sparingly — only \
         when the decision genuinely requires deeper reasoning."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "What you need help with. Be specific about \
                                   the decision point, tradeoffs, or uncertainty."
                },
                "context_summary": {
                    "type": "string",
                    "description": "Brief summary of what you've done so far \
                                   and what you're trying to achieve."
                }
            },
            "required": ["question"]
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        // This tool is NEVER executed through the normal tool pipeline.
        // The dispatcher intercepts `consult_advisor` calls and routes them
        // directly to the advisor model. If we get here, something is wrong.
        Err(ToolError::ExecutionFailed(
            "consult_advisor should be intercepted by the dispatcher. \
             If you see this error, AdvisorExecutor mode may not be properly configured."
                .to_string(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }
}

/// Per-turn advisor call counter.
///
/// Tracks how many times the executor has consulted the advisor in the
/// current agent turn to enforce the `max_advisor_calls` budget.
#[derive(Debug)]
pub struct AdvisorCallBudget {
    calls: AtomicU32,
    max_calls: u32,
}

impl AdvisorCallBudget {
    /// Create a new budget with the given maximum.
    pub fn new(max_calls: u32) -> Self {
        Self {
            calls: AtomicU32::new(0),
            max_calls,
        }
    }

    /// Try to consume one advisor call. Returns the current count if
    /// the budget allows, or an error if exceeded.
    pub fn try_consume(&self) -> Result<u32, String> {
        let current = self.calls.fetch_add(1, Ordering::Relaxed);
        if current >= self.max_calls {
            self.calls.fetch_sub(1, Ordering::Relaxed); // rollback
            Err(format!(
                "Advisor call limit reached ({}/{} calls used this turn). \
                 Continue without advisor guidance.",
                self.max_calls, self.max_calls
            ))
        } else {
            Ok(current + 1)
        }
    }

    /// Reset the counter (e.g. at the start of a new turn).
    pub fn reset(&self) {
        self.calls.store(0, Ordering::Relaxed);
    }

    /// Current number of calls made.
    pub fn current(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}

/// Execute an advisor consultation.
///
/// Called by the dispatcher when it intercepts a `consult_advisor` tool call.
/// Routes the executor's question to the advisor model and returns the guidance.
pub async fn execute_advisor_consultation(
    advisor_provider: &dyn LlmProvider,
    config: &AdvisorConfig,
    question: &str,
    context_summary: Option<&str>,
    conversation_context: &[ChatMessage],
) -> Result<String, LlmError> {
    // Build advisor request
    let system_prompt = if config.advisor_system_prompt.is_empty() {
        ADVISOR_SYSTEM_PROMPT.to_string()
    } else {
        config.advisor_system_prompt.clone()
    };

    // Include a trimmed conversation context for the advisor
    let mut context_text = String::new();
    if let Some(summary) = context_summary {
        context_text.push_str("## Executor Context\n");
        context_text.push_str(summary);
        context_text.push_str("\n\n");
    }

    // Include recent conversation messages (trimmed to last ~10 for context window)
    let recent_msgs: Vec<_> = conversation_context
        .iter()
        .rev()
        .take(10)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    if !recent_msgs.is_empty() {
        context_text.push_str("## Recent Conversation\n");
        for msg in &recent_msgs {
            context_text.push_str(&format!(
                "[{:?}]: {}\n",
                msg.role,
                if msg.content.len() > 500 {
                    format!("{}...", &msg.content[..500])
                } else {
                    msg.content.clone()
                }
            ));
        }
        context_text.push('\n');
    }

    let advisor_request = CompletionRequest::new(vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(format!(
            "The executor model needs guidance.\n\n\
             ## Question\n{}\n\n\
             {}",
            question, context_text
        )),
    ])
    .with_max_tokens(1024); // Advisor generates short guidance

    let response = advisor_provider.complete(advisor_request).await?;

    tracing::info!(
        advisor_model = %advisor_provider.active_model_name(),
        input_tokens = response.input_tokens,
        output_tokens = response.output_tokens,
        cost_usd = ?response.cost_usd,
        "Advisor consultation complete"
    );

    Ok(response.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advisor_tool_name() {
        let tool = ConsultAdvisorTool;
        assert_eq!(tool.name(), "consult_advisor");
    }

    #[test]
    fn advisor_tool_schema() {
        let tool = ConsultAdvisorTool;
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["question"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"question".into())
        );
    }

    #[test]
    fn advisor_call_budget_basic() {
        let budget = AdvisorCallBudget::new(3);
        assert_eq!(budget.current(), 0);
        assert!(budget.try_consume().is_ok());
        assert_eq!(budget.current(), 1);
        assert!(budget.try_consume().is_ok());
        assert_eq!(budget.current(), 2);
        assert!(budget.try_consume().is_ok());
        assert_eq!(budget.current(), 3);
        // 4th call should fail
        assert!(budget.try_consume().is_err());
        assert_eq!(budget.current(), 3); // rollback
    }

    #[test]
    fn advisor_call_budget_reset() {
        let budget = AdvisorCallBudget::new(2);
        assert!(budget.try_consume().is_ok());
        assert!(budget.try_consume().is_ok());
        assert!(budget.try_consume().is_err());
        budget.reset();
        assert_eq!(budget.current(), 0);
        assert!(budget.try_consume().is_ok());
    }

    #[tokio::test]
    async fn advisor_tool_execute_errors() {
        let tool = ConsultAdvisorTool;
        let ctx = JobContext::default();
        let result = tool.execute(serde_json::json!({}), &ctx).await;
        assert!(result.is_err());
    }
}
