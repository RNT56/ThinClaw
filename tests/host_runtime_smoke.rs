#![cfg(feature = "web-gateway")]

use std::io::Read;
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::sync::{Mutex, MutexGuard};
use tokio::time::sleep;

const AUTH_TOKEN: &str = "thinclaw-smoke-token";
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);
static RUNTIME_SMOKE_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

async fn runtime_smoke_guard() -> MutexGuard<'static, ()> {
    RUNTIME_SMOKE_LOCK.lock().await
}

fn reserve_local_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind ephemeral port")
        .local_addr()
        .expect("read local addr")
        .port()
}

fn thinclaw_binary() -> String {
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_thinclaw") {
        return path;
    }

    let exe_name = if cfg!(windows) {
        "thinclaw.exe"
    } else {
        "thinclaw"
    };
    let fallback = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        .and_then(|deps_dir| deps_dir.parent().map(|parent| parent.join(exe_name)));

    match fallback {
        Some(path) if path.exists() => path.to_string_lossy().into_owned(),
        Some(path) => panic!(
            "smoke test could not find {exe_name}; expected Cargo to set CARGO_BIN_EXE_thinclaw or build {}",
            path.display()
        ),
        None => panic!(
            "smoke test could not resolve a fallback path for {exe_name}; run with `cargo test --bin thinclaw --test host_runtime_smoke ...`"
        ),
    }
}

fn spawn_runtime(temp: &TempDir, port: u16) -> Child {
    spawn_runtime_with_port_env(temp, &port.to_string(), None)
}

fn spawn_runtime_with_port_env(
    temp: &TempDir,
    port: &str,
    bound_addr_file: Option<&std::path::Path>,
) -> Child {
    let home = temp.path().join("home");
    std::fs::create_dir_all(&home).expect("create smoke THINCLAW_HOME");

    let mut command = Command::new(thinclaw_binary());
    command
        .current_dir(temp.path())
        .arg("run")
        .arg("--no-onboard")
        .env("THINCLAW_HOME", &home)
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("ONBOARD_COMPLETED", "true")
        .env("CLI_ENABLED", "false")
        .env("HTTP_HOST", "127.0.0.1")
        .env("HTTP_PORT", "0")
        .env("HTTP_WEBHOOK_SECRET", "thinclaw-smoke-secret")
        .env("GATEWAY_ENABLED", "true")
        .env("GATEWAY_HOST", "127.0.0.1")
        .env("GATEWAY_PORT", port)
        .env("GATEWAY_AUTH_TOKEN", AUTH_TOKEN)
        .env("DATABASE_BACKEND", "libsql")
        .env("LIBSQL_PATH", home.join("thinclaw.db"))
        .env("LLM_BACKEND", "ollama")
        .env("OLLAMA_BASE_URL", "http://127.0.0.1:11434")
        .env("SANDBOX_ENABLED", "false")
        .env("HEARTBEAT_ENABLED", "false")
        .env("ROUTINES_ENABLED", "false")
        .env("EXPERIMENTS_ENABLED", "false")
        .env("BUILDER_ENABLED", "false")
        .env("SKILLS_ENABLED", "false")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(path) = bound_addr_file {
        command.env("THINCLAW_GATEWAY_BOUND_ADDR_FILE", path);
    }

    command.spawn().expect("spawn thinclaw smoke runtime")
}

fn read_child_output(child: &mut Child) -> String {
    let mut stdout = String::new();
    let mut stderr = String::new();

    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }

    format!("stdout:\n{stdout}\n\nstderr:\n{stderr}")
}

async fn wait_for_gateway_ready(child: &mut Child, port: u16) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let health_url = format!("http://127.0.0.1:{port}/api/health");
    let status_url = format!("http://127.0.0.1:{port}/api/gateway/status?token={AUTH_TOKEN}");
    let mut last_error: String;

    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("failed to poll runtime process: {e}"))?
        {
            let output = read_child_output(child);
            return Err(format!(
                "runtime exited before gateway became ready (status: {status}).\n{output}"
            ));
        }

        match client.get(&health_url).send().await {
            Ok(response) if response.status().is_success() => {
                match response.json::<serde_json::Value>().await {
                    Ok(health)
                        if health.get("status")
                            == Some(&serde_json::Value::String("healthy".to_string())) =>
                    {
                        match client.get(&status_url).send().await {
                            Ok(status) if status.status().is_success() => {
                                match status.json::<serde_json::Value>().await {
                                    Ok(status_json) if status_json.get("uptime_secs").is_some() => {
                                        return Ok(());
                                    }
                                    Ok(status_json) => {
                                        last_error = format!(
                                            "gateway status response missing uptime_secs: {status_json}"
                                        );
                                    }
                                    Err(e) => {
                                        last_error = format!(
                                            "failed to decode gateway status response: {e}"
                                        );
                                    }
                                }
                            }
                            Ok(status) => {
                                last_error =
                                    format!("gateway status endpoint returned {}", status.status());
                            }
                            Err(e) => {
                                last_error = format!("gateway status endpoint failed: {e}");
                            }
                        }
                    }
                    Ok(health) => {
                        last_error = format!("gateway health response not ready: {health}");
                    }
                    Err(e) => {
                        last_error = format!("failed to decode health response: {e}");
                    }
                }
            }
            Ok(response) => {
                last_error = format!("gateway health endpoint returned {}", response.status());
            }
            Err(e) => {
                last_error = format!("gateway health endpoint failed: {e}");
            }
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let output = read_child_output(child);
            return Err(format!(
                "gateway did not become ready within {:?}. Last observed error: {}.\n{output}",
                STARTUP_TIMEOUT, last_error
            ));
        }

        sleep(POLL_INTERVAL).await;
    }
}

#[tokio::test]
async fn run_no_onboard_binds_gateway() {
    let _guard = runtime_smoke_guard().await;
    let temp = TempDir::new().expect("create temp dir");
    let port = reserve_local_port();
    let mut child = spawn_runtime(&temp, port);

    if let Err(error) = wait_for_gateway_ready(&mut child, port).await {
        panic!("{error}");
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn run_no_onboard_binds_explicit_gateway_port_3000_when_available() {
    let _guard = runtime_smoke_guard().await;
    let Ok(listener) = TcpListener::bind(("127.0.0.1", 3000)) else {
        eprintln!("skipping explicit 3000 smoke because the port is already in use");
        return;
    };
    drop(listener);

    let temp = TempDir::new().expect("create temp dir");
    let mut child = spawn_runtime(&temp, 3000);

    if let Err(error) = wait_for_gateway_ready(&mut child, 3000).await {
        panic!("{error}");
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn run_no_onboard_binds_gateway_port_zero() {
    let _guard = runtime_smoke_guard().await;
    let temp = TempDir::new().expect("create temp dir");
    let bound_addr_file = temp.path().join("gateway-bound-addr");
    let mut child = spawn_runtime_with_port_env(&temp, "0", Some(&bound_addr_file));

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    let port = loop {
        if let Some(status) = child.try_wait().expect("poll runtime") {
            let output = read_child_output(&mut child);
            panic!("runtime exited before writing bound addr ({status}).\n{output}");
        }
        if let Ok(raw) = std::fs::read_to_string(&bound_addr_file) {
            let addr: std::net::SocketAddr = raw.trim().parse().expect("parse bound addr");
            break addr.port();
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let output = read_child_output(&mut child);
            panic!(
                "runtime did not write bound addr within {:?}.\n{output}",
                STARTUP_TIMEOUT
            );
        }
        sleep(POLL_INTERVAL).await;
    };

    if let Err(error) = wait_for_gateway_ready(&mut child, port).await {
        panic!("{error}");
    }

    let _ = child.kill();
    let _ = child.wait();
}
