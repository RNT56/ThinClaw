use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use sysinfo::{Pid, ProcessesToUpdate, System};

const MAX_TRACKER_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ChildProcess {
    pub pid: u32,
    /// Human-facing process label supplied by the launcher.
    pub name: String,
    pub service: String,
    /// Process metadata captured after spawn. Old tracker files lack these
    /// fields and are deliberately never killed, because a bare PID/name pair
    /// is unsafe after PID reuse.
    #[serde(default)]
    pub observed_name: Option<String>,
    #[serde(default)]
    pub executable: Option<PathBuf>,
    #[serde(default)]
    pub start_time: Option<u64>,
}

pub struct ProcessTracker {
    pub file_path: PathBuf,
    processes: Mutex<Vec<ChildProcess>>,
}

impl ProcessTracker {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let file_path = app_data_dir.join("child_processes.json");
        let processes = load_tracker(&file_path).unwrap_or_else(|error| {
            tracing::error!("[ProcessTracker] Ignoring unsafe tracker state: {error}");
            Vec::new()
        });
        Self {
            file_path,
            processes: Mutex::new(processes),
        }
    }

    fn persist(&self, processes: &[ChildProcess]) -> Result<(), String> {
        let json = serde_json::to_string_pretty(processes)
            .map_err(|error| format!("failed to encode child-process tracker: {error}"))?;
        crate::config::write_config_file(&self.file_path, &json)
    }

    pub fn add_pid(&self, pid: u32, name: &str, service: &str) {
        if pid == 0 {
            tracing::error!("[ProcessTracker] Refusing to track PID 0");
            return;
        }
        let identity = capture_process_identity(pid);
        if identity.start_time.is_none() {
            tracing::error!(
                "[ProcessTracker] Could not capture a stable identity for PID={pid}; crash recovery will not kill it by PID alone"
            );
        }
        let record = ChildProcess {
            pid,
            name: name.to_string(),
            service: service.to_string(),
            observed_name: identity.observed_name,
            executable: identity.executable,
            start_time: identity.start_time,
        };

        let mut guard = self
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard.retain(|process| process.pid != pid);
        guard.push(record);
        if let Err(error) = self.persist(&guard) {
            tracing::error!("[ProcessTracker] Failed to persist PID={pid}: {error}");
        }
    }

    pub fn remove_pid(&self, pid: u32) {
        let mut guard = self
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard.retain(|process| process.pid != pid);
        if let Err(error) = self.persist(&guard) {
            tracing::error!("[ProcessTracker] Failed to persist PID removal: {error}");
        }
    }

    fn kill_processes(&self, records: &[ChildProcess]) -> HashSet<ProcessIdentity> {
        let mut system = System::new_all();
        system.refresh_processes(ProcessesToUpdate::All, true);
        let mut completed = HashSet::new();

        for record in records {
            let identity = ProcessIdentity::from_record(record);
            let Some(expected_start_time) = record.start_time else {
                tracing::warn!(
                    "[ProcessTracker] Dropping legacy PID={} record without killing it; no start-time identity was persisted",
                    record.pid
                );
                completed.insert(identity);
                continue;
            };
            let sys_pid = Pid::from_u32(record.pid);
            let Some(process) = system.process(sys_pid) else {
                tracing::info!("[ProcessTracker] PID={} is already gone", record.pid);
                completed.insert(identity);
                continue;
            };

            let observed_name = process.name().to_string_lossy();
            let name_matches = record
                .observed_name
                .as_deref()
                .is_some_and(|expected| expected.eq_ignore_ascii_case(&observed_name));
            let executable_matches = match (&record.executable, process.exe()) {
                (Some(expected), Some(actual)) => expected == actual,
                (None, _) => true,
                (Some(_), None) => false,
            };
            if process.start_time() != expected_start_time || !name_matches || !executable_matches {
                tracing::warn!(
                    "[ProcessTracker] Refusing to kill reused or changed PID={} (label={})",
                    record.pid,
                    record.name
                );
                completed.insert(identity);
                continue;
            }

            // Descendants are killed first while the validated parent relation
            // is still present in this process snapshot.
            let descendants = descendant_pids(&system, sys_pid);
            for descendant in descendants.into_iter().rev() {
                if let Some(child) = system.process(descendant) {
                    let _ = child.kill();
                }
            }
            if process.kill() {
                tracing::info!(
                    "[ProcessTracker] Killed orphaned PID={} ({})",
                    record.pid,
                    observed_name
                );
                completed.insert(identity);
            } else {
                tracing::error!(
                    "[ProcessTracker] Failed to kill validated orphan PID={}",
                    record.pid
                );
            }
        }
        completed
    }

    pub fn cleanup_all(&self) {
        self.cleanup_matching(|_| true);
    }

    pub fn cleanup_by_service(&self, service_name: &str) {
        self.cleanup_matching(|process| process.service == service_name);
    }

    fn cleanup_matching(&self, predicate: impl Fn(&ChildProcess) -> bool) {
        let records = {
            let guard = self
                .processes
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            guard
                .iter()
                .filter(|process| predicate(process))
                .cloned()
                .collect::<Vec<_>>()
        };
        let completed = self.kill_processes(&records);
        let mut guard = self
            .processes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard.retain(|process| !completed.contains(&ProcessIdentity::from_record(process)));
        if let Err(error) = self.persist(&guard) {
            tracing::error!("[ProcessTracker] Failed to persist cleanup result: {error}");
        }
    }
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct ProcessIdentity {
    pid: u32,
    start_time: Option<u64>,
}

impl ProcessIdentity {
    fn from_record(record: &ChildProcess) -> Self {
        Self {
            pid: record.pid,
            start_time: record.start_time,
        }
    }
}

#[derive(Default)]
struct CapturedIdentity {
    observed_name: Option<String>,
    executable: Option<PathBuf>,
    start_time: Option<u64>,
}

fn capture_process_identity(pid: u32) -> CapturedIdentity {
    let mut system = System::new();
    let sys_pid = Pid::from_u32(pid);
    system.refresh_processes(ProcessesToUpdate::Some(&[sys_pid]), true);
    system
        .process(sys_pid)
        .map(|process| CapturedIdentity {
            observed_name: Some(process.name().to_string_lossy().into_owned()),
            executable: process.exe().map(PathBuf::from),
            start_time: Some(process.start_time()),
        })
        .unwrap_or_default()
}

fn descendant_pids(system: &System, parent: Pid) -> Vec<Pid> {
    let mut descendants = Vec::new();
    let mut frontier = vec![parent];
    while let Some(current) = frontier.pop() {
        for (pid, process) in system.processes() {
            if process.parent() == Some(current) && !descendants.contains(pid) {
                descendants.push(*pid);
                frontier.push(*pid);
            }
        }
    }
    descendants
}

fn load_tracker(path: &std::path::Path) -> Result<Vec<ChildProcess>, String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("failed to inspect tracker: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("tracker path is not a regular file".to_string());
    }
    if metadata.len() > MAX_TRACKER_BYTES {
        return Err(format!(
            "tracker exceeds the {MAX_TRACKER_BYTES}-byte limit"
        ));
    }
    let file =
        std::fs::File::open(path).map_err(|error| format!("failed to open tracker: {error}"))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_TRACKER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read tracker: {error}"))?;
    if bytes.len() as u64 > MAX_TRACKER_BYTES {
        return Err(format!(
            "tracker exceeds the {MAX_TRACKER_BYTES}-byte limit"
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| format!("invalid tracker JSON: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_records_deserialize_without_unsafe_identity() {
        let record: ChildProcess =
            serde_json::from_str(r#"{"pid":42,"name":"llama-server","service":"chat"}"#).unwrap();
        assert_eq!(record.start_time, None);
        assert_eq!(record.executable, None);
        assert_eq!(record.observed_name, None);
    }

    #[test]
    fn descendant_search_finds_current_test_child_tree_shape() {
        // The algorithm's empty-system base case must remain finite and exact.
        let system = System::new();
        assert!(descendant_pids(&system, Pid::from_u32(1)).is_empty());
    }
}
