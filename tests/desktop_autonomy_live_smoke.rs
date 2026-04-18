use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

use thinclaw::config::DesktopAutonomyConfig;
use thinclaw::desktop_autonomy::{DesktopAutonomyManager, DesktopCanaryManifest};
use tokio::sync::Mutex;
use uuid::Uuid;

static LIVE_SMOKE_ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn live_smoke_enabled() -> bool {
    std::env::var("THINCLAW_LIVE_DESKTOP_SMOKE").ok().as_deref() == Some("1")
}

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<OsString>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: live smoke tests serialize access with LIVE_SMOKE_ENV_LOCK and only mutate
        // process env for the duration of a single ignored test run.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        // SAFETY: guarded by LIVE_SMOKE_ENV_LOCK in the test harness.
        unsafe {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn git_head_contents(relative_path: &str) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .arg("show")
        .arg(format!("HEAD:{relative_path}"))
        .output()
        .map_err(|e| format!("failed to read HEAD:{relative_path}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!(
                "git show HEAD:{relative_path} exited with {}",
                output.status
            )
        } else {
            stderr
        });
    }
    String::from_utf8(output.stdout)
        .map_err(|e| format!("HEAD:{relative_path} was not valid UTF-8: {e}"))
}

fn labeled_patch(relative_path: &str, updated_contents: &str) -> Result<String, String> {
    let original = git_head_contents(relative_path)?;
    let temp = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
    let original_path = temp.path().join("original");
    let updated_path = temp.path().join("updated");
    std::fs::write(&original_path, original)
        .map_err(|e| format!("failed to write original temp file: {e}"))?;
    std::fs::write(&updated_path, updated_contents)
        .map_err(|e| format!("failed to write updated temp file: {e}"))?;

    let output = Command::new("git")
        .arg("diff")
        .arg("--no-index")
        .arg("--label")
        .arg(format!("a/{relative_path}"))
        .arg("--label")
        .arg(format!("b/{relative_path}"))
        .arg(&original_path)
        .arg(&updated_path)
        .output()
        .map_err(|e| format!("failed to generate patch for {relative_path}: {e}"))?;
    match output.status.code() {
        Some(0) => Err(format!(
            "generated patch for {relative_path} was unexpectedly empty"
        )),
        Some(1) => String::from_utf8(output.stdout)
            .map_err(|e| format!("patch for {relative_path} was not valid UTF-8: {e}")),
        _ => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            Err(if stderr.is_empty() {
                format!("git diff for {relative_path} exited with {}", output.status)
            } else {
                stderr
            })
        }
    }
}

fn promotable_rollout_patch(label: &str) -> Result<String, String> {
    let mut readme = git_head_contents("README.md")?;
    if !readme.ends_with('\n') {
        readme.push('\n');
    }
    readme.push_str(&format!("<!-- live desktop rollout smoke: {label} -->\n"));
    labeled_patch("README.md", &readme)
}

fn blocked_rollout_patch(label: &str) -> Result<String, String> {
    let mut lib_rs = git_head_contents("src/lib.rs")?;
    if !lib_rs.ends_with('\n') {
        lib_rs.push('\n');
    }
    lib_rs.push_str(&format!(
        "const __LIVE_DESKTOP_SMOKE_BLOCKER_{label}: () = ;\n"
    ));
    labeled_patch("src/lib.rs", &lib_rs)
}

async fn assert_rollout_lifecycle(
    manager: &DesktopAutonomyManager,
    deployment_label: &str,
) -> Result<(), String> {
    let successful_first = manager
        .local_autorollout(
            &format!("live-smoke-{deployment_label}"),
            Uuid::new_v4(),
            &promotable_rollout_patch(&format!("{deployment_label}-first"))?,
            &format!("live smoke promotion {deployment_label} first"),
        )
        .await?;
    assert!(
        successful_first.promoted,
        "first rollout should promote successfully: {:?}",
        successful_first.checks
    );
    assert_eq!(
        manager.current_build_id().as_deref(),
        Some(successful_first.build_id.as_str()),
        "first rollout should become the active build"
    );

    let blocked_before = manager.current_build_id();
    let blocked = manager
        .local_autorollout(
            &format!("live-smoke-{deployment_label}"),
            Uuid::new_v4(),
            &blocked_rollout_patch(&format!(
                "{}_{}",
                deployment_label.replace('-', "_"),
                Uuid::new_v4().simple()
            ))?,
            &format!("live smoke promotion blocker {deployment_label}"),
        )
        .await?;
    assert!(
        !blocked.promoted,
        "blocked rollout should not promote: {:?}",
        blocked.checks
    );
    assert!(
        blocked.checks.iter().any(|check| !check.passed),
        "blocked rollout should include at least one failing check"
    );
    assert_eq!(
        manager.current_build_id(),
        blocked_before,
        "failed rollout must leave the active build unchanged"
    );

    let successful_second = manager
        .local_autorollout(
            &format!("live-smoke-{deployment_label}"),
            Uuid::new_v4(),
            &promotable_rollout_patch(&format!("{deployment_label}-second"))?,
            &format!("live smoke promotion {deployment_label} second"),
        )
        .await?;
    assert!(
        successful_second.promoted,
        "second rollout should promote successfully: {:?}",
        successful_second.checks
    );
    assert_ne!(
        successful_first.build_id, successful_second.build_id,
        "two successful promotions should produce distinct build ids"
    );
    assert_eq!(
        manager.current_build_id().as_deref(),
        Some(successful_second.build_id.as_str()),
        "second rollout should become the active build"
    );

    let rollback = manager.rollback().await?;
    assert_eq!(
        rollback
            .get("rolled_back")
            .and_then(|value| value.as_bool()),
        Some(true),
        "rollback should report success"
    );
    assert_eq!(
        rollback.get("build_id").and_then(|value| value.as_str()),
        Some(successful_first.build_id.as_str()),
        "rollback should restore the previously promoted build"
    );
    assert_eq!(
        manager.current_build_id().as_deref(),
        Some(successful_first.build_id.as_str()),
        "rollback should restore the prior promoted build as active"
    );

    Ok(())
}

async fn make_manager(
    deployment_mode: thinclaw::settings::DesktopDeploymentMode,
    target_username: Option<String>,
) -> Result<DesktopAutonomyManager, String> {
    let settings = thinclaw::settings::Settings::default();
    let mut config = DesktopAutonomyConfig::resolve(&settings).map_err(|e| e.to_string())?;
    config.enabled = true;
    config.profile = thinclaw::settings::DesktopAutonomyProfile::RecklessDesktop;
    config.deployment_mode = deployment_mode;
    config.target_username = target_username;
    Ok(DesktopAutonomyManager::new(config, None, None))
}

async fn execute_live_smoke_path(
    deployment_mode: thinclaw::settings::DesktopDeploymentMode,
    target_username: Option<String>,
    build_id: &str,
    bootstrap_assertion: impl Fn(&thinclaw::desktop_autonomy::AutonomyBootstrapReport),
) {
    if !live_smoke_enabled() {
        return;
    }

    let _lock = LIVE_SMOKE_ENV_LOCK.lock().await;
    let temp_home = tempfile::tempdir().expect("tempdir");
    let _thinclaw_home = ScopedEnvVar::set("THINCLAW_HOME", temp_home.path());
    let _emergency_stop = ScopedEnvVar::set(
        "DESKTOP_AUTONOMY_EMERGENCY_STOP_PATH",
        temp_home.path().join("AUTONOMY_DISABLED"),
    );

    let manager = make_manager(deployment_mode, target_username)
        .await
        .expect("live smoke manager");
    let bootstrap = manager.bootstrap().await.expect("bootstrap");
    bootstrap_assertion(&bootstrap);

    if !bootstrap.passed {
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = DesktopCanaryManifest {
        build_id: build_id.to_string(),
        proposal_id: "live".to_string(),
        report_path: temp.path().join("canary-report.json"),
        shadow_home: temp.path().join("shadow-home"),
        session_id: manager.default_session_id(),
        fixture_paths: bootstrap.fixture_paths.clone(),
    };
    let report = manager
        .execute_canary_manifest(&manifest)
        .await
        .expect("live canary report");
    assert_eq!(report.checks.len(), 7);
    assert!(
        report.passed,
        "live smoke canaries must pass before rollout lifecycle assertions: {:?}",
        report.checks
    );

    let deployment_label = match deployment_mode {
        thinclaw::settings::DesktopDeploymentMode::WholeMachineAdmin => "whole-machine-admin",
        thinclaw::settings::DesktopDeploymentMode::DedicatedUser => "dedicated-user",
    };
    assert_rollout_lifecycle(&manager, deployment_label)
        .await
        .expect("rollout lifecycle assertions");
}

async fn run_whole_machine_admin_live_desktop_smoke() {
    execute_live_smoke_path(
        thinclaw::settings::DesktopDeploymentMode::WholeMachineAdmin,
        None,
        "live-whole-machine",
        |bootstrap| {
            assert!(
                bootstrap.passed || bootstrap.blocking_reason.is_some(),
                "bootstrap should either pass or explain what blocked it"
            );
        },
    )
    .await;
}

#[cfg(target_os = "macos")]
#[ignore]
#[tokio::test]
async fn whole_machine_admin_live_desktop_smoke() {
    run_whole_machine_admin_live_desktop_smoke().await;
}

#[cfg(target_os = "windows")]
#[ignore]
#[tokio::test]
async fn windows_whole_machine_admin_live_desktop_smoke() {
    run_whole_machine_admin_live_desktop_smoke().await;
}

#[cfg(target_os = "linux")]
#[ignore]
#[tokio::test]
async fn linux_whole_machine_admin_live_desktop_smoke() {
    run_whole_machine_admin_live_desktop_smoke().await;
}

async fn run_dedicated_user_live_desktop_smoke() {
    if !live_smoke_enabled() {
        return;
    }

    let Some(target_username) = std::env::var("THINCLAW_LIVE_DEDICATED_USERNAME").ok() else {
        return;
    };

    execute_live_smoke_path(
        thinclaw::settings::DesktopDeploymentMode::DedicatedUser,
        Some(target_username),
        "live-dedicated-user",
        |bootstrap| {
            assert!(
                bootstrap.passed
                    || bootstrap.blocking_reason.as_deref() == Some("needs_target_user_login")
                    || bootstrap.blocking_reason.as_deref()
                        == Some("requires_privileged_bootstrap")
                    || bootstrap.blocking_reason.as_deref() == Some("requires_supported_apps")
                    || bootstrap.blocking_reason.as_deref()
                        == Some("session_launcher_install_failed"),
                "bootstrap should pass or report the dedicated-user blocker"
            );
        },
    )
    .await;
}

#[cfg(target_os = "macos")]
#[ignore]
#[tokio::test]
async fn dedicated_user_live_desktop_smoke() {
    run_dedicated_user_live_desktop_smoke().await;
}

#[cfg(target_os = "windows")]
#[ignore]
#[tokio::test]
async fn windows_dedicated_user_live_desktop_smoke() {
    run_dedicated_user_live_desktop_smoke().await;
}

#[cfg(target_os = "linux")]
#[ignore]
#[tokio::test]
async fn linux_dedicated_user_live_desktop_smoke() {
    if !live_smoke_enabled() {
        return;
    }

    let Some(target_username) = std::env::var("THINCLAW_LIVE_DEDICATED_USERNAME").ok() else {
        return;
    };

    execute_live_smoke_path(
        thinclaw::settings::DesktopDeploymentMode::DedicatedUser,
        Some(target_username),
        "live-dedicated-user-linux",
        |bootstrap| {
            assert!(
                bootstrap.passed
                    || bootstrap.blocking_reason.as_deref() == Some("needs_target_user_login")
                    || bootstrap.blocking_reason.as_deref() == Some("unsupported_deployment_mode")
                    || bootstrap.blocking_reason.as_deref() == Some("requires_supported_apps")
                    || bootstrap.blocking_reason.as_deref() == Some("unsupported_display_stack"),
                "linux dedicated-user smoke should pass or return an explicit best-effort blocker"
            );
        },
    )
    .await;
}
