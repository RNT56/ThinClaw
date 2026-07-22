use super::*;

fn default_true() -> bool {
    true
}

fn default_prompt_project_context_max_tokens() -> usize {
    8_000
}

fn default_prompt_context_window_tokens() -> usize {
    32_000
}

fn default_prompt_total_tokens() -> usize {
    16_000
}

fn default_prompt_output_reserve_tokens() -> usize {
    4_096
}

fn default_prompt_safety_margin_percent() -> u8 {
    10
}

fn default_prompt_contract_version() -> String {
    "v2".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptRolloutMode {
    Legacy,
    Shadow,
    #[default]
    V2,
}

impl PromptRolloutMode {
    pub fn effective_for_session(
        self,
        freeze_enabled: bool,
        frozen_contract: Option<&str>,
    ) -> Self {
        if !freeze_enabled {
            return self;
        }
        match frozen_contract {
            Some("v2") => Self::V2,
            Some("legacy") => Self::Legacy,
            _ => self,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptSettings {
    #[serde(default = "default_true")]
    pub session_freeze_enabled: bool,
    #[serde(default = "default_prompt_project_context_max_tokens")]
    pub project_context_max_tokens: usize,
    #[serde(default)]
    pub rollout_mode: PromptRolloutMode,
    #[serde(default = "default_prompt_contract_version")]
    pub contract_version: String,
    #[serde(default = "default_prompt_context_window_tokens")]
    pub context_window_tokens: usize,
    #[serde(default = "default_prompt_total_tokens")]
    pub max_total_tokens: usize,
    #[serde(default = "default_prompt_output_reserve_tokens")]
    pub output_reserve_tokens: usize,
    #[serde(default = "default_prompt_safety_margin_percent")]
    pub safety_margin_percent: u8,
}

impl Default for PromptSettings {
    fn default() -> Self {
        Self {
            session_freeze_enabled: true,
            project_context_max_tokens: default_prompt_project_context_max_tokens(),
            rollout_mode: PromptRolloutMode::V2,
            contract_version: default_prompt_contract_version(),
            context_window_tokens: default_prompt_context_window_tokens(),
            max_total_tokens: default_prompt_total_tokens(),
            output_reserve_tokens: default_prompt_output_reserve_tokens(),
            safety_margin_percent: default_prompt_safety_margin_percent(),
        }
    }
}

impl PromptSettings {
    /// Constrain persisted/database values before they participate in prompt
    /// allocation or token arithmetic. Settings are operator-editable and a
    /// malformed value must degrade predictably rather than eliminating the
    /// entire prompt budget or requesting enormous buffers.
    pub fn normalize_runtime_bounds(&mut self) {
        self.project_context_max_tokens = self.project_context_max_tokens.clamp(128, 64_000);
        self.context_window_tokens = self.context_window_tokens.clamp(1_024, 2_000_000);
        self.max_total_tokens = self.max_total_tokens.clamp(128, 256_000);
        self.output_reserve_tokens = self.output_reserve_tokens.clamp(64, 128_000);
        self.safety_margin_percent = self.safety_margin_percent.min(25);
        if self.contract_version.is_empty()
            || self.contract_version.len() > 64
            || self.contract_version.chars().any(char::is_control)
        {
            self.contract_version = default_prompt_contract_version();
        }
    }

    /// Adapt token reservations to the model actually selected for this turn.
    pub fn normalize_for_context_window(&mut self, context_window_tokens: usize) {
        self.normalize_runtime_bounds();
        let context_window_tokens = context_window_tokens.max(256);
        self.output_reserve_tokens = self
            .output_reserve_tokens
            .min(context_window_tokens.saturating_sub(128))
            .min(context_window_tokens / 2);
        self.max_total_tokens = self
            .max_total_tokens
            .min(context_window_tokens.saturating_sub(self.output_reserve_tokens));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_deserialize_with_production_v2_defaults() {
        let settings: PromptSettings = serde_json::from_str(
            r#"{"session_freeze_enabled":true,"project_context_max_tokens":8000}"#,
        )
        .unwrap();
        assert_eq!(settings.rollout_mode, PromptRolloutMode::V2);
        assert_eq!(settings.contract_version, "v2");
        assert_eq!(settings.max_total_tokens, 16_000);
    }

    #[test]
    fn session_contract_changes_only_at_a_session_boundary() {
        assert_eq!(
            PromptRolloutMode::V2.effective_for_session(true, Some("legacy")),
            PromptRolloutMode::Legacy
        );
        assert_eq!(
            PromptRolloutMode::Legacy.effective_for_session(true, Some("v2")),
            PromptRolloutMode::V2
        );
        assert_eq!(
            PromptRolloutMode::V2.effective_for_session(false, Some("legacy")),
            PromptRolloutMode::V2
        );
    }

    #[test]
    fn malformed_prompt_limits_are_normalized_for_runtime() {
        let mut settings = PromptSettings {
            project_context_max_tokens: usize::MAX,
            context_window_tokens: 0,
            max_total_tokens: usize::MAX,
            output_reserve_tokens: usize::MAX,
            safety_margin_percent: u8::MAX,
            contract_version: "\0".to_string(),
            ..PromptSettings::default()
        };

        settings.normalize_for_context_window(4_096);

        assert_eq!(settings.project_context_max_tokens, 64_000);
        assert_eq!(settings.context_window_tokens, 1_024);
        assert_eq!(settings.output_reserve_tokens, 2_048);
        assert_eq!(settings.max_total_tokens, 2_048);
        assert_eq!(settings.safety_margin_percent, 25);
        assert_eq!(settings.contract_version, "v2");
    }
}
