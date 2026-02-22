# OpenClaw Data Flow Trace

> **Date:** 2026-02-20
> **Scope:** Four core data flows within the Scrappy → OpenClaw application stack.
> **Excluded:** Deployment / Ansible / remote provisioning.

---

## Table of Contents

1. [Sending a Chat Message with an Attached Document](#1-sending-a-chat-message-with-an-attached-document)
2. [Memory System (MemoryEditor)](#2-memory-system-memoryeditor)
3. [Web Search RIG Execution](#3-web-search-rig-execution)
4. [Chat Session Lifecycle](#4-chat-session-lifecycle)

---

## 1. Sending a Chat Message with an Attached Document

This flow traces the full journey from a user selecting a file, through ingestion and
embedding, to the LLM generating a response grounded in that document's content.

### 1.1 Document Ingestion

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **UI selection** | Frontend | `src/components/chat/ChatInput.tsx` | User clicks the **Paperclip → Document** button (calls `handleFileUpload`) or types `@` in the textarea to mention an already-ingested document. |
| **File dialog** | Frontend | `ChatLayout.tsx` | `handleFileUpload` opens a native file picker via Tauri's `dialog.open()`. |
| **Upload** | IPC → Backend | `src-tauri/src/rag.rs` → `upload_document` | The selected file is copied into the app-data `documents/` directory. A row is inserted into the SQLite `documents` table with `id`, `name`, `hash`, `path`, `mime_type`. |
| **Ingestion** | Backend | `src-tauri/src/rag.rs` → `ingest_document` | The file is read, parsed (PDF/text/image OCR), split into overlapping chunks (~500 tokens each), and each chunk is embedded via the sidecar embedding model. |
| **Vector storage** | Backend | `src-tauri/src/vector_store.rs` | Embeddings are stored in a USEARCH index file. Chunk metadata (text, doc_id, offset) goes into the SQLite `chunks` table. The `VectorScope` determines the index file location (Global, Project-scoped, or Chat-scoped). |

### 1.2 Message Assembly & Dispatch

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **State update** | Frontend | `ChatLayout.tsx` | The document's `{ id, name }` is appended to the `ingestedFiles` state array. It renders as a removable chip below the textarea. |
| **Send** | Frontend | `ChatLayout.tsx` → `handleSend` | Constructs a `Message` object with `role: "user"`, `content: <text>`, and `attached_docs: [{ id, name }]`. |
| **Persistence** | IPC → Backend | `src-tauri/src/history.rs` → `save_message` | The message (including `attached_docs` JSON) is persisted to the local SQLite `messages` table, keyed by `conversation_id`. |
| **Stream request** | IPC → Backend | `src-tauri/src/chat.rs` → `chat_stream` | The frontend calls `commands.chatStream(conversationId, messages, channel)`. A Tauri `Channel` is opened for SSE-style streaming back to the UI. |

### 1.3 Retrieval-Augmented Generation (RAG)

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **Orchestrator entry** | Backend | `src-tauri/src/rig_lib/orchestrator.rs` → `run_turn` | The orchestrator receives the full message history. It extracts `attached_docs` from every message (both history and current turn) into `all_doc_ids`. |
| **Context retrieval** | Backend | `src-tauri/src/rag.rs` → `retrieve_context_internal` | Embeds the user's query, performs a vector similarity search against the USEARCH index (filtered to the relevant `doc_ids`), re-ranks results via the `RerankerWrapper`, and returns the top-k text chunks. |
| **Visual preview** | Backend | `orchestrator.rs` (Manual Mode) | For the first 2 attached documents, the orchestrator checks for cached JPEG previews in `app_data/previews/{hash}.jpg`. If found, they are base64-encoded and injected as `image_url` content parts for multimodal models. |
| **Prompt injection** | Backend | `orchestrator.rs` | **Manual Mode:** Context is directly prepended to the system prompt as `[ATTACHED CONTEXT]: ...`. **Tool Mode (Sandbox):** The LLM is given access to `rag_search(query)` as a callable Rhai function, allowing it to query the vector store on-demand. |
| **LLM completion** | Backend | `unified_provider.rs` → `stream_raw_completion` | The assembled conversation (system + history + context + user query) is streamed to the configured LLM provider (local llama.cpp or cloud API). |
| **Streaming to UI** | Backend → Frontend | `chat.rs` + `chat-context.tsx` | Tokens are streamed back via the Tauri `Channel`. The `ChatProvider` accumulates deltas into `fullMessage` and renders them in `MessageBubble`. |

### 1.4 Sequence Diagram (Simplified)

```
User → ChatInput → handleFileUpload() → dialog.open()
                                           ↓
                                      rag::upload_document() → SQLite + disk
                                           ↓
                                      rag::ingest_document() → chunks + embeddings → VectorStore
                                           ↓
User types message + Enter
                                           ↓
ChatLayout::handleSend() → history::save_message() → SQLite
                         → chat::chat_stream()
                                           ↓
                         orchestrator::run_turn()
                              ├── collect attached_docs IDs from all messages
                              ├── rag::retrieve_context_internal(query, doc_ids)
                              │        ├── embed(query) → sidecar
                              │        ├── vector_search(embedding) → USEARCH
                              │        └── rerank(chunks) → RerankerWrapper
                              ├── inject context into system prompt
                              └── provider.stream_raw_completion(conversation)
                                           ↓
                         Channel → ChatProvider → MessageBubble (streaming)
```

---

## 2. Memory System (MemoryEditor)

The memory system provides long-term persistence across chat sessions through a
dedicated `MEMORY.md` file stored in the OpenClaw workspace directory.

### 2.1 Architecture

```
┌──────────────────┐     IPC      ┌────────────────────┐     Filesystem
│  MemoryEditor.tsx │ ──────────→ │  commands.rs        │ ──────────→  MEMORY.md
│  (React editor)  │ ←────────── │  openclaw_get_memory│ ←──────────  (workspace/)
│                  │             │  openclaw_save_memory│
└──────────────────┘              └────────────────────┘
```

### 2.2 Data Retrieval

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **Component mount** | Frontend | `src/components/openclaw/MemoryEditor.tsx` | On mount, calls `commands.openclawGetMemory()`. |
| **IPC** | Tauri command | `src-tauri/src/openclaw/commands.rs` → `openclaw_get_memory` | Reads `OpenClawConfig` to determine the workspace directory. Constructs path as `workspace_dir/MEMORY.md`. |
| **File read** | Backend | Same function | If the file exists: `std::fs::read_to_string(path)` returns the raw markdown. If not: returns `"No memory file found."`. |
| **Display** | Frontend | `MemoryEditor.tsx` | Content is loaded into a textarea or markdown editor for the user to review and modify. |

### 2.3 Data Modification

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **User edits** | Frontend | `MemoryEditor.tsx` | The user modifies the markdown content directly in the editor. |
| **Save action** | Frontend | `MemoryEditor.tsx` | Clicking "Save" calls `commands.openclawSaveMemory(content)`. |
| **IPC** | Tauri command | `commands.rs` → `openclaw_save_memory` | Receives the full markdown string. |
| **Directory creation** | Backend | Same function | `tokio::fs::create_dir_all(parent)` ensures the workspace directory tree exists. |
| **File write** | Backend | Same function | `tokio::fs::write(path, content)` performs an async atomic write to `MEMORY.md`. |

### 2.4 How the Agent Uses Memory

The OpenClaw Engine (the background Node.js process) reads `MEMORY.md` during:
- **Boot:** Loaded as part of the agent's initial system context.
- **Refresh cycles:** The engine can re-read the file when instructed by its internal reasoning to "recall" user preferences or project context.
- **Agent writes:** The agent itself can modify `MEMORY.md` through tool calls, creating a bidirectional persistence mechanism.

---

## 3. Web Search RIG Execution

The Web Search system enables the LLM to ground its responses in real-time
internet data. It operates through the RIG (Research Intelligence Gateway)
subsystem.

### 3.1 Triggering Conditions

| Condition | Result | Source |
|-----------|--------|--------|
| User toggles the **Globe icon** ON in `ChatInput.tsx` | `isWebSearchEnabled = true`, `force_web_search = true` | `ChatInput.tsx`, `orchestrator.rs` |
| User leaves Globe OFF but `allow_web_search` is configured | The LLM decides autonomously whether to search | `orchestrator.rs` (auto mode) |
| Neither condition | Web search tools are not exposed to the LLM | System prompt excludes `web_search` |

### 3.2 Execution Flow

#### 3.2a Sandbox Mode (Rhai — Current Default)

```
orchestrator::run_sandbox_loop()
    ↓
LLM generates <rhai_code> block:
    let results = web_search("query");
    results
    ↓
Sandbox::execute(script)
    ↓
Rhai engine calls web_search() host function
    ↓
web_search.rs::perform_web_search(query)
    ├── reqwest GET → DuckDuckGo HTML
    ├── scraper parses <a class="result__a"> elements
    └── Returns Vec<WebSearchResult> { title, link, snippet }
    ↓
Results serialized as <tool_result> and injected into conversation
    ↓
LLM synthesizes final response
```

#### 3.2b Legacy Tool Mode (Fallback)

```
orchestrator::run_legacy_tool_loop()
    ↓
LLM generates <tool_code> block with web_search call
    ↓
ToolRouter parses the XML block
    ↓
tool_router::handle_web_search()
    ↓
web_search.rs::perform_web_search(query)     [same as above]
    ↓
Results injected as <tool_result>
    ↓
LLM synthesizes final response
```

### 3.3 Web Search Implementation Detail

**File:** `src-tauri/src/web_search.rs`

```
perform_web_search(query: &str) → Result<(String, Vec<WebSearchResult>), String>
```

| Step | Detail |
|------|--------|
| **HTTP client** | `reqwest::Client` with a Firefox User-Agent header to avoid bot blocking. |
| **Request** | GET `https://html.duckduckgo.com/html/?q={query}` |
| **Parsing** | Uses the `scraper` crate with CSS selectors to extract result links (`a.result__a`), titles, and snippets (`a.result__snippet`). |
| **Output** | Returns a tuple: (formatted markdown string of results, structured `Vec<WebSearchResult>`). |
| **Error handling** | Network errors, empty results, and parse failures are all mapped to descriptive `Err(String)`. |

### 3.4 System Prompt Behavior

The orchestrator constructs different system prompts based on the search mode:

- **Force mode** (`force_web_search = true`): `"ALWAYS SEARCH: The user has explicitly enabled web search. You MUST use web_search for every query..."`
- **Auto mode** (`allow_web_search = true`, `force = false`): `"REPLY DIRECTLY for greetings, code, creative writing... USE TOOLS ONLY when the user needs real-time information..."`

### 3.5 ReAct Loop

The sandbox mode runs a **ReAct loop** (up to 5 turns) in `run_sandbox_loop`:

1. LLM generates text (streamed to UI) + optional `<rhai_code>` block.
2. If code is detected, it's executed in the sandbox.
3. Results are fed back as `<tool_result>`.
4. LLM generates next turn (may synthesize answer or issue another tool call).
5. Loop exits when the LLM responds without a code block, or after 5 turns.

---

## 4. Chat Session Lifecycle

OpenClaw sessions are managed externally by the **OpenClaw Engine** (a Node.js
background process). Scrappy acts as a controller/frontend that communicates with
the engine via a WebSocket gateway.

### 4.1 Architecture Overview

```
┌─────────────────┐   Tauri IPC   ┌──────────────────┐   WebSocket    ┌──────────────────┐
│  OpenClawSidebar│ ────────────→ │  commands.rs     │ ────────────→ │  OpenClaw Engine │
│  OpenClawChat   │ ←──────────── │  ws_client.rs    │ ←──────────── │  (Gateway)       │
│  (React UI)     │   Events      │  normalizer.rs   │   Events      │  (Node.js)       │
└─────────────────┘               └──────────────────┘               └──────────────────┘
                                                                            ↓
                                                                     Internal DB
                                                                     (sessions, messages)
```

### 4.2 Connection & Handshake

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **WS connect** | Backend | `ws_client.rs` → `OpenClawWsClient::connect` | Establishes a WebSocket connection to `ws://localhost:{port}/ws`. |
| **Challenge/Response** | Backend | `ws_client.rs` | The gateway sends `connect.challenge`. The client responds with `build_connect_req()` from `frames.rs`, including auth token, device ID, and scopes (`operator.read`, `operator.write`, `operator.approvals`, `operator.admin`). |
| **Connected** | Backend → Frontend | `normalizer.rs` | On success, emits `UiEvent::Connected { protocol }` to the frontend via Tauri event system. |
| **Reconnection** | Backend | `ws_client.rs` | Automatic reconnection with exponential backoff on disconnect. |

### 4.3 Session Discovery

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **UI request** | Frontend | `OpenClawSidebar.tsx` | Calls `commands.openclawGetSessions()` on mount and periodically. |
| **RPC** | Backend | `commands.rs` → `openclaw_get_sessions` | Sends `sessions.list` RPC via `ws_handle.sessions_list()`. |
| **Response parsing** | Backend | Same function | Deserializes the gateway's JSON response into `Vec<OpenClawSession>`. |
| **Guaranteed `agent:main`** | Backend | Same function | If `agent:main` is not in the list, it's synthetically added (it always exists). |
| **Sorting** | Backend | Same function | Sessions are sorted: `agent:main` first, then by `updated_at_ms` descending. |
| **UI rendering** | Frontend | `OpenClawSidebar.tsx` | Sessions are displayed as a scrollable list with titles, timestamps, and source badges. |

### 4.4 Loading Chat History

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **Session selection** | Frontend | `OpenClawSidebar.tsx` → `OpenClawChatView.tsx` | User clicks a session. The view calls `commands.openclawGetHistory(sessionKey, limit)`. |
| **RPC** | Backend | `commands.rs` → `openclaw_get_history` | Sends `chat.history` RPC via `ws_handle.chat_history(session_key, limit, None)`. |
| **Message parsing** | Backend | Same function | Complex parsing logic handles multiple upstream formats: raw `text` field, `content` as string, `content` as array of `{type, text}` blocks, and `toolCall` content parts. Tool calls are rendered as `[Tool Call: name] Input: {...}`. |
| **Normalization** | Backend | Same function | Each message is mapped to `OpenClawMessage { id, role, content, timestamp, source, metadata }`. |
| **UI rendering** | Frontend | `OpenClawChatView.tsx` | Messages are rendered in a scrollable chat view with role-based styling, tool call cards, and timestamp headers. |

### 4.5 Sending a Message

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **User input** | Frontend | `OpenClawChatView.tsx` | User types in the chat input and presses Enter or clicks Send. |
| **IPC call** | Frontend → Backend | `commands.openclawSendMessage(sessionKey, text, deliver)` | `deliver: true` means the message should trigger an agent run. |
| **Idempotency** | Backend | `commands.rs` → `openclaw_send_message` | Generates a unique idempotency key: `scrappy:{session_key}:{uuid}:{timestamp_ms}`. |
| **RPC** | Backend | `ws_client.rs` → `chat_send` | Sends `chat.send` RPC to the gateway with `sessionKey`, `idempotencyKey`, `text`, and `deliver` flag. |
| **Event streaming** | Gateway → Backend → Frontend | `ws_client.rs` → `normalizer.rs` | The engine streams responses as WebSocket events. The normalizer converts them into `UiEvent` variants. |

### 4.6 Event Normalization Pipeline

The normalizer (`normalizer.rs`) is the translation layer between the engine's evolving
protocol and Scrappy's stable UI contract.

| Upstream Event | UiEvent | Detail |
|----------------|---------|--------|
| `chat` + `state: "delta"` | `AssistantSnapshot` | Accumulated text snapshot (v2/v3 protocol). |
| `chat` + `state: "final"` | `AssistantFinal` | Complete message with optional usage stats. |
| `chat` + `state: "error"` | `RunStatus { status: "error" }` | Error with message. |
| `chat` + `kind: "assistant.delta"` | `AssistantDelta` | Incremental token append. |
| `chat` + `kind: "assistant.final"` | `AssistantFinal` | Final message with usage. |
| `chat` + `kind: "tool"` | `ToolUpdate` | Tool execution status. |
| `chat` + `kind: "run.status"` | `RunStatus` | Run lifecycle change. |
| `agent` + `stream: "tool"` | `ToolUpdate` | Tool phase (start/output/error). |
| `agent` + `stream: "lifecycle"` | `RunStatus` | Run phase (start/end/error). |
| `agent` + `stream: "assistant"` | `AssistantSnapshot` | Agent token stream. |
| `tool.start` / `tool.end` / `tool.error` | `ToolUpdate` | Top-level tool events. |
| `exec.approval.requested` | `ApprovalRequested` | Tool approval gate. |
| `exec.approval.resolved` | `ApprovalResolved` | Approval decision. |
| `canvas` | `CanvasUpdate` | HTML/JSON canvas content. |
| `web.login.*` | `WebLogin` | QR code / login flow. |
| Fallback: `{ delta: "..." }` | `AssistantDelta` | Legacy protocol heuristic. |
| Fallback: `{ text: "..." }` | `AssistantFinal` | Legacy protocol heuristic. |

**LLM Token Sanitization:** All assistant text passes through `strip_llm_tokens()` which
removes leaked ChatML markers (`<|im_start|>`, `<|im_end|>`), Llama tokens
(`<|eot_id|>`), thinking blocks (`<think>...</think>`), and collapses excessive newlines.

### 4.7 Session Deletion

| Step | Layer | File | Detail |
|------|-------|------|--------|
| **Guard** | Backend | `commands.rs` → `openclaw_delete_session` | `agent:main` cannot be deleted — returns early with an error. |
| **Step 1: Abort** | Backend | Same function | Sends `chat.abort(session_key)` to stop any active agent runs. Errors are logged but ignored (best-effort). |
| **Step 2: Delay** | Backend | Same function | `tokio::time::sleep(600ms)` — gives the engine time to wind down the run. |
| **Step 3: Delete** | Backend | Same function | Sends `sessions.delete(session_key)`. On success → done. |
| **Step 4: Fallback (if still active)** | Backend | Same function | If deletion fails with "still active" or "UNAVAILABLE", calls `sessions.reset(session_key)` to break the run association. |
| **Step 5: Retry** | Backend | Same function | Waits 800ms after reset, then retries `sessions.delete`. |
| **UI update** | Frontend | `OpenClawSidebar.tsx` | On success, the session is removed from the sidebar list. On failure, an error toast is shown. |

### 4.8 Key Distinction: OpenClaw vs. Standard Scrappy Chat

| Aspect | Standard Scrappy Chat | OpenClaw Chat |
|--------|----------------------|---------------|
| **Persistence** | Local SQLite (`history.rs`) | OpenClaw Engine's internal DB |
| **Message format** | `Message { role, content, images, attached_docs }` | `OpenClawMessage { id, role, content, timestamp, source, metadata }` |
| **Transport** | Direct Tauri IPC + `Channel` streaming | WebSocket → RPC → Event stream |
| **Inference** | Local model or cloud API (direct) | Delegated to OpenClaw Engine (which may use local or cloud) |
| **Session management** | Frontend-driven (`conversation_id` in SQLite) | Engine-managed (`session_key` via gateway RPC) |
| **Tool execution** | Orchestrator sandbox (Rhai) or legacy `<tool_code>` | Engine-internal tool system with approval gates |

---

## Appendix: Key Files Reference

| File | Purpose |
|------|---------|
| `src/components/chat/ChatInput.tsx` | Chat input component with file upload, mentions, slash commands |
| `src/components/chat/ChatLayout.tsx` | Main chat view orchestrating messages, input, and state |
| `src/components/chat/chat-context.tsx` | React context for streaming chat state management |
| `src/components/openclaw/OpenClawChatView.tsx` | OpenClaw-specific chat view with WS event handling |
| `src/components/openclaw/OpenClawSidebar.tsx` | Session list, navigation, and deletion controls |
| `src/components/openclaw/MemoryEditor.tsx` | MEMORY.md editor component |
| `src-tauri/src/chat.rs` | Tauri commands for standard chat streaming |
| `src-tauri/src/history.rs` | SQLite persistence for standard conversations |
| `src-tauri/src/rag.rs` | Document ingestion, chunking, embedding, and retrieval |
| `src-tauri/src/vector_store.rs` | USEARCH vector index management |
| `src-tauri/src/web_search.rs` | DuckDuckGo web search implementation |
| `src-tauri/src/rig_lib/orchestrator.rs` | Main orchestration: context assembly, RAG, tool loops |
| `src-tauri/src/openclaw/commands.rs` | Tauri commands for OpenClaw gateway operations |
| `src-tauri/src/openclaw/ws_client.rs` | WebSocket client with reconnection and RPC correlation |
| `src-tauri/src/openclaw/normalizer.rs` | Event normalization and LLM token sanitization |
| `src-tauri/src/openclaw/frames.rs` | WebSocket frame models (Req/Res/Event) |
| `src-tauri/src/openclaw/ipc.rs` | Internal IPC bridge for tool execution |
