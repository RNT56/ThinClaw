<p align="center">
  <img src="backend/icons/128x128@2x.png" width="160" alt="ThinClaw Desktop Logo">
</p>

# ThinClaw Desktop: The ThinClaw Companion App

ThinClaw Desktop is a professional, open-source AI cockpit designed for executive-level workflows, privacy-focused developers, and power users. Built on a high-performance **Tauri v2 / Rust** backend, it intentionally contains two AI systems: the **Direct AI Workbench** for standard local/cloud chat, RAG, voice, and media generation, and the **ThinClaw Agent Cockpit** for autonomous ThinClaw runtime control.

![ThinClaw Desktop App Preview](assets/app-preview.png)

---

## Installation & Setup

For a deep dive into environment configuration, prerequisites, and production builds for all engines, see the **[Development Setup Guide](documentation/setup.md)**. For release acceptance, use the **[Manual Smoke Checklist](documentation/manual-smoke-checklist.md)** and record a dated smoke report.

### 1. Requirements
- **macOS / Linux / Windows** (Tauri v2 compatible).
- **Node.js 22.x+** and **npm** (for frontend tooling only).
- **Rust (Stable)**.

### 2. Quick Start
```bash
# 1. Install project dependencies
npm install

# 2. Automated sidecar initialization (Chromium, AI binaries)
npm run setup:all

# 3. Launch in Developer Mode (default engine: llama.cpp)
npm run tauri:dev:llamacpp

# Or with a different inference engine:
# npm run tauri:dev:mlx
# npm run tauri:dev:ollama
```

### 3. Setup Advice
- **Secrets**: Go to **Settings > Secrets** to add your API keys. ThinClaw Desktop supports **Anthropic**, **OpenAI**, **Google Gemini**, **Groq**, **OpenRouter**, and **Brave Search**. Remember to toggle "Grant Access" for each key to enable them for the agent.
- **Custom Secrets**: You can now add arbitrary custom secrets for specialized agent workflows.
- **Hugging Face**: A **Hugging Face Read Token** is highly recommended. It may be required on first launch to download gated LLMs (like Llama/Gemma) or specialized diffusion models. You can add this in **Settings > Secrets**.
- **Models**: Download models via the in-app **Model Browser** (Settings → Models). Use the **Library** tab for bundled models or the **Discover** tab to search HuggingFace Hub. The Discover tab auto-filters by your active engine (GGUF for llama.cpp/Ollama, MLX safetensors for MLX, AWQ for vLLM).

### 4. Alpha Contract Docs
- **[Runtime Boundaries](documentation/runtime-boundaries.md)**: the two-system Desktop architecture: Direct AI Workbench vs ThinClaw Agent Cockpit, shared services, state ownership, and iOS implications.
- **[Bridge Contract](documentation/bridge-contract.md)**: stable Tauri command, event, routing, and binding contract.
- **[Runtime Parity Checklist](documentation/runtime-parity-checklist.md)**: local ThinClaw runtime parity status and fixture/release criteria.
- **[Remote Gateway Route Matrix](documentation/remote-gateway-route-matrix.md)**: local/remote command behavior and unavailable-response rules.
- **[Environment Requirements](documentation/env-requirements.md)**: required toolchains, env vars, and final gate commands.
- **[Secrets Policy](documentation/secrets-policy.md)**: ThinClaw key naming, grants, legacy fallback, and remote secrecy rules.
- **[Manual Smoke Checklist](documentation/manual-smoke-checklist.md)**: repeatable local and remote release smoke.
- **[External Release Prerequisites](documentation/external-release-prerequisites.md)**: release-operator inputs that must stay gated or documented.

---

## Vision & Key Capabilities

*   **ThinClaw Agent Cockpit**: Full implementation of the ThinClaw streaming protocol, enabling the autonomous agent runtime to plan, execute tools, and reflect in real-time.
*   **Direct AI Workbench (Rig + Inference Router)**: A high-performance direct chat/media stack built on `rig-core`, local/cloud inference engines, RAG, deep web search, and visual asset generation.
*   **Autonomous Agency**: The ThinClaw agent ecosystem enables human-in-the-loop agents that can execute shell commands, manage files, and browse the web.
*   **Custom Secrets & Privacy**: Securely manage Anthropic, OpenAI, Gemini, Groq, OpenRouter, and custom API keys with granular "Grant Access" controls.
*   **Multi-Engine Inference**: Supports **llama.cpp** (Metal/CUDA), **MLX** (Apple Silicon), **vLLM** (CUDA), and **Ollama** as swappable local inference backends — selected at compile time via Cargo feature flags. Each engine exposes a unified OpenAI-compatible HTTP API.
*   **HuggingFace Hub Discovery**: Live search of HuggingFace models filtered by the active engine, with GGUF quantization picker, auto-mmproj detection, and streamed downloads.
*   **Standalone Gateway Support**: Use the embedded ThinClaw runtime locally or connect to remote ThinClaw gateways for distributed agent control.
*   **Imagine Studio**: A dedicated creative suite for image generation with custom bespoke icons, multiple provider support (Local Stable Diffusion, Gemini Imagen 3), and a high-performance integrated **Gallery** with real-time generation progress tracking, horizontal recent-generations strip, and settings restoration support.
*   **MCP Server Integration**: Manage first-class ThinClaw MCP servers, tools, resources, prompts, OAuth, interaction responses, and legacy Rhai sandbox tools from Desktop.
*   **Voice I/O (TTS & STT)**: Native Text-to-Speech (Piper) and Speech-to-Text (Whisper) sidecars for fully voice-enabled conversations.
*   **Human-in-the-Loop (HITL)**: Advanced security protocols that pause execution for explicit user approval of high-risk shell commands.
*   **Knowledge OS (RAG)**: Enterprise-grade retrieval pipeline with vector search (`usearch`), ONNX reranking, and citation-backed generation.
*   **Web Intelligence**: Deep web scraping via bundled Chromium and real-time news search via Brave Search.
*   **Spotlight Command Bar**: An ultra-fast, system-wide AI overlay for quick queries, neural lookups, and rapid brain access (`Cmd+Shift+K`).

---

## Spotlight: Global AI Access

ThinClaw Desktop includes a premium **Spotlight Bar**—a glassmorphic, system-wide interface that brings the power of your neural engine to any application.

-   **Instant Summon**: Press `Cmd + Shift + K` (macOS) to toggle the Spotlight bar from anywhere.
-   **Neural Status**: A biological status indicator shows your brain state in real-time (Green = Active/Local Brain Online, Gray = Inactive).
-   **Transient Intelligence**: Optimized for "quick-tap" queries. By default, Spotlight sessions are purged upon closing to keep your primary history clean and focused.
-   **Hotkeys**:
    -   `Cmd + L`: Purge the current spotlight session and start fresh.
    -   `Esc`: Hide the bar instantly.
    -   `Enter`: Send prompt.
    -   `Shift + Enter`: Multi-line input.

---

## Technical Architecture

ThinClaw Desktop intentionally contains two AI systems. The **Direct AI Workbench** powers standard local/cloud chat, RAG, voice, and media generation without autonomous agent behavior. The **ThinClaw Agent Cockpit** embeds or remotely controls the ThinClaw autonomous runtime. Read the **[Runtime Boundaries](documentation/runtime-boundaries.md)** before changing either path.

ThinClaw Desktop uses a **Modular Sidecar Architecture**. The Rust core orchestrates several specialized processes to keep the main application lightweight and responsive.

```mermaid
graph TD
    subgraph Frontend [React 19 Frontend - frontend/src/]
        Shell[App Shell / Navigation]
        DirectUI[Chat + Imagine Modes]
        AgentUI[ThinClaw Mode]
    end

    subgraph Backend [Rust Core - backend/src/]
        Tauri[Tauri v2 Main]
        DirectBackend[Direct Workbench Commands]
        SidecarManager[Sidecar Manager]
        EngineManager[Engine Manager]
        InferenceRouter[Inference Router]
        RigAgent[Rig Orchestrator]
        ThinClawBridge[ThinClaw Bridge]
        RemoteProxy[Remote Gateway Proxy]
    end

    subgraph Sidecars [Sidecar Processes]
        Llama[llama.cpp / MLX / vLLM / Ollama]
        Chromium[Chromium Web Scraper]
        Whisper[Whisper STT]
        TTS[Piper TTS]
        SD[Stable Diffusion]
    end

    subgraph AgentRuntime [ThinClaw Runtime]
        Embedded[Embedded ThinClaw Library]
        RemoteGateway[Remote ThinClaw Gateway]
    end

    subgraph Storage [Persistence]
        DirectSQLite[(Direct Workbench SQLite)]
        Vectors[(Direct RAG USearch Index)]
        ThinClawStore[(ThinClaw Runtime Store)]
        Identity[(Keychain + Identity.json)]
    end

    Shell --> DirectUI
    Shell --> AgentUI
    DirectUI <-->|chat_stream / imagine_generate / history commands| DirectBackend
    AgentUI <-->|thinclaw_* commands + thinclaw-event| ThinClawBridge
    DirectBackend --> SidecarManager
    DirectBackend --> EngineManager
    DirectBackend --> InferenceRouter
    DirectBackend --> RigAgent
    SidecarManager --> Sidecars
    EngineManager --> Llama
    RigAgent --> Chromium
    RigAgent --> Vectors
    DirectBackend <-->|reads/writes| DirectSQLite
    DirectBackend <-->|reads/writes| Vectors
    ThinClawBridge --> Embedded
    ThinClawBridge --> RemoteProxy
    RemoteProxy --> RemoteGateway
    Embedded <-->|runtime state| ThinClawStore
    ThinClawBridge <-->|secrets/grants| Identity
    DirectBackend <-->|provider secrets| Identity
```

### 1. The ThinClaw Agent Cockpit
The heart of ThinClaw Desktop's autonomous agency. Built on the **ThinClaw** Rust agent runtime, running **in-process** as a library crate — no Node.js sidecar, no WebSocket bridge:
-   **Session Management**: Each conversation has a dedicated thread with persistent history.
-   **Tool System**: Built-in tools for `exec` (shell), `file_io`, `browser`, `skill` extensions, and **MCP remote tools**.
-   **Streaming Response**: Real-time streaming of tokens, tool inputs, and internal "thinking" via `TauriChannel`.

### 2. The Direct AI Workbench (`backend/src/chat.rs`, `backend/src/rig_lib`, `backend/src/inference`)
A non-autonomous local/cloud AI stack for standard chat, RAG, search, voice, and media generation. It focuses on direct user requests:
-   **RAG Integration**: Direct access to the `usearch` vector store for context injection.
-   **Deep Search**: Utilizes `DDGSearchTool` and `ScrapePageTool` for gathering real-time information.
-   **Image Generation**: Native integration with image generation sidecars via `ImageGenTool`, featuring a premium studio interface with real-time progress tracking, style presets, and multi-resolution support (512px to 2K).

---

## ThinClaw Configuration & Lifecycle

ThinClaw is highly configurable through a combination of system files and workspace-level markdown instructions.

### 1. System Infrastructure
These files handle the mechanical aspects of the agent:
- **`identity.json`**: Stores your persistent device ID, auth token, grant flags, and enabled provider/model lists. **Does not contain API keys** — those are stored in the macOS Keychain.
- **Runtime config**: Core ThinClaw runtime config defining the gateway port (default `18789`), model providers, and channel settings. During alpha it is stored in the legacy-compatible `thinclaw.json` file.

### 2. Workspace Markdown (The Agent's "Brain")
The agent's personality and rules are defined by markdown files in its workspace. These are injected into the system prompt on session start:
- **`AGENTS.md`**: Core operational manual. Covers memory usage, group chat etiquette, and interaction rules (e.g., avoiding multiple responses to the same input).
- **`SOUL.md`**: Defines your agent's persona, values, and fundamental behavior.
- **`IDENTITY.md`**: High-level identity markers like name, "creature type," and signature emoji.
- **`USER.md`**: Stores what the agent knows about *you* (name, preferences, context).
- **`TOOLS.md`**: Practical conventions for tool usage (camera names, SSH details, shell preferences).

### 3. Lifecycle & Automation
- **`BOOTSTRAP.md`**: A one-time setup ritual performed by the agent in a new workspace.
- **`BOOT.md`**: Startup checklist executed every time the gateway/agent restarts.
- **`HEARTBEAT.md`**: A proactive, periodic checklist for automated tasks (e.g., checking weather, emails, or project status every 30 minutes).

### 4. Management & Visibility
- **Settings Tab**: Manage API keys, model selection, gateway connection modes, and customize your **Spotlight Global Shortcut**.
- **Persona Editing**: Modify `.md` files in the workspace directory to refine the agent's behavior in real-time. For built-in personas, you can find the prompt definitions in `backend/src/personas.rs`.
- **Logs/Transcripts**: Full interaction logs and tool histories are stored as JSONL in the ThinClaw session directory.

### 5. Cloud Inference Providers
ThinClaw Desktop features native integration with the world's most powerful inference engines:
- **Anthropic**: Support for **Claude 4.5 Sonnet** and **Opus** with native Tool Use.
- **OpenAI**: First-class support for **GPT-5.2** (with specialized reasoning) and **GPT-4o** variants.
- **Google Gemini**: Integrated **Gemini 2.0/3.0 Flash/Pro** with support for massive 1M+ token contexts and **Imagen 3** image generation.
- **Groq**: Ultra-fast cloud inference for open models like **Llama 3.3 70B** and **Mixtral**.
- **OpenRouter**: Gateway access to 100+ specialized models via a single API key.
- **Custom Secrets**: Define and grant access to any external API key for use in custom agent tools.

Configure all API keys in **Settings > Secrets**. Toggle "Grant Access" per key to control agent access at runtime.

---

## Project Structure

### Backend (`backend/`)
-   `src/thinclaw/`: ThinClaw integration layer.
    -   `commands/`: Tauri IPC command handlers (`gateway.rs`, `keys.rs`, `sessions.rs`, `rpc.rs`, etc.)
    -   `runtime_bridge.rs`: ThinClaw agent lifecycle — init, config, Agent construction, shutdown.
    -   `tauri_channel.rs`: `TauriChannel` bridging ThinClaw events to Tauri `emit()`.
    -   `secrets_adapter.rs`: Keychain ↔ ThinClaw secrets adapter.
    -   `tool_bridge.rs`: MCP tool bridge for ThinClaw agent tool calls.
    -   `sanitizer.rs`: LLM token stripping (ChatML, Llama, Jinja markers).
    -   `ui_types.rs`: `UiEvent` enum — stable UI contract (16+ variants).
-   `src/rig_lib/`: Direct AI Workbench orchestration and specialized tools.
    -   `tools/`: `web_search.rs` (29KB), `calculator_tool.rs` (37KB), `scrape_page.rs`, `image_gen_tool.rs`, `rag_tool.rs`.
    -   `orchestrator.rs`: Multi-turn web search and synthesis pipeline.
    -   `unified_provider.rs`: Unified inference provider abstraction.
-   `src/engine/`: Multi-engine inference system (`InferenceEngine` trait, engine implementations for llama.cpp, MLX, vLLM, Ollama).
-   `src/inference/`: InferenceRouter — 5-modality routing (Chat, Embedding, TTS, STT, Diffusion) with local and cloud backends.
-   `src/cloud/`: Cloud storage system — 7 providers (S3, GDrive, iCloud, OneDrive, Dropbox, WebDAV, SFTP) + encryption + sync.
-   `src/sidecar.rs`: The manager for all background binaries (Llama, Chromium, Whisper, TTS, SD).
-   `src/hf_hub.rs`: HuggingFace Hub model discovery, file parsing, and download.
-   `src/templates.rs`: Prompt templates (ChatML, Llama3, Mistral, **Gemma**, **Qwen**) used for model formatting.
-   `src/tts.rs` / `src/stt.rs`: Text-to-Speech (Piper) and Speech-to-Text (Whisper) integration.
-   `src/imagine.rs` / `src/image_gen.rs` / `src/images.rs`: Imagine Studio and image generation pipeline.
-   MCP tools crate: Rust crate providing the MCP sandbox (Rhai scripts, tool discovery, HTTP client).
-   `documentation/`: Current Desktop runtime boundaries, bridge contract, setup, route matrix, secrets policy, smoke checklist, and release-gate docs.

### Frontend (`frontend/src/`)
-   `components/chat/`: The high-performance chat interface.
-   `components/thinclaw/`: Visualizations for ThinClaw status and tool execution.
-   `components/imagine/`: Imagine Studio UI (gallery, prompt, style presets).
-   `components/settings/`: Settings pages including `McpTab.tsx`, `SettingsSidebar.tsx`, `SettingsPages.tsx`.
-   `hooks/use-thinclaw-stream.ts`: Real-time agent event processing.
-   `hooks/use-chat.ts`: Core chat state management.

---

## Developer Guide: Extending ThinClaw Desktop

### Adding a New Prompt Template
Templates are defined in `backend/src/templates.rs`. To add one:
1.  Define a new `pub const` with your Jinja-like template (ChatML, Llama3, Mistral, Gemma, and Qwen formats already exist).
2.  Add it to the renderer logic in the model manager (`src/model_manager.rs`).

### Adding a New ThinClaw Skill
Skills extend the **ThinClaw** agent:
1.  Create a skill definition with a JSON schema in the ThinClaw skill directory.
2.  Implement the `execute` logic.
3.  The UI will automatically handle rendering based on the tool metadata.

### Adding a Native Rust Tool (Rig)
1.  Implement the `Tool` trait in `backend/src/rig_lib/tools/`.
2.  Register the tool in `RigManager::new` within `backend/src/rig_lib/agent.rs`.
3.  Ensure the tool emits progress events to the UI if long-running.

### Adding an MCP Remote Tool
1.  Register the server through **Settings > MCP** or the ThinClaw gateway MCP APIs.
2.  Verify tools, resources, prompts, OAuth, and interaction callbacks in the MCP management UI.
3.  Keep the legacy Rhai sandbox path only for compatibility with older local tool servers.

---

## Security & Safety Philosophy

1.  **Strict Local-First**: Your data and AI transcripts stay on your machine.
2.  **Keychain-Secured Secrets**: API keys are stored in the **macOS Keychain** (AES-256 encrypted at rest), never in plaintext config files. `identity.json` stores only non-sensitive metadata (grant flags, enabled providers).
3.  **Explicit Grant Enforcement**: Saving an API key does **not** automatically expose it to the agent. You must toggle "Grant Access" per key in Settings › Secrets. Only granted keys are injected into the ThinClaw engine as environment variables.
4.  **Environment Variable Gating**: Sensitive credentials (Custom LLM keys, AWS Bedrock) are only injected into the engine process when their corresponding feature is explicitly enabled.
5.  **Human Governance**: Every dangerous command triggers a UI approval request (HITL).
6.  **Sandbox Ready**: Tool execution can be configured to run in Docker containers.

---

## Contributing & Community

ThinClaw Desktop is an evolving platform. We welcome contributions to the RAG pipeline, new agent skills, or UI refinements.

1.  Read `documentation/runtime-boundaries.md` before deciding whether a feature belongs to the Direct AI Workbench, the ThinClaw Agent Cockpit, or shared Desktop infrastructure.
2.  Check `backend/src/thinclaw/commands/` and `backend/src/rig_lib/agent.rs` for backend extension points.

---

## License

Distributed under the **GNU General Public License v3.0** (Strong Copyleft). See `License.md` for more information and attribution requirements.
