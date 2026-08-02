//! `thinclaw doctor` - active health diagnostics.
//!
//! Probes external dependencies and validates configuration to surface
//! problems before they bite during normal operation. Each check reports
//! pass/fail with actionable guidance on failures.

use std::path::PathBuf;

use serde::Serialize;

/// Run all diagnostic checks and print results.
pub async fn run_doctor_command(
    linux_profile: crate::platform::LinuxReadinessProfile,
    context: &crate::cli::CliContext,
) -> Result<(), crate::cli::CliError> {
    let mut checks = Vec::new();

    // ── Configuration checks ──────────────────────────────────

    check("LLM configuration", check_llm_config().await, &mut checks);

    check("Database backend", check_database().await, &mut checks);

    check(
        "Secrets posture",
        check_secrets_posture().await,
        &mut checks,
    );

    check("Workspace directory", check_workspace_dir(), &mut checks);

    // ── Linux readiness checks ────────────────────────────────

    let linux = crate::platform::linux_readiness_report(linux_profile).await;
    for probe in &linux.probes {
        check_linux_probe(probe, &mut checks);
    }

    // ── Optional external binary checks ───────────────────────

    check_optional_binary(
        "cloudflared",
        &["--version"],
        is_tunnel_provider("cloudflare"),
        "only required when TUNNEL_PROVIDER=cloudflare",
        &mut checks,
    );

    check_optional_binary(
        "ngrok",
        &["version"],
        is_tunnel_provider("ngrok"),
        "only required when TUNNEL_PROVIDER=ngrok",
        &mut checks,
    );

    check_optional_binary(
        "tailscale",
        &["version"],
        is_tunnel_provider("tailscale"),
        "only required when TUNNEL_PROVIDER=tailscale",
        &mut checks,
    );

    // ── Summary ───────────────────────────────────────────────

    let passed = checks.iter().filter(|check| check.state == "pass").count();
    let failed = checks.iter().filter(|check| check.state == "fail").count();
    let skipped = checks.iter().filter(|check| check.state == "skip").count();
    let report = DoctorReport {
        readiness_profile: linux_profile.as_str().to_string(),
        healthy: failed == 0,
        passed,
        failed,
        skipped,
        checks,
    };
    context
        .output()
        .write_record("doctor", &report, render_doctor_report)?;

    if failed > 0 {
        Err(crate::cli::CliError::unhealthy_reported())
    } else {
        Ok(())
    }
}

// ── Individual checks ───────────────────────────────────────

fn check(name: &str, result: CheckResult, checks: &mut Vec<DoctorCheck>) {
    let (state, detail) = match result {
        CheckResult::Pass(detail) => ("pass", detail),
        CheckResult::Fail(detail) => ("fail", detail),
        CheckResult::Skip(detail) => ("skip", detail),
    };
    checks.push(DoctorCheck {
        name: name.to_string(),
        state,
        detail,
    });
}

fn check_linux_probe(probe: &crate::platform::LinuxProbe, checks: &mut Vec<DoctorCheck>) {
    let result = match probe.status {
        crate::platform::LinuxProbeStatus::Pass => CheckResult::Pass(probe.detail.clone()),
        crate::platform::LinuxProbeStatus::Fail => {
            let mut detail = probe.detail.clone();
            if let Some(guidance) = &probe.guidance {
                detail.push(' ');
                detail.push_str(guidance);
            }
            CheckResult::Fail(detail)
        }
        crate::platform::LinuxProbeStatus::Skip => CheckResult::Skip(probe.detail.clone()),
    };
    check(probe.label, result, checks);
}

enum CheckResult {
    Pass(String),
    Fail(String),
    Skip(String),
}

fn is_tunnel_provider(provider: &str) -> bool {
    std::env::var("TUNNEL_PROVIDER")
        .map(|value| value.eq_ignore_ascii_case(provider))
        .unwrap_or(false)
}

fn check_optional_binary(
    name: &str,
    args: &[&str],
    required: bool,
    skip_reason: &str,
    checks: &mut Vec<DoctorCheck>,
) {
    let result = if required {
        check_binary(name, args)
    } else {
        CheckResult::Skip(skip_reason.to_string())
    };
    check(name, result, checks);
}

#[derive(Debug, Serialize)]
struct DoctorCheck {
    name: String,
    state: &'static str,
    detail: String,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    readiness_profile: String,
    healthy: bool,
    passed: usize,
    failed: usize,
    skipped: usize,
    checks: Vec<DoctorCheck>,
}

fn render_doctor_report(report: &DoctorReport) -> String {
    let mut lines = vec![format!(
        "ThinClaw doctor: {} passed, {} failed, {} skipped ({})",
        report.passed, report.failed, report.skipped, report.readiness_profile
    )];
    for check in &report.checks {
        lines.push(format!(
            "[{}] {}: {}",
            check.state, check.name, check.detail
        ));
    }
    lines.join("\n")
}

async fn check_llm_config() -> CheckResult {
    let backend = std::env::var("LLM_BACKEND")
        .ok()
        .unwrap_or_else(|| "openai_compatible".into());

    match backend.as_str() {
        "openai" => {
            if std::env::var("OPENAI_API_KEY").is_ok() {
                CheckResult::Pass("OpenAI API key configured".into())
            } else {
                CheckResult::Fail("LLM_BACKEND=openai but OPENAI_API_KEY not set".into())
            }
        }
        "anthropic" | "claude" => {
            if std::env::var("ANTHROPIC_API_KEY").is_ok() {
                CheckResult::Pass("Anthropic API key configured".into())
            } else {
                CheckResult::Fail("LLM_BACKEND=anthropic but ANTHROPIC_API_KEY not set".into())
            }
        }
        "ollama" => {
            let url = std::env::var("OLLAMA_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:11434".into());
            CheckResult::Pass(format!("Ollama configured ({})", url))
        }
        "tinfoil" => {
            if std::env::var("TINFOIL_API_KEY").is_ok() {
                CheckResult::Pass("Tinfoil API key configured".into())
            } else {
                CheckResult::Fail("LLM_BACKEND=tinfoil but TINFOIL_API_KEY not set".into())
            }
        }
        _ => {
            if std::env::var("LLM_BASE_URL").is_ok() {
                CheckResult::Pass(format!(
                    "OpenAI-compatible endpoint configured ({})",
                    std::env::var("LLM_BASE_URL").unwrap_or_default()
                ))
            } else {
                CheckResult::Fail(
                    "LLM_BACKEND=openai_compatible but LLM_BASE_URL not set. \
                     Set LLM_BASE_URL to your endpoint (e.g. https://openrouter.ai/api/v1)"
                        .into(),
                )
            }
        }
    }
}

async fn check_database() -> CheckResult {
    let backend = std::env::var("DATABASE_BACKEND")
        .ok()
        .unwrap_or_else(|| "postgres".into());

    match backend.as_str() {
        "libsql" | "turso" | "sqlite" => {
            let path = std::env::var("LIBSQL_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| crate::config::default_libsql_path());

            if path.exists() {
                CheckResult::Pass(format!("libSQL database exists ({})", path.display()))
            } else {
                CheckResult::Pass(format!(
                    "libSQL database not found at {} (will be created on first run)",
                    path.display()
                ))
            }
        }
        _ => {
            if std::env::var("DATABASE_URL").is_ok() {
                // Try to connect
                match try_pg_connect().await {
                    Ok(()) => CheckResult::Pass("PostgreSQL connected".into()),
                    Err(e) => CheckResult::Fail(format!("PostgreSQL connection failed: {e}")),
                }
            } else {
                CheckResult::Fail("DATABASE_URL not set".into())
            }
        }
    }
}

async fn check_secrets_posture() -> CheckResult {
    let probe = crate::platform::secure_store::probe_availability().await;
    let env_present = std::env::var_os("SECRETS_MASTER_KEY").is_some();
    let env_allowed = std::env::var("THINCLAW_ALLOW_ENV_MASTER_KEY")
        .ok()
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false);

    if env_present && !env_allowed {
        return CheckResult::Fail(
            "SECRETS_MASTER_KEY is present but ignored by strict defaults; use the OS secure store or set THINCLAW_ALLOW_ENV_MASTER_KEY=1 deliberately"
                .into(),
        );
    }

    if probe.available {
        if probe.env_fallback {
            CheckResult::Pass("explicit env fallback is configured for local_encrypted v2".into())
        } else {
            CheckResult::Pass(format!("{}; local_encrypted v2 expected", probe.detail))
        }
    } else {
        CheckResult::Fail(format!("{} {}", probe.detail, probe.guidance))
    }
}

#[cfg(feature = "postgres")]
async fn try_pg_connect() -> Result<(), String> {
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL not set".to_string())?;

    let config = deadpool_postgres::Config {
        url: Some(url),
        ..Default::default()
    };
    let pool = config
        .create_pool(
            Some(deadpool_postgres::Runtime::Tokio1),
            tokio_postgres::NoTls,
        )
        .map_err(|e| format!("pool error: {e}"))?;

    let client = tokio::time::timeout(std::time::Duration::from_secs(5), pool.get())
        .await
        .map_err(|_| "connection timeout (5s)".to_string())?
        .map_err(|e| format!("{e}"))?;

    client
        .execute("SELECT 1", &[])
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(())
}

#[cfg(not(feature = "postgres"))]
async fn try_pg_connect() -> Result<(), String> {
    Err("postgres feature not compiled in".into())
}

fn check_workspace_dir() -> CheckResult {
    let dir = crate::platform::resolve_thinclaw_home();

    if dir.exists() {
        if dir.is_dir() {
            CheckResult::Pass(format!("{}", dir.display()))
        } else {
            CheckResult::Fail(format!("{} exists but is not a directory", dir.display()))
        }
    } else {
        CheckResult::Pass(format!("{} will be created on first run", dir.display()))
    }
}

fn check_binary(name: &str, args: &[&str]) -> CheckResult {
    let mut command = thinclaw_platform::std_process_command!("src.cli.doctor.std.1", name);
    command.args(args);
    match thinclaw_platform::bounded_std_command_output(
        &mut command,
        std::time::Duration::from_secs(10),
        64 * 1024,
        64 * 1024,
    ) {
        Ok(output) => {
            let version = String::from_utf8_lossy(&output.stdout);
            let version = version.trim();
            // Some tools print version to stderr
            let version = if version.is_empty() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                stderr.trim().lines().next().unwrap_or("").to_string()
            } else {
                version.lines().next().unwrap_or("").to_string()
            };

            if output.status.success() {
                CheckResult::Pass(version)
            } else {
                CheckResult::Fail(format!("exited with {}", output.status))
            }
        }
        Err(_) => CheckResult::Skip(format!("{name} not found in PATH")),
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::doctor::*;

    #[test]
    fn check_binary_finds_host_shell() {
        let launcher = crate::platform::shell_launcher();
        let mut args: Vec<&str> = launcher.prefix_args().to_vec();
        args.push("echo ok");

        match check_binary(launcher.program(), &args) {
            CheckResult::Pass(_) => {}
            other => panic!(
                "expected Pass for host shell {}, got: {}",
                launcher.program(),
                format_result(&other)
            ),
        }
    }

    #[test]
    fn check_binary_skips_nonexistent() {
        match check_binary("__thinclaw_nonexistent_binary__", &["--version"]) {
            CheckResult::Skip(_) => {}
            other => panic!(
                "expected Skip for nonexistent binary, got: {}",
                format_result(&other)
            ),
        }
    }

    #[test]
    fn check_workspace_dir_does_not_panic() {
        let result = check_workspace_dir();
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    #[tokio::test]
    async fn check_llm_config_does_not_panic() {
        let result = check_llm_config().await;
        match result {
            CheckResult::Pass(_) | CheckResult::Fail(_) | CheckResult::Skip(_) => {}
        }
    }

    fn format_result(r: &CheckResult) -> String {
        match r {
            CheckResult::Pass(s) => format!("Pass({s})"),
            CheckResult::Fail(s) => format!("Fail({s})"),
            CheckResult::Skip(s) => format!("Skip({s})"),
        }
    }
}
