//! Desktop observability adapter and local crash reports.
//!
//! The core observer remains authoritative for log/Prometheus output. This
//! adapter fans the same metadata-only records into the typed Tauri event bus
//! and persists redacted error/panic diagnostics locally. Reports are never
//! uploaded and retention is deliberately bounded.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};

use serde::{Deserialize, Serialize};
use tauri::Emitter as _;
use thinclaw_core::observability::{Observer, ObserverEvent, ObserverMetric};

use super::ui_types::{UiEvent, UiObserverRecord};

const CRASH_REPORT_VERSION: u32 = 1;
const MAX_CRASH_REPORTS: usize = 20;
const MAX_MESSAGE_CHARS: usize = 4_096;
const MAX_BACKTRACE_CHARS: usize = 32_768;

#[derive(Clone)]
pub struct DesktopCrashReporter {
    directory: Arc<PathBuf>,
    write_guard: Arc<Mutex<()>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CrashReport {
    version: u32,
    id: String,
    occurred_at: String,
    kind: String,
    component: String,
    message: String,
    location: Option<String>,
    thread: Option<String>,
    backtrace: Option<String>,
    app_version: String,
}

impl DesktopCrashReporter {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory: Arc::new(directory),
            write_guard: Arc::new(Mutex::new(())),
        }
    }

    pub fn install_panic_hook(&self) {
        static INSTALL: Once = Once::new();
        let reporter = self.clone();
        INSTALL.call_once(move || {
            let previous = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                reporter.record_panic(info);
                previous(info);
            }));
        });
    }

    fn record_observer_error(&self, component: &str, message: &str) {
        let report = CrashReport::new(
            "observer_error",
            component,
            message,
            None,
            std::thread::current().name().map(str::to_string),
            None,
        );
        self.persist_best_effort(&report);
    }

    fn record_panic(&self, info: &std::panic::PanicHookInfo<'_>) {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|value| (*value).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_string());
        let location = info.location().map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        });
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let report = CrashReport::new(
            "panic",
            "desktop",
            &message,
            location,
            std::thread::current().name().map(str::to_string),
            Some(&backtrace),
        );
        self.persist_best_effort(&report);
    }

    fn persist_best_effort(&self, report: &CrashReport) {
        if let Err(error) = self.persist(report) {
            eprintln!("[desktop-crash-reporter] failed to persist report: {error}");
        }
    }

    fn persist(&self, report: &CrashReport) -> std::io::Result<PathBuf> {
        let _guard = self
            .write_guard
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        std::fs::create_dir_all(self.directory.as_ref())?;

        let basename = format!(
            "{}-{}-{}.json",
            chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
            report.kind,
            report.id,
        );
        let destination = self.directory.join(basename);
        let temporary = self.directory.join(format!(".{}.tmp", report.id));
        let bytes = serde_json::to_vec_pretty(report).map_err(std::io::Error::other)?;

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&temporary, &destination)?;
        self.enforce_retention()?;
        Ok(destination)
    }

    fn enforce_retention(&self) -> std::io::Result<()> {
        let mut reports = std::fs::read_dir(self.directory.as_ref())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        reports.sort();
        let remove_count = reports.len().saturating_sub(MAX_CRASH_REPORTS);
        for path in reports.into_iter().take(remove_count) {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }
}

impl CrashReport {
    fn new(
        kind: &str,
        component: &str,
        message: &str,
        location: Option<String>,
        thread: Option<String>,
        backtrace: Option<&str>,
    ) -> Self {
        Self {
            version: CRASH_REPORT_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            occurred_at: chrono::Utc::now().to_rfc3339(),
            kind: kind.to_string(),
            component: sanitize_diagnostic(component, 256),
            message: sanitize_diagnostic(message, MAX_MESSAGE_CHARS),
            location: location.map(|value| sanitize_diagnostic(&value, 1_024)),
            thread: thread.map(|value| sanitize_diagnostic(&value, 256)),
            backtrace: backtrace.map(|value| sanitize_diagnostic(value, MAX_BACKTRACE_CHARS)),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

fn sanitize_diagnostic(value: &str, max_chars: usize) -> String {
    let redacted = thinclaw_core::repo_projects::ci::redact_sensitive_text(value);
    redacted
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
        .take(max_chars)
        .collect()
}

pub struct DesktopObserver {
    core: Arc<dyn Observer>,
    app_handle: tauri::AppHandle<tauri::Wry>,
    crash_reporter: DesktopCrashReporter,
}

impl DesktopObserver {
    pub fn new(
        core: Arc<dyn Observer>,
        app_handle: tauri::AppHandle<tauri::Wry>,
        crash_reporter: DesktopCrashReporter,
    ) -> Self {
        Self {
            core,
            app_handle,
            crash_reporter,
        }
    }

    pub fn emit_desktop_event(&self, event: &ObserverEvent) {
        self.emit(observer_event_record(event));
        if let ObserverEvent::Error { component, message } = event {
            self.crash_reporter
                .record_observer_error(component, message);
        }
    }

    fn emit(&self, record: UiObserverRecord) {
        let _ = self
            .app_handle
            .emit("thinclaw-event", UiEvent::ObserverRecord { record });
    }
}

impl Observer for DesktopObserver {
    fn record_event(&self, event: &ObserverEvent) {
        self.core.record_event(event);
        self.emit_desktop_event(event);
    }

    fn record_metric(&self, metric: &ObserverMetric) {
        self.core.record_metric(metric);
        self.emit(observer_metric_record(metric));
    }

    fn flush(&self) {
        self.core.flush();
    }

    fn name(&self) -> &str {
        "desktop"
    }
}

fn observer_event_record(event: &ObserverEvent) -> UiObserverRecord {
    let mut attributes = BTreeMap::new();
    let (name, success, duration_ms) = match event {
        ObserverEvent::AgentStart { provider, model } => {
            attributes.insert("provider".into(), sanitize_diagnostic(provider, 256));
            attributes.insert("model".into(), sanitize_diagnostic(model, 256));
            ("agent_start", None, None)
        }
        ObserverEvent::LlmRequest {
            provider,
            model,
            message_count,
        } => {
            attributes.insert("provider".into(), sanitize_diagnostic(provider, 256));
            attributes.insert("model".into(), sanitize_diagnostic(model, 256));
            attributes.insert("message_count".into(), message_count.to_string());
            ("llm_request", None, None)
        }
        ObserverEvent::LlmResponse {
            provider,
            model,
            duration,
            success,
            error_message,
        } => {
            attributes.insert("provider".into(), sanitize_diagnostic(provider, 256));
            attributes.insert("model".into(), sanitize_diagnostic(model, 256));
            if let Some(error) = error_message {
                attributes.insert(
                    "error".into(),
                    sanitize_diagnostic(error, MAX_MESSAGE_CHARS),
                );
            }
            ("llm_response", Some(*success), Some(duration_ms(*duration)))
        }
        ObserverEvent::ToolCallStart { tool } => {
            attributes.insert("tool".into(), sanitize_diagnostic(tool, 256));
            ("tool_call_start", None, None)
        }
        ObserverEvent::ToolCallEnd {
            tool,
            duration,
            success,
        } => {
            attributes.insert("tool".into(), sanitize_diagnostic(tool, 256));
            (
                "tool_call_end",
                Some(*success),
                Some(duration_ms(*duration)),
            )
        }
        ObserverEvent::TurnComplete => ("turn_complete", Some(true), None),
        ObserverEvent::ChannelMessage { channel, direction } => {
            attributes.insert("channel".into(), sanitize_diagnostic(channel, 256));
            attributes.insert("direction".into(), sanitize_diagnostic(direction, 64));
            ("channel_message", None, None)
        }
        ObserverEvent::HeartbeatTick => ("heartbeat_tick", Some(true), None),
        ObserverEvent::AgentEnd {
            duration,
            tokens_used,
        } => {
            if let Some(tokens) = tokens_used {
                attributes.insert("tokens_used".into(), tokens.to_string());
            }
            ("agent_end", Some(true), Some(duration_ms(*duration)))
        }
        ObserverEvent::Error { component, message } => {
            attributes.insert("component".into(), sanitize_diagnostic(component, 256));
            attributes.insert(
                "message".into(),
                sanitize_diagnostic(message, MAX_MESSAGE_CHARS),
            );
            ("error", Some(false), None)
        }
    };
    UiObserverRecord {
        record_type: "event".into(),
        name: name.into(),
        success,
        duration_ms,
        attributes,
    }
}

fn observer_metric_record(metric: &ObserverMetric) -> UiObserverRecord {
    let mut attributes = BTreeMap::new();
    let (name, success, duration_ms) = match metric {
        ObserverMetric::RequestLatency(duration) => {
            ("request_latency", None, Some(duration_ms(*duration)))
        }
        ObserverMetric::TokensUsed(tokens) => {
            attributes.insert("tokens".into(), tokens.to_string());
            ("tokens_used", None, None)
        }
        ObserverMetric::ActiveJobs(jobs) => {
            attributes.insert("jobs".into(), jobs.to_string());
            ("active_jobs", None, None)
        }
        ObserverMetric::QueueDepth(depth) => {
            attributes.insert("depth".into(), depth.to_string());
            ("queue_depth", None, None)
        }
        ObserverMetric::LoopStarted(kind) => {
            attributes.insert("loop_kind".into(), kind.as_str().into());
            ("loop_started", None, None)
        }
        ObserverMetric::LoopRun(summary) => {
            attributes.insert("loop_kind".into(), summary.kind.as_str().into());
            attributes.insert("stop_reason".into(), summary.stop_reason.as_str().into());
            attributes.insert("iterations".into(), summary.iterations.to_string());
            attributes.insert("retries".into(), summary.retries.to_string());
            ("loop_run", Some(!summary.stop_reason.is_failure()), None)
        }
        ObserverMetric::LoopPhaseRun(phase) => {
            attributes.insert("loop_kind".into(), phase.kind.as_str().into());
            attributes.insert("phase".into(), sanitize_diagnostic(&phase.phase, 256));
            attributes.insert("stop_reason".into(), phase.stop_reason.as_str().into());
            attributes.insert("iterations".into(), phase.iterations.to_string());
            attributes.insert("retries".into(), phase.retries.to_string());
            (
                "loop_phase_run",
                Some(!phase.stop_reason.is_failure()),
                Some(duration_ms(phase.duration)),
            )
        }
    };
    UiObserverRecord {
        record_type: "metric".into(),
        name: name.into(),
        success,
        duration_ms,
        attributes,
    }
}

fn duration_ms(duration: std::time::Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observer_error_records_are_redacted() {
        let record = observer_event_record(&ObserverEvent::Error {
            component: "provider".into(),
            message: "OPENAI_API_KEY=sk-secret-value".into(),
        });
        let message = record.attributes.get("message").expect("message");
        assert!(message.contains("[REDACTED:secret]"));
        assert!(!message.contains("sk-secret-value"));
        assert_eq!(record.success, Some(false));
    }

    #[test]
    fn crash_reports_are_private_and_retention_is_bounded() {
        let temp = tempfile::tempdir().expect("tempdir");
        let reporter = DesktopCrashReporter::new(temp.path().to_path_buf());
        for index in 0..(MAX_CRASH_REPORTS + 5) {
            reporter.record_observer_error(
                "test",
                &format!("token={index}-secret OPENAI_API_KEY=sk-sensitive"),
            );
        }

        let paths = std::fs::read_dir(temp.path())
            .expect("read reports")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
            .collect::<Vec<_>>();
        assert_eq!(paths.len(), MAX_CRASH_REPORTS);
        let report: CrashReport =
            serde_json::from_slice(&std::fs::read(&paths[0]).expect("read report"))
                .expect("parse report");
        assert!(report.message.contains("[REDACTED:secret]"));
        assert!(!report.message.contains("sk-sensitive"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&paths[0])
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn metric_records_preserve_duration() {
        let record = observer_metric_record(&ObserverMetric::RequestLatency(
            std::time::Duration::from_millis(875),
        ));
        assert_eq!(record.name, "request_latency");
        assert_eq!(record.duration_ms, Some(875));
    }
}
