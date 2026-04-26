use super::*;

pub fn install_global_manager(manager: Option<Arc<DesktopAutonomyManager>>) {
    match GLOBAL_MANAGER.write() {
        Ok(mut guard) => *guard = manager,
        Err(poisoned) => *poisoned.into_inner() = manager,
    }
}

pub fn desktop_autonomy_manager() -> Option<Arc<DesktopAutonomyManager>> {
    match GLOBAL_MANAGER.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub fn reckless_desktop_active() -> bool {
    desktop_autonomy_manager()
        .as_ref()
        .is_some_and(|manager| manager.is_reckless_enabled())
}
