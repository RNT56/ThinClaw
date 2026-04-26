use super::*;

pub struct MemoryProviderManager {
    pub(in crate::agent::learning) store: Arc<dyn Database>,
    pub(in crate::agent::learning) providers: Vec<Arc<dyn MemoryProvider>>,
}
