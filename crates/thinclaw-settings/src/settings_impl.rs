use super::*;

impl Settings {
    /// Reconstruct Settings from a flat key-value map (as stored in the DB).
    ///
    /// Each key is a dotted path (e.g., "agent.name"), value is a JSONB value.
    /// Missing keys get their default value.
    pub fn from_db_map(map: &std::collections::HashMap<String, serde_json::Value>) -> Self {
        // Reconstruct the full nested JSON tree from flattened DB key-value
        // pairs, then deserialize all at once.
        //
        // The previous approach called `set()` per-key, which silently failed
        // for HashMap-based fields like `provider_models` because `set()`
        // cannot create intermediate map keys that don't exist in the default
        // struct.  By rebuilding the tree first, all nested structures —
        // including dynamic HashMap entries — roundtrip correctly.
        let mut tree = serde_json::to_value(Self::default())
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

        for (key, value) in map {
            if matches!(value, serde_json::Value::Null) {
                continue; // null means default, skip
            }
            insert_dotted_path(&mut tree, key, value.clone());
        }

        match serde_json::from_value::<Self>(tree.clone()) {
            Ok(settings) => settings,
            Err(e) => {
                tracing::warn!(
                    "from_db_map full-tree deserialize failed, falling back to per-key set(): {}",
                    e
                );
                // Fall back to the legacy per-key approach so we don't lose
                // everything on a single bad key.
                let mut settings = Self::default();
                for (key, value) in map {
                    let value_str = match value {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Bool(b) => b.to_string(),
                        serde_json::Value::Number(n) => n.to_string(),
                        serde_json::Value::Null => continue,
                        other => other.to_string(),
                    };
                    match settings.set(key, &value_str) {
                        Ok(()) => {}
                        Err(e) if e.starts_with("Path not found") => {}
                        Err(e) => {
                            tracing::warn!(
                                "Failed to apply DB setting '{}' = '{}': {}",
                                key,
                                value_str,
                                e
                            );
                        }
                    }
                }
                settings
            }
        }
    }

    /// Flatten Settings into a key-value map suitable for DB storage.
    ///
    /// Each entry is a (dotted_path, JSONB value) pair.
    pub fn to_db_map(&self) -> std::collections::HashMap<String, serde_json::Value> {
        let json = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(_) => return std::collections::HashMap::new(),
        };

        let mut map = std::collections::HashMap::new();
        collect_settings_json(&json, String::new(), &mut map);
        map
    }

    /// Get the default settings file path (~/.thinclaw/settings.json).
    pub fn default_path() -> std::path::PathBuf {
        thinclaw_platform::state_paths().settings_file
    }

    /// Load settings from disk, returning default if not found.
    pub fn load() -> Self {
        Self::load_from(&Self::default_path())
    }

    /// Load settings from a specific path (used by bootstrap legacy migration).
    pub fn load_from(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(data) => serde_json::from_str(&data).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Default TOML config file path (~/.thinclaw/config.toml).
    pub fn default_toml_path() -> PathBuf {
        thinclaw_platform::state_paths().config_file
    }

    /// Load settings from a TOML file.
    ///
    /// Returns `None` if the file doesn't exist. Returns an error only
    /// if the file exists but can't be parsed.
    pub fn load_toml(path: &std::path::Path) -> Result<Option<Self>, String> {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("failed to read {}: {}", path.display(), e)),
        };

        let settings: Self = toml::from_str(&data)
            .map_err(|e| format!("invalid TOML in {}: {}", path.display(), e))?;
        Ok(Some(settings))
    }

    /// Write a well-commented TOML config file with current settings.
    pub fn save_toml(&self, path: &std::path::Path) -> Result<(), String> {
        let raw = toml::to_string_pretty(self)
            .map_err(|e| format!("failed to serialize settings: {}", e))?;

        let content = format!(
            "# ThinClaw configuration file.\n\
             #\n\
             # Priority: env var > this file > database settings > defaults.\n\
             # Uncomment and edit values to override defaults.\n\
             # Run `thinclaw config init` to regenerate this file.\n\
             #\n\
             # Documentation: https://github.com/RNT56/thinclaw\n\
             \n\
             {raw}"
        );

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create {}: {}", parent.display(), e))?;
        }

        std::fs::write(path, content)
            .map_err(|e| format!("failed to write {}: {}", path.display(), e))
    }

    /// Merge values from `other` into `self`, preferring `other` for
    /// fields that differ from the default.
    ///
    /// This enables layering: load DB/JSON settings as the base, then
    /// overlay TOML values on top. Only fields that the TOML file
    /// explicitly changed (i.e. differ from Default) are applied.
    pub fn merge_from(&mut self, other: &Self) {
        let default_json = match serde_json::to_value(Self::default()) {
            Ok(v) => v,
            Err(_) => return,
        };
        let other_json = match serde_json::to_value(other) {
            Ok(v) => v,
            Err(_) => return,
        };
        let mut self_json = match serde_json::to_value(&*self) {
            Ok(v) => v,
            Err(_) => return,
        };

        merge_non_default(&mut self_json, &other_json, &default_json);

        if let Ok(merged) = serde_json::from_value(self_json) {
            *self = merged;
        }
    }

    /// Get a setting value by dotted path (e.g., "agent.max_parallel_jobs").
    pub fn get(&self, path: &str) -> Option<String> {
        let json = serde_json::to_value(self).ok()?;
        let mut current = &json;

        for part in path.split('.') {
            current = current.get(part)?;
        }

        match current {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            serde_json::Value::Bool(b) => Some(b.to_string()),
            serde_json::Value::Null => Some("null".to_string()),
            serde_json::Value::Array(arr) => Some(serde_json::to_string(arr).unwrap_or_default()),
            serde_json::Value::Object(obj) => Some(serde_json::to_string(obj).unwrap_or_default()),
        }
    }

    /// Set a setting value by dotted path.
    ///
    /// Returns error if path is invalid or value cannot be parsed.
    pub fn set(&mut self, path: &str, value: &str) -> Result<(), String> {
        let mut json = serde_json::to_value(&self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        let parts: Vec<&str> = path.split('.').collect();
        if parts.is_empty() {
            return Err("Empty path".to_string());
        }

        // Navigate to parent and set the final key
        let mut current = &mut json;
        for part in &parts[..parts.len() - 1] {
            current = current
                .get_mut(*part)
                .ok_or_else(|| format!("Path not found: {}", path))?;
        }

        let final_key = parts.last().expect("parts is non-empty after split");
        let obj = current
            .as_object_mut()
            .ok_or_else(|| format!("Parent is not an object: {}", path))?;

        // Try to infer the type from the existing value
        let new_value = if let Some(existing) = obj.get(*final_key) {
            match existing {
                serde_json::Value::Bool(_) => {
                    let b = value
                        .parse::<bool>()
                        .map_err(|_| format!("Expected boolean for {}, got '{}'", path, value))?;
                    serde_json::Value::Bool(b)
                }
                serde_json::Value::Number(n) => {
                    if n.is_u64() {
                        let n = value.parse::<u64>().map_err(|_| {
                            format!("Expected integer for {}, got '{}'", path, value)
                        })?;
                        serde_json::Value::Number(n.into())
                    } else if n.is_i64() {
                        let n = value.parse::<i64>().map_err(|_| {
                            format!("Expected integer for {}, got '{}'", path, value)
                        })?;
                        serde_json::Value::Number(n.into())
                    } else {
                        let n = value.parse::<f64>().map_err(|_| {
                            format!("Expected number for {}, got '{}'", path, value)
                        })?;
                        serde_json::Number::from_f64(n)
                            .map(serde_json::Value::Number)
                            .unwrap_or(serde_json::Value::String(value.to_string()))
                    }
                }
                serde_json::Value::Null => {
                    // Could be Option<T>. Parse as JSON to infer the value type.
                    //
                    // Pitfall: numeric-looking strings like "684480568" parse as
                    // serde_json::Value::Number. This works for Option<i64> fields
                    // (e.g. telegram_owner_id) but breaks Option<String> fields
                    // (e.g. notifications.recipient) since serde won't coerce
                    // Number → String.
                    //
                    // Solution: try inserting the parsed value and deserializing
                    // the whole Settings. If that fails, fall back to String.
                    serde_json::from_str(value)
                        .unwrap_or(serde_json::Value::String(value.to_string()))
                }
                serde_json::Value::Array(_) => {
                    // Try to parse as JSON array first; if that fails, try
                    // comma-separated string (e.g. "openai/gpt-4o,groq/llama" from
                    // the WebUI text input) and convert it into a JSON array.
                    serde_json::from_str(value).unwrap_or_else(|_| {
                        let items: Vec<serde_json::Value> = value
                            .split(',')
                            .map(|s| s.trim())
                            .filter(|s| !s.is_empty())
                            .map(|s| serde_json::Value::String(s.to_string()))
                            .collect();
                        serde_json::Value::Array(items)
                    })
                }
                serde_json::Value::Object(_) => serde_json::from_str(value)
                    .map_err(|e| format!("Invalid JSON object for {}: {}", path, e))?,
                serde_json::Value::String(_) => serde_json::Value::String(value.to_string()),
            }
        } else {
            // Key doesn't exist, try to parse as JSON or use string
            serde_json::from_str(value).unwrap_or(serde_json::Value::String(value.to_string()))
        };

        obj.insert((*final_key).to_string(), new_value.clone());

        // Deserialize back to Settings.
        // If this fails and the value was inserted into a Null field as a Number
        // (e.g. "684480568" into an Option<String>), retry with String.
        match serde_json::from_value(json.clone()) {
            Ok(s) => {
                *self = s;
            }
            Err(e) => {
                if matches!(new_value, serde_json::Value::Number(_)) {
                    // Retry: the field is likely Option<String>, not Option<i64>.
                    // Re-navigate to the parent and insert as String instead.
                    let mut cur = &mut json;
                    for part in &parts[..parts.len() - 1] {
                        cur = cur.get_mut(*part).expect("path already validated");
                    }
                    cur.as_object_mut().expect("parent is object").insert(
                        (*final_key).to_string(),
                        serde_json::Value::String(value.to_string()),
                    );
                    *self = serde_json::from_value(json).map_err(|e2| {
                        format!(
                            "Failed to apply setting: {} (also tried as string: {})",
                            e, e2
                        )
                    })?;
                } else {
                    return Err(format!("Failed to apply setting: {}", e));
                }
            }
        }

        Ok(())
    }

    /// Reset a setting to its default value.
    pub fn reset(&mut self, path: &str) -> Result<(), String> {
        let default = Self::default();
        let default_value = default
            .get(path)
            .ok_or_else(|| format!("Unknown setting: {}", path))?;

        self.set(path, &default_value)
    }

    /// List all settings as (path, value) pairs.
    pub fn list(&self) -> Vec<(String, String)> {
        let json = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };

        let mut results = Vec::new();
        collect_settings(&json, String::new(), &mut results);
        results.sort_by(|a, b| a.0.cmp(&b.0));
        results
    }
}
