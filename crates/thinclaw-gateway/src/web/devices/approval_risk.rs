//! Gateway-side approval **risk tier** classification (D-K3, single source of
//! truth).
//!
//! Design authority: `docs/MOBILE_SECURITY.md` D-K3 — "the `risk` tier is
//! computed gateway-side (single source of truth) and carried in approval
//! events and push categories, never approximated client-side." The mobile
//! client uses this tier to decide biometric gating (Face ID on **high**-risk
//! approvals) and whether interactive approve-from-notification is offered
//! (low-risk only, D-N3); the widget/watch refuse high-risk approvals entirely.
//!
//! We deliberately classify from the **tool name** (plus, as a hook, the raw
//! parameters) at the gateway rather than threading a risk value through the
//! internal `StatusUpdate` enum — that enum is matched exhaustively in many
//! places and adding a field ripples widely. Classification here keeps the
//! privacy/authority decision in one small, auditable place.
//!
//! ## Classification policy (least-privilege, D-K3)
//!
//! The mapping is an **allowlist of high-risk substrings** with a
//! **conservative default**: anything that is not *clearly* a read-only /
//! informational tool is treated as [`ApprovalRisk::High`]. Concretely:
//!
//! - **High** — a tool whose name contains any of the side-effecting or
//!   egress substrings in [`HIGH_RISK_SUBSTRINGS`] (shell/exec/command,
//!   http/fetch/network egress, browser automation, filesystem writes/deletes,
//!   deploy/install, etc.), OR any tool not matched by the low-risk allowlist.
//! - **Low** — only tools whose name matches the read-only allowlist in
//!   [`LOW_RISK_SUBSTRINGS`] (read/search/list/get/time/todo/memory-read …)
//!   *and* do not also match a high-risk substring.
//!
//! Rationale for defaulting **unknown → High**: an unrecognised tool might do
//! anything, and the cost of over-gating (an extra Face ID prompt) is far lower
//! than the cost of under-gating (a destructive action approved from a lock
//! screen without biometric confirmation). This matches the spec's
//! least-privilege intent for the biometric gate.

use serde::Serialize;

/// Risk tier for a tool approval, computed gateway-side. Serialised
/// snake_case (`"low"` / `"high"`) on approval events and used to pick the push
/// category. Carried on [`crate::web::types::SseEvent::ApprovalNeeded`] and
/// [`crate::web::types::PendingApprovalEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ApprovalRisk {
    /// Read-only / informational tools: no biometric gate, interactive
    /// approve-from-notification allowed (D-N3).
    Low,
    /// Side-effecting, egress, or unrecognised tools: Face ID required to
    /// approve; refused entirely on widget/watch (D-K3).
    High,
}

impl ApprovalRisk {
    /// Stable snake_case wire string (`"low"` / `"high"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ApprovalRisk::Low => "low",
            ApprovalRisk::High => "high",
        }
    }
}

/// Substrings that mark a tool name as **high risk**. Matched case-insensitively
/// against the tool name. Kept as a flat, auditable allowlist of dangerous
/// verbs/domains: side effects, code execution, network egress, browser
/// control, filesystem mutation, and deployment/installation.
const HIGH_RISK_SUBSTRINGS: &[&str] = &[
    // Code / command execution
    "exec",
    "shell",
    "command",
    "run_command",
    "bash",
    "sh",
    "process",
    // Network egress
    "http",
    "fetch",
    "curl",
    "request",
    "download",
    "upload",
    "webhook",
    // Browser automation
    "browser",
    "navigate",
    "click",
    "puppeteer",
    "playwright",
    "cdp",
    // Filesystem mutation
    "write",
    "delete",
    "remove",
    "rm",
    "edit",
    "create",
    "mkdir",
    "move",
    "rename",
    "chmod",
    "truncate",
    "append",
    // Deploy / install / package
    "deploy",
    "install",
    "publish",
    "release",
    "push",
    "commit",
    "apply",
    // Secrets / credentials / send
    "secret",
    "credential",
    "send",
    "email",
    "message",
    "post",
];

/// Substrings that mark a tool name as **low risk** (read-only /
/// informational), *unless* it also matches a high-risk substring. Matched
/// case-insensitively against the tool name.
const LOW_RISK_SUBSTRINGS: &[&str] = &[
    "read", "search", "list", "get", "show", "view", "query", "find", "lookup", "time", "date",
    "todo", "recall", "info", "status", "describe", "inspect", "history",
];

/// Classify a tool approval into a [`ApprovalRisk`] tier from its `tool_name`
/// (and raw `parameters`, reserved for future refinement).
///
/// Policy (see the module docs):
/// 1. If the name contains any [`HIGH_RISK_SUBSTRINGS`] → `High` (a mutating /
///    egress verb wins even if a read-ish word also appears, e.g.
///    `read_and_delete`).
/// 2. Otherwise, if the name matches a [`LOW_RISK_SUBSTRINGS`] read-only word →
///    `Low`.
/// 3. Otherwise (unrecognised) → `High` (conservative least-privilege default).
#[must_use]
pub fn classify(tool_name: &str, _parameters: &str) -> ApprovalRisk {
    let name = tool_name.to_ascii_lowercase();

    if HIGH_RISK_SUBSTRINGS.iter().any(|s| name.contains(s)) {
        return ApprovalRisk::High;
    }
    if LOW_RISK_SUBSTRINGS.iter().any(|s| name.contains(s)) {
        return ApprovalRisk::Low;
    }
    // Unknown tool: default High for safety (least-privilege biometric gate).
    ApprovalRisk::High
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_high(tool: &str) {
        assert_eq!(
            classify(tool, "{}"),
            ApprovalRisk::High,
            "expected {tool:?} to classify High"
        );
    }

    fn assert_low(tool: &str) {
        assert_eq!(
            classify(tool, "{}"),
            ApprovalRisk::Low,
            "expected {tool:?} to classify Low"
        );
    }

    #[test]
    fn shell_and_execution_tools_are_high() {
        for tool in [
            "shell",
            "shell.execute",
            "execute_code",
            "execute_shell",
            "run_command",
            "bash",
            "process.spawn",
            "exec",
        ] {
            assert_high(tool);
        }
    }

    #[test]
    fn network_egress_tools_are_high() {
        for tool in [
            "http_request",
            "fetch",
            "web_fetch",
            "curl",
            "download_file",
        ] {
            assert_high(tool);
        }
    }

    #[test]
    fn browser_tools_are_high() {
        for tool in [
            "browser",
            "browser_navigate",
            "playwright_click",
            "cdp_eval",
        ] {
            assert_high(tool);
        }
    }

    #[test]
    fn filesystem_mutations_are_high() {
        for tool in [
            "write_file",
            "fs_write",
            "delete_file",
            "file_edit",
            "create_directory",
            "rename_path",
        ] {
            assert_high(tool);
        }
    }

    #[test]
    fn deploy_install_send_tools_are_high() {
        for tool in [
            "deploy_app",
            "install_extension",
            "git_commit",
            "publish_release",
            "send_message",
            "post_message",
        ] {
            assert_high(tool);
        }
    }

    #[test]
    fn read_only_tools_are_low() {
        for tool in [
            "read_file",
            "fs_read",
            "search",
            "list_files",
            "get_time",
            "time",
            "todo_list",
            "memory_recall",
            "view_thread",
            "lookup_contact",
        ] {
            assert_low(tool);
        }
    }

    #[test]
    fn mutating_verb_wins_over_read_word() {
        // A high-risk substring beats an also-present read-ish word.
        assert_high("read_then_write");
        assert_high("list_and_delete");
        assert_high("get_and_post");
    }

    #[test]
    fn unknown_tools_default_high() {
        for tool in ["frobnicate", "mystery_tool", "quux", ""] {
            assert_high(tool);
        }
    }

    #[test]
    fn classification_is_case_insensitive() {
        assert_high("SHELL.Execute");
        assert_high("HTTP_Request");
        assert_low("Read_File");
        assert_low("GET_Time");
    }

    #[test]
    fn serialises_snake_case() {
        assert_eq!(
            serde_json::to_string(&ApprovalRisk::Low).unwrap(),
            "\"low\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalRisk::High).unwrap(),
            "\"high\""
        );
        assert_eq!(ApprovalRisk::Low.as_str(), "low");
        assert_eq!(ApprovalRisk::High.as_str(), "high");
    }
}
