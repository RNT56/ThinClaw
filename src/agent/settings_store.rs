//! Root database adapter for the extracted agent settings port.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thinclaw_agent::ports::{SettingEntry, SettingsPort};

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

fn setting_entry_from_row(row: crate::history::SettingRow) -> SettingEntry {
    SettingEntry {
        key: row.key,
        value: row.value,
        updated_at: row.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn setting_entry_adapter_preserves_db_fields() {
        let updated_at = Utc::now();
        let entry = setting_entry_from_row(crate::history::SettingRow {
            key: "learning.enabled".to_string(),
            value: serde_json::json!(true),
            updated_at,
        });

        assert_eq!(entry.key, "learning.enabled");
        assert_eq!(entry.value, serde_json::json!(true));
        assert_eq!(entry.updated_at, updated_at);
    }
}
