#![cfg(feature = "acp")]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

struct AcpStdioHarness {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout_rx: mpsc::Receiver<String>,
    stdout_thread: Option<thread::JoinHandle<()>>,
    stderr: Option<ChildStderr>,
    _temp_home: tempfile::TempDir,
}

impl AcpStdioHarness {
    fn spawn() -> Self {
        Self::spawn_with_agent_runtime(false, &[])
    }

    fn spawn_agent(extra_env: &[(&str, &str)]) -> Self {
        Self::spawn_with_agent_runtime(true, extra_env)
    }

    fn spawn_with_agent_runtime(agent_runtime: bool, extra_env: &[(&str, &str)]) -> Self {
        let temp_home = tempfile::tempdir().expect("temp THINCLAW_HOME");
        let mut command = Command::new(env!("CARGO_BIN_EXE_thinclaw-acp"));
        command
            .arg("--no-db")
            .arg("--workspace")
            .arg(temp_home.path())
            .env("THINCLAW_HOME", temp_home.path())
            .env("LLM_BACKEND", "openai_compatible")
            .env("LLM_BASE_URL", "http://127.0.0.1:9/v1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if agent_runtime {
            command.env("THINCLAW_ACP_AGENT_STDIO_SMOKE", "1");
        } else {
            command.env("THINCLAW_ACP_STDIO_SMOKE", "1");
        }
        for (name, value) in extra_env {
            command.env(name, value);
        }
        let mut child = command.spawn().expect("spawn thinclaw-acp");
        let stdout = child.stdout.take().expect("stdout");
        let (stdout_tx, stdout_rx) = mpsc::channel();
        let stdout_thread = thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                let _ = stdout_tx.send(line);
            }
        });
        Self {
            stdin: child.stdin.take(),
            stderr: child.stderr.take(),
            child,
            stdout_rx,
            stdout_thread: Some(stdout_thread),
            _temp_home: temp_home,
        }
    }

    fn write_line(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin");
        stdin.write_all(line.as_bytes()).expect("write request");
        stdin.write_all(b"\n").expect("write newline");
        stdin.flush().expect("flush request");
    }

    fn read_json(&self) -> serde_json::Value {
        let line = self
            .stdout_rx
            .recv_timeout(Duration::from_secs(20))
            .expect("ACP stdout response");
        Self::parse_stdout_line(&line)
    }

    fn read_json_with_timeout(&self, timeout: Duration) -> Option<serde_json::Value> {
        let line = match self.stdout_rx.recv_timeout(timeout) {
            Ok(line) => line,
            Err(mpsc::RecvTimeoutError::Timeout) => return None,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("ACP stdout closed before response");
            }
        };
        Some(Self::parse_stdout_line(&line))
    }

    fn parse_stdout_line(line: &str) -> serde_json::Value {
        assert!(
            !line.trim().is_empty(),
            "stdout must not contain blank NDJSON lines"
        );
        let value: serde_json::Value = serde_json::from_str(&line).expect("stdout line is JSON");
        assert_eq!(value["jsonrpc"], serde_json::json!("2.0"));
        value
    }

    fn read_until_response(
        &self,
        id: serde_json::Value,
    ) -> (Vec<serde_json::Value>, serde_json::Value) {
        let mut seen = Vec::new();
        loop {
            let value = self.read_json();
            if value.get("id") == Some(&id) {
                return (seen, value);
            }
            seen.push(value);
        }
    }

    fn read_until_two_responses(
        &self,
        first_id: serde_json::Value,
        second_id: serde_json::Value,
    ) -> (serde_json::Value, serde_json::Value) {
        let mut first = None;
        let mut second = None;
        loop {
            let value = self.read_json();
            if value.get("id") == Some(&first_id) {
                first = Some(value);
            } else if value.get("id") == Some(&second_id) {
                second = Some(value);
            }
            if let (Some(first), Some(second)) = (first.clone(), second.clone()) {
                return (first, second);
            }
        }
    }

    fn read_until_method(&self, method: &str) -> (Vec<serde_json::Value>, serde_json::Value) {
        let mut seen = Vec::new();
        loop {
            let value = self.read_json();
            if value["method"] == serde_json::json!(method) {
                return (seen, value);
            }
            seen.push(value);
        }
    }

    fn finish(mut self) -> std::process::ExitStatus {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("poll child") {
                break status;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                let _ = self.child.wait().expect("collect killed child");
                let mut stderr = String::new();
                if let Some(mut pipe) = self.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                panic!("thinclaw-acp stdio smoke timed out\nstderr:\n{stderr}");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        if let Some(handle) = self.stdout_thread.take() {
            let _ = handle.join();
        }
        status
    }
}

fn initialize_agent_session(harness: &mut AcpStdioHarness) -> String {
    harness.write_line(r#"{"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"agent-stdio-smoke","version":"0"}}}"#);
    let init = harness.read_json();
    assert_eq!(init["id"], serde_json::json!("init"));
    assert_eq!(init["result"]["protocolVersion"], serde_json::json!(1));

    harness.write_line(r#"{"jsonrpc":"2.0","id":"new","method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#);
    let new_session = harness.read_json();
    assert_eq!(new_session["id"], serde_json::json!("new"));
    new_session["result"]["sessionId"]
        .as_str()
        .expect("session id")
        .to_string()
}

fn has_update_kind(values: &[serde_json::Value], kind: &str) -> bool {
    values.iter().any(|value| {
        value["method"] == serde_json::json!("session/update")
            && value["params"]["update"]["sessionUpdate"] == serde_json::json!(kind)
    })
}

fn run_acp_stdio(input: &str) -> std::process::Output {
    let temp_home = tempfile::tempdir().expect("temp THINCLAW_HOME");
    let mut child = Command::new(env!("CARGO_BIN_EXE_thinclaw-acp"))
        .arg("--no-db")
        .arg("--workspace")
        .arg(temp_home.path())
        .env("THINCLAW_HOME", temp_home.path())
        .env("THINCLAW_ACP_STDIO_SMOKE", "1")
        .env("LLM_BACKEND", "openai_compatible")
        .env("LLM_BASE_URL", "http://127.0.0.1:9/v1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn thinclaw-acp");

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        stdin.write_all(input.as_bytes()).expect("write request");
        stdin.flush().expect("flush request");
    }
    drop(child.stdin.take());

    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if child.try_wait().expect("poll child").is_some() {
            return child.wait_with_output().expect("collect output");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let output = child.wait_with_output().expect("collect killed output");
            panic!(
                "thinclaw-acp stdio smoke timed out\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn thinclaw_acp_stdout_is_clean_ndjson_for_basic_transcript() {
    let input = concat!(
        "{\n",
        "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1,\"clientCapabilities\":{},\"clientInfo\":{\"name\":\"stdio-smoke\",\"version\":\"0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":\"new-1\",\"method\":\"session/new\",\"params\":{\"cwd\":\"/tmp\",\"mcpServers\":[]}}\n",
    );
    let output = run_acp_stdio(input);
    assert!(
        output.status.success(),
        "thinclaw-acp failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        3,
        "stdout should contain one NDJSON response per request"
    );
    for line in &lines {
        assert!(
            !line.trim().is_empty(),
            "stdout must not contain blank lines"
        );
        let value: serde_json::Value = serde_json::from_str(line).expect("stdout line is JSON");
        assert_eq!(value["jsonrpc"], serde_json::json!("2.0"));
    }

    assert_eq!(
        lines[0].parse::<serde_json::Value>().unwrap()["error"]["code"],
        -32700
    );
    assert_eq!(
        lines[1].parse::<serde_json::Value>().unwrap()["result"]["protocolVersion"],
        serde_json::json!(1)
    );
    assert_eq!(
        lines[2].parse::<serde_json::Value>().unwrap()["id"],
        serde_json::json!("new-1")
    );
}

#[test]
fn thinclaw_acp_interactive_transcript_covers_list_load_and_error_shapes() {
    let mut harness = AcpStdioHarness::spawn();

    harness.write_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"stdio-smoke","version":"0"}}}"#);
    let initialize = harness.read_json();
    assert_eq!(initialize["id"], serde_json::json!(1));
    assert_eq!(
        initialize["result"]["protocolVersion"],
        serde_json::json!(1)
    );
    assert_eq!(initialize["result"]["authMethods"], serde_json::json!([]));

    harness.write_line(r#"{"jsonrpc":"2.0","id":"auth","method":"authenticate","params":{}}"#);
    let auth = harness.read_json();
    assert_eq!(auth["id"], serde_json::json!("auth"));
    assert_eq!(auth["result"], serde_json::json!({}));

    harness.write_line(r#"{"jsonrpc":"2.0","id":"bad-cwd","method":"session/new","params":{"cwd":"relative","mcpServers":[]}}"#);
    let bad_cwd = harness.read_json();
    assert_eq!(bad_cwd["id"], serde_json::json!("bad-cwd"));
    assert_eq!(bad_cwd["error"]["code"], serde_json::json!(-32602));

    harness.write_line(r#"{"jsonrpc":"2.0","id":"bad-mcp","method":"session/new","params":{"cwd":"/tmp","mcpServers":[{"type":"http","url":"http://127.0.0.1:1234"}]}}"#);
    let bad_mcp = harness.read_json();
    assert_eq!(bad_mcp["id"], serde_json::json!("bad-mcp"));
    assert_eq!(bad_mcp["error"]["code"], serde_json::json!(-32602));
    assert!(
        bad_mcp["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("not advertised"))
    );

    harness.write_line(r#"{"jsonrpc":"2.0","id":"new","method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#);
    let new_session = harness.read_json();
    let session_id = new_session["result"]["sessionId"]
        .as_str()
        .expect("session id")
        .to_string();
    assert_eq!(new_session["id"], serde_json::json!("new"));
    assert!(
        new_session["result"]["modes"]["availableModes"]
            .as_array()
            .is_some_and(|modes| !modes.is_empty())
    );

    harness.write_line(
        r#"{"jsonrpc":"2.0","id":"list","method":"session/list","params":{"cwd":"/tmp"}}"#,
    );
    let list = harness.read_json();
    let sessions = list["result"]["sessions"].as_array().expect("sessions");
    assert!(
        sessions
            .iter()
            .any(|session| session["sessionId"] == serde_json::json!(session_id))
    );

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"load","method":"session/load","params":{{"sessionId":"{session_id}","cwd":"/tmp","mcpServers":[]}}}}"#
    ));
    let load = harness.read_json();
    assert_eq!(load["id"], serde_json::json!("load"));
    assert!(load.get("result").is_some());

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"prompt","method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"hello"}}]}}}}"#
    ));
    let prompt = harness.read_json();
    assert_eq!(prompt["id"], serde_json::json!("prompt"));
    assert_eq!(prompt["error"]["code"], serde_json::json!(-32601));
    assert!(
        prompt["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("agent runtime"))
    );

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"cancel","method":"session/cancel","params":{{"sessionId":"{session_id}"}}}}"#
    ));
    let cancel = harness.read_json();
    assert_eq!(cancel["id"], serde_json::json!("cancel"));
    assert_eq!(cancel["error"]["code"], serde_json::json!(-32601));
    assert!(
        cancel["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("agent runtime"))
    );

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"close","method":"session/close","params":{{"sessionId":"{session_id}"}}}}"#
    ));
    let close = harness.read_json();
    assert_eq!(close["id"], serde_json::json!("close"));
    assert_eq!(close["error"]["code"], serde_json::json!(-32601));
    assert!(
        close["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("agent runtime"))
    );

    harness.write_line(r#"{"jsonrpc":"2.0","id":"unknown","method":"not/a-method","params":{}}"#);
    let unknown = harness.read_json();
    assert_eq!(unknown["error"]["code"], serde_json::json!(-32601));

    let status = harness.finish();
    assert!(status.success(), "thinclaw-acp exited with {status}");
}

#[test]
fn thinclaw_acp_agent_prompt_streams_updates_from_real_runtime() {
    let mut harness = AcpStdioHarness::spawn_agent(&[]);
    let session_id = initialize_agent_session(&mut harness);

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"prompt-stream","method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"stream please"}}]}}}}"#
    ));
    let (updates, prompt) = harness.read_until_response(serde_json::json!("prompt-stream"));
    assert_eq!(
        prompt["result"]["stopReason"],
        serde_json::json!("end_turn")
    );
    assert!(
        has_update_kind(&updates, "agent_message_chunk"),
        "expected streamed agent message chunk, got {updates:#?}"
    );
    assert!(
        has_update_kind(&updates, "usage_update"),
        "expected usage update, got {updates:#?}"
    );

    let status = harness.finish();
    assert!(status.success(), "thinclaw-acp exited with {status}");
}

#[test]
fn thinclaw_acp_agent_permission_approve_reject_cancel_and_timeout_are_transcripts() {
    let mut harness =
        AcpStdioHarness::spawn_agent(&[("THINCLAW_ACP_PROMPT_APPROVAL_TIMEOUT_MS", "150")]);
    let session_id = initialize_agent_session(&mut harness);

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"approve-prompt","method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"approval please"}}]}}}}"#
    ));
    let (before_permission, permission) = harness.read_until_method("session/request_permission");
    assert!(
        has_update_kind(&before_permission, "tool_call"),
        "expected tool_call before permission request, got {before_permission:#?}"
    );
    let permission_id = permission["id"].clone();
    harness.write_line(
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": permission_id,
            "result": { "outcome": "selected", "optionId": "allow-once" }
        })
        .to_string(),
    );
    let (mut approve_updates, approve_prompt) =
        harness.read_until_response(serde_json::json!("approve-prompt"));
    assert_eq!(
        approve_prompt["result"]["stopReason"],
        serde_json::json!("end_turn")
    );
    if !has_update_kind(&approve_updates, "tool_call")
        && !has_update_kind(&approve_updates, "tool_call_update")
        && let Some(late_update) = harness.read_json_with_timeout(Duration::from_secs(5))
    {
        approve_updates.push(late_update);
    }
    assert!(
        has_update_kind(&approve_updates, "tool_call")
            || has_update_kind(&approve_updates, "tool_call_update"),
        "expected tool lifecycle update after approval, got {approve_updates:#?}"
    );

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"reject-prompt","method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"approval reject path"}}]}}}}"#
    ));
    let (_, permission) = harness.read_until_method("session/request_permission");
    harness.write_line(
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": permission["id"].clone(),
            "result": { "outcome": "selected", "optionId": "reject-once" }
        })
        .to_string(),
    );
    let (_, reject_prompt) = harness.read_until_response(serde_json::json!("reject-prompt"));
    assert_eq!(
        reject_prompt["result"]["stopReason"],
        serde_json::json!("end_turn")
    );

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"cancel-permission-prompt","method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"approval cancel path"}}]}}}}"#
    ));
    let (_, permission) = harness.read_until_method("session/request_permission");
    harness.write_line(
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": permission["id"].clone(),
            "result": { "outcome": "cancelled" }
        })
        .to_string(),
    );
    let (_, cancel_permission_prompt) =
        harness.read_until_response(serde_json::json!("cancel-permission-prompt"));
    assert_eq!(
        cancel_permission_prompt["result"]["stopReason"],
        serde_json::json!("cancelled")
    );

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"timeout-prompt","method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"approval timeout path"}}]}}}}"#
    ));
    let (_, permission) = harness.read_until_method("session/request_permission");
    assert!(permission["id"].is_number() || permission["id"].is_string());
    let (_, timeout_prompt) = harness.read_until_response(serde_json::json!("timeout-prompt"));
    assert_eq!(
        timeout_prompt["result"]["stopReason"],
        serde_json::json!("cancelled")
    );

    let status = harness.finish();
    assert!(status.success(), "thinclaw-acp exited with {status}");
}

#[test]
fn thinclaw_acp_agent_cancel_and_close_abort_real_prompt_turns() {
    let mut harness = AcpStdioHarness::spawn_agent(&[]);
    let session_id = initialize_agent_session(&mut harness);

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"slow-cancel-prompt","method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"slow cancel"}}]}}}}"#
    ));
    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"cancel","method":"session/cancel","params":{{"sessionId":"{session_id}"}}}}"#
    ));
    let (cancelled_prompt, cancel_response) = harness.read_until_two_responses(
        serde_json::json!("slow-cancel-prompt"),
        serde_json::json!("cancel"),
    );
    assert_eq!(cancel_response["result"], serde_json::json!({}));
    assert_eq!(
        cancelled_prompt["result"]["stopReason"],
        serde_json::json!("cancelled")
    );

    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"slow-close-prompt","method":"session/prompt","params":{{"sessionId":"{session_id}","prompt":[{{"type":"text","text":"slow close"}}]}}}}"#
    ));
    harness.write_line(&format!(
        r#"{{"jsonrpc":"2.0","id":"close","method":"session/close","params":{{"sessionId":"{session_id}"}}}}"#
    ));
    let (closed_prompt, close_response) = harness.read_until_two_responses(
        serde_json::json!("slow-close-prompt"),
        serde_json::json!("close"),
    );
    assert_eq!(close_response["result"], serde_json::json!({}));
    assert_eq!(
        closed_prompt["result"]["stopReason"],
        serde_json::json!("cancelled")
    );

    let status = harness.finish();
    assert!(status.success(), "thinclaw-acp exited with {status}");
}
