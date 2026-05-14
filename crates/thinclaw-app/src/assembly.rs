//! Root-independent startup assembly plans.
//!
//! The concrete application builder still lives in the root crate while
//! subsystems are being extracted. This module provides plain data contracts
//! for the decisions that can already be made without depending on those root
//! types: lifecycle policy, local entrypoint selection, workspace-scoped tool
//! registration, and dependency injection expectations.

use std::path::PathBuf;

use crate::runtime::{
    AppBuilderFlags, RuntimeEntryMode, RuntimeExecRegistrationMode, execute_code_registration_mode,
    process_registration_mode,
};

/// A startup phase that a concrete runtime builder can execute or skip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeLifecyclePhase {
    Database,
    Secrets,
    LlmRuntime,
    CoreTools,
    Extensions,
    UserTools,
    Hooks,
}

/// Whether an assembly phase or dependency should be constructed in-process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeAssemblyRequirement {
    /// The root-independent plan expects the concrete builder to construct it.
    Required,
    /// The dependency may be absent, but the concrete builder should wire it
    /// when available.
    Optional,
    /// A host runtime is expected to inject the dependency before assembly.
    External,
    /// The phase or dependency is intentionally disabled for this run.
    Disabled,
}

impl RuntimeAssemblyRequirement {
    /// Returns true when a concrete builder should run local construction.
    pub const fn should_build(self) -> bool {
        matches!(self, Self::Required | Self::Optional)
    }
}

/// Lifecycle policy for the mechanical application startup phases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLifecyclePolicy {
    pub database: RuntimeAssemblyRequirement,
    pub secrets: RuntimeAssemblyRequirement,
    pub llm_runtime: RuntimeAssemblyRequirement,
    pub core_tools: RuntimeAssemblyRequirement,
    pub extensions: RuntimeAssemblyRequirement,
    pub user_tools: RuntimeAssemblyRequirement,
    pub hooks: RuntimeAssemblyRequirement,
}

impl RuntimeLifecyclePolicy {
    /// Builds the default lifecycle policy from app builder flags.
    pub fn from_flags(flags: AppBuilderFlags) -> Self {
        let database = if flags.no_db {
            RuntimeAssemblyRequirement::Disabled
        } else {
            RuntimeAssemblyRequirement::Required
        };

        Self {
            database,
            secrets: RuntimeAssemblyRequirement::Optional,
            llm_runtime: RuntimeAssemblyRequirement::Required,
            core_tools: RuntimeAssemblyRequirement::Required,
            extensions: RuntimeAssemblyRequirement::Optional,
            user_tools: RuntimeAssemblyRequirement::Optional,
            hooks: RuntimeAssemblyRequirement::Optional,
        }
    }

    /// Returns the ordered phases that should be considered by a builder.
    pub fn enabled_phases(&self) -> Vec<RuntimeLifecyclePhase> {
        [
            (RuntimeLifecyclePhase::Database, self.database),
            (RuntimeLifecyclePhase::Secrets, self.secrets),
            (RuntimeLifecyclePhase::LlmRuntime, self.llm_runtime),
            (RuntimeLifecyclePhase::CoreTools, self.core_tools),
            (RuntimeLifecyclePhase::Extensions, self.extensions),
            (RuntimeLifecyclePhase::UserTools, self.user_tools),
            (RuntimeLifecyclePhase::Hooks, self.hooks),
        ]
        .into_iter()
        .filter_map(|(phase, requirement)| requirement.should_build().then_some(phase))
        .collect()
    }
}

impl Default for RuntimeLifecyclePolicy {
    fn default() -> Self {
        Self::from_flags(AppBuilderFlags::default())
    }
}

/// Local interactive runtime selected by the entrypoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalRuntimeChannel {
    Repl,
    Tui,
    SingleMessage,
}

/// Entrypoint policy that can be computed before root channel types exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RuntimeEntrypointPlan {
    pub mode: RuntimeEntryMode,
    pub local_runtime_requested: bool,
    pub local_channel: Option<LocalRuntimeChannel>,
}

impl RuntimeEntrypointPlan {
    /// Computes the local entrypoint selected by CLI/runtime mode.
    pub fn new(mode: RuntimeEntryMode, cli_channel_enabled: bool, single_message: bool) -> Self {
        let local_channel = if single_message {
            Some(LocalRuntimeChannel::SingleMessage)
        } else {
            match mode {
                RuntimeEntryMode::Cli => Some(LocalRuntimeChannel::Repl),
                RuntimeEntryMode::Tui => Some(LocalRuntimeChannel::Tui),
                RuntimeEntryMode::Default if cli_channel_enabled => Some(LocalRuntimeChannel::Repl),
                RuntimeEntryMode::Default => None,
            }
        };

        Self {
            mode,
            local_runtime_requested: local_channel.is_some(),
            local_channel,
        }
    }
}

/// Root-independent inputs for deciding which native channels should be
/// registered by a concrete runtime adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NativeChannelActivationInput {
    pub cli_only: bool,
    pub signal_configured: bool,
    pub nostr_configured: bool,
    pub discord_configured: bool,
    pub imessage_configured: bool,
    pub apple_mail_configured: bool,
    pub bluebubbles_configured: bool,
    pub gmail_configured: bool,
    pub http_configured: bool,
    pub gateway_configured: bool,
    pub wasm_channels_enabled: bool,
    pub wasm_channels_dir_exists: bool,
}

/// Native channel registration decisions that do not depend on root channel
/// implementation types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeChannelActivationPlan {
    pub signal: bool,
    pub nostr: bool,
    pub discord: bool,
    pub imessage: bool,
    pub apple_mail: bool,
    pub bluebubbles: bool,
    pub gmail: bool,
    pub http: bool,
    pub gateway: bool,
    pub wasm_channels: bool,
}

impl NativeChannelActivationPlan {
    pub const fn from_input(input: NativeChannelActivationInput) -> Self {
        let native_enabled = !input.cli_only;
        Self {
            signal: native_enabled && input.signal_configured,
            nostr: native_enabled && input.nostr_configured,
            discord: native_enabled && input.discord_configured,
            imessage: native_enabled && input.imessage_configured,
            apple_mail: native_enabled && input.apple_mail_configured,
            bluebubbles: native_enabled && input.bluebubbles_configured,
            gmail: native_enabled && input.gmail_configured,
            http: native_enabled && input.http_configured,
            gateway: input.gateway_configured,
            wasm_channels: input.wasm_channels_enabled && input.wasm_channels_dir_exists,
        }
    }
}

/// Root-independent workspace mode names understood by the app runtime.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub enum RuntimeWorkspaceMode {
    Sandboxed,
    Project,
    #[default]
    Unrestricted,
    Custom(String),
}

impl RuntimeWorkspaceMode {
    /// Parses the string mode currently stored in configuration.
    pub fn from_config_value(value: impl AsRef<str>) -> Self {
        match value.as_ref() {
            "sandboxed" => Self::Sandboxed,
            "project" => Self::Project,
            "unrestricted" | "" => Self::Unrestricted,
            other => Self::Custom(other.to_string()),
        }
    }

    /// Returns the mode value expected by existing registration policy helpers.
    pub fn as_config_value(&self) -> &str {
        match self {
            Self::Sandboxed => "sandboxed",
            Self::Project => "project",
            Self::Unrestricted => "unrestricted",
            Self::Custom(value) => value.as_str(),
        }
    }
}

/// Filesystem scope implied by a workspace plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceFilesystemScope {
    WorkspaceRoot,
    WorkingDirectory,
    FullFilesystem,
}

/// Directory DTO for tools whose root crate implementations need workspace
/// paths but should not decide policy themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceDirectoryPlan {
    pub base_dir: Option<PathBuf>,
    pub working_dir: Option<PathBuf>,
    pub create_dir: Option<PathBuf>,
    pub scope: WorkspaceFilesystemScope,
}

impl WorkspaceDirectoryPlan {
    /// Computes the base/working directory pair for a workspace mode.
    pub fn for_mode(
        mode: &RuntimeWorkspaceMode,
        workspace_root: Option<PathBuf>,
        default_sandbox_workspace: PathBuf,
        default_project_workspace: PathBuf,
    ) -> Self {
        match mode {
            RuntimeWorkspaceMode::Sandboxed => {
                let dir = workspace_root.unwrap_or(default_sandbox_workspace);
                Self {
                    base_dir: Some(dir.clone()),
                    working_dir: Some(dir.clone()),
                    create_dir: Some(dir),
                    scope: WorkspaceFilesystemScope::WorkspaceRoot,
                }
            }
            RuntimeWorkspaceMode::Project => {
                let dir = workspace_root.unwrap_or(default_project_workspace);
                Self {
                    base_dir: None,
                    working_dir: Some(dir.clone()),
                    create_dir: Some(dir),
                    scope: WorkspaceFilesystemScope::WorkingDirectory,
                }
            }
            RuntimeWorkspaceMode::Unrestricted | RuntimeWorkspaceMode::Custom(_) => Self {
                base_dir: None,
                working_dir: None,
                create_dir: None,
                scope: WorkspaceFilesystemScope::FullFilesystem,
            },
        }
    }

    /// Computes the user-tool execution directories for a workspace mode.
    ///
    /// User tools use the same base/working directory semantics as runtime
    /// workspace-aware tools, but preserve the historical behavior of only
    /// creating the sandbox workspace directory eagerly.
    pub fn for_user_tools(
        mode: &RuntimeWorkspaceMode,
        workspace_root: Option<PathBuf>,
        default_sandbox_workspace: PathBuf,
        default_project_workspace: PathBuf,
    ) -> Self {
        let mut plan = Self::for_mode(
            mode,
            workspace_root,
            default_sandbox_workspace,
            default_project_workspace,
        );
        if !matches!(mode, RuntimeWorkspaceMode::Sandboxed) {
            plan.create_dir = None;
        }
        plan
    }
}

/// Inputs needed to compute tool registration plans without root config types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRuntimeAssemblyInput {
    pub workspace_mode: RuntimeWorkspaceMode,
    pub workspace_root: Option<PathBuf>,
    pub sandbox_enabled: bool,
    pub allow_local_tools: bool,
    pub builder_enabled: bool,
    pub default_sandbox_workspace: PathBuf,
    pub default_project_workspace: PathBuf,
}

/// Registration plan for runtime tools whose implementation still lives in
/// root-owned modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRuntimeAssemblyPlan {
    pub workspace_mode: RuntimeWorkspaceMode,
    pub local_tools_enabled: bool,
    pub process_registration: RuntimeExecRegistrationMode,
    pub execute_code_registration: RuntimeExecRegistrationMode,
    pub dev_tools_workspace: Option<WorkspaceDirectoryPlan>,
    pub builder_workspace: Option<WorkspaceDirectoryPlan>,
    pub search_files_workspace: Option<WorkspaceDirectoryPlan>,
    pub user_tools_workspace: WorkspaceDirectoryPlan,
}

impl ToolRuntimeAssemblyPlan {
    /// Computes root-independent tool registration policy.
    pub fn from_input(input: ToolRuntimeAssemblyInput) -> Self {
        let mode_value = input.workspace_mode.as_config_value();
        let directory_plan = || {
            WorkspaceDirectoryPlan::for_mode(
                &input.workspace_mode,
                input.workspace_root.clone(),
                input.default_sandbox_workspace.clone(),
                input.default_project_workspace.clone(),
            )
        };

        let process_registration = if input.allow_local_tools {
            process_registration_mode(mode_value)
        } else {
            RuntimeExecRegistrationMode::Disabled
        };

        let execute_code_registration = if input.allow_local_tools {
            execute_code_registration_mode(mode_value, input.sandbox_enabled)
        } else {
            RuntimeExecRegistrationMode::Disabled
        };

        let builder_workspace = (input.builder_enabled
            && (input.allow_local_tools || !input.sandbox_enabled))
            .then(directory_plan);
        let dev_tools_workspace =
            (input.allow_local_tools && builder_workspace.is_none()).then(directory_plan);
        let search_files_workspace = input.allow_local_tools.then(directory_plan);
        let user_tools_workspace = WorkspaceDirectoryPlan::for_user_tools(
            &input.workspace_mode,
            input.workspace_root.clone(),
            input.default_sandbox_workspace.clone(),
            input.default_project_workspace.clone(),
        );

        Self {
            workspace_mode: input.workspace_mode,
            local_tools_enabled: input.allow_local_tools,
            process_registration,
            execute_code_registration,
            dev_tools_workspace,
            builder_workspace,
            search_files_workspace,
            user_tools_workspace,
        }
    }
}

/// Dependency contract for later root builder extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDependencyPlan {
    pub config: RuntimeAssemblyRequirement,
    pub log_broadcaster: RuntimeAssemblyRequirement,
    pub database: RuntimeAssemblyRequirement,
    pub secrets_store: RuntimeAssemblyRequirement,
    pub providers_settings: RuntimeAssemblyRequirement,
    pub hardware_bridge: RuntimeAssemblyRequirement,
}

impl RuntimeDependencyPlan {
    pub fn from_lifecycle(policy: &RuntimeLifecyclePolicy) -> Self {
        Self {
            config: RuntimeAssemblyRequirement::External,
            log_broadcaster: RuntimeAssemblyRequirement::External,
            database: policy.database,
            secrets_store: policy.secrets,
            providers_settings: RuntimeAssemblyRequirement::Optional,
            hardware_bridge: RuntimeAssemblyRequirement::Optional,
        }
    }
}

/// Full root-independent runtime plan assembled from smaller DTOs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAssemblyPlan {
    pub entrypoint: RuntimeEntrypointPlan,
    pub lifecycle: RuntimeLifecyclePolicy,
    pub dependencies: RuntimeDependencyPlan,
    pub tools: ToolRuntimeAssemblyPlan,
}

impl RuntimeAssemblyPlan {
    /// Creates a complete assembly plan from already-decoupled inputs.
    pub fn new(
        entrypoint: RuntimeEntrypointPlan,
        lifecycle: RuntimeLifecyclePolicy,
        tools: ToolRuntimeAssemblyPlan,
    ) -> Self {
        let dependencies = RuntimeDependencyPlan::from_lifecycle(&lifecycle);
        Self {
            entrypoint,
            lifecycle,
            dependencies,
            tools,
        }
    }
}

/// Trait for concrete builders that can expose their root-independent plan.
pub trait DescribesRuntimeAssembly {
    fn runtime_assembly_plan(&self) -> &RuntimeAssemblyPlan;
}

/// Trait for adapters that provide an externally assembled dependency.
pub trait RuntimeDependencyProvider<Dependency> {
    type Error;

    fn provide_dependency(&self, plan: &RuntimeAssemblyPlan) -> Result<Dependency, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_input(mode: RuntimeWorkspaceMode) -> ToolRuntimeAssemblyInput {
        ToolRuntimeAssemblyInput {
            workspace_mode: mode,
            workspace_root: None,
            sandbox_enabled: true,
            allow_local_tools: true,
            builder_enabled: false,
            default_sandbox_workspace: PathBuf::from("/tmp/thinclaw-sandbox"),
            default_project_workspace: PathBuf::from("/tmp/thinclaw-project"),
        }
    }

    #[test]
    fn lifecycle_policy_skips_database_when_no_db_flag_is_set() {
        let policy = RuntimeLifecyclePolicy::from_flags(AppBuilderFlags { no_db: true });

        assert_eq!(policy.database, RuntimeAssemblyRequirement::Disabled);
        assert!(
            !policy
                .enabled_phases()
                .contains(&RuntimeLifecyclePhase::Database)
        );
        assert!(
            policy
                .enabled_phases()
                .contains(&RuntimeLifecyclePhase::LlmRuntime)
        );
    }

    #[test]
    fn entrypoint_plan_selects_local_channel() {
        assert_eq!(
            RuntimeEntrypointPlan::new(RuntimeEntryMode::Default, true, false).local_channel,
            Some(LocalRuntimeChannel::Repl)
        );
        assert_eq!(
            RuntimeEntrypointPlan::new(RuntimeEntryMode::Default, false, false).local_channel,
            None
        );
        assert_eq!(
            RuntimeEntrypointPlan::new(RuntimeEntryMode::Tui, false, false).local_channel,
            Some(LocalRuntimeChannel::Tui)
        );
        assert_eq!(
            RuntimeEntrypointPlan::new(RuntimeEntryMode::Default, false, true).local_channel,
            Some(LocalRuntimeChannel::SingleMessage)
        );
    }

    #[test]
    fn sandboxed_tool_plan_uses_isolated_execution_and_workspace_root() {
        let plan = ToolRuntimeAssemblyPlan::from_input(tool_input(RuntimeWorkspaceMode::Sandboxed));

        assert_eq!(
            plan.process_registration,
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            plan.execute_code_registration,
            RuntimeExecRegistrationMode::DockerSandbox
        );
        let search_workspace = plan.search_files_workspace.expect("search workspace");
        assert_eq!(
            search_workspace.scope,
            WorkspaceFilesystemScope::WorkspaceRoot
        );
        assert_eq!(
            search_workspace.base_dir,
            Some(PathBuf::from("/tmp/thinclaw-sandbox"))
        );
    }

    #[test]
    fn project_tool_plan_uses_working_directory_without_base_dir() {
        let mut input = tool_input(RuntimeWorkspaceMode::Project);
        input.workspace_root = Some(PathBuf::from("/workspace/project"));

        let plan = ToolRuntimeAssemblyPlan::from_input(input);

        assert_eq!(
            plan.process_registration,
            RuntimeExecRegistrationMode::Disabled
        );
        assert_eq!(
            plan.execute_code_registration,
            RuntimeExecRegistrationMode::Disabled
        );
        let dev_workspace = plan.dev_tools_workspace.expect("dev workspace");
        assert_eq!(dev_workspace.base_dir, None);
        assert_eq!(
            dev_workspace.working_dir,
            Some(PathBuf::from("/workspace/project"))
        );
        assert_eq!(plan.user_tools_workspace.base_dir, None);
        assert_eq!(
            plan.user_tools_workspace.working_dir,
            Some(PathBuf::from("/workspace/project"))
        );
        assert_eq!(plan.user_tools_workspace.create_dir, None);
    }

    #[test]
    fn unrestricted_tool_plan_keeps_local_registration_available() {
        let plan =
            ToolRuntimeAssemblyPlan::from_input(tool_input(RuntimeWorkspaceMode::Unrestricted));

        assert_eq!(
            plan.process_registration,
            RuntimeExecRegistrationMode::LocalHost
        );
        assert_eq!(
            plan.execute_code_registration,
            RuntimeExecRegistrationMode::LocalHost
        );
        assert_eq!(
            plan.search_files_workspace.expect("search workspace").scope,
            WorkspaceFilesystemScope::FullFilesystem
        );
        assert_eq!(plan.user_tools_workspace.base_dir, None);
        assert_eq!(plan.user_tools_workspace.working_dir, None);
    }

    #[test]
    fn user_tool_workspace_preserves_sandbox_creation_policy() {
        let plan = ToolRuntimeAssemblyPlan::from_input(tool_input(RuntimeWorkspaceMode::Sandboxed));

        assert_eq!(
            plan.user_tools_workspace.base_dir,
            Some(PathBuf::from("/tmp/thinclaw-sandbox"))
        );
        assert_eq!(
            plan.user_tools_workspace.working_dir,
            Some(PathBuf::from("/tmp/thinclaw-sandbox"))
        );
        assert_eq!(
            plan.user_tools_workspace.create_dir,
            Some(PathBuf::from("/tmp/thinclaw-sandbox"))
        );
    }

    #[test]
    fn builder_workspace_suppresses_duplicate_dev_tool_registration() {
        let mut input = tool_input(RuntimeWorkspaceMode::Project);
        input.builder_enabled = true;

        let plan = ToolRuntimeAssemblyPlan::from_input(input);

        assert!(plan.builder_workspace.is_some());
        assert!(plan.dev_tools_workspace.is_none());
    }

    #[test]
    fn runtime_assembly_plan_derives_dependency_policy() {
        let lifecycle = RuntimeLifecyclePolicy::from_flags(AppBuilderFlags { no_db: true });
        let tools =
            ToolRuntimeAssemblyPlan::from_input(tool_input(RuntimeWorkspaceMode::Unrestricted));
        let plan = RuntimeAssemblyPlan::new(
            RuntimeEntrypointPlan::new(RuntimeEntryMode::Cli, false, false),
            lifecycle,
            tools,
        );

        assert_eq!(
            plan.dependencies.config,
            RuntimeAssemblyRequirement::External
        );
        assert_eq!(
            plan.dependencies.database,
            RuntimeAssemblyRequirement::Disabled
        );
        assert_eq!(
            plan.dependencies.log_broadcaster,
            RuntimeAssemblyRequirement::External
        );
    }

    #[test]
    fn native_channel_plan_disables_external_channels_for_cli_only() {
        let plan = NativeChannelActivationPlan::from_input(NativeChannelActivationInput {
            cli_only: true,
            signal_configured: true,
            gateway_configured: true,
            wasm_channels_enabled: true,
            wasm_channels_dir_exists: true,
            ..NativeChannelActivationInput::default()
        });

        assert!(!plan.signal);
        assert!(plan.gateway);
        assert!(plan.wasm_channels);
    }
}
