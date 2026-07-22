pub(super) fn insert_dotted_path(
    root: &mut serde_json::Value,
    path: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    const MAX_PATH_BYTES: usize = 2048;
    const MAX_PATH_COMPONENTS: usize = 64;
    const MAX_COMPONENT_BYTES: usize = 256;

    let parts: Vec<&str> = path.split('.').collect();
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || parts.len() > MAX_PATH_COMPONENTS
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > MAX_COMPONENT_BYTES
                || part.chars().any(char::is_control)
        })
    {
        return Err("settings path is empty, malformed, or oversized".to_string());
    }

    let mut current = root;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_object_mut()
            .ok_or_else(|| format!("settings path conflicts at '{part}'"))?
            .entry(*part)
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
        if !current.is_object() {
            return Err(format!("settings path conflicts at '{part}'"));
        }
    }

    if let Some(final_key) = parts.last() {
        current
            .as_object_mut()
            .ok_or_else(|| format!("settings path conflicts at '{final_key}'"))?
            .insert((*final_key).to_string(), value);
    }
    Ok(())
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
