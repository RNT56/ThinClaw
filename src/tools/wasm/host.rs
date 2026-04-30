//! Compatibility facade for WASM host state.

use std::collections::HashSet;

use crate::tools::wasm::capabilities::Capabilities;
use crate::tools::wasm::error::WasmError;

pub use thinclaw_tools::wasm::host::{LogEntry, LogLevel};

#[derive(Debug)]
pub struct HostState {
    inner: thinclaw_tools::wasm::HostState,
}

impl HostState {
    pub fn new(capabilities: Capabilities) -> Self {
        Self {
            inner: thinclaw_tools::wasm::HostState::new(capabilities.into()),
        }
    }

    pub fn new_with_user(capabilities: Capabilities, user_id: impl Into<String>) -> Self {
        Self {
            inner: thinclaw_tools::wasm::HostState::new_with_user(capabilities.into(), user_id),
        }
    }

    pub fn with_available_secret_names(mut self, names: HashSet<String>) -> Self {
        self.inner = self.inner.with_available_secret_names(names);
        self
    }

    pub fn minimal() -> Self {
        Self::new(Capabilities::default())
    }

    pub fn user_id(&self) -> Option<&str> {
        self.inner.user_id()
    }

    pub fn capabilities(&self) -> &thinclaw_tools::wasm::Capabilities {
        self.inner.capabilities()
    }

    pub fn log(&mut self, level: LogLevel, message: String) -> Result<(), WasmError> {
        self.inner.log(level, message)
    }

    pub fn now_millis(&self) -> u64 {
        self.inner.now_millis()
    }

    pub fn workspace_read(&self, path: &str) -> Result<Option<String>, WasmError> {
        self.inner.workspace_read(path)
    }

    pub fn take_logs(&mut self) -> Vec<LogEntry> {
        self.inner.take_logs()
    }

    pub fn logs_dropped(&self) -> usize {
        self.inner.logs_dropped()
    }

    pub fn secret_exists(&self, name: &str) -> bool {
        self.inner.secret_exists(name)
    }

    pub fn check_http_allowed(&self, url: &str, method: &str) -> Result<(), String> {
        self.inner.check_http_allowed(url, method)
    }

    pub fn check_tool_invoke_allowed(&self, alias: &str) -> Result<String, String> {
        self.inner.check_tool_invoke_allowed(alias)
    }

    pub fn record_http_request(&mut self) -> Result<(), String> {
        self.inner.record_http_request()
    }

    pub fn record_tool_invoke(&mut self) -> Result<(), String> {
        self.inner.record_tool_invoke()
    }

    pub fn http_request_count(&self) -> u32 {
        self.inner.http_request_count()
    }

    pub fn tool_invoke_count(&self) -> u32 {
        self.inner.tool_invoke_count()
    }
}
