# HuggingFace Hub Model Discovery

> **Status:** Implemented · **Last updated:** 2026-02-24

## Overview

Scrappy now includes **live HuggingFace Hub model discovery** directly in the
Model Management settings tab. Users can search the entire HF model catalogue,
filter results by the active inference engine, preview file details
(quantization types, directory sizes), and download models — all without
leaving the app.

The system is **engine-aware by design**: a single build of Scrappy compiles
with exactly one inference engine (determined by Cargo feature flags), and the
discovery pipeline automatically adapts search tags, file parsing, and download
strategy to match.

---

## Architecture at a Glance

```
┌───────────────────────────────────────────────────────────────────────┐
│                        Frontend (React/TS)                           │
│                                                                      │
│  ModelBrowser ─── tabs ──→  Library (existing)                       │
│                         └─→  Discover (HFDiscovery.tsx) ◄────────┐   │
│                                                                  │   │
│  ModelContext  ◄── discoveryState (persistent across tab switch) ─┘   │
│      │                                                               │
│      ├── downloadHfFiles(repoId, files, destSubdir)                  │
│      ├── downloading / progress (throttled ~4fps)                    │
│      └── engineInfo (from get_active_engine_info)                    │
└────────┬─────────────────────────────────────────────────────────────┘
         │  Tauri IPC (invoke)
┌────────▼─────────────────────────────────────────────────────────────┐
│                     Backend (Rust · hf_hub.rs)                       │
│                                                                      │
│  discover_hf_models(query, engine, limit)                            │
│      → GET https://huggingface.co/api/models?search=…&tags=…         │
│      → Parses JSON → Vec<HfModelCard>                                │
│                                                                      │
│  get_model_files(repo_id, engine)                                    │
│      → GET https://huggingface.co/api/models/{repo_id}/tree/main     │
│      → GGUF: regex-extracts quant types, detects mmproj              │
│      → MLX/vLLM: lists all model files, skips non-model assets       │
│      → Returns ModelDownloadInfo                                     │
│                                                                      │
│  download_hf_model_files(repo_id, files, dest_subdir)                │
│      → Streams each file via reqwest + futures::StreamExt            │
│      → Emits dual download_progress events (per-file + per-repo)     │
│      → Saves to {app_data}/models/LLM/{sanitized_repo_id}/          │
└──────────────────────────────────────────────────────────────────────┘
```

---

## Engine ↔ Discovery Mapping

Each inference engine maps to a specific HuggingFace tag used to filter search
results. This ensures users only see models compatible with their build:

| Engine        | Cargo Feature   | HF Tag   | Model Format       | Download Strategy        |
|---------------|-----------------|----------|--------------------|--------------------------|
| **llama.cpp** | `llamacpp`      | `gguf`   | Single `.gguf` file | Pick one quant + optional mmproj |
| **MLX**       | `mlx`           | `mlx`    | Directory (safetensors) | Download all model files   |
| **vLLM**      | `vllm`          | `awq`    | Directory (safetensors) | Download all model files   |
| **Ollama**    | `ollama`        | `gguf`   | Single `.gguf` file | Pick one quant + optional mmproj |

The mapping is defined in `backend/src/hf_hub.rs → engine_to_hf_tag()`:

```rust
fn engine_to_hf_tag(engine: &str) -> Option<&'static str> {
    match engine {
        "llamacpp" => Some("gguf"),
        "mlx"      => Some("mlx"),
        "vllm"     => Some("awq"),
        "ollama"   => Some("gguf"),
        _          => None,
    }
}
```

---

## Backend API (`hf_hub.rs`)

### Types

```rust
/// A model card from HF Hub search
HfModelCard { id, author, name, downloads, likes, tags, last_modified, gated }

/// A single file within a repo
HfFileInfo { filename, size, size_display, quant_type, is_mmproj }

/// Parsed file tree for an entire repo
ModelDownloadInfo { repo_id, is_multi_file, files, mmproj_file, total_size, total_size_display }
```

### Commands

#### `discover_hf_models`

```
discover_hf_models(query: String, engine: String, limit: Option<u32>)
  → Result<Vec<HfModelCard>, String>
```

- Calls `GET https://huggingface.co/api/models?search={query}&tags={tag}&sort=downloads&direction=-1&limit={limit}`
- Results are sorted by download count (most popular first)
- Limit is capped at 100
- Handles rate limiting (`429`) with user-friendly error

#### `get_model_files`

```
get_model_files(repo_id: String, engine: String)
  → Result<ModelDownloadInfo, String>
```

- Fetches the file tree from `GET /api/models/{repo_id}/tree/main`
- **GGUF mode** (`llamacpp`, `ollama`):
  - Regex-extracts quantization types from filenames (e.g. `Q4_K_M`, `IQ3_XXS`, `F16`, `UD-Q5_K_XL`)
  - Detects `mmproj` files (multimodal vision projectors) and surfaces them separately
  - Sorts quants by file size ascending (smallest first)
- **Directory mode** (`mlx`, `vllm`):
  - Collects all meaningful files (safetensors, JSON configs, tokenizer files)
  - Skips non-model assets (`.md`, `.jpg`, `.png`, `.gif`, `.gitattributes`, `.gitignore`)
  - Sets `is_multi_file = true`

#### `download_hf_model_files`

```
download_hf_model_files(repo_id: String, files: Vec<String>, dest_subdir: Option<String>)
  → Result<String, String>
```

- Downloads files to `{app_data}/models/LLM/{sanitized_repo_id}/`
- Uses only a **connect timeout** (30s), no total request timeout — important for multi-GB safetensor files
- Pre-fetches content lengths via HEAD requests for accurate overall progress calculation
- Emits **dual `download_progress` events** per chunk:
  1. **Repo-level** (keyed by `repo_id`): drives the "Download All" overall progress bar
  2. **File-level** (keyed by `filename`): drives individual GGUF quant row progress bars
- Progress events are throttle-gated (≥0.1% change OR ≥150ms since last emit)
- Preserves directory structure for multi-file downloads (e.g. subdirectories within the repo)

### Authentication

All HF API requests use the **SecretStore** to inject a `Bearer` token if the
user has provided a HuggingFace access token. This is required for:
- Gated models (Llama-3, Gemma, etc.)
- Higher rate limits on the HF API

```rust
async fn build_hf_client(app: &AppHandle) -> Result<reqwest::Client, String> {
    // Reads from SecretStore → Keychain-backed in-memory cache
    if let Some(store) = app.try_state::<SecretStore>() {
        if let Some(token) = store.huggingface_token() {
            // Inject Authorization: Bearer {token}
        }
    }
}
```

---

## Frontend UX (`HFDiscovery.tsx`)

### Component Architecture

```
ModelBrowser
├── Tab: "Library" — existing curated model list + local models
└── Tab: "Discover" — HFDiscovery component
    ├── Search bar (debounced, 300ms)
    ├── Engine info chip (shows active engine + HF tag)
    ├── Result cards (model name, author, downloads, likes, gated badge)
    │   └── Expanded view (fetched on click):
    │       ├── GGUF: Quant picker grid (Q4_K_M, Q5_K_S, Q8_0, etc.)
    │       │   └── Per-quant size, download button, progress bar
    │       │   └── mmproj checkbox for vision models
    │       └── MLX/vLLM: File list + total size + "Download All" button
    │           └── Overall progress bar during download
    └── Downloading models pinned to top of results
```

### State Management

Discovery state is stored in `ModelContext.discoveryState` (not component-local
state) so it **survives tab switches**. This includes:

```typescript
interface DiscoveryState {
    searchQuery: string;
    results: HfModelCard[];
    hasSearched: boolean;
    expandedModel: string | null;
    downloadingFiles: Set<string>;
    repoProgress: Record<string, RepoProgressInfo>;
}
```

The `HFDiscovery` component uses a `display: none` keep-alive pattern — when
the user switches away from the Discover tab and returns, the component remounts
instantly without re-fetching. If file info was already loaded for a model, it's
served from a local `fileInfoCache` (React state within the component).

### Smart Sorting

Results are automatically re-sorted:
1. **Downloading models** pinned to top (priority 2)
2. **Already-downloaded models** shown next (priority 1) — detected by comparing
   sanitized repo IDs against `localModels[]` paths
3. **Other results** in original order (by HF download count)

### Download Integration

Downloads from HF Hub flow through `ModelContext.downloadHfFiles()`, which:
1. Tracks the download key in global `downloading` state
2. Invokes `download_hf_model_files` via Tauri IPC
3. Progress updates arrive via the throttled `download_progress` event listener
   (buffered in refs, flushed at ~4fps)
4. On completion, triggers `refreshModels()` to reload the local model list

---

## Engine Infrastructure (`engine/mod.rs`)

### The `InferenceEngine` Trait

All four engines implement a common trait that provides engine-agnostic
lifecycle management:

```rust
#[async_trait]
pub trait InferenceEngine: Send + Sync {
    async fn start(&self, model_path: &str, context_size: u32, options: EngineStartOptions)
        -> Result<(u16, String), String>;
    async fn stop(&self) -> Result<(), String>;
    async fn is_ready(&self) -> bool;
    fn base_url(&self) -> Option<String>;
    fn model_id(&self) -> Option<String>;
    fn max_context(&self) -> Option<u32>;
    fn display_name(&self) -> &'static str;
    fn engine_id(&self) -> &'static str;
    fn uses_single_file_model(&self) -> bool;
    fn hf_search_tag(&self) -> &'static str;
}
```

### `EngineManager`

The `EngineManager` holds the active engine instance behind a `tokio::Mutex`:

```rust
pub struct EngineManager {
    pub engine: tokio::sync::Mutex<Option<Box<dyn InferenceEngine>>>,
    pub app_data_dir: PathBuf,
}
```

Registered as Tauri managed state on startup. Commands like `start_engine`,
`stop_engine`, and `is_engine_ready` operate on the engine through this
manager.

### Engine Implementations

| File                  | Engine    | Notes |
|-----------------------|-----------|-------|
| `engine_llamacpp.rs`  | llama.cpp | Wraps the sidecar `llama-server` binary. Uses GGUF quants. |
| `engine_mlx.rs`       | MLX       | Spawns `mlx_lm.server` via a Python venv. Requires first-launch bootstrap. |
| `engine_vllm.rs`      | vLLM      | Spawns vLLM server via Python venv. Requires NVIDIA CUDA GPU. |
| `engine_ollama.rs`    | Ollama    | Connects to external Ollama daemon. Uses GGUF internally via `ollama pull`. |

### `get_active_engine_info` Command

Exposes the compile-time engine selection to the frontend:

```rust
pub fn get_active_engine_info() -> EngineInfo {
    #[cfg(feature = "mlx")]
    return EngineInfo { id: "mlx", display_name: "MLX (Apple Silicon)", hf_tag: "mlx", ... };

    #[cfg(feature = "vllm")]
    return EngineInfo { id: "vllm", display_name: "vLLM (CUDA)", hf_tag: "awq", ... };

    #[cfg(feature = "llamacpp")]
    return EngineInfo { id: "llamacpp", display_name: "llama.cpp", hf_tag: "gguf", ... };

    #[cfg(feature = "ollama")]
    return EngineInfo { id: "ollama", display_name: "Ollama", hf_tag: "gguf", ... };
}
```

This `EngineInfo` is consumed by:
- `HFDiscovery.tsx` — to set the correct search tag and choose single-file vs directory UI
- `ActiveEngineChip.tsx` — to show the engine badge in the Model Browser header
- `EngineSetupBanner.tsx` — to detect if MLX/vLLM first-launch bootstrap is needed
- `ServerSettings` (in `SettingsPages.tsx`) — to show the correct inference status

---

## Engine-Specific Setup

### `EngineSetupBanner.tsx`

Python-based engines (MLX, vLLM) require a first-launch bootstrap step:
1. The `get_engine_setup_status` command checks if the Python venv exists
2. If `needs_setup` is true, a banner appears above the Model Browser
3. Clicking "Set Up Now" invokes `setup_engine`, which:
   - Creates a Python venv (`mlx-env/` or `vllm-env/`)
   - Installs `mlx_lm` or `vllm` via `pip`
   - Emits `engine_setup_progress` events for UI progress tracking

### `ActiveEngineChip.tsx`

A small status badge shown in the Model Browser header. Uses per-engine accent
colours:
- **llama.cpp** → primary theme colour
- **MLX** → amber
- **vLLM** → emerald
- **Ollama** → violet

---

## How Models Are Discovered Per Engine

### llama.cpp / Ollama (GGUF)

```
User searches "llama 3" → HF API filtered by tag:gguf
 ↓
Results show repos like "unsloth/Llama-3.1-8B-Instruct-GGUF"
 ↓
User clicks → get_model_files parses *.gguf files:
  ├── Regex extracts quant types: Q4_K_M, Q5_K_S, Q8_0, F16, etc.
  ├── mmproj files detected and offered as optional checkbox
  └── Files sorted by size (smallest quant first)
 ↓
User picks a quant (e.g. Q4_K_M, 4.9 GB)
 ↓
Single file downloaded → {app_data}/models/LLM/unsloth_Llama-3.1-8B-Instruct-GGUF/
 ↓
Model appears in Library tab, ready to select
```

### MLX (Apple Silicon safetensors)

```
User searches "gemma 3" → HF API filtered by tag:mlx
 ↓
Results show repos like "mlx-community/gemma-3-4b-it"
 ↓
User clicks → get_model_files lists all model files:
  ├── config.json, tokenizer.json, tokenizer_config.json
  ├── model-00001-of-00002.safetensors (2.1 GB)
  ├── model-00002-of-00002.safetensors (1.8 GB)
  └── Total: 4.2 GB shown
 ↓
"Download All" button downloads entire directory
 ↓
Files saved preserving structure → {app_data}/models/LLM/mlx-community_gemma-3-4b-it/
 ↓
EngineManager.start() loads the directory via mlx_lm.server
```

### vLLM (AWQ / CUDA safetensors)

```
User searches "mistral" → HF API filtered by tag:awq
 ↓
Results show repos like "TheBloke/Mistral-7B-AWQ"
 ↓
User clicks → get_model_files lists all model files:
  ├── config.json, tokenizer files
  ├── model.safetensors (4.5 GB)
  └── Total: 4.8 GB shown
 ↓
"Download All" downloads entire directory
 ↓
Files saved → {app_data}/models/LLM/TheBloke_Mistral-7B-AWQ/
 ↓
EngineManager.start() loads via vLLM server
```

---

## Curated Model Library (Fallback)

In addition to live HF Hub discovery, Scrappy maintains a **curated model
library** (`frontend/src/lib/model-library.ts`) with hand-picked models
categorised by type:

| Category      | Examples                                                |
|---------------|---------------------------------------------------------|
| **LLM**       | Llama 3.3 70B, Gemma 3 27B, Qwen 2.5, Phi-4, Mistral  |
| **Embedding** | Nomic Embed v2, mxbai Embed Large v1                   |
| **STT**       | Whisper Large v3, Whisper Medium                        |
| **TTS**       | Piper Lessac, Piper Amy                                 |
| **Image Gen** | FLUX.1 Dev, FLUX.1 Schnell, SD3.5 Medium               |
| **Cloud**     | Anthropic, OpenAI, Gemini, Groq, xAI (via API key)     |

Each entry includes pre-configured download URLs, size info, VRAM requirements,
and recommended RAM minimums. This library is shown in the "Library" tab and
provides a quick-start experience for users who don't want to search HF Hub
manually.

---

## File Inventory

| File                                            | Purpose                                    |
|-------------------------------------------------|--------------------------------------------|
| `backend/src/hf_hub.rs`                         | Backend: HF API search, file parsing, download streaming |
| `backend/src/engine/mod.rs`                     | Engine trait, EngineManager, `get_active_engine_info` |
| `backend/src/engine/engine_llamacpp.rs`         | llama.cpp engine implementation             |
| `backend/src/engine/engine_mlx.rs`              | MLX engine implementation                   |
| `backend/src/engine/engine_vllm.rs`             | vLLM engine implementation                  |
| `backend/src/engine/engine_ollama.rs`            | Ollama engine implementation                |
| `frontend/src/components/settings/HFDiscovery.tsx`  | HF Hub search and download UI           |
| `frontend/src/components/settings/ModelBrowser.tsx`  | Model management tabs (Library + Discover) |
| `frontend/src/components/settings/ActiveEngineChip.tsx` | Engine status badge component         |
| `frontend/src/components/settings/EngineSetupBanner.tsx` | First-launch bootstrap banner        |
| `frontend/src/components/model-context.tsx`      | State management: downloads, progress, engine info |
| `frontend/src/lib/model-library.ts`              | Curated model definitions (static library) |

---

## Server & Memory Integration

The **Settings → Server & Memory** page now correctly recognises all inference
engines:

| Engine          | Status Source                       | Display Name               |
|-----------------|-------------------------------------|----------------------------|
| **llama.cpp**   | `SidecarManager.chat_process`       | "llama.cpp Inference"      |
| **MLX**         | `EngineManager.is_ready()`          | "MLX (Apple Silicon) Inference" |
| **vLLM**        | `EngineManager.is_ready()`          | "vLLM (CUDA) Inference"   |
| **Ollama**      | `EngineManager.is_ready()`          | "Ollama Inference"         |
| **Cloud Only**  | N/A                                 | "No local inference engine" |

The restart button dispatches to the correct backend command:
- **llama.cpp**: `start_chat_server` (sidecar restart)
- **MLX / vLLM / Ollama**: `stop_engine` + `start_engine` (EngineManager)

---

## Performance Notes

- **Search debouncing**: 300ms debounce on keystrokes prevents excessive HF API calls
- **Download progress throttling**: Events buffered in `useRef`, flushed at ~4fps via `setInterval` + `requestAnimationFrame` to prevent React render thrashing
- **Context splitting**: `ModelContext` uses a two-context architecture (`ModelStateContext` + `ModelProgressContext`) so download progress updates don't cause the stable model list to re-render
- **File info caching**: Once a repo's file tree is fetched, it's cached in component state and survives re-expansion of the same card
- **Keep-alive tabs**: Both Library and Discover tabs use `display: none` when inactive rather than unmounting, preserving scroll position and state
