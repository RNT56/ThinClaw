# Model Update Sync Service - Remote Server Spec

This document defines the specification for a FastAPI-based Model Update Sync Service and its integration within the Scrappy application.

## 1. System Overview
**Purpose**: Automatically fetch and sync the latest model metadata and variants from external sources (Hugging Face Hub, GitHub) to the Scrappy local model library.
**Communication**: Tauri (Frontend/IPC) <-> FastAPI (Remote API).

---

## 2. FastAPI API Design

### 2.1 API Versioning
All endpoints should be prefixed with `/api/v1`.

### 2.2 Endpoints

| Endpoint | Method | Description |
| :--- | :--- | :--- |
| `/models` | GET | Returns the full catalog of available models (Extended Metadata). |
| `/models/updates` | GET | Returns a list of model IDs with newer versions than provided timestamps. |
| `/models/{model_id}` | GET | Returns detailed metadata for a specific model. |
| `/models/{model_id}/variants` | GET | Returns all available quantizations/variants for a model. |
| `/standard-assets` | GET | Returns the list of standard assets (VAE, CLIP, etc.) required for various model families. |
| `/health` | GET | Basic health check to verify server availability. |

### 2.3 Error Responses
The API should return standard HTTP status codes. Error bodies should follow this format:
```json
{
  "error": "Short error code",
  "message": "Detailed human-readable explanation"
}
```

### 2.4 Data Structures

The models should follow the `ExtendedModelDefinition` used in the Scrappy frontend to ensure seamless integration.

#### Model Snapshot (`/models`)
```json
[
  {
    "id": "gemma-3-12b-it-qat",
    "name": "Gemma 3 12B IT QAT",
    "description": "Google's Gemma 3 12B, Quantization-Aware Training. Multimodal.",
    "family": "Gemma",
    "category": "LLM",
    "version": "1.2.0",
    "last_updated": 1708080000,
    "tags": ["Multimodal", "Gemma", "Quality"],
    "template": "gemma",
    "recommendedForAgent": true,
    "gated": false,
    "mmproj": {
      "filename": "mmproj-model-f16-12B.gguf",
      "url": "https://huggingface.co/...",
      "size": "1.8 GB"
    },
    "variants": [
      {
        "name": "Q4_0",
        "filename": "gemma-3-12b-it-q4_0.gguf",
        "url": "https://huggingface.co/...",
        "size": "7.6 GB",
        "vram_required_gb": 10,
        "recommended_min_ram": 12
      }
    ],
    "components": []
  }
]
```

#### Update Check Request (`/models/updates`)
**Query Parameters**: `?last_sync=1708000000` (Unix Timestamp)
**Response**:
```json
{
  "updates": [
    {
      "id": "gemma-3-12b-it-qat",
      "new_version": "1.2.1",
      "critical": true,
      "changelog": "Fixed quantization artifacts in Q4_0 variant."
    }
  ]
}
```

---

## 3. Data Flow & Integration

### 3.1 Trigger (Tauri → FastAPI)
The Tauri app initiates a check:
1.  **On Launch**: If `settings.auto_check_updates` is enabled.
2.  **Explicitly**: When the user clicks "Refresh" in the `ModelBrowser`.

### 3.2 Local Persistence (Tauri SQLite)
To support version tracking and offline access, Scrappy should implement a `models_catalog` table in the local SQLite database (`scrappy.db`).

```sql
CREATE TABLE IF NOT EXISTS models_catalog (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    metadata JSON NOT NULL, -- Full ExtendedModelDefinition
    local_version TEXT,
    remote_version TEXT,
    last_checked_at INTEGER,
    status TEXT -- 'installed', 'outdated', 'unavailable'
);
```

### 3.3 Auto-Download Workflow
1.  **Detection**: Tauri fetches `/models/updates`.
2.  **Comparison**: Check `local_version` from `models_catalog` against `new_version`.
3.  **Notification**: If `outdated`, show update available badge in `ModelBrowser`.
4.  **Action**: User (or auto-sync) triggers:
    ```typescript
    invoke("download_model", {
        url: remoteVariant.url,
        filename: `${category}/${sanitizedName}/${remoteVariant.filename}`
    });
    ```
5.  **Cleanup**: Upon successful download, Tauri deletes the old version and updates `local_version`.

---

## 4. Implementation Details

### 4.1 FastAPI Implementation (Python)
- **Framework**: `FastAPI`
- **Dependencies**: `requests`, `python-dotenv`, `pydantic`.
- **Logic**: A background task or cron job periodically scrapes Hugging Face (using `huggingface_hub` library) to update the JSON catalog.

### 4.2 Scraper Logic (Remote Side)
- **Hugging Face**: Use the `huggingface_hub` Python package to list files in a repository. 
  - Look for `.gguf` for LLMs.
  - Look for `.safetensors` or `.sft` for Diffusion.
  - Parse the README or use Hub metadata to extract model family and templates.
- **GitHub**: Use the GitHub API `/releases/latest` to fetch pre-compiled binaries (e.g., for `whisper.cpp` models).
- **Caching**: The remote server MUST cache the scraped metadata in its own database (e.g., PostgreSQL or Redis) to avoid hitting HF/GitHub rate limits.

### 4.3 Auth & Security
- **Gated Models**: If `model.gated` is true, Tauri must pass the `HF_TOKEN` (retrieved from `commands.getHfToken()`) to the `download_model` Rust sidecar, which injects the `Authorization` header.
- **Verification**: Remote server should sign the catalog with an Ed25519 key; Tauri verifies the signature to prevent MitM attacks.

### 4.3 Handling Complex Models (Diffusion)
For models with `components` (VAE, Text Encoders):
- The FastAPI response must include the full list of required components.
- Tauri's `model_manager.rs` should continue to support batch downloading these into the correct hierarchy (e.g., `models/Diffusion/ModelName/vae/`).

---

### 4.4 Sync Protocol Details
- **Full Refresh**: `GET /models` should return the absolute current state.
- **Incremental Sync**: `GET /models/updates?last_sync=TS` allows Tauri to only fetch changes if it already has a local cache.
- **Environment**: For production, the API MUST be served over HTTPS. For local testing, Scrappy will support `http://localhost:8000`.

### 4.5 Development Environment Setup
To facilitate development, the server should include a `seed_data.py` script that populates the database with a few standard models (Gemma, Llama, Flux) fetched from Hugging Face.

---

## 5. Development Checklist

- [ ] **Phase 1: API Server**
  - [ ] Initialize FastAPI project.
  - [ ] Implement Hugging Face scraper logic for GGUF/Safetensors.
  - [ ] Define Pydantic models for `ExtendedModelDefinition`.
- [ ] **Phase 2: Tauri Refactor**
  - [ ] Add `models_catalog` table to SQLite migrations.
  - [ ] Modify `ModelBrowser.tsx` to fetch from FastAPI instead of hardcoded constant.
  - [ ] Implement "Update Available" UI states.
- [ ] **Phase 3: Automation**
  - [ ] Add `auto_download_updates` toggle in Settings.
  - [ ] Implement background sync worker in Rust.
