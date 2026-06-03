//! Root database adapter for the extracted agent settings port.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{SettingEntry, SettingsPort};
use thinclaw_agent::settings_records::setting_entry_from_row;

use crate::db::Database;
use crate::error::DatabaseError;

pub struct RootSettingsPort {
    store: Arc<dyn Database>,
}

impl RootSettingsPort {
    pub fn shared(store: Arc<dyn Database>) -> Arc<dyn SettingsPort> {
        Arc::new(Self { store })
    }
}

#[async_trait]
impl SettingsPort for RootSettingsPort {
    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingEntry>, DatabaseError> {
        let rows = self.store.list_settings(user_id).await?;
        Ok(rows.into_iter().map(setting_entry_from_row).collect())
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.store.get_all_settings(user_id).await
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.store.set_all_settings(user_id, settings).await
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        self.store.has_settings(user_id).await
    }
}
