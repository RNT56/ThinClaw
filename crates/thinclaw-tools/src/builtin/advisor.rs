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
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use thinclaw_llm::route_planner::{ADVISOR_SYSTEM_PROMPT, AdvisorConfig};
use thinclaw_llm_core::{ChatMessage, CompletionRequest, LlmProvider, Role, TurnAwareness};
use thinclaw_tools_core::{
    Tool, ToolApprovalClass, ToolError, ToolMetadata, ToolOutput, ToolSideEffectLevel,
};
use thinclaw_types::JobContext;
use thinclaw_types::error::LlmError;

/// Name of the advisor tool that executors can call.
pub const ADVISOR_TOOL_NAME: &str = "consult_advisor";
const ADVISOR_MAX_TOKENS: u32 = 1400;
const ADVISOR_THINKING_BUDGET_TOKENS: u32 = 3072;
const ADVISOR_RESPONSE_SCHEMA: &str = r#"Return ONLY a single JSON object with this schema:
{
  "recommendation": "continue" | "revise" | "stop",
  "summary": "short strategic guidance",
  "plan_steps": ["step 1", "step 2"],
  "corrections": ["correction or warning"],
  "stop_reason": "required when recommendation is stop, otherwise null",
  "confidence": 0.0 to 1.0
}"#;

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

    fn metadata(&self) -> ToolMetadata {
        ToolMetadata {
            authoritative_source: true,
            live_data: false,
            side_effect_level: ToolSideEffectLevel::Read,
            approval_class: ToolApprovalClass::Never,
            parallel_safe: false,
            route_intents: Vec::new(),
        }
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

    fn estimated_cost(&self, _params: &serde_json::Value) -> Option<Decimal> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorRecommendation {
    Continue,
    Revise,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorEnvelopeStatus {
    Ok,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvisorConsultationMode {
    Manual,
    Auto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisorDecision {
    pub recommendation: AdvisorRecommendation,
    pub summary: String,
    #[serde(default)]
    pub plan_steps: Vec<String>,
    #[serde(default)]
    pub corrections: Vec<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    pub confidence: f32,
}

impl AdvisorDecision {
    fn normalize(mut self) -> Result<Self, String> {
        self.summary = self.summary.trim().to_string();
        self.plan_steps = self
            .plan_steps
            .into_iter()
            .map(|step| step.trim().to_string())
            .filter(|step| !step.is_empty())
            .collect();
        self.corrections = self
            .corrections
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
        self.stop_reason = self
            .stop_reason
            .map(|reason| reason.trim().to_string())
            .filter(|reason| !reason.is_empty());
        self.confidence = self.confidence.clamp(0.0, 1.0);

        if self.summary.is_empty() {
            return Err("summary must not be empty".to_string());
        }
        if self.recommendation == AdvisorRecommendation::Stop && self.stop_reason.is_none() {
            self.stop_reason = Some(self.summary.clone());
        }

        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvisorConsultationEnvelope {
    pub status: AdvisorEnvelopeStatus,
    pub mode: AdvisorConsultationMode,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advisor_decision: Option<AdvisorDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl AdvisorConsultationEnvelope {
    pub fn ok(
        mode: AdvisorConsultationMode,
        reason: impl Into<String>,
        advisor_decision: AdvisorDecision,
    ) -> Self {
        Self {
            status: AdvisorEnvelopeStatus::Ok,
            mode,
            reason: reason.into(),
            advisor_decision: Some(advisor_decision),
            code: None,
            message: None,
        }
    }

    pub fn error(
        mode: AdvisorConsultationMode,
        reason: impl Into<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status: AdvisorEnvelopeStatus::Error,
            mode,
            reason: reason.into(),
            advisor_decision: None,
            code: Some(code.into()),
            message: Some(message.into()),
        }
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
) -> Result<AdvisorDecision, LlmError> {
    let system_prompt = if config.advisor_system_prompt.is_empty() {
        ADVISOR_SYSTEM_PROMPT.to_string()
    } else {
        config.advisor_system_prompt.clone()
    };
    let prompt = build_advisor_prompt(question, context_summary, conversation_context);
    let decision = request_advisor_decision(advisor_provider, &system_prompt, &prompt).await?;

    tracing::info!(
        advisor_model = %advisor_provider.active_model_name(),
        recommendation = ?decision.recommendation,
        confidence = decision.confidence,
        "Advisor consultation complete"
    );

    Ok(decision)
}

async fn request_advisor_decision(
    advisor_provider: &dyn LlmProvider,
    system_prompt: &str,
    prompt: &str,
) -> Result<AdvisorDecision, LlmError> {
    let mut request = CompletionRequest::new(vec![
        ChatMessage::system(format!(
            "{}\n\n{}",
            system_prompt.trim(),
            ADVISOR_RESPONSE_SCHEMA
        )),
        ChatMessage::user(prompt.to_string()),
    ])
    .with_max_tokens(ADVISOR_MAX_TOKENS)
    .with_temperature(0.1);
    if advisor_supports_thinking(&advisor_provider.active_model_name()) {
        request = request.with_thinking(ADVISOR_THINKING_BUDGET_TOKENS);
    }
    let response = advisor_provider.complete(request).await?;

    match parse_advisor_decision(&response.content, &advisor_provider.active_model_name()) {
        Ok(decision) => Ok(decision),
        Err(parse_error) => {
            tracing::warn!(
                error = %parse_error,
                "Advisor returned invalid JSON; requesting one repair attempt"
            );
            let mut repair_request = CompletionRequest::new(vec![
                ChatMessage::system(format!(
                    "{}\n\n{}",
                    system_prompt.trim(),
                    ADVISOR_RESPONSE_SCHEMA
                )),
                ChatMessage::user(format!(
                    "Repair the previous answer into valid JSON only.\n\nPrevious answer:\n{}\n\nReturn only the corrected JSON object.",
                    response.content
                )),
            ])
            .with_max_tokens(ADVISOR_MAX_TOKENS)
            .with_temperature(0.0);
            if advisor_supports_thinking(&advisor_provider.active_model_name()) {
                repair_request = repair_request.with_thinking(ADVISOR_THINKING_BUDGET_TOKENS);
            }
            let repair_response = advisor_provider.complete(repair_request).await?;

            parse_advisor_decision(
                &repair_response.content,
                &advisor_provider.active_model_name(),
            )
        }
    }
}

fn parse_advisor_decision(content: &str, provider_name: &str) -> Result<AdvisorDecision, LlmError> {
    let json = extract_json_from_text(content).unwrap_or(content);
    let decision: AdvisorDecision =
        serde_json::from_str(json).map_err(|error| LlmError::InvalidResponse {
            provider: provider_name.to_string(),
            reason: format!("advisor decision JSON parse failed: {error}"),
        })?;

    decision
        .normalize()
        .map_err(|error| LlmError::InvalidResponse {
            provider: provider_name.to_string(),
            reason: format!("advisor decision validation failed: {error}"),
        })
}

fn extract_json_from_text(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (start < end).then_some(&text[start..=end])
}

fn advisor_supports_thinking(model_name: &str) -> bool {
    thinclaw_config::model_compat::find_model(model_name)
        .or_else(|| {
            model_name
                .split_once('/')
                .and_then(|(_, model)| thinclaw_config::model_compat::find_model(model))
        })
        .map(|compat| compat.supports_thinking)
        .unwrap_or(false)
}

fn build_advisor_prompt(
    question: &str,
    context_summary: Option<&str>,
    conversation_context: &[ChatMessage],
) -> String {
    let awareness = TurnAwareness::from_messages(conversation_context);
    let mut sections = Vec::new();
    sections.push(section("Escalation Reason", question));

    if let Some(summary) = context_summary
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
    {
        sections.push(section("Executor Summary", summary));
    }

    sections.push(section(
        "Conversation State",
        &awareness.context_snapshot(None),
    ));

    if let Some(last_user_objective) = awareness
        .last_user_objective
        .as_deref()
        .map(|message| trim_text(message, 1_200))
    {
        sections.push(section("Last User Objective", &last_user_objective));
    }

    let earlier_user_context = awareness
        .recent_user_messages
        .iter()
        .rev()
        .skip(1)
        .map(|message| trim_text(message, 700))
        .collect::<Vec<_>>();
    if !earlier_user_context.is_empty() {
        sections.push(section(
            "Recent User Context",
            &earlier_user_context
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n\n"),
        ));
    }

    let assistant_plan = awareness
        .recent_assistant_messages
        .iter()
        .map(|message| trim_text(message, 700))
        .collect::<Vec<_>>();
    if !assistant_plan.is_empty() {
        sections.push(section(
            "Recent Assistant Reasoning",
            &assistant_plan.join("\n\n"),
        ));
    }

    let tool_plans = awareness
        .recent_assistant_tool_plans
        .iter()
        .map(|plan| trim_text(&plan.content, 700))
        .collect::<Vec<_>>();
    if !tool_plans.is_empty() {
        sections.push(section(
            "Recent Planned Tool Actions",
            &tool_plans.join("\n\n"),
        ));
    }

    let recent_failures = awareness
        .recent_tool_outcomes
        .iter()
        .filter(|outcome| outcome.is_error)
        .map(|outcome| format!("- {}: {}", outcome.name, trim_text(&outcome.content, 420)))
        .collect::<Vec<_>>();
    if !recent_failures.is_empty() {
        sections.push(section(
            "Recent Failures Or Warnings",
            &recent_failures.join("\n"),
        ));
    }

    let recent_tool_outcomes = awareness
        .recent_tool_outcomes
        .iter()
        .map(|outcome| format!("- {}: {}", outcome.name, trim_text(&outcome.content, 360)))
        .collect::<Vec<_>>();
    if !recent_tool_outcomes.is_empty() {
        sections.push(section(
            "Recent Tool Outcomes",
            &recent_tool_outcomes.join("\n"),
        ));
    }

    let compact_conversation = conversation_context
        .iter()
        .rev()
        .filter(|message| !message.content.trim().is_empty())
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|message| {
            let role_label = match message.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => message.name.as_deref().unwrap_or("tool"),
            };
            format!(
                "[{}] {}",
                role_label,
                trim_text(message.content.trim(), 320)
            )
        })
        .collect::<Vec<_>>();
    if !compact_conversation.is_empty() {
        sections.push(section(
            "Compact Recent Conversation",
            &compact_conversation.join("\n"),
        ));
    }

    format!(
        "The executor needs strategic guidance. Act like a senior reviewer: prioritize correctness, concrete next actions, recovery from risk, and whether execution should continue, be revised, or stop.\n\n{}",
        sections.join("\n\n")
    )
}

fn section(title: &str, body: &str) -> String {
    format!("## {}\n{}", title, body.trim())
}

fn trim_text(value: &str, max_chars: usize) -> String {
    if value.len() <= max_chars {
        value.to_string()
    } else {
        let end = floor_char_boundary(value, max_chars);
        format!("{}...", &value[..end])
    }
}

fn floor_char_boundary(value: &str, max_bytes: usize) -> usize {
    if max_bytes >= value.len() {
        return value.len();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use rust_decimal::Decimal;

    use thinclaw_llm_core::{
        CompletionResponse, FinishReason, ToolCompletionRequest, ToolCompletionResponse,
    };

    struct SequentialAdvisorProvider {
        responses: Mutex<VecDeque<String>>,
    }

    impl SequentialAdvisorProvider {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(ToOwned::to_owned)
                        .collect::<VecDeque<_>>(),
                ),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for SequentialAdvisorProvider {
        fn model_name(&self) -> &str {
            "advisor-test"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            let content = self
                .responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .expect("missing scripted advisor response");
            Ok(CompletionResponse {
                content,
                provider_model: None,
                cost_usd: None,
                thinking_content: None,
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
                token_capture: None,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            unreachable!("advisor provider should not be asked for tool completions")
        }
    }

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
    fn advisor_tool_metadata_is_read_only_and_not_parallel_safe() {
        let tool = ConsultAdvisorTool;
        let metadata = tool.metadata();
        assert_eq!(metadata.side_effect_level, ToolSideEffectLevel::Read);
        assert_eq!(metadata.approval_class, ToolApprovalClass::Never);
        assert!(!metadata.parallel_safe);
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

    #[tokio::test]
    async fn advisor_consultation_parses_valid_json() {
        let provider = SequentialAdvisorProvider::new(vec![
            r#"{
            "recommendation":"revise",
            "summary":"Tighten the plan before continuing.",
            "plan_steps":["Re-check the routing path","Retry with safer state handling"],
            "corrections":["Do not reuse the failing tool signature"],
            "stop_reason":null,
            "confidence":0.82
        }"#,
        ]);
        let config = AdvisorConfig {
            advisor_target: "primary".to_string(),
            max_advisor_calls: 4,
            advisor_system_prompt: String::new(),
        };

        let decision = execute_advisor_consultation(
            &provider,
            &config,
            "How should I recover?",
            Some("Tool calls are looping."),
            &[ChatMessage::user("Please fix the advisor path.")],
        )
        .await
        .expect("advisor decision");

        assert_eq!(decision.recommendation, AdvisorRecommendation::Revise);
        assert_eq!(decision.plan_steps.len(), 2);
    }

    #[tokio::test]
    async fn advisor_consultation_repairs_invalid_json_once() {
        let provider = SequentialAdvisorProvider::new(vec![
            "not-json-at-all",
            r#"{"recommendation":"continue","summary":"Proceed with the corrected plan.","plan_steps":["Continue carefully"],"corrections":[],"stop_reason":null,"confidence":0.6}"#,
        ]);
        let config = AdvisorConfig {
            advisor_target: "primary".to_string(),
            max_advisor_calls: 4,
            advisor_system_prompt: String::new(),
        };

        let decision = execute_advisor_consultation(
            &provider,
            &config,
            "Need guidance",
            None,
            &[ChatMessage::user("Investigate the bug.")],
        )
        .await
        .expect("repaired advisor decision");

        assert_eq!(decision.recommendation, AdvisorRecommendation::Continue);
    }

    #[tokio::test]
    async fn advisor_consultation_returns_structured_error_after_failed_repair() {
        let provider = SequentialAdvisorProvider::new(vec!["oops", "still not json"]);
        let config = AdvisorConfig {
            advisor_target: "primary".to_string(),
            max_advisor_calls: 4,
            advisor_system_prompt: String::new(),
        };

        let error = execute_advisor_consultation(
            &provider,
            &config,
            "Need guidance",
            None,
            &[ChatMessage::user("Investigate the bug.")],
        )
        .await
        .expect_err("advisor parse should fail");

        assert!(matches!(error, LlmError::InvalidResponse { .. }));
    }

    #[test]
    fn advisor_prompt_uses_richer_context_sections() {
        let prompt = build_advisor_prompt(
            "How should I recover?",
            Some("The executor is trying to finish a complex review."),
            &[
                ChatMessage::user(
                    "Please design the migration architecture and review the implementation risks.",
                ),
                ChatMessage::assistant_with_tool_calls(
                    Some("I should inspect the existing implementation first.".to_string()),
                    vec![
                        thinclaw_llm_core::ToolCall {
                            id: "call_1".to_string(),
                            name: "read_file".to_string(),
                            arguments: serde_json::json!({"path":"src/main.rs"}),
                        },
                        thinclaw_llm_core::ToolCall {
                            id: "call_2".to_string(),
                            name: "search_code".to_string(),
                            arguments: serde_json::json!({"query":"migration"}),
                        },
                    ],
                ),
                ChatMessage::tool_result(
                    "call_1",
                    "read_file",
                    "{\"status\":\"error\",\"message\":\"config missing\"}",
                ),
            ],
        );

        assert!(prompt.contains("Conversation State"));
        assert!(prompt.contains("Recent Planned Tool Actions"));
        assert!(prompt.contains("Recent Failures Or Warnings"));
        assert!(prompt.contains("Compact Recent Conversation"));
    }

    #[test]
    fn advisor_supports_thinking_for_known_frontier_models() {
        assert!(advisor_supports_thinking("gpt-5.4"));
        assert!(advisor_supports_thinking("openai/gpt-5.4"));
        assert!(!advisor_supports_thinking("unknown-model"));
    }
}
