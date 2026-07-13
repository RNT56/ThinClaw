use super::RemoteGatewayProxy;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    confirm_action: bool,
    body: String,
}

async fn start_fixture_gateway(
    expected_requests: usize,
) -> (
    String,
    Arc<Mutex<Vec<RecordedRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture gateway");
    let addr = listener.local_addr().expect("fixture gateway address");
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let recorded_for_task = Arc::clone(&recorded);

    let handle = tokio::spawn(async move {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().await.expect("accept fixture request");
            let mut buffer = Vec::new();
            let headers_end = loop {
                let mut chunk = [0_u8; 1024];
                let read = stream.read(&mut chunk).await.expect("read fixture request");
                assert!(read > 0, "fixture client closed before sending headers");
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(pos) = find_headers_end(&buffer) {
                    break pos;
                }
            };

            let header_text = String::from_utf8_lossy(&buffer[..headers_end]).to_string();
            let content_length = header_text
                .lines()
                .find_map(|line| {
                    line.split_once(':').and_then(|(name, value)| {
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                })
                .unwrap_or(0);
            let body_start = headers_end + 4;
            while buffer.len() < body_start + content_length {
                let mut chunk = [0_u8; 1024];
                let read = stream.read(&mut chunk).await.expect("read fixture body");
                assert!(read > 0, "fixture client closed before sending body");
                buffer.extend_from_slice(&chunk[..read]);
            }

            let request_line = header_text.lines().next().expect("request line");
            let mut request_parts = request_line.split_whitespace();
            let method = request_parts.next().unwrap_or_default().to_string();
            let path = request_parts.next().unwrap_or_default().to_string();
            let authorization = header_text.lines().find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("authorization")
                        .then(|| value.trim().to_string())
                })
            });
            let confirm_action = header_text.lines().any(|line| {
                line.split_once(':')
                    .map(|(name, value)| {
                        name.eq_ignore_ascii_case("x-confirm-action")
                            && value.trim().eq_ignore_ascii_case("true")
                    })
                    .unwrap_or(false)
            });
            let body = String::from_utf8_lossy(&buffer[body_start..body_start + content_length])
                .to_string();

            recorded_for_task.lock().await.push(RecordedRequest {
                method: method.clone(),
                path: path.clone(),
                authorization,
                confirm_action,
                body: body.clone(),
            });

            let response = fixture_response(&method, &path, &body);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                response.len(),
                response
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write fixture response");
        }
    });

    (format!("http://{addr}"), recorded, handle)
}

fn find_headers_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn fixture_response(method: &str, path: &str, body: &str) -> String {
    match (method, path) {
        ("POST", "/api/chat/send") => serde_json::json!({
            "accepted": true,
            "thread_id": "thread-1",
            "echo": body
        }),
        ("POST", "/api/chat/abort") => serde_json::json!({ "aborted": true }),
        ("POST", "/api/chat/thread/thread-1/reset") => serde_json::json!({ "reset": true }),
        ("POST", "/api/chat/thread/thread-1/compact") => {
            serde_json::json!({ "compacted": true })
        }
        ("POST", "/api/memory/delete") => serde_json::json!({ "deleted": true }),
        ("GET", "/api/cache/stats") => serde_json::json!({
            "hits": 7,
            "misses": 2,
            "evictions": 1,
            "size": 3,
            "size_bytes": 3,
            "hit_rate": 0.777
        }),
        ("GET", "/api/logs/recent") => {
            serde_json::json!({ "logs": ["fixture log"], "lines": 1 })
        }
        ("GET", "/api/gateway/status") => serde_json::json!({ "status": "running" }),
        ("GET", "/api/hooks") => serde_json::json!({
            "total": 1,
            "hooks": [{ "name": "hook-a", "kind": "BeforeAgent", "enabled": true }]
        }),
        ("POST", "/api/hooks") => {
            let value: serde_json::Value = serde_json::from_str(body).expect("hook body json");
            serde_json::json!({
                "hooks_registered": 1,
                "webhooks_registered": 0,
                "source": value.get("source").and_then(|v| v.as_str()).unwrap_or("unknown")
            })
        }
        ("DELETE", "/api/hooks/hook-a") => serde_json::json!({ "removed": true }),
        ("GET", "/api/providers") => serde_json::json!({
            "providers": [{ "slug": "openai", "has_key": true }]
        }),
        ("GET", "/api/providers/config") => serde_json::json!({
            "primary": { "provider": "openai", "model": "gpt-5.1" },
            "routing": {
                "primary_pool": ["openai/gpt-5.1"],
                "cheap_pool": ["openai/gpt-5.1-mini"],
                "advisor_ready": true,
                "rules": []
            }
        }),
        ("PUT", "/api/providers/config") => serde_json::json!({ "saved": true }),
        ("GET", "/api/providers/openai/models") => serde_json::json!({
            "models": [{ "id": "gpt-5.1", "supports_tools": true }]
        }),
        ("POST", "/api/providers/route/simulate") => serde_json::json!({
            "target": "openai/gpt-5.1",
            "fallback_chain": ["openai/gpt-5.1-mini"],
            "advisor_ready": true,
            "diagnostics": ["fixture route"]
        }),
        ("POST", "/api/providers/openai/key") => serde_json::json!({ "saved": true }),
        ("DELETE", "/api/providers/openai/key") => serde_json::json!({ "deleted": true }),
        ("GET", "/api/costs/summary") => serde_json::json!({ "total_usd": 1.25 }),
        ("GET", "/api/costs/export") => return "model,cost_usd\nopenai/gpt-5.1,1.25\n".to_string(),
        ("POST", "/api/costs/reset") => serde_json::json!({ "reset": true }),
        ("GET", "/api/jobs") => serde_json::json!({
            "jobs": [{ "id": "job-1", "state": "running" }],
            "capabilities": { "detail": true, "restart": true, "prompt": true, "files": true }
        }),
        ("GET", "/api/jobs/summary") => serde_json::json!({ "total": 1, "in_progress": 1 }),
        ("GET", "/api/jobs/job-1") => serde_json::json!({ "id": "job-1", "state": "running" }),
        ("POST", "/api/jobs/job-1/cancel") => serde_json::json!({ "cancelled": true }),
        ("POST", "/api/jobs/job-1/restart") => serde_json::json!({ "restarted": true }),
        ("POST", "/api/jobs/job-1/prompt") => serde_json::json!({ "accepted": true }),
        ("GET", "/api/jobs/job-1/events") => serde_json::json!({
            "events": [{ "kind": "started" }],
            "events_available": true
        }),
        ("GET", "/api/jobs/job-1/files/list?path=src") => serde_json::json!({
            "files": [{ "path": "src/main.rs", "kind": "file" }]
        }),
        ("GET", "/api/jobs/job-1/files/read?path=src%2Fmain.rs") => {
            serde_json::json!({ "path": "src/main.rs", "content": "fn main() {}" })
        }
        ("GET", "/api/autonomy/status") => serde_json::json!({ "enabled": false }),
        ("POST", "/api/autonomy/bootstrap") => serde_json::json!({ "bootstrapped": true }),
        ("POST", "/api/autonomy/pause") => serde_json::json!({ "paused": true }),
        ("POST", "/api/autonomy/resume") => serde_json::json!({ "resumed": true }),
        ("GET", "/api/autonomy/permissions") => serde_json::json!({
            "allowed": false,
            "reason": "fixture host permissions missing"
        }),
        ("POST", "/api/autonomy/rollback") => serde_json::json!({ "rolled_back": true }),
        ("GET", "/api/autonomy/rollouts") => serde_json::json!({ "rollouts": [] }),
        ("GET", "/api/autonomy/checks") => serde_json::json!({ "checks": [] }),
        ("GET", "/api/autonomy/evidence") => serde_json::json!({ "evidence": [] }),
        ("GET", "/api/learning/status?limit=5") => serde_json::json!({ "available": true }),
        ("GET", "/api/learning/provider-health") => serde_json::json!({ "providers": [] }),
        ("POST", "/api/learning/code-proposals/proposal-1/review") => {
            serde_json::json!({ "reviewed": true })
        }
        ("GET", "/api/experiments/projects") => serde_json::json!({ "projects": [] }),
        ("GET", "/api/experiments/campaigns") => serde_json::json!({ "campaigns": [] }),
        ("GET", "/api/experiments/campaigns/campaign-1/trials") => {
            serde_json::json!({ "trials": [] })
        }
        ("POST", "/api/experiments/campaigns/campaign-1/pause") => {
            serde_json::json!({ "paused": true })
        }
        ("POST", "/api/experiments/providers/gpu-clouds/runpod/validate") => {
            serde_json::json!({ "valid": true })
        }
        ("GET", "/api/mcp/servers") => serde_json::json!({ "servers": [] }),
        ("GET", "/api/mcp/interactions") => serde_json::json!({ "interactions": [] }),
        ("POST", "/api/skills/install") => serde_json::json!({ "installed": true }),
        _ if method == "GET" && path.starts_with("/api/chat/thread/thread-1/export?") => {
            serde_json::json!({ "format": "markdown", "content": "fixture transcript" })
        }
        _ => panic!("unexpected fixture route: {method} {path}"),
    }
    .to_string()
}

#[test]
fn unavailable_errors_are_explicitly_typed_by_prefix() {
    let error = RemoteGatewayProxy::unavailable("chat abort", "no endpoint");
    assert!(matches!(
        error,
        crate::thinclaw::bridge::BridgeError::Unavailable {
            capability,
            reason,
            remediation: Some(remediation),
            ..
        } if capability == "chat abort"
            && reason.contains("no endpoint")
            && remediation.contains("upgrade the remote gateway")
    ));
}

#[test]
fn constructor_normalizes_trailing_slash() {
    let proxy = RemoteGatewayProxy::new("http://127.0.0.1:18789/", "token")
        .expect("valid private gateway URL");
    assert_eq!(proxy.base_url(), "http://127.0.0.1:18789");
}

#[test]
fn constructor_rejects_missing_credentials_and_unsafe_transport() {
    assert!(RemoteGatewayProxy::new("http://127.0.0.1:18789", "").is_err());
    assert!(RemoteGatewayProxy::new("http://gateway.example.com:18789", "token").is_err());
    assert!(RemoteGatewayProxy::new("https://gateway.example.com/api", "token").is_err());
    assert!(RemoteGatewayProxy::new("https://user@gateway.example.com", "token").is_err());

    assert!(RemoteGatewayProxy::new("https://gateway.example.com", "token").is_ok());
    assert!(RemoteGatewayProxy::new("http://100.100.1.2:18789", "token").is_ok());
    assert!(RemoteGatewayProxy::new("http://agent.tailnet.ts.net:18789", "token").is_ok());
}

#[tokio::test]
async fn raw_secret_injection_is_unavailable_in_remote_mode() {
    let proxy =
        RemoteGatewayProxy::new("http://127.0.0.1:18789", "token").expect("valid fixture proxy");
    let error = proxy
        .inject_secrets(&std::collections::HashMap::new())
        .await
        .expect_err("remote raw secret injection should stay disabled");

    assert!(matches!(
        error,
        crate::thinclaw::bridge::BridgeError::Unavailable {
            capability,
            reason,
            remediation: Some(remediation),
            ..
        } if capability == "remote raw secret injection"
            && reason.contains("raw secret injection is disabled")
            && remediation.contains("provider vault save/delete")
    ));
}

#[tokio::test]
async fn health_check_proves_the_bearer_credential_on_an_authenticated_route() {
    let (base_url, recorded, fixture) = start_fixture_gateway(1).await;
    let proxy = RemoteGatewayProxy::new(&base_url, "fixture-token").expect("valid fixture proxy");

    assert!(proxy
        .health_check()
        .await
        .expect("authenticated health check"));
    fixture.await.expect("fixture gateway task");

    let requests = recorded.lock().await;
    assert_eq!(requests[0].path, "/api/gateway/status");
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer fixture-token")
    );
}

#[tokio::test]
async fn fixture_acceptance_remote_chat_and_session_routes() {
    let (base_url, recorded, server) = start_fixture_gateway(11).await;
    let proxy = RemoteGatewayProxy::new(&base_url, "fixture-token").expect("valid fixture proxy");

    let sent = proxy
        .send_message("thread-1", "fixture message")
        .await
        .expect("send message");
    assert_eq!(sent["accepted"], true);
    proxy.abort_chat("thread-1").await.expect("abort chat");
    proxy
        .reset_session("thread-1")
        .await
        .expect("reset session");
    let compact = proxy
        .compact_session("thread-1")
        .await
        .expect("compact session");
    assert_eq!(compact["compacted"], true);
    let transcript = proxy
        .export_session("thread-1", "markdown")
        .await
        .expect("export session");
    assert_eq!(transcript["content"], "fixture transcript");
    proxy
        .delete_file("notes/one.md")
        .await
        .expect("memory delete");
    let cache = proxy.cache_stats().await.expect("cache stats");
    assert_eq!(cache["hits"], 7);
    let logs = proxy.logs_recent().await.expect("recent logs");
    assert_eq!(logs["logs"][0], "fixture log");
    let hooks = proxy.list_hooks().await.expect("hooks list");
    assert_eq!(hooks["total"], 1);
    let registered = proxy
        .register_hooks(r#"{"rules":[]}"#, Some("fixture"))
        .await
        .expect("hooks register");
    assert_eq!(registered["hooks_registered"], 1);
    let removed = proxy
        .unregister_hook("hook-a")
        .await
        .expect("hooks unregister");
    assert_eq!(removed["removed"], true);

    server.await.expect("fixture server completes");
    let recorded = recorded.lock().await;
    let route_pairs = recorded
        .iter()
        .map(|request| (request.method.as_str(), request.path.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        route_pairs,
        vec![
            ("POST", "/api/chat/send"),
            ("POST", "/api/chat/abort"),
            ("POST", "/api/chat/thread/thread-1/reset"),
            ("POST", "/api/chat/thread/thread-1/compact"),
            ("GET", "/api/chat/thread/thread-1/export?format=markdown"),
            ("POST", "/api/memory/delete"),
            ("GET", "/api/cache/stats"),
            ("GET", "/api/logs/recent"),
            ("GET", "/api/hooks"),
            ("POST", "/api/hooks"),
            ("DELETE", "/api/hooks/hook-a"),
        ]
    );
    assert!(
        recorded
            .iter()
            .all(|request| request.authorization.as_deref() == Some("Bearer fixture-token")),
        "every fixture request should carry bearer auth"
    );
    assert!(recorded[0].body.contains("\"content\":\"fixture message\""));
    assert!(recorded[1].body.contains("\"thread_id\":\"thread-1\""));
    assert!(recorded[5].body.contains("\"path\":\"notes/one.md\""));
    assert!(recorded[9].body.contains("\"source\":\"fixture\""));
}

#[tokio::test]
async fn fixture_acceptance_remote_management_routes() {
    let (base_url, recorded, server) = start_fixture_gateway(39).await;
    let proxy = RemoteGatewayProxy::new(&base_url, "fixture-token").expect("valid fixture proxy");

    let providers = proxy.list_provider_status().await.expect("provider status");
    assert_eq!(providers["providers"][0]["slug"], "openai");
    let config = proxy.get_providers_config().await.expect("provider config");
    assert_eq!(config["routing"]["advisor_ready"], true);
    proxy
        .set_providers_config(&serde_json::json!({
            "primary": { "provider": "openai", "model": "gpt-5.1" },
            "routing": {
                "primary_pool": ["openai/gpt-5.1"],
                "cheap_pool": ["openai/gpt-5.1-mini"]
            }
        }))
        .await
        .expect("set provider config");
    let models = proxy.get_provider_models("openai").await.expect("models");
    assert_eq!(models["models"][0]["id"], "gpt-5.1");
    let route = proxy
        .simulate_route(&serde_json::json!({ "prompt": "use tools" }))
        .await
        .expect("route simulation");
    assert_eq!(route["target"], "openai/gpt-5.1");
    assert_eq!(
        proxy
            .save_provider_key("openai", "sk-fixture")
            .await
            .expect("save key")["saved"],
        true
    );
    proxy
        .delete_provider_key("openai")
        .await
        .expect("delete key");

    assert_eq!(
        proxy.get_cost_summary().await.expect("cost summary")["total_usd"],
        1.25
    );
    assert!(proxy
        .export_cost_csv()
        .await
        .expect("cost export")
        .contains("model,cost_usd"));
    proxy.reset_costs().await.expect("reset costs");

    assert_eq!(
        proxy.get_jobs().await.expect("jobs")["jobs"][0]["id"],
        "job-1"
    );
    assert_eq!(
        proxy.get_jobs_summary().await.expect("job summary")["total"],
        1
    );
    assert_eq!(
        proxy.get_job_detail("job-1").await.expect("job detail")["state"],
        "running"
    );
    assert_eq!(
        proxy.cancel_job("job-1").await.expect("cancel job")["cancelled"],
        true
    );
    assert_eq!(
        proxy.restart_job("job-1").await.expect("restart job")["restarted"],
        true
    );
    assert_eq!(
        proxy
            .prompt_job("job-1", Some("continue".into()), false)
            .await
            .expect("prompt job")["accepted"],
        true
    );
    assert_eq!(
        proxy.get_job_events("job-1").await.expect("job events")["events_available"],
        true
    );
    assert_eq!(
        proxy
            .list_job_files("job-1", Some("src"))
            .await
            .expect("job files")["files"][0]["path"],
        "src/main.rs"
    );
    assert_eq!(
        proxy
            .read_job_file("job-1", "src/main.rs")
            .await
            .expect("job file")["content"],
        "fn main() {}"
    );

    assert_eq!(
        proxy.get_autonomy_status().await.expect("autonomy status")["enabled"],
        false
    );
    assert_eq!(
        proxy
            .bootstrap_autonomy()
            .await
            .expect("autonomy bootstrap")["bootstrapped"],
        true
    );
    assert_eq!(
        proxy
            .pause_autonomy(Some("fixture".into()))
            .await
            .expect("autonomy pause")["paused"],
        true
    );
    assert_eq!(
        proxy.resume_autonomy().await.expect("autonomy resume")["resumed"],
        true
    );
    assert_eq!(
        proxy
            .get_autonomy_permissions()
            .await
            .expect("autonomy permissions")["allowed"],
        false
    );
    assert_eq!(
        proxy.rollback_autonomy().await.expect("autonomy rollback")["rolled_back"],
        true
    );
    assert!(proxy
        .get_autonomy_rollouts()
        .await
        .expect("autonomy rollouts")["rollouts"]
        .is_array());
    assert!(proxy.get_autonomy_checks().await.expect("autonomy checks")["checks"].is_array());
    assert!(proxy
        .get_autonomy_evidence()
        .await
        .expect("autonomy evidence")["evidence"]
        .is_array());

    assert_eq!(
        proxy
            .get_json("/api/learning/status?limit=5")
            .await
            .expect("learning status")["available"],
        true
    );
    assert!(proxy
        .get_json("/api/learning/provider-health")
        .await
        .expect("learning provider health")["providers"]
        .is_array());
    assert_eq!(
        proxy
            .post_json(
                "/api/learning/code-proposals/proposal-1/review",
                &serde_json::json!({ "decision": "approve", "note": "fixture" })
            )
            .await
            .expect("learning review")["reviewed"],
        true
    );

    assert!(proxy
        .get_experiment_projects()
        .await
        .expect("experiment projects")["projects"]
        .is_array());
    assert!(proxy
        .get_json("/api/experiments/campaigns")
        .await
        .expect("experiment campaigns")["campaigns"]
        .is_array());
    assert!(proxy
        .get_json("/api/experiments/campaigns/campaign-1/trials")
        .await
        .expect("experiment trials")["trials"]
        .is_array());
    assert_eq!(
        proxy
            .post_json(
                "/api/experiments/campaigns/campaign-1/pause",
                &serde_json::json!({})
            )
            .await
            .expect("experiment campaign action")["paused"],
        true
    );
    assert_eq!(
        proxy
            .post_json(
                "/api/experiments/providers/gpu-clouds/runpod/validate",
                &serde_json::json!({})
            )
            .await
            .expect("gpu validate")["valid"],
        true
    );

    assert!(proxy.get_mcp_servers().await.expect("mcp servers")["servers"].is_array());
    assert!(proxy
        .get_json("/api/mcp/interactions")
        .await
        .expect("mcp interactions")["interactions"]
        .is_array());
    assert_eq!(
        proxy
            .post_json_confirm(
                "/api/skills/install",
                &serde_json::json!({ "name": "fixture-skill" })
            )
            .await
            .expect("skill install")["installed"],
        true
    );

    server.await.expect("fixture server completes");
    let recorded = recorded.lock().await;
    let route_pairs = recorded
        .iter()
        .map(|request| (request.method.as_str(), request.path.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        route_pairs,
        vec![
            ("GET", "/api/providers"),
            ("GET", "/api/providers/config"),
            ("PUT", "/api/providers/config"),
            ("GET", "/api/providers/openai/models"),
            ("POST", "/api/providers/route/simulate"),
            ("POST", "/api/providers/openai/key"),
            ("DELETE", "/api/providers/openai/key"),
            ("GET", "/api/costs/summary"),
            ("GET", "/api/costs/export"),
            ("POST", "/api/costs/reset"),
            ("GET", "/api/jobs"),
            ("GET", "/api/jobs/summary"),
            ("GET", "/api/jobs/job-1"),
            ("POST", "/api/jobs/job-1/cancel"),
            ("POST", "/api/jobs/job-1/restart"),
            ("POST", "/api/jobs/job-1/prompt"),
            ("GET", "/api/jobs/job-1/events"),
            ("GET", "/api/jobs/job-1/files/list?path=src"),
            ("GET", "/api/jobs/job-1/files/read?path=src%2Fmain.rs"),
            ("GET", "/api/autonomy/status"),
            ("POST", "/api/autonomy/bootstrap"),
            ("POST", "/api/autonomy/pause"),
            ("POST", "/api/autonomy/resume"),
            ("GET", "/api/autonomy/permissions"),
            ("POST", "/api/autonomy/rollback"),
            ("GET", "/api/autonomy/rollouts"),
            ("GET", "/api/autonomy/checks"),
            ("GET", "/api/autonomy/evidence"),
            ("GET", "/api/learning/status?limit=5"),
            ("GET", "/api/learning/provider-health"),
            ("POST", "/api/learning/code-proposals/proposal-1/review"),
            ("GET", "/api/experiments/projects"),
            ("GET", "/api/experiments/campaigns"),
            ("GET", "/api/experiments/campaigns/campaign-1/trials"),
            ("POST", "/api/experiments/campaigns/campaign-1/pause"),
            (
                "POST",
                "/api/experiments/providers/gpu-clouds/runpod/validate"
            ),
            ("GET", "/api/mcp/servers"),
            ("GET", "/api/mcp/interactions"),
            ("POST", "/api/skills/install"),
        ]
    );
    assert!(
        recorded
            .iter()
            .all(|request| request.authorization.as_deref() == Some("Bearer fixture-token")),
        "every management fixture request should carry bearer auth"
    );
    assert!(
        recorded
            .iter()
            .any(|request| request.path == "/api/skills/install" && request.confirm_action),
        "mutating skill install should carry x-confirm-action"
    );
    assert!(recorded[2].body.contains("\"cheap_pool\""));
    assert!(recorded[5].body.contains("\"api_key\":\"sk-fixture\""));
    assert!(recorded[15].body.contains("\"content\":\"continue\""));
    assert!(recorded[21].body.contains("\"reason\":\"fixture\""));
}
