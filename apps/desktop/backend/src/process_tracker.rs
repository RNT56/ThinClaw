use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use sysinfo::{Pid, ProcessesToUpdate, System};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChildProcess {
    pub pid: u32,
    pub name: String,
    pub service: String, // "chat" or "embedding"
}

pub struct ProcessTracker {
    pub file_path: PathBuf,
    // Provide in-memory cache if needed, but file is source of truth for crash recovery
    // But since multiple threads might access, let's just lock file access logic via mutex if needed,
    // or just rely on OS atomicity for now (simplest is loads/saves whole file).
    // Actually, simple Mutex<Vec> wrapper is better for runtime performance,
    // and we sync to disk on change.
    processes: Mutex<Vec<ChildProcess>>,
}

impl ProcessTracker {
    pub fn new(app_data_dir: PathBuf) -> Self {
        let file_path = app_data_dir.join("child_processes.json");

        let processes = if file_path.exists() {
            if let Ok(content) = fs::read_to_string(&file_path) {
                serde_json::from_str(&content).unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        Self {
            file_path,
            processes: Mutex::new(processes),
        }
    }

    fn save(&self) {
        if let Ok(guard) = self.processes.lock() {
            if let Ok(json) = serde_json::to_string_pretty(&*guard) {
                let _ = fs::write(&self.file_path, json);
            }
        }
    }

    pub fn add_pid(&self, pid: u32, name: &str, service: &str) {
        if let Ok(mut guard) = self.processes.lock() {
            // Avoid duplicates
            if guard.iter().any(|p| p.pid == pid) {
                return;
            }
            guard.push(ChildProcess {
                pid,
                name: name.to_string(),
                service: service.to_string(),
            });
        }
        self.save();
    }

    pub fn remove_pid(&self, pid: u32) {
        if let Ok(mut guard) = self.processes.lock() {
            guard.retain(|p| p.pid != pid);
        }
        self.save();
    }

    // Kill a specific list of processes
    fn kill_processes(&self, procs: Vec<ChildProcess>) {
        // Create system object once
        let mut system = System::new_all();
        system.refresh_processes(ProcessesToUpdate::All, true);

        for proc in procs {
            let sys_pid = Pid::from_u32(proc.pid);
            if let Some(process) = system.process(sys_pid) {
                let proc_name = process.name().to_string_lossy();
                // Verify name
                if proc_name.to_lowercase().contains(&proc.name.to_lowercase()) {
                    println!(
                        "[ProcessTracker] Killing orphaned process: PID={} Name={}",
                        proc.pid, proc_name
                    );
                    process.kill();
                } else {
                    println!("[ProcessTracker] SKIP killing PID={} - Name mismatch (Expected: {}, Found: {})", proc.pid, proc.name, proc_name);
                }
            } else {
                // Process dead already?
                println!(
                    "[ProcessTracker] PID={} not found, assuming dead.",
                    proc.pid
                );
            }
        }
    }

    pub fn cleanup_all(&self) {
        let procs = {
            let mut guard = self.processes.lock().unwrap_or_else(|e| e.into_inner());
            let clones = guard.clone();
            guard.clear(); // We assume we are killing them all, so clear state
            clones
        };
        self.save(); // Save empty early to avoid re-killing if we crash now

        self.kill_processes(procs);
    }

    pub fn cleanup_by_service(&self, service_name: &str) {
        let procs_to_kill = {
            let mut guard = self.processes.lock().unwrap_or_else(|e| e.into_inner());
            let (to_kill, keep): (Vec<_>, Vec<_>) = guard
                .clone()
                .into_iter()
                .partition(|p| p.service == service_name);
            *guard = keep;
            to_kill
        };
        self.save();

        self.kill_processes(procs_to_kill);
    }
}
