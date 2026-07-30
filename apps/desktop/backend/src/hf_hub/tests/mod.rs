use super::*;
use super::download::*;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

fn minimal_gguf() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&3_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes.extend_from_slice(&0_u64.to_le_bytes());
    bytes
}

fn managed_gguf_manifest(
    primary_path: &str,
    files: &[&str],
) -> crate::model_manager::ManagedModelManifest {
    crate::model_manager::ManagedModelManifest {
        schema_version: 1,
        install_id: "hf-http-test-install".to_string(),
        source: "huggingface".to_string(),
        repo_id: Some("owner/repo".to_string()),
        revision: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
        category: "LLM".to_string(),
        task: Some("chat".to_string()),
        runtime: "llamacpp".to_string(),
        format: "gguf".to_string(),
        artifact_kind: if files.len() > 1 {
            "gguf_sharded".to_string()
        } else {
            "gguf_single".to_string()
        },
        artifact_id: Some("http-test-artifact".to_string()),
        companion_artifact_id: None,
        companion_path: None,
        primary_path: Some(primary_path.to_string()),
        files: files.iter().map(|path| (*path).to_string()).collect(),
        quantization: Some("Q4_K_M".to_string()),
    }
}

#[derive(Clone)]
struct MockHttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    body_delay: std::time::Duration,
}

impl MockHttpResponse {
    fn json(status: u16, value: serde_json::Value) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: serde_json::to_vec(&value).expect("mock JSON"),
            body_delay: std::time::Duration::ZERO,
        }
    }

    fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body,
            body_delay: std::time::Duration::ZERO,
        }
    }

    fn with_header(mut self, name: &str, value: String) -> Self {
        self.headers.push((name.to_string(), value));
        self
    }

    fn with_body_delay(mut self, delay: std::time::Duration) -> Self {
        self.body_delay = delay;
        self
    }
}

#[derive(Debug, Clone)]
struct RecordedHttpRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
}

struct MockHttpServer {
    base_url: reqwest::Url,
    requests: Arc<Mutex<Vec<RecordedHttpRequest>>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockHttpServer {
    async fn spawn<F>(responses: F) -> Self
    where
        F: FnOnce(&reqwest::Url) -> Vec<MockHttpResponse>,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock HTTP server");
        let address = listener.local_addr().expect("mock server address");
        let base_url =
            reqwest::Url::parse(&format!("http://{address}/")).expect("mock server URL");
        let responses = responses(&base_url);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.expect("mock HTTP request");
                let mut request = Vec::new();
                let mut buffer = [0_u8; 2_048];
                loop {
                    let read = socket.read(&mut buffer).await.expect("read mock request");
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    assert!(request.len() <= 64 * 1024, "mock request header too large");
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8(request).expect("UTF-8 mock request");
                let mut lines = request.split("\r\n");
                let request_line = lines.next().expect("mock request line");
                let mut request_parts = request_line.split_ascii_whitespace();
                let method = request_parts
                    .next()
                    .expect("mock request method")
                    .to_string();
                let target = request_parts
                    .next()
                    .expect("mock request target")
                    .to_string();
                let mut headers = BTreeMap::new();
                for line in lines.take_while(|line| !line.is_empty()) {
                    if let Some((name, value)) = line.split_once(':') {
                        headers
                            .insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
                    }
                }
                recorded
                    .lock()
                    .expect("recorded requests")
                    .push(RecordedHttpRequest {
                        method,
                        target,
                        headers,
                    });

                let reason = match response.status {
                    200 => "OK",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    429 => "Too Many Requests",
                    _ => "Mock",
                };
                let mut head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                    response.status,
                    reason,
                    response.body.len()
                );
                for (name, value) in response.headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("\r\n");
                socket
                    .write_all(head.as_bytes())
                    .await
                    .expect("write mock response headers");
                if !response.body_delay.is_zero() {
                    tokio::time::sleep(response.body_delay).await;
                }
                let _ = socket.write_all(&response.body).await;
                let _ = socket.shutdown().await;
            }
        });
        Self {
            base_url,
            requests,
            task,
        }
    }

    fn requests(&self) -> Vec<RecordedHttpRequest> {
        self.requests.lock().expect("recorded requests").clone()
    }

    async fn finish(self) {
        tokio::time::timeout(std::time::Duration::from_secs(3), self.task)
            .await
            .expect("mock server finished")
            .expect("mock server task");
    }
}

fn tree_file(path: &str, size: u64) -> HfTreeFile {
    HfTreeFile {
        path: path.to_string(),
        size,
        oid: None,
        sha256: None,
    }
}

mod catalog;
mod downloads;
