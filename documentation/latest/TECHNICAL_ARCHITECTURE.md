# Scrappy — Technical Architecture Reference

> **Last updated:** 2026-02-23  
> **Version:** 0.1.0  
> **Stack:** Tauri v2 · Rust 2021 edition · React 19 · TypeScript 5.8 · Vite 7

---

## Table of Contents

1. [High-Level Overview](#1-high-level-overview)
2. [Repository Layout](#2-repository-layout)
3. [Frontend (React / TypeScript)](#3-frontend-react--typescript)
   - 3.1 [Entry Point & Window Model](#31-entry-point--window-model)
   - 3.2 [Component Tree](#32-component-tree)
   - 3.3 [Context Providers](#33-context-providers)
   - 3.4 [Key Hooks](#34-key-hooks)
   - 3.5 [Library Modules (`src/lib/`)](#35-library-modules-srclib)
4. [Backend (Rust / Tauri)](#4-backend-rust--tauri)
   - 4.1 [Tauri Entry Point & Application Lifecycle](#41-tauri-entry-point--application-lifecycle)
   - 4.2 [Managed State Registry](#42-managed-state-registry)
   - 4.3 [Core Modules](#43-core-modules)
   - 4.4 [Chat Pipeline (`chat.rs`)](#44-chat-pipeline-chatrs)
   - 4.5 [Sidecar Manager (`sidecar.rs`)](#45-sidecar-manager-sidecarrs)
   - 4.6 [Configuration (`config.rs`)](#46-configuration-configrs)
5. [OpenClaw Integration (`src-tauri/src/openclaw/`)](#5-openclaw-integration)
   - 5.1 [Module Structure](#51-module-structure)
   - 5.2 [WebSocket Client (`ws_client.rs`)](#52-websocket-client-ws_clientrs)
   - 5.3 [Commands (`commands/`)](#53-commands)
   - 5.4 [Config (`config/`)](#54-config)
   - 5.5 [IPC & Event Normalizer](#55-ipc--event-normalizer)
   - 5.6 [Fleet & Remote Deploy](#56-fleet--remote-deploy)
6. [Native Rig Agent (`src-tauri/src/rig_lib/`)](#6-native-rig-agent)
   - 6.1 [UnifiedProvider](#61-unifiedprovider)
   - 6.2 [RigManager / Agent](#62-rigmanager--agent)
   - 6.3 [Orchestrator](#63-orchestrator)
   - 6.4 [Rig Tools](#64-rig-tools)
   - 6.5 [LlamaProvider](#65-llamaprovider)
   - 6.6 [Sandbox Factory & MCP Tools](#66-sandbox-factory--mcp-tools)
7. [Knowledge OS — RAG Pipeline](#7-knowledge-os--rag-pipeline)
   - 7.1 [Document Ingestion](#71-document-ingestion)
   - 7.2 [Vector Store (`vector_store.rs`)](#72-vector-store)
   - 7.3 [Reranker (`reranker.rs`)](#73-reranker)
   - 7.4 [Retrieval & Context Injection](#74-retrieval--context-injection)
8. [Imagine Studio — Image Generation](#8-imagine-studio--image-generation)
   - 8.1 [`image_gen.rs` — sd.cpp Engine](#81-image_genrs--sdcpp-engine)
   - 8.2 [`imagine.rs` — Imagine Mode Commands](#82-imaginers--imagine-mode-commands)
   - 8.3 [Frontend Components](#83-frontend-components)
9. [Model Management](#9-model-management)
   - 9.1 [Local GGUF Models](#91-local-gguf-models)
   - 9.2 [GGUF Metadata Parsing (`gguf.rs`)](#92-gguf-metadata-parsing-ggufrs)
   - 9.3 [Remote Model Catalog](#93-remote-model-catalog)
   - 9.4 [Model Family & Stop-Token Handling](#94-model-family--stop-token-handling)
10. [Persistence Layer](#10-persistence-layer)
    - 10.1 [SQLite Schema & Migrations](#101-sqlite-schema--migrations)
    - 10.2 [Vector Index Files](#102-vector-index-files)
    - 10.3 [Flat-File Configuration](#103-flat-file-configuration)
11. [Process Lifecycle & Sidecar Binaries](#11-process-lifecycle--sidecar-binaries)
    - 11.1 [ProcessTracker](#111-processtracker)
    - 11.2 [Bundled Binaries](#112-bundled-binaries)
12. [IPC Contract (Tauri Commands)](#12-ipc-contract-tauri-commands)
13. [Security Model](#13-security-model)
14. [Build System & Toolchain](#14-build-system--toolchain)
15. [Known Technical Debt](#15-known-technical-debt)

---

## 1. High-Level Overview

Scrappy is a **desktop AI command centre** built on Tauri v2. It orchestrates a family of first-class and third-party AI engines from a single, glassmorphic React shell.

```
┌──────────────────── Tauri Window ────────────────────────────────┐
│                                                                    │
│  React 19 Frontend  ←──── IPC / Events ────►  Rust Core (Tauri)  │
│                                                    │               │
│                                          ┌─────────┼─────────┐    │
│                                          │         │         │    │
│                                     OpenClaw  Rig Agent  Sidecars │
│                                     (Node.js)  (rig-core) (llama, │
│                                          │          │     sd, STT)│
│                                          └─────────┴─────────┘    │
│                                                    │               │
│                                          ┌─────────┴─────────┐    │
│                                          │                   │    │
│                                       SQLite           usearch    │
│                                    + JSONL logs       vector index │
└────────────────────────────────────────────────────────────────────┘
```

**Two parallel inference paths coexist:**

| Path | Engine | Primary uses |
|------|--------|-------------|
| **OpenClaw** | Node.js agent sidecar (ACP protocol over WebSockets) | Full autonomous agentic loops, Slack/Telegram bot channels, skill ecosystem, HITL approval, cron jobs |
| **Native Rig** | `rig-core` Rust crate + `UnifiedProvider` | Direct chat (Rig), RAG retrieval, deep web search, image generation tool, MCP sandboxed code execution |

Both paths stream tokens to the frontend via Tauri's typed IPC `Channel<StreamChunk>`.

---

## 2. Repository Layout

```
.
├── src/                        # React / TypeScript frontend
│   ├── App.tsx                 # Root component, window routing
│   ├── main.tsx                # Vite entry
│   ├── components/
│   │   ├── chat/               # Chat UI (ChatLayout shell, ChatProvider, views/, sidebars/, …)
│   │   ├── openclaw/           # OpenClaw-specific views & controls
│   │   ├── imagine/            # Imagine Studio UI
│   │   ├── settings/           # Settings pages (Secrets, Gateway, Personas, …)
│   │   ├── navigation/         # Sidebar navigation
│   │   ├── onboarding/         # First-run wizard
│   │   ├── projects/           # Project management UI
│   │   └── icons/              # Custom SVG icon set
│   ├── hooks/                  # Custom React hooks
│   └── lib/                    # TS utility modules & Tauri command wrappers
│
├── src-tauri/
│   ├── src/                    # Rust source
│   │   ├── lib.rs              # Tauri builder, state init, command registration
│   │   ├── main.rs             # Binary entry (calls lib::run)
│   │   ├── chat.rs             # chat_stream / chat_completion commands
│   │   ├── config.rs           # UserConfig, ConfigManager
│   │   ├── gguf.rs             # GGUF binary metadata parser
│   │   ├── history.rs          # Chat history CRUD (SQLite)
│   │   ├── image_gen.rs        # sd.cpp-based image generation engine
│   │   ├── imagine.rs          # Imagine Studio commands + Gemini Imagen 3
│   │   ├── images.rs           # Image response types
│   │   ├── model_manager.rs    # GGUF model scanning, download, catalog
│   │   ├── openclaw/           # OpenClaw integration module
│   │   ├── permissions.rs      # OS-level permission checks (macOS)
│   │   ├── personas.rs         # Built-in persona definitions
│   │   ├── process_tracker.rs  # Cross-restart PID cleanup
│   │   ├── projects.rs         # Project CRUD commands
│   │   ├── rag.rs              # Document ingestion & RAG retrieval
│   │   ├── reranker.rs         # ONNX cross-encoder reranker
│   │   ├── rig_lib/            # Native Rust agent (rig-core)
│   │   ├── sidecar.rs          # SidecarManager — spawns all sub-processes
│   │   ├── stt.rs              # Speech-to-text (Whisper) commands
│   │   ├── system.rs           # System info commands
│   │   ├── templates.rs        # ChatML / Llama3 / Mistral prompt templates
│   │   ├── vector_store.rs     # usearch vector index wrapper
│   │   └── web_search.rs       # Brave Search API wrapper
│   ├── openclaw-engine/        # Bundled Node.js OpenClaw gateway
│   ├── scrappy-mcp-tools/      # Workspace crate — MCP sandbox & tools
│   ├── migrations/             # SQLite migrations (sqlx)
│   ├── capabilities/           # Tauri permission scopes (JSON)
│   ├── bin/                    # Pre-built sidecars (llama-server, sd, whisper, node, …)
│   └── Cargo.toml
│
├── documentation/              # In-depth design docs + this file
├── .agent/plans/               # Implementation plans for major features
├── index.html                  # Vite HTML shell
├── vite.config.ts
├── tailwind.config.cjs
└── package.json
```

---

## 3. Frontend (React / TypeScript)

### 3.1 Entry Point & Window Model

Tauri spawns **two independent WebView windows**:

| Label | Size | Purpose |
|-------|------|---------|
| *(default)* | 800×600 min | Main application |
| `spotlight` | 600×150, transparent, always-on-top | Global spotlight chat overlay |

`App.tsx` detects the window label via `getCurrentWebviewWindow().label` and renders the appropriate root component:
- **Main window** → `<ChatLayout>` (or `<OnboardingWizard>` if setup incomplete)
- **Spotlight window** → `<SpotlightBar>` with a fully transparent background

First-run detection calls `openclaw.getOpenClawStatus()` and checks `status.setup_completed`. If false or `status.dev_mode_wizard` is true, the `OnboardingWizard` is rendered instead of `ChatLayout`.

### 3.2 Component Tree

```
App
├── ThemeProvider            (localStorage key "vite-ui-theme", dark/light/system)
│   └── ConfigProvider
│       └── ModelProvider
│           ├── ChatLayout                   ← thin shell (~75 lines); wraps ChatProvider
│           │   └── ChatProvider             ← all shared state + handlers (useChatLayout)
│           │       ├── Sidebar              ← collapsible sidebar; AnimatePresence slice switcher
│           │       │   ├── ChatSidebar      ← logo, New Chat, ProjectsSidebar
│           │       │   ├── OpenClawSidebarSlice
│           │       │   ├── ImagineSidebarSlice
│           │       │   ├── SettingsSidebarSlice
│           │       │   └── ModeNavigator    ← bottom icon rail
│           │       └── Main area (AnimatePresence)
│           │           ├── ChatView         ← Virtuoso list + model bar + ChatInput
│           │           │   ├── MessageBubble    ← Markdown + tool blocks (37 KB)
│           │           │   ├── ChatInput        ← multi-modal input (29 KB)
│           │           │   └── ModelSelector    ← provider/model picker
│           │           ├── OpenClawView     ← OpenClaw page router
│           │           │   ├── OpenClawChatView (59 KB)
│           │           │   ├── OpenClawDashboard
│           │           │   ├── OpenClawChannels
│           │           │   ├── OpenClawPresence
│           │           │   ├── FleetCommandCenter
│           │           │   └── … (other OpenClaw pages)
│           │           ├── ImagineView      ← ImagineGeneration / ImagineGallery
│           │           └── SettingsView     ← SettingsContent + SettingsPages
│           │               ├── SecretsTab   ← API key management (64 KB)
│           │               ├── GatewayTab   ← OpenClaw gateway config (68 KB)
│           │               ├── ModelBrowser ← GGUF catalog & download (63 KB)
│           │               └── … (other settings tabs)
│           └── Toaster (sonner)
```

**Key sizing notes:**
- `ChatLayout.tsx` is now a ~75-line shell. All routing state lives in `ChatProvider.tsx` (~450 lines), consumed via `useChatLayout()`. Views are in `chat/views/`; sidebar slices in `chat/sidebars/`.
- `OpenClawChatView.tsx` (59 KB) handles the live streaming view with full tool-event rendering.
- `SecretsTab.tsx` (64 KB) and `GatewayTab.tsx` (68 KB) are the most complex settings pages.

### 3.3 Context Providers

| Provider | File | Provides |
|----------|------|----------|
| `ThemeProvider` | `theme-provider.tsx` | Dark/light/system theme; reads from localStorage; synchronised across both windows |
| `ConfigProvider` | `config-context.tsx` | Wraps `UserConfig` from Rust via `get_user_config` / `update_user_config` commands |
| `ModelProvider` | `model-context.tsx` (21 KB) | Active provider, model selection, inference mode; bridges `ModelSelector` UI to `chat_stream` payload |
| `ChatLayoutProvider` | `chat/ChatProvider.tsx` (~450 lines) | All layout state (active mode, input, attachments, Imagine, OpenClaw), hooks, and handlers — consumed via `useChatLayout()`. Replaces the old inline state in `ChatLayout.tsx`. |
| `ChatProvider` *(generation jobs)* | `chat/chat-context.tsx` (11 KB) | Active conversation ID, message list, send/abort actions |

### 3.4 Key Hooks

| Hook | File | Purpose |
|------|------|---------|
| `useChat` | `use-chat.ts` (25 KB) | Central chat logic: message dispatch, streaming, abort, history loading |
| `useAutoStart` | `use-auto-start.ts` | Automatically starts llama-server and OpenClaw on app launch |
| `useProjects` | `use-projects.ts` | Project CRUD via Tauri commands |
| `useAudioRecorder` | `use-audio-recorder.ts` | Browser MediaRecorder → Whisper STT pipeline |
| `useOpenClawStream` | `use-openclaw-stream.ts` | Thin wrapper around the OpenClaw ACP event stream |
| `useConfig` | `use-config.ts` | Consumes `ConfigProvider` |

### 3.5 Library Modules (`src/lib/`)

| Module | Size | Role |
|--------|------|------|
| `bindings.ts` | 58 KB | Auto-generated Tauri command type bindings (via tauri-specta) |
| `openclaw.ts` | 15 KB | Typed wrappers around all `openclaw_*` Tauri commands |
| `model-library.ts` | 46 KB | Curated catalogue of cloud models (Anthropic, OpenAI, Gemini, Groq, OpenRouter) with metadata |
| `imagine.ts` | 3 KB | Wrappers for `imagine_generate`, `imagine_list_images`, etc. |
| `app-themes.ts` | 9 KB | CSS custom-property theme definitions |
| `style-library.ts` | 7 KB | Image generation style presets |
| `syntax-themes.ts` | 8 KB | Syntax highlighting colour themes for code blocks |
| `prompt-enhancer.ts` | 3 KB | Client-side prompt enhancement utilities |
| `fs-utils.ts` | 2 KB | Tauri FS plugin helper wrappers |
| `vision.ts` | 445 B | Base64 image utilities for vision payloads |
| `utils.ts` | 356 B | General utility helpers |

---

## 4. Backend (Rust / Tauri)

### 4.1 Tauri Entry Point & Application Lifecycle

`src-tauri/src/lib.rs` (461 lines) contains the `run()` function which:

1. **Initialises tracing** (file-backed appender → `scrappy.log`).
2. **Opens / migrates the SQLite database** using `sqlx` with file path `$APP_DATA/scrappy.db`.
3. **Initialises managed state** (see §4.2).
4. **Builds the Tauri application**, including:
   - System tray icon with a context menu (Quit item).
   - Global shortcut registration for the Spotlight bar (default `Cmd+Shift+K`, user-configurable).
   - `WindowEvent::CloseRequested` handler: silently hides the window rather than quitting; only the main window triggers a full shutdown.
5. **Registers all `#[command]` functions** in a single `tauri_specta::collect_commands!` call.

Two special Tauri commands are defined at the top of `lib.rs`:
- `hide_spotlight` — hides the spotlight window.
- `toggle_spotlight` — shows or hides the spotlight window, centering it on screen.

### 4.2 Managed State Registry

All state is registered via `app.manage(...)` before `run()`:

| State type | Source | Notes |
|------------|--------|-------|
| `SqlitePool` | `sqlx::SqlitePoolOptions` | Connection pool to SQLite; shared across all Tauri commands |
| `SidecarManager` | `sidecar.rs` | Arc-based manager for all child processes |
| `VectorStoreManager` | `vector_store.rs` | Lazy-loaded per-scope usearch indices |
| `RerankerWrapper` | `reranker.rs` | Optional ONNX cross-encoder; degrades gracefully |
| `ConfigManager` | `config.rs` | `UserConfig` backed by `config.json` in app data dir |
| `DownloadManager` | `model_manager.rs` | Tracks active GGUF download cancellation tokens |
| `ProcessTracker` | `process_tracker.rs` | Cross-restart orphan PID cleanup |
| `OpenClawManager` | `openclaw/commands/` | WebSocket handle + OpenClaw config |
| `RigManagerCache` | `rig_cache.rs` | Caches the last-built `RigManager` alongside a `RigManagerKey`; rebuilt only when provider, model, token, context size, tools, or knowledge content changes |
| `EngineManager` | `engine/mod.rs` | Holds the active `Box<dyn InferenceEngine>` + `app_data_dir`. Auto-creates the correct engine instance based on compile-time feature flag (`llamacpp`, `mlx`, `vllm`, or `ollama`). Exposes `start_engine`, `stop_engine`, `is_engine_ready`, `setup_engine`, and `get_engine_setup_status` Tauri commands. |

### 4.3 Core Modules

| File | Lines | Summary |
|------|-------|---------|
| `chat.rs` | ~ | Primary chat command dispatcher; routes to Rig or OpenClaw based on payload; calls `resolve_provider()` |
| `rig_cache.rs` | ~ | `RigManagerCache` — Tauri-managed state that caches the last-built `RigManager` by key |
| `sidecar.rs` | 1090 | Process spawning, port allocation, lifecycle management for all AI sidecars |
| `config.rs` | ~ | `UserConfig` schema + `ConfigManager` (file-backed, Mutex-guarded, async writes) |
| `model_manager.rs` | 774 | GGUF model scanning, HuggingFace download, catalog CRUD |
| `gguf.rs` | ~ | Binary parser for GGUF metadata (model family, context length, architecture) |
| `rag.rs` | 925 | Full document ingestion + retrieval pipeline (embedding → chunking → vector indexing) |
| `vector_store.rs` | 318 | `usearch`-backed per-scope vector store, scoped by Global / Project / Chat |
| `reranker.rs` | 163 | ONNX cross-encoder (`ms-marco-MiniLM-L-6-v2`) with graceful degradation |
| `image_gen.rs` | 679 | `sd.cpp` CLI wrapper with architecture detection (FLUX, SD 1.5, SDXL, SD3.5, Qwen, Wan) |
| `imagine.rs` | 605 | Imagine Studio commands + Gemini Imagen 3 API |
| `tts.rs` | ~ | Piper-based TTS sidecar: `tts_synthesize` command (stdin→stdout, returns base64 PCM) |
| `personas.rs` | ~ | Built-in persona prompt constants |
| `templates.rs` | ~ | ChatML, Llama 3, Mistral prompt format strings |
| `history.rs` | ~ | Conversation + message CRUD (SQLite) |
| `projects.rs` | ~ | Project entity CRUD (SQLite) |
| `stt.rs` | ~ | Whisper STT server spawn + transcription endpoint |
| `web_search.rs` | ~ | Brave Search API thin wrapper |
| `system.rs` | ~ | `sysinfo`-based machine info commands |
| `process_tracker.rs` | 131 | PID registry (persisted to JSON) for orphan cleanup on restart |
| `engine/mod.rs` | ~460 | `InferenceEngine` trait, `EngineManager` state, engine Tauri commands (`setup_engine`, `start_engine`, `stop_engine`, `is_engine_ready`, `get_active_engine_info`, `get_engine_setup_status`). Compile-time feature flags select the active engine. |
| `engine/engine_llamacpp.rs` | ~ | LlamaCpp engine: wraps `llama-server` sidecar via existing `SidecarManager` |
| `engine/engine_mlx.rs` | ~ | MLX engine: `uv` (bundled sidecar) bootstraps Python + `mlx-openai-server` (unified text+vision+audio server) at runtime |
| `engine/engine_vllm.rs` | ~ | vLLM engine: `uv` bootstraps Python + `vllm.entrypoints.openai.api_server` (Linux CUDA only) |
| `engine/engine_ollama.rs` | ~ | Ollama engine: detects/connects to existing Ollama daemon |
| `hf_hub.rs` | ~ | HuggingFace Hub model discovery: `discover_hf_models`, `get_model_files`, `download_hf_model_files` — live HF API search with engine-aware tag filtering |

### 4.4 Chat Pipeline (`chat.rs`)

The `chat_stream` command (the primary inference entrypoint) follows this decision tree:

```
chat_stream(ChatPayload, Channel<StreamChunk>)
    │
    ├─ payload.use_openclaw? ──YES──► Forward to OpenClawManager.send_to_live_openclaw()
    │                                  (proxies message to agent via WS, streams back ACP events)
    │
    └─ NO ──► Determine provider from ChatPayload.model_config
                   │
                   ├─ LocalModel ──► Start llama-server if not running (SidecarManager)
                   │                 └─► Construct RigManager(ProviderKind::Local, llama port)
                   │
                   ├─ Anthropic/OpenAI/Gemini/Groq/OpenRouter
                   │                 └─► Construct RigManager(ProviderKind::*, api_key)
                   │
                   └─► Orchestrator.run_turn(messages, permissions, ...)
                            ├─ Retrieve RAG context if docs attached or project active
                            ├─ Inject web search results if enabled
                            ├─ Stream tokens via on_event.send(StreamChunk::Token(...))
                             ├─ Route tool calls through MCP sandbox (Rhai) path
                            └─ Emit StreamChunk::Done with TokenUsage
```

`ChatPayload` fields include: `conversation_id`, `messages`, `model_config` (provider, model name, API key, temperature, context window), `project_id`, `attached_docs`, `enable_web_search`, `enable_rag`, `persona_instructions`, `use_openclaw`.

`StreamChunk` is a tagged enum: `Token(String)`, `Done`, `Error(String)`, `ToolCall { name, input, output }`, `WebSearch { status, results }`, `Usage(TokenUsage)`.

### 4.5 Sidecar Manager (`sidecar.rs`)

`SidecarManager` (1090 lines) is an `Arc<Mutex<Inner>>` wrapper that manages:

| Sidecar | Binary | Function |
|---------|--------|---------|
| **Chat server** | `llama-server` | OpenAI-compatible API for local GGUF inference |
| **Embedding server** | `llama-server` | Dedicated instance for document embeddings |
| **Summarizer server** | `llama-server` | Smaller model for RAG chunk summarization |
| **STT server** | `whisper-server` | Whisper HTTP API for transcription |
| **Image server** | `sd` (sd.cpp) | Stable Diffusion CLI (one-shot invocation, not persistent) |
| **TTS server** | `piper` | Piper TTS binary — one-shot CLI invoked per `tts_synthesize` command; returns raw PCM audio as base64 |
| **Node.js** | `node` | OpenClaw engine runner |

Port allocation uses `generate_config()` which finds a free TCP port via `TcpListener::bind("127.0.0.1:0")`. All ports + authentication tokens are stored in `SidecarManager`'s inner state.

`SidecarEvent` (emitted to frontend via `app.emit`):
```rust
pub enum SidecarEvent {
    Started { service: String },
    Stopped { service: String },
    Crashed { service: String, code: i32 },
    Progress { service: String, message: String, progress: f32, total: f32},
}
```

### 4.6 Configuration (`config.rs`)

`UserConfig` is the central configuration schema, serialized to `$APP_DATA/config.json`. Key fields:

| Field | Type | Default | Notes |
|-------|------|---------|-------|
| `local_model_path` | `Option<String>` | `None` | Path to active chat GGUF |
| `embedding_model_path` | `Option<String>` | `None` | Path to embedding GGUF |
| `summarizer_model_path` | `Option<String>` | `None` | Path to summarizer GGUF |
| `context_window` | `u32` | `4096` | llama-server `-c` argument |
| `temperature` | `f32` | `0.7` | Sampling temperature |
| `top_p` | `f32` | `0.9` | Top-p sampling |
| `enable_web_search` | `bool` | `false` | Global toggle for Rig web search |
| `n_gpu_layers` | `i32` | `0` | GPU offload layers for llama.cpp |
| `chunk_size` | `u32` | `512` | RAG chunk size in tokens |
| `vector_dims` | `u32` | `768` | Embedding vector dimensions |
| `sd_threads` | `u32` | `8` | Thread count for sd.cpp |
| `persona` | `String` | `"default"` | Active persona identifier |
| `spotlight_shortcut` | `String` | `"CmdOrCtrl+Shift+K"` | Global shortcut |
| `mcp_base_url` | `Option<String>` | `None` | Remote MCP server URL (read by `get_mcp_config()` in `ipc.rs`) |
| `mcp_auth_token` | `Option<String>` | `None` | Remote MCP auth token |
| `mcp_cache_ttl_secs` | `u64` | `300` | TTL for `ToolRegistryCache` in seconds |
| `mcp_tool_result_max_chars` | `usize` | `5000` | Max characters before `summarize_result` truncates a tool response |
| `knowledge_bits` | `Vec<KnowledgeBit>` | `[]` | Pinned user knowledge entries |
| `custom_personas` | `Vec<CustomPersona>` | `[]` | User-defined personas |

`ConfigManager` wraps a `Mutex<UserConfig>` and exposes `get_config()`, `save_config()`, and `reload()`. `save_config()` updates the in-memory state synchronously, then spawns a `tokio::fs::write` task to flush to disk asynchronously — keeping the hot path non-blocking.

---

## 5. OpenClaw Integration

### 5.1 Module Structure

```
src/openclaw/
├── mod.rs           # Public re-exports
├── commands/        # Tauri commands exposed to frontend
│   ├── mod.rs       # OpenClawManager struct + all #[command] fns
│   ├── gateway.rs   # Gateway start/stop/status
│   ├── keys.rs      # API key & secrets management
│   ├── rpc.rs       # Generic RPC forwarding commands
│   ├── sessions.rs  # Session CRUD commands
│   └── types.rs     # Shared Rust types
├── config/          # OpenClaw config (openclaw.json / identity.json)
│   ├── mod.rs
│   └── types.rs     # OpenClawConfig, IdentityConfig, etc.
├── ws_client.rs     # WebSocket client (challenge-response auth, reconnect)
├── normalizer.rs    # Raw ACP events → stable UiEvent
├── frames.rs        # WsFrame / WsError types
├── ipc.rs           # Tauri event emission helpers
├── fleet.rs         # Multi-agent fleet management
├── deploy.rs        # Remote deployment helpers
└── extra_commands.rs
```

### 5.2 WebSocket Client (`ws_client.rs`)

The `OpenClawWsClient` implements the **ACP (Agent Communication Protocol)** WebSocket connection to the OpenClaw gateway (default port `18789`).

**Connection lifecycle:**
1. TCP connect → TLS handshake (if `wss://`).
2. Challenge/response authentication using the device token from `identity.json`.
3. RPC message loop: JSON-framed `{ id, method, params }` / `{ id, result/error }`.
4. Incoming event stream forwarded to UI via IPC.
5. Automatic exponential backoff reconnection on disconnect.

`OpenClawWsHandle` provides typed async RPC methods:

| Method | Description |
|--------|-------------|
| `status()` | Gateway health and version |
| `sessions_list()` | Active session enumeration |
| `chat_history(session_key, limit, before)` | Paginated message history |
| `chat_send(session_key, idempotency_key, text)` | Send user message |
| `chat_abort(session_key, run_id)` | Abort in-flight generation |
| `session_delete / session_reset` | Session management |
| `approval_resolve(id, approved)` | HITL approval resolution |
| `cron_list / cron_run / cron_history` | Automation scheduling |
| `skills_list / skills_status / skills_update / skills_install` | Skill management |
| `config_schema / config_get / config_set` | Agent runtime config |
| `web_login_whatsapp / web_login_telegram` | Channel auth |

### 5.3 Commands (`commands/`)

`OpenClawManager` (a Tauri managed state struct) owns the `Option<OpenClawWsHandle>` and the `OpenClawConfig`. Tauri commands in `commands/` include:

- **Gateway**: `start_openclaw_gateway`, `stop_openclaw_gateway`, `get_openclaw_status`, `restart_openclaw_gateway`, `get_openclaw_diagnostics`
- **Sessions**: `get_openclaw_sessions`, `get_openclaw_chat_history`, `openclaw_send_message`, `delete_openclaw_session`, `reset_openclaw_session`, `get_all_openclaw_messages`
- **Keys**: `save_slack_config`, `save_telegram_config`, `save_anthropic_key`, `save_cloud_config`, `save_gateway_settings`, `save_custom_llm_config`, `get_hf_token`, `save_custom_secret`, `delete_custom_secret`, `list_custom_secrets`
- **RPC**: `openclaw_rpc`, `openclaw_resolve_approval`, `openclaw_abort_run`
- **Cron**: `get_openclaw_cron_jobs`, `run_openclaw_cron_job`, `get_openclaw_cron_history`
- **Skills**: `get_openclaw_skills`, `get_openclaw_skills_status`, `update_openclaw_skill`, `install_openclaw_skill`

### 5.4 Config

`OpenClawConfig` reads from (and writes to) several JSON files in `$APP_DATA/OpenClaw/`:

| File | Contents |
|------|---------|
| `state/identity.json` | Device ID, auth token, API keys, cloud provider enables |
| `openclaw.json` | Gateway port, model config, channel settings |
| `auth-profiles.json` | API keys authorized for agent use (Brave, custom secrets) |

The config layer is split into `types.rs` (data structures) and `mod.rs` (read/write logic).

### 5.5 IPC & Event Normalizer

`normalizer.rs` transforms raw ACP WebSocket frames into a stable `UiEvent` enum:

```rust
pub enum UiEvent {
    SessionList(Vec<UiSession>),
    MessageReceived(UiMessage),
    ToolCall { name, input },
    ToolResult { name, output },
    Thinking(String),
    ApprovalRequest { id, command, risk_level },
    Done,
    Error(String),
    // ...
}
```

`ipc.rs` emits these to the frontend via `app.emit("openclaw-event", event)`.

### 5.6 Fleet & Remote Deploy

`fleet.rs` manages **multi-agent fleet** scenarios where multiple OpenClaw gateways are connected simultaneously. `RemoteDeployWizard.tsx` in the frontend drives the Ansible-based remote deployment flow documented in `REMOTE_DEPLOYMENT.md`.

---

## 6. Native Rig Agent

### 6.1 UnifiedProvider

`rig_lib/unified_provider.rs` (757 lines) is the core LLM adapter. It implements `rig::CompletionModel` and provides a single interface for all inference backends:

```rust
pub enum ProviderKind {
    OpenAI, Anthropic, Gemini, Groq, Local, OpenRouter
}
```

For each provider, `UnifiedProvider` dispatches to a dedicated completion method:
- `completion_openai` — standard OpenAI-compatible API (used for Local, Groq, OpenRouter, OpenAI)
- `completion_anthropic` — Anthropic Messages API (handles tool streaming via SSE)
- `completion_gemini` — Google Gemini REST API (v1beta/generateContent)

Streaming equivalents `stream_anthropic`, `stream_gemini`, and `stream_raw_completion` return `Pin<Box<dyn Stream<Item = Result<ProviderEvent, String>>>>`.

`ProviderEvent` carries:
```rust
pub enum ProviderEvent {
    Content(String),
    Usage(TokenUsage),
    ContextUpdate(Vec<Message>),
}
```

Model family information (e.g., `"chatml"`, `"llama3"`, `"gemma"`) is threaded through `UnifiedProvider` to control stop-token injection and system-prompt sanitization.

**Per-model-family features:**
- **ChatML**: injects standard ChatML stop tokens (`<|im_end|>`, etc.)
- **Llama 3**: uses `<|eot_id|>` stop token, specific BOS/EOS handling
- **Gemma**: sanitizes system prompts (no native system role; injected as first user turn)
- **Reasoning models** (GPT-o series): `temperature` set to `None` (not supported)

### 6.2 RigManager / Agent

`rig_lib/agent.rs` (273 lines) builds and owns a `rig::Agent` instance configured with:

- `UnifiedProvider` as the completion model
- `DDGSearchTool` (DuckDuckGo web search)
- `ScrapePageTool` (Chromium-backed scraper)
- `RAGTool` (in-process vector retrieval)
- `ImageGenTool` (triggers `generate_image` command)

Key methods:
- `chat(&str)` → non-streaming single-turn
- `rag_chat(query, history)` → RAG-augmented chat
- `stream_chat(prompt, history)` → raw token stream
- `stream_rag_chat(query, history)` → RAG-augmented stream
- `explicit_search(query)` → direct DDG search as a string result
- `is_cancelled()` → checks the atomic cancel flag

`RigManager` is **cached across requests** via `RigManagerCache` (a Tauri-managed state in `rig_cache.rs`). `chat_stream` calls `cache.get_or_build(key, || RigManager::new(…))` — the manager is only rebuilt when provider, model, token, context window size, tool set, or knowledge content changes. This avoids discarding connection pools and model state on every request.

### 6.3 Orchestrator

`rig_lib/orchestrator.rs` (~560 lines) is the **agentic loop controller**. It uses a single, unified execution mode:

**MCP Sandbox Loop** (the only path)
- Uses `McpOrchestratorConfig` to optionally connect to a remote MCP server
- Routes all tool calls through `Sandbox` (from `scrappy-mcp-tools`) via `build_sandbox_unconditional()`, which ensures a Rhai sandbox is always available for host tools even without a remote MCP server
- Bridges `ToolEvent`s from `TauriEventReporter` to structured `"tool_event"` Tauri events (kind, message, tool_name, percentage, status) emitted to the frontend

> **Note:** The legacy `run_legacy_tool_loop` (~490 lines), which parsed `<tool_code>`/`</tool_code>` XML tags, was deleted. `run_turn` now unconditionally uses the sandbox path.

`Orchestrator.run_turn()` is the main entry-point, returning a `Stream<ProviderEvent>`:

```rust
orchestrator.run_turn(
    messages,
    permissions,    // ToolPermissions { allow_shell, allow_file_io, allow_browser }
    project_id,
    persona_instructions,
    conversation_id,
)
```

### 6.4 Rig Tools

Located in `rig_lib/tools/`:

| Tool file | Tool struct | Description |
|-----------|------------|-------------|
| `web_search.rs` (30 KB) | `DDGSearchTool` | DuckDuckGo HTML scraper + optional Brave Search API fallback; includes `SummarizerTool` pipeline |
| `scrape_page.rs` (9 KB) | `ScrapePageTool` | Chromium-based content extraction via `chromiumoxide`; falls back to `html2text` |
| `rag_tool.rs` (4 KB) | `RAGTool` | Calls `retrieve_context_internal` for in-process RAG |
| `image_gen_tool.rs` (3 KB) | `ImageGenTool` | Invokes `generate_image` command |
| `models.rs` | — | Shared tool argument/output types |
| `trusted_sources.rs` | — | Domain allowlist for trusted scraping sources |

### 6.5 LlamaProvider

`rig_lib/llama_provider.rs` (25 KB) is a lower-level provider that wraps the llama-server's OpenAI-compatible HTTP API with custom stop-token injection, GGUF format detection, and context window management. It is used when more granular control over the local inference call is needed than `UnifiedProvider` provides.

### 6.6 Sandbox Factory & MCP Tools

`rig_lib/sandbox_factory.rs` (22 KB) constructs `Sandbox` instances from `scrappy-mcp-tools`. The `scrappy-mcp-tools` workspace crate provides:

| File | Role |
|------|------|
| `sandbox.rs` | Process isolation abstraction (Docker-compatible) |
| `client.rs` | MCP protocol client |
| `discovery.rs` | MCP server/tool discovery |
| `events.rs` | `ToolEvent` type + `StatusReporter` trait |
| `tools/` | Built-in MCP tools (file, shell, browser, …) |
| `skills/` | Skill definitions |

`tool_discovery.rs` and `tool_router.rs` handle dynamic tool enumeration and dispatch.

---

## 7. Knowledge OS — RAG Pipeline

### 7.1 Document Ingestion

`rag.rs: ingest_document()` — full pipeline:

```
1. Read file bytes
2. Compute SHA-256 hash → deduplicate
3. extract_document_content():
   a. PDF → pdf-extract (text extraction)
   b. PDF (complex/scanned) → vision model OCR via llama-server
   c. HTML → html2text + Chromium scraper fallback
   d. Plain text / code → direct
4. Chunk text into fixed-size windows (chunk_size tokens, with overlap)
5. Ensure embedding server is live:
   a. Probe SidecarManager for existing config (port + token)
   b. If found, HTTP health-check GET /health with 2 s timeout
   c. If dead or absent → call start_embedding_server_core() with
      the embedding_model_path passed from the frontend
      (probes dimension, reinits vector store if needed, spawns server)
6. POST text chunks to embedding server → float32 embedding vectors
7. Store (chunk text, embedding_id) in SQLite document_chunks table
8. Store embedding vector in usearch VectorStore for the appropriate scope
9. Persist VectorStore to disk (.usearch file)
```

`ingest_document` accepts an `embedding_model_path: Option<String>` parameter from the frontend. The frontend no longer manages embedding server lifecycle; it simply passes the configured model path and lets the backend handle the rest.

Deduplication is hash-based — re-ingesting the same file skips all processing.

**Scope routing:** Documents are placed into one of three vector scopes:
- `VectorScope::Global` — available to all chats
- `VectorScope::Project(project_id)` — available within a specific project
- `VectorScope::Chat(chat_id)` — available only in a single conversation

### 7.2 Vector Store

`vector_store.rs: VectorStoreManager` manages multiple `VectorStore` instances (one per scope), each wrapping a `usearch::Index`:
- **Metric**: Cosine similarity
- **Scalar kind**: F32
- **Dimensions**: Runtime-configurable via `Arc<Mutex<usize>>`; initialized from `UserConfig.vector_dimensions` (default 768). `start_embedding_server_core` probes the actual model's `hidden_size` and calls `VectorStoreManager::reinit()` when the dimension changes.
- **`reinit(new_dim)`**: Purges all existing indices (calls `purge_by_dimension` to remove vectors with mismatched dims), creates fresh usearch indices at the new dimension, and updates the persisted config.
- **Persistence**: `$APP_DATA/vectors/<scope>.usearch`

`search_scoped()` searches across multiple scopes and merges results.

### 7.3 Reranker

`reranker.rs` implements a two-stage retrieval pipeline:

1. **Bi-encoder retrieval** (usearch ANN) → top-K candidates
2. **Cross-encoder reranking** (ONNX runtime) → fine-grained relevance scoring

The ONNX model is `Xenova/ms-marco-MiniLM-L-6-v2` (quantized), auto-downloaded from HuggingFace on first use to `$APP_DATA/reranker_model.onnx`. `RerankerWrapper` degrades gracefully: if the model fails to load, retrieval continues without reranking (documents returned in ANN order with zero scores logged).

### 7.4 Retrieval & Context Injection

`rag.rs: retrieve_context_internal()`:
1. Generate query embedding via embedding server.
2. Search across configured `VectorScope`s.
3. Fetch chunk texts from SQLite.
4. Rerank with cross-encoder.
5. Build context string with citation metadata (`[Source: filename, chunk N]`).
6. Optionally interleave web-search results.
7. Prepend context to the user message in the prompt.

---

## 8. Imagine Studio — Image Generation

### 8.1 `image_gen.rs` — sd.cpp Engine

`generate_image()` is the low-level sd.cpp wrapper (679 lines). It:

1. Detects the diffusion architecture from the model filename:

| Architecture | Detection keywords |
|-------------|-------------------|
| `Flux1` | `flux1`, `flux-1` |
| `Flux2Klein` | `flux2klein`, `flux-2-klein` |
| `SD 1.5` | `sd15`, `v1-5`, `stable-diffusion-v1` |
| `SD 2.1` | `sd21`, `v2-1` |
| `SDXL` | `sdxl`, `xl-base` |
| `SD 3.5 Medium` | `sd35-medium`, `sd3.5-medium` |
| `SD 3.5 Large Turbo` | `sd35-large-turbo` |
| `Qwen Image` | `qwen-vl`, `qwenvl-image` |
| `Wan 2.1` | `wan21`, `wan-2.1` |
| `Unknown` | fallback |

2. Builds the CLI argument array for `sd` binary.
3. Spawns the process via `tauri_plugin_shell::ShellExt`.
4. Streams stdout progress via regex parsing (e.g., `step X/Y`).
5. Returns `ImageResponse { path, base64, width, height, seed }`.

### 8.2 `imagine.rs` — Imagine Mode Commands

`imagine.rs` adds persistence, gallery management, and Gemini Imagen 3 support on top of `image_gen.rs`:

| Command | Description |
|---------|-------------|
| `imagine_generate` | Generate image; persist metadata to SQLite `generated_images` table; route to local sd.cpp or Gemini Imagen 3 |
| `imagine_list_images` | Paginated gallery with favorites filter |
| `imagine_search_images` | Full-text search over generation prompts |
| `imagine_toggle_favorite` | Toggle favorite flag |
| `imagine_delete_image` | Delete from DB + filesystem |
| `imagine_get_stats` | Count/stats for the gallery |

`generate_with_gemini()` calls the Gemini Imagen 3 REST API (`imagegeneration:predict`) using the API key from `identity.json`, returning base64-encoded PNG.

### 8.3 Frontend Components

| Component | Size | Role |
|-----------|------|------|
| `ImagineGeneration.tsx` | 49 KB | Prompt input, style/resolution pickers, real-time progress, generation trigger |
| `ImagineGallery.tsx` | 31 KB | Virtualized image grid, favorites, search, delete |
| `ImagineSidebar.tsx` | 8 KB | Recent-generations horizontal strip |

---

## 9. Model Management

### 9.1 Local GGUF Models

Models are stored in `$APP_DATA/models/` and scanned recursively by `scan_models_recursive()`. Each `ModelFile` entry carries: `path`, `filename`, `size_bytes`, `display_size`, `modified_at`.

A second "standard models" directory (`$APP_DATA/standard_models/`) holds pre-downloaded embedding and reranker models.

### 9.2 GGUF Metadata Parsing (`gguf.rs`)

`get_model_metadata(path)` reads the GGUF binary header to extract:
- Architecture name (e.g., `llama`, `gemma`, `phi3`, `mistral`)
- Context length (`llama.context_length`)
- Embedding length, head count, etc.
- Tokenizer vocabulary info

The detected architecture is mapped to a **model family string** (`"chatml"`, `"llama3"`, `"gemma"`, `"phi3"`, `"mistral"`, `"default"`) and threaded through `SidecarManager` → `UnifiedProvider` → `LlamaProvider` for stop-token configuration.

### 9.3 Remote Model Catalog

`model_manager.rs` includes a `RemoteModelEntry` struct and `update_remote_model_catalog` / `get_remote_model_catalog` commands that persist a user-curated list of remote GGUF models (with HuggingFace URLs) in the SQLite `remote_model_catalog` table.

### 9.4 Model Family & Stop-Token Handling

Stop tokens are injected at the llama-server CLI level via the `--stopping-string` argument in `SidecarManager::start_chat_server()`. The correct tokens are selected based on the GGUF-detected model family:

| Family | Stop tokens |
|--------|------------|
| `chatml` | `<\|im_end\|>` |
| `llama3` | `<\|eot_id\|>`, `<\|end_of_text\|>` |
| `gemma` | `<end_of_turn>` |
| `phi3` | `<\|end\|>` |
| `mistral` | `[INST]`, `[/INST]` |
| `default` | `### Human:` |

---

## 10. Persistence Layer

### 10.1 SQLite Schema & Migrations

Migrations live in `src-tauri/migrations/` (10 total, chronological):

| Migration | Table(s) Created |
|-----------|-----------------|
| `20240101000000_init.sql` | `conversations`, `messages` |
| `20240101000001_fts.sql` | `messages_fts` (FTS5 virtual table) |
| `20240115000001_add_images_to_messages.sql` | `messages.images` column |
| `20260116000000_add_chat_id_to_documents.sql` | `documents.chat_id` column |
| `20260116205500_add_attached_docs.sql` | `attached_docs` |
| `20260117000000_add_projects.sql` | `projects`, `project_members` |
| `20260119000000_add_web_search_results.sql` | `web_search_results` |
| `20260127000000_add_reordering.sql` | `conversations.order_index` |
| `20260208000000_model_catalog.sql` | `remote_model_catalog` |
| `20260209000000_generated_images.sql` | `generated_images` |

`sqlx::migrate!()` is called on startup; migrations are embedded in the binary.

### 10.2 Vector Index Files

Stored in `$APP_DATA/vectors/`:
- `global.usearch` — all non-project, non-chat documents
- `project_{id}.usearch` — per-project documents
- `chat_{id}.usearch` — per-conversation documents

Each file is a binary usearch index. `VectorStoreManager` lazily opens indices on first access and keeps them in memory in a `HashMap<VectorScope, Arc<VectorStore>>`.

### 10.3 Flat-File Configuration

All configuration files are stored in `~/Library/Application Support/com.schack.scrappy/`:

| File | Contents |
|------|---------|
| `config.json` | `UserConfig` (Scrappy Rust side) |
| `child_processes.json` | `ProcessTracker` PID list |
| `OpenClaw/state/identity.json` | Device ID, token, cloud API keys |
| `OpenClaw/openclaw.json` | OpenClaw gateway runtime config |
| `OpenClaw/auth-profiles.json` | Authorized API keys |
| `OpenClaw/agents/main/sessions/*.jsonl` | ACP session transcripts |
| `reranker_model.onnx` | Downloaded ONNX reranker model |
| `reranker_tokenizer.json` | Reranker tokenizer |
| `models/` | User GGUF models |
| `standard_models/` | Standard embedding/reranker GGUFs |
| `vectors/` | usearch index files |
| `images/` | Generated images (Imagine Studio) |

---

## 11. Process Lifecycle & Sidecar Binaries

### 11.1 ProcessTracker

`process_tracker.rs` solves the **orphan process problem**: if Scrappy crashes, previously spawned `llama-server` / `node` processes would continue running and consuming resources.

On startup, `ProcessTracker::new()` loads `child_processes.json`. `cleanup_all()` is called in `lib.rs` during the `setup_completed` check to kill any surviving orphans before spawning new ones. `sysinfo` is used to verify PID-to-name mapping before sending `SIGKILL`, preventing accidental killing of unrelated processes with recycled PIDs.

### 11.2 Bundled Binaries

All sidecars are pre-compiled and listed in `tauri.conf.json: bundle.externalBin`:

| Binary | Purpose |
|--------|---------|
| `bin/llama-server` | llama.cpp HTTP inference server (chat + embeddings) |
| `bin/whisper` | Whisper CLI (offline transcription) |
| `bin/whisper-server` | Whisper HTTP server |
| `bin/sd` | stable-diffusion.cpp CLI |
| `bin/piper` | Piper TTS binary — must be manually bundled; invoked per `tts_synthesize` command |
| `bin/node` | Bundled Node.js runtime for OpenClaw engine |

Dynamic libraries (`.dylib`) and Metal shaders (`.metal`) are bundled as resources. The `openclaw-engine/` directory (Node.js code) is also bundled as a resource.

### 11.3 MLX Multimedia Stack (Apple Silicon)

On macOS Apple Silicon builds (feature `mlx`), a single Python-based server — `mlx-openai-server` —
provides all local AI capabilities through standard OpenAI-compatible `/v1` endpoints. It runs inside
a `uv`-managed venv at `~/.scrappy/mlx-env/` and is spawned as a subprocess by `engine_mlx.rs`.

| Capability | `--model-type` | Underlying Library | `.cpp` Equivalent | Status |
|---|---|---|---|---|
| Text LLM | `lm` | `mlx-lm` | `llama.cpp` (llama-server) | ✅ Implemented |
| Vision / Multimodal | `multimodal` | `mlx-vlm` | `llama.cpp` + `--mmproj` | ✅ Implemented |
| Speech-to-Text (STT) | `whisper` | `mlx-whisper` (~10× faster than whisper.cpp) | `whisper.cpp` / whisper-server | 📋 Planned |
| Text-to-Speech (TTS) | *(via mlx-audio)* | `mlx-audio` (Kokoro, Qwen3-TTS) | Piper TTS sidecar | 📋 Planned |
| Embeddings | `embeddings` | `mlx-embeddings` | `llama.cpp --embedding` | 📋 Planned |
| Image Generation | `image-generation` | `mflux` (Flux, SD, Qwen Image) | `sd.cpp` | 📋 Planned |
| Image Editing | `image-edit` | `mflux` (Flux Kontext) | — | 📋 Planned |

**Vision model detection:** `engine_mlx.rs::is_vision_model()` reads the model's `config.json` and
checks for a `vision_config` key (present in all VLMs: Ministral 3, Pixtral, Gemma 3, Qwen-VL, etc.).
If detected, `--model-type multimodal` is passed; otherwise `--model-type lm`.

**Multi-model mode:** `mlx-openai-server` supports a YAML config file for running multiple models
behind one endpoint (e.g., small text model + vision model + whisper), routed by `model_id` in the
API request. This is the path for future multi-capability local inference.

---

## 12. IPC Contract (Tauri Commands)

Scrappy uses **tauri-specta** to generate TypeScript bindings (`src/lib/bindings.ts`, 58 KB) from Rust `#[command]` functions. All commands are collected via `tauri_specta::collect_commands!` in `lib.rs`.

**Command categories and their source files:**

| Category | Module | Examples |
|----------|--------|---------|
| Chat | `chat.rs` | `chat_stream`, `chat_completion`, `count_tokens` |
| Config | `config.rs` | `get_user_config`, `update_user_config`, `get_hf_token` |
| History | `history.rs` | `get_conversations`, `create_conversation`, `delete_conversation`, `get_messages` |
| Models | `model_manager.rs` | `list_models`, `download_model`, `cancel_download`, `delete_local_model`, `get_model_metadata` |
| RAG | `rag.rs` | `ingest_document`, `retrieve_context`, `check_vector_index_integrity` |
| Image Gen | `image_gen.rs` | `generate_image` |
| Imagine | `imagine.rs` | `imagine_generate`, `imagine_list_images`, `imagine_get_stats` |
| Sidecar | `sidecar.rs` | `start_chat_server`, `stop_chat_server`, `get_sidecar_status` |
| STT | `stt.rs` | `start_stt_server`, `transcribe_audio` |
| TTS | `tts.rs` | `tts_synthesize` — synthesizes speech from text using Piper; returns base64 PCM |
| OpenClaw | `openclaw/commands/` | `start_openclaw_gateway`, `get_openclaw_status`, `openclaw_send_message`, … |
| Rig | `rig_lib/mod.rs` | `rig_check_web_search`, `agent_chat` |
| Projects | `projects.rs` | `create_project`, `list_projects`, `delete_project` |
| System | `system.rs` | `get_system_info` |
| Personas | `personas.rs` | `get_personas` |
| Spotlight | `lib.rs` | `toggle_spotlight`, `hide_spotlight` |
| Engine | `engine/mod.rs` | `get_active_engine_info`, `get_engine_setup_status`, `setup_engine`, `start_engine`, `stop_engine`, `is_engine_ready` |
| HF Hub | `hf_hub.rs` | `discover_hf_models`, `get_model_files`, `download_hf_model_files` |

Events emitted from Rust to frontend (via `app.emit()`):

| Event name | Payload | Source |
|------------|---------|--------|
| `sidecar-event` | `SidecarEvent` | `sidecar.rs` |
| `openclaw-event` | `UiEvent` | `openclaw/ipc.rs` |
| `download-progress` | `DownloadProgress` | `model_manager.rs` |
| `imagine-progress` | `{ step, total, preview_b64 }` | `image_gen.rs` |
| `engine_setup_progress` | `{ stage, message }` | `engine/mod.rs` — emitted during MLX/vLLM first-launch bootstrap |

---

## 13. Security Model

### 13.1 API Key Storage — macOS Keychain

All API keys are stored in the **macOS Keychain** (AES-256 encrypted at rest), not in JSON config files.
The `keychain` module (`backend/src/openclaw/config/keychain.rs`) wraps the macOS Security framework via the `security-framework` crate.

**Key flow:**

```
┌───────────────────────────┐
│  macOS Keychain (encrypted)│
│  AES-256 at rest           │
└──────────┬────────────────┘
           │ keychain::get_key()       ← Tauri app process only
           ▼
┌───────────────────────────┐
│  OpenClawConfig (in-memory)│
│  xxx_api_key + xxx_granted │
└──────────┬────────────────┘
           │ write_config()            ← checks `granted` per key
           ▼
┌───────────────────────────┐
│  auth-profiles.json       │          ← ONLY granted keys written
│  (on disk, read by engine) │
└───────────────────────────┘
           │
           ▼
┌───────────────────────────┐
│  OpenClaw Engine (Node.js) │          ← Separate process
│  Reads auth-profiles.json  │
│  + env vars from env_vars()│
└───────────────────────────┘
```

- `identity.json` stores **only** non-sensitive metadata (granted flags, display names, enabled providers list). It never contains API keys.
- `SecretStore` (`secret_store.rs`) is an app-level managed state. The `snapshot()` method was intentionally removed — it returned all keys without checking grant flags, which was a potential leak vector.

### 13.2 Explicit Grant Enforcement

**Saving a key does NOT auto-grant it to OpenClaw.** The `update_*_key()` methods in `identity.rs` write to Keychain and update in-memory state, but leave `xxx_granted` unchanged. The user must explicitly toggle the grant via Settings › Secrets.

Only key *deletion* auto-revokes: setting a key to `None` forces `xxx_granted = false` to prevent stale authorizations.

This applies uniformly to:
- Major providers: Anthropic, OpenAI, OpenRouter, Gemini, Groq, Brave Search
- Implicit providers: xAI, Venice, Together, Moonshot, MiniMax, NVIDIA, Qianfan, Mistral, Xiaomi
- Amazon Bedrock credentials (access key, secret key, region)
- HuggingFace token

### 13.3 Environment Variable Gating

The `env_vars()` method (used when spawning the OpenClaw engine process) conditionally exposes secrets:

| Env Var | Condition |
|---------|-----------|
| `OPENCLAW_CUSTOM_LLM_KEY` / `_URL` / `_MODEL` | Only set when `custom_llm_enabled = true` |
| `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_REGION` | Only set when `bedrock_granted = true` |
| `OPENCLAW_GATEWAY_TOKEN` | Always set (engine's own auth token) |

### 13.4 Content Security Policy

Defined in `tauri.conf.json`:
```
default-src 'self' ipc: http://ipc.localhost;
img-src 'self' blob: data: asset: http://asset.localhost https://asset.localhost
```
External network requests from the WebView are blocked; all external calls go through Rust.

### 13.5 Other Security Boundaries

1. **Asset protocol scope**: Only `$APP_DATA/images/**` is accessible via `asset://` URLs (Imagine Studio gallery). All other filesystem access goes through explicit Tauri commands with path validation.

2. **HITL approval**: High-risk OpenClaw tool executions (shell commands above a configurable risk threshold) are blocked by the gateway until the user explicitly approves them via `openclaw_resolve_approval`.

3. **Model allowlist**: `enabled_cloud_providers` and `enabled_cloud_models` in `identity.json` strictly limit which providers and models the OpenClaw agent can use, preventing cost overruns from model hallucination or injection.

4. **macOS entitlements** (`Entitlements.plist`): Only the minimum required entitlements are granted (network access, file access, child process spawning).

5. **Orphan process cleanup**: `ProcessTracker` with PID-name verification ensures no lingering inference processes persist after crashes.

---

## 14. Build System & Toolchain

### Frontend

| Tool | Version | Role |
|------|---------|------|
| Vite | 7.x | Dev server (port 1420) + production bundler |
| TypeScript | ~5.8.3 | Static typing; strict mode enabled |
| React | 19.1.0 | UI library |
| Tailwind CSS | 3.4 | Utility CSS (PostCSS pipeline) |
| tauri-specta | 2.0.0-rc | Auto-generates TS bindings from Rust types |

**Build:** `tsc && vite build` → `dist/`, consumed by Tauri as `frontendDist`.

### Backend

| Tool | Version | Role |
|------|---------|------|
| Rust | Stable 2021 | Backend language |
| Tauri | 2.x | App framework |
| sqlx | 0.7 | Async SQLite (compile-time query checking) |
| rig-core | 0.7.0 | LLM agent framework |
| usearch | 2.23.0 | High-performance vector index |
| ort | 2.0.0-rc.9 | ONNX runtime (reranker) |
| tokenizers | 0.22.2 | HuggingFace tokenizer (reranker) |
| chromiumoxide | 0.8.0 | Async Chromium DevTools Protocol |
| reqwest | 0.12 | Async HTTP client |
| tokio-tungstenite | 0.24 | WebSocket client (OpenClaw gateway) |
| ed25519-dalek | 2.2.0 | Challenge-response auth signing |
| rhai | 1.x | Embedded scripting for agent automation |
| clawscan | 1.0.0 | Antivirus/safety scanning |

**Build profiles:**
- **dev**: `debug = "line-tables-only"`, `split-debuginfo = "unpacked"`, incremental compilation
- **release**: `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`

### Sidecar Setup Scripts

| Script | Purpose |
|--------|---------|
| `npm run setup:node` | Downloads platform-specific Node.js binary |
| `npm run setup:chromium` | Downloads Chromium binary for web scraping |
| `npm run setup:ai` | Downloads AI model binaries (llama, sd, whisper) |
| `npm run setup:openclaw-engine` | Installs Node.js dependencies for the OpenClaw engine |
| `scripts/setup_uv.sh` | Downloads the `uv` Python package manager as a Tauri sidecar (for MLX/vLLM builds) |
| `scripts/generate_tauri_overrides.sh` | Generates `tauri.override.json` for engine-specific builds (externalBin, productName) |

---

## 15. Known Technical Debt

| Item | Location | Notes |
|------|----------|-------|
| **`start_image_server` is a no-op** | `sidecar.rs` | `sd.cpp` is invoked as a one-shot CLI per request, not as a persistent server; the Tauri command only stores the model path |
| **`start_tts_server` is a no-op** | `sidecar.rs` | Piper TTS is also invoked as a one-shot CLI (stdin→stdout) per `tts_synthesize` call; no persistent server is started |
| **`mcp_cache_ttl_secs` wired in config but not yet plumbed** | `config.rs`, `tool_discovery.rs` | `mcp_cache_ttl_secs` is stored in `UserConfig` but `ToolRegistryCache` must still be instantiated with this value at the call site in `tool_discovery.rs` |
| **Whisper server unauthenticated** | `sidecar.rs`, `stt.rs` | `whisper-server` has no `--api-key` flag; STT endpoint is unauthenticated on localhost. Blocked on upstream (`whisper.cpp`) adding server auth support. Workaround: bound to `127.0.0.1` only. |
| **Piper binary not auto-downloaded** | `src-tauri/bin/piper` | The `download_ai_binaries.js` script does not yet fetch Piper; it must be placed manually. |
