use super::*;

pub struct DesktopAutonomyManager {
    pub(super) config: DesktopAutonomyConfig,
    pub(super) database_config: Option<DatabaseConfig>,
    pub(super) store: Option<Arc<dyn Database>>,
    pub(super) session_manager: DesktopSessionManager,
    pub(super) state_root: PathBuf,
    pub(super) sidecar_script_path: PathBuf,
    pub(super) runtime_state: RwLock<RuntimeState>,
}
