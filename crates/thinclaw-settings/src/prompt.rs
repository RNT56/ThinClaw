use super::*;

fn default_true() -> bool {
    true
}

fn default_prompt_project_context_max_tokens() -> usize {
    8_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSettings {
    #[serde(default = "default_true")]
    pub session_freeze_enabled: bool,
    #[serde(default = "default_prompt_project_context_max_tokens")]
    pub project_context_max_tokens: usize,
}

impl Default for PromptSettings {
    fn default() -> Self {
        Self {
            session_freeze_enabled: true,
            project_context_max_tokens: default_prompt_project_context_max_tokens(),
        }
    }
}
