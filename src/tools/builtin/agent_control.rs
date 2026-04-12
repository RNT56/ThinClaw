//! Agent control tools for autonomous reasoning and user communication.
//!
//! These tools give the agent the ability to:
//! - Think internally without showing output to the user (`agent_think`)
//! - Send visible progress messages to the user without terminating the loop (`emit_user_message`)
//!
//! Both are tool calls, so the agentic loop naturally continues after execution.
//! This is the key difference from a regular text response, which terminates the loop.

use std::time::Instant;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// Internal reasoning / scratchpad tool.
///
/// Allows the agent to "think out loud" — organizing its reasoning, planning
/// next steps, or reflecting on tool results — without showing anything to
/// the user. The thought content is preserved in the conversation context
/// (as a tool call + result) so the LLM remembers its reasoning in
/// subsequent iterations, but it is never surfaced to the user.
///
/// This is especially useful for models that don't have native extended
/// thinking (chain-of-thought) support.
pub struct AgentThinkTool;

#[async_trait]
impl Tool for AgentThinkTool {
    fn name(&self) -> &str {
        "agent_think"
    }

    fn description(&self) -> &str {
        "Internal reasoning scratchpad. Use this to think through a problem, plan your \
         next steps, decide whether to continue working or respond to the user, or \
         reflect on tool results before acting. Your thought is NOT shown to the user \
         but IS remembered in this conversation. Use this when you need to: \
         (1) plan a multi-step approach, (2) evaluate whether you're done, \
         (3) decide which tool to use next, or (4) reason about complex information."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "thought": {
                    "type": "string",
                    "description": "Your internal reasoning. Be specific about what you're thinking, \
                                    what you've learned, and what you plan to do next."
                }
            },
            "required": ["thought"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let thought = require_str(&params, "thought")?;

        if thought.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "thought cannot be empty".to_string(),
            ));
        }

        // The thought is preserved in context as the tool call arguments.
        // Return a minimal acknowledgment — the LLM sees this as confirmation
        // that its thinking was recorded and can now proceed.
        Ok(ToolOutput::success(
            serde_json::json!({
                "status": "thought_recorded",
                "length": thought.len(),
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool, trusted content
    }
}

/// User-facing progress message tool.
///
/// Allows the agent to send a visible message to the user without
/// terminating the agentic loop. This is how the agent keeps the user
/// informed during long-running multi-step tasks.
///
/// The dispatcher intercepts this tool's output and forwards the message
/// to the user's channel (Scrappy, Telegram, CLI, etc.) as a real message.
/// The loop then continues with the next iteration.
///
/// Contrast with a regular text response, which would end the loop entirely.
pub struct EmitUserMessageTool;

#[async_trait]
impl Tool for EmitUserMessageTool {
    fn name(&self) -> &str {
        "emit_user_message"
    }

    fn description(&self) -> &str {
        "Send a visible progress message to the user WITHOUT ending your work. \
         Use this for meaningful checkpoints while you continue working: share major milestones, \
         interim results, blockers, or ask for clarification without stopping the loop. \
         Avoid narrating every routine tool call or micro-step unless the user explicitly wants detailed progress. \
         After calling this, you will continue executing — your loop does NOT stop. \
         Only use a regular text response when you are DONE with your work."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "The message to show to the user. Use markdown formatting. \
                                    Keep it concise and milestone-oriented — this is a checkpoint, not a final answer."
                },
                "message_type": {
                    "type": "string",
                    "enum": ["progress", "interim_result", "question", "warning"],
                    "description": "Type of message: 'progress' for status updates, \
                                    'interim_result' for partial findings the user should see, \
                                    'question' for asking the user something while continuing, \
                                    'warning' for blockers or issues that need attention.",
                    "default": "progress"
                }
            },
            "required": ["message"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();

        let message = require_str(&params, "message")?;

        if message.trim().is_empty() {
            return Err(ToolError::InvalidParameters(
                "message cannot be empty".to_string(),
            ));
        }

        let message_type = params
            .get("message_type")
            .and_then(|v| v.as_str())
            .unwrap_or("progress");

        // The actual message delivery is handled by the dispatcher, which
        // intercepts tool results from "emit_user_message" and forwards
        // the message to the user's channel.
        //
        // We return the message content and type so the dispatcher can
        // extract it without re-parsing the parameters.
        Ok(ToolOutput::success(
            serde_json::json!({
                "status": "message_sent",
                "message": message,
                "message_type": message_type,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false // Internal tool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_think_records_thought() {
        let tool = AgentThinkTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"thought": "I need to check the user's timezone first"}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.result["status"], "thought_recorded");
    }

    #[tokio::test]
    async fn test_agent_think_rejects_empty() {
        let tool = AgentThinkTool;
        let ctx = JobContext::default();

        let err = tool
            .execute(serde_json::json!({"thought": "   "}), &ctx)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_agent_think_schema() {
        let tool = AgentThinkTool;
        assert_eq!(tool.name(), "agent_think");
        assert!(!tool.requires_sanitization());

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["thought"].is_object());
        assert!(
            schema["required"]
                .as_array()
                .unwrap()
                .contains(&"thought".into())
        );
    }

    #[tokio::test]
    async fn test_emit_user_message_sends() {
        let tool = EmitUserMessageTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "message": "Working on step 2 of 5...",
                    "message_type": "progress"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(result.result["status"], "message_sent");
        assert_eq!(result.result["message"], "Working on step 2 of 5...");
        assert_eq!(result.result["message_type"], "progress");
    }

    #[tokio::test]
    async fn test_emit_user_message_default_type() {
        let tool = EmitUserMessageTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"message": "Still working..."}), &ctx)
            .await
            .unwrap();

        assert_eq!(result.result["message_type"], "progress");
    }

    #[tokio::test]
    async fn test_emit_user_message_rejects_empty() {
        let tool = EmitUserMessageTool;
        let ctx = JobContext::default();

        let err = tool
            .execute(serde_json::json!({"message": ""}), &ctx)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_emit_user_message_schema() {
        let tool = EmitUserMessageTool;
        assert_eq!(tool.name(), "emit_user_message");

        let schema = tool.parameters_schema();
        assert!(schema["properties"]["message"].is_object());
        assert!(schema["properties"]["message_type"].is_object());
    }

    #[test]
    fn test_emit_user_message_description_mentions_checkpoint_usage() {
        let tool = EmitUserMessageTool;

        assert!(tool.description().contains("meaningful checkpoints"));
        assert!(
            tool.description()
                .contains("Avoid narrating every routine tool call")
        );
    }
}
