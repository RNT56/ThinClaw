//! Root-independent tool registry storage and filtering.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

#[cfg(feature = "document-extraction")]
use crate::builtin::ExtractDocumentTool;
use crate::builtin::{
    ADVISOR_TOOL_NAME, AgentManagementPort, AgentThinkTool, AppleMailTool, ApplyPatchTool,
    CanvasTool, ClarifyTool, ConsultAdvisorTool, CreateAgentTool, DesktopAutonomyPort,
    DesktopAutonomyTool, DeviceInfoTool, EchoTool, EmitUserMessageTool, ExtensionManagementPort,
    FileToolHost, GrepTool, HomeAssistantTool, HttpTool, JsonTool, ListAgentsTool, ListDirTool,
    LlmListModelsTool, LlmSelectTool, MessageAgentTool, MoaTool, ProcessTool, ReadFileTool,
    RemoveAgentTool, SearchFilesTool, SendMessageFn, SendMessageTool, SharedModelOverride,
    SharedProcessRegistry, SharedTodoStore, TimeTool, TodoTool, ToolActivateTool, ToolAuthTool,
    ToolInstallTool, ToolListTool, ToolRemoveTool, ToolSearchTool, TtsTool, UpdateAgentTool,
    VisionAnalyzeTool, WebSearchTool, WriteFileTool,
};
use crate::execution::LocalExecutionBackend;
use crate::wasm::SharedCredentialRegistry;
#[cfg(feature = "wasm-runtime")]
use crate::wasm::{
    Capabilities, HostToolInvoker, OAuthRefreshConfig, ResourceLimits, WasmError, WasmStorageError,
    WasmToolRuntime, WasmToolStore, WasmToolWrapper,
};
use thinclaw_llm_core::{LlmProvider, ToolDefinition};
#[cfg(feature = "wasm-runtime")]
use thinclaw_secrets::CredentialLocation;
use thinclaw_secrets::SecretsStore;
use thinclaw_tools_core::{
    ApprovalRequirement, RateLimiter, Tool, ToolDescriptor, ToolDomain, ToolExecutionLane,
    ToolProfile,
};
/// Stable provenance vocabulary for every live tool identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolOrigin {
    Core,
    Memory,
    Dev,
    Job,
    ExtensionAdmin,
    Skill,
    Learning,
    RepoProject,
    Media,
    Desktop,
    HardwareBridge,
    Channel,
    Subagent,
    Llm,
    Agent,
    Routine,
    Wasm,
    Mcp,
    UserTool,
    NativePlugin,
}

pub const ALL_TOOL_ORIGINS: &[ToolOrigin] = &[
    ToolOrigin::Core,
    ToolOrigin::Memory,
    ToolOrigin::Dev,
    ToolOrigin::Job,
    ToolOrigin::ExtensionAdmin,
    ToolOrigin::Skill,
    ToolOrigin::Learning,
    ToolOrigin::RepoProject,
    ToolOrigin::Media,
    ToolOrigin::Desktop,
    ToolOrigin::HardwareBridge,
    ToolOrigin::Channel,
    ToolOrigin::Subagent,
    ToolOrigin::Llm,
    ToolOrigin::Agent,
    ToolOrigin::Routine,
    ToolOrigin::Wasm,
    ToolOrigin::Mcp,
    ToolOrigin::UserTool,
    ToolOrigin::NativePlugin,
];

impl std::fmt::Display for ToolOrigin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = serde_json::to_value(self).map_err(|_| std::fmt::Error)?;
        formatter.write_str(value.as_str().ok_or(std::fmt::Error)?)
    }
}

/// One compile-time static capability identity. Runtime predicates decide
/// whether a catalogued identity is actually inserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticToolDescriptor {
    pub name: &'static str,
    pub origin: ToolOrigin,
}

macro_rules! static_tools {
    ($( $origin:ident => [$( $name:literal ),* $(,)?] ),* $(,)?) => {
        pub const STATIC_TOOL_CATALOG: &[StaticToolDescriptor] = &[
            $($(StaticToolDescriptor { name: $name, origin: ToolOrigin::$origin },)*)*
        ];
        /// Reserved names are generated from the same catalog as descriptors.
        pub const PROTECTED_TOOL_NAMES: &[&str] = &[$($($name,)*)*];
    };
}

static_tools! {
    Core => ["echo", "time", "json", "device_info", "canvas", "clarify",
        "agent_think", "emit_user_message", "http", "web_search", "extract_document",
        "homeassistant", "browser", "vision_analyze", "mixture_of_agents"],
    Dev => ["shell", "read_file", "write_file", "list_dir", "apply_patch", "grep",
        "build_software", "todo", "process", "execute_code", "search_files"],
    Memory => ["memory_search", "session_search", "memory_write", "memory_read",
        "memory_tree", "memory_delete"],
    ExtensionAdmin => ["tool_search", "tool_install", "tool_auth", "tool_activate",
        "tool_list", "tool_remove"],
    Skill => ["skill_inspect", "skill_read", "skill_list", "skill_search", "skill_check",
        "skill_install", "skill_update", "skill_audit", "skill_snapshot", "skill_publish",
        "skill_tap_list", "skill_tap_add", "skill_tap_remove", "skill_tap_refresh",
        "skill_remove", "skill_reload", "skill_trust_promote"],
    Learning => ["prompt_manage", "skill_manage", "learning_status", "learning_outcomes",
        "learning_history", "learning_feedback", "external_memory_recall",
        "external_memory_export", "external_memory_setup", "external_memory_off",
        "external_memory_status", "learning_proposal_review"],
    RepoProject => ["repo_project_create", "repo_project_plan", "repo_project_status",
        "repo_project_pause", "repo_project_resume", "repo_project_enroll",
        "repo_project_setup", "repo_project_approve", "repo_project_request_credential",
        "repo_project_set_credential", "repo_project_list_repos", "repo_project_connect"],
    Media => ["tts", "image_generate", "comfy_health", "comfy_check_deps",
        "comfy_run_workflow", "comfy_manage"],
    Channel => ["apple_mail", "send_message", "nostr_actions"],
    Desktop => ["screen_capture", "camera_capture", "talk_mode", "location",
        "desktop_apps", "desktop_ui", "desktop_screen", "desktop_calendar_native",
        "desktop_numbers_native", "desktop_pages_native", "autonomy_control"],
    HardwareBridge => ["capture_camera_frame", "record_audio_clip", "capture_screenshot"],
    Job => ["create_job", "list_jobs", "job_status", "cancel_job", "job_events", "job_prompt"],
    Subagent => ["spawn_subagent", "list_subagents", "cancel_subagent"],
    Llm => ["llm_select", "llm_list_models", "consult_advisor"],
    Agent => ["create_agent", "list_agents", "update_agent", "remove_agent", "message_agent"],
    Routine => ["routine_create", "routine_list", "routine_update", "routine_delete",
        "routine_history"],
}

pub fn static_tool_descriptor(name: &str) -> Option<&'static StaticToolDescriptor> {
    STATIC_TOOL_CATALOG
        .iter()
        .find(|descriptor| descriptor.name == name)
}

#[derive(Clone)]
pub struct RegistrationRequest {
    pub tool: Arc<dyn Tool>,
    pub origin: ToolOrigin,
    pub source_id: String,
    pub source_digest: Option<String>,
    pub replace: bool,
}

impl RegistrationRequest {
    pub fn new(tool: Arc<dyn Tool>, origin: ToolOrigin, source_id: impl Into<String>) -> Self {
        Self {
            tool,
            origin,
            source_id: source_id.into(),
            source_digest: None,
            replace: false,
        }
    }

    pub fn with_digest(mut self, digest: impl Into<String>) -> Self {
        self.source_digest = Some(digest.into());
        self
    }

    pub fn replacing(mut self) -> Self {
        self.replace = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegistrationConflict {
    pub name: String,
    pub requested_origin: ToolOrigin,
    pub requested_source_id: String,
    pub existing_origin: Option<ToolOrigin>,
    pub existing_source_id: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationOutcome {
    Inserted { revision: u64 },
    Rebound { revision: u64 },
    Unchanged { revision: u64 },
    Rejected { conflict: RegistrationConflict },
}

impl RegistrationOutcome {
    pub fn accepted(&self) -> bool {
        !matches!(self, Self::Rejected { .. })
    }

    pub fn changed(&self) -> bool {
        matches!(self, Self::Inserted { .. } | Self::Rebound { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegistryIdentity {
    pub name: String,
    pub origin: ToolOrigin,
    pub source_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<String>,
    pub revision: u64,
    pub registered_at: chrono::DateTime<chrono::Utc>,
    pub compiled: bool,
    /// `None` means configuration is not authoritatively known at the registry
    /// boundary; registration alone must never imply it.
    pub configured: Option<bool>,
    pub registered: bool,
    pub dependency: String,
    pub exposed: bool,
    pub ready: String,
    pub approval: String,
    pub health: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegistrySnapshot {
    pub schema_version: u8,
    pub revision: u64,
    pub sealed: bool,
    pub identities: Vec<RegistryIdentity>,
}

#[derive(Clone)]
struct RegistryEntry {
    tool: Arc<dyn Tool>,
    identity: RegistryIdentity,
}

fn registry_identity(
    name: String,
    origin: ToolOrigin,
    source_id: String,
    source_digest: Option<String>,
    revision: u64,
    tool: &dyn Tool,
) -> RegistryIdentity {
    let descriptor = tool.descriptor();
    let approval = match descriptor.metadata.approval_class {
        thinclaw_tools_core::ToolApprovalClass::Never => "never",
        thinclaw_tools_core::ToolApprovalClass::Conditional => "conditional",
        thinclaw_tools_core::ToolApprovalClass::Always => "always",
    };
    let exposed = !HIDDEN_BY_DEFAULT_TOOL_NAMES.contains(&name.as_str());
    let dynamic = matches!(
        origin,
        ToolOrigin::Wasm | ToolOrigin::Mcp | ToolOrigin::UserTool | ToolOrigin::NativePlugin
    );
    RegistryIdentity {
        name,
        origin,
        source_id,
        source_digest,
        revision,
        registered_at: chrono::Utc::now(),
        compiled: true,
        // A tool only reaches the registry after its constructor/loader has
        // accepted the concrete runtime configuration.
        configured: Some(true),
        registered: true,
        dependency: if dynamic { "loaded" } else { "satisfied" }.to_string(),
        exposed,
        ready: if exposed { "ready" } else { "hidden_by_policy" }.to_string(),
        approval: approval.to_string(),
        health: if dynamic {
            "not_probed"
        } else {
            "not_required"
        }
        .to_string(),
        reasons: if exposed {
            Vec::new()
        } else {
            vec!["hidden from default model exposure by runtime policy".to_string()]
        },
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistrySealError {
    #[error("startup tool registrations were rejected: {identities:?}")]
    RegistrationFailures { identities: Vec<String> },
    #[error("cannot seal an empty runtime tool registry")]
    EmptyRegistry,
    #[error(
        "tool descriptor identities differ from the live registry: registry={registry:?}, descriptors={descriptors:?}"
    )]
    DescriptorMismatch {
        registry: Vec<String>,
        descriptors: Vec<String>,
    },
}

const IMPLICIT_CAPABILITY_TOOLS: &[&str] = &["agent_think", "emit_user_message"];
const HIDDEN_BY_DEFAULT_TOOL_NAMES: &[&str] = &[
    "external_memory_recall",
    "external_memory_export",
    "external_memory_setup",
    "external_memory_off",
    "external_memory_status",
];
const SKILL_ADMIN_TOOLS: &[&str] = &[
    "skill_search",
    "skill_check",
    "skill_install",
    "skill_update",
    "skill_audit",
    "skill_snapshot",
    "skill_publish",
    "skill_tap_list",
    "skill_tap_add",
    "skill_tap_remove",
    "skill_tap_refresh",
    "skill_remove",
    "skill_reload",
    "skill_trust_promote",
    "skill_manage",
];

/// Registry of available tools.
pub struct ToolRegistry {
    entries: RwLock<HashMap<String, RegistryEntry>>,
    revision: std::sync::atomic::AtomicU64,
    sealed: std::sync::atomic::AtomicBool,
    startup_registration_failures: Mutex<Vec<String>>,
    snapshot_tx: tokio::sync::watch::Sender<RegistrySnapshot>,
    rate_limiter: RateLimiter,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        let (snapshot_tx, _) = tokio::sync::watch::channel(RegistrySnapshot {
            schema_version: 2,
            revision: 0,
            sealed: false,
            identities: Vec::new(),
        });
        Self {
            entries: RwLock::new(HashMap::new()),
            revision: std::sync::atomic::AtomicU64::new(0),
            sealed: std::sync::atomic::AtomicBool::new(false),
            startup_registration_failures: Mutex::new(Vec::new()),
            snapshot_tx,
            rate_limiter: RateLimiter::new(),
        }
    }

    /// Get the shared rate limiter for checking built-in tool limits.
    pub fn rate_limiter(&self) -> &RateLimiter {
        &self.rate_limiter
    }

    /// Register a legacy dynamic tool with deterministic user-tool provenance.
    pub async fn register(&self, tool: Arc<dyn Tool>) -> RegistrationOutcome {
        let source_id = format!("legacy/user-tool/{}", tool.name());
        self.register_request(RegistrationRequest::new(
            tool,
            ToolOrigin::UserTool,
            source_id,
        ))
    }

    /// Register a tool as a static built-in.
    pub async fn register_builtin(&self, tool: Arc<dyn Tool>) -> RegistrationOutcome {
        self.register_static(tool)
    }

    /// Register a static tool synchronously. The short metadata lock cannot
    /// silently fail under async contention.
    pub fn register_sync(&self, tool: Arc<dyn Tool>) -> RegistrationOutcome {
        self.register_static(tool)
    }

    fn register_static(&self, tool: Arc<dyn Tool>) -> RegistrationOutcome {
        let name = tool.name().to_string();
        let origin = static_tool_descriptor(&name)
            .map(|descriptor| descriptor.origin)
            .unwrap_or(ToolOrigin::Core);
        let outcome = self.register_request_inner(
            RegistrationRequest::new(tool, origin, format!("builtin/{name}")),
            true,
        );
        self.record_startup_rejection(&outcome);
        outcome
    }

    pub fn register_request(&self, request: RegistrationRequest) -> RegistrationOutcome {
        let outcome = self.register_request_inner(request, false);
        self.record_startup_rejection(&outcome);
        outcome
    }

    fn record_startup_rejection(&self, outcome: &RegistrationOutcome) {
        if self.sealed.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        if let RegistrationOutcome::Rejected { conflict } = outcome {
            let identity = format!(
                "{} [{}:{}] {}",
                conflict.name,
                conflict.requested_origin,
                conflict.requested_source_id,
                conflict.reason
            );
            let mut failures = self
                .startup_registration_failures
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !failures.contains(&identity) {
                failures.push(identity);
            }
        }
    }

    fn register_request_inner(
        &self,
        request: RegistrationRequest,
        static_registration: bool,
    ) -> RegistrationOutcome {
        let name = request.tool.name().to_string();
        if let Some(descriptor) = static_tool_descriptor(&name) {
            let exact_source_id = format!("builtin/{name}");
            if !static_registration
                || request.origin != descriptor.origin
                || request.source_id != exact_source_id
            {
                return RegistrationOutcome::Rejected {
                    conflict: RegistrationConflict {
                        name,
                        requested_origin: request.origin,
                        requested_source_id: request.source_id,
                        existing_origin: None,
                        existing_source_id: None,
                        reason: "identity is reserved by the static tool catalog".to_string(),
                    },
                };
            }
        }

        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(existing) = entries.get(&name) {
            let same_owner = existing.identity.origin == request.origin
                && existing.identity.source_id == request.source_id;
            if !same_owner || !request.replace {
                return if same_owner && Arc::ptr_eq(&existing.tool, &request.tool) {
                    RegistrationOutcome::Unchanged {
                        revision: existing.identity.revision,
                    }
                } else {
                    RegistrationOutcome::Rejected {
                        conflict: RegistrationConflict {
                            name,
                            requested_origin: request.origin,
                            requested_source_id: request.source_id,
                            existing_origin: Some(existing.identity.origin),
                            existing_source_id: Some(existing.identity.source_id.clone()),
                            reason: if same_owner {
                                "same-source replacement requires replace=true".to_string()
                            } else {
                                "identity is owned by a different source".to_string()
                            },
                        },
                    }
                };
            }
        }

        let rebound = entries.contains_key(&name);
        let revision = self.next_revision();
        let identity = registry_identity(
            name.clone(),
            request.origin,
            request.source_id,
            request.source_digest,
            revision,
            request.tool.as_ref(),
        );
        entries.insert(
            name.clone(),
            RegistryEntry {
                tool: request.tool,
                identity,
            },
        );
        self.publish_snapshot_locked(&entries);
        if rebound {
            RegistrationOutcome::Rebound { revision }
        } else {
            RegistrationOutcome::Inserted { revision }
        }
    }

    /// Atomically reserve and insert a complete dynamic source activation.
    pub fn register_batch(
        &self,
        requests: Vec<RegistrationRequest>,
    ) -> Result<Vec<RegistrationOutcome>, RegistrationConflict> {
        let mut names = HashSet::new();
        for request in &requests {
            let name = request.tool.name().to_string();
            if !names.insert(name.clone()) {
                return Err(RegistrationConflict {
                    name,
                    requested_origin: request.origin,
                    requested_source_id: request.source_id.clone(),
                    existing_origin: None,
                    existing_source_id: None,
                    reason: "activation contains the identity more than once".to_string(),
                });
            }
        }

        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|error| error.into_inner());
        for request in &requests {
            let name = request.tool.name().to_string();
            if static_tool_descriptor(&name).is_some() {
                return Err(RegistrationConflict {
                    name,
                    requested_origin: request.origin,
                    requested_source_id: request.source_id.clone(),
                    existing_origin: None,
                    existing_source_id: None,
                    reason: "identity is reserved by the static tool catalog".to_string(),
                });
            }
            if let Some(existing) = entries.get(&name) {
                let same_owner = existing.identity.origin == request.origin
                    && existing.identity.source_id == request.source_id;
                if !same_owner || !request.replace {
                    return Err(RegistrationConflict {
                        name,
                        requested_origin: request.origin,
                        requested_source_id: request.source_id.clone(),
                        existing_origin: Some(existing.identity.origin),
                        existing_source_id: Some(existing.identity.source_id.clone()),
                        reason: "activation identity conflicts with the live registry".to_string(),
                    });
                }
            }
        }

        let mut outcomes = Vec::with_capacity(requests.len());
        let revision = if requests.is_empty() {
            self.revision.load(std::sync::atomic::Ordering::Acquire)
        } else {
            self.next_revision()
        };
        for request in requests {
            let name = request.tool.name().to_string();
            let rebound = entries.contains_key(&name);
            let identity = registry_identity(
                name.clone(),
                request.origin,
                request.source_id,
                request.source_digest,
                revision,
                request.tool.as_ref(),
            );
            entries.insert(
                name.clone(),
                RegistryEntry {
                    tool: request.tool,
                    identity,
                },
            );
            outcomes.push(if rebound {
                RegistrationOutcome::Rebound { revision }
            } else {
                RegistrationOutcome::Inserted { revision }
            });
        }
        self.publish_snapshot_locked(&entries);
        Ok(outcomes)
    }

    /// Atomically replace the complete population owned by one dynamic source.
    /// Additions, replacements, and stale removals publish exactly one N+1
    /// revision, so snapshot consumers can never observe a half-reconciled source.
    pub fn reconcile_source(
        &self,
        origin: ToolOrigin,
        source_id: &str,
        requests: Vec<RegistrationRequest>,
    ) -> Result<Vec<RegistrationOutcome>, RegistrationConflict> {
        let mut names = HashSet::new();
        for request in &requests {
            let name = request.tool.name().to_string();
            if request.origin != origin || request.source_id != source_id {
                return Err(RegistrationConflict {
                    name,
                    requested_origin: request.origin,
                    requested_source_id: request.source_id.clone(),
                    existing_origin: None,
                    existing_source_id: None,
                    reason: "reconcile batch contains a different source owner".to_string(),
                });
            }
            if static_tool_descriptor(&name).is_some() || !names.insert(name.clone()) {
                return Err(RegistrationConflict {
                    name,
                    requested_origin: request.origin,
                    requested_source_id: request.source_id.clone(),
                    existing_origin: None,
                    existing_source_id: None,
                    reason: "reconcile identity is reserved or duplicated".to_string(),
                });
            }
        }

        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|error| error.into_inner());
        for request in &requests {
            let name = request.tool.name().to_string();
            if let Some(existing) = entries.get(&name) {
                let same_owner =
                    existing.identity.origin == origin && existing.identity.source_id == source_id;
                if !same_owner || !request.replace {
                    return Err(RegistrationConflict {
                        name,
                        requested_origin: request.origin,
                        requested_source_id: request.source_id.clone(),
                        existing_origin: Some(existing.identity.origin),
                        existing_source_id: Some(existing.identity.source_id.clone()),
                        reason: "reconcile identity conflicts with the live registry".to_string(),
                    });
                }
            }
        }

        let owned_before = entries
            .values()
            .filter(|entry| {
                entry.identity.origin == origin && entry.identity.source_id == source_id
            })
            .map(|entry| entry.identity.name.clone())
            .collect::<HashSet<_>>();
        if requests.is_empty() && owned_before.is_empty() {
            return Ok(Vec::new());
        }
        let revision = self.next_revision();
        entries.retain(|_, entry| {
            entry.identity.origin != origin
                || entry.identity.source_id != source_id
                || names.contains(&entry.identity.name)
        });
        let mut outcomes = Vec::with_capacity(requests.len());
        for request in requests {
            let name = request.tool.name().to_string();
            let rebound = owned_before.contains(&name);
            let identity = registry_identity(
                name.clone(),
                request.origin,
                request.source_id,
                request.source_digest,
                revision,
                request.tool.as_ref(),
            );
            entries.insert(
                name,
                RegistryEntry {
                    tool: request.tool,
                    identity,
                },
            );
            outcomes.push(if rebound {
                RegistrationOutcome::Rebound { revision }
            } else {
                RegistrationOutcome::Inserted { revision }
            });
        }
        self.publish_snapshot_locked(&entries);
        Ok(outcomes)
    }

    fn next_revision(&self) -> u64 {
        self.revision
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1
    }

    fn snapshot_from_entries(&self, entries: &HashMap<String, RegistryEntry>) -> RegistrySnapshot {
        let mut identities = entries
            .values()
            .map(|entry| entry.identity.clone())
            .collect::<Vec<_>>();
        identities.sort_by(|left, right| {
            (
                &left.name,
                left.origin.to_string(),
                &left.source_id,
                left.revision,
            )
                .cmp(&(
                    &right.name,
                    right.origin.to_string(),
                    &right.source_id,
                    right.revision,
                ))
        });
        RegistrySnapshot {
            schema_version: 2,
            revision: self.revision.load(std::sync::atomic::Ordering::Acquire),
            sealed: self.sealed.load(std::sync::atomic::Ordering::Acquire),
            identities,
        }
    }

    fn publish_snapshot_locked(&self, entries: &HashMap<String, RegistryEntry>) {
        self.snapshot_tx
            .send_replace(self.snapshot_from_entries(entries));
    }

    pub fn snapshot(&self) -> RegistrySnapshot {
        let entries = self
            .entries
            .read()
            .unwrap_or_else(|error| error.into_inner());
        self.snapshot_from_entries(&entries)
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<RegistrySnapshot> {
        self.snapshot_tx.subscribe()
    }

    /// Publish a coherent N+1 capability revision for a runtime-owned hot
    /// mutation that changes a non-tool capability while leaving the tool
    /// identity population unchanged.
    pub fn advance_capability_revision(&self) -> RegistrySnapshot {
        // Capability-only mutations must take the same exclusive boundary as
        // tool population changes. A read guard would allow two callers to
        // increment concurrently and both publish the later revision, losing
        // the one-receipt/one-revision relationship.
        let entries = self
            .entries
            .write()
            .unwrap_or_else(|error| error.into_inner());
        self.next_revision();
        let snapshot = self.snapshot_from_entries(&entries);
        self.snapshot_tx.send_replace(snapshot.clone());
        snapshot
    }

    /// Seal the fully assembled startup population after verifying exact name
    /// parity with the descriptors exposed to the agent.
    pub fn seal_startup(&self) -> Result<RegistrySnapshot, RegistrySealError> {
        let mut failures = self
            .startup_registration_failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if !failures.is_empty() {
            failures.sort();
            return Err(RegistrySealError::RegistrationFailures {
                identities: failures,
            });
        }
        let entries = self
            .entries
            .read()
            .unwrap_or_else(|error| error.into_inner());
        if entries.is_empty() {
            return Err(RegistrySealError::EmptyRegistry);
        }
        let mut registry = entries.keys().cloned().collect::<Vec<_>>();
        let mut descriptors = entries
            .values()
            .map(|entry| entry.tool.descriptor().name)
            .collect::<Vec<_>>();
        registry.sort();
        descriptors.sort();
        if registry != descriptors {
            return Err(RegistrySealError::DescriptorMismatch {
                registry,
                descriptors,
            });
        }
        self.sealed
            .store(true, std::sync::atomic::Ordering::Release);
        let snapshot = self.snapshot_from_entries(&entries);
        self.snapshot_tx.send_replace(snapshot.clone());
        Ok(snapshot)
    }

    /// Register the root-independent default built-ins.
    ///
    /// Host-specific tools such as browser backends, app adapters, and
    /// sandbox/job orchestration are intentionally registered by the root/app
    /// layer after it has concrete runtime dependencies.
    pub fn register_core_builtin_tools(
        &self,
        credential_registry: Option<Arc<SharedCredentialRegistry>>,
        secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    ) {
        self.register_sync(Arc::new(EchoTool));
        self.register_sync(Arc::new(TimeTool));
        self.register_sync(Arc::new(JsonTool));
        self.register_sync(Arc::new(DeviceInfoTool::new()));
        self.register_sync(Arc::new(CanvasTool));
        self.register_sync(Arc::new(ClarifyTool));
        self.register_sync(Arc::new(AgentThinkTool));
        self.register_sync(Arc::new(EmitUserMessageTool));

        let mut http = HttpTool::new();
        if let (Some(credential_registry), Some(secrets_store)) =
            (credential_registry, secrets_store)
        {
            http = http.with_credentials(credential_registry, secrets_store);
        }
        self.register_sync(Arc::new(http));

        // Zero-config web search (no API key). Always on so a fresh install can
        // answer current-events / lookup questions out of the box.
        self.register_sync(Arc::new(WebSearchTool::new()));

        #[cfg(feature = "document-extraction")]
        self.register_sync(Arc::new(ExtractDocumentTool));

        if let Some(home_assistant) = HomeAssistantTool::from_env() {
            self.register_sync(Arc::new(home_assistant));
            tracing::info!("Registered Home Assistant tool (HASS_URL + HASS_TOKEN)");
        }
    }

    /// Register filesystem development tools with an optional base directory.
    pub fn register_filesystem_tools(
        &self,
        base_dir: Option<PathBuf>,
        file_host: Arc<dyn FileToolHost>,
    ) {
        if let Some(base_dir) = base_dir {
            self.register_sync(Arc::new(
                ReadFileTool::new()
                    .with_base_dir(base_dir.clone())
                    .with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(
                WriteFileTool::new()
                    .with_base_dir(base_dir.clone())
                    .with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(ListDirTool::new().with_base_dir(base_dir.clone())));
            self.register_sync(Arc::new(
                ApplyPatchTool::new()
                    .with_base_dir(base_dir.clone())
                    .with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(GrepTool::new().with_base_dir(base_dir)));
        } else {
            // No workspace base configured: the filesystem tools run in
            // unrestricted mode (no path containment). This is the deliberate
            // trusted-local-operator contract, but surface it so an operator can
            // see the agent has unsandboxed filesystem access.
            tracing::warn!(
                "Filesystem tools registered without a base directory: operating in \
                 unrestricted mode (no path containment). Configure a workspace base \
                 directory to sandbox file access."
            );
            self.register_sync(Arc::new(
                ReadFileTool::new().with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(
                WriteFileTool::new().with_host(Arc::clone(&file_host)),
            ));
            self.register_sync(Arc::new(ListDirTool::new()));
            self.register_sync(Arc::new(ApplyPatchTool::new().with_host(file_host)));
            self.register_sync(Arc::new(GrepTool::new()));
        }
    }

    /// Register extension-management tools from a host-provided lifecycle port.
    pub fn register_extension_management_tools(&self, port: Arc<dyn ExtensionManagementPort>) {
        self.register_sync(Arc::new(ToolSearchTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolInstallTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolAuthTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolActivateTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolListTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ToolRemoveTool::new(port)));
    }

    /// Register desktop-autonomy tools from a host-provided desktop port.
    pub fn register_desktop_autonomy_tools(&self, port: Arc<dyn DesktopAutonomyPort>) {
        self.register_sync(Arc::new(DesktopAutonomyTool::apps(Arc::clone(&port))));
        self.register_sync(Arc::new(DesktopAutonomyTool::ui(Arc::clone(&port))));
        self.register_sync(Arc::new(DesktopAutonomyTool::screen(Arc::clone(&port))));
        self.register_sync(Arc::new(DesktopAutonomyTool::calendar_native(Arc::clone(
            &port,
        ))));
        self.register_sync(Arc::new(DesktopAutonomyTool::numbers_native(Arc::clone(
            &port,
        ))));
        self.register_sync(Arc::new(DesktopAutonomyTool::pages_native(Arc::clone(
            &port,
        ))));
        self.register_sync(Arc::new(DesktopAutonomyTool::control(port)));
    }

    /// Register agent-management tools from a host-provided agent registry port.
    pub fn register_agent_management_tools(&self, port: Arc<dyn AgentManagementPort>) {
        self.register_sync(Arc::new(CreateAgentTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(ListAgentsTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(UpdateAgentTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(RemoveAgentTool::new(Arc::clone(&port))));
        self.register_sync(Arc::new(MessageAgentTool::new(port)));
    }

    /// Register LLM model selection/discovery tools.
    pub fn register_llm_tools(
        &self,
        model_override: SharedModelOverride,
        primary_llm: Arc<dyn LlmProvider>,
        cheap_llm: Option<Arc<dyn LlmProvider>>,
    ) {
        self.register_sync(Arc::new(LlmSelectTool::new(model_override)));
        self.register_sync(Arc::new(LlmListModelsTool::new(primary_llm, cheap_llm)));
    }

    /// Register advisor consultation when the advisor lane is ready.
    pub fn register_advisor_tool(&self, advisor_ready: bool) {
        if advisor_ready {
            self.register_sync(Arc::new(ConsultAdvisorTool));
        }
    }

    /// Reconcile advisor tool visibility with current advisor readiness.
    pub async fn reconcile_advisor_tool_readiness(&self, advisor_ready: bool) {
        if advisor_ready {
            self.register_advisor_tool(true);
        } else {
            let _ = self
                .unregister_static(ADVISOR_TOOL_NAME, ToolOrigin::Llm)
                .await;
        }
    }

    /// Register the extracted vision analysis tool.
    pub fn register_vision_tool(&self, llm: Arc<dyn LlmProvider>) {
        self.register_sync(Arc::new(VisionAnalyzeTool::new(llm)));
    }

    /// Register the extracted Mixture-of-Agents tool if the model set is viable.
    pub fn register_moa_tool(
        &self,
        primary: Arc<dyn LlmProvider>,
        cheap: Option<Arc<dyn LlmProvider>>,
        reference_models: Vec<String>,
        aggregator_model: Option<String>,
        min_successful: usize,
    ) -> bool {
        let tool = MoaTool::new(
            primary,
            cheap,
            reference_models,
            aggregator_model,
            min_successful,
        );
        if !tool.is_viable() {
            return false;
        }
        self.register_sync(Arc::new(tool));
        true
    }

    /// Register the extracted cross-platform send-message tool.
    pub fn register_send_message_tool(&self, send_fn: Option<SendMessageFn>) {
        let mut tool = SendMessageTool::new();
        if let Some(send_fn) = send_fn {
            tool = tool.with_send_fn(send_fn);
        }
        self.register_sync(Arc::new(tool));
    }

    /// Register the extracted background process tool.
    pub fn register_process_tool(
        &self,
        registry: SharedProcessRegistry,
        backend: Option<Arc<dyn LocalExecutionBackend>>,
    ) {
        let mut tool = ProcessTool::new(registry);
        if let Some(backend) = backend {
            tool = tool.with_backend(backend);
        }
        self.register_sync(Arc::new(tool));
    }

    /// Register the extracted in-session todo tool.
    pub fn register_todo_tool(&self, store: SharedTodoStore) {
        self.register_sync(Arc::new(TodoTool::new(store)));
    }

    /// Register the extracted TTS tool.
    pub fn register_tts_tool(
        &self,
        secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,
        output_dir: PathBuf,
    ) {
        self.register_sync(Arc::new(TtsTool::new(secrets, output_dir)));
    }

    /// Register the extracted Apple Mail tool.
    pub fn register_apple_mail_tool(&self, db_path: Option<PathBuf>) -> bool {
        let tool = if let Some(path) = db_path {
            AppleMailTool::new(path)
        } else if let Some(tool) = AppleMailTool::auto_detect() {
            tool
        } else {
            return false;
        };
        self.register_sync(Arc::new(tool));
        true
    }

    /// Register the extracted filename/path search tool.
    pub fn register_search_files_tool(&self, base_dir: Option<PathBuf>) {
        let mut tool = SearchFilesTool::new();
        if let Some(base_dir) = base_dir {
            tool = tool.with_base_dir(base_dir);
        }
        self.register_sync(Arc::new(tool));
    }

    /// Register a WASM tool from bytes.
    ///
    /// This validates and compiles the component, wraps it as a tool, and
    /// optionally publishes declared credential mappings for the shared HTTP
    /// injection registry.
    #[cfg(feature = "wasm-runtime")]
    pub async fn register_wasm_tool<I>(
        &self,
        reg: WasmToolRegistration<'_, I>,
        credential_registry: Option<&SharedCredentialRegistry>,
    ) -> Result<(), WasmError>
    where
        I: HostToolInvoker + 'static,
    {
        let credential_mappings = reg
            .capabilities
            .http
            .as_ref()
            .map(|http| {
                http.credentials
                    .values()
                    .filter(|mapping| {
                        matches!(
                            mapping.location,
                            CredentialLocation::AuthorizationBearer
                                | CredentialLocation::AuthorizationBasic { .. }
                                | CredentialLocation::Header { .. }
                                | CredentialLocation::QueryParam { .. }
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if credential_registry.is_some() {
            SharedCredentialRegistry::validate_source_mappings(reg.name, &credential_mappings)
                .map_err(|error| WasmError::ConfigError(error.to_string()))?;
        }

        let prepared = reg
            .runtime
            .prepare(reg.name, reg.wasm_bytes, reg.limits)
            .await?;

        let mut wrapper = WasmToolWrapper::new(Arc::clone(reg.runtime), prepared, reg.capabilities);

        if let Some(description) = reg.description {
            wrapper = wrapper.with_description(description);
        }
        if let Some(schema) = reg.schema {
            wrapper = wrapper.with_schema(schema);
        }
        if let Some(store) = reg.secrets_store {
            wrapper = wrapper.with_secrets_store(store);
        }
        if let Some(oauth) = reg.oauth_refresh {
            wrapper = wrapper.with_oauth_refresh(oauth);
        }
        if let Some(invoker) = reg.tool_invoker {
            wrapper = wrapper.with_tool_invoker(invoker);
        }

        let digest = blake3::hash(reg.wasm_bytes).to_hex().to_string();
        let source_id = format!("wasm/{}", reg.name);
        let outcome = self.register_request(
            RegistrationRequest::new(Arc::new(wrapper), ToolOrigin::Wasm, source_id.clone())
                .with_digest(digest)
                .replacing(),
        );
        if !outcome.accepted() {
            return Err(WasmError::ConfigError(format!(
                "WASM tool '{}' conflicts with a protected or built-in tool name",
                reg.name
            )));
        }

        if let Some(registry) = credential_registry {
            let count = credential_mappings.len();
            if let Err(error) = registry.replace_source_mappings(reg.name, credential_mappings) {
                let _ = self
                    .unregister_owned(reg.name, ToolOrigin::Wasm, &source_id)
                    .await;
                return Err(WasmError::ConfigError(format!(
                    "failed to publish WASM credential mappings: {error}"
                )));
            }
            tracing::debug!(
                name = reg.name,
                credential_count = count,
                "Added credential mappings from WASM tool"
            );
        }

        tracing::info!(name = reg.name, "Registered WASM tool");
        Ok(())
    }

    /// Register a WASM tool from persisted storage.
    #[cfg(feature = "wasm-runtime")]
    pub async fn register_wasm_tool_from_storage<I>(
        &self,
        store: &dyn WasmToolStore,
        runtime: &Arc<WasmToolRuntime>,
        user_id: &str,
        name: &str,
        tool_invoker: Option<Arc<I>>,
        credential_registry: Option<&SharedCredentialRegistry>,
    ) -> Result<(), WasmRegistrationError>
    where
        I: HostToolInvoker + 'static,
    {
        let tool_with_binary = store
            .get_with_binary(user_id, name)
            .await
            .map_err(WasmRegistrationError::Storage)?;

        let capabilities = store
            .get_capabilities(user_id, tool_with_binary.tool.id)
            .await
            .map_err(WasmRegistrationError::Storage)?
            .map(|capabilities| capabilities.to_capabilities())
            .transpose()
            .map_err(WasmRegistrationError::Storage)?
            .unwrap_or_default();

        self.register_wasm_tool(
            WasmToolRegistration {
                name: &tool_with_binary.tool.name,
                wasm_bytes: &tool_with_binary.wasm_binary,
                runtime,
                capabilities,
                limits: None,
                description: Some(&tool_with_binary.tool.description),
                schema: Some(tool_with_binary.tool.parameters_schema.clone()),
                secrets_store: None,
                oauth_refresh: None,
                tool_invoker,
            },
            credential_registry,
        )
        .await
        .map_err(WasmRegistrationError::Wasm)?;

        tracing::info!(
            name = tool_with_binary.tool.name,
            user_id = user_id,
            trust_level = %tool_with_binary.tool.trust_level,
            "Registered WASM tool from storage"
        );

        Ok(())
    }

    /// Unregister a tool.
    pub async fn unregister(&self, name: &str) -> Option<Arc<dyn Tool>> {
        if static_tool_descriptor(name).is_some() {
            return None;
        }
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let removed = entries.remove(name)?;
        self.next_revision();
        self.publish_snapshot_locked(&entries);
        Some(removed.tool)
    }

    /// Remove a dynamic identity only when the caller owns its exact source.
    pub async fn unregister_owned(
        &self,
        name: &str,
        origin: ToolOrigin,
        source_id: &str,
    ) -> Option<Arc<dyn Tool>> {
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let entry = entries.get(name)?;
        if entry.identity.origin != origin || entry.identity.source_id != source_id {
            return None;
        }
        let removed = entries.remove(name)?;
        self.next_revision();
        self.publish_snapshot_locked(&entries);
        Some(removed.tool)
    }

    /// Reconcile a conditionally present static capability. Only the catalog
    /// origin may remove it; its name remains reserved for later re-insertion.
    pub async fn unregister_static(&self, name: &str, origin: ToolOrigin) -> Option<Arc<dyn Tool>> {
        if static_tool_descriptor(name).map(|item| item.origin) != Some(origin) {
            return None;
        }
        let mut entries = self
            .entries
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let entry = entries.get(name)?;
        if entry.identity.origin != origin || entry.identity.source_id != format!("builtin/{name}")
        {
            return None;
        }
        let removed = entries.remove(name)?;
        self.next_revision();
        self.publish_snapshot_locked(&entries);
        Some(removed.tool)
    }

    /// Get a tool by name.
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.entries
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(name)
            .map(|entry| Arc::clone(&entry.tool))
    }

    /// Check if a tool exists.
    pub async fn has(&self, name: &str) -> bool {
        self.entries
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .contains_key(name)
    }

    /// List all tool names.
    pub async fn list(&self) -> Vec<String> {
        let mut names = self
            .entries
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    /// Get the number of registered tools.
    pub fn count(&self) -> usize {
        self.entries
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    /// Get all tools.
    pub async fn all(&self) -> Vec<Arc<dyn Tool>> {
        self.entries
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .map(|entry| Arc::clone(&entry.tool))
            .collect()
    }

    /// Ask every registered tool to release long-lived resources.
    ///
    /// The snapshot avoids holding the registry lock across tool-controlled
    /// awaits. Shutdown is deterministic and bounded per tool so one faulty
    /// extension cannot prevent the rest from draining.
    pub async fn shutdown_all(&self) {
        const TOOL_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

        let mut tools = self
            .entries
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(|(name, entry)| (name.clone(), Arc::clone(&entry.tool)))
            .collect::<Vec<_>>();
        tools.sort_by(|left, right| left.0.cmp(&right.0));

        for (name, tool) in tools {
            match tokio::time::timeout(TOOL_SHUTDOWN_TIMEOUT, tool.shutdown()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(tool = %name, %error, "Tool shutdown failed");
                }
                Err(_) => {
                    tracing::warn!(
                        tool = %name,
                        timeout_secs = TOOL_SHUTDOWN_TIMEOUT.as_secs(),
                        "Tool shutdown timed out"
                    );
                }
            }
        }
    }

    /// Get tool descriptors for internal routing and policy decisions.
    pub async fn tool_descriptors(&self) -> Vec<ToolDescriptor> {
        let mut descriptors = self
            .entries
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .map(|entry| entry.tool.descriptor())
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        descriptors
    }

    /// Get a single tool descriptor by name.
    pub async fn tool_descriptor(&self, name: &str) -> Option<ToolDescriptor> {
        self.entries
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(name)
            .map(|entry| entry.tool.descriptor())
    }

    fn descriptor_to_definition(descriptor: ToolDescriptor) -> ToolDefinition {
        ToolDefinition {
            name: descriptor.name,
            description: descriptor.description,
            parameters: descriptor.parameters,
        }
    }

    /// Get tool definitions for LLM function calling.
    pub async fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tool_descriptors()
            .await
            .into_iter()
            .map(Self::descriptor_to_definition)
            .collect()
    }

    /// Parse an optional string-array allowlist from metadata.
    pub fn metadata_string_list(metadata: &serde_json::Value, key: &str) -> Option<Vec<String>> {
        metadata.get(key).and_then(|value| {
            value.as_array().map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
        })
    }

    /// Check whether a skill is allowed by metadata-scoped capabilities.
    pub fn skill_name_allowed_by_metadata(metadata: &serde_json::Value, skill_name: &str) -> bool {
        match Self::metadata_string_list(metadata, "allowed_skills") {
            Some(allowed_skills) => {
                let allowed: HashSet<&str> = allowed_skills.iter().map(String::as_str).collect();
                allowed.contains(skill_name)
            }
            None => true,
        }
    }

    /// Check whether a tool name is allowed by the provided capability bundle.
    pub fn tool_name_allowed_for_capabilities(
        tool_name: &str,
        allowed_tools: Option<&[String]>,
        allowed_skills: Option<&[String]>,
    ) -> bool {
        if allowed_skills.is_some() && SKILL_ADMIN_TOOLS.contains(&tool_name) {
            return false;
        }

        match allowed_tools {
            Some(allowed_tools) => {
                allowed_tools.iter().any(|name| name == tool_name)
                    || IMPLICIT_CAPABILITY_TOOLS.contains(&tool_name)
            }
            None => true,
        }
    }

    /// Check whether a tool name is allowed by metadata-scoped capabilities.
    pub fn tool_name_allowed_by_metadata(metadata: &serde_json::Value, tool_name: &str) -> bool {
        let allowed_tools = Self::metadata_string_list(metadata, "allowed_tools");
        let allowed_skills = Self::metadata_string_list(metadata, "allowed_skills");
        Self::tool_name_allowed_for_capabilities(
            tool_name,
            allowed_tools.as_deref(),
            allowed_skills.as_deref(),
        )
    }

    fn filter_tool_definitions_for_capabilities(
        defs: Vec<ToolDefinition>,
        allowed_tools: Option<&[String]>,
        allowed_skills: Option<&[String]>,
        visible_hidden_tools: Option<&[String]>,
    ) -> Vec<ToolDefinition> {
        defs.into_iter()
            .filter(|def| {
                Self::tool_name_allowed_for_capabilities(&def.name, allowed_tools, allowed_skills)
                    && Self::tool_name_visible_for_turn(&def.name, visible_hidden_tools)
            })
            .collect()
    }

    /// Filter tool definitions by execution lane/profile metadata in addition to capability grants.
    pub async fn filter_tool_definitions_for_execution_profile(
        &self,
        defs: Vec<ToolDefinition>,
        lane: ToolExecutionLane,
        profile: ToolProfile,
        metadata: &serde_json::Value,
    ) -> Vec<ToolDefinition> {
        let allowed_names: HashSet<String> = self
            .all()
            .await
            .into_iter()
            .filter_map(|tool| {
                let descriptor = tool.descriptor();
                (tool_allowed_for_lane(tool.as_ref(), &descriptor, lane)
                    && descriptor_allowed_for_profile(&descriptor, lane, profile, metadata))
                .then_some(descriptor.name)
            })
            .collect();

        defs.into_iter()
            .filter(|def| allowed_names.contains(&def.name))
            .collect()
    }

    fn tool_name_visible_for_turn(
        tool_name: &str,
        visible_hidden_tools: Option<&[String]>,
    ) -> bool {
        if !HIDDEN_BY_DEFAULT_TOOL_NAMES.contains(&tool_name) {
            return true;
        }

        visible_hidden_tools
            .map(|visible| visible.iter().any(|name| name == tool_name))
            .unwrap_or(false)
    }

    /// Get tool definitions filtered for a routed agent/subagent capability bundle.
    pub async fn tool_definitions_for_capabilities(
        &self,
        allowed_tools: Option<&[String]>,
        allowed_skills: Option<&[String]>,
        visible_hidden_tools: Option<&[String]>,
    ) -> Vec<ToolDefinition> {
        let defs = self.tool_definitions().await;
        Self::filter_tool_definitions_for_capabilities(
            defs,
            allowed_tools,
            allowed_skills,
            visible_hidden_tools,
        )
    }

    /// Get tool definitions filtered for autonomous execution.
    pub async fn tool_definitions_for_autonomous(&self) -> Vec<ToolDefinition> {
        const DISPATCHER_ONLY_TOOLS: &[&str] =
            &["spawn_subagent", "list_subagents", "cancel_subagent"];

        let mut defs = self
            .all()
            .await
            .into_iter()
            .filter(|tool| {
                tool.requires_approval(&serde_json::json!({})) != ApprovalRequirement::Always
                    && !DISPATCHER_ONLY_TOOLS.contains(&tool.name())
            })
            .map(|tool| tool.descriptor())
            .collect::<Vec<_>>();
        defs.sort_by(|left, right| left.name.cmp(&right.name));
        let defs = defs
            .into_iter()
            .map(Self::descriptor_to_definition)
            .collect();
        Self::filter_tool_definitions_for_capabilities(defs, None, None, None)
    }

    /// Get autonomous tool definitions filtered by a capability bundle.
    pub async fn tool_definitions_for_autonomous_capabilities(
        &self,
        allowed_tools: Option<&[String]>,
        allowed_skills: Option<&[String]>,
        visible_hidden_tools: Option<&[String]>,
    ) -> Vec<ToolDefinition> {
        let defs = self.tool_definitions_for_autonomous().await;
        Self::filter_tool_definitions_for_capabilities(
            defs,
            allowed_tools,
            allowed_skills,
            visible_hidden_tools,
        )
    }

    /// Get tool definitions for specific tools.
    pub async fn tool_definitions_for(&self, names: &[&str]) -> Vec<ToolDefinition> {
        let tools = self
            .entries
            .read()
            .unwrap_or_else(|error| error.into_inner());
        names
            .iter()
            .filter_map(|name| tools.get(*name))
            .map(|entry| Self::descriptor_to_definition(entry.tool.descriptor()))
            .collect()
    }

    /// Get tool definitions filtered by domain.
    pub async fn tool_definitions_for_domain(&self, domain: ToolDomain) -> Vec<ToolDefinition> {
        let mut defs = self
            .all()
            .await
            .into_iter()
            .filter(|tool| tool.domain() == domain)
            .map(|tool| Self::descriptor_to_definition(tool.descriptor()))
            .collect::<Vec<_>>();
        defs.sort_by(|left, right| left.name.cmp(&right.name));
        defs
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("count", &self.count())
            .finish()
    }
}

pub fn deny_reason_for_profile(
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
    profile: ToolProfile,
    metadata: &serde_json::Value,
) -> Option<String> {
    if !ToolRegistry::tool_name_allowed_by_metadata(metadata, &descriptor.name) {
        return Some("Tool is not permitted in this agent context".to_string());
    }

    let explicit_tools = ToolRegistry::metadata_string_list(metadata, "allowed_tools");
    if descriptor.is_coordination_tool() {
        return None;
    }

    if let Some(explicit_tools) = explicit_tools {
        if explicit_tools.iter().any(|name| name == &descriptor.name) {
            return None;
        }

        return Some(format!(
            "Tool '{}' is not granted in this delegated context. Add it to allowed_tools or keep this step in the main agent.",
            descriptor.name
        ));
    }

    let implicitly_allowed = match profile {
        ToolProfile::Standard => true,
        ToolProfile::Restricted => descriptor.is_safe_read_only_orchestrator(),
        ToolProfile::ExplicitOnly => false,
        ToolProfile::Acp => descriptor_allowed_for_acp(descriptor),
    };

    if implicitly_allowed {
        None
    } else {
        Some(format!(
            "Tool '{}' is blocked in the {} lane under the '{}' tool profile. Grant it explicitly via allowed_tools or keep this work in the main agent.",
            descriptor.name,
            lane.as_str(),
            profile.as_str()
        ))
    }
}

fn descriptor_allowed_for_acp(descriptor: &ToolDescriptor) -> bool {
    let name = descriptor.name.as_str();
    if descriptor.is_coordination_tool() {
        return true;
    }

    matches!(
        name,
        "read_file"
            | "write_file"
            | "list_dir"
            | "apply_patch"
            | "grep"
            | "search_files"
            | "shell"
            | "process"
            | "execute_code"
            | "session_search"
            | "browser"
            | "vision_analyze"
            | "llm_list_models"
            | "llm_select"
    ) || name.starts_with("memory_")
        || name.starts_with("external_memory_")
        || name.starts_with("skill_")
}

pub fn deny_reason_for_lane(
    tool: &dyn Tool,
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
) -> Option<String> {
    if !matches!(
        lane,
        ToolExecutionLane::Scheduler
            | ToolExecutionLane::Worker
            | ToolExecutionLane::WorkerRuntime
            | ToolExecutionLane::Subagent
    ) {
        return None;
    }

    const DISPATCHER_ONLY_TOOLS: &[&str] = &["spawn_subagent", "list_subagents", "cancel_subagent"];
    if DISPATCHER_ONLY_TOOLS.contains(&descriptor.name.as_str()) {
        return Some(format!(
            "Tool '{}' requires dispatcher interception and is not available in the {} lane.",
            descriptor.name,
            lane.as_str()
        ));
    }

    if tool.requires_approval(&serde_json::json!({})) == ApprovalRequirement::Always {
        return Some(format!(
            "Tool '{}' requires explicit human approval and cannot run in the {} lane.",
            descriptor.name,
            lane.as_str()
        ));
    }

    None
}

/// Check whether a tool descriptor is usable for the given lane/profile/metadata tuple.
pub fn descriptor_allowed_for_profile(
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
    profile: ToolProfile,
    metadata: &serde_json::Value,
) -> bool {
    deny_reason_for_profile(descriptor, lane, profile, metadata).is_none()
}

/// Check whether a concrete tool may be exposed/executed in the given lane at all.
pub fn tool_allowed_for_lane(
    tool: &dyn Tool,
    descriptor: &ToolDescriptor,
    lane: ToolExecutionLane,
) -> bool {
    deny_reason_for_lane(tool, descriptor, lane).is_none()
}

/// Error when registering a WASM tool from storage.
#[cfg(feature = "wasm-runtime")]
#[derive(Debug, thiserror::Error)]
pub enum WasmRegistrationError {
    #[error("Storage error: {0}")]
    Storage(#[from] WasmStorageError),

    #[error("WASM error: {0}")]
    Wasm(#[from] WasmError),
}

/// Configuration for registering a WASM tool with the extracted registry.
#[cfg(feature = "wasm-runtime")]
pub struct WasmToolRegistration<'a, I: HostToolInvoker> {
    /// Unique name for the tool.
    pub name: &'a str,
    /// Raw WASM component bytes.
    pub wasm_bytes: &'a [u8],
    /// WASM runtime for compilation and execution.
    pub runtime: &'a Arc<WasmToolRuntime>,
    /// Security capabilities to grant the tool.
    pub capabilities: Capabilities,
    /// Optional resource limits (uses runtime defaults if omitted).
    pub limits: Option<ResourceLimits>,
    /// Optional description override.
    pub description: Option<&'a str>,
    /// Optional parameter schema override.
    pub schema: Option<serde_json::Value>,
    /// Secrets store for credential injection at request time.
    pub secrets_store: Option<Arc<dyn SecretsStore + Send + Sync>>,
    /// OAuth refresh configuration for auto-refreshing expired tokens.
    pub oauth_refresh: Option<OAuthRefreshConfig>,
    /// Optional host-mediated bridge for WASM tool_invoke aliases.
    pub tool_invoker: Option<Arc<I>>,
}

#[cfg(test)]
#[path = "registry_registration_tests.rs"]
mod registration_tests;
