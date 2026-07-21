use super::*;

struct TestDockerRuntime {
    debug_port: u16,
}

#[async_trait]
impl BrowserDockerRuntime for TestDockerRuntime {
    fn image_label(&self) -> String {
        "test/chromium".to_string()
    }

    fn http_endpoint(&self) -> String {
        format!("http://127.0.0.1:{}", self.debug_port)
    }

    fn is_available(&self) -> bool {
        true
    }

    async fn start(&self) -> Result<(), String> {
        Ok(())
    }

    async fn wait_for_ready(&self, _timeout: Duration) -> Result<(), String> {
        Ok(())
    }

    async fn stop(&self) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn test_navigation_guard() {
    assert!(is_url_allowed("https://example.com").is_ok());
    assert!(is_url_allowed("https://google.com/search?q=test").is_ok());
    assert!(is_url_allowed("file:///etc/passwd").is_err());
    assert!(is_url_allowed("chrome://settings").is_err());
    assert!(is_url_allowed("http://localhost:3000").is_err());
    assert!(is_url_allowed("http://127.0.0.1:8080").is_err());
    assert!(is_url_allowed("http://[::1]/").is_err());
    assert!(is_url_allowed("http://10.0.0.1/").is_err());
    assert!(is_url_allowed("http://100.64.0.1/").is_err());
    assert!(is_url_allowed("http://169.254.169.254/latest/meta-data/").is_err());
    assert!(is_url_allowed("https://service.internal/").is_err());
    assert!(is_url_allowed("https://printer.local/").is_err());
    assert!(is_url_allowed("https://user:secret@example.com/").is_err());
    assert!(is_url_allowed("http://example.com:22").is_err());
    assert!(is_url_allowed("http://example.com:8080").is_err());
    assert!(is_url_allowed("https://example.com:8443").is_err());
    assert!(is_url_allowed("ftp://example.com/file").is_err());
}

#[test]
fn browser_proxy_config_requires_authenticated_loopback_endpoint() {
    let valid = BrowserProxyConfig {
        endpoint: "http://127.0.0.1:49152".to_string(),
        username: "thinclaw".to_string(),
        password: "one-time-secret".to_string(),
    };
    assert!(valid.validate().is_ok());

    for endpoint in [
        "http://localhost:49152",
        "http://0.0.0.0:49152",
        "https://127.0.0.1:49152",
        "http://user:password@127.0.0.1:49152",
        "http://127.0.0.1:49152/path",
        "http://127.0.0.1:49152/?query=1",
    ] {
        let mut invalid = valid.clone();
        invalid.endpoint = endpoint.to_string();
        assert!(
            invalid.validate().is_err(),
            "unexpectedly trusted {endpoint}"
        );
    }

    let mut invalid = valid;
    invalid.password = "contains\ncontrol".to_string();
    assert!(invalid.validate().is_err());
}

#[test]
fn evaluated_page_values_are_bounded() {
    let hostile = serde_json::Value::Array(
        (0..1_000)
            .map(|index| {
                serde_json::json!({
                    format!("page-key-{index}-{}", "k".repeat(2_000)):
                        "\"\\\n".repeat(2_000),
                })
            })
            .collect(),
    );
    let (bounded, truncated) = bound_json_value(hostile);
    let encoded = serde_json::to_vec(&bounded).unwrap();
    assert!(truncated);
    assert!(encoded.len() <= MAX_EVALUATE_RESULT_BYTES);
}

#[test]
fn browser_session_scope_isolates_principals_and_unthreaded_jobs() {
    let conversation_id = uuid::Uuid::new_v4();
    let mut first = JobContext::with_identity("principal-a", "actor-a", "test", "test");
    first.conversation_id = Some(conversation_id);
    let mut follow_up = JobContext::with_identity("principal-a", "actor-b", "test", "test");
    follow_up.conversation_id = Some(conversation_id);
    let mut attacker = JobContext::with_identity("principal-b", "actor-a", "test", "test");
    attacker.conversation_id = Some(conversation_id);

    assert_eq!(
        BrowserSessionScope::from_context(&first).0,
        BrowserSessionScope::from_context(&follow_up).0
    );
    assert_ne!(
        BrowserSessionScope::from_context(&first).0,
        BrowserSessionScope::from_context(&attacker).0
    );

    let unthreaded_one = JobContext::with_user("principal-a", "test", "test");
    let unthreaded_two = JobContext::with_user("principal-a", "test", "test");
    assert_ne!(
        BrowserSessionScope::from_context(&unthreaded_one).0,
        BrowserSessionScope::from_context(&unthreaded_two).0
    );
}

#[tokio::test]
async fn network_guard_rejects_non_public_literals_without_dns() {
    assert!(is_network_url_allowed("http://127.0.0.1/").await.is_err());
    assert!(is_network_url_allowed("ws://[::1]/socket").await.is_err());
    assert!(is_network_url_allowed("data:text/plain,ok").await.is_ok());
}

#[test]
fn test_interactive_roles() {
    assert!(is_interactive_role("button"));
    assert!(is_interactive_role("textbox"));
    assert!(is_interactive_role("link"));
    assert!(!is_interactive_role("generic"));
    assert!(!is_interactive_role("heading"));
    assert!(!is_interactive_role("paragraph"));
}

#[test]
fn test_browser_tool_schema() {
    let tool = BrowserTool::new(PathBuf::from("/tmp/test-browser"));
    assert_eq!(tool.name(), "browser");

    let schema = tool.parameters_schema();
    let action = schema["properties"]["action"].clone();
    assert!(action["enum"].as_array().unwrap().len() >= 7);
}

#[test]
fn test_execution_timeout_override() {
    let tool = BrowserTool::new(PathBuf::from("/tmp/test-browser"));
    assert_eq!(tool.execution_timeout(), Duration::from_secs(120));
}

#[test]
fn test_new_with_docker() {
    let docker_config = Arc::new(TestDockerRuntime { debug_port: 9222 });
    let tool =
        BrowserTool::new_with_docker(PathBuf::from("/tmp/test-browser"), docker_config.clone());
    assert_eq!(tool.name(), "browser");
    assert!(tool.docker_config.is_some());
    assert_eq!(
        tool.docker_config.unwrap().http_endpoint(),
        "http://127.0.0.1:9222"
    );
}

#[test]
fn test_new_without_docker() {
    let tool = BrowserTool::new(PathBuf::from("/tmp/test-browser"));
    assert!(tool.docker_config.is_none());
}

#[test]
fn test_new_with_cloud_provider() {
    let tool = BrowserTool::new_with_cloud(
        PathBuf::from("/tmp/test-browser"),
        Some("browser_use".to_string()),
    );
    assert_eq!(tool.cloud_provider.as_deref(), Some("browser_use"));
}
