//! Clarify tool for structured clarifying questions.
//!
//! Provides a structured way for the agent to ask the user multiple-choice
//! or open-ended questions. Channels can render these appropriately
//! (inline keyboard buttons on Telegram, block kit on Slack, etc.).

use std::time::Duration;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

/// A structured clarifying question.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClarifyQuestion {
    /// The question text.
    pub question: String,
    /// Optional choices (for multiple-choice questions).
    pub options: Option<Vec<ClarifyOption>>,
    /// Whether free-form text input is accepted even with options.
    pub allow_freeform: bool,
}

/// A single option in a clarifying question.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ClarifyOption {
    /// Display label.
    pub label: String,
    /// Machine value.
    pub value: String,
}

/// Structured clarifying question tool.
pub struct ClarifyTool;

#[async_trait]
impl Tool for ClarifyTool {
    fn name(&self) -> &str {
        "clarify"
    }

    fn description(&self) -> &str {
        "Ask the user a structured clarifying question. Optionally provide multiple-choice \
         options for easy selection. Use when you need specific information before proceeding. \
         The question will be formatted appropriately for the user's platform \
         (buttons on Telegram/Discord, numbered list in CLI, etc.)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "options": {
                    "type": "array",
                    "description": "Optional choices for multiple-choice questions",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": {
                                "type": "string",
                                "description": "Display text for this option"
                            },
                            "value": {
                                "type": "string",
                                "description": "Machine value for this option (returned when selected)"
                            }
                        },
                        "required": ["label", "value"]
                    }
                },
                "allow_freeform": {
                    "type": "boolean",
                    "description": "If true (default), accept free-text answers even when options are provided"
                }
            },
            "required": ["question"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let question = require_str(&params, "question")?;
        let allow_freeform = params
            .get("allow_freeform")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let options: Option<Vec<ClarifyOption>> = params
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let label = item.get("label")?.as_str()?;
                        let value = item.get("value")?.as_str()?;
                        Some(ClarifyOption {
                            label: label.to_string(),
                            value: value.to_string(),
                        })
                    })
                    .collect()
            });

        // Build the formatted question for text-based channels
        let mut formatted = String::from(question);

        if let Some(ref opts) = options {
            formatted.push_str("\n\n");
            for (i, opt) in opts.iter().enumerate() {
                formatted.push_str(&format!("{}. {}\n", i + 1, opt.label));
            }
            if allow_freeform {
                formatted.push_str("\n(You can also type a custom answer)");
            }
        }

        let clarify_question = ClarifyQuestion {
            question: question.to_string(),
            options: options.clone(),
            allow_freeform,
        };

        let result = serde_json::json!({
            "question": clarify_question.question,
            "options": options.as_ref().map(|opts| {
                opts.iter().map(|o| serde_json::json!({
                    "label": o.label,
                    "value": o.value,
                })).collect::<Vec<_>>()
            }),
            "allow_freeform": allow_freeform,
            "formatted": formatted,
            "type": "clarify_question",
        });

        Ok(ToolOutput::success(result, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        false // Agent-generated content
    }

    fn execution_timeout(&self) -> Duration {
        Duration::from_secs(5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_clarify_simple_question() {
        let tool = ClarifyTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"question": "What language do you prefer?"}),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(
            result.result.get("question").unwrap().as_str().unwrap(),
            "What language do you prefer?"
        );
        assert!(result.result.get("options").unwrap().is_null());
        assert!(result.result.get("allow_freeform").unwrap().as_bool().unwrap());
    }

    #[tokio::test]
    async fn test_clarify_with_options() {
        let tool = ClarifyTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "question": "Which framework?",
                    "options": [
                        {"label": "React", "value": "react"},
                        {"label": "Vue", "value": "vue"},
                        {"label": "Svelte", "value": "svelte"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();

        let options = result.result.get("options").unwrap().as_array().unwrap();
        assert_eq!(options.len(), 3);
        assert_eq!(options[0]["label"], "React");
    }

    #[tokio::test]
    async fn test_clarify_no_freeform() {
        let tool = ClarifyTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "question": "Yes or no?",
                    "options": [
                        {"label": "Yes", "value": "yes"},
                        {"label": "No", "value": "no"}
                    ],
                    "allow_freeform": false
                }),
                &ctx,
            )
            .await
            .unwrap();

        let freeform = result.result.get("allow_freeform").unwrap().as_bool().unwrap();
        assert!(!freeform);

        let formatted = result.result.get("formatted").unwrap().as_str().unwrap();
        assert!(!formatted.contains("custom answer"));
    }

    #[tokio::test]
    async fn test_clarify_formatted_output() {
        let tool = ClarifyTool;
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({
                    "question": "Pick one:",
                    "options": [
                        {"label": "A", "value": "a"},
                        {"label": "B", "value": "b"}
                    ]
                }),
                &ctx,
            )
            .await
            .unwrap();

        let formatted = result.result.get("formatted").unwrap().as_str().unwrap();
        assert!(formatted.contains("1. A"));
        assert!(formatted.contains("2. B"));
    }

    #[tokio::test]
    async fn test_clarify_missing_question() {
        let tool = ClarifyTool;
        let ctx = JobContext::default();

        let err = tool
            .execute(serde_json::json!({}), &ctx)
            .await
            .unwrap_err();

        assert!(err.to_string().contains("question"));
    }
}
