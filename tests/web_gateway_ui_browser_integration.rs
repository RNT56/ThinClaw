#![cfg(feature = "browser")]

mod db_contract {
    #[path = "../db_contract/fixtures.rs"]
    pub mod fixtures;
    #[path = "../db_contract/support.rs"]
    pub mod support;
}

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use serde_json::Value;
use tempfile::TempDir;
use thinclaw::agent::SessionManager;
use thinclaw::channels::web::server::{GatewayState, start_server};
use thinclaw::channels::web::sse::SseManager;
use thinclaw::channels::web::ws::WsConnectionTracker;
use thinclaw::db::Database;
use thinclaw::workspace::Workspace;

use crate::db_contract::fixtures;
use crate::db_contract::support::contract_db_or_skip;

const AUTH_TOKEN: &str = "test-browser-ui-token";
const UI_TIMEOUT: Duration = Duration::from_secs(20);

fn find_chrome() -> Option<PathBuf> {
    let candidates = if cfg!(target_os = "macos") {
        vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/snap/bin/chromium",
        ]
    } else {
        vec![
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe",
            r"C:\Program Files (x86)\BraveSoftware\Brave-Browser\Application\brave.exe",
        ]
    };

    for path in candidates {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    let path_env = std::env::var_os("PATH")?;
    let names: &[&str] = if cfg!(target_os = "windows") {
        &["chrome.exe", "msedge.exe", "brave.exe"]
    } else {
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
        ]
    };

    for dir in std::env::split_paths(&path_env) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

async fn launch_browser() -> Option<(Browser, TempDir)> {
    let chrome_path = match find_chrome() {
        Some(path) => path,
        None => {
            eprintln!("skipping browser UI integration test: Chrome/Chromium binary not found");
            return None;
        }
    };

    let profile_dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("skipping browser UI integration test: tempdir failed: {err}");
            return None;
        }
    };

    let config = match BrowserConfig::builder()
        .chrome_executable(chrome_path)
        .user_data_dir(profile_dir.path())
        .window_size(1440, 960)
        .new_headless_mode()
        .no_sandbox()
        .arg("disable-gpu")
        .arg("disable-dev-shm-usage")
        .arg("no-default-browser-check")
        .build()
    {
        Ok(config) => config,
        Err(err) => {
            eprintln!("skipping browser UI integration test: browser config failed: {err}");
            return None;
        }
    };

    let (browser, mut handler) = match Browser::launch(config).await {
        Ok(pair) => pair,
        Err(err) => {
            eprintln!("skipping browser UI integration test: browser launch failed: {err}");
            return None;
        }
    };

    tokio::spawn(async move { while handler.next().await.is_some() {} });

    Some((browser, profile_dir))
}

async fn start_ui_gateway(
    db: Arc<dyn Database>,
    user_id: &str,
    actor_id: &str,
) -> Result<SocketAddr, thinclaw::error::ChannelError> {
    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: Some(Arc::new(Workspace::new_with_db(user_id, Arc::clone(&db)))),
        session_manager: Some(Arc::new(SessionManager::new())),
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: Some(db),
        job_manager: None,
        prompt_queue: None,
        context_manager: None,
        scheduler: tokio::sync::RwLock::new(None),
        user_id: user_id.to_string(),
        actor_id: actor_id.to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        llm_runtime: None,
        skill_registry: None,
        skill_catalog: None,
        chat_rate_limiter: thinclaw::channels::web::rate_limiter::RateLimiter::new(30, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        cost_tracker: None,
        startup_time: std::time::Instant::now(),
        restart_requested: std::sync::atomic::AtomicBool::new(false),
        routine_engine: None,
        secrets_store: None,
        channel_manager: None,
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().expect("valid bind addr");
    start_server(addr, state, AUTH_TOKEN.to_string(), vec![]).await
}

async fn eval_value(page: &Page, expression: &str) -> Value {
    let result = page
        .evaluate_expression(expression)
        .await
        .expect("browser expression should evaluate");
    result.value().cloned().unwrap_or(Value::Null)
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_i64().unwrap_or_default() > 0,
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(map) => !map.is_empty(),
        Value::Null => false,
    }
}

async fn wait_for_expression(page: &Page, expression: &str, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let value = eval_value(page, expression).await;
        if is_truthy(&value) {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for browser expression: {}",
            expression
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn click_selector(page: &Page, selector: &str) {
    let selector_js = serde_json::to_string(selector).expect("selector should serialize");
    let expression = format!(
        "(() => {{ const el = document.querySelector({selector_js}); if (!el) return false; el.click(); return true; }})()"
    );
    let value = eval_value(page, &expression).await;
    assert!(
        value.as_bool().unwrap_or(false),
        "selector was not clickable: {}",
        selector
    );
}

#[tokio::test]
async fn browser_harness_loads_learning_and_outcome_backed_research_flow() {
    let Some(ctx) = contract_db_or_skip().await else {
        return;
    };
    let Some((browser, _profile_dir)) = launch_browser().await else {
        return;
    };

    let user_id = fixtures::user("browser_ui_user");
    let actor_id = "browser-ui-actor".to_string();
    ctx.db
        .set_setting(&user_id, "learning.enabled", &serde_json::json!(true))
        .await
        .expect("learning.enabled should be set");
    ctx.db
        .set_setting(
            &user_id,
            "learning.outcomes.enabled",
            &serde_json::json!(true),
        )
        .await
        .expect("learning.outcomes.enabled should be set");
    ctx.db
        .set_setting(&user_id, "experiments.enabled", &serde_json::json!(true))
        .await
        .expect("experiments.enabled should be set");

    let mut contract = fixtures::outcome_contract(&user_id);
    contract.actor_id = Some(actor_id.clone());
    contract.status = "evaluated".to_string();
    contract.final_verdict = Some("negative".to_string());
    contract.final_score = Some(-1.0);
    contract.contract_type = "tool_durability".to_string();
    contract.source_kind = "artifact_version".to_string();
    contract.summary = Some("Repeated negative outcome signal for USER.md".to_string());
    contract.metadata = serde_json::json!({
        "pattern_key": "artifact:prompt:USER.md",
        "artifact_type": "prompt",
        "artifact_name": "USER.md",
        "last_evaluator": "outcome_evaluator_v1"
    });
    contract.evaluated_at = Some(chrono::Utc::now());
    ctx.db
        .insert_outcome_contract(&contract)
        .await
        .expect("insert_outcome_contract should succeed");

    let bound_addr = start_ui_gateway(Arc::clone(&ctx.db), &user_id, &actor_id)
        .await
        .expect("gateway should start");
    let page = browser
        .new_page(format!("http://{bound_addr}/?token={AUTH_TOKEN}"))
        .await
        .expect("browser should open gateway page");

    wait_for_expression(
        &page,
        "(() => !!document.getElementById('app') && getComputedStyle(document.getElementById('app')).display !== 'none')()",
        UI_TIMEOUT,
    )
    .await;
    wait_for_expression(
        &page,
        "(() => { const btn = document.getElementById('research-tab-button'); return !!btn && getComputedStyle(btn).display !== 'none'; })()",
        UI_TIMEOUT,
    )
    .await;

    click_selector(&page, "[data-tab=\"learning\"]").await;
    wait_for_expression(
        &page,
        "(() => document.querySelectorAll('#learning-outcomes-tbody tr[data-outcome-id]').length)()",
        UI_TIMEOUT,
    )
    .await;
    click_selector(
        &page,
        "#learning-outcomes-tbody tr[data-outcome-id] button[data-action=\"view-outcome\"]",
    )
    .await;
    let detail_text = wait_for_expression(
        &page,
        "(() => { const node = document.querySelector('#learning-outcome-detail [data-outcome-detail-id]'); return node ? document.getElementById('learning-outcome-detail').innerText : ''; })()",
        UI_TIMEOUT,
    )
    .await;
    let detail_text = detail_text
        .as_str()
        .expect("detail text should be a string")
        .to_string();
    assert!(
        detail_text.contains("Repeated negative outcome signal for USER.md"),
        "expected outcome detail to render summary, got: {detail_text}"
    );
    assert!(
        detail_text.contains("Last Evaluator: outcome_evaluator_v1"),
        "expected outcome detail to render evaluator provenance, got: {detail_text}"
    );

    click_selector(&page, "#research-tab-button").await;
    wait_for_expression(
        &page,
        "(() => document.querySelectorAll('#research-opportunities-list [data-research-source=\"outcome_learning\"]').length)()",
        UI_TIMEOUT,
    )
    .await;
    let opportunity_summary = eval_value(
        &page,
        "(() => { const card = document.querySelector('#research-opportunities-list [data-research-source=\"outcome_learning\"]'); return card ? card.innerText : ''; })()",
    )
    .await;
    let opportunity_summary = opportunity_summary
        .as_str()
        .expect("opportunity summary should be a string")
        .to_string();
    assert!(
        opportunity_summary
            .to_ascii_lowercase()
            .contains("negative outcome"),
        "expected outcome-backed research card, got: {opportunity_summary}"
    );

    click_selector(
        &page,
        "#research-opportunities-list [data-research-source=\"outcome_learning\"] button[data-action=\"create-project\"]",
    )
    .await;
    let project_name = wait_for_expression(
        &page,
        "(() => document.getElementById('research-project-name')?.value || '')()",
        UI_TIMEOUT,
    )
    .await;
    let project_name = project_name
        .as_str()
        .expect("project name should be a string")
        .to_string();
    assert!(
        project_name.contains("Outcome prompt benchmark for USER.md"),
        "expected outcome-backed project hint name, got: {project_name}"
    );

    let mutable_paths = eval_value(
        &page,
        "(() => document.getElementById('research-project-mutable')?.value || '')()",
    )
    .await;
    let metric_name = eval_value(
        &page,
        "(() => document.getElementById('research-project-metric-name')?.value || '')()",
    )
    .await;
    assert_eq!(mutable_paths.as_str().unwrap_or_default(), "USER.md");
    assert_eq!(
        metric_name.as_str().unwrap_or_default(),
        "outcome_success_rate"
    );
}
