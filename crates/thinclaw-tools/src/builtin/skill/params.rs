//! Skill tool policy: params.

use crate::ports::ToolSkillCheckSource;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSearchParams {
    pub query: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInspectParams {
    pub name: String,
    pub include_content: bool,
    pub include_files: bool,
    pub audit: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallParams {
    pub name: String,
    pub force: bool,
    pub approve_risky: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillUpdateParams {
    pub name: String,
    pub approve_risky: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillPublishParams {
    pub name: String,
    pub target_repo: String,
    pub dry_run: bool,
    pub remote_write: bool,
    pub confirm_remote_write: bool,
    pub approve_risky: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillReloadParams {
    pub name: Option<String>,
    pub all: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTrustPromoteParams {
    pub name: String,
    pub target_trust: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTapAddParams {
    pub repo: String,
    pub path: String,
    pub branch: Option<String>,
    pub trust_level: String,
    pub replace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTapRemoveParams {
    pub repo: String,
    pub path: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillTapRefreshParams {
    pub repo: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillCheckInput {
    InlineContent(String),
    Url(String),
    Path(String),
}

impl SkillCheckInput {
    pub fn source_kind(&self) -> &'static str {
        match self {
            Self::InlineContent(_) => "content",
            Self::Url(_) => "url",
            Self::Path(_) => "path",
        }
    }

    pub fn source_ref(&self) -> String {
        match self {
            Self::InlineContent(_) => "(inline content)".to_string(),
            Self::Url(url) => url.clone(),
            Self::Path(path) => path.clone(),
        }
    }

    pub fn inline_content(&self) -> Option<&str> {
        match self {
            Self::InlineContent(content) => Some(content),
            Self::Url(_) | Self::Path(_) => None,
        }
    }
}

impl From<ToolSkillCheckSource> for SkillCheckInput {
    fn from(source: ToolSkillCheckSource) -> Self {
        match source {
            ToolSkillCheckSource::InlineContent { content } => Self::InlineContent(content),
            ToolSkillCheckSource::Path { path } => Self::Path(path),
            ToolSkillCheckSource::Url { url } => Self::Url(url),
        }
    }
}

impl From<SkillCheckInput> for ToolSkillCheckSource {
    fn from(input: SkillCheckInput) -> Self {
        match input {
            SkillCheckInput::InlineContent(content) => Self::InlineContent { content },
            SkillCheckInput::Path(path) => Self::Path { path },
            SkillCheckInput::Url(url) => Self::Url { url },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillListParams {
    pub verbose: bool,
}
