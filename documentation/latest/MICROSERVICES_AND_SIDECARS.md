# Scrappy — Microservices & Sidecar Reference

> **Last updated:** 2026-03-01  
> **Scope:** All external processes spawned or managed by the Tauri host, the multi-engine inference system (`InferenceEngine` trait + `EngineManager`), the IronClaw in-process agent engine, the `scrappy-mcp-tools` Rust crate, and all build- and dev-time infrastructure scripts.

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [SidecarManager — the Process Supervisor](#2-sidecarmanager--the-process-supervisor)
2a. [Multi-Engine Inference System](#2a-multi-engine-inference-system)
   - 2a.1 [InferenceEngine Trait](#2a1-inferenceengine-trait)
   - 2a.2 [EngineManager — Tauri Managed State](#2a2-enginemanager--tauri-managed-state)
   - 2a.3 [Engine Implementations](#2a3-engine-implementations)
   - 2a.4 [Engine Tauri Commands](#2a4-engine-tauri-commands)
   - 2a.5 [Engine Setup & Bootstrap](#2a5-engine-setup--bootstrap)
2b. [HuggingFace Hub Model Discovery](#2b-huggingface-hub-model-discovery)
2c. [InferenceRouter & SecretStore](#2c-inferencerouter--secretstore)
3. [Sidecar: llama-server (LLM Inference)](#3-sidecar-llama-server-llm-inference)
   - 3.1 [Chat Server Instance](#31-chat-server-instance)
   - 3.2 [Embedding Server Instance](#32-embedding-server-instance)
   - 3.3 [Summarizer Server Instance](#33-summarizer-server-instance)
4. [Sidecar: whisper-server (Speech-to-Text)](#4-sidecar-whisper-server-speech-to-text)
5. [Sidecar: sd-server (Image Generation)](#5-sidecar-sd-server-image-generation)
5a. [Sidecar: piper (Text-to-Speech)](#5a-sidecar-piper-text-to-speech)
6. [IronClaw Agent Engine (In-Process)](#6-ironclaw-agent-engine-in-process)
   - 6.1 [Purpose and Role](#61-purpose-and-role)
   - 6.2 [Integration Architecture](#62-integration-architecture)
   - 6.3 [Startup Flow](#63-startup-flow)
   - 6.4 [IronClaw API Surface](#64-ironclaw-api-surface)
   - 6.5 [StatusUpdate → UiEvent Conversion](#65-statusupdate--uievent-conversion)
   - 6.6 [Background Tasks](#66-background-tasks)
   - 6.7 [Remote Deployment (Retained)](#67-remote-deployment-retained)
7. [scrappy-mcp-tools Crate](#7-scrappy-mcp-tools-crate)
   - 7.1 [Rhai Sandbox](#71-rhai-sandbox)
   - 7.2 [MCP HTTP Client](#72-mcp-http-client)
   - 7.3 [Tool Discovery & Registry Cache](#73-tool-discovery--registry-cache)
   - 7.4 [Skill System](#74-skill-system)
   - 7.5 [Built-in Tool Modules](#75-built-in-tool-modules)
8. [External MCP Server Integration](#8-external-mcp-server-integration)
   - 8.1 [What it is](#81-what-it-is)
   - 8.2 [McpRequestHandler — Reverse-RPC Entry Point](#82-mcprequesthandler--reverse-rpc-entry-point)
   - 8.3 [ToolRouter — Three-Tier Dispatch](#83-toolrouter--three-tier-dispatch)
   - 8.4 [sandbox_factory — Sandbox Wiring](#84-sandbox_factory--sandbox-wiring)
   - 8.5 [tool_discovery — Unified Tool Search](#85-tool_discovery--unified-tool-search)
   - 8.6 [Host Tools Reference](#86-host-tools-reference)
   - 8.7 [Remote MCP Tool Bindings in the Sandbox](#87-remote-mcp-tool-bindings-in-the-sandbox)
   - 8.8 [Activation / Configuration](#88-activation--configuration)
9. [ProcessTracker — Zombie Prevention](#9-processtracker--zombie-prevention)
10. [Build-time Infrastructure Scripts](#10-build-time-infrastructure-scripts)
    - 10.1 [download_ai_binaries.js](#101-download_ai_binariesjs)
    - 10.2 [download_node.js](#102-download_nodejs)
    - 10.3 [setup_uv.sh](#103-setup_uvsh)
    - 10.4 [setup_llama.sh](#104-setup_llamash)
    - 10.5 [generate_tauri_overrides.sh](#105-generate_tauri_overridessh)
    - 10.6 [setup_chromium.sh](#106-setup_chromiumsh)
11. [Port & Token Reference](#11-port--token-reference)
12. [Tauri Sidecar Registration](#12-tauri-sidecar-registration)
13. [Event Flow Diagram](#13-event-flow-diagram)
14. [Replacing or Extending Services](#14-replacing-or-extending-services)

---

## 1. Architecture Overview

Scrappy's process model is a **multi-process star topology**: the Rust/Tauri host sits at the center and manages several child processes that each provide a distinct capability.

**Each build of Scrappy targets exactly ONE inference engine** (selected via Cargo feature flags at compile time). The `EngineManager` holds the active `Box<dyn InferenceEngine>` instance; the rest of the stack is engine-agnostic.

```
┌──────────────────────────────────────────────────────────────┐
│                   Tauri Host (Rust)                          │
│                   backend/src/lib.rs                         │
│                                                              │
│  EngineManager ──── Box<dyn InferenceEngine>                 │
│    (one per build)  ├─ LlamaCppEngine  (feature: llamacpp)   │
│                     ├─ MlxEngine       (feature: mlx)        │
│                     ├─ VllmEngine      (feature: vllm)       │
│                     └─ OllamaEngine    (feature: ollama)     │
│                                                              │
│  SidecarManager ─── llama-server (chat)      :53755  [1]     │
│                 ─── llama-server (embedding) :53756           │
│                 ─── whisper-server (STT)     :53757           │
│                 ─── llama-server (summarizer):53758           │
│                                                              │
│  IronClawState ──── ironclaw library (in-process)            │
│                     ├─ Agent + background tasks              │
│                     ├─ TauriChannel (Channel impl)           │
│                     └─ KeychainSecretsAdapter                │
│                                                              │
│  ProcessTracker — global PID registry for cleanup            │
└──────────────────────────────────────────────────────────────┘
         │
         │  sd binary / whisper-cli / piper
         │  (invoked as one-shot CLI tools, no persistent server)
         │
┌────────┴────────────────────┐
│  Frontend (React WebView)   │
│  Communicates via:          │
│  • Tauri IPC commands       │
│  • Tauri Events             │
└─────────────────────────────┘

[1] For llamacpp builds. MLX/vLLM spawn a Python process
    instead. Ollama connects to an external daemon.
```

All child processes are **sidecar binaries** bundled inside the Tauri app (or downloaded at first run) and managed exclusively by the host — they are never user-accessible directly.

---

## 2. SidecarManager — the Process Supervisor

**File:** `backend/src/sidecar.rs`  
**Rust type:** `SidecarManager` (cloneable, all fields wrapped in `Arc<Mutex<>>` or `Arc<AtomicBool>`)

The `SidecarManager` is registered as Tauri global state on app startup. It provides:

| Field | Type | Purpose |
|-------|------|---------|
| `chat_process` | `Arc<Mutex<Option<SidecarProcess>>>` | Active llama-server chat instance |
| `embedding_process` | same | Active llama-server embedding instance |
| `summarizer_process` | same | Active llama-server summarizer instance |
| `stt_process` | same | Active whisper-server instance |
| `stt_model_path` | `Arc<Mutex<Option<String>>>` | Path to Whisper GGUF (CLI use) |
| `image_model_path` | `Arc<Mutex<Option<String>>>` | Path to SD model (CLI invocation) |
| `tts_model_path` | `Arc<Mutex<Option<String>>>` | Path to TTS model (reserved) |
| `is_chat_stop_intentional` | `Arc<Mutex<bool>>` | Guards restart vs. crash distinction |
| `cancellation_token` | `Arc<AtomicBool>` | Signals orchestrator to abort generation |
| `generation_lock` | `Arc<tokio::sync::Mutex<()>>` | Prevents concurrent inference calls |
| `detected_model_family` | `Arc<Mutex<Option<String>>>` | GGUF-detected family (chatml/llama3/etc.) |

### SidecarProcess struct

```rust
pub struct SidecarProcess {
    pub child:        Option<CommandChild>,  // tauri_plugin_shell handle
    pub port:         u16,
    pub token:        String,                // 32-char random alphanumeric API key
    pub context_size: u32,
    pub model_family: String,
}
```

**Drop semantics:** `Drop for SidecarProcess` calls `child.kill()` — any process automatically dies when its `SidecarProcess` is dropped, even if the Tauri app crashes.

### Port / Token generation

```rust
fn generate_config(preferred_port: Option<u16>) -> (u16, String) {
    // Try preferred port; fall back to OS-assigned random port
    // Token: 32 random alphanumeric chars via rand::Alphanumeric
}
```

### `SidecarEvent` — events emitted to the frontend

```rust
pub enum SidecarEvent {
    Started  { service: String },
    Stopped  { service: String },
    Crashed  { service: String, code: i32 },
    Error    { service: String, message: String },
    Progress { service: String, message: String, progress: f32, total: f32 },
}
```
Emitted on Tauri event channel `"sidecar_event"`.

---

## 2a. Multi-Engine Inference System

**Files:** `backend/src/engine/mod.rs`, `engine_llamacpp.rs`, `engine_mlx.rs`, `engine_vllm.rs`, `engine_ollama.rs`

The multi-engine system abstracts how **local chat inference** is provided. Each build of Scrappy compiles exactly **one** engine implementation, selected by a Cargo feature flag (`default = ["llamacpp"]` in `backend/Cargo.toml`).

> **Scope:** This section covers only local chat inference engines. For the full multi-modal routing layer (Chat + Embedding + TTS + STT + Diffusion, local + cloud), see [§2c InferenceRouter & SecretStore](#2c-inferencerouter--secretstore).

### 2a.1 InferenceEngine Trait

```rust
#[async_trait]
pub trait InferenceEngine: Send + Sync {
    async fn start(&self, model_path: &str, context_size: u32, options: EngineStartOptions)
        -> Result<(u16, String), String>;  // Returns (port, api_token)
    async fn stop(&self) -> Result<(), String>;
    async fn is_ready(&self) -> bool;
    fn base_url(&self) -> Option<String>;
    fn display_name(&self) -> &'static str;
    fn engine_id(&self) -> &'static str;
    fn uses_single_file_model(&self) -> bool;
    fn hf_search_tag(&self) -> &'static str;
}
```

All engines expose an **OpenAI-compatible HTTP API** on a local port, so the rest of the stack (`chat.rs`, `rig_lib`, Orchestrator) is engine-agnostic.

### 2a.2 EngineManager — Tauri Managed State

```rust
pub struct EngineManager {
    pub engine: tokio::sync::Mutex<Option<Box<dyn InferenceEngine>>>,
    pub app_data_dir: PathBuf,
}
```

Registered as `app.manage(EngineManager::new(app_data_dir))` in `lib.rs`. The `create_engine()` method instantiates the correct engine struct based on compile-time feature flags (priority: mlx > vllm > llamacpp > ollama).

### 2a.3 Engine Implementations

| Engine | File | Binary | Bootstrap | Model Format | HF Tag |
|--------|------|--------|-----------|-------------|--------|
| **LlamaCpp** | `engine_llamacpp.rs` | Bundled `llama-server` sidecar | None | Single GGUF file | `gguf` |
| **MLX** | `engine_mlx.rs` | `uv` (bundled) → Python + `mlx-openai-server` | First-launch: creates `mlx-env/` venv, installs `mlx-openai-server` + `mlx-vlm` (~200MB, 2-3 min) | Safetensors directory | `mlx` |
| **vLLM** | `engine_vllm.rs` | `uv` (bundled) → Python + `vllm.entrypoints.openai.api_server` | First-launch: creates `vllm-env/` venv, installs `vllm` (~1GB, 5-10 min) | AWQ / HF directory | `awq` |
| **Ollama** | `engine_ollama.rs` | External `ollama` daemon | None (user must install [ollama.ai](https://ollama.ai)) | GGUF (managed by Ollama) | `gguf` |

**MLX/vLLM bootstrap flow:**
1. `setup_engine` Tauri command invoked (via `EngineSetupBanner` frontend component)
2. `engine.bootstrap()` creates a Python virtual environment via the bundled `uv` sidecar
3. Installs the inference framework into the venv (`pip install mlx-openai-server mlx-vlm` or `pip install vllm`)
4. **(MLX only)** Applies the VLM `attention_mask` → `mask` patch (see below)
4. Progress emitted via `engine_setup_progress` Tauri events (`{ stage, message }`)
5. Subsequent starts use the cached venv — no re-bootstrap needed

### 2a.4 Engine Tauri Commands

| Command | Purpose |
|---------|---------|
| `get_active_engine_info` | Returns `EngineInfo { id, display_name, available, requires_setup, description, hf_tag, single_file_model }` — used by frontend to adapt UI |
| `get_engine_setup_status` | Returns `EngineSetupStatus { needs_setup, setup_in_progress, message }` — checks if venv exists |
| `setup_engine` | Triggers first-launch bootstrap (MLX/vLLM only); emits `engine_setup_progress` events |
| `start_engine` | Starts the active engine with a given model path + context size; returns `{ port, token }` |
| `stop_engine` | Stops the active engine and frees GPU/RAM |
| `is_engine_ready` | Health check — returns `true` if the engine's HTTP endpoint is accepting requests |

### 2a.5 Engine Setup & Bootstrap

**Setup detection:** `get_engine_setup_status` checks for the existence of the Python venv:
- MLX: `$APP_DATA/mlx-env/bin/python3`
- vLLM: `$APP_DATA/vllm-env/bin/python3`
- llamacpp / Ollama: always returns `needs_setup = false`

**Frontend flow:**
1. `ModelBrowser` renders `EngineSetupBanner` which calls `get_engine_setup_status` on mount
2. If `needs_setup = true`, an amber banner appears with a "Set Up Now" button
3. Clicking it calls `setup_engine` and shows a 3-stage progress bar (Create Environment → Install Packages → Ready)
4. `engine_setup_progress` events update the UI in real time
6. On completion, banner turns green; on error, banner turns red with a "Retry" button

**MLX VLM Patch (`apply_vlm_attention_mask_patch`):**

After installing `mlx-openai-server`, the bootstrap automatically patches the `mlx-vlm` handler (`app/handler/mlx_vlm.py`) to rename `attention_mask` → `mask` in the `vision_inputs` dictionary. This fixes an upstream bug where `stream_generate()` expects a `mask` key but the HuggingFace processor returns `attention_mask`, causing a crash (`expand_dims(NoneType, int)`) when processing images with models like Gemma 3. The patch is idempotent and survives package upgrades.

**MLX Multimodal Support:**

When the loaded model is a vision-language model (VLM), the engine passes `--model-type multimodal` to `mlx-openai-server`. The model's `config.json` is inspected at startup to detect `model_type` values associated with vision capabilities (e.g. `gemma3`, `qwen2_vl`, `pixtral`, `llava`).

**Tauri event `engine_setup_progress`:**
```json
{ "stage": "creating_venv" | "installing" | "complete" | "error", "message": "..." }
```

---

## 2b. HuggingFace Hub Model Discovery

**File:** `backend/src/hf_hub.rs`

Provides live search of HuggingFace Hub models, filtered by the active engine's tag. Used by the **Discover** tab in the frontend `ModelBrowser`.

### Tauri Commands

| Command | Purpose |
|---------|---------|
| `discover_hf_models(query, engine, limit?, pipeline_tags?)` | Searches HF API (`/api/models?search=...&filter=...&pipeline_tag=...`) sorted by downloads. Supports multiple `pipeline_tags` (e.g. `["text-generation", "image-text-to-text"]`) — makes one request per tag, merges, deduplicates, re-sorts. Post-filters results to verify the engine tag is genuinely present in each model's `tags` array. Returns `Vec<HfModelCard>`. |
| `get_model_files(repo_id, engine)` | Fetches the repo file tree, parses GGUF quantization types or MLX/vLLM directory listing. Returns `ModelDownloadInfo`. |
| `download_hf_model_files(repo_id, files, dest_subdir?, category?)` | Downloads selected files to `$APP_DATA/models/{category}/`, emitting `download_progress` events. `category` defaults to `"LLM"` but can be `"Embedding"`, `"Diffusion"`, `"STT"`, etc. |

### Engine-to-tag mapping

| Engine ID | HF Filter Tag | API Parameter | Model format |
|-----------|--------------|---------------|--------------|
| `llamacpp` | `gguf` | `filter=gguf` | Single GGUF files with quantization picker |
| `mlx` | `mlx` | `filter=mlx` | Safetensors directory (Download All) |
| `vllm` | `awq` | `filter=awq` | AWQ/HF directory (Download All) |
| `ollama` | `gguf` | `filter=gguf` | Single GGUF files (same as llamacpp) |

> **Why `filter=` instead of `tags=` or `library=`?** The HF API's `tags=` and `library=` parameters are unreliable search hints that return false positives (e.g. `library=mlx` returns `sentence-transformers/all-MiniLM-L6-v2`). The `filter=` parameter performs strict tag matching — the same mechanism the HF Hub web UI uses when clicking library filter chips.

### Key features
- **HF token injection**: Reads `huggingface_token` from `SecretStore` (macOS Keychain) for gated model access
- **GGUF quant detection**: Regex extracts quantization type (`Q4_K_M`, `IQ3_XXS`, `F16`, etc.) from filenames
- **mmproj auto-detection**: Identifies multimodal projector files and auto-includes them in downloads
- **Post-filter validation**: After each API response, verifies the engine tag is genuinely in each model's `tags` array (safety net against API false positives)
- **Rate limit handling**: Returns user-friendly error message when HF API rate limit is hit
- **Progress events**: Emits `download_progress` events compatible with the existing `model_manager.rs` format

---

## 2c. InferenceRouter & SecretStore

**Files:** `backend/src/inference/mod.rs`, `backend/src/inference/backends/`, `backend/src/inference/providers.rs`, `backend/src/secret_store.rs`

The `InferenceRouter` is a **cross-modal routing layer** that manages all 5 AI modalities (Chat, Embedding, TTS, STT, Diffusion) and routes each to either a local sidecar or a cloud provider backend. It complements the `EngineManager` (§2a), which handles only local chat inference.

> **Relationship to §2a:** `EngineManager` manages the local inference engine binary (llama-server, MLX, vLLM, Ollama) for chat. `InferenceRouter` sits above it and adds: (a) cloud provider fallback for chat, (b) 4 additional modalities (Embedding, TTS, STT, Diffusion), (c) per-modality backend selection stored in `UserConfig`.

### Architecture

```
┌─────────────────────────────────────────────────┐
│  InferenceRouter (Tauri Managed State)           │
│    Arc<SecretStore> ─────► macOS Keychain         │
│                                                   │
│    ┌──── Chat ────┐  ┌── Embedding ─┐            │
│    │ Local (sidecar)│  │ Local (sidecar)│          │
│    │ Cloud (14 APIs)│  │ Cloud (Cohere, │          │
│    └────────────────┘  │ Voyage, OpenAI)│          │
│                        └───────────────┘          │
│    ┌──── TTS ─────┐  ┌──── STT ────┐             │
│    │ Local (piper) │  │ Local (whisper)│           │
│    │ Cloud (11Labs)│  │ Cloud (Deepgram)│          │
│    └──────────────┘  └───────────────┘            │
│    ┌── Diffusion ─┐                               │
│    │ Local (sd.cpp)│                               │
│    │ Cloud(Stability│                              │
│    │  fal, OpenAI) │                               │
│    └──────────────┘                               │
└─────────────────────────────────────────────────┘
```

### SecretStore — Single Source of Truth for API Keys

**File:** `backend/src/secret_store.rs`

`SecretStore` is a zero-state wrapper over the `keychain` module. It provides 21 typed convenience accessors:

| Method | Keychain Slug |
|--------|---------------|
| `openai_key()` | `openai` |
| `anthropic_key()` | `anthropic` |
| `gemini_key()` | `gemini` |
| `groq_key()` | `groq` |
| `openrouter_key()` | `openrouter` |
| `xai_key()` | `xai` |
| `mistral_key()` | `mistral` |
| `together_key()` | `together` |
| `venice_key()` | `venice` |
| `moonshot_key()` | `moonshot` |
| `minimax_key()` | `minimax` |
| `nvidia_key()` | `nvidia` |
| `qianfan_key()` | `qianfan` |
| `xiaomi_key()` | `xiaomi` |
| `cohere_key()` | `cohere` |
| `voyage_key()` | `voyage` |
| `deepgram_key()` | `deepgram` |
| `elevenlabs_key()` | `elevenlabs` |
| `stability_key()` | `stability` |
| `fal_key()` | `fal` |
| `huggingface_key()` | `huggingface` |

All accessors delegate to `keychain::get_key(slug)`, which reads from the in-memory cache populated at startup by `keychain::load_all()`.

### Provider Endpoints Registry

`PROVIDER_ENDPOINTS` in `inference/providers.rs` is a compile-time map of 14 cloud providers:

| Provider | Base URL | Modalities |
|----------|---------|------------|
| OpenAI | `api.openai.com` | Chat, Embedding, TTS, Diffusion |
| Anthropic | `api.anthropic.com` | Chat |
| Google/Gemini | `generativelanguage.googleapis.com` | Chat, Embedding |
| Groq | `api.groq.com` | Chat |
| OpenRouter | `openrouter.ai` | Chat |
| xAI | `api.x.ai` | Chat |
| Mistral | `api.mistral.ai` | Chat, Embedding |
| Together | `api.together.xyz` | Chat, Embedding |
| Venice | `api.venice.ai` | Chat |
| Cohere | `api.cohere.com` | Chat, Embedding |
| Deepgram | `api.deepgram.com` | STT |
| ElevenLabs | `api.elevenlabs.io` | TTS |
| Stability AI | `api.stability.ai` | Diffusion |
| fal.ai | `fal.run` | Diffusion |

### Tauri Commands

| Command | Purpose |
|---------|---------|
| `get_inference_backends` | Returns current backend selection for all 5 modalities |
| `update_inference_backend` | Updates backend for a specific modality; persists to `UserConfig` |

### UserConfig Integration

`UserConfig` (in `config.rs`) stores the user's per-modality backend preferences:

| Field | Type | Default |
|-------|------|--------|
| `chat_backend` | `String` | `"local"` |
| `embedding_backend` | `String` | `"local"` |
| `tts_backend` | `String` | `"local"` |
| `stt_backend` | `String` | `"local"` |
| `diffusion_backend` | `String` | `"local"` |
| `inference_models` | `HashMap<String, String>` | `{}` |

---

## 3. Sidecar: llama-server (LLM Inference)

> **Note:** This section describes the llama.cpp sidecar used in `llamacpp` builds (the default). For other engines, see §2a.

**Binary:** `llama-server` (from [llama.cpp](https://github.com/ggerganov/llama.cpp) release `b4618`)  
**Platform naming:** `llama-server-aarch64-apple-darwin` (macOS ARM64), `llama-server-x86_64-unknown-linux-gnu` (Linux x64), `llama-server-x86_64-pc-windows-msvc.exe` (Windows x64)  
**API:** OpenAI-compatible HTTP REST (`/v1/chat/completions`, `/v1/embeddings`, `/health`)

The same binary is launched up to **three times** in different modes for different roles.

### 3.1 Chat Server Instance

**Default port:** `53755` (preferred; falls back to random if occupied)  
**Tauri command:** `start_chat_server`

**Arguments passed:**

| Arg | Value | Notes |
|-----|-------|-------|
| `--model` | user-selected GGUF path | |
| `--ctx-size` | user-set context (default 16384) | |
| `--n-gpu-layers` | `-1` (all layers on GPU) | |
| `--host` | `127.0.0.1` or `0.0.0.0` | Controlled by `expose_network` option |
| `--port` | `53755` (or random) | |
| `--api-key` | random 32-char token | Used for all subsequent requests |
| `--cache-prompt` | (flag) | Enables KV-cache persistence across requests |
| `--slot-save-path` | `$APP_DATA/prompt_cache/` | Directory for cache slot files |
| `--flash-attn on` | (flag) | Metal/CUDA Flash Attention |
| `--cont-batching` | (flag) | Continuous batching for throughput |
| `--mlock` | optional | Lock model weights in RAM |
| `--cache-type-k q4_0` | optional | Q4 KV cache quantization |
| `--cache-type-v q4_0` | optional | Q4 KV cache quantization |
| `--chat-template` | auto-detected from GGUF | See template detection below |
| `--mmproj` | auto-detected `.mmproj` file | Enables vision/multimodal |

**Template detection (model family auto-detection):**

The `gguf::read_gguf_metadata()` function reads the GGUF file header to detect the model family before spawning:

| Detected Family | Chat Template | Notes |
|----------------|--------------|-------|
| `llama3` | `LLAMA3_TEMPLATE` | Explicit Jinja2 template injected |
| `mistral` | `MISTRAL_TEMPLATE` | Explicit Jinja2 template injected |
| `gemma` | `None` (native) | llama-server handles Gemma natively |
| `qwen` | `QWEN_TEMPLATE` | Explicit Jinja2 template injected |
| `deepseek` | `None` (native) | llama-server handles natively |
| `glm` | `None` (native) | llama-server handles natively |
| `chatml` / unknown | `CHATML_TEMPLATE` | Default fallback |

**Note on stop tokens:** Stop tokens are NOT injected as `--stop` CLI args (llama-server doesn't support that flag). They are enforced at the HTTP API request level by the OpenClaw engine config.

**mmproj discovery priority:**
1. Explicit `mmproj` argument from user/command
2. `{model_path}.mmproj` file alongside model
3. Smart scan: sibling files in parent directory containing `"mmproj"` and ending in `.gguf`/`.bin`

**Readiness polling:** After spawning, the host polls `GET http://127.0.0.1:{port}/health` every 500ms for up to 120 seconds. A 200 OK response triggers a `SidecarEvent::Started`.

**Crash detection:** If the process exits with non-zero code and `is_chat_stop_intentional == false`, a `SidecarEvent::Crashed` is emitted to the frontend; the `chat_process` state is cleared.

**Environment:** On macOS, `DYLD_LIBRARY_PATH` is set to `$RESOURCE_DIR/bin:$CWD/backend/bin` so the bundled `.dylib` and `.metal` shader files are found at runtime. On Linux, `LD_LIBRARY_PATH` is set analogously for `.so` files.

**Progress parsing:** The monitor task listens to stdout/stderr and parses `"prompt processing progress = N"` lines, forwarding them as `SidecarEvent::Progress`.

 ### 3.2 Embedding Server Instance

**Default port:** `53756`  
**Tauri commands:** `start_embedding_server` (user-triggered), auto-started by `ingest_document`

The embedding server implementation depends on the active build feature:

#### llamacpp builds (default)

Spawns the bundled `llama-server` sidecar in embedding-only mode:

| Arg | Value | Notes |
|-----|-------|-------|
| `--embedding` | (flag) | Activates embedding-only mode |
| `--ctx-size` | `4096` | Fixed; not user-configurable |
| `--batch-size` | `512` | Optimal for batch embedding |
| `--ubatch-size` | `512` | Micro-batch size |
| `--n-gpu-layers` | `0` | CPU-only; prevents VRAM contention with chat |

**Environment:** On macOS, `DYLD_LIBRARY_PATH` is set to `$RESOURCE_DIR/bin:$CWD/backend/bin` so that shared libraries like `libmtmd.0.dylib` are found at runtime.

#### MLX builds (`--features mlx`)

Spawns the Python script `backend/scripts/mlx_embed_server.py` via the `mlx-env` virtualenv:

```
$APP_DATA/mlx-env/bin/python3 mlx_embed_server.py \
  --model <model_dir> --port 53756 --host 127.0.0.1 --api-key <token>
```

The server exposes the same OpenAI-compatible `/v1/embeddings` endpoint, so `rag.rs` is engine-agnostic.

#### Lifecycle — on-demand auto-start

The core startup logic lives in `start_embedding_server_core()` (public free function in `sidecar.rs`), called by both the Tauri command and by `ingest_document` automatically:

1. **Dimension probe:** Reads `config.json` (or `tokenizer_config.json`) from the model directory to extract `hidden_size`.
2. **Reinit:** If the probed dimension differs from the current `VectorStoreManager` dimension (e.g. switching from a 384-dim to a 768-dim model), the vector store is purged and reinitialized at the new size. `config.json:vector_dimensions` is updated.
3. **Spawn:** The appropriate server process is launched and `SidecarManager.embedding_process` is set.
4. **Health-check wait:** `ingest_document` probes `GET /health` before proceeding; `start_embedding_server_core` waits for `"Server listening"` on stdout (30 s deadline).

`ingest_document` performs a live HTTP health-check (`GET /health`) before attempting to embed. If the server is dead or never started, it calls `start_embedding_server_core` automatically — the frontend no longer needs to manually manage embedding server lifecycle for ingestion.

**Used by:** `rag.rs` — vector store ingestion (`POST /v1/embeddings`) and similarity retrieval.

### 3.3 Summarizer Server Instance

**Default port:** `53758`  
**Tauri command:** `start_summarizer_server`

Identical launch config to the chat server except:
- No `--flash-attn`, `--cont-batching`, `--cache-prompt` flags
- No mmproj handling
- Context size is user-configurable via the command arg
- `n_gpu_layers = -1` (all layers on GPU)

**Used by:** RAG pipeline in `rag.rs` for summarizing long documents before chunking and indexing.

---

## 4. Sidecar: whisper-server (Speech-to-Text)

**Binary:** `whisper-server` (from [whisper.cpp](https://github.com/ggerganov/whisper.cpp) release `v1.7.4`)  
**Platform naming:** `whisper-server-aarch64-apple-darwin`  
**Default port:** `53757`  
**Tauri command:** `start_stt_server`

**Arguments:**

| Arg | Value |
|-----|-------|
| `-m` | user-selected Whisper GGUF path |
| `--port` | `53757` (or random) |
| `--host` | `127.0.0.1` |

**Note:** whisper-server does not support `--api-key`, so no auth token is issued for it. The token field in `SidecarProcess` is generated but not sent.

**Environment (macOS):** Same `DYLD_LIBRARY_PATH` pattern as llama-server for bundled `.dylib` files.

**API used:** `POST /inference` with audio file upload (WAV format). The frontend hook `useAudioRecorder` records via `MediaRecorder`, passes the audio blob to the backend, which invokes whisper-server and returns the transcript.

**Supported models:** Any Whisper GGUF file (tiny, base, small, medium, large-v3, turbo). Downloaded via `ModelBrowser` component from the model catalogue.

---

## 5. Sidecar: sd-server (Image Generation)

**Binary:** `sd` (from [stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp) release `master-7010bb4`)  
**Platform naming:** `sd-aarch64-apple-darwin`

**Invocation model:** Unlike the other three sidecars, the SD binary is **NOT** launched as a persistent server. It is invoked as a **one-shot CLI tool** per image generation request by `image_gen.rs`. The `start_image_server` Tauri command simply stores the model path in `SidecarManager.image_model_path` — no process is actually started at that point.

**Actual invocation (in `image_gen.rs`):**

```
sd [args] \
  --mode txt2img \
  --model <model_path> \
  --output <output_path.png> \
  --prompt "<prompt>[style snippet]" \
  --negative-prompt "<quality boosters>" \
  --steps <steps> \
  --cfg-scale <guidance> \
  --width <W> \
  --height <H> \
  [--vae <vae_path>] \
  [--clip-l <clip_l_path>] \
  [--t5xxl <t5xxl_path>] \
  [--type <quant>]
```

**Key features of image_gen.rs:**
- **Architecture detection** — inspects model filename to determine SD1.x, SDXL, SD3, or FLUX architecture and applies appropriate defaults
- **Component discovery** — scans the model's parent directory for matching VAE, CLIP-L, T5XXL projector files (required for FLUX/SD3)
- **Progress events** — tails stderr, parses `"step N/N"` lines, emits `image_gen_progress` Tauri events to the frontend
- **Output management** — images are written to `$APP_DATA/images/<uuid>.png`; path is returned to frontend for display via `assetUrl`

**Tauri event `image_gen_progress`:**
```json
{ "step": 15, "total_steps": 20, "elapsed_secs": 8.3 }
```

**Tauri event `image_gen_success`:**
```json
{ "message_id": "...", "image_path": "/absolute/path/image.png" }
```

---


---

## 5a. Sidecar: piper (Text-to-Speech)

**Binary:** Piper TTS, registered as Tauri sidecar `bin/tts` (must be manually placed at `backend/bin/tts-{target-triple}`, e.g. `tts-aarch64-apple-darwin`)  
**Tauri command:** `tts_synthesize`  
**Source:** `backend/src/tts.rs`

**Invocation model:** Like the `sd` binary, the TTS sidecar is **NOT** launched as a persistent server. `tts_synthesize` invokes the binary as a **one-shot CLI process** per synthesis request using stdin→stdout piping. The Rust code calls `.sidecar("bin/tts")` to spawn the process.

**Arguments:**

| Arg | Value |
|-----|-------|
| `--model` | path to a Piper `.onnx` voice model |
| `--output-raw` | (flag) — output raw 16-bit PCM on stdout instead of WAV |

**Request/response flow:**

```
Frontend: commands.ttsSynthesize(text)
          │
          ▼
tts_synthesize (Rust, tts.rs)
          │
          ├─ Spawn: piper --model <voice_model_path> --output-raw
          ├─ Write text to stdin
          ├─ Collect stdout (raw PCM bytes)
          └─ Return: base64-encoded PCM string
                     │
                     ▼
Frontend: Web Audio API decodes base64 PCM → AudioBuffer → AudioContext.play()
```

**Frontend consumer:** The `MessageBubble` component renders a speaker-icon **"Read Aloud"** button on every assistant message. Clicking it calls `commands.ttsSynthesize`, decodes the returned base64 PCM and plays it via `AudioContext`.

**Note on auth:** Piper runs locally with no network access and requires no API key.

**Bundling note:** The `download_ai_binaries.js` script does not yet auto-download the TTS binary. It must be placed manually in `backend/bin/` with the Tauri sidecar naming convention: `tts-{target-triple}` (e.g. `tts-aarch64-apple-darwin`, `tts-x86_64-unknown-linux-gnu`). The binary must be the Piper executable renamed to match the `bin/tts` sidecar registration in `tauri.conf.json`.

## 6. IronClaw Agent Engine (In-Process)

> **Architecture change (2026-02-28):** The Node.js-based `openclaw-engine` sidecar has been replaced with **IronClaw**, a Rust library linked directly into the Tauri binary. This eliminated the Node.js runtime (~120 MB), the WebSocket bridge (~2,166 LOC), and reduced time-to-first-token from ~200ms to ~50ms. For the full migration story, see `ironclaw_library_roadmap.md` and `ironclaw_integration_roadmap.md`.

### 6.1 Purpose and Role

IronClaw is an **in-process Rust AI agent engine** that provides:

- Multi-turn agentic sessions with tool execution and approval flow
- Session persistence and memory (SOUL.md, MEMORY.md, workspace files)
- Cron job scheduling (automated routines)
- Skill system (installable agent capabilities)
- Extension manager (auth-gated integrations)
- Canvas output (HTML/JSON structured results)
- Multi-provider LLM abstraction with safety guardrails

The engine runs **inside the Tauri process** — no child process, no WebSocket, no serialization overhead.

### 6.2 Integration Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                       Tauri Host (Rust)                          │
│                                                                   │
│  IronClawState (managed state)                                    │
│  ├─ inner: RwLock<Option<IronClawInner>>                         │
│  │         ├─ agent: Arc<Agent>                                   │
│  │         ├─ inject_tx: mpsc::Sender<IncomingMessage>           │
│  │         ├─ bg_handle: BackgroundTasksHandle                    │
│  │         └─ log_broadcaster: Arc<LogBroadcaster>               │
│  ├─ app_handle: AppHandle<Wry>                                   │
│  └─ state_dir: PathBuf                                           │
│                                                                   │
│  TauriChannel (impl ironclaw::Channel)                           │
│  ├─ respond()     → emit UiEvent::AssistantFinal                 │
│  ├─ send_status() → emit UiEvent::{Delta,ToolUpdate,Error,...}   │
│  └─ broadcast()   → emit UiEvent::AssistantFinal (system)        │
│                                                                   │
│  KeychainSecretsAdapter (impl ironclaw::SecretsStore)            │
│  └─ maps ironclaw secret names → Scrappy keychain slugs          │
│                                                                   │
│  66 openclaw_* Tauri commands                                     │
│  └─ call ironclaw::api::* directly (no WS, no serialization)     │
└─────────────────────────────────────────────────────────────────┘
        │
        │  emit("openclaw-event", UiEvent)
        ▼
┌─────────────────────────────────┐
│  Frontend (React WebView)       │
│  listen("openclaw-event")       │
│  → OpenClawChatView.tsx         │
└─────────────────────────────────┘
```

### 6.3 Startup Flow

```
Tauri setup() hook
    │
    ├─ 1. Register IronClawState::new_stopped(app_handle, state_dir)
    │      as Tauri managed state
    │
    ├─ 2. If auto_start enabled:
    │      IronClawState::start(secrets_store)
    │      │
    │      ├─ Load Config from $STATE_DIR/ironclaw.toml
    │      ├─ Create TauriChannel + mpsc::Sender
    │      ├─ AppBuilder::build_all() → DB, secrets, LLM, tools, extensions
    │      ├─ Agent::new(deps, channel_manager, ...)
    │      ├─ agent.start_background_tasks()
    │      └─ emit UiEvent::Connected { protocol: 2 }
    │
    └─ 3. On app exit:
           IronClawState::shutdown()
           ├─ agent.shutdown_background(bg_handle)
           └─ channels.shutdown_all()
```

### 6.4 IronClaw API Surface

The `ironclaw::api` module (1,320 LOC, 8 submodules) provides framework-agnostic functions:

| Module | Key Functions |
|--------|-------------|
| `api::chat` | `send_message()`, `resolve_approval()`, `abort()` |
| `api::sessions` | `list_sessions()`, `get_history()`, `create_session()`, `delete_session()`, `clear_session()` |
| `api::memory` | `get_file()`, `write_file()`, `list_files()`, `clear()`, `search()` |
| `api::config` | `get_config()`, `set_config()` |
| `api::skills` | `list_skills()`, `toggle_skill()`, `install_skill()` |
| `api::routines` | `list_routines()`, `trigger_routine()` |
| `api::system` | `get_status()`, `health_check()`, `list_models()`, `tail_logs()`, `diagnostics()` |
| `api::extensions` | `list_extensions()`, `toggle_extension()` |

### 6.5 StatusUpdate → UiEvent Conversion

`ironclaw_types.rs` maps IronClaw's `StatusUpdate` enum to Scrappy's `UiEvent`:

| StatusUpdate Variant | UiEvent Variant |
|---------------------|----------------|
| `Thinking(_)` | `RunStatus { status: "in_flight" }` |
| `StreamChunk(delta)` | `AssistantDelta { delta }` (sanitized) |
| `ToolStarted { name }` | `ToolUpdate { status: "started" }` |
| `ToolCompleted { name, success }` | `ToolUpdate { status: "ok"/"error" }` |
| `ToolResult { name, preview }` | `ToolUpdate { status: "stream", output }` |
| `Status(text)` | `RunStatus { status: text }` |
| `ApprovalNeeded { ... }` | `ApprovalRequested { ... }` |
| `AuthRequired { ... }` | `WebLogin { status: auth_url }` |
| `AuthCompleted { ... }` | `WebLogin { status: "authenticated"/"failed" }` |
| `JobStarted { ... }` | `CanvasUpdate { content_type: "json" }` |
| `Error { message, code }` | `Error { code, message }` |

### 6.6 Background Tasks

`BackgroundTasksHandle` manages long-running agent tasks:

- **Self-repair:** Periodic health checks and recovery
- **Session pruning:** Cleanup of stale/expired sessions
- **Heartbeat:** Keep-alive for connected channels
- **Routine engine:** Cron job scheduler (`RoutineEngine::fire_manual()`)

All tasks listen to a `watch::Sender<bool>` for graceful shutdown.

### 6.7 Remote Deployment (Retained)

The `deploy-remote.sh` script and `Dockerfile` are retained in `openclaw-engine/` for deploying IronClaw to remote Linux servers via Ansible + Docker. The desktop app can connect to remote instances via Tailscale.

---


## 7. scrappy-mcp-tools Crate

**Location:** `backend/scrappy-mcp-tools/`  
**Type:** Separate Rust workspace crate (not a Tauri plugin)  
**Purpose:** MCP (Model Context Protocol) client, sandboxed Rhai script execution, tool discovery, and skill management for agent tool integration.

### 7.1 Rhai Sandbox

**File:** `src/sandbox.rs`  
**Engine:** [Rhai](https://rhai.rs) scripting language, sync mode

The sandbox provides **safe, resource-constrained execution** of LLM-generated scripts that invoke tool functions:

```rust
pub struct SandboxConfig {
    pub max_operations:    u64,     // Default: 100,000 (prevents infinite loops)
    pub timeout_seconds:   u64,     // Default: 30s (belt-and-suspenders timeout)
    pub max_result_size:   usize,   // Default: 1 MB
}
```

**Forbidden patterns** (static check before execution):
- `std::fs` — no filesystem access
- `std::net` — no network access
- `std::process` — no process spawning
- `unsafe` — no unsafe Rust
- `extern` — no external bindings

**Execution flow:**
1. `validate_script(script)` — static forbidden-pattern check
2. `engine.eval_with_scope(script)` — execute in isolated Rhai scope
3. Timeout check (wall clock, fallback to op-count limit)
4. Result serialization (string, unit, or Rhai map → JSON)
5. Size check

**Built-in sandbox functions registered:**
- `json_stringify(map)` → JSON string
- `parse_json(str)` → Rhai Dynamic
- `timestamp_now()` → RFC3339 string

**Error to LLM feedback:** Every `SandboxError` implements `to_llm_feedback()` returning a human-readable string that the orchestrator injects back into the agent's context, enabling self-correction.

**Tool registration:** External tools are registered on `sandbox.engine_mut()` using Rhai's `register_fn` API. The orchestrator in `rig_lib/orchestrator.rs` registers all available MCP tools as Rhai functions before executing agent-generated scripts.

### 7.2 MCP HTTP Client

**File:** `src/client.rs`

```rust
pub struct McpConfig {
    pub base_url:   String,   // e.g. "https://api.scrappy.dev"
    pub auth_token: String,   // JWT bearer token
    pub timeout_ms: u64,      // Default: 30,000
}
```

The `McpClient` wraps `reqwest::Client` with:
- `Authorization: Bearer <token>` header on all requests
- Typed `call_tool<T>(tool: &str, arguments: Value) -> McpResult<T>` convenience method
- Raw `call_tool_raw()` for untyped callers

**Request format:** `POST {base_url}/tools/call` with body:
```json
{ "tool": "tool_name", "arguments": { ...params } }
```

**Response format:**
```json
{ "result": <T>, "error": "string | null" }
```

### 7.3 Tool Discovery & Registry Cache

**File:** `src/discovery.rs`

Wraps the `search_tools` MCP tool to progressively discover what tools are available:

| `DetailLevel` | Request | Response |
|---------------|---------|---------|
| `Categories` | `""` query | Map of category → `{ description, tool_count }` |
| `Names` | specific query | List of `{ name, description }` |
| `Full` | specific query | List with full `inputSchema` |

**`ToolRegistryCache`** — in-memory cache with configurable TTL:
- `get_categories(client)` — fetches and caches category list
- `get_tool(client, name)` — fetches individual tool schema if not cached

The cache avoids redundant network round-trips to the MCP server during agent execution.

### 7.4 Skill System

**Files:** `src/skills/manifest.rs`, `src/skills/manager.rs`

Skills are reusable, parameterized Rhai scripts that compose MCP tools into higher-level capabilities.

**Skill structure on disk:**
```
skills/
  built_in/           ← shipped with app (read-only)
    weather.skill.toml
    weather.rhai
  <user_skills_dir>/  ← $APP_DATA/skills/ (writable)
    custom.skill.toml
    custom.rhai
```

**`SkillManifest` (TOML format):**
```toml
name        = "Get Weather"
description = "Fetch current weather for a location"
version     = "1.0.0"
author      = "Scrappy"
tools_used  = ["weather.current"]
script_file = "weather.rhai"

[[parameters]]
name        = "location"
description = "City name or coordinates"
param_type  = "string"
required    = true
```

**`SkillManager`** operations:
- `list_skills()` — aggregates built-in + user skills; user overrides built-ins by ID
- `get_skill(id)` — retrieve a specific skill (searches both paths)
- `prepare_script(id, params)` — validates required params and prepends a `const` block:
  ```
  // -- Injected Parameters --
  const location = "Berlin";

  // -- Skill Script --
  let result = weather_current(location);
  result
  ```
- `save_skill(id, manifest, script)` — writes `.skill.toml` + `.rhai` to user directory

**Built-in skills directory:** `scrappy-mcp-tools/skills/built_in/` (10 items in tree).

### 7.5 Built-in Tool Modules

**Directory:** `src/tools/`

Typed wrappers around specific MCP tool categories:

| Module | File | Tools Covered |
|--------|------|--------------|
| `ai_tools` | `ai_tools.rs` | AI/ML model utility tools |
| `economics` | `economics.rs` | Economic indicators, GDP, CPI |
| `finance` | `finance.rs` | Stock quotes, market data, FX rates |
| `health` | `health.rs` | Health and medical data tools |
| `knowledge` | `knowledge.rs` | Knowledge base search, wiki |
| `models` | `models.rs` | AI model catalogue queries |
| `news` | `news.rs` | News headlines, search |
| `politics` | `politics.rs` | Political data, voting records |

Each module provides strongly-typed Rust structs for request/response and calls `McpClient.call_tool<T>()`.

---

## 8. ProcessTracker — Zombie Prevention

**File:** `backend/src/process_tracker.rs`  
**Rust type:** `ProcessTracker` (Tauri global state, `Arc<Mutex<HashMap<u32, ProcessEntry>>>`)

Every child process spawned by `SidecarManager` is registered in `ProcessTracker` with:
- `pid: u32` — OS process ID
- `service_name: &str` — binary name (e.g. `"llama-server"`)
- `service_role: &str` — role (e.g. `"chat"`, `"embedding"`)

**Key operations:**
- `add_pid(pid, service_name, service_role)` — register
- `remove_pid(pid)` — called by monitor task on `CommandEvent::Terminated`
- `cleanup_by_service(role)` — kills all tracked PIDs matching a role before restarting that service (prevents duplicates if previous monitor task didn't clean up)
- `kill_all()` — called on app shutdown via `on_window_event`

This prevents "zombie" llama-server processes from accumulating after crashes or rapid restarts (a common pain point in desktop AI apps).

---

## 10. Build-time Infrastructure Scripts

### 10.1 download_ai_binaries.js

**File:** `backend/scripts/download_ai_binaries.js`  
**Run:** `node download_ai_binaries.js` (typically in a `preinstall` or `prepare` npm script)  
**Purpose:** Download and install the three ML inference binaries for the current platform

**Binaries managed:**

| Binary | Source | Platforms |
|--------|--------|--------|
| `llama-server-{triple}` | `llama.cpp` release `b4618` | macOS ARM64, Linux x64, Windows x64 |
| `whisper-server-{triple}` | `whisper.cpp` release `v1.7.4` | macOS ARM64, Linux x64, Windows x64 |
| `sd-{triple}` | `stable-diffusion.cpp` release `master-7010bb4` | macOS ARM64, Linux x64 |

> `{triple}` = Rust target triple, e.g. `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`

**Also copies:**
- macOS: `*.dylib` (shared libraries), `*.metal` (Metal shader kernels)
- Linux: `*.so` (shared libraries, CUDA/CPU)
- `whisper-cli` — Whisper CLI binary (renamed to `whisper-{triple}`)

**Idempotent:** Skips any binary that already exists in `backend/bin/`.

**Error handling:** Non-fatal on any individual binary failure — logs a warning and prints the download URL for manual installation.

### 10.2 download_node.js

**File:** `backend/scripts/download_node.js`  
**Purpose:** Download Node.js `v24.13.0` binaries for all target platforms

| Platform binary | Source |
|----------------|--------|
| `node-aarch64-apple-darwin` | nodejs.org `darwin-arm64.tar.gz` |
| `node-x86_64-apple-darwin` | nodejs.org `darwin-x64.tar.gz` |
| `node-x86_64-pc-windows-msvc.exe` | nodejs.org `win-x64/node.exe` |
| `node-x86_64-unknown-linux-gnu` | nodejs.org `linux-x64.tar.xz` |

The downloaded Node.js binary is registered as a Tauri sidecar binary in `tauri.conf.json` and used to run the `openclaw-engine` wrapper script. This ensures users never need Node.js installed separately.

### 10.3 setup_uv.sh

**File:** `scripts/setup_uv.sh`  
**Purpose:** Download the `uv` Python package manager binary as a Tauri sidecar (for MLX/vLLM builds)

| Platform binary | Source |
|----------------|--------|
| `uv-aarch64-apple-darwin` | `astral.sh/uv` GitHub Releases |
| `uv-x86_64-unknown-linux-gnu` | `astral.sh/uv` GitHub Releases |

**Only needed for `--features mlx` or `--features vllm` builds.** The `uv` binary is used at first launch to bootstrap a Python virtual environment containing the inference framework.

### 10.4 setup_llama.sh

**File:** `scripts/setup_llama.sh`  
**Purpose:** Cross-platform download of the `llama-server` binary from GitHub Releases

Detects the current OS and architecture, downloads the correct release ZIP from `ggerganov/llama.cpp`, extracts the binary and shared libraries, and places them in `backend/bin/`.

| Platform | Asset suffix | Target binary |
|----------|-------------|---------------|
| macOS ARM64 | `bin-macos-arm64` | `llama-server-aarch64-apple-darwin` |
| macOS x86_64 | `bin-macos-x64` | `llama-server-x86_64-apple-darwin` |
| Linux x86_64 | `bin-ubuntu-x64` | `llama-server-x86_64-unknown-linux-gnu` |
| Windows (Git Bash/WSL) | `bin-win-avx2-x64` | `llama-server-x86_64-pc-windows-msvc.exe` |

**Usage:** `bash scripts/setup_llama.sh [release_tag]` — defaults to `b4406` if no tag specified.

**Post-download (macOS):** Fixes `@rpath` references in `libllama.dylib` via `install_name_tool` so the dynamic library is found alongside the binary.

### 10.5 generate_tauri_overrides.sh

**File:** `scripts/generate_tauri_overrides.sh`  
**Purpose:** Generate `tauri.override.json` for engine-specific builds

This script modifies Tauri's configuration overrides to include the correct `externalBin` list and product name suffix for each engine variant (e.g. `Scrappy MLX`, `Scrappy vLLM`). Called by CI before `tauri build`.

### 10.6 setup_chromium.sh

**File:** `backend/scripts/setup_chromium.sh`  
**Purpose:** Download a specific Chromium snapshot for browser automation / web scraping

**Configuration:**
- Revision: `1313161`
- Target: `backend/resources/chromium/`
- Source: `https://storage.googleapis.com/chromium-browser-snapshots/Mac_Arm/{revision}/chrome-mac.zip` (macOS) or `Linux_x64/{revision}/chrome-linux.zip` (Linux)

**Post-download (macOS):** Runs `xattr -cr` on `Chromium.app` to remove macOS quarantine attributes.

**Usage:** Used headlessly for JavaScript-rendered web page scraping via the `rig_lib` web search pipeline. The Chromium binary path is passed via environment variable or config at runtime.

---

## 11. Port & Token Reference

| Service | Default Port | Preferred Port | Auth | Managed By |
|---------|------------|----------------|------|-----------|
| llama-server (chat) | `53755` | Fixed | 32-char random token | `SidecarManager` |
| llama-server (embedding) | `53756` | Fixed | 32-char random token | `SidecarManager` |
| whisper-server (STT) | `53757` | Fixed | None | `SidecarManager` |
| llama-server (summarizer) | `53758` | Fixed | 32-char random token | `SidecarManager` |
| openclaw-engine gateway | `18789` | Fixed | User-configured token | `OpenClawManager` |

All ports are tried first as preferred; if occupied (e.g. another Scrappy instance) the OS assigns a random ephemeral port, which is stored in `SidecarProcess.port` and communicated to the engine config.

All tokens are generated at process start (not stored persistently) and re-generated on each restart. The llama-server token is passed to the OpenClaw engine config so both sides have matching credentials.

---

## 12. Tauri Sidecar Registration

`backend/tauri.conf.json` — `bundle.externalBin` list:

```json
"externalBin": [
    "bin/llama-server",
    "bin/whisper-server",
    "bin/whisper",
    "bin/sd",
    "bin/node",
    "bin/piper",
    "bin/openclaw-engine-wrapper"
]
```

> **Engine-specific builds** may also include `bin/uv` (for MLX/vLLM Python bootstrapping). The `generate_tauri_overrides.sh` script adjusts this list per engine.

Tauri automatically adds the platform triple suffix at bundle time (e.g. `llama-server-aarch64-apple-darwin`) matching the binaries in `backend/bin/`. The `sidecar()` / `command()` shell calls in Rust use the short name; Tauri resolves to the full path.

---

## 13. Event Flow Diagram

```
[User types prompt in ChatInput]
         │
         ▼
[Rust: chat.rs chatStream command]
         │
         ├─ (RAG enabled?) → llamaServer /v1/embeddings → similarity search
         │
         ├─ (web search?) → rig_lib web search pipeline → Chromium/scraping → reranker
         │
         ├─ (local model) → llamaServer /v1/chat/completions (SSE stream)
         │       Tokens emitted via Tauri Channel<StreamChunk>
         │
         └─ (cloud model) → rig_lib agent via Anthropic/OpenAI API
                  tokens → Tauri Channel<StreamChunk>

[User triggers OpenClaw agent task]
         │
         ▼
[Rust: openclaw/commands → OpenClawWsHandle.chat_send()]
         │
         ▼
[WS JSON frame → openclaw-engine Node.js process]
         │
         ├─ engine processes turn, calls tools (fs, web, messaging, etc.)
         │
         ├─ engine → WS events (StreamChunk, ToolCall, ApprovalRequest, ...)
         │
         ▼
[ws_client.rs receives frames → normalizer → UiEvent]
         │
         ▼
[Rust: app.emit("openclaw-event", ui_event)]
         │
         ▼
[Frontend: OpenClawChatView useOpenClawStream hook → UI update]

[Image generation request]
         │
         ▼
[Rust: imagine.rs → image_gen.rs → spawn sd binary (one-shot)]
         │
         ├─ Progress: stderr "step N/N" → emit("image_gen_progress", ...)
         └─ Done: emit("image_gen_success", { message_id, image_path })
```

---

## 14. Replacing or Extending Services

### Adding a new inference engine
1. Add a new file `backend/src/engine/engine_myengine.rs`
2. Implement `InferenceEngine` for your struct (must expose OpenAI-compatible HTTP API)
3. Add a feature flag in `backend/Cargo.toml`: `myengine = []`
4. Add the conditional compilation block in `engine/mod.rs` (`create_engine()`, `get_active_engine_info()`)
5. Update `hf_hub.rs` `engine_to_hf_tag()` to map your engine to an HF `filter=` tag
6. Update `frontend/src/components/settings/ActiveEngineChip.tsx` `ENGINE_STYLES` with your engine's colour

### Replacing llama-server
Any OpenAI-compatible HTTP server (`/v1/chat/completions`, `/v1/embeddings`) can substitute for llama-server. Change `SidecarManager.start_chat_server` to spawn your binary instead, preserving the same port/token pattern. The rest of the stack (chat.rs, rag.rs) depends only on the HTTP API format.

### Replacing whisper-server
Any server accepting audio POST requests and returning a JSON transcript can substitute. Update `use-audio-recorder.ts` frontend hook and the backend invocation in `chat.rs` to match the new API.

### Replacing openclaw-engine
The Rust WS client (`ws_client.rs`) and normalizer (`normalizer.rs`) would need to be adapted for a different gateway protocol. All OpenClaw-specific RPC methods are isolated in `OpenClawWsHandle`. The frontend `OpenClawChatView` consumes only `UiEvent` variants — only the normalizer layer needs updating.

### Adding a new MCP tool module
1. Add a new file in `scrappy-mcp-tools/src/tools/`
2. Define typed request/response structs
3. Call `McpClient.call_tool::<ResponseType>(tool_name, args)`
4. Register the module in `tools/mod.rs`
5. Register the Rust function in the Rhai sandbox (via `sandbox.engine_mut().register_fn(...)`) in the orchestrator

### Adding a new skill
1. Create `<skill_id>.skill.toml` (see manifest schema in §7.4)
2. Create matching `<skill_id>.rhai` script using available tool functions
3. Place in `skills/built_in/` (shipped) or `$APP_DATA/skills/` (user)
4. No code changes required — `SkillManager.list_skills()` discovers automatically

---

## 8. External MCP Server Integration

### 8.1 What it is

The application supports connecting to an **external FastAPI MCP (Model Context Protocol) server** — a remote HTTP service that exposes a catalogue of tools the AI agent can call. This is the "cloud brain" extension point for the sandbox: when configured, the agent gains access to an arbitrary set of remote tools (finance data, news, health search, knowledge bases, etc.) without shipping those data sources locally.

**The external MCP server is optional.** When `mcp_base_url` is `None` (the default), the system operates with host tools and skills only. The entire integration is additive — nothing breaks if the MCP server is not reachable.

**Current status:** `mcp_base_url` and `mcp_auth_token` are stored in `UserConfig` and loaded from live `ConfigManager` in `get_mcp_config()` in `ipc.rs`. The `GatewayTab` settings page exposes UI fields that write these values to `UserConfig`. No code changes are needed to point at a custom MCP server — configure the URL and token in the Gateway settings tab.

---

### 8.2 McpRequestHandler — Reverse-RPC Entry Point

**File:** `backend/src/openclaw/ipc.rs`

When the `openclaw-engine` Node.js gateway needs a tool executed on the Rust host, it sends a WebSocket `Req` frame with `method` starting with `"mcp."`. The `ws_client.rs` loop detects this prefix and dispatches to `McpRequestHandler`:

```rust
pub struct McpRequestHandler {
    app: tauri::AppHandle,
}

pub async fn handle_request(&self, method: &str, params: Value) -> Result<Value, String>
```

**Handled methods:**

| Method | Handler | What it does |
|--------|---------|-------------|
| `mcp.list_tools` | → `search_tools({query:""})` | Returns all available tools (host + skills + remote) |
| `mcp.search_tools` | → `search_tools(params)` | Filtered tool search |
| `mcp.call_tool` | → `call_tool(params)` | Execute a specific tool by name + args |
| `mcp.list_skills` | → `list_skills()` | Returns all available skills |
| `mcp.run_skill` | → `run_skill(params)` | Execute a named skill with args |
| `mcp.save_skill` | → `save_skill(params)` | Persist a new skill to disk |

This is the **bridge between the openclaw-engine gateway and the Rust tool ecosystem** — the Node.js agent doesn't execute tools directly; it asks the Rust host to execute them via this channel.

**`McpOrchestratorConfig`** (resolved per-request in `get_mcp_config()`):

```rust
pub struct McpOrchestratorConfig {
    pub mcp_base_url:         Option<String>,    // None = no remote MCP
    pub mcp_auth_token:       Option<String>,    // JWT bearer for remote MCP
    pub sandbox_enabled:      bool,              // Always true in ipc.rs
    pub user_skills_path:     Option<PathBuf>,   // $APP_CONFIG_DIR/skills/
    pub builtin_skills_path:  Option<PathBuf>,   // $RESOURCE_DIR/scrappy-mcp-tools/skills/built_in
}
```

**Response format** (MCP content envelope):
```json
{
  "content": [{ "type": "text", "text": "<result>" }],
  "isError": false
}
```
All tool outputs are wrapped in this envelope before being returned to the gateway, matching the [MCP spec](https://modelcontextprotocol.io/spec).

**Auto-summarization middleware:** After any tool call resolves, `summarize_result(result, 5000)` truncates any `content[].text` exceeding 5,000 characters with `"... [Truncated N chars]"`. This prevents context window overflow when tools return large results (e.g. long articles, API dumps).

---

### 8.3 ToolRouter — Three-Tier Dispatch

**File:** `backend/src/rig_lib/tool_router.rs`

The `ToolRouter` is called for every `mcp.call_tool` request. It routes to the correct backend in strict priority order:

```
Tool name arrives
       │
       ▼
Tier 1: Is it a known Skill? (SkillManager.get_skill)
       ├── YES → Run via Sandbox: run_skill("<name>", args_json)
       │
       ▼
Tier 2: Is it a Host Tool? (registry-driven — ToolRouter::host_tool_names())
       ├── Derived from tool_discovery::get_host_tools_definitions()
       ├── Currently: { "web_search", "rag_search", "read_file" }
       ├── Adding a new host tool in tool_discovery auto-routes it here
       │
       ▼
Tier 3: Remote MCP (if McpClient configured)
       └── McpClient.call_tool_raw(name, args)
```

> **Architecture note (2026-02-22):** Tier 2 was previously a hardcoded `match` block. It is now **registry-driven**: `ToolRouter::host_tool_names()` derives the set of host tool names from `tool_discovery::get_host_tools_definitions()`. Adding a new host tool in one place auto-routes it everywhere — no changes to `ToolRouter` are required.

**Sandbox routing for host tools:** Even host tools are routed *through* the sandbox — the sandbox has `web_search`, `rag_search`, and `read_file` functions registered as Rhai built-ins that invoke the backing Rust implementations. This gives security isolation and consistent result serialization.

**Error propagation:** Each tier returns `Err(String)`. If all tiers are exhausted (tool not found, no MCP client), the error `"Tool 'X' not found or no router configured for it"` is returned to the gateway, which forwards it to the agent so it can self-correct.

---

### 8.4 sandbox_factory — Sandbox Wiring

**File:** `backend/src/rig_lib/sandbox_factory.rs`

`create_sandbox(rig, mcp_config, reporter) -> Option<Sandbox>` is the factory that wires all tool functions into the Rhai engine. It is called in two contexts:
- By the **Orchestrator** (`rig_lib/orchestrator.rs`) for Rig agent turns
- By the **McpRequestHandler** (`openclaw/ipc.rs`) for gateway tool calls

**Functions registered unconditionally** (always available):

| Rhai fn | Implementation | Notes |
|---------|---------------|-------|
| `web_search(query)` | `RigManager.explicit_search()` | DuckDuckGo + scraping pipeline |
| `rag_search(query)` | `rag::retrieve_context_internal()` | Vector similarity search in SQLite |
| `read_file(path)` | `std::fs::read_to_string()` | Read-only; truncated at 20,000 chars |
| `list_skills()` | `SkillManager.list_skills()` | Returns JSON array of available skills |
| `run_skill(id, args_json)` | `SkillManager.prepare_script()` + inline `eval` | Executes skill in a fresh Rhai scope |
| `search_tools(query)` | `tool_discovery::search_all_tools()` | Returns JSON of all matching tools |
| `save_skill(id, script, desc)` | `SkillManager.save_skill()` | Persists a new user skill to disk |

**Async bridging:** Rhai is synchronous. All async operations (web_search, rag_search, etc.) use `tokio::task::block_in_place` + `Handle::current().block_on(...)` to synchronously await the async result within the Rhai engine execution context.

**Functions registered only when `mcp_base_url` is set** (remote MCP tools):

| Rhai fn | Implementation |
|---------|---------------|
| `mcp_call(tool_name, args_json)` | Generic raw call to any remote MCP tool |
| `finance::get_stock_price(symbol)` | Typed via `tools::finance` module |
| `news::get_news(category, limit)` | Typed via `tools::news` module |
| `news::search_news(query)` | Typed via `tools::news` module |
| `news::get_headlines(country, limit)` | Typed via `tools::news` module |
| `knowledge::rag_query(query)` | Typed via `tools::knowledge` module |
| `economics::get_economic_data(country)` | Typed via `tools::economics` module |
| `models::get_model_catalog()` | Typed via `tools::models` module |
| `health::search_medical_research(q, n)` | Typed via `tools::health` module |
| `ai_tools::summarize_text(text, len)` | Typed via `tools::ai_tools` module |

---

### 8.5 tool_discovery — Unified Tool Search

**File:** `backend/src/rig_lib/tool_discovery.rs`

`search_all_tools(query, mcp_client, skill_manager, include_host) -> SearchResult` aggregates tools from all three tiers into a single list for the agent to browse:

```
1. Host tools   → get_host_tools_definitions() (static list of 3 tools)
2. Skills       → SkillManager.list_skills() → filtered by query
3. Remote MCP   → McpClient → search_tools (DetailLevel::Full)
```

Each skill is converted to a standard `ToolInfo` with a JSON Schema `inputSchema` (derived from `SkillManifest.parameters`). Skill names are prefixed with `[Skill]` in the description to distinguish them from host tools.

The result is returned to the gateway as a JSON serialized `SearchResult`, allowing the engine to present the agent with a full catalogue of what it can do.

---

### 8.6 Host Tools Reference

The three host tools that are always available in every sandbox:

| Tool | Rhai call | Description | Key limit |
|------|-----------|-------------|-----------|
| `web_search` | `web_search("query")` | Full web search via DuckDuckGo + Chromium scraping pipeline | Rate-limited by network |
| `rag_search` | `rag_search("query")` | Semantic search across locally ingested documents | Requires embedding server running |
| `read_file` | `read_file("/absolute/path")` | Read a local file's text content | Read-only; 20,000 char limit |

`web_search` invokes `RigManager.explicit_search()` which orchestrates the full RAG/web pipeline in `rig_lib` — DuckDuckGo search → Chromium page fetching → `clawscan` extraction → reranker.

`rag_search` calls `rag::retrieve_context_internal()` with the user's conversation ID (for scoping results to the current chat) and the full vector store + reranker stack.

---

### 8.7 Remote MCP Tool Bindings in the Sandbox

When `mcp_base_url` is configured, the sandbox registers both a **generic escape hatch** and **typed domain-specific functions**:

**Generic escape hatch:**
```rhai
// Call any remote tool by name
let result = mcp_call("custom_tool", `{"param1":"value"}`);
```

**Typed domain bindings (safer, structured return values):**
```rhai
// Finance
let price = finance::get_stock_price("AAPL");

// News
let news = news::get_news("technology", 5);
let headlines = news::get_headlines("US", 10);
let articles = news::search_news("AI regulation");

// Knowledge
let docs = knowledge::rag_query("quantum computing");

// Economics
let data = economics::get_economic_data("Germany");

// Model catalog
let models = models::get_model_catalog();

// Health
let research = health::search_medical_research("cancer immunotherapy", 10);

// AI Tools
let summary = ai_tools::summarize_text("long text here...", "short");
```

All typed bindings use the `tools::*` modules from `scrappy-mcp-tools/src/tools/`, which provide strongly-typed Rust request/response structs and automatically handle serialization.

---

### 8.8 Activation / Configuration

`mcp_base_url` and `mcp_auth_token` are standard `UserConfig` fields read from live `ConfigManager` in `get_mcp_config()`. The **Gateway settings tab** (`GatewayTab.tsx`) exposes input fields that write directly to these config keys.

**To activate the external MCP server:**

1. Open Settings → Gateway tab
2. Enter the MCP server URL (e.g. `https://api.example.com`) in the "MCP Base URL" field
3. Enter the auth token in the "MCP Auth Token" field
4. Save — the next tool call will use the configured server

**Expected MCP server API:**

```
POST /tools/call
Authorization: Bearer <token>
Content-Type: application/json

{ "tool": "<tool_name>", "arguments": { ...params } }

→ { "result": <any>, "error": null | "string" }
```

```
POST /tools/call (with search_tools)
{ "tool": "search_tools", "arguments": { "query": "finance", "detail": "full" } }

→ {
    "result": {
      "tools": [
        { "name": "get_stock_price", "description": "...", "inputSchema": {...} }
      ]
    }
  }
```

The external server must implement at minimum the `search_tools` tool (for discovery) and whatever domain tools you want available in the sandbox.

