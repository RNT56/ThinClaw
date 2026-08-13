use super::*;

fn default_true() -> bool {
    true
}

fn default_extensions_user_tools_dir() -> String {
    thinclaw_platform::resolve_data_dir("user-tools")
        .to_string_lossy()
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionsSettings {
    #[serde(default = "default_extensions_user_tools_dir")]
    pub user_tools_dir: String,
    #[serde(default)]
    pub allow_native_plugins: bool,
    /// Explicit compatibility opt-in for legacy in-process native loading.
    /// Admission alone is insufficient because a native fault can terminate
    /// the host. Keep false unless the plugin is audited and that risk is
    /// consciously accepted.
    #[serde(default)]
    pub allow_unsafe_in_process_native_plugins: bool,
    #[serde(default = "default_true")]
    pub require_plugin_signatures: bool,
    #[serde(default)]
    pub trusted_manifest_keys: Vec<String>,
    #[serde(default)]
    pub trusted_manifest_public_keys: HashMap<String, String>,
    #[serde(default)]
    pub native_plugin_allowlist_dirs: Vec<String>,
}

impl Default for ExtensionsSettings {
    fn default() -> Self {
        Self {
            user_tools_dir: default_extensions_user_tools_dir(),
            allow_native_plugins: false,
            allow_unsafe_in_process_native_plugins: false,
            require_plugin_signatures: true,
            trusted_manifest_keys: Vec::new(),
            trusted_manifest_public_keys: HashMap::new(),
            native_plugin_allowlist_dirs: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_admission_and_unsafe_compatibility_default_off_independently() {
        let settings = ExtensionsSettings::default();
        assert!(!settings.allow_native_plugins);
        assert!(!settings.allow_unsafe_in_process_native_plugins);
    }

    #[test]
    fn legacy_extension_settings_do_not_gain_unsafe_native_compatibility() {
        let settings: ExtensionsSettings = serde_json::from_value(serde_json::json!({
            "allow_native_plugins": true
        }))
        .unwrap();
        assert!(settings.allow_native_plugins);
        assert!(!settings.allow_unsafe_in_process_native_plugins);
    }
}
