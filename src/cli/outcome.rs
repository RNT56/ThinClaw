//! Typed command termination and dispatch results.

use std::process::ExitCode;

/// Stable public process-exit classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitClass {
    Success = 0,
    Operational = 1,
    Usage = 2,
    Unhealthy = 3,
    Interrupted = 130,
}

impl ExitClass {
    pub const fn code(self) -> u8 {
        self as u8
    }

    pub fn exit_code(self) -> ExitCode {
        ExitCode::from(self.code())
    }
}

/// Successful terminal-command outcome, including complete unhealthy reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliOutcome {
    Success,
    Unhealthy,
    Interrupted,
}

impl CliOutcome {
    pub const fn exit_class(self) -> ExitClass {
        match self {
            Self::Success => ExitClass::Success,
            Self::Unhealthy => ExitClass::Unhealthy,
            Self::Interrupted => ExitClass::Interrupted,
        }
    }
}

/// Whether a parsed invocation was handled by the immediate CLI or continues
/// into the long-running runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliDispatch {
    Runtime,
    Handled(CliOutcome),
}

/// Typed command error with one stable exit classification.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct CliError {
    class: ExitClass,
    message: String,
    reported: bool,
}

impl CliError {
    pub fn operational(message: impl Into<String>) -> Self {
        Self::new(ExitClass::Operational, message)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ExitClass::Usage, message)
    }

    pub fn unhealthy_reported() -> Self {
        Self {
            class: ExitClass::Unhealthy,
            message: "one or more required readiness checks failed".to_string(),
            reported: true,
        }
    }

    pub fn interrupted() -> Self {
        Self {
            class: ExitClass::Interrupted,
            message: "interrupted".to_string(),
            reported: true,
        }
    }

    pub fn new(class: ExitClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
            reported: false,
        }
    }

    pub const fn exit_class(&self) -> ExitClass {
        self.class
    }

    pub const fn was_reported(&self) -> bool {
        self.reported
    }
}

impl From<anyhow::Error> for CliError {
    fn from(error: anyhow::Error) -> Self {
        Self::operational(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_exit_classes_do_not_drift() {
        assert_eq!(CliOutcome::Success.exit_class().code(), 0);
        assert_eq!(CliError::operational("failure").exit_class().code(), 1);
        assert_eq!(CliError::usage("bad arguments").exit_class().code(), 2);
        assert_eq!(CliOutcome::Unhealthy.exit_class().code(), 3);
        assert_eq!(CliOutcome::Interrupted.exit_class().code(), 130);
    }

    #[test]
    fn reported_errors_do_not_require_duplicate_diagnostics() {
        assert!(CliError::unhealthy_reported().was_reported());
        assert!(CliError::interrupted().was_reported());
        assert!(!CliError::operational("failure").was_reported());
    }
}
