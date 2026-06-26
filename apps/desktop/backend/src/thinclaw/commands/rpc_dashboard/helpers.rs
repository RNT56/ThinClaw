//! Shared JSON-shape and value-normalization helpers for the dashboard RPC
//! commands. These are reused across the cost, routing, and channel surfaces.

pub(super) fn json_number_as_f64(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_u64().map(|n| n as f64))
        .or_else(|| value.as_i64().map(|n| n as f64))
}

pub(super) fn json_f64_field(value: &serde_json::Value, key: &str) -> f64 {
    value.get(key).and_then(json_number_as_f64).unwrap_or(0.0)
}

pub(super) fn json_bool_field(value: &serde_json::Value, key: &str) -> bool {
    value.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

pub(super) fn json_string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn json_f64_map(
    value: &serde_json::Value,
    key: &str,
) -> std::collections::HashMap<String, f64> {
    value
        .get(key)
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| json_number_as_f64(v).map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn json_string_vec_field(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn json_string_vec(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn normalize_provider_order(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        normalized.push(value.to_string());
    }
    normalized
}

pub(super) fn setting_value(raw: serde_json::Value) -> serde_json::Value {
    if let Some(value) = raw.get("value").cloned() {
        value
    } else {
        raw
    }
}
