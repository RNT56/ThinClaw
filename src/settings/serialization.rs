pub(super) fn insert_dotted_path(
    root: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }

    let mut current = root;
    for part in &parts[..parts.len() - 1] {
        // Navigate into (or create) intermediate objects.
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        current = current
            .as_object_mut()
            .expect("just ensured object")
            .entry(*part)
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    }

    if let Some(final_key) = parts.last() {
        if !current.is_object() {
            *current = serde_json::Value::Object(serde_json::Map::new());
        }
        current
            .as_object_mut()
            .expect("just ensured object")
            .insert((*final_key).to_string(), value);
    }
}

/// Recursively collect settings paths with their JSON values (for DB storage).
pub(super) fn collect_settings_json(
    value: &serde_json::Value,
    prefix: String,
    results: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                collect_settings_json(val, path, results);
            }
        }
        other => {
            results.insert(prefix, other.clone());
        }
    }
}

/// Recursively collect settings paths and values.
pub(super) fn collect_settings(
    value: &serde_json::Value,
    prefix: String,
    results: &mut Vec<(String, String)>,
) {
    match value {
        serde_json::Value::Object(obj) => {
            for (key, val) in obj {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", prefix, key)
                };
                collect_settings(val, path, results);
            }
        }
        serde_json::Value::Array(arr) => {
            let display = serde_json::to_string(arr).unwrap_or_default();
            results.push((prefix, display));
        }
        serde_json::Value::String(s) => {
            results.push((prefix, s.clone()));
        }
        serde_json::Value::Number(n) => {
            results.push((prefix, n.to_string()));
        }
        serde_json::Value::Bool(b) => {
            results.push((prefix, b.to_string()));
        }
        serde_json::Value::Null => {
            results.push((prefix, "null".to_string()));
        }
    }
}

/// Recursively merge `other` into `target`, but only for fields where
/// `other` differs from `defaults`. This means only explicitly-set values
/// in the TOML file override the base settings.
pub(super) fn merge_non_default(
    target: &mut serde_json::Value,
    other: &serde_json::Value,
    defaults: &serde_json::Value,
) {
    match (target, other, defaults) {
        (
            serde_json::Value::Object(t),
            serde_json::Value::Object(o),
            serde_json::Value::Object(d),
        ) => {
            for (key, other_val) in o {
                let default_val = d.get(key).cloned().unwrap_or(serde_json::Value::Null);
                if let Some(target_val) = t.get_mut(key) {
                    merge_non_default(target_val, other_val, &default_val);
                } else if other_val != &default_val {
                    t.insert(key.clone(), other_val.clone());
                }
            }
        }
        (target, other, defaults) => {
            if other != defaults {
                *target = other.clone();
            }
        }
    }
}
