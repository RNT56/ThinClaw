use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillParameter {
    pub name: String,
    pub description: String,
    pub param_type: String, // "string", "number", "boolean", "array"
    pub required: bool,
    pub default: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: Option<String>,
    pub tools_used: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<SkillParameter>,
    pub script_file: String, // Relative path to .rhai file
}

impl SkillManifest {
    /// Save manifest to TOML string
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }

    /// Load manifest from TOML string
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }
}
