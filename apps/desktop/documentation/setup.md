# ThinClaw Desktop: Development Setup Guide

> **Last updated:** 2026-02-23

ThinClaw Desktop is a cross-platform desktop AI application built with Tauri (Rust + React). It supports **multiple inference engines** — the engine is selected at compile time via Cargo feature flags.

| Engine | Platforms | Model Format | Notes |
|--------|-----------|-------------|-------|
| **llama.cpp** *(default)* | macOS (Metal), Linux (CUDA/CPU), Windows | GGUF | Bundled sidecar binary |
| **MLX** | macOS Apple Silicon only | MLX safetensors (directory) | Python auto-bootstrapped via `uv` at first launch |
| **vLLM** | Linux CUDA only | AWQ / HuggingFace (directory) | Python auto-bootstrapped via `uv` at first launch |
| **Ollama** | macOS, Linux, Windows | GGUF (via Ollama daemon) | Requires external Ollama install from [ollama.ai](https://ollama.ai) |
| **Cloud Only** | All | — | No local inference; cloud providers only |

---

## 📋 Prerequisites

### All Platforms

- **Rust (Stable)**: Backend language.
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Node.js (v22+)**: Frontend tooling.
  - [Download Node.js](https://nodejs.org/)

### macOS

- **Xcode Command Line Tools**:
  ```bash
  xcode-select --install
  ```
- **Homebrew** *(recommended)*:
  ```bash
  /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
  ```

### Linux

- **System libraries** (Ubuntu/Debian):
  ```bash
  sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
       libssl-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev
  ```
- **CUDA** *(required for vLLM engine or GPU-accelerated llama.cpp)*: Install the NVIDIA CUDA toolkit from [developer.nvidia.com](https://developer.nvidia.com/cuda-downloads).

### Windows

- **Microsoft C++ Build Tools**: Install ≥ Visual Studio 2019 build tools with the "Desktop development with C++" workload.
- **WebView2**: Ships with Windows 10/11 by default; install manually on older systems from [developer.microsoft.com](https://developer.microsoft.com/microsoft-edge/webview2/).

---

## 🚀 Getting Started

### 1. Clone the Repository
```bash
git clone https://github.com/RNT56/ThinClaw.git
cd ThinClaw/apps/desktop
```

### 2. Install Dependencies & Download Sidecars
```bash
npm install
npm run setup:all
```

This does everything in one go:
1. Installs React frontend + dev dependencies
2. Then `setup:all` runs sub-scripts in sequence:

| Script | npm command | What it does |
|--------|------------|-------------|
| `setup_chromium.sh` | `npm run setup:chromium` | Downloads Chromium for local web scraping |
| `download_ai_binaries.js` | `npm run setup:ai` | Downloads llama-server, whisper-server, sd (Stable Diffusion) binaries + shared libraries |

All scripts are **idempotent** — they skip binaries that already exist in `backend/bin/`.

> **Tip:** You can also run any sub-script individually, e.g. `npm run setup:ai` to re-download just the AI binaries.

### 3. Advanced: Individual Sidecar Setup
If you need specific binaries individually or want to update just one:
```bash
# Just the llama.cpp inference server (cross-platform)
bash scripts/setup_llama.sh          # uses latest known-good release
bash scripts/setup_llama.sh b4618    # pin to a specific release tag

# Just the uv Python manager (for MLX/vLLM builds only)
bash scripts/setup_uv.sh
```

### 4. Verify Binary Setup
After setup, you should see the following in `backend/bin/` (macOS ARM64 example):
```
backend/bin/
├── llama-server-aarch64-apple-darwin
├── whisper-server-aarch64-apple-darwin
├── whisper-aarch64-apple-darwin
├── sd-aarch64-apple-darwin
├── tts-aarch64-apple-darwin
├── libllama.dylib
├── libggml*.dylib
└── *.metal
```

On Linux, the suffix would be `x86_64-unknown-linux-gnu`. On Windows, `x86_64-pc-windows-msvc.exe`.

> **Note:** `tts` (Piper) must currently be placed manually — see [Troubleshooting](#-troubleshooting).

---

## 🖥️ Running in Development

### Default (llama.cpp)
```bash
npm run tauri dev
```
This uses the default Cargo feature `llamacpp`. The app will launch with llama.cpp as the inference engine.

### Other Engines (dev mode)
To develop with a different engine, pass the feature flags through to Cargo:
```bash
# MLX (macOS Apple Silicon only)
npm run tauri dev -- -- --no-default-features --features mlx

# vLLM (Linux CUDA only)
npm run tauri dev -- -- --no-default-features --features vllm

# Ollama (requires `ollama serve` running separately)
npm run tauri dev -- -- --no-default-features --features ollama

# Cloud-only (no local inference)
npm run tauri dev -- -- --no-default-features
```

> **Note on `-- --`:** The first `--` passes args to the Tauri CLI; the second `--` passes args to `cargo build`.

---

## 📦 Production Builds

ThinClaw Desktop provides **dedicated npm scripts** for building each engine variant. Each script:
1. Runs `scripts/generate_tauri_overrides.sh` to generate a `backend/tauri.override.json` with the correct sidecar/resource bundle list for that engine
2. Invokes `tauri build` with the appropriate `--config` and `--features` flags

### llama.cpp Build *(default)*
```bash
npm run tauri:build:llamacpp
```
**Bundles:** llama-server, whisper-server, sd, piper, Chromium, all `.dylib`/`.metal` assets.

### MLX Build *(macOS Apple Silicon)*
```bash
# First download uv (one-time)
bash scripts/setup_uv.sh

# Build
npm run tauri:build:mlx
```
**Bundles:** `uv` (Python manager), whisper-server, piper, Chromium.
**Does NOT bundle:** llama-server, sd (not needed for MLX inference).
**First-launch behavior:** The app shows an `EngineSetupBanner` guiding the user through a one-time Python environment setup (~200 MB, 2-3 minutes).

### vLLM Build *(Linux CUDA)*
```bash
# First download uv (one-time)
bash scripts/setup_uv.sh

# Build
npm run tauri:build:vllm
```
**Bundles:** `uv`, whisper-server, piper, Chromium.
**First-launch behavior:** Same as MLX — one-time Python bootstrap (~1 GB, 5-10 minutes).

### Ollama Build
```bash
npm run tauri:build:ollama
```
**Bundles:** whisper-server, piper, Chromium.
**Does NOT bundle:** llama-server, uv (Ollama manages its own inference).
**Requires:** User must install [Ollama](https://ollama.ai) separately and run `ollama serve`.

### Cloud-Only Build
```bash
npm run tauri:build:cloud
```
**Bundles:** Chromium.
**Minimal footprint** — no local inference at all. Uses cloud providers (Anthropic, OpenAI, Gemini, Groq, etc.) exclusively.

### What `generate_tauri_overrides.sh` Does

The `scripts/generate_tauri_overrides.sh` script generates `backend/tauri.override.json` which controls what gets bundled:

| Engine | `externalBin` included | `resources` included |
|--------|----------------------|---------------------|
| llamacpp | llama-server, whisper, whisper-server, sd, tts | `*.dylib`, `*.metal`, chromium |
| mlx / vllm | uv, whisper, whisper-server, tts | `libwhisper*.dylib`, chromium |
| ollama | whisper, whisper-server, tts | `libwhisper*.dylib`, chromium |
| none (cloud) | *(none)* | chromium |

---

## 🧪 Testing

```bash
# Frontend tests (Vitest)
npm test                      # single run
npm run test:watch            # watch mode
npm run test:ui               # browser UI
npm run test:coverage         # with coverage

# Backend tests (Cargo)
npm run test:backend          # default engine (llamacpp)
npm run test:backend:all-engines  # test all engine feature combinations
```

---

## ⚙️ Inference Engine Details

### llama.cpp *(default)*
- Uses **GGUF** models exclusively for local inference.
- On macOS: Metal GPU acceleration is automatic (all GPU layers, Flash Attention).
- On Linux: CUDA acceleration when available; CPU fallback otherwise.
- Recommended model families and quantizations:
  - **Llama 3.x**: `Llama-3-8B-Instruct` (Q4_K_M or Q8_0)
  - **Gemma 3**: `gemma-3-4b-it` or `gemma-3-12b-it`
  - **Mistral / Mixtral**: `Mistral-7B-Instruct`
  - **Qwen 2.5**: `Qwen2.5-7B-Instruct`
- Models can be downloaded from the in-app **Model Browser** (Settings → Models → Library/Discover tabs) or placed manually in the `models/` folder inside the app data directory. Find the path in **Settings → Troubleshooting**.

### MLX *(macOS Apple Silicon)*
- Uses **MLX safetensors** model directories from HuggingFace.
- First launch triggers a one-time Python environment setup (~200 MB) via `uv`. The `EngineSetupBanner` component guides you through this process.
- Best performance on M1/M2/M3/M4 chips due to Apple's unified memory architecture.
- Use the **Discover** tab in the Model Browser to search for MLX-tagged models.

### vLLM *(Linux CUDA)*
- Uses **AWQ / HuggingFace** model directories.
- Requires an NVIDIA GPU with CUDA support.
- First launch triggers a one-time Python environment setup (~1 GB) via `uv`.
- Use the **Discover** tab in the Model Browser to search for AWQ-tagged models.

### Ollama
- Connects to an existing **Ollama daemon** running on the system.
- Install Ollama from [ollama.ai](https://ollama.ai) and run `ollama serve` before starting ThinClaw Desktop.
- Uses GGUF models managed by Ollama's own model library.

---

## 🍎 Apple Silicon Optimization

These tips apply when running on macOS with M-series chips (any engine):

### Unified Memory
ThinClaw Desktop takes advantage of macOS Unified Memory. In **Settings → Server & Memory**, allocate sufficient RAM for your models. Metal handles the distribution between CPU/GPU automatically.

### Native Scraper
The bundled Chromium instance runs natively on Apple Silicon, ensuring minimal overhead when the agent browses the web.

---

## 🔑 Recommended API Keys & Tokens

While ThinClaw Desktop can run fully local AI, its advanced features benefit from the following:

| Provider | Models | Link |
|----------|--------|------|
| **Anthropic** | Claude 4.5 Sonnet, Claude 4.5 Opus (native Tool Use) | [Get a Key](https://console.anthropic.com/) |
| **OpenAI** | GPT-5.2, o3/o4-mini reasoning models | [Get a Key](https://platform.openai.com/) |
| **Google Gemini** | Gemini 2.0/3.0 Flash/Pro (1M+ token context), Imagen 3 | [Get a Key](https://aistudio.google.com/) |
| **Groq** | Ultra-fast inference for open models (Llama 3.3 70B, Mixtral) | [Get a Key](https://console.groq.com/keys) |
| **OpenRouter** | Gateway to 100+ specialized models via single key | [Get a Key](https://openrouter.ai/keys) |
| **Brave Search** | Deep web research for the agent | [Get a Key](https://brave.com/search/api/) |
| **HuggingFace** | Read Access Token for gated models (Llama 3, Gemma). Add in **Settings → Secrets** | [Get a Token](https://huggingface.co/settings/tokens) |

**Security notes:**
- All API keys are stored in the **macOS Keychain** (AES-256 encrypted at rest), never in plaintext config files.
- **Saving a key does NOT automatically grant it** to the ThinClaw agent. After saving, you must toggle **"Grant Access"** for each key in Settings › Secrets. This ensures keys are only exposed to the agent when you explicitly permit it.
- Deleting a key automatically revokes its grant.

---

## 🔌 MCP Server (External Tool Integration)

ThinClaw Desktop supports connecting to a custom **FastAPI MCP server** to extend the agent with remote tools (finance APIs, news feeds, domain-specific capabilities).

- Configure in **Settings → MCP Server**.
- Enter your server's **Base URL** and optional **JWT Bearer Token**.
- Toggle **Enable MCP Sandbox** to allow the agent to discover and execute MCP tools via Rhai scripts during conversations.
- Use the **Test Connection** button to verify reachability before enabling.

---

## 📁 Project Structure Reference

```
apps/desktop/
├── frontend/               # React + TypeScript + Vite
│   ├── src/
│   │   ├── components/     # React components
│   │   ├── hooks/          # Custom React hooks
│   │   ├── lib/            # Utilities and bindings
│   │   └── index.css       # Design system (Tailwind + shadcn tokens)
│   ├── tailwind.config.cjs
│   └── vite.config.ts
├── backend/                # Rust + Tauri v2
│   ├── src/
│   │   ├── engine/         # InferenceEngine trait + implementations
│   │   ├── openclaw/       # ThinClaw gateway integration
│   │   ├── rig_lib/        # Rig agent, orchestrator, tools
│   │   ├── hf_hub.rs       # HuggingFace Hub model discovery
│   │   ├── sidecar.rs      # SidecarManager
│   │   ├── chat.rs         # Chat streaming commands
│   │   └── ...
│   ├── bin/                # Sidecar binaries (downloaded)
│   ├── scripts/            # download_ai_binaries.js, setup_chromium.sh
│   ├── mcp-tools crate     # MCP crate (sandbox, tools, skills)
│   ├── Cargo.toml          # Features: llamacpp, mlx, vllm, ollama
│   └── tauri.conf.json     # Tauri configuration
├── scripts/                # Top-level build scripts
│   ├── generate_tauri_overrides.sh  # Engine-specific bundle config
│   ├── setup_llama.sh               # Cross-platform llama-server download
│   └── setup_uv.sh                  # uv Python manager download (MLX/vLLM)
├── documentation/          # Architecture docs
└── package.json            # npm scripts (dev, build, test, setup)
```

---

## 🔧 Troubleshooting

### All Platforms
- **Dependencies Error**: If a binary fails to start, ensure you've run `npm run setup:all` to fetch the required shared libraries.
- **TTS (Piper)**: The `download_ai_binaries.js` script does not yet auto-download the TTS sidecar. You must manually place the Piper binary at `backend/bin/tts-{target-triple}` (e.g., `tts-aarch64-apple-darwin` for macOS ARM64). The sidecar is registered as `bin/tts` in `tauri.conf.json`.
- **Engine Mismatch**: If you see errors about missing models, ensure you're running with the correct engine feature. Check the `ActiveEngineChip` badge in the Model Browser — it shows the compiled engine.

### macOS
- **"App is Damaged" / Gatekeeper**: If binaries won't run due to security settings:
  ```bash
  xattr -dr com.apple.quarantine backend/bin/
  ```
- **Metal Performance**: If inference feels slow, verify that your model is fully loaded into GPU layers (configurable in Settings).
- **MLX setup banner not appearing**: This only shows for `--features mlx` builds. With the default `llamacpp` feature, the banner is hidden.

### Linux
- **Missing shared libraries**: Ensure `libwebkit2gtk-4.1` and related packages are installed (see Prerequisites).
- **CUDA not detected**: Verify `nvidia-smi` works and CUDA toolkit is on `PATH`.
- **vLLM bootstrap fails**: Ensure you have sufficient disk space (~1 GB) and that `python3` is available on `PATH` (the `uv` tool needs to detect it for venv creation).

### Windows
- **WebView2 missing**: Download from [developer.microsoft.com](https://developer.microsoft.com/microsoft-edge/webview2/).
- **Build tools**: Ensure "Desktop development with C++" workload is installed via Visual Studio Installer.
- **llama-server crash**: Windows builds use the AVX2 variant by default. If your CPU doesn't support AVX2, you'll need to manually download the non-AVX build.

---

## 📜 License
Distributed under the **GNU General Public License v3.0** (Strong Copyleft). See `License.md` for full terms and attribution requirements.
