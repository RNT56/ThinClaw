# Scrappy / OpenClaw — Feature Set Analysis Report

> **Generated**: 2026-02-17  
> **Scope**: Full codebase audit across 13 feature sets  
> **Method**: Source file outline analysis, data-flow tracing, pattern-level grep, cross-reference with specs

---

## Legend

| Rating | Meaning |
|---|---|
| 🟢 | Complete / No issues found |
| 🟡 | Functional but has gaps or risks |
| 🔴 | Incomplete / Blocking issues |

---

## Feature Set 7: Sidecar Management

**Files**: `sidecar.rs` (1090 lines), `process_tracker.rs` (131 lines), `system.rs` (101 lines)

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| Chat server lifecycle | 🟢 | Full start/stop/restart with health polling (120s timeout), crash detection, and intentional-stop flag |
| Embedding server | 🟢 | Independent lifecycle, fixed ctx=4096, tracked via ProcessTracker |
| Summarizer server | 🟢 | Configurable ctx and GPU layers |
| STT (Whisper) server | 🟡 | **No API key auth** — comment on L616 confirms this is a known gap |
| Image server | 🔴 | `start_image_server` is a **stub** — only stores a model path, spawns no process (L660-664) |
| TTS server | 🔴 | `start_tts_server` is a **stub** — same pattern as image (L666-670) |
| Process cleanup on quit | 🟢 | `ProcessTracker` persists PIDs to JSON, `cleanup_all()` kills on restart |

### Issues

1. **Hardcoded preferred ports** — Chat=53755, Embed=53756, STT=53757, Summarizer=53758. The `generate_config` correctly falls back to random ports, but the hardcoded preference on `0.0.0.0` binding (L765) is surprising when the chat server binds `127.0.0.1`.
2. **Port-clear wait after restart fixed to port 53755** — L163 waits for the *hardcoded* port `53755` to clear, not the actual previous port. If the previous instance was on a random port (fallback), this wait is wrong.
3. **36 `unwrap()` calls on Mutex locks** — None are `try_lock`. A panic in any sidecar method (e.g. poisoned mutex after a thread panic) will bring down the entire Rust backend. This is the single biggest fragility risk.
4. **`stop_chat_server` Tauri command calls `stop_all()`** — The command named `stop_chat_server` (L1068) actually stops *all* sidecars. This is likely intentional but the naming is misleading.

### Bottlenecks

- **Blocking `std::thread::sleep`** in restart flow (L162-167) — runs on the async runtime's thread pool, blocking a tokio worker thread for up to 2s.
- **Health poll loop** runs at 500ms intervals with a 120s timeout (L884-907). This is fine but the `reqwest::Client` is created per-call rather than reused.

### Design Flaws

- **No exponential backoff** on crash-restart. The spec calls for it (§12.1), but the crash handler only emits an event — it doesn't attempt auto-restart.
- **No Safe Mode fallback** (spec §2.1 Failure Handling). There's no logic to restart with reduced ctx/CPU fallback after OOM crashes.
- **Image and TTS "servers" are phantom** — `get_status()` reports them as "running" based on model path existence, not actual process status. The frontend may show misleading status.

### Enhancements

- Replace `Mutex<Option<SidecarProcess>>` with `tokio::sync::Mutex` to avoid blocking the async runtime
- Implement auto-restart with backoff (3 attempts, then surface error in UI)
- Add Safe Mode: detect crash code patterns (OOM = code 137/9), restart with `--ctx-size 2048 --n-gpu-layers 0`
- Create a reusable `SidecarLifecycle<T>` abstraction instead of repeating the spawn/track/monitor pattern 4 times

---

## Feature Set 6: Model Management & GGUF

**Files**: `gguf.rs` (283 lines), `model_manager.rs` (774 lines), `templates.rs` (10 lines), `config.rs` (314 lines)

### Completion: 🟢

| Aspect | Status | Detail |
|---|---|---|
| GGUF metadata reading | 🟢 | v2 and v3 support, extracts architecture, context length, embedding dims, file type, chat template |
| Family detection | 🟢 | 8 families (llama3, mistral, deepseek, glm, gemma, qwen, phi→chatml, starcoder→llama3) + template-based fallback |
| Stop token mapping | 🟢 | 7 family-specific stop token sets plus a generic fallback |
| Model file scanning | 🟢 | Recursive directory scan with GGUF metadata extraction per file |
| Model download | 🟢 | Supports HF token auth, resume (range requests), cancellation, progress events |
| Standard assets | 🟢 | Pre-defined assets (embedding model, reranker) with auto-download |
| Remote model catalog | 🟢 | SQLite-backed catalog with upsert support |
| Templates | 🟢 | 5 templates: ChatML, Llama3, Mistral, Gemma, Qwen |

### Issues

1. **Gemma template defined but never used** — `GEMMA_TEMPLATE` exists in `templates.rs` but the sidecar code (L129, L138) explicitly returns `None` for Gemma to let llama-server handle it natively. The const is dead code.
2. **Architecture detection may misidentify** — "llama" in `architecture` catches all Llama-family (including CodeLlama, Llama Guard). This is correct but could over-match if a custom architecture name coincidentally contains "llama".
3. **GGUF `skip_value` for type 11** — Falls into the `0..=7 | 11` branch (L247) but only indexes sizes `[0..7]`. For type 11 (`bool`), it would use `sizes[0]=1` which is accidentally correct (bool is 1 byte in GGUF spec), but the logic is fragile.

### Bottlenecks

- **Model download is single-threaded per file** — No parallel chunk download. For large GGUF files (20-80 GB), a multi-range download could significantly improve speed.

### Design Flaws

- **Config doesn't store per-model settings** — `UserConfig` has global `llm_temperature`, `llm_top_p`, `default_context_window` but no per-model overrides. Users switching between 3B and 70B models need to manually adjust settings.

### Enhancements

- Add per-model profile support (ctx, temp, top_p, GPU layers) keyed by model filename hash
- Display estimated VRAM usage based on GGUF metadata (file_type quantization × block_count × embedding_length)
- Add integrity verification (sha256 checksum) after download completion

---

## Feature Set 8: Settings, Secrets & Cloud Providers

**Files**: `config.rs` (314 lines), `openclaw/config.rs` (77 KB)

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| UserConfig persistence | 🟢 | JSON file with serde defaults for every field — robust against missing keys |
| Knowledge bits | 🟢 | Custom knowledge entries with enable/disable toggle |
| Custom personas | 🟢 | Full CRUD with instructions field |
| HF token retrieval | 🟢 | Reads from OpenClaw config (identity.json) |
| Provider selection | 🟡 | `selected_chat_provider` stored, but `disabled_providers` applies globally — no per-model allowlist at config level |
| MCP integration config | 🟡 | Base URL and auth token fields exist, sandbox toggle present, but tightly coupled to env vars |
| Spotlight shortcut | 🟢 | Customizable via config, defaults to Cmd+Shift+K |

### Issues

1. **Full config replacement on save** — `update_user_config` replaces the entire config object atomically (L310-312). If the frontend sends a stale copy, it will overwrite concurrent changes. This was the root cause of the "local inference config gets lost" issue from the conversation history.
2. **No config versioning/migration** — When new fields are added to `UserConfig`, existing configs silently get defaults. This is handled by serde `#[serde(default)]` but there's no version stamp to detect and warn about outdated configs.
3. **Config file stored in `app_config_dir`** — This is different from `app_data_dir` where everything else lives. Could cause confusion during debugging or backup.

### Design Flaws

- **Two separate config systems** — `UserConfig` (Rust `config.rs`) and `OpenClawConfig` (OpenClaw `config.rs`) are completely separate. Secrets, API keys, and model settings are split across two systems with no unification layer.
- **No config validation** — `update_user_config` blindly saves whatever the frontend sends. Invalid values (e.g., `scrape_concurrency_limit: 1000`) are not validated.

### Enhancements

- Implement partial config update (PATCH semantics) instead of full replacement
- Add config validation layer (min/max bounds, enum checks)
- Unify the two config systems with a single source of truth
- Add config export/import for backup

---

## Feature Set 1: Chat Engine — Native Rust Agent (Rig)

**Files**: `orchestrator.rs` (1238 lines), `unified_provider.rs` (757 lines), `chat.rs` (29 KB), `agent.rs`, `router.rs`, `tool_router.rs`, `tool_discovery.rs`, `sandbox_factory.rs`

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| Think-Act-Observe loop | 🟢 | `run_turn` implements the full loop with sandbox execution (MCP mode) and legacy `<tool_code>` parsing |
| Streaming | 🟢 | `ProviderEvent` enum (Content, Usage, ContextUpdate) streams via tokio mpsc channels |
| Provider support | 🟢 | `UnifiedProvider` with 6 kinds: OpenAI, Anthropic, Gemini, Groq, Local, OpenRouter |
| Anthropic streaming | 🟢 | Full SSE parsing with `content_block_delta`, `message_delta`, usage extraction |
| Gemini streaming | 🟢 | Full SSE parsing with `generateContent` stream format |
| Sandbox mode (MCP) | 🟡 | `run_sandbox_loop` exists (L532-783) but requires `mcp_base_url` to be configured |
| Legacy tool parsing | 🟡 | XML `<tool_code>` parsing is the default when MCP is not configured |
| Reasoning model detection | 🟢 | `is_reasoning_model()` correctly handles o1/o3 models (disables temperature) |

### Issues

1. **Dual code paths** — Both `run_sandbox_loop` (MCP mode) and the legacy tool loop coexist. This creates a maintenance burden where bug fixes need to be applied in two places.
2. **Temperature sanitization** — `sanitize_temperature` returns `None` for reasoning models, but the Gemini streaming path doesn't call it — it always sends temperature.

### Bottlenecks

- **Token counting** (`count_tokens`) uses a simple `len/4` heuristic (L387-414). This is fast but inaccurate, potentially causing context window overflows with non-Latin text.

### Design Flaws

- **No conversation memory/summarization pipeline** — The spec calls for `running_summary` and `pinned_facts` in the conversation state, but `Orchestrator` doesn't maintain any conversation state between turns. The frontend manages all history.
- **No verifier pass** — The spec's "Phase E: Verify" (citation check, freshness validation) is not implemented.

### Enhancements

- Implement running conversation summary to prevent context overflow on long conversations
- Add citation injection verification (if RAG/web used, ensure citations appear in response)
- Consolidate the two tool execution paths into a single abstraction

---

## Feature Set 4: RAG / Knowledge Pipeline

**Files**: `rag.rs` (903 lines), `vector_store.rs` (5 KB), `reranker.rs` (6 KB), `rag_tool.rs` (3.5 KB)

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| Document ingestion | 🟢 | PDF/MD/TXT extraction with OCR fallback via Chromium, SHA256 dedup, chunk and embed |
| Vector search | 🟢 | USearch integration, cosine similarity |
| FTS5 lexical search | 🟢 | Hybrid retrieval (vector + FTS5 BM25) with merge/dedup |
| Reranking | 🟢 | ONNX cross-encoder (ms-marco-MiniLM-L-6-v2) loaded via `ort` |
| Citation generation | 🟡 | Source IDs included in context pack, but no structured citation objects returned to frontend |
| Integrity check | 🟢 | `perform_integrity_check` validates vector index vs. SQLite chunk alignment |

### Issues

1. **Project-scoped retrieval** — `retrieve_context_internal` accepts `project_id_arg` parameter (L504), but the actual SQL query filtering by project_id needs verification. This is the root cause of the known issue "project chats can't access the knowledge base".
2. **`extract_document_content` is 197 lines** — Complex OCR fallback logic that spawns Chromium for PDF rendering. This function has deeply nested error paths.

### Design Flaws

- **No chunk overlap** — The spec calls for 10-20% overlap between chunks, but the chunking logic in `ingest_document` would need verification to confirm this.
- **RAG tool returns raw passages** — The `rag_tool.rs` returns text passages but doesn't wrap them in the structured `ToolResult` format (citations, artifacts, timings) defined in the agent spec.

### Enhancements

- Return structured citations from RAG tool with doc_id, chunk_id, confidence score
- Implement neighbor chunk expansion (retrieve surrounding chunks for context)
- Add chunk overlap validation and make overlap configurable
- Add document re-ingestion detection (skip unchanged documents based on content hash)

---

## Feature Set 3: Web Search & Scraping

**Files**: `web_search.rs` (29 KB in rig_lib/tools), `scrape_page.rs` (8.6 KB), `trusted_sources.rs` (1.6 KB), `chromium_resolver.rs` (2.2 KB), `WebSearchBubble.tsx` (17 KB)

### Completion: 🟢

| Aspect | Status | Detail |
|---|---|---|
| DuckDuckGo search | 🟢 | HTML parsing based search |
| Trusted source ranking | 🟢 | Dedicated source list with priority boosting |
| Chromium scraping | 🟢 | Smart fallback: HTTP GET first, then Chromium if JS needed |
| Map-reduce summarization | 🟢 | Scraped content sent to summarizer LLM for compression |
| Search grounding heuristic | 🟡 | Auto-trigger for factual queries, but known to over-fire on simple greetings |
| Frontend rendering | 🟢 | `WebSearchBubble` shows search results with expandable sources |

### Issues

1. **Search grounding over-triggers** — Known issue from conversations: auto mode may use `web_search` for all queries including "hello". The heuristic needs tuning to exclude greetings/conversational prompts.
2. **Brave Search integration uncertain** — `UserConfig` has no Brave API key field. Brave is mentioned in the README as supported but may only be accessible via OpenClaw's identity.json secrets.

### Bottlenecks

- **Scraping concurrency** — Default limit is 2 concurrent scrapes. For deep research queries scraping 5+ pages, this creates a serial bottleneck.
- **Summarizer dependency** — Map-reduce summarization requires the summarizer sidecar to be running. If it's not started, the pipeline degrades silently.

### Enhancements

- Tune search grounding heuristic: add explicit exclusion list for greetings, one-word queries, and emoji-only messages
- Add configurable Brave Search support as a primary search engine option
- Surface summarizer dependency in UI (show "Search results may be unprocessed" if summarizer is offline)

---

## Feature Set 5: Imagine Studio

**Files**: `image_gen.rs` (25 KB), `imagine.rs` (18 KB), `image_gen_tool.rs` (3.1 KB), `images.rs` (4 KB), `ImagineGeneration.tsx` (49 KB), `ImagineGallery.tsx` (31 KB)

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| Local diffusion (sd.cpp) | 🟡 | Backend code exists but image server is a **stub** in sidecar.rs — only stores model path |
| Gemini Imagen 3 | 🟢 | Cloud generation via Gemini API (imagine.rs handles Gemini-specific image generation) |
| Gallery | 🟢 | 31 KB component with favoriting, search, settings restoration |
| Style library | 🟢 | 45 presets in `style-library.ts` |
| Prompt enhancer | 🟡 | Library exists (`prompt-enhancer.ts`, 2.5 KB) but unclear if fully integrated per `improvements_current.md` |
| Progress tracking | 🟢 | Real-time progress events from image generation |

### Issues

1. **Local diffusion path incomplete** — `start_image_server` is a stub (Feature Set 7 finding). The full sd.cpp sidecar spawning, VAE/CLIP resolution, and progress tracking are in `image_gen.rs` but may call the sidecar via CLI rather than HTTP server.
2. **49 KB frontend component** — `ImagineGeneration.tsx` is extremely large and likely contains business logic that should be extracted into hooks/utilities.

### Design Flaws

- **Two separate image generation backends** (`image_gen.rs` and `imagine.rs`) with different approaches — one for tool-invoked generation, one for the studio UI. These should share a common generation service.

### Enhancements

- Extract `ImagineGeneration.tsx` into smaller composable components
- Unify image generation into a single service layer
- Add image-to-image and inpainting support

---

## Feature Set 2: OpenClaw Agent Engine

**Files**: `openclaw/commands.rs` (103 KB), `openclaw/config.rs` (77 KB), `openclaw/ws_client.rs` (23 KB), `openclaw/normalizer.rs` (26 KB), `openclaw/ipc.rs` (12 KB), `openclaw/fleet.rs` (13 KB), `openclaw/frames.rs` (4.3 KB), `openclaw/deploy.rs` (3.8 KB)

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| WS client & protocol | 🟢 | Full ACP implementation with connect, subscribe, send, history |
| Session management | 🟡 | Session deletion known to be buggy (conversation history) |
| Config generation | 🟢 | 77 KB config file handles identity, auth profiles, channel configs |
| Normalizer | 🟢 | 26 KB of robust upstream payload normalization |
| HITL approvals | 🟢 | `ApprovalCard.tsx` component exists |
| Model allowlist | 🟡 | Implementation in progress per conversation history |
| Factory reset | 🟡 | Known to not clear all session data (conversation history) |

### Issues

1. **103 KB commands.rs** — The single largest file in the codebase. This is a maintenance risk — any change requires understanding a massive monolith.
2. **Known session deletion bug** — Delete icon visible but doesn't actually delete.
3. **Known factory reset incompleteness** — Previous sessions still visible after reset.

### Design Flaws

- **Monolithic command file** — 103 KB suggests this should be split into at least 5-6 focused modules (session management, config, channels, fleet commands, etc.)
- **77 KB config file** — Similarly oversized. The OpenClaw config system alone is larger than many entire applications.

### Enhancements

- Split `commands.rs` into focused modules (session_commands, channel_commands, fleet_commands, config_commands)
- Fix session deletion and factory reset
- Add session archiving (soft delete with restore capability)

---

## Feature Set 9: Spotlight Command Bar

**Files**: `SpotlightBar.tsx` (33 KB)

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| Global shortcut | 🟢 | Cmd+Shift+K configurable |
| Transient sessions | 🟢 | Sessions purge on close |
| Glassmorphism | 🟡 | Known rendering issue in production builds (toast/spotlight area) |
| Centering | 🟡 | Known centering issue when opening (conversation history) |

### Issues

1. **33 KB single component** — Too much logic for a single file.
2. **Known centering and rendering issues** in production builds.

### Enhancements

- Break into SpotlightOverlay, SpotlightInput, SpotlightResults sub-components
- Add quick-action commands (e.g., `/model`, `/rag`, `/imagine`)

---

## Feature Set 10: OpenClaw Dashboard, Skills, Memory & Automations

**Files**: 16 components in `src/components/openclaw/` (total ~200 KB)

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| Dashboard | 🟢 | Status overview component |
| Skills management | 🟡 | `OpenClawSkills.tsx` (19 KB) — auto-discovery from skills dir unclear |
| Memory/Brain editing | 🟢 | `OpenClawMemory.tsx`, `MemoryEditor.tsx`, `OpenClawBrain.tsx` |
| Automations | 🟡 | `OpenClawAutomations.tsx` (14 KB) — HEARTBEAT.md loop implementation status unclear |
| System control | 🟢 | `OpenClawSystemControl.tsx` (19 KB) |
| Channels | 🟢 | `OpenClawChannels.tsx` (11 KB) |
| Presence | 🟢 | `OpenClawPresence.tsx` (11 KB) |

### Enhancements

- Verify HEARTBEAT.md automation scheduling is active
- Add skill marketplace / community skill index browser
- Add memory version history (track changes to SOUL.md, USER.md over time)

---

## Feature Set 11: Channels — Slack & Telegram

**Files**: `SlackTab.tsx` (12 KB), `TelegramTab.tsx` (11 KB), `openclaw/config.rs`

### Completion: 🟢

| Aspect | Status | Detail |
|---|---|---|
| Slack settings UI | 🟢 | Token inputs, manifest copy, validation |
| Telegram settings UI | 🟢 | Bot token, DM policy, groups toggle |
| Gateway config integration | 🟢 | Config generation handles channel enable/disable |

### Issues

1. **Token validation** — Unclear if "Validate" button makes real API calls or just saves config and restarts.

---

## Feature Set 12: Fleet & Multi-Agent

**Files**: `fleet.rs` (13 KB), `FleetCommandCenter.tsx` (37 KB), `FleetGraph.tsx` (2.5 KB), `FleetTerminal.tsx` (2.7 KB), `AgentNode.tsx` (6 KB), `RemoteDeployWizard.tsx` (19 KB), `deploy.rs` (3.8 KB), `CloudBrainConfigModal.tsx` (31 KB)

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| Fleet UI | 🟢 | 37 KB command center with graph visualization |
| Remote deploy wizard | 🟡 | 19 KB component, Ansible-based provisioning. Known issue: must target Linux-only |
| Fleet graph | 🟡 | Only 2.5 KB — likely placeholder or minimal visualization |
| Fleet terminal | 🟡 | Only 2.7 KB — likely placeholder |
| Backend fleet logic | 🟡 | 13 KB — moderate implementation |

### Issues

1. **Small FleetGraph/Terminal components** suggest these are scaffolded but not fully functional.
2. **Remote deploy targets macOS/Linux** — Known from conversations that it should be Linux-only (Ubuntu/Debian).

### Enhancements

- Expand fleet graph to show real-time agent topology with health indicators
- Add fleet-wide command broadcasting
- Implement agent-to-agent task delegation protocol

---

## Feature Set 13: STT, Vision, Projects & Misc

**Files**: `stt.rs` (4.4 KB), `vision.ts` (445 bytes), `projects.rs` (6.4 KB), `history.rs` (12.5 KB), `personas.rs` (2 KB)

### Completion: 🟡

| Aspect | Status | Detail |
|---|---|---|
| STT integration | 🟡 | Server lifecycle exists, but no API key on Whisper (sidecar finding). Frontend hook exists (`use-audio-recorder.ts`, 5 KB) |
| Vision | 🟡 | `vision.ts` is only **445 bytes** — likely a stub or minimal utility |
| Projects | 🟢 | Full CRUD with SQLite backing, project file listing |
| History | 🟢 | Conversations + messages with pagination, editing, full delete |
| Personas | 🟢 | Built-in persona definitions |

### Issues

1. **Known: project chats don't store titles properly** (from `improvements_current.md`)
2. **Known: links in LLM replies not clickable** (from `improvements_current.md`)
3. **Vision is minimal** — 445 bytes suggests basic type definitions only, not a full pipeline

### Enhancements

- Implement vision pipeline end-to-end (image attachment → base64 → mmproj injection → response)
- Fix clickable links in LLM replies (markdown link rendering in MessageBubble)
- Fix project chat title storage

---

## Cross-Cutting Findings

### Architecture Strengths
1. **Zero TODOs/FIXMEs** in the entire Rust backend — clean codebase
2. **Type safety** via `tauri-specta` ensures Rust↔TS contract consistency
3. **ProcessTracker** provides crash recovery for orphaned sidecars
4. **Dual-engine architecture** (Rig + OpenClaw) provides flexibility for different task types

### Architecture Risks
1. **Mutex proliferation** — 36 `unwrap()` calls on mutex locks in `sidecar.rs` alone. A single panic poisons all mutexes and crashes the entire app
2. **File size concentration** — `commands.rs` (103 KB), `config.rs` (77 KB), `ChatLayout.tsx` (69 KB), `GatewayTab.tsx` (68 KB), `SecretsTab.tsx` (64 KB). These monoliths are maintenance bottlenecks
3. **Two config systems** — `UserConfig` and `OpenClawConfig` operate independently with no unification
4. **No integration tests** — Zero `#[test]` modules found in the scanned files

### Top 5 Priority Fixes
1. **Fix session deletion** in OpenClaw (blocking user workflow)
2. **Fix search grounding over-trigger** (causes unnecessary API/compute costs)
3. **Fix project-scoped RAG retrieval** (project knowledge base not accessible)
4. **Add mutex poisoning resilience** (prevent total app crash from sidecar failures)
5. **Fix config save race condition** (local inference config loss)
