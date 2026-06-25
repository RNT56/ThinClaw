//! Readiness summary, validation item rendering, and follow-up draft
//! bookkeeping used by the completion surfaces.

use super::{FollowupDraft, ReadinessSummary, SetupWizard, ValidationItem, ValidationLevel};

impl SetupWizard {
    pub(super) fn readiness_summary(&self) -> ReadinessSummary {
        let ready_now = self
            .step_statuses
            .values()
            .filter(|status| matches!(status, super::StepStatus::Completed))
            .count();
        let needs_attention = self
            .step_statuses
            .values()
            .filter(|status| matches!(status, super::StepStatus::NeedsAttention))
            .count();
        let followups = self.followups.len();

        let headline = if followups == 0 && needs_attention == 0 {
            "Launch-ready".to_string()
        } else if followups > 0 || needs_attention > 0 {
            "Attention queued".to_string()
        } else {
            "Bringing systems online".to_string()
        };

        ReadinessSummary {
            ready_now,
            needs_attention,
            followups,
            headline,
        }
    }

    pub(super) fn validation_items(&self) -> Vec<ValidationItem> {
        let mut items = Vec::new();

        if self.settings.database_backend.is_some() {
            items.push(ValidationItem {
                level: ValidationLevel::Info,
                title: "Core runtime".to_string(),
                detail: self
                    .settings
                    .database_backend
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
            });
        } else {
            items.push(ValidationItem {
                level: ValidationLevel::Error,
                title: "Core runtime".to_string(),
                detail: "Storage still needs to be configured before ThinClaw can fully launch."
                    .to_string(),
            });
        }

        if self.settings.llm_backend.is_some() && self.settings.selected_model.is_some() {
            items.push(ValidationItem {
                level: ValidationLevel::Info,
                title: "AI stack".to_string(),
                detail: format!(
                    "{} / {}",
                    self.settings.llm_backend.as_deref().unwrap_or("unknown"),
                    self.settings
                        .selected_model
                        .as_deref()
                        .unwrap_or("unselected")
                ),
            });
        } else {
            items.push(ValidationItem {
                level: ValidationLevel::Error,
                title: "AI stack".to_string(),
                detail:
                    "Primary provider or model still needs review before the agent is fully ready."
                        .to_string(),
            });
        }

        let enabled_channels = self.configured_channel_names();
        if enabled_channels.is_empty() {
            items.push(ValidationItem {
                level: ValidationLevel::Warning,
                title: "Channels".to_string(),
                detail: "Only the built-in terminal path is confirmed right now.".to_string(),
            });
        } else {
            items.push(ValidationItem {
                level: ValidationLevel::Info,
                title: "Channels".to_string(),
                detail: enabled_channels.join(", "),
            });
        }

        if !self.followups.is_empty() {
            items.push(ValidationItem {
                level: ValidationLevel::Warning,
                title: "Follow-ups".to_string(),
                detail: format!(
                    "{} follow-up item(s) are queued for after launch.",
                    self.followups.len()
                ),
            });
        }

        items
    }

    pub(super) fn add_followup(&mut self, draft: FollowupDraft) {
        if let Some(existing) = self.followups.iter_mut().find(|item| item.id == draft.id) {
            *existing = draft;
        } else {
            self.followups.push(draft);
        }
    }

    pub(super) fn remove_followup(&mut self, id: &str) {
        self.followups.retain(|item| item.id != id);
    }
}
