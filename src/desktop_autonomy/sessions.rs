use super::*;

#[derive(Debug)]
pub struct DesktopSessionManager {
    max_concurrent_jobs: usize,
    sessions: Mutex<HashMap<String, Arc<Semaphore>>>,
}

impl DesktopSessionManager {
    pub fn new(max_concurrent_jobs: usize) -> Self {
        Self {
            max_concurrent_jobs: max_concurrent_jobs.max(1),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub async fn acquire(&self, session_id: &str) -> Result<DesktopSessionLease, String> {
        let semaphore = {
            let mut guard = self.sessions.lock().await;
            guard
                .entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Semaphore::new(self.max_concurrent_jobs)))
                .clone()
        };
        let permit = semaphore
            .acquire_owned()
            .await
            .map_err(|_| "desktop session manager is shutting down".to_string())?;
        Ok(DesktopSessionLease {
            session_id: session_id.to_string(),
            _permit: permit,
        })
    }
}

#[derive(Debug)]
pub struct DesktopSessionLease {
    session_id: String,
    _permit: OwnedSemaphorePermit,
}

impl DesktopSessionLease {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}
