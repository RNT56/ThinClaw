//! Application assembly helpers.
//!
//! This crate owns root-independent startup policy and process wiring. The
//! root crate keeps adapters for assembly steps that still depend on root-only
//! subsystems while those subsystems are being extracted.

pub mod acp;
pub mod assembly;
pub mod model;
pub mod runtime;

pub use acp::{AcpRuntimeConfigInput, AcpRuntimeConfigPlan, MainToolProfilePlan};
pub use assembly::{
    DescribesRuntimeAssembly, LocalRuntimeChannel, NativeChannelActivationInput,
    NativeChannelActivationPlan, RuntimeAssemblyPlan, RuntimeAssemblyRequirement,
    RuntimeDependencyPlan, RuntimeDependencyProvider, RuntimeEntrypointPlan, RuntimeLifecyclePhase,
    RuntimeLifecyclePolicy, RuntimeWorkspaceMode, ToolRuntimeAssemblyInput,
    ToolRuntimeAssemblyPlan, WorkspaceDirectoryPlan, WorkspaceFilesystemScope,
};
pub use model::{apply_model_override, overridden_model_for_backend};
pub use runtime::{
    AppBuilderFlags, PeriodicPersistencePlan, QuietStartupSpinner, RuntimeCommandIntent,
    RuntimeEntryMode, RuntimeEnvBootstrapPlan, RuntimeExecRegistrationMode, RuntimeShutdownAction,
    RuntimeShutdownPlan, block_on_async_main, desktop_autonomy_headless_blocker,
    desktop_autonomy_headless_blocker_for, execute_code_registration_mode, init_cli_tracing,
    process_registration_mode, relaunch_current_process, restart_is_managed_by_service,
    run_async_entrypoint, should_show_quiet_startup_spinner,
};
