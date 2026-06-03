//! Conversion policy between settings-store ports and history records.

use thinclaw_history::SettingRow;

use crate::ports::SettingEntry;

pub fn setting_entry_from_row(row: SettingRow) -> SettingEntry {
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
        let entry = setting_entry_from_row(SettingRow {
            key: "learning.enabled".to_string(),
            value: serde_json::json!(true),
            updated_at,
        });

        assert_eq!(entry.key, "learning.enabled");
        assert_eq!(entry.value, serde_json::json!(true));
        assert_eq!(entry.updated_at, updated_at);
    }
}
