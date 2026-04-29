use super::*;

fn default_true() -> bool {
    true
}

fn default_extensions_user_tools_dir() -> String {
    crate::platform::resolve_data_dir("user-tools")
        .to_string_lossy()
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionsSettings {
    #[serde(default = "default_extensions_user_tools_dir")]
    pub user_tools_dir: String,
    #[serde(default)]
    pub allow_native_plugins: bool,
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
            require_plugin_signatures: true,
            trusted_manifest_keys: Vec::new(),
            trusted_manifest_public_keys: HashMap::new(),
            native_plugin_allowlist_dirs: Vec::new(),
        }
    }
}
