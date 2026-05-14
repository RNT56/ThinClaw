//! Compatibility adapter for extracted desktop autonomy tools.

use async_trait::async_trait;

use crate::desktop_autonomy::DesktopAutonomyManager;

pub use thinclaw_tools::builtin::desktop_autonomy::{DesktopAutonomyPort, DesktopAutonomyTool};

#[async_trait]
impl DesktopAutonomyPort for DesktopAutonomyManager {
    async fn apps_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        DesktopAutonomyManager::apps_action(self, action, params).await
    }

    async fn ui_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        DesktopAutonomyManager::ui_action(self, action, params).await
    }

    async fn screen_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        DesktopAutonomyManager::screen_action(self, action, params).await
    }

    async fn calendar_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        DesktopAutonomyManager::calendar_action(self, action, params).await
    }

    async fn numbers_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        DesktopAutonomyManager::numbers_action(self, action, params).await
    }

    async fn pages_action(
        &self,
        action: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        DesktopAutonomyManager::pages_action(self, action, params).await
    }

    async fn status(&self) -> Result<serde_json::Value, String> {
        serde_json::to_value(DesktopAutonomyManager::status(self).await)
            .map_err(|error| format!("failed to serialize autonomy status: {error}"))
    }

    async fn pause(&self, reason: Option<String>) {
        DesktopAutonomyManager::pause(self, reason).await;
    }

    async fn resume(&self) -> Result<(), String> {
        DesktopAutonomyManager::resume(self).await
    }

    async fn bootstrap(&self) -> Result<serde_json::Value, String> {
        serde_json::to_value(DesktopAutonomyManager::bootstrap(self).await?)
            .map_err(|error| format!("failed to serialize bootstrap report: {error}"))
    }

    async fn desktop_permission_status(&self) -> Result<serde_json::Value, String> {
        DesktopAutonomyManager::desktop_permission_status(self).await
    }

    async fn rollback(&self) -> Result<serde_json::Value, String> {
        DesktopAutonomyManager::rollback(self).await
    }

    fn desktop_action_timeout_secs(&self) -> u64 {
        self.config().desktop_action_timeout_secs
    }
}
