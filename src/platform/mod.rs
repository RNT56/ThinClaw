//! Platform abstractions for host-sensitive behavior.
//!
//! Windows support is added through this layer so macOS/Linux behavior can
//! remain stable while individual call sites move off ad-hoc platform checks.

pub mod gateway_access;
pub mod linux_readiness;
pub mod paths;
pub mod secure_store;
pub mod shell;

pub use linux_readiness::{
    LinuxProbe, LinuxProbeStatus, LinuxReadinessProfile, LinuxReadinessReport,
    linux_readiness_report,
};
pub use thinclaw_platform::*;
