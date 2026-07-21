//! CI signal classification and prompt/log redaction helpers for repo projects.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubCiScope {
    Workflow,
    Job,
    Check,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubCiCheck {
    pub scope: GitHubCiScope,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
}

impl GitHubCiCheck {
    pub fn new(
        scope: GitHubCiScope,
        name: impl Into<String>,
        conclusion: Option<impl Into<String>>,
        log: Option<impl Into<String>>,
    ) -> Self {
        Self {
            scope,
            name: name.into(),
            status: None,
            conclusion: conclusion.map(Into::into),
            log: log.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiOutcome {
    Passing,
    Failing,
    Pending,
    Neutral,
    Skipped,
    Cancelled,
    TimedOut,
    ActionRequired,
    Unknown,
}

impl CiOutcome {
    pub fn is_green(self) -> bool {
        matches!(self, Self::Passing | Self::Neutral | Self::Skipped)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiFailureKind {
    Compilation,
    Tests,
    Formatting,
    Lint,
    DependencyResolution,
    ToolchainSetup,
    Permission,
    Timeout,
    Cancelled,
    SecurityScan,
    SecretLeak,
    Infrastructure,
    ActionRequired,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairAction {
    FixCompilation,
    FixTests,
    RunFormatter,
    FixLint,
    UpdateDependencies,
    FixCiEnvironment,
    FixPermissions,
    RetryCi,
    RequestHuman,
    RotateSecret,
    InspectLogs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairRecommendation {
    pub action: RepairAction,
    pub summary: String,
    pub prompt_hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiClassification {
    pub scope: GitHubCiScope,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    pub outcome: CiOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<CiFailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<RepairRecommendation>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiSuiteClassification {
    pub checks_green: bool,
    pub failure_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_failure_kind: Option<CiFailureKind>,
    pub summary: String,
    pub checks: Vec<CiClassification>,
}

pub fn classify_ci_check(check: &GitHubCiCheck) -> CiClassification {
    let log = check.log.as_deref().unwrap_or_default();
    let conclusion = normalize_optional(check.conclusion.as_deref());
    let status = normalize_optional(check.status.as_deref());
    let log_failure_kind = classify_failure_from_log(log);
    let (outcome, failure_kind) = match conclusion.as_deref() {
        Some("success") => (CiOutcome::Passing, None),
        Some("neutral") => (CiOutcome::Neutral, None),
        Some("skipped") => (CiOutcome::Skipped, None),
        Some("cancelled") => (CiOutcome::Cancelled, Some(CiFailureKind::Cancelled)),
        Some("timed_out") => (CiOutcome::TimedOut, Some(CiFailureKind::Timeout)),
        Some("action_required") => (
            CiOutcome::ActionRequired,
            Some(CiFailureKind::ActionRequired),
        ),
        Some("startup_failure") => (CiOutcome::Failing, Some(CiFailureKind::ToolchainSetup)),
        Some("failure") => (
            CiOutcome::Failing,
            Some(log_failure_kind.unwrap_or(CiFailureKind::Unknown)),
        ),
        Some(_) => {
            if let Some(kind) = log_failure_kind {
                (CiOutcome::Failing, Some(kind))
            } else {
                (CiOutcome::Unknown, Some(CiFailureKind::Unknown))
            }
        }
        None if matches!(
            status.as_deref(),
            Some("queued" | "in_progress" | "pending" | "waiting")
        ) =>
        {
            (CiOutcome::Pending, None)
        }
        None => {
            if let Some(kind) = log_failure_kind {
                (CiOutcome::Failing, Some(kind))
            } else {
                (CiOutcome::Unknown, None)
            }
        }
    };

    let evidence = failure_kind
        .map(|kind| evidence_lines(log, kind))
        .unwrap_or_default();
    let recommendation = failure_kind.map(recommendation_for);

    CiClassification {
        scope: check.scope,
        name: redact_sensitive_text(&check.name),
        status,
        conclusion,
        outcome,
        failure_kind,
        recommendation,
        evidence,
    }
}

pub fn classify_ci_checks(checks: &[GitHubCiCheck]) -> CiSuiteClassification {
    let mut classified = checks.iter().map(classify_ci_check).collect::<Vec<_>>();
    classified.sort_by(|left, right| {
        scope_rank(left.scope)
            .cmp(&scope_rank(right.scope))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.conclusion.cmp(&right.conclusion))
            .then_with(|| left.status.cmp(&right.status))
    });

    let checks_green =
        !classified.is_empty() && classified.iter().all(|check| check.outcome.is_green());
    let failure_count = classified
        .iter()
        .filter(|check| !check.outcome.is_green())
        .count();
    let primary_failure_kind = classified.iter().find_map(|check| check.failure_kind);
    let summary = if checks_green {
        format!("all {} CI check(s) are green", classified.len())
    } else if let Some(kind) = primary_failure_kind {
        format!(
            "{failure_count} CI check(s) are not green; primary failure: {}",
            failure_kind_label(kind)
        )
    } else {
        format!("{failure_count} CI check(s) are not green")
    };

    CiSuiteClassification {
        checks_green,
        failure_count,
        primary_failure_kind,
        summary,
        checks: classified,
    }
}

pub fn redact_sensitive_text(input: &str) -> String {
    let mut redacted = input.to_string();
    for pattern in redaction_patterns() {
        redacted = pattern
            .regex
            .replace_all(&redacted, pattern.replacement)
            .into_owned();
    }
    redacted
}

pub fn failure_kind_label(kind: CiFailureKind) -> &'static str {
    match kind {
        CiFailureKind::Compilation => "compilation",
        CiFailureKind::Tests => "tests",
        CiFailureKind::Formatting => "formatting",
        CiFailureKind::Lint => "lint",
        CiFailureKind::DependencyResolution => "dependency_resolution",
        CiFailureKind::ToolchainSetup => "toolchain_setup",
        CiFailureKind::Permission => "permission",
        CiFailureKind::Timeout => "timeout",
        CiFailureKind::Cancelled => "cancelled",
        CiFailureKind::SecurityScan => "security_scan",
        CiFailureKind::SecretLeak => "secret_leak",
        CiFailureKind::Infrastructure => "infrastructure",
        CiFailureKind::ActionRequired => "action_required",
        CiFailureKind::Unknown => "unknown",
    }
}

fn classify_failure_from_log(log: &str) -> Option<CiFailureKind> {
    let lower = log.to_ascii_lowercase();
    if lower.trim().is_empty() {
        return None;
    }

    if contains_any(
        &lower,
        &[
            "detected secret",
            "secret scanning",
            "gitleaks",
            "trufflehog",
            "private key detected",
            "leaked secret",
        ],
    ) {
        return Some(CiFailureKind::SecretLeak);
    }
    if contains_any(
        &lower,
        &[
            "critical severity",
            "high severity vulnerability",
            "security audit failed",
            "cargo audit",
            "npm audit",
            "dependabot alert",
        ],
    ) {
        return Some(CiFailureKind::SecurityScan);
    }
    if contains_any(
        &lower,
        &[
            "timed out",
            "timeout",
            "exceeded the maximum execution time",
        ],
    ) {
        return Some(CiFailureKind::Timeout);
    }
    if contains_any(
        &lower,
        &[
            "permission denied",
            "resource not accessible by integration",
            "http 403",
            "403 forbidden",
            "could not read from remote repository",
        ],
    ) {
        return Some(CiFailureKind::Permission);
    }
    if contains_any(
        &lower,
        &[
            "failed to resolve",
            "could not find package",
            "no matching distribution found",
            "npm err! eresolve",
            "lock file needs to be updated",
            "cargo.lock needs to be updated",
            "package-lock.json is out of date",
        ],
    ) {
        return Some(CiFailureKind::DependencyResolution);
    }
    if contains_any(
        &lower,
        &[
            "cargo fmt --check",
            "would be formatted",
            "prettier --check",
            "black --check",
            "gofmt",
            "rustfmt",
        ],
    ) {
        return Some(CiFailureKind::Formatting);
    }
    if contains_any(
        &lower,
        &[
            "clippy",
            "eslint",
            "flake8",
            "rubocop",
            "lint failed",
            "linter failed",
        ],
    ) {
        return Some(CiFailureKind::Lint);
    }
    if contains_any(
        &lower,
        &[
            "error[e",
            "failed to compile",
            "could not compile",
            "compilation failed",
            "tsc",
            "typescript error",
            "javac",
            "build failed",
        ],
    ) {
        return Some(CiFailureKind::Compilation);
    }
    if contains_any(
        &lower,
        &[
            "test result: failed",
            "test failed",
            "tests failed",
            "failures:",
            "assertionerror",
            "assertion failed",
            "failed tests/",
            "panic at",
        ],
    ) {
        return Some(CiFailureKind::Tests);
    }
    if contains_any(
        &lower,
        &[
            "hosted runner",
            "connection reset",
            "temporarily unavailable",
            "service unavailable",
            "rate limit",
            "no space left on device",
            "runner lost communication",
        ],
    ) {
        return Some(CiFailureKind::Infrastructure);
    }

    Some(CiFailureKind::Unknown)
}

fn evidence_lines(log: &str, kind: CiFailureKind) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in log.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if evidence_line_matches_kind(&lower, kind)
            || contains_any(
                &lower,
                &["error", "failed", "failure", "timed out", "denied"],
            )
        {
            lines.push(truncate(&redact_sensitive_text(trimmed), 240));
        }
        if lines.len() == 5 {
            break;
        }
    }

    if lines.is_empty() {
        log.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(3)
            .map(|line| truncate(&redact_sensitive_text(line), 240))
            .collect()
    } else {
        lines
    }
}

fn evidence_line_matches_kind(line: &str, kind: CiFailureKind) -> bool {
    match kind {
        CiFailureKind::Compilation => contains_any(
            line,
            &["error[e", "failed to compile", "could not compile", "tsc"],
        ),
        CiFailureKind::Tests => contains_any(
            line,
            &[
                "test result: failed",
                "test failed",
                "failures:",
                "assertion",
                "panic at",
            ],
        ),
        CiFailureKind::Formatting => contains_any(
            line,
            &["fmt", "formatted", "prettier", "black --check", "gofmt"],
        ),
        CiFailureKind::Lint => contains_any(line, &["clippy", "eslint", "flake8", "lint"]),
        CiFailureKind::DependencyResolution => contains_any(
            line,
            &[
                "failed to resolve",
                "could not find package",
                "no matching distribution",
                "lock file",
            ],
        ),
        CiFailureKind::ToolchainSetup => contains_any(
            line,
            &["startup", "toolchain", "setup", "rustup", "node-version"],
        ),
        CiFailureKind::Permission => {
            contains_any(line, &["permission denied", "403", "not accessible"])
        }
        CiFailureKind::Timeout => contains_any(line, &["timed out", "timeout"]),
        CiFailureKind::Cancelled => contains_any(line, &["cancelled", "canceled"]),
        CiFailureKind::SecurityScan => {
            contains_any(line, &["severity", "security", "audit", "dependabot"])
        }
        CiFailureKind::SecretLeak => contains_any(line, &["secret", "gitleaks", "trufflehog"]),
        CiFailureKind::Infrastructure => contains_any(
            line,
            &[
                "hosted runner",
                "connection reset",
                "temporarily unavailable",
            ],
        ),
        CiFailureKind::ActionRequired => contains_any(line, &["action required", "required"]),
        CiFailureKind::Unknown => false,
    }
}

fn recommendation_for(kind: CiFailureKind) -> RepairRecommendation {
    let (action, summary, prompt_hint) = match kind {
        CiFailureKind::Compilation => (
            RepairAction::FixCompilation,
            "Fix compile/build errors before rerunning CI.",
            "Inspect the compiler errors, make the smallest source change that restores the build, then run the failing build command locally.",
        ),
        CiFailureKind::Tests => (
            RepairAction::FixTests,
            "Fix failing tests or update incorrect assertions.",
            "Reproduce the failing test target, identify the behavioral regression, and add or update focused tests with the fix.",
        ),
        CiFailureKind::Formatting => (
            RepairAction::RunFormatter,
            "Run the project formatter and commit formatting-only changes.",
            "Run the formatter used by the failing job, then keep the diff limited to formatting unless a real syntax issue is present.",
        ),
        CiFailureKind::Lint => (
            RepairAction::FixLint,
            "Fix lint diagnostics without broad refactors.",
            "Apply targeted lint fixes, preserving behavior and the surrounding style.",
        ),
        CiFailureKind::DependencyResolution => (
            RepairAction::UpdateDependencies,
            "Repair dependency or lockfile resolution.",
            "Inspect the package manager error, update the lockfile or manifest consistently, and rerun the dependency check.",
        ),
        CiFailureKind::ToolchainSetup => (
            RepairAction::FixCiEnvironment,
            "Fix workflow setup/toolchain configuration.",
            "Inspect setup steps and version pins before changing application code.",
        ),
        CiFailureKind::Permission => (
            RepairAction::FixPermissions,
            "Fix token, repository, or workflow permission configuration.",
            "Check whether the job needs GitHub permissions, secrets, or a human permission change before retrying.",
        ),
        CiFailureKind::Timeout => (
            RepairAction::RetryCi,
            "CI timed out.",
            "Look for hangs or unexpectedly slow tests; retry only if the evidence points to runner slowness.",
        ),
        CiFailureKind::Cancelled => (
            RepairAction::RetryCi,
            "CI was cancelled before completion.",
            "Do not change code solely for a cancellation; retry or wait for a fresh run unless other evidence is present.",
        ),
        CiFailureKind::SecurityScan => (
            RepairAction::RequestHuman,
            "Security scanning reported a blocking finding.",
            "Triage the vulnerability or policy finding and request human confirmation for risk acceptance.",
        ),
        CiFailureKind::SecretLeak => (
            RepairAction::RotateSecret,
            "Secret scanning reported leaked credentials.",
            "Treat the credential as compromised, remove it from the diff/history where possible, and request rotation.",
        ),
        CiFailureKind::Infrastructure => (
            RepairAction::RetryCi,
            "CI appears to have failed due to runner or service infrastructure.",
            "Retry after checking for transient runner, network, or provider failures.",
        ),
        CiFailureKind::ActionRequired => (
            RepairAction::RequestHuman,
            "GitHub requires a human action before CI can proceed.",
            "Surface the required approval or manual action instead of attempting an automated code fix.",
        ),
        CiFailureKind::Unknown => (
            RepairAction::InspectLogs,
            "CI failed, but the failure class is unknown.",
            "Inspect the full job logs and classify the failing command before making changes.",
        ),
    };

    RepairRecommendation {
        action,
        summary: summary.to_string(),
        prompt_hint: prompt_hint.to_string(),
    }
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase().replace('-', "_"))
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn scope_rank(scope: GitHubCiScope) -> u8 {
    match scope {
        GitHubCiScope::Workflow => 0,
        GitHubCiScope::Job => 1,
        GitHubCiScope::Check => 2,
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

struct RedactionPattern {
    regex: Regex,
    replacement: &'static str,
}

fn redaction_patterns() -> &'static [RedactionPattern] {
    static PATTERNS: OnceLock<Vec<RedactionPattern>> = OnceLock::new();
    PATTERNS
        .get_or_init(|| {
            vec![
                RedactionPattern {
                    regex: Regex::new(
                        r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
                    )
                    .expect("private key redaction regex is valid"),
                    replacement: "[REDACTED:private_key]",
                },
                RedactionPattern {
                    regex: Regex::new(r"\bsk-ant-[A-Za-z0-9_-]{16,}\b")
                        .expect("Anthropic key redaction regex is valid"),
                    replacement: "[REDACTED:anthropic_key]",
                },
                RedactionPattern {
                    regex: Regex::new(r"\bsk-(?:proj-)?[A-Za-z0-9_-]{20,}\b")
                        .expect("OpenAI key redaction regex is valid"),
                    replacement: "[REDACTED:openai_key]",
                },
                RedactionPattern {
                    regex: Regex::new(
                        r"\b(?:gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,})\b",
                    )
                    .expect("GitHub token redaction regex is valid"),
                    replacement: "[REDACTED:github_token]",
                },
                RedactionPattern {
                    regex: Regex::new(r"\bxox[baprs]-[A-Za-z0-9-]{16,}\b")
                        .expect("Slack token redaction regex is valid"),
                    replacement: "[REDACTED:slack_token]",
                },
                RedactionPattern {
                    regex: Regex::new(r"\bAKIA[0-9A-Z]{16}\b")
                        .expect("AWS key id redaction regex is valid"),
                    replacement: "[REDACTED:aws_access_key]",
                },
                RedactionPattern {
                    regex: Regex::new(
                        r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b",
                    )
                    .expect("JWT redaction regex is valid"),
                    replacement: "[REDACTED:jwt]",
                },
                RedactionPattern {
                    regex: Regex::new(
                        r#"(?i)\b([A-Z0-9_]*(?:TOKEN|SECRET|PASSWORD|API[_-]?KEY|PRIVATE[_-]?KEY)[A-Z0-9_]*)\s*[:=]\s*['"]?[^'"\s]{8,}['"]?"#,
                    )
                    .expect("environment secret redaction regex is valid"),
                    replacement: "$1=[REDACTED:secret]",
                },
                RedactionPattern {
                    regex: Regex::new(r"(?i)\b(bearer|token)\s+[A-Za-z0-9._~+/=-]{16,}")
                        .expect("bearer token redaction regex is valid"),
                    replacement: "$1 [REDACTED:secret]",
                },
                RedactionPattern {
                    regex: Regex::new(r"([a-zA-Z][a-zA-Z0-9+.-]*://)[^/\s:@]+:[^/\s@]+@")
                        .expect("URL credential redaction regex is valid"),
                    replacement: "$1[REDACTED:credentials]@",
                },
            ]
        })
        .as_slice()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_compile_and_test_failures_from_logs() {
        let compile = GitHubCiCheck::new(
            GitHubCiScope::Job,
            "build",
            Some("failure"),
            Some("error[E0425]: cannot find value `x` in this scope\ncould not compile `crate`"),
        );
        let tests = GitHubCiCheck::new(
            GitHubCiScope::Check,
            "unit",
            Some("failure"),
            Some("failures:\nthread 'it_works' panicked at src/lib.rs:3:9\n"),
        );

        let compile_classification = classify_ci_check(&compile);
        let test_classification = classify_ci_check(&tests);

        assert_eq!(
            compile_classification.failure_kind,
            Some(CiFailureKind::Compilation)
        );
        assert_eq!(test_classification.failure_kind, Some(CiFailureKind::Tests));
        assert_eq!(
            compile_classification.recommendation.unwrap().action,
            RepairAction::FixCompilation
        );
    }

    #[test]
    fn classifies_conclusion_without_logs() {
        let timed_out = GitHubCiCheck::new(
            GitHubCiScope::Workflow,
            "ci",
            Some("timed_out"),
            None::<String>,
        );

        let classification = classify_ci_check(&timed_out);

        assert_eq!(classification.outcome, CiOutcome::TimedOut);
        assert_eq!(classification.failure_kind, Some(CiFailureKind::Timeout));
        assert_eq!(
            classification.recommendation.unwrap().action,
            RepairAction::RetryCi
        );
    }

    #[test]
    fn suite_classification_is_sorted_and_green_only_when_all_checks_are_green() {
        let checks = vec![
            GitHubCiCheck::new(
                GitHubCiScope::Check,
                "z-check",
                Some("success"),
                None::<String>,
            ),
            GitHubCiCheck::new(
                GitHubCiScope::Workflow,
                "a-workflow",
                Some("success"),
                None::<String>,
            ),
            GitHubCiCheck::new(
                GitHubCiScope::Job,
                "lint",
                Some("failure"),
                Some("eslint failed"),
            ),
        ];

        let suite = classify_ci_checks(&checks);

        assert!(!suite.checks_green);
        assert_eq!(suite.failure_count, 1);
        assert_eq!(suite.primary_failure_kind, Some(CiFailureKind::Lint));
        assert_eq!(suite.checks[0].name, "a-workflow");
        assert_eq!(suite.checks[1].name, "lint");
    }

    #[test]
    fn redacts_secrets_from_logs() {
        let synthetic_openai_key = format!("{}{}", "sk-proj-", "a".repeat(48));
        let raw = format!(
            "OPENAI_API_KEY={synthetic_openai_key}\n\
             Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz123456\n\
             remote=https://user:password@example.com/repo.git\n\
             -----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----"
        );

        let redacted = redact_sensitive_text(&raw);

        assert!(!redacted.contains(&synthetic_openai_key));
        assert!(!redacted.contains("ghp_abcdefghijklmnopqrstuvwxyz123456"));
        assert!(!redacted.contains("user:password"));
        assert!(!redacted.contains("BEGIN PRIVATE KEY"));
        assert!(redacted.contains("OPENAI_API_KEY=[REDACTED:secret]"));
        assert!(redacted.contains("[REDACTED:private_key]"));
    }
}
