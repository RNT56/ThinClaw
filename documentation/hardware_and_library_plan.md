# Feature Plan: Hardware Detection & Model Library

## 1. Hardware Detection & Recommendation
**Goal:** Automatically detect device capabilities to recommend suitable models on first startup.

### Technical Approach
- **Backend (Rust):**
  - Integrate `sysinfo` crate to query system metrics.
  - Expose a new command `get_system_specs` returning:
    - Total RAM (GB)
    - CPU Brand/Cores
    - GPU Information (VRAM if accessible via `sysinfo` or `nvidia-smi`/`rocm-smi` fallback, though `sysinfo` is decent for total memory).
    - Platform/OS.

- **Frontend (React):**
  - On app mount, check if `localModels` list is empty.
  - If empty, invoke `get_system_specs`.
  - Compare specs against a "Recommendation Logic":
    - **Entry Level (<8GB RAM):** Recommend 3B/7B Q4 models.
    - **Mid Range (16GB RAM):** Recommend 7B/8B Q5/Q6 or 10-12B models.
    - **High End (32GB+ RAM):** Recommend 14B+ or larger quants.
  - Display a "Hardware Detected" toast or modal with the recommendation and a one-click download.

## 2. Curated Model Library
**Goal:** Expand the hardcoded model list into a rich, structured library.

- **Structure:**
  - Create a `ModelDefinition` interface:
    - `id`: Unique ID
    - `name`: Display name
    - `description`: Usage context (coding, chat, roleplay)
    - `family`: (Llama 3, Qwen 2, etc.)
    - `parameters`: Size (7B, 14B)
    - `quantization`: (Q4_K_M, Q8_0)
    - `vram_required_gb`: Estimated VRAM
    - `filename`: GGUF filename
    - `download_url`: Direct link
    - `tags`: ["coding", "fast", "multilingual"]

- **Integration:**
  - Replace the static `RECOMMENDED_MODELS` array in `model-context.tsx` with this structured registry.
  - Update `ModelBrowser` to support filtering/searching if the list grows large.

## 3. Realtime Download Progress
**Current State Analysis:**
- Backend `download_model` already emits `download_progress` events.
- Frontend `ModelContext` listens for these events and updates a `downloading` state record.
- **Issue:** The visual feedback might be subtle or broken if the event payload isn't matching perfectly or if the UI isn't re-rendering.
- **Plan:**
  - Verify event payload structure matching `DownloadEvent` interface.
  - Ensure the `Progress` component in `ModelBrowser` receives distinct updates.
  - Add "Status" text (Connecting, Downloading X%, Finalizing) for better UX.

## 4. Fix Model Deletion
**Issue:** Clicking delete does "nothing".
- **Potential Causes:**
  - `delete_local_model` backend error (permission, file not found).
  - Frontend not refreshing the list after deletion.
  - Event propagation failure.
- **Fix:**
  - Add explicit error logging in frontend `deleteModel`.
  - Ensure `refreshModels()` is awaited after delete command.
  - Verify `filename` passed matches filesystem exactly.

## 5. Implementation Roadmap
1.  **Add `sysinfo` dependency** to `src-tauri`.
2.  **Implement `get_system_specs`** in `system.rs` (new module).
3.  **Fix Delete Functionality** before adding new features.
4.  **Refine Download UI** to be robust.
5.  **Implement Recommendation Logic** and new **Model Registry**.
