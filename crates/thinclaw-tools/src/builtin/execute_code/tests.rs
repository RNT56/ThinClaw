use super::*;
use crate::builtin::{ReadFileTool, WriteFileTool};

fn missing_tool_runtime(error: &ToolError) -> bool {
    let text = error.to_string().to_ascii_lowercase();
    text.contains("no such file")
        || text.contains("not found")
        || text.contains("node")
        || text.contains("npx")
        || text.contains("tsx")
}

/// Minimal backend used purely to exercise approval policy by backend kind.
/// Execution methods are never invoked by `requires_approval`.
struct KindOnlyBackend(ExecutionBackendKind);

#[async_trait]
impl LocalExecutionBackend for KindOnlyBackend {
    fn kind(&self) -> ExecutionBackendKind {
        self.0
    }

    async fn run_shell(
        &self,
        _request: crate::execution::CommandExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        unimplemented!("not used in approval-policy tests")
    }

    async fn start_process(
        &self,
        _request: crate::execution::ProcessStartRequest,
    ) -> Result<crate::execution::StartedProcess, ToolError> {
        unimplemented!("not used in approval-policy tests")
    }

    async fn run_script(
        &self,
        _request: ScriptExecutionRequest,
    ) -> Result<ExecutionResult, ToolError> {
        unimplemented!("not used in approval-policy tests")
    }
}

#[test]
fn test_requires_approval_bare_host_forces_always() {
    // Default backend is the bare host (LocalHostExecutionBackend).
    let tool = ExecuteCodeTool::new();
    assert_eq!(tool.backend.kind(), ExecutionBackendKind::LocalHost);
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Always,
        "bare-host code execution must always require explicit approval"
    );
}

#[test]
fn test_requires_approval_remote_runner_forces_always() {
    let tool = ExecuteCodeTool::new().with_backend(Arc::new(KindOnlyBackend(
        ExecutionBackendKind::RemoteRunnerAdapter,
    )));
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::Always,
        "remote-runner execution without advertised isolation must require approval"
    );
}

#[test]
fn test_requires_approval_docker_sandbox_allows_auto_approval() {
    let tool = ExecuteCodeTool::new().with_backend(Arc::new(KindOnlyBackend(
        ExecutionBackendKind::DockerSandbox,
    )));
    assert_eq!(
        tool.requires_approval(&serde_json::json!({})),
        ApprovalRequirement::UnlessAutoApproved,
        "container-isolated execution keeps the auto-approval policy"
    );
}

#[test]
fn test_interpreter_for() {
    assert_eq!(
        ExecuteCodeTool::interpreter_for("python")
            .unwrap()
            .extension,
        ".py"
    );
    assert_eq!(
        ExecuteCodeTool::interpreter_for("javascript")
            .unwrap()
            .program,
        "node"
    );
    if cfg!(target_os = "windows") {
        assert_eq!(
            ExecuteCodeTool::interpreter_for("cmd").unwrap().extension,
            ".cmd"
        );
    } else {
        assert_eq!(
            ExecuteCodeTool::interpreter_for("bash").unwrap().program,
            "bash"
        );
    }
    assert!(ExecuteCodeTool::interpreter_for("cobol").is_err());
}

#[test]
fn test_interpreter_args() {
    let args = ExecuteCodeTool::interpreter_args(
        &ExecuteCodeTool::interpreter_for("python").unwrap(),
        Path::new("/tmp/test.py"),
    );
    assert_eq!(args, vec!["/tmp/test.py".to_string()]);
}

#[test]
fn test_tool_rpc_allowlist_blocks_nested_execute_code() {
    assert!(!tool_rpc_allows("execute_code", &serde_json::json!({})));
    assert!(tool_rpc_allows(
        "http",
        &serde_json::json!({"method": "GET", "url": "https://example.com"})
    ));
    assert!(!tool_rpc_allows(
        "http",
        &serde_json::json!({"method": "POST", "url": "https://example.com"})
    ));
}

#[test]
fn test_tool_rpc_auto_approval_policy_is_read_only() {
    assert!(tool_rpc_auto_approves("read_file", &serde_json::json!({})));
    assert!(tool_rpc_auto_approves(
        "memory_read",
        &serde_json::json!({})
    ));
    assert!(!tool_rpc_auto_approves(
        "write_file",
        &serde_json::json!({})
    ));
    assert!(!tool_rpc_auto_approves(
        "memory_write",
        &serde_json::json!({})
    ));
    assert!(tool_rpc_auto_approves(
        "http",
        &serde_json::json!({"method": "HEAD"})
    ));
    assert!(!tool_rpc_auto_approves(
        "http",
        &serde_json::json!({"method": "PATCH"})
    ));
}

#[tokio::test]
async fn test_execute_python_subprocess() {
    let tool = ExecuteCodeTool::new();
    let ctx = JobContext::default();

    let result = tool
        .execute(
            serde_json::json!({
                "language": "python",
                "code": "print('hello from python')"
            }),
            &ctx,
        )
        .await
        .expect("python subprocess execution should succeed");

    assert!(
        result
            .result
            .get("output")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("hello from python")
    );
}

#[tokio::test]
async fn test_execute_python_subprocess_resolves_project_local_imports() {
    let temp_dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        temp_dir.path().join("helper_mod.py"),
        "VALUE = 'workspace import'",
    )
    .await
    .unwrap();

    let tool = ExecuteCodeTool::new().with_working_dir(temp_dir.path().to_path_buf());
    let ctx = JobContext::default();
    let result = tool
        .execute(
            serde_json::json!({
                "language": "python",
                "code": "import helper_mod\nprint(helper_mod.VALUE)"
            }),
            &ctx,
        )
        .await
        .expect("python subprocess should import local workspace modules");

    assert!(
        result
            .result
            .get("output")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.contains("workspace import"))
    );
}

#[tokio::test]
async fn test_tool_rpc_requires_registry() {
    let tool = ExecuteCodeTool::new();
    let ctx = JobContext::default();
    let result = tool
        .execute(
            serde_json::json!({
                "language": "python",
                "mode": "tool_rpc",
                "code": "print('hello')"
            }),
            &ctx,
        )
        .await;
    assert!(result.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rpc_can_read_file() {
    let temp_dir = tempfile::tempdir().unwrap();
    let target = temp_dir.path().join("hello.txt");
    tokio::fs::write(&target, "hello from file").await.unwrap();

    let registry = Arc::new(ToolRegistry::new());
    registry.register_sync(Arc::new(ReadFileTool::new()));
    registry.register_sync(Arc::new(WriteFileTool::new()));

    let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
    let ctx = JobContext::default();
    let result = tool
        .execute(
            serde_json::json!({
                "language": "python",
                "mode": "tool_rpc",
                "code": format!(
                    "from thinclaw_tools import read_file\nprint(read_file(path='{}')['content'])",
                    target.display()
                )
            }),
            &ctx,
        )
        .await
        .expect("tool_rpc file read should succeed");

    let text = result.result.get("output").unwrap().as_str().unwrap();
    assert!(text.contains("hello from file"));
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rpc_json_result_format_returns_output_json() {
    let registry = Arc::new(ToolRegistry::new());
    registry.register_sync(Arc::new(ReadFileTool::new()));

    let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
    let ctx = JobContext::default();
    let result = tool
        .execute(
            serde_json::json!({
                "language": "python",
                "mode": "tool_rpc",
                "result_format": "json",
                "code": "import json\nprint(json.dumps({'status': 'ok', 'count': 2}))"
            }),
            &ctx,
        )
        .await
        .expect("tool_rpc json result should succeed");

    assert_eq!(
        result.result.get("mode").and_then(|v| v.as_str()),
        Some("tool_rpc")
    );
    assert_eq!(
        result.result.get("runtime_family").and_then(|v| v.as_str()),
        Some("execution_backend")
    );
    assert_eq!(
        result.result.get("runtime_mode").and_then(|v| v.as_str()),
        Some("tool_rpc_python")
    );
    assert_eq!(
        result
            .result
            .get("network_isolation")
            .and_then(|v| v.as_str()),
        Some(host_local_network_isolation(false).as_str())
    );
    assert_eq!(
        result.result.get("result_format").and_then(|v| v.as_str()),
        Some("json")
    );
    assert_eq!(
        result
            .result
            .get("output_json")
            .and_then(|v| v.get("status"))
            .and_then(|v| v.as_str()),
        Some("ok")
    );
    assert_eq!(
        result
            .result
            .get("output_json")
            .and_then(|v| v.get("count"))
            .and_then(|v| v.as_i64()),
        Some(2)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rpc_javascript_supports_host_tools_and_workspace_require() {
    let temp_dir = tempfile::tempdir().unwrap();
    let target = temp_dir.path().join("hello.txt");
    let helper = temp_dir.path().join("helper_js.cjs");
    tokio::fs::write(&target, "hello from javascript")
        .await
        .unwrap();
    tokio::fs::write(
        &helper,
        "module.exports = { VALUE: 'workspace js module' };",
    )
    .await
    .unwrap();

    let registry = Arc::new(ToolRegistry::new());
    registry.register_sync(Arc::new(ReadFileTool::new()));

    let tool = ExecuteCodeTool::new()
        .with_working_dir(temp_dir.path().to_path_buf())
        .with_tool_registry(Arc::downgrade(&registry));
    let ctx = JobContext::default();
    let result = match tool
        .execute(
            serde_json::json!({
                "language": "javascript",
                "mode": "tool_rpc",
                "result_format": "json",
                "code": format!(
                    "const helper = require('./{}');\nconst file = await read_file({{ path: '{}' }});\nconst normalized = file.content.replace(/^\\s*\\d+\\u2502\\s?/m, '').trim();\nconsole.log(JSON.stringify({{ status: 'ok', helper: helper.VALUE, content: normalized }}));",
                    helper.file_name().unwrap().to_string_lossy(),
                    target.display()
                )
            }),
            &ctx,
        )
        .await
    {
        Ok(result) => result,
        Err(error) if missing_tool_runtime(&error) => {
            eprintln!("skipping javascript tool_rpc test: {error}");
            return;
        }
        Err(error) => panic!("javascript tool_rpc should succeed: {error}"),
    };

    assert_eq!(
        result
            .result
            .get("runtime_mode")
            .and_then(|value| value.as_str()),
        Some("tool_rpc_javascript")
    );
    assert_eq!(
        result
            .result
            .get("output_json")
            .and_then(|value| value.get("helper"))
            .and_then(|value| value.as_str()),
        Some("workspace js module")
    );
    assert_eq!(
        result
            .result
            .get("output_json")
            .and_then(|value| value.get("content"))
            .and_then(|value| value.as_str()),
        Some("hello from javascript")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rpc_typescript_supports_workspace_require() {
    let temp_dir = tempfile::tempdir().unwrap();
    let helper = temp_dir.path().join("helper_ts.cjs");
    tokio::fs::write(
        &helper,
        "module.exports = { VALUE: 'workspace ts module' };",
    )
    .await
    .unwrap();

    let registry = Arc::new(ToolRegistry::new());
    let tool = ExecuteCodeTool::new()
        .with_working_dir(temp_dir.path().to_path_buf())
        .with_tool_registry(Arc::downgrade(&registry));
    let ctx = JobContext::default();
    let result = match tool
        .execute(
            serde_json::json!({
                "language": "typescript",
                "mode": "tool_rpc",
                "result_format": "json",
                "code": format!(
                    "const helper = require('./{}');\nconsole.log(JSON.stringify({{ status: 'ok', helper: helper.VALUE }}));",
                    helper.file_name().unwrap().to_string_lossy()
                )
            }),
            &ctx,
        )
        .await
    {
        Ok(result) => result,
        Err(error) if missing_tool_runtime(&error) => {
            eprintln!("skipping typescript tool_rpc test: {error}");
            return;
        }
        Err(error) => panic!("typescript tool_rpc should succeed: {error}"),
    };

    assert_eq!(
        result
            .result
            .get("runtime_mode")
            .and_then(|value| value.as_str()),
        Some("tool_rpc_typescript")
    );
    assert_eq!(
        result
            .result
            .get("output_json")
            .and_then(|value| value.get("helper"))
            .and_then(|value| value.as_str()),
        Some("workspace ts module")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rpc_rejects_inner_tools_that_require_approval() {
    let temp_dir = tempfile::tempdir().unwrap();
    let target = temp_dir.path().join("blocked.txt");

    let registry = Arc::new(ToolRegistry::new());
    registry.register_sync(Arc::new(WriteFileTool::new()));

    let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
    let ctx = JobContext::default();
    let error = tool
        .execute(
            serde_json::json!({
                "language": "python",
                "mode": "tool_rpc",
                "code": format!(
                    "from thinclaw_tools import write_file\nwrite_file(path='{}', content='blocked')\nprint('done')",
                    target.display()
                )
            }),
            &ctx,
        )
        .await
        .expect_err("tool_rpc write_file should fail closed on approval");

    assert!(error.to_string().contains("requires approval"));
    assert!(!target.exists());
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rpc_json_result_format_ignores_stderr_noise() {
    let registry = Arc::new(ToolRegistry::new());
    let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
    let ctx = JobContext::default();
    let result = tool
        .execute(
            serde_json::json!({
                "language": "python",
                "mode": "tool_rpc",
                "result_format": "json",
                "code": "import json, sys\nsys.stderr.write('warning on stderr\\n')\nprint(json.dumps({'status': 'ok'}))"
            }),
            &ctx,
        )
        .await
        .expect("tool_rpc json parsing should use stdout only");

    assert_eq!(
        result
            .result
            .get("output_json")
            .and_then(|v| v.get("status"))
            .and_then(|v| v.as_str()),
        Some("ok")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rpc_resolves_project_local_imports() {
    let temp_dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        temp_dir.path().join("helper_rpc.py"),
        "VALUE = 'tool rpc import'",
    )
    .await
    .unwrap();

    let registry = Arc::new(ToolRegistry::new());
    let tool = ExecuteCodeTool::new()
        .with_working_dir(temp_dir.path().to_path_buf())
        .with_tool_registry(Arc::downgrade(&registry));
    let ctx = JobContext::default();
    let result = tool
        .execute(
            serde_json::json!({
                "language": "python",
                "mode": "tool_rpc",
                "code": "import helper_rpc\nprint(helper_rpc.VALUE)"
            }),
            &ctx,
        )
        .await
        .expect("tool_rpc should import local workspace modules");

    assert!(
        result
            .result
            .get("output")
            .and_then(|value| value.as_str())
            .is_some_and(|value| value.contains("tool rpc import"))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_tool_rpc_json_result_format_rejects_invalid_json() {
    let registry = Arc::new(ToolRegistry::new());
    let tool = ExecuteCodeTool::new().with_tool_registry(Arc::downgrade(&registry));
    let ctx = JobContext::default();
    let result = tool
        .execute(
            serde_json::json!({
                "language": "python",
                "mode": "tool_rpc",
                "result_format": "json",
                "code": "print('not valid json')"
            }),
            &ctx,
        )
        .await;

    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("expected JSON final output")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn test_execute_inner_tool_rpc_blocks_disallowed_tools() {
    let registry = Arc::new(ToolRegistry::new());
    let ctx = JobContext::default();
    let host: Arc<dyn ToolRpcHost> = Arc::new(RegistryToolRpcHost::new(Arc::downgrade(&registry)));

    let result = execute_inner_tool_rpc(
        &host,
        &ctx,
        ToolRpcRequest {
            name: "shell".to_string(),
            params: serde_json::json!({"command": "echo hi"}),
        },
    )
    .await;

    assert!(matches!(result, Err(ToolError::NotAuthorized(_))));
}
