//! Bridge contract primitives (TDO-001).
//!
//! Normalizes how desktop Tauri commands express dual-mode (embedded vs remote
//! gateway) availability. Historically "this isn't available in local mode" was
//! signalled two incompatible ways: some commands returned `Err(String)` (e.g.
//! `local_unavailable` in `commands/rpc_jobs_autonomy.rs`), others returned
//! `Ok(unavailable(...))` JSON. The frontend cannot reliably tell "gated, here's
//! why" from "failed". `BridgeError` makes a gated state a single, typed,
//! machine-readable outcome carrying its remediation, so the UI can render a CTA
//! instead of an error toast.
//!
//! This module is the foundation the rest of WS-1 (route-table registry, bridge
//! linter, generated route matrix) builds on. It is intentionally additive: it
//! does not yet replace existing `Result<_, String>` signatures — commands are
//! migrated incrementally.

use serde::{Deserialize, Serialize};

/// How a command behaves across the dual-mode runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RouteMode {
    /// Works in both embedded and remote-gateway mode.
    LocalAndRemote,
    /// Only meaningful against a remote gateway (e.g. sandbox job restart, GPU launch).
    RemoteOnly,
    /// Only meaningful in embedded mode (e.g. local sidecar control).
    LocalOnly,
}

/// A typed command outcome that distinguishes a *gated* capability (with its
/// reason + remediation) from a genuine runtime error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeError {
    /// The capability is intentionally unavailable in the current runtime mode.
    Unavailable {
        /// Short capability label, e.g. "manual outcome evaluation".
        capability: String,
        /// Why it is unavailable right now.
        reason: String,
        /// What the user must do to satisfy it (shown as a CTA), if anything.
        remediation: Option<String>,
        /// Which runtime mode *would* satisfy it.
        satisfied_by: RouteMode,
    },
    /// A genuine error (kept distinct from the gated state above).
    /// Struct variant (not a tuple) so the internally-tagged (`tag = "kind"`)
    /// representation stays valid for serde/specta export.
    Runtime { message: String },
}

impl std::fmt::Display for BridgeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeError::Unavailable {
                capability, reason, ..
            } => write!(f, "unavailable: {capability}: {reason}"),
            BridgeError::Runtime { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for BridgeError {}

/// Lets existing `?`/`.map_err(|e| e.to_string())` sites migrate to
/// `Result<T, BridgeError>` mechanically: any string error becomes `Runtime`.
impl From<String> for BridgeError {
    fn from(value: String) -> Self {
        BridgeError::Runtime { message: value }
    }
}

impl From<&str> for BridgeError {
    fn from(value: &str) -> Self {
        BridgeError::Runtime {
            message: value.to_string(),
        }
    }
}

/// Build a `BridgeError::Unavailable` for a capability that is gated in the
/// current runtime mode. Replaces the ad-hoc `local_unavailable`/`unavailable`
/// helpers with one typed, frontend-renderable shape.
pub fn gated(
    capability: impl Into<String>,
    reason: impl Into<String>,
    remediation: impl Into<String>,
    satisfied_by: RouteMode,
) -> BridgeError {
    BridgeError::Unavailable {
        capability: capability.into(),
        reason: reason.into(),
        remediation: Some(remediation.into()),
        satisfied_by,
    }
}

// ---------------------------------------------------------------------------
// Route table (TDO-002)
// ---------------------------------------------------------------------------
//
// Maps Tauri command names to their RouteMode. This is the seed of the bridge
// linter; it is intentionally not exhaustive. Additional commands are added as
// they are audited.
//
// Ordering within each RouteMode group is alphabetical. Do not mix modes within
// a group — keep RemoteOnly, LocalOnly, and LocalAndRemote entries together so
// that reviewers can verify the assignment at a glance.

/// Route table: classifies Tauri commands by [`RouteMode`]. Every command whose
/// generated binding returns `BridgeError` (a *gated* command) MUST appear here
/// — enforced by `all_gated_commands_are_classified`. Non-gated commands are
/// enrolled incrementally; their absence is not an error.
pub static ROUTE_TABLE: &[(&str, RouteMode)] = &[
    // ---- RemoteOnly ---------------------------------------------------------
    // Require a live remote gateway; no embedded-mode implementation.
    (
        "thinclaw_experiments_gpu_launch_test",
        RouteMode::RemoteOnly,
    ),
    ("thinclaw_experiments_gpu_validate", RouteMode::RemoteOnly),
    ("thinclaw_extension_reconnect", RouteMode::RemoteOnly),
    ("thinclaw_job_prompt", RouteMode::RemoteOnly),
    ("thinclaw_job_restart", RouteMode::RemoteOnly),
    ("thinclaw_learning_evaluate_outcomes", RouteMode::RemoteOnly),
    // ---- LocalOnly ----------------------------------------------------------
    // Embedded-only: sidecar servers, local-filesystem features (checkpoints,
    // trajectory archive), and local mutations the gateway owns separately.
    ("direct_runtime_start_chat_server", RouteMode::LocalOnly),
    ("direct_runtime_start_stt_server", RouteMode::LocalOnly),
    ("direct_runtime_stop_chat_server", RouteMode::LocalOnly),
    ("thinclaw_checkpoint_diff", RouteMode::LocalOnly),
    ("thinclaw_checkpoint_restore", RouteMode::LocalOnly),
    ("thinclaw_checkpoints_list", RouteMode::LocalOnly),
    // Agent-loop eval drives the embedded agent; no remote-gateway equivalent.
    ("thinclaw_experiments_run_eval", RouteMode::LocalOnly),
    ("thinclaw_install_skill_repo", RouteMode::LocalOnly),
    ("thinclaw_session_search", RouteMode::LocalOnly),
    ("thinclaw_set_autonomy_mode", RouteMode::LocalOnly),
    ("thinclaw_skills_toggle", RouteMode::LocalOnly),
    ("thinclaw_trajectory_records", RouteMode::LocalOnly),
    ("thinclaw_trajectory_stats", RouteMode::LocalOnly),
    // ---- LocalAndRemote -----------------------------------------------------
    // Work in both modes; some (autonomy_*) still gate *execution* behind host
    // policy via BridgeError::Unavailable.
    ("thinclaw_autonomy_checks", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_evidence", RouteMode::LocalAndRemote),
    ("thinclaw_autonomy_rollouts", RouteMode::LocalAndRemote),
    ("thinclaw_cost_summary", RouteMode::LocalAndRemote),
    ("thinclaw_get_sessions", RouteMode::LocalAndRemote),
    ("thinclaw_jobs_list", RouteMode::LocalAndRemote),
    ("thinclaw_routine_create", RouteMode::LocalAndRemote),
    ("thinclaw_send_message", RouteMode::LocalAndRemote),
    ("thinclaw_skills_list", RouteMode::LocalAndRemote),
];

/// Look up the [`RouteMode`] for a Tauri command name.
///
/// Returns `None` when the command is not yet registered in the route table —
/// this is intentional: the table is additive and commands are enrolled
/// incrementally.
pub fn route_mode(command: &str) -> Option<RouteMode> {
    ROUTE_TABLE
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, mode)| *mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gated_builds_unavailable_with_remediation() {
        let err = gated(
            "manual outcome evaluation",
            "requires the gateway outcome service",
            "connect a remote gateway",
            RouteMode::RemoteOnly,
        );
        match &err {
            BridgeError::Unavailable {
                capability,
                remediation,
                satisfied_by,
                ..
            } => {
                assert_eq!(capability, "manual outcome evaluation");
                assert_eq!(remediation.as_deref(), Some("connect a remote gateway"));
                assert_eq!(*satisfied_by, RouteMode::RemoteOnly);
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        assert!(err
            .to_string()
            .contains("unavailable: manual outcome evaluation"));
    }

    #[test]
    fn gated_serializes_with_kind_tag() {
        let err = gated("x", "y", "z", RouteMode::LocalOnly);
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "unavailable");
        assert_eq!(json["satisfied_by"], "local_only");
    }

    #[test]
    fn string_error_maps_to_runtime() {
        let err: BridgeError = "boom".to_string().into();
        assert_eq!(
            err,
            BridgeError::Runtime {
                message: "boom".to_string()
            }
        );
    }

    #[test]
    fn runtime_error_serializes_with_kind_tag() {
        // Regression guard: the internally-tagged enum must stay serde/specta
        // exportable — a tuple variant here breaks `cargo run --example export_bindings`.
        let err: BridgeError = "boom".to_string().into();
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "runtime");
        assert_eq!(json["message"], "boom");
    }

    // ---- route table tests (TDO-002) ----------------------------------------

    #[test]
    fn route_table_is_non_empty() {
        assert!(
            !ROUTE_TABLE.is_empty(),
            "ROUTE_TABLE must have at least one entry"
        );
    }

    #[test]
    fn route_table_command_names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (name, _) in ROUTE_TABLE {
            assert!(
                seen.insert(*name),
                "duplicate command name in ROUTE_TABLE: {name}"
            );
        }
    }

    fn snake_to_camel(s: &str) -> String {
        let mut out = String::new();
        let mut upper = false;
        for c in s.chars() {
            if c == '_' {
                upper = true;
            } else if upper {
                out.push(c.to_ascii_uppercase());
                upper = false;
            } else {
                out.push(c);
            }
        }
        out
    }

    fn camel_to_snake(s: &str) -> String {
        let mut out = String::new();
        for c in s.chars() {
            if c.is_ascii_uppercase() {
                out.push('_');
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    /// Every command listed in ROUTE_TABLE must be a real registered command
    /// (present in the generated bindings) — guards against typos/stale rows.
    #[test]
    fn route_table_commands_are_registered() {
        let bindings = include_str!("../../../frontend/src/lib/bindings.ts");
        for (cmd, _) in ROUTE_TABLE {
            let camel = snake_to_camel(cmd);
            assert!(
                bindings.contains(&format!("async {camel}(")),
                "ROUTE_TABLE references `{cmd}` (`{camel}`) which is not a registered command in bindings.ts"
            );
        }
    }

    /// TDO-002 linter: every gated command (its generated binding returns
    /// `BridgeError`) must be classified in ROUTE_TABLE, so the route-matrix can
    /// never silently omit a gated capability.
    #[test]
    fn all_gated_commands_are_classified() {
        let bindings = include_str!("../../../frontend/src/lib/bindings.ts");
        let mut checked = 0;
        for line in bindings.lines() {
            let line = line.trim();
            if !line.starts_with("async ") || !line.contains("BridgeError>") {
                continue;
            }
            let name = line["async ".len()..]
                .split('(')
                .next()
                .unwrap_or("")
                .trim();
            if name.is_empty() {
                continue;
            }
            let snake = camel_to_snake(name);
            assert!(
                route_mode(&snake).is_some(),
                "gated command `{snake}` returns BridgeError but is not classified in ROUTE_TABLE"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "expected at least one gated (BridgeError) command in bindings.ts"
        );
    }

    #[test]
    fn route_mode_remote_only_command() {
        assert_eq!(
            route_mode("thinclaw_job_restart"),
            Some(RouteMode::RemoteOnly),
            "thinclaw_job_restart must be RemoteOnly"
        );
    }

    #[test]
    fn route_mode_unknown_command_returns_none() {
        assert_eq!(route_mode("nope"), None);
    }
}
