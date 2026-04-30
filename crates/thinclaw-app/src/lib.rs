//! Application assembly helpers.
//!
//! This crate owns root-independent startup policy and process wiring. The
//! root crate keeps adapters for assembly steps that still depend on root-only
//! subsystems while those subsystems are being extracted.

pub mod assembly;
pub mod runtime;

pub use assembly::{
    DescribesRuntimeAssembly, LocalRuntimeChannel, RuntimeAssemblyPlan, RuntimeAssemblyRequirement,
    RuntimeDependencyPlan, RuntimeDependencyProvider, RuntimeEntrypointPlan, RuntimeLifecyclePhase,
    RuntimeLifecyclePolicy, RuntimeWorkspaceMode, ToolRuntimeAssemblyInput,
    ToolRuntimeAssemblyPlan, WorkspaceDirectoryPlan, WorkspaceFilesystemScope,
};
pub use runtime::{
    AppBuilderFlags, QuietStartupSpinner, RuntimeEntryMode, RuntimeExecRegistrationMode,
    block_on_async_main, desktop_autonomy_headless_blocker, desktop_autonomy_headless_blocker_for,
    execute_code_registration_mode, init_cli_tracing, process_registration_mode,
    relaunch_current_process, restart_is_managed_by_service, should_show_quiet_startup_spinner,
};
