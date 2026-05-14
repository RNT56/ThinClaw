use super::*;

fn default_true() -> bool {
    true
}

fn default_max_output_length() -> usize {
    100_000
}

fn default_smart_approval_mode() -> String {
    "off".to_string()
}

fn default_external_scanner_mode() -> String {
    "fail_open".to_string()
}

/// Safety configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetySettings {
    /// Maximum output length in bytes.
    #[serde(default = "default_max_output_length")]
    pub max_output_length: usize,

    /// Whether injection check is enabled.
    #[serde(default = "default_true")]
    pub injection_check_enabled: bool,

    /// Whether prompt construction should redact user identifiers.
    #[serde(default = "default_true")]
    pub redact_pii_in_prompts: bool,

    /// Shell smart-approval mode for soft-flagged commands.
    #[serde(default = "default_smart_approval_mode")]
    pub smart_approval_mode: String,

    /// External shell-scanner mode: "off", "fail_open", or "fail_closed".
    #[serde(default = "default_external_scanner_mode")]
    pub external_scanner_mode: String,

    /// Optional absolute path to a first-party external shell scanner binary.
    #[serde(default)]
    pub external_scanner_path: Option<PathBuf>,

    /// Whether external shell scanners must carry verified ThinClaw provenance.
    #[serde(default)]
    pub external_scanner_require_verified: bool,
}

impl Default for SafetySettings {
    fn default() -> Self {
        Self {
            max_output_length: default_max_output_length(),
            injection_check_enabled: true,
            redact_pii_in_prompts: true,
            smart_approval_mode: default_smart_approval_mode(),
            external_scanner_mode: default_external_scanner_mode(),
            external_scanner_path: None,
            external_scanner_require_verified: false,
        }
    }
}
