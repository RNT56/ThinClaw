//! Elevated execution mode.
//!
//! Certain operations (e.g., system commands, file writes outside workspace)
//! require elevated privileges. This module manages the elevation state
//! and provides guards for privileged operations.
//!
//! Configuration:
//! - `ELEVATED_MODE` — "true" to enable elevated mode (default: false)
//! - `ELEVATED_COMMANDS` — comma-separated list of allowed elevated commands
//! - `ELEVATED_TIMEOUT` — seconds before elevated mode auto-expires (default: 300)

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Elevated mode configuration.
#[derive(Debug, Clone)]
pub struct ElevatedConfig {
    /// Whether elevated mode is available (can be activated).
    pub available: bool,
    /// Commands that are allowed in elevated mode.
    pub allowed_commands: HashSet<String>,
    /// Timeout before elevated mode auto-expires.
    pub timeout: Duration,
    /// Whether to require explicit user confirmation for each elevated action.
    pub require_confirmation: bool,
}

impl Default for ElevatedConfig {
    fn default() -> Self {
        Self {
            available: false,
            allowed_commands: HashSet::new(),
            timeout: Duration::from_secs(300),
            require_confirmation: true,
        }
    }
}

impl ElevatedConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();

        if let Ok(val) = std::env::var("ELEVATED_MODE") {
            config.available = val == "1" || val.eq_ignore_ascii_case("true");
        }

        if let Ok(cmds) = std::env::var("ELEVATED_COMMANDS") {
            for cmd in cmds.split(',') {
                let trimmed = cmd.trim().to_string();
                if !trimmed.is_empty() {
                    config.allowed_commands.insert(trimmed);
                }
            }
        }

        if let Ok(timeout) = std::env::var("ELEVATED_TIMEOUT")
            && let Ok(secs) = timeout.parse()
        {
            config.timeout = Duration::from_secs(secs);
        }

        if let Ok(val) = std::env::var("ELEVATED_REQUIRE_CONFIRM") {
            config.require_confirmation = val != "0" && !val.eq_ignore_ascii_case("false");
        }

        config
    }
}

/// Runtime state for elevated mode.
pub struct ElevatedMode {
    config: ElevatedConfig,
    /// Whether elevated mode is currently active.
    active: AtomicBool,
    /// When elevated mode was activated.
    activated_at: Option<Instant>,
    /// Number of elevated operations performed.
    operation_count: u32,
}

impl ElevatedMode {
    pub fn new(config: ElevatedConfig) -> Self {
        Self {
            config,
            active: AtomicBool::new(false),
            activated_at: None,
            operation_count: 0,
        }
    }

    /// Activate elevated mode.
    pub fn activate(&mut self) -> Result<(), ElevatedError> {
        if !self.config.available {
            return Err(ElevatedError::NotAvailable);
        }
        self.active.store(true, Ordering::SeqCst);
        self.activated_at = Some(Instant::now());
        self.operation_count = 0;
        Ok(())
    }

    /// Deactivate elevated mode.
    pub fn deactivate(&mut self) {
        self.active.store(false, Ordering::SeqCst);
        self.activated_at = None;
    }

    /// Check if elevated mode is currently active (considering timeout).
    pub fn is_active(&self) -> bool {
        if !self.active.load(Ordering::SeqCst) {
            return false;
        }
        if let Some(activated) = self.activated_at
            && activated.elapsed() > self.config.timeout
        {
            return false; // Expired
        }
        true
    }

    /// Check if a specific command is allowed in elevated mode.
    pub fn is_command_allowed(&self, command: &str) -> bool {
        if !self.is_active() {
            return false;
        }
        // If no specific commands are configured, all are allowed
        if self.config.allowed_commands.is_empty() {
            return true;
        }
        self.config.allowed_commands.contains(command)
    }

    /// Guard an elevated operation. Returns Ok if allowed.
    pub fn guard(&mut self, operation: &str) -> Result<(), ElevatedError> {
        if !self.config.available {
            return Err(ElevatedError::NotAvailable);
        }
        if !self.is_active() {
            return Err(ElevatedError::NotActive);
        }
        if !self.config.allowed_commands.is_empty()
            && !self.config.allowed_commands.contains(operation)
        {
            return Err(ElevatedError::CommandNotAllowed(operation.to_string()));
        }
        self.operation_count += 1;
        Ok(())
    }

    /// Whether confirmation is required.
    pub fn requires_confirmation(&self) -> bool {
        self.config.require_confirmation
    }

    /// Remaining time before expiry.
    pub fn remaining_time(&self) -> Option<Duration> {
        self.activated_at
            .map(|t| self.config.timeout.saturating_sub(t.elapsed()))
    }

    /// Number of elevated operations performed this session.
    pub fn operation_count(&self) -> u32 {
        self.operation_count
    }
}

/// Errors related to elevated mode.
#[derive(Debug, Clone, PartialEq)]
pub enum ElevatedError {
    NotAvailable,
    NotActive,
    CommandNotAllowed(String),
    Expired,
}

impl std::fmt::Display for ElevatedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAvailable => write!(f, "Elevated mode is not available"),
            Self::NotActive => write!(f, "Elevated mode is not active"),
            Self::CommandNotAllowed(cmd) => {
                write!(f, "Command '{}' is not allowed in elevated mode", cmd)
            }
            Self::Expired => write!(f, "Elevated mode has expired"),
        }
    }
}

impl std::error::Error for ElevatedError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_not_available() {
        let config = ElevatedConfig::default();
        assert!(!config.available);
    }

    #[test]
    fn test_activate_when_available() {
        let config = ElevatedConfig {
            available: true,
            ..Default::default()
        };
        let mut mode = ElevatedMode::new(config);
        assert!(mode.activate().is_ok());
        assert!(mode.is_active());
    }

    #[test]
    fn test_activate_when_unavailable() {
        let config = ElevatedConfig::default();
        let mut mode = ElevatedMode::new(config);
        assert_eq!(mode.activate(), Err(ElevatedError::NotAvailable));
    }

    #[test]
    fn test_deactivate() {
        let config = ElevatedConfig {
            available: true,
            ..Default::default()
        };
        let mut mode = ElevatedMode::new(config);
        mode.activate().unwrap();
        mode.deactivate();
        assert!(!mode.is_active());
    }

    #[test]
    fn test_command_allowlist() {
        let mut allowed = HashSet::new();
        allowed.insert("sudo".to_string());
        let config = ElevatedConfig {
            available: true,
            allowed_commands: allowed,
            ..Default::default()
        };
        let mut mode = ElevatedMode::new(config);
        mode.activate().unwrap();

        assert!(mode.is_command_allowed("sudo"));
        assert!(!mode.is_command_allowed("rm"));
    }

    #[test]
    fn test_empty_allowlist_allows_all() {
        let config = ElevatedConfig {
            available: true,
            ..Default::default()
        };
        let mut mode = ElevatedMode::new(config);
        mode.activate().unwrap();
        assert!(mode.is_command_allowed("anything"));
    }

    #[test]
    fn test_guard_increments_count() {
        let config = ElevatedConfig {
            available: true,
            ..Default::default()
        };
        let mut mode = ElevatedMode::new(config);
        mode.activate().unwrap();
        mode.guard("test").unwrap();
        mode.guard("test2").unwrap();
        assert_eq!(mode.operation_count(), 2);
    }

    #[test]
    fn test_timeout_expiry() {
        let config = ElevatedConfig {
            available: true,
            timeout: Duration::from_millis(0), // Immediate timeout
            ..Default::default()
        };
        let mut mode = ElevatedMode::new(config);
        mode.activate().unwrap();
        std::thread::sleep(Duration::from_millis(10));
        assert!(!mode.is_active());
    }

    #[test]
    fn test_error_display() {
        let err = ElevatedError::CommandNotAllowed("rm".to_string());
        assert!(format!("{}", err).contains("rm"));
    }
}
