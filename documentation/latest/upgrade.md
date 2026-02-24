# Upgrade Plan: Multi-Engine Inference & HF Hub Model Discovery

> **Scope:** This document captures all architectural decisions, implementation details, gaps, todos, and design rationale for two major upcoming features:
> 1. **Multi-Engine Inference Core** — each build ships with ONE inference engine per platform (llama.cpp, MLX, vLLM, Ollama, or cloud-only)
> 2. **HuggingFace Hub Dynamic Model Discovery** — replace the static `model-library.ts` registry with live HF API search, file-tree parsing, and smart multi-file downloads
>
> **Last updated:** 2026-02-23  
> **Status:** All phases (1–6) fully implemented. All backend todos, frontend todos, CI matrix, and integration wiring complete.  
> **Relates to:** `MICROSERVICES_AND_SIDECARS.md`, `TECHNICAL_ARCHITECTURE.md`, `RIG_IMPLEMENTATION.md`
>
> ### Key Architectural Decision: One Engine Per Build
> Each platform build compiles with **one** inference engine feature flag. There is no runtime engine switching.
> - macOS ARM → `mlx` or `llamacpp` (Metal)
> - macOS Intel → `llamacpp` (CPU)
> - Linux CUDA → `vllm` or `llamacpp` (CUDA)
> - Windows → `llamacpp`
> - Cloud-only → no engine feature
>
> This eliminates: Engine Selector UI, `selected_inference_engine` config field, setup wizard, engine-switch confirmation dialog.

---

## Table of Contents

1. [Context — Current State Baseline](#1-context--current-state-baseline)
2. [Feature A: Multi-Engine Inference Core](#2-feature-a-multi-engine-inference-core)
   - 2.1 [Engine × Platform Matrix](#21-engine--platform-matrix)
   - 2.2 [Bundled vs. External Processes](#22-bundled-vs-external-processes)
   - 2.3 [Cargo Feature Architecture](#23-cargo-feature-architecture)
   - 2.4 [Build-Time Auto-Discovery (build.rs)](#24-build-time-auto-discovery-buildrs)
   - 2.5 [Platform-Specific Tauri Config Strategy](#25-platform-specific-tauri-config-strategy)
   - 2.6 [MLX Engine: uv Bootstrap Strategy](#26-mlx-engine-uv-bootstrap-strategy)
   - 2.7 [vLLM Engine: Linux + CUDA](#27-vllm-engine-linux--cuda)
   - 2.8 [Ollama Engine: Universal Daemon](#28-ollama-engine-universal-daemon)
   - 2.9 [EngineProcess Abstraction (New Rust Trait)](#29-engineprocess-abstraction-new-rust-trait)
   - 2.10 [UserConfig Changes](#210-userconfig-changes)
   - 2.11 [get_supported_engines() Command](#211-get_supported_engines-command)
   - 2.12 [Cross-Compilation & CI Strategy](#212-cross-compilation--ci-strategy)
   - 2.13 [Backend Todos](#213-backend-todos)
   - 2.14 [Frontend Todos](#214-frontend-todos)
   - 2.15 [Gaps & Open Questions](#215-gaps--open-questions)
3. [Feature B: HF Hub Dynamic Model Discovery](#3-feature-b-hf-hub-dynamic-model-discovery)
   - 3.1 [Problem with Current Model Library](#31-problem-with-current-model-library)
   - 3.2 [HF API Endpoints Used](#32-hf-api-endpoints-used)
   - 3.3 [Engine-Tag Mapping](#33-engine-tag-mapping)
   - 3.4 [discover_hf_models() Command](#34-discover_hf_models-command)
   - 3.5 [get_model_files() Command — Tree API + Quant Parsing](#35-get_model_files-command--tree-api--quant-parsing)
   - 3.6 [Multi-File vs. Single-File Download Logic](#36-multi-file-vs-single-file-download-logic)
   - 3.7 [download_hf_model_files() Command](#37-download_hf_model_files-command)
   - 3.8 [ModelDownloadInfo Type](#38-modeldownloadinfo-type)
   - 3.9 [Static Library vs. Discovery: Coexistence Strategy](#39-static-library-vs-discovery-coexistence-strategy)
   - 3.10 [What Already Works (No Changes Needed)](#310-what-already-works-no-changes-needed)
   - 3.11 [Backend Todos](#311-backend-todos)
   - 3.12 [Frontend Todos](#312-frontend-todos)
   - 3.13 [Gaps & Open Questions](#313-gaps--open-questions)
4. [Shared Infrastructure Changes](#4-shared-infrastructure-changes)
5. [Implementation Order (Recommended)](#5-implementation-order-recommended)
6. [Known Constraints & Non-Goals](#6-known-constraints--non-goals)

---

## 1. Context — Current State Baseline

### What Exists Today

| Component | State | File(s) |
|---|---|---|
| Inference engine | **llama.cpp only** — Metal-compiled binary for macOS ARM | `backend/bin/llama-server-aarch64-apple-darwin` |
| Sidecar manager | Hardcoded to `llama-server` Tauri sidecar | `backend/src/sidecar.rs` |
| Model discovery | Static 1,114-line TypeScript registry | `frontend/src/lib/model-library.ts` |
| Model download | `reqwest` streaming + `tokio::sync::Notify` cancel | `backend/src/model_manager.rs` |
| Download progress | Tauri events `download_progress` with `{filename, downloaded, total, percentage}` | `model_manager.rs` |
| HF token support | `Authorization: Bearer` injected if `huggingface_token` set | `model_manager.rs` |
| GGUF chat template | Read from `tokenizer.chat_template` KV in GGUF metadata | `backend/src/gguf.rs` |
| Family detection | Heuristics on arch string + chat template content | `gguf.rs` `detect_model_family()` |
| Template → server | Passed to `llama-server` via `--chat-template` CLI arg | `sidecar.rs` `start_chat_server()` |
| Health-check polling | Already implemented — `/health` poll every 500ms, 120s timeout | `sidecar.rs` lines 882–911 |
| Dynamic port binding | Already implemented — OS assigns free port via `TcpListener::bind("0.0.0.0:0")` | `sidecar.rs` `generate_config()` |
| Zombie prevention | `Drop` implemented on `SidecarProcess` struct | `sidecar.rs` |
| System specs detection | `sysinfo` crate — CPU brand, RAM, platform | `backend/src/system.rs` |

### What Is Missing

- No `selected_inference_engine` in `UserConfig`
- No `get_supported_engines()` Tauri command
- No MLX, vLLM, or Ollama engine paths in `SidecarManager`
- No live HF API search (only static TypeScript list)
- No HF Tree API integration for dynamic file/quant discovery
- No multi-file (MLX/vLLM folder) download logic
- No `hf_hub.rs` module

### Key Architectural Fact

All inference engines we support expose an **OpenAI-compatible HTTP API** on a local port. This means `chat.rs` → `resolve_provider()` → `ProviderKind::Local` → `http://127.0.0.1:{port}/v1` requires **zero changes** when a different engine is running. The entire `rig_lib` stack, Orchestrator, and streaming pipeline are engine-agnostic already.

---

## 2. Feature A: Multi-Engine Inference Core

### 2.1 Engine × Platform Matrix

| Engine | macOS (Apple Silicon) | macOS (Intel) | Linux (CUDA GPU) | Linux (CPU) | Windows (NVIDIA) | Windows (CPU) |
|---|---|---|---|---|---|---|
| **llama.cpp** | ✅ Metal | ✅ CPU | ✅ CUDA | ✅ CPU | ✅ CUDA | ✅ CPU |
| **MLX** (`mlx_lm`) | ✅ Best option | ❌ Intel only | ❌ | ❌ | ❌ | ❌ |
| **vLLM** | ❌ | ❌ | ✅ Best CUDA | ❌ too slow | ⚠️ WSL2 only | ❌ |
| **Ollama** | ✅ Metal (if installed) | ✅ CPU | ✅ CUDA | ✅ CPU | ✅ CUDA | ✅ CPU |
| **Cloud-only (none)** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

**Priority per platform:**
- **macOS Apple Silicon:** MLX (best performance, Apple-native) → llama.cpp Metal (bundled fallback) → Ollama → Cloud
- **macOS Intel:** llama.cpp CPU → Ollama → Cloud
- **Linux CUDA:** vLLM (best throughput) → llama.cpp CUDA → Ollama → Cloud
- **Linux CPU:** llama.cpp CPU → Ollama → Cloud
- **Windows CUDA:** llama.cpp CUDA → Ollama → Cloud
- **Windows CPU:** llama.cpp CPU → Ollama → Cloud

---

### 2.2 Bundled vs. External Processes

This is the **critical architectural distinction** between engine types:

#### Bundled (Tauri Sidecar) — llama.cpp

```
Scrappy.app/Contents/MacOS/
├── Scrappy                          ← Tauri app binary
└── bin/
    ├── llama-server-aarch64-apple-darwin  ← bundled, signed, notarized
    ├── *.metal                            ← GPU shaders
    └── *.dylib                            ← linked libraries
```

- Managed via `tauri_plugin_shell` — Tauri resolves path, signs, and co-manages lifecycle
- Killed automatically when app exits (our `Drop` impl on `SidecarProcess`)
- Works offline — no internet required at first launch
- **Must be in `tauri.conf.json` `externalBin`**

#### External Process — MLX, vLLM

```
User's system (not inside .app):
├── /usr/local/bin/uv               ← bundled 5MB Rust binary (ships in Scrappy)
└── ~/Library/Application Support/com.schack.scrappy/
    └── mlx-env/                    ← created on first launch by uv
        ├── bin/python3
        ├── bin/mlx_lm              ← the actual server entry point
        └── lib/python3.12/site-packages/mlx_lm/
```

- Managed via `tokio::process::Command` (not Tauri shell plugin)
- PID tracked manually in `SidecarManager` (or the new `EngineManager`)
- Must be explicitly killed on app exit — **not automatic** unless we implement Drop manually
- Requires internet on first launch to download packages
- **mlx_lm spawned command:** `python3 -m mlx_lm.server --model /path/to/model --port {port}`

#### Detached Daemon — Ollama

```
System-level (not managed by Scrappy):
└── /usr/local/bin/ollama           ← installed by user, runs as system service
    └── Listening on localhost:11434
```

- Scrappy does **not** start or stop Ollama — it just connects to the existing daemon
- Detection: check if `http://localhost:11434/v1/models` returns 200
- No process management code needed — just use the existing `ProviderKind::Local` path pointed at port `11434`
- **One complication:** Ollama manages its own model loading — Scrappy tells it which model via the API, not via CLI args

---

### 2.3 Cargo Feature Architecture

```toml
# backend/Cargo.toml
[features]
# Default: llama.cpp only (all platforms)
default = ["llamacpp"]

# Inference engine features
llamacpp = []    # Bundled binary sidecar — all platforms
mlx = []         # External Python process — macOS Apple Silicon only
vllm = []        # External Python process — Linux CUDA only  
ollama = []      # Detect existing daemon — all platforms

# Platform-conditional deps (mlx-rs is only meaningful for future native work)
[target.'cfg(target_os = "macos")'.dependencies]
# mlx-rs = { version = "0.x", optional = true }  # Gated if we ever go native MLX
```

```rust
// Runtime feature checks in Rust code
#[cfg(feature = "mlx")]
mod engine_mlx;

#[cfg(feature = "vllm")]
mod engine_vllm;

#[cfg(any(feature = "mlx", feature = "vllm"))]
mod engine_external;  // Shared external process management code
```

**Build invocations:**
```bash
# macOS Apple Silicon (MLX primary + llama.cpp fallback)
cargo tauri build --features "llamacpp,mlx" --config tauri.conf.macos.json

# Linux CUDA (vLLM primary + llama.cpp fallback)
cargo tauri build --features "llamacpp,vllm" --config tauri.conf.linux.json

# Windows (llama.cpp + Ollama detection)
cargo tauri build --features "llamacpp,ollama" --config tauri.conf.windows.json
```

---

### 2.4 Build-Time Auto-Discovery (build.rs)

A `build.rs` script at `backend/build.rs` (or root `build.rs`) automates feature selection based on compile-target and host capabilities:

```rust
// build.rs (concept — not yet written)
fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    // Emit rustc cfg flags for conditional compilation
    match (target_os.as_str(), target_arch.as_str()) {
        ("macos", "aarch64") => {
            println!("cargo:rustc-cfg=engine_metal_available");
            println!("cargo:rustc-cfg=engine_mlx_supported");   // uv bootstrap possible
        }
        ("linux", _) => {
            if probe_cuda() {
                println!("cargo:rustc-cfg=engine_cuda_available");
                println!("cargo:rustc-cfg=engine_vllm_supported");
            }
        }
        ("windows", _) => {
            if probe_cuda() {
                println!("cargo:rustc-cfg=engine_cuda_available");
            }
        }
        _ => {}
    }
}

fn probe_cuda() -> bool {
    // Check nvcc or nvidia-smi on PATH at compile time
    std::process::Command::new("nvidia-smi")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
```

These `rustc-cfg` flags (different from Cargo features — they're compile-time constants) control which code paths compile in. The `get_supported_engines()` runtime function then does a **second pass** of runtime detection (is MLX actually installed? is Ollama running?).

---

### 2.5 Platform-Specific Tauri Config Strategy

`tauri.conf.json` is static — it cannot conditionally include binaries. Solution: **one config file per platform build target**.

```
backend/
├── tauri.conf.json             ← Base shared config (no externalBin)
├── tauri.conf.macos.json       ← macOS: llama-server + .dylib + .metal
├── tauri.conf.macos-mlx.json   ← macOS MLX-primary: no llama-server (smaller bundle)
├── tauri.conf.linux.json       ← Linux: llama-server (CUDA build)
└── tauri.conf.windows.json     ← Windows: llama-server.exe (CUDA build)
```

**tauri.conf.macos.json** (standard macOS — llama.cpp + MLX detection):
```json
{
  "bundle": {
    "externalBin": ["bin/llama-server", "bin/uv"],
    "resources": ["bin/*.dylib", "bin/*.metal", "resources/chromium", "openclaw-engine/**/*"]
  }
}
```

**tauri.conf.macos-mlx.json** (MLX-primary, no llama.cpp):
```json
{
  "bundle": {
    "externalBin": ["bin/uv"],
    "resources": ["resources/chromium", "openclaw-engine/**/*"]
  }
}
```

**Decision: Default macOS build includes both** `uv` (for MLX bootstrap) **and** `llama-server` (Metal, as fallback). This avoids stranding users who don't have `mlx_lm` yet and provides an instant-start experience. A dedicated MLX-only build is available for advanced users.

---

### 2.6 MLX Engine: uv Bootstrap Strategy

**Decision:** Option 2 — Bundle `uv` binary, create isolated venv in `app_data_dir/mlx-env` on first MLX engine activation.

#### Installation Flow

```
User selects "MLX" in Settings
         ↓
Rust: check app_data_dir/mlx-env/bin/python3 exists?
         │ Yes                    │ No
         ↓                       ↓
check mlx_lm version        emit "engine_setup_required"
  (pip show mlx_lm)         event → frontend shows
         │                  setup wizard
   ┌─────┴─────┐
   │  OK       │  Outdated (optional)
   ↓           ↓
spawn mlx_lm   offer background
server         upgrade (uv pip
               install --upgrade)
```

#### uv Bootstrap Rust code (concept)

```rust
// In new backend/src/engine_mlx.rs
pub async fn ensure_mlx_environment(app: &AppHandle) -> Result<PathBuf, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let env_dir = app_data.join("mlx-env");
    let python = env_dir.join("bin/python3");

    if python.exists() {
        // Already installed — verify mlx_lm is present
        return Ok(env_dir);
    }

    // uv is bundled as a Tauri external binary
    let uv_path = app.path().resource_dir()
        .map_err(|e| e.to_string())?
        .join("bin/uv");  // or via app.shell().sidecar("uv")

    // Emit progress events to frontend
    emit_setup_progress(&app, "Creating Python 3.12 environment...", 0.1);
    
    // Step 1: Create isolated venv with specific Python version
    tokio::process::Command::new(&uv_path)
        .args(["venv", "--python", "3.12", env_dir.to_str().unwrap()])
        .status().await
        .map_err(|e| format!("uv venv failed: {}", e))?;

    emit_setup_progress(&app, "Downloading MLX packages (this takes ~2 minutes)...", 0.3);

    // Step 2: Install mlx_lm into the isolated venv
    tokio::process::Command::new(&uv_path)
        .args([
            "pip", "install", "mlx_lm",
            "--python", python.to_str().unwrap()
        ])
        .status().await
        .map_err(|e| format!("mlx_lm install failed: {}", e))?;

    emit_setup_progress(&app, "MLX engine ready!", 1.0);
    Ok(env_dir)
}

pub async fn start_mlx_server(
    app: &AppHandle,
    model_path: &str,
    port: u16,
) -> Result<tokio::process::Child, String> {
    let env_dir = ensure_mlx_environment(app).await?;
    let python = env_dir.join("bin/python3");

    let child = tokio::process::Command::new(&python)
        .args([
            "-m", "mlx_lm.server",
            "--model", model_path,
            "--port", &port.to_string(),
            "--host", "127.0.0.1",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())  // capture for logging
        .spawn()
        .map_err(|e| format!("Failed to spawn mlx_lm server: {}", e))?;

    Ok(child)
}
```

#### MLX Server Lifecycle

- Port assignment: same as llama.cpp — OS-assigned via `TcpListener::bind("127.0.0.1:0")` (already in `sidecar.rs`)
- Health-check: same poll loop as llama.cpp — `GET /health` every 500ms, 120s timeout (already in `sidecar.rs`)
- Kill on exit: `tokio::process::Child::kill()` — must be called in Drop or on app exit
- Model path: MLX uses HuggingFace model directories (not single `.gguf` files) — `--model mlx-community/Llama-3-8B-4bit` or path to local directory
- API: OpenAI-compatible on the same port — `ProviderKind::Local` in `chat.rs` needs zero changes

#### mlx_lm Auto-Update

Background update check run once per day (configurable):
```rust
// check_mlx_updates() runs in background task on app start if MLX is active engine
tokio::process::Command::new(&uv_path)
    .args(["pip", "install", "--upgrade", "mlx_lm", "--python", python_path])
    .status().await?;
```

---

### 2.7 vLLM Engine: Linux + CUDA

**Identical pattern to MLX** — bundled `uv` binary, isolated venv in `app_data_dir/vllm-env`.

Key differences from MLX:
- Package: `pip install vllm` (instead of `mlx_lm`)
- **Prerequisite check:** Before installing, verify CUDA is available:
  ```rust
  fn cuda_available() -> bool {
      std::process::Command::new("nvidia-smi")
          .arg("--version")
          .output()
          .map(|o| o.status.success())
          .unwrap_or(false)
  }
  ```
- If CUDA not found: show UI error — *"vLLM requires an NVIDIA GPU with CUDA drivers. Falling back to llama.cpp."*
- Server start command: `python3 -m vllm.entrypoints.openai.api_server --model {path} --port {port}`
- Only compiled-in when `cargo feature = "vllm"` and `target_os = "linux"`

---

### 2.8 Ollama Engine: Universal Daemon

Ollama is special — Scrappy never starts or stops it. It's always either running or not.

**Detection at runtime:**
```rust
async fn ollama_running() -> bool {
    reqwest::Client::new()
        .get("http://localhost:11434/v1/models")
        .timeout(std::time::Duration::from_secs(2))
        .send().await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}
```

**Installation check:**
```rust
fn ollama_installed() -> bool {
    std::process::Command::new("ollama")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
```

**If not installed:** Show install link to [ollama.ai](https://ollama.ai) — we do not auto-install Ollama (it's a system-level daemon requiring elevated permissions).

**Model loading:** When Ollama is the active engine, `start_chat_server()` sends a `POST /api/pull` to load the requested model, then waits for readiness. The `ProviderKind::Local` path in `chat.rs` points at port `11434` — no other changes.

**Complication:** Ollama uses its own model ID format (e.g., `llama3:8b`, not a file path). The HF Hub downloader flow must map GGUF downloads to Ollama-compatible models (or use `ollama pull` for models in the Ollama registry). This needs further design.

---

### 2.9 EngineProcess Abstraction (New Rust Trait)

To cleanly replace the hardcoded `llama-server` sidecar with a multi-engine system, introduce a trait:

```rust
// backend/src/engine/mod.rs (new file)
#[async_trait]
pub trait InferenceEngine: Send + Sync {
    /// Start the engine with the given model. Returns the port it's listening on.
    async fn start(&self, model_path: &str, context_size: u32) -> Result<u16, String>;
    
    /// Stop the engine and free GPU/RAM.
    async fn stop(&self) -> Result<(), String>;
    
    /// Returns true if the engine HTTP endpoint is accepting requests.
    async fn is_ready(&self) -> bool;
    
    /// The base URL for OpenAI-compatible API calls.
    fn base_url(&self) -> String;
    
    /// Human-readable engine name for UI display.
    fn display_name(&self) -> &'static str;
    
    /// Returns true if this engine requires a single-file model (false = directory).
    fn uses_single_file_model(&self) -> bool;
}

pub struct LlamaCppEngine { /* SidecarProcess */ }
pub struct MlxEngine { /* Child process + env_dir */ }
pub struct VllmEngine { /* Child process + env_dir */ }
pub struct OllamaEngine { /* No process, just port */ }
```

`SidecarManager` would then hold `Option<Box<dyn InferenceEngine>>` as its active engine, replacing the current hardcoded `chat_process: Mutex<Option<SidecarProcess>>`.

---

### 2.10 UserConfig Changes

Add to `backend/src/config.rs` `UserConfig` struct:

```rust
/// Which local inference engine the user has selected.
/// "llamacpp" | "mlx" | "vllm" | "ollama" | null (cloud-only)
#[serde(default)]
pub selected_inference_engine: Option<String>,

/// Whether to auto-update MLX/vLLM Python packages on startup.
#[serde(default = "default_true")]
pub auto_update_inference_engine: bool,
```

**Default:** `None` → falls back to `"llamacpp"` if llama-server is bundled, or `null`/cloud-only otherwise. The `get_supported_engines()` command provides the UI with the list of valid values.

---

### 2.11 get_supported_engines() Command

New command in `backend/src/engine/mod.rs`, registered in `lib.rs`:

```rust
#[derive(Serialize, Type)]
pub struct EngineInfo {
    pub id: String,              // "llamacpp" | "mlx" | "vllm" | "ollama" | "none"
    pub display_name: String,    // "llama.cpp (Metal)" etc.
    pub available: bool,         // Compiled-in AND runtime-ready
    pub installed: bool,         // Runtime detection (is uv env present? is ollama running?)
    pub requires_setup: bool,    // True if first-launch installation needed
    pub description: String,
}

#[tauri::command]
#[specta::specta]
pub async fn get_supported_engines(app: AppHandle) -> Vec<EngineInfo> {
    let mut engines = vec![];

    // Cloud-only is always available
    engines.push(EngineInfo {
        id: "none".into(),
        display_name: "Cloud Only".into(),
        available: true,
        installed: true,
        requires_setup: false,
        description: "Use only cloud providers (Anthropic, OpenAI, etc.)".into(),
    });

    // llama.cpp — only if compiled in
    #[cfg(feature = "llamacpp")]
    engines.push(EngineInfo {
        id: "llamacpp".into(),
        display_name: "llama.cpp (Metal)".into(),
        available: true,
        installed: true,
        requires_setup: false,
        description: "Fast local inference via Metal GPU (Apple Silicon only)".into(),
    });

    // MLX — only on macOS, only if compiled in
    #[cfg(all(target_os = "macos", feature = "mlx"))]
    {
        let env_ready = app.path().app_data_dir()
            .map(|d| d.join("mlx-env/bin/python3").exists())
            .unwrap_or(false);
        engines.push(EngineInfo {
            id: "mlx".into(),
            display_name: "MLX (Apple Silicon)".into(),
            available: true,
            installed: env_ready,
            requires_setup: !env_ready,
            description: "Apple's MLX framework — best performance on Apple Silicon".into(),
        });
    }

    // vLLM — only on Linux, only if compiled in
    #[cfg(all(target_os = "linux", feature = "vllm"))]
    {
        let cuda = cuda_available();
        let env_ready = /* check vllm-env */ false;
        engines.push(EngineInfo {
            id: "vllm".into(),
            display_name: "vLLM (CUDA)".into(),
            available: cuda,
            installed: env_ready,
            requires_setup: !env_ready,
            description: "High-throughput inference — requires NVIDIA GPU with CUDA".into(),
        });
    }

    // Ollama — all platforms, runtime detection
    #[cfg(feature = "ollama")]
    {
        let running = ollama_running().await;
        engines.push(EngineInfo {
            id: "ollama".into(),
            display_name: "Ollama".into(),
            available: ollama_installed(),
            installed: running,
            requires_setup: false, // We don't install Ollama ourselves
            description: "Community model runner — install from ollama.ai".into(),
        });
    }

    engines
}
```

---

### 2.12 Cross-Compilation & CI Strategy

**Local development from Apple Silicon:** Cannot cross-compile native binaries (CUDA, Windows SDK). Use:
- **OrbStack** or Docker for Linux CPU testing
- GitHub Actions for all release builds

**GitHub Actions Release Matrix:**

```yaml
# .github/workflows/release.yml
jobs:
  build:
    strategy:
      matrix:
        include:
          - os: macos-14          # Apple Silicon (M-series runner)
            features: "llamacpp,mlx,ollama"
            config: tauri.conf.macos.json
            artifact: Scrappy_macOS_arm64.dmg

          - os: ubuntu-22.04      # Linux CUDA (self-hosted GPU runner recommended)
            features: "llamacpp,vllm,ollama"
            config: tauri.conf.linux.json
            artifact: Scrappy_linux_x64.AppImage

          - os: windows-latest    # Windows
            features: "llamacpp,ollama"
            config: tauri.conf.windows.json
            artifact: Scrappy_windows_x64.msi

    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      
      - name: Download llama-server binary for platform
        run: ./scripts/download-llama-server.sh ${{ runner.os }}
        # Downloads precompiled llama-server from llama.cpp GitHub Releases
      
      - name: Download uv binary
        run: ./scripts/download-uv.sh
        # uv is a single static Rust binary — download from astral.sh
        
      - name: Build Tauri app
        run: cargo tauri build --features "${{ matrix.features }}" --config "${{ matrix.config }}"
        
      - name: Upload artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.artifact }}
          path: backend/target/release/bundle/**/*
```

**Key script: `scripts/download-llama-server.sh`** — downloads the pre-compiled llama-server binary from the official [llama.cpp GitHub Releases](https://github.com/ggerganov/llama.cpp/releases) for the target platform. This avoids maintaining our own compile infrastructure.

---

### 2.13 Backend Todos

- [x] **Create `backend/src/engine/mod.rs`** — define `InferenceEngine` trait
- [x] **Create `backend/src/engine/engine_llamacpp.rs`** — wrap existing `SidecarProcess` logic
- [x] **Create `backend/src/engine/engine_mlx.rs`** — uv bootstrap + `mlx_lm.server` spawn
- [x] **Create `backend/src/engine/engine_vllm.rs`** — uv bootstrap + `vllm.entrypoints` spawn (Linux only)
- [x] **Create `backend/src/engine/engine_ollama.rs`** — daemon detection + connect
- [x] **Refactor `SidecarManager`** — `EngineManager` with `Box<dyn InferenceEngine>` + `start_engine`/`stop_engine`/`is_engine_ready` commands
- [x] **Add `selected_inference_engine` to `UserConfig`** — SKIPPED BY DESIGN: one engine per build, no runtime selection needed
- [x] **Add `auto_update_inference_engine` to `UserConfig`** — SKIPPED BY DESIGN
- [x] **Implement `get_supported_engines()`** Tauri command — renamed `get_active_engine_info()` (singular engine per build)
- [x] **Implement `setup_engine(id: String)`** Tauri command — triggers uv bootstrap for MLX/vLLM with progress events
- [x] **Add engine setup progress events** — `engine_setup_progress` event with `{ stage, message }`
- [x] **Update `tauri.conf.json`** — dynamic `tauri.override.json` via `generate_tauri_overrides.sh`
- [x] **Create platform-specific Tauri configs** — dynamic generation via script (better than static files)
- [x] **Create `backend/build.rs`** — compile-time feature flag logging
- [x] **Update `Cargo.toml` features** — `mlx`, `vllm`, `ollama`, `llamacpp` features added
- [x] **Register new commands in `lib.rs`** — all 6 engine commands + 3 HF Hub commands
- [x] **Write `scripts/download-llama-server.sh`** — `scripts/setup_llama.sh` (pre-existing)
- [x] **Write `scripts/download-uv.sh`** — `scripts/setup_uv.sh`

---

### 2.14 Frontend Todos

- [x] **Engine Selector UI** in Settings — SKIPPED BY DESIGN: one engine per build, `ActiveEngineChip` displays the active engine instead
- [x] **Engine Setup Wizard** — `EngineSetupBanner` component with progress bar from `engine_setup_progress` events
- [x] **Update model selection flow** — HFDiscovery handles directory vs. single-file display based on `single_file_model`
- [x] **Display active engine** — `ActiveEngineChip` component in Model Browser header

---

### 2.15 Gaps & Open Questions

| Question | Status | Decision |
|---|---|---|
| When MLX is selected, should llama-server still be bundled? | Decided | No — one engine per build. MLX build does not bundle llama-server. |
| How does Ollama model identity work with our HF download flow? | **Resolved** | Ollama builds use `ollama pull` for Ollama-registry models only. HF-downloaded GGUFs only work with llama.cpp builds. |
| What is the user experience when a user switches engines mid-session? | **Resolved** | No runtime switching — each build has one engine. UI only shows compatible models. |
| Should vLLM support Windows via WSL2? | Decided | No — too complex, WSL2 CUDA is fragile |
| mlx-rs crate for native MLX (vs. Python process) | Decided | Defer — `mlx-rs` is low-level tensors only, no inference server |
| macOS Intel support for MLX? | Decided | No — MLX requires Apple Silicon |

---

## 3. Feature B: HF Hub Dynamic Model Discovery

### 3.1 Problem with Current Model Library

`frontend/src/lib/model-library.ts` has **1,114 lines** of manually hardcoded model definitions including:
- Full `https://huggingface.co/...` direct download URLs (break when HF renames repos)
- Manually typed file sizes as strings — `"7.6 GB"` (stale as soon as new quants drop)
- Manual tracking of mmproj files for multimodal models
- No way to discover new models without a code change + release

`ModelBrowser.tsx` (908 lines) consumes this static data. The `ModelVariant` type currently looks like:
```typescript
interface ModelVariant {
    name: string;      // e.g. "Q4_K_M"
    filename: string;
    url: string;       // hardcoded direct URL
    size: string;      // hardcoded string "7.6 GB"
    vram_required_gb: number;
    recommended_min_ram: number;
}
```

**The static library stays** — it is the "curated/featured/recommended" tab. The HF Discovery feature is **additive**, not a replacement. Users who want curated recommendations use the existing Library; users who want to search anything use the new Discover tab.

---

### 3.2 HF API Endpoints Used

All requests hit the public HF API (no auth required for search; auth via HF token for gated repos):

| Endpoint | Purpose | Auth required |
|---|---|---|
| `GET https://huggingface.co/api/models?search=<q>&tags=<tag>&sort=downloads&direction=-1&limit=20` | Search models by engine tag | No (public) |
| `GET https://huggingface.co/api/models/<repo_id>/tree/main` | List all files in a repo | No (public); Yes for gated |
| `GET https://huggingface.co/<repo_id>/resolve/main/<filename>` | Download a file | No (public); Yes for gated |

The HF token is already stored in OpenClaw config and injected for downloads in `model_manager.rs`. Same token applies to discovery requests for gated repos.

---

### 3.3 Engine-Tag Mapping

The search tag determines what kind of models are returned:

| Engine | HF Tag | File Structure |
|---|---|---|
| `llamacpp` | `gguf` | Single `.gguf` files per quantization |
| `mlx` | `mlx` | Full directory — `*.safetensors` + `config.json` + `*.npz` |
| `vllm` | `awq` or `gptq` or `safetensors` | Full directory |
| `ollama` | Not applicable — use Ollama registry directly | N/A |

Sorting by `downloads` (descending) naturally filters out low-quality / experimental repos and surfaces the most-used community models at the top.

---

### 3.4 discover_hf_models() Command

New Tauri command in `backend/src/hf_hub.rs`:

```rust
#[derive(Serialize, Type)]
pub struct HfModelCard {
    pub id: String,           // "unsloth/Llama-3-8B-GGUF"
    pub author: String,       // "unsloth"
    pub name: String,         // "Llama-3-8B-GGUF"
    pub downloads: u64,
    pub likes: u32,
    pub tags: Vec<String>,
    pub last_modified: String,
    pub gated: bool,          // requires HF token
}

#[tauri::command]
#[specta::specta]
pub async fn discover_hf_models(
    app: AppHandle,
    query: String,
    engine: String,           // "llamacpp" | "mlx" | "vllm"
    limit: Option<u32>,
) -> Result<Vec<HfModelCard>, String> {
    let tag = match engine.as_str() {
        "llamacpp" => "gguf",
        "mlx"      => "mlx",
        "vllm"     => "awq",  // could also be "gptq" — offer both?
        _          => return Ok(vec![]),
    };

    let client = build_hf_client(&app).await?;  // Injects HF token if available
    let url = format!(
        "https://huggingface.co/api/models?search={}&tags={}&sort=downloads&direction=-1&limit={}",
        urlencoding::encode(&query), tag, limit.unwrap_or(20)
    );

    let response: Vec<serde_json::Value> = client
        .get(&url)
        .send().await.map_err(|e| e.to_string())?
        .json().await.map_err(|e| e.to_string())?;

    // Map to HfModelCard structs
    Ok(response.into_iter().filter_map(|v| parse_model_card(v)).collect())
}
```

---

### 3.5 get_model_files() Command — Tree API + Quant Parsing

After user clicks a model card, fetch its file tree and parse it intelligently:

```rust
#[derive(Serialize, Type)]
pub struct HfFileInfo {
    pub filename: String,
    pub size: u64,            // Exact bytes from HF API
    pub size_display: String, // "7.6 GB"
    pub quant_type: Option<String>,  // "Q4_K_M", "Q8_0" etc. (llama.cpp only)
    pub is_mmproj: bool,      // True if this is a multimodal projector
}

#[derive(Serialize, Type)]
pub struct ModelDownloadInfo {
    pub repo_id: String,
    pub is_multi_file: bool,  // true for MLX/vLLM, false for llama.cpp
    pub files: Vec<HfFileInfo>,
    pub mmproj_file: Option<HfFileInfo>,  // Auto-detected for llama.cpp
}

#[tauri::command]
#[specta::specta]
pub async fn get_model_files(
    app: AppHandle,
    repo_id: String,
    engine: String,
) -> Result<ModelDownloadInfo, String> {
    let client = build_hf_client(&app).await?;
    let url = format!("https://huggingface.co/api/models/{}/tree/main", repo_id);
    
    let tree: Vec<serde_json::Value> = client
        .get(&url)
        .send().await.map_err(|e| e.to_string())?
        .json().await.map_err(|e| e.to_string())?;

    let mut info = ModelDownloadInfo {
        repo_id: repo_id.clone(),
        is_multi_file: engine != "llamacpp",
        files: vec![],
        mmproj_file: None,
    };

    if engine == "llamacpp" {
        // Regex to extract quantization type from GGUF filename
        // Matches: Q4_K_M, IQ3_XXS, F16, Q8_0, UD-Q5_K_XL, etc.
        let re = regex::Regex::new(
            r"(?i)[-_]((?:UD-)?(?:q[0-9]_[a-z0-9_]+|iq[0-9]_[a-z0-9_]+|f16|f32|bf16))\.gguf$"
        ).unwrap();

        for file in &tree {
            if let Some(path) = file["path"].as_str() {
                if !path.ends_with(".gguf") { continue; }

                let size = file["size"].as_u64().unwrap_or(0);
                let is_mmproj = path.to_lowercase().contains("mmproj");

                let quant_type = re.captures(path)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_uppercase());

                let file_info = HfFileInfo {
                    filename: path.to_string(),
                    size,
                    size_display: format_bytes(size),
                    quant_type,
                    is_mmproj,
                };

                if is_mmproj {
                    info.mmproj_file = Some(file_info);
                } else {
                    info.files.push(file_info);
                }
            }
        }
        // Sort by file size ascending (smallest quant first in UI)
        info.files.sort_by_key(|f| f.size);

    } else {
        // MLX / vLLM: collect all meaningful files (skip README, images, etc.)
        let skip_exts = [".md", ".jpg", ".png", ".gitattributes", ".gitignore"];
        
        for file in &tree {
            if file["type"].as_str() == Some("directory") { continue; }
            if let Some(path) = file["path"].as_str() {
                if skip_exts.iter().any(|ext| path.ends_with(ext)) { continue; }
                
                let size = file["size"].as_u64().unwrap_or(0);
                info.files.push(HfFileInfo {
                    filename: path.to_string(),
                    size,
                    size_display: format_bytes(size),
                    quant_type: None,
                    is_mmproj: false,
                });
            }
        }
    }

    Ok(info)
}
```

---

### 3.6 Multi-File vs. Single-File Download Logic

The `is_multi_file` flag drives completely different UX and download logic:

**`is_multi_file = false` (llama.cpp, GGUF)**
- Show quantization dropdown: `Q4_K_M (4.3 GB)`, `Q8_0 (8.3 GB)`, etc.
- User picks one file
- If `mmproj_file` is present: silently add it to the download queue alongside the selected main file
- Result: 1 or 2 files downloaded into `app_data_dir/models/`

**`is_multi_file = true` (MLX, vLLM)**
- Show total size (sum of all `HfFileInfo.size`)
- Single "Download Repository" button
- All files in `info.files` are downloaded, preserving any subdirectory structure
- Result: a full model directory in `app_data_dir/models/{repo_id_sanitized}/`
- The engine is then pointed at the **directory**, not a file

---

### 3.7 download_hf_model_files() Command

Reuses the existing streaming download infrastructure from `model_manager.rs`:

```rust
#[tauri::command]
#[specta::specta]
pub async fn download_hf_model_files(
    app: AppHandle,
    download_manager: State<'_, DownloadManager>,
    repo_id: String,
    files_to_download: Vec<String>,   // Array of filenames from get_model_files()
    dest_subdir: Option<String>,      // Custom subdir name; defaults to sanitized repo_id
) -> Result<String, String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let sanitized = repo_id.replace('/', "_");
    let dest_dir = app_data
        .join("models")
        .join(dest_subdir.unwrap_or(sanitized));

    for filename in &files_to_download {
        let url = format!(
            "https://huggingface.co/{}/resolve/main/{}",
            repo_id, filename
        );
        let dest_filename = format!("{}/{}", repo_id.replace('/', "_"), filename);
        
        // Delegate to existing download_model() which handles:
        // - reqwest streaming with byte chunks
        // - progress events (download_progress)
        // - cancellation via Notify
        // - HF token injection
        download_model_internal(&app, &download_manager, url, dest_filename).await?;
    }

    Ok(dest_dir.to_string_lossy().to_string())
}
```

This reuses the existing `download_model` implementation's inner logic rather than duplicating the streaming/progress/cancellation machinery.

---

### 3.8 ModelDownloadInfo Type

Full type definition for TypeScript bindings (generated via `specta`):

```rust
// Exposed to TypeScript via specta
pub struct HfFileInfo {
    pub filename: String,
    pub size: u64,            // bytes
    pub size_display: String, // "7.6 GB", "820 MB"
    pub quant_type: Option<String>,  // "Q4_K_M" or null
    pub is_mmproj: bool,
}

pub struct ModelDownloadInfo {
    pub repo_id: String,
    pub is_multi_file: bool,
    pub files: Vec<HfFileInfo>,
    pub mmproj_file: Option<HfFileInfo>,
}
```

---

### 3.9 Static Library vs. Discovery: Coexistence Strategy

```
ModelBrowser.tsx
├── Tab: "Library"        ← existing curated models from model-library.ts
│   - Handpicked, annotated with VRAM requirements
│   - Works offline (no HF API calls)
│   - Download URLs already known
│
└── Tab: "Discover"       ← NEW: live HF search
    - Search box → calls discover_hf_models()
    - Engine selector (shows available engines)
    - Model cards with download count badges
    - Click → calls get_model_files() → shows quant picker
    - Download → calls download_hf_model_files()
    - Downloaded models appear in the "Library" tab
      local-only section immediately after download
```

This avoids breaking the existing 908-line `ModelBrowser.tsx` — the Discover tab is a new sibling panel, not a replacement for the existing UI.

---

### 3.10 What Already Works (No Changes Needed)

These existing pieces are reused as-is by the HF Discovery feature:

| Feature | File | Reuse |
|---|---|---|
| Streaming download + progress events | `model_manager.rs` | Full reuse — `download_hf_model_files` delegates here |
| Cancellation via `tokio::sync::Notify` | `model_manager.rs` | Full reuse |
| HF token injection (`Authorization: Bearer`) | `model_manager.rs` | Full reuse — same config source |
| Download progress UI in `ModelBrowser.tsx` | Frontend | Full reuse — same `downloading` state dict |
| Local model scanning (`list_models`) | `model_manager.rs` | Full reuse — discovered models appear automatically |
| `ProviderKind::Local` → OpenAI API | `rig_lib/unified_provider.rs` | Full reuse — no changes needed |
| Health-check polling | `sidecar.rs` | Full reuse |

---

### 3.11 Backend Todos

- [x] **Create `backend/src/hf_hub.rs`** module
- [x] **Implement `discover_hf_models()`** — HF search API with tag routing
- [x] **Implement `get_model_files()`** — HF Tree API + GGUF regex + mmproj detection
- [x] **Implement `download_hf_model_files()`** — multi-file downloader reusing existing streaming logic
- [x] **Add `build_hf_client()`** helper — constructs `reqwest::Client` with optional HF token
- [x] **Add `format_bytes()` utility** — formats u64 bytes as "7.6 GB" etc.
- [x] **Add `regex` crate to `Cargo.toml`** — already present
- [x] **Register new commands in `lib.rs`**:
  - `hf_hub::discover_hf_models`
  - `hf_hub::get_model_files`
  - `hf_hub::download_hf_model_files`
- [x] **Generate TypeScript bindings** via specta (automatic on next `tauri dev`)

---

### 3.12 Frontend Todos

- [x] **Create `frontend/src/components/settings/HFDiscovery.tsx`** — Discover tab component
  - Search input with debounce (350ms)
  - Engine info badge (auto-detected from build)
  - Model card grid — repo_id, download count, likes, `gated` badge
  - Click-to-expand: calls `get_model_files()`, shows quant picker or total size
  - Download button — uses `downloadHfFiles()` from model-context (shared download state)
  - mmproj auto-include note: *"Vision projector will be included"*
- [x] **Update `ModelBrowser.tsx`** — added "Discover" tab alongside "Library"
- [x] **Extend `model-context.tsx`** — `downloadHfFiles(repoId, files, destSubdir?)` method added
- [x] **Update `DownloadProgress` handling** — global `downloading` state tracks HF downloads

---

### 3.13 Gaps & Open Questions

| Question | Status | Decision |
|---|---|---|
| Should VRAM requirements be auto-fetched? | **Resolved** | Skip for v1 — HF API does not provide VRAM data. Use local heuristics later. |
| What happens if user downloads a GGUF via Discover while MLX is active engine? | **Resolved** | Impossible — one engine per build. HF Discovery only shows models tagged for the build's engine. |
| Rate limiting — HF API limits unauthenticated requests | **Resolved** | 429 handling implemented with exponential backoff + user message. Users typically have HF tokens (prompted during setup). |
| Should search results paginate or load-more? | Decided | Start with `limit=20` + load-more button; pagination via `offset` param |
| Model card thumbnails / preview images from HF? | Decided: No for v1 | Keep cards simple — name, downloads, tags only |
| Ollama model compatibility with GGUF downloads? | **Resolved** | See engine section — Ollama builds only support `ollama pull`, no HF GGUF downloads. |

---

## 4. Shared Infrastructure Changes

### New Files to Create

| File | Purpose |
|---|---|
| `backend/src/engine/mod.rs` | `InferenceEngine` trait + engine factory |
| `backend/src/engine/engine_llamacpp.rs` | llama.cpp sidecar impl |
| `backend/src/engine/engine_mlx.rs` | MLX uv bootstrap + process |
| `backend/src/engine/engine_vllm.rs` | vLLM uv bootstrap + process |
| `backend/src/engine/engine_ollama.rs` | Ollama daemon detection |
| `backend/src/hf_hub.rs` | HF API discovery + download |
| `backend/build.rs` | Compile-time CUDA probe + cfg flags |
| `backend/tauri.conf.macos.json` | macOS bundle config |
| `backend/tauri.conf.linux.json` | Linux bundle config |
| `backend/tauri.conf.windows.json` | Windows bundle config |
| `scripts/download-llama-server.sh` | CI: fetch precompiled llama.cpp binaries |
| `scripts/download-uv.sh` | CI: fetch uv binary |
| `.github/workflows/release.yml` | Multi-platform CI matrix |
| `frontend/src/components/settings/HFDiscovery.tsx` | Discover tab UI |

### Files to Modify

| File | Changes |
|---|---|
| `backend/src/config.rs` | Add `selected_inference_engine`, `auto_update_inference_engine` |
| `backend/src/sidecar.rs` | Refactor to use `InferenceEngine` trait — major refactor |
| `backend/src/lib.rs` | Register new commands |
| `backend/Cargo.toml` | Add features, add `regex` crate if missing |
| `backend/tauri.conf.json` | Add `bin/uv` to `externalBin`; reference platform configs |
| `frontend/src/components/settings/ModelBrowser.tsx` | Add Discover tab |
| `frontend/src/components/model-context.tsx` | Add `downloadHfFiles()` method |

---

## 5. Implementation Order (Recommended)

The features are independent enough that either could be implemented first, but this order minimises rework:

### Phase 1 — Foundation (no user-visible changes)
1. Add `selected_inference_engine` to `UserConfig` + generate new bindings
2. Implement `get_supported_engines()` command (returns only `llamacpp` + `none` for now)
3. Create `engine/mod.rs` with `InferenceEngine` trait
4. Create `engine/engine_llamacpp.rs` — wrap current `SidecarProcess`
5. Refactor `SidecarManager` to use `Box<dyn InferenceEngine>` internally

### Phase 2 — HF Discovery Backend
1. Create `hf_hub.rs` with `discover_hf_models()` + `get_model_files()`
2. Implement `download_hf_model_files()`
3. Register all commands, generate bindings

### Phase 3 — HF Discovery Frontend
1. Create `HFDiscovery.tsx` Discover tab
2. Update `ModelBrowser.tsx` to include it
3. Extend `model-context.tsx` with new download method

### Phase 4 — MLX Engine
1. Create `engine/engine_mlx.rs` with uv bootstrap
2. Add MLX to `get_supported_engines()` (macOS with `#[cfg(feature = "mlx")]`)
3. Create engine setup wizard frontend component
4. Update Tauri config files + CI

### Phase 5 — vLLM + Ollama Engines
1. `engine/engine_vllm.rs` (Linux CUDA)
2. `engine/engine_ollama.rs` (detection + connect)
3. Update CI matrix for Linux build

### Phase 6 — CI & Release Pipeline
1. Write `scripts/download-llama-server.sh`
2. Write `scripts/download-uv.sh`
3. Create `.github/workflows/release.yml`
4. Platform-specific `tauri.conf.*.json` files

---

## 6. Known Constraints & Non-Goals

| Constraint | Detail |
|---|---|
| **Zero changes to chat pipeline** | `chat.rs`, `rig_lib`, `Orchestrator` require no modifications — all engines expose the same OpenAI HTTP API |
| **No cross-compilation from Mac** | Native binaries (CUDA, Windows SDK) require native OS runners — handled by CI |
| **mlx-rs is not used** | The `mlx-rs` crate provides raw tensor ops, not an inference server — MLX integration is via spawning `mlx_lm` Python process |
| **vLLM is Linux-only** | WSL2 CUDA support is too fragile for a v1 implementation |
| **Ollama model management is not owned by Scrappy** | We detect and connect; we do not manage Ollama's model library |
| **HF Cache (`~/.cache/huggingface`) is not reused** | We download to `app_data_dir/models/` — HF native cache reuse is a v2 nice-to-have |
| **Static model library is preserved** | `model-library.ts` stays as the curated/recommended list — HF Discovery is additive |
| **Jinja template rendering stays with llama-server** | The current approach (pass `--chat-template` to llama-server) works for all engines that use llama.cpp; MLX handles its own templates internally |
