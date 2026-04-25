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
}

impl AcpStdioHarness {
    fn spawn() -> Self {
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
        assert!(
            !line.trim().is_empty(),
            "stdout must not contain blank NDJSON lines"
        );
        let value: serde_json::Value = serde_json::from_str(&line).expect("stdout line is JSON");
        assert_eq!(value["jsonrpc"], serde_json::json!("2.0"));
        value
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

    harness.write_line(r#"{"jsonrpc":"2.0","id":"bad-cwd","method":"session/new","params":{"cwd":"relative","mcpServers":[]}}"#);
    let bad_cwd = harness.read_json();
    assert_eq!(bad_cwd["id"], serde_json::json!("bad-cwd"));
    assert_eq!(bad_cwd["error"]["code"], serde_json::json!(-32602));

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

    harness.write_line(r#"{"jsonrpc":"2.0","id":"unknown","method":"not/a-method","params":{}}"#);
    let unknown = harness.read_json();
    assert_eq!(unknown["error"]["code"], serde_json::json!(-32601));

    let status = harness.finish();
    assert!(status.success(), "thinclaw-acp exited with {status}");
}
