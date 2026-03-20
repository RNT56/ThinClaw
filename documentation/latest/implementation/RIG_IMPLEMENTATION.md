# RIG Implementation Reference

> **Scope:** This document covers every aspect of how `rig-core` is used in the Scrappy/OpenClaw backend — which files implement it, which providers it routes to, which agents and tools it powers, how the Orchestrator extends it, and how the full `chat_stream` lifecycle works end-to-end.
>
> **Last reviewed:** 2026-02-23  
> **Crate version:** `rig-core = "0.7.0"` (`backend/Cargo.toml` line 53)

---

## Table of Contents

1. [What is rig-core and Why We Use It](#1-what-is-rig-core-and-why-we-use-it)
2. [File Map — Every RIG-Related File](#2-file-map--every-rig-related-file)
3. [Provider Layer — UnifiedProvider](#3-provider-layer--unifiedprovider)
   - 3.1 [ProviderKind enum](#31-providerkind-enum)
   - 3.2 [CompletionModel implementation](#32-completionmodel-implementation)
   - 3.3 [Streaming — stream_raw_completion](#33-streaming--stream_raw_completion)
   - 3.4 [Provider-specific quirks](#34-provider-specific-quirks)
4. [Local Provider — LlamaProvider](#4-local-provider--llamaprovider)
   - 4.1 [Responsibilities](#41-responsibilities)
   - 4.2 [Model-family stop tokens](#42-model-family-stop-tokens)
   - 4.3 [Message alternation enforcement](#43-message-alternation-enforcement)
   - 4.4 [Token counting](#44-token-counting)
5. [The RigManager Agent](#5-the-rigmanager-agent)
   - 5.1 [Construction](#51-construction)
   - 5.2 [Preamble and mode flags](#52-preamble-and-mode-flags)
   - 5.3 [Native Rig-tool wiring](#53-native-rig-tool-wiring)
   - 5.4 [Public methods](#54-public-methods)
   - 5.5 [Cancellation check](#55-cancellation-check)
6. [RigManager Cache](#6-rigmanager-cache)
   - 6.1 [Purpose](#61-purpose)
   - 6.2 [Cache key](#62-cache-key)
   - 6.3 [Invalidation strategy](#63-invalidation-strategy)
7. [Orchestrator — the ReAct Loop](#7-orchestrator--the-react-loop)
   - 7.1 [Role and position](#71-role-and-position)
   - 7.2 [McpOrchestratorConfig](#72-mcporchestratorconfig)
   - 7.3 [ToolPermissions](#73-toolpermissions)
   - 7.4 [run_turn() walkthrough](#74-run_turn-walkthrough)
   - 7.5 [Manual Mode path](#75-manual-mode-path)
   - 7.6 [Auto/Tool Mode — Sandbox ReAct loop](#76-autotool-mode--sandbox-react-loop)
   - 7.7 [Summarisation pipeline](#77-summarisation-pipeline)
8. [Sandbox Factory](#8-sandbox-factory)
   - 8.1 [create_sandbox()](#81-create_sandbox)
   - 8.2 [Rhai host tools registered](#82-rhai-host-tools-registered)
   - 8.3 [Remote MCP typed bindings](#83-remote-mcp-typed-bindings)
9. [Tool Discovery](#9-tool-discovery)
10. [Tool Router](#10-tool-router)
11. [Tools — Rig Native (`Tool` trait)](#11-tools--rig-native-tool-trait)
    - 11.1 [DDGSearchTool (web_search)](#111-ddgsearchtool-web_search)
    - 11.2 [ScrapePageTool](#112-scrapepagetool)
    - 11.3 [CalculatorTool](#113-calculatortool)
    - 11.4 [RAGTool (knowledge_search)](#114-ragtool-knowledge_search)
    - 11.5 [ImageGenTool (generate_image)](#115-imagegentool-generate_image)
12. [Deterministic Router (legacy)](#12-deterministic-router-legacy)
13. [Provider Resolution — chat.rs](#13-provider-resolution--chatrs)
14. [Full chat_stream Lifecycle](#14-full-chat_stream-lifecycle)
15. [Tauri State Registration](#15-tauri-state-registration)
16. [Exposed Tauri Commands](#16-exposed-tauri-commands)
17. [Design Decisions and Trade-offs](#17-design-decisions-and-trade-offs)
18. [Known Limitations and Tech Debt](#18-known-limitations-and-tech-debt)

---

## 1. What is rig-core and Why We Use It

[`rig-core`](https://github.com/0xPlaygrounds/rig) is a Rust library for building LLM-powered agents with a clean, trait-based abstraction over inference providers. We use it for:

- **`CompletionModel` trait** — lets us write one `Agent<UnifiedProvider>` that works across any provider backend.
- **`AgentBuilder`** — composable DSL for attaching tools (as `Tool` trait impls) and a system preamble to an agent.
- **`Tool` trait** — standardised interface for type-safe tool definitions and calls. The agent's built-in function-calling loop handles argument deserialisation, tool dispatch, and result injection automatically.
- **`CompletionRequest` / `CompletionResponse` / `ModelChoice`** — shared types for both agent-loop calls and our own direct streaming calls.

We use rig as **scaffolding** — the type system and agent builder. The production streaming path (`chat_stream` → `Orchestrator`) bypasses the Rig agent's sync tool loop in favour of our custom Rhai-based ReAct loop, which gives us full control over streaming, cancellation, and multi-turn tool execution.

---

## 2. File Map — Every RIG-Related File

| File (relative to `backend/src/`) | Size | Role |
|---|---|---|
| `rig_lib/mod.rs` | 76 L | Module root; exports `RigManager`; two Tauri commands |
| `rig_lib/agent.rs` | 302 L | `RigManager` struct — the agent wrapper |
| `rig_lib/unified_provider.rs` | 757 L | `UnifiedProvider` — multi-backend `CompletionModel` |
| `rig_lib/llama_provider.rs` | 709 L | `LlamaProvider` — streaming specialist for OpenAI-compat |
| `rig_lib/orchestrator.rs` | 1181 L | `Orchestrator` — full ReAct loop, summarisation, MCP |
| `rig_lib/router.rs` | 140 L | `Router` — deterministic keyword routing (legacy planning) |
| `rig_lib/tool_router.rs` | 320 L | `ToolRouter` — registry-driven Skills/Host/MCP dispatch |
| `rig_lib/tool_discovery.rs` | 140 L | `get_host_tools_definitions()` + `search_all_tools()` |
| `rig_lib/sandbox_factory.rs` | 555 L | `create_sandbox()` — Rhai engine with all tools wired |
| `rig_lib/chromium_resolver.rs` | ~80 L | Chromium binary discovery for ScrapePageTool |
| `rig_lib/tools/mod.rs` | 9 L | Re-exports all tool modules |
| `rig_lib/tools/web_search.rs` | 686 L | `DDGSearchTool` — DuckDuckGo + scraping + map-reduce summarisation |
| `rig_lib/tools/scrape_page.rs` | ~300 L | `ScrapePageTool` — Chromium/chromiumoxide page scraper |
| `rig_lib/tools/calculator_tool.rs` | 1219 L | `CalculatorTool` + `evaluate()` — hand-written recursive-descent parser |
| `rig_lib/tools/rag_tool.rs` | 114 L | `RAGTool` — vector store retrieval (`knowledge_search`) |
| `rig_lib/tools/image_gen_tool.rs` | 102 L | `ImageGenTool` — Stable Diffusion via sidecar |
| `rig_lib/tools/models.rs` | ~50 L | Shared response types: `ToolResult`, `Citation` |
| `rig_lib/tools/trusted_sources.rs` | ~60 L | Domain allow-list for search result ranking |
| `rig_cache.rs` | 122 L | `RigManagerCache` — Tokio-mutex keyed cache |
| `chat.rs` | 803 L | Primary consumer: `chat_stream`, `chat_completion`, `count_tokens` |
| `lib.rs` | 466 L | App bootstrap — manages `RigManagerCache` as Tauri state |

---

## 3. Provider Layer — UnifiedProvider

**File:** `rig_lib/unified_provider.rs`

`UnifiedProvider` is the core provider abstraction. It implements `rig::completion::CompletionModel` and acts as a universal adapter for all supported LLM backends.

### 3.1 ProviderKind enum

```rust
pub enum ProviderKind {
    OpenAI,
    Anthropic,
    Gemini,
    Groq,        // OpenAI-compatible
    Local,       // llama.cpp, OpenAI-compatible
    OpenRouter,  // OpenAI-compatible
}
```

### 3.2 CompletionModel implementation

```rust
impl CompletionModel for UnifiedProvider {
    type Response = Vec<ModelChoice>;

    async fn completion(&self, request: CompletionRequest)
        -> Result<CompletionResponse<Vec<ModelChoice>>, CompletionError>
    {
        match self.kind {
            ProviderKind::Anthropic => self.completion_anthropic(request).await,
            ProviderKind::Gemini    => self.completion_gemini(request).await,
            _                       => self.completion_openai(request).await,
        }
    }
}
```

This is the entry-point used by the Rig `Agent` when its internal tool-calling loop needs a non-streaming completion (e.g. tool-call disambiguation). The three private methods build provider-specific JSON bodies and parse the response into `ModelChoice::Message` or `ModelChoice::ToolCall`.

**Anthropic** differences:
- System prompt goes in a top-level `system` field, not a message
- Tools use `input_schema` instead of `parameters`
- Content is a typed array (`text`, `tool_use`, …)

**Gemini** differences:
- System prompt injected as `system_instruction` top-level field
- Turns use `contents[].parts` with `role: "user"/"model"` (not `"assistant"`)
- Tool calls come back as `functionCall` in parts

**OpenAI / Groq / OpenRouter / Local** — identical request shape; only auth headers differ.

### 3.3 Streaming — stream_raw_completion

```rust
pub async fn stream_raw_completion(
    &self,
    messages: Vec<Value>,
    temperature: Option<f64>,
) -> Result<Pin<Box<dyn Stream<Item = Result<ProviderEvent, String>> + Send>>, String>
```

This is the **primary streaming entry-point** used by the Orchestrator. It returns a `ProviderEvent` stream:

```rust
pub enum ProviderEvent {
    Content(String),                      // text token
    Usage(crate::chat::TokenUsage),       // prompt/completion token counts
    ContextUpdate(Vec<crate::chat::Message>), // summarisation result
}
```

**Routing inside `stream_raw_completion`:**
- `Local | OpenAI | Groq | OpenRouter` → delegates to `LlamaProvider::stream_raw_completion()`
- `Anthropic` → `stream_anthropic()` — SSE via `eventsource_stream`, handles `content_block_delta` and `message_delta` events
- `Gemini` → `stream_gemini()` — SSE via `streamGenerateContent?alt=sse`, handles `<think>` / thought parts for Gemini 3

### 3.4 Provider-specific quirks

| Provider | Quirk |
|---|---|
| `OpenRouter` | Adds `HTTP-Referer` and `X-Title` headers |
| Reasoning models (`o1-*`, `o3-*`, `gpt-5*`) | `sanitize_temperature()` strips the temperature field entirely (these models reject it) |
| `Gemini` | Merges consecutive same-role turns (Gemini rejects them); wraps thought parts in `<think>…</think>` |
| `Local` | Delegates all streaming to `LlamaProvider` which adds per-family stop tokens and merges same-role messages |

---

## 4. Local Provider — LlamaProvider

**File:** `rig_lib/llama_provider.rs`

`LlamaProvider` handles all OpenAI-compatible backends — including the llama.cpp sidecar, OpenAI proper, Groq, and OpenRouter — for **streaming** and **token counting** operations.

### 4.1 Responsibilities

- `impl CompletionModel for LlamaProvider` — non-streaming completions (used by Rig tool loop)
- `stream_completion()` — legacy history-based streaming (used by `RigManager::stream_chat`)
- `stream_raw_completion()` — raw `Vec<Value>` messages streaming (used by Orchestrator)
- `count_tokens()` — calls `/tokenize` on llama.cpp; falls back to `len/3` for cloud providers

### 4.2 Model-family stop tokens

```rust
// In LlamaChatRequest:
stop: if is_local {
    Some(crate::gguf::stop_tokens_for_family(&self.model_family))
} else {
    None
}
```

`gguf::stop_tokens_for_family()` returns per-family stop sequences (e.g. `<|im_end|>` for ChatML, nothing for Gemma which uses native EOS). This prevents ChatML models (Mistral, Qwen) from hallucinating extra turns.

### 4.3 Message alternation enforcement

Before sending to the API, `stream_raw_completion` enforces **strict user/assistant alternation**:

```
For each message (non-system):
    If same role as last message → merge by appending content with \n\n
```

Mistral's chat template throws an exception on consecutive same-role messages. This merger is the safety net for cases where the Orchestrator's multi-turn logic produces back-to-back user messages (e.g. tool result injection + synthesis instruction).

### 4.4 Token counting

`count_tokens()` uses a very short timeout (3 s) and calls `POST /tokenize`. On timeout or non-local endpoints, it falls back to `len/3` (approx 3 chars/token). This is called by the Orchestrator's heuristic-first token check before deciding whether to summarise history.

**System prompt sanitisation for Gemma:** When `model_family == "gemma"`, the `sanitize_system_prompt_for_local()` function:
1. Replaces `<tag>` angle-bracket patterns with `` `tag` `` (prevents clash with Gemma's `<start_of_turn>` special tokens)
2. Replaces negative instruction phrasing with positive equivalents
3. Collapses double spaces

---

## 5. The RigManager Agent

**File:** `rig_lib/agent.rs`

`RigManager` is the top-level agent struct. It wraps a Rig `Agent<UnifiedProvider>` and exposes a set of async methods used by the Orchestrator and by legacy Tauri commands.

### 5.1 Construction

```rust
pub struct RigManager {
    pub agent: Arc<Agent<UnifiedProvider>>,
    pub provider: UnifiedProvider,      // Direct provider access for streaming
    pub summarizer_provider: Option<UnifiedProvider>,
    pub app_handle: Option<tauri::AppHandle>,
    pub context_window: usize,
    pub conversation_id: Option<String>,
}
```

`#[derive(Clone)]` — the `Arc<Agent>` makes clones cheap. This is required by `RigManagerCache`.

### 5.2 Preamble and mode flags

`RigManager::new()` builds the system preamble based on `enable_web_search`:

**Research Mode (`enable_web_search = true`):**
- Tells the LLM about `web_search` and `calculator` tools
- Instructs direct reply for greetings, code, creative writing, general knowledge
- Requires the LLM to start with a `Thought:` before any action

**Standard Mode (`enable_web_search = false`):**
- Still always registers `CalculatorTool`, `RAGTool`, `ImageGenTool`
- Instructs: no tools for chat, calculator for math, web_search only on explicit request, image only on explicit command

**User Context injection:** If `user_context: Option<String>` is non-`None`, the content is appended to the preamble inside `<user_knowledge>` tags.

### 5.3 Native Rig-tool wiring

```rust
let mut builder = AgentBuilder::new(provider.clone()).preamble(&base_preamble);

if enable_web_search {
    builder = builder
        .tool(DDGSearchTool { app, max_total_chars: context_window * 4 * 60/100, summarizer, conversation_id })
        .tool(ScrapePageTool { app: Mutex::new(app) });
}

let agent = builder
    .tool(CalculatorTool)
    .tool(RAGTool { app: app.expect("required") })
    .tool(ImageGenTool { app: app.expect("required") })
    .build();
```

`max_total_chars` for web search = 60% of the context window in characters (~4 chars/token).

**Note:** The Rig `Agent` uses the native JSON function-calling API. Tools only get invoked when `ProviderKind` supports function calls consistently. For the Orchestrator path, tools are instead registered as Rhai functions (see §8).

### 5.4 Public methods

| Method | Description |
|---|---|
| `chat(&str)` | Single prompt via `agent.prompt()` — non-streaming, uses Rig's native tool loop |
| `explicit_search(&str)` | Directly calls `DDGSearchTool.call()` without going through the agent — used by the Orchestrator's Rhai `web_search()` binding |
| `rag_chat(query, history)` | Runs `explicit_search` then constructs a RAG prompt and calls `agent.chat()` |
| `stream_rag_chat(query, history)` | Same as `rag_chat` but calls `provider.stream_completion()` for a streaming result |
| `stream_chat(prompt, history)` | Direct streaming via `provider.stream_completion()` — **bypasses the agent tool loop** (Manual Mode) |
| `is_cancelled()` | Checks `SidecarManager.cancellation_token` via the `AppHandle` |

### 5.5 Cancellation check

```rust
pub fn is_cancelled(&self) -> bool {
    if let Some(app) = &self.app_handle {
        if let Some(state) = app.try_state::<SidecarManager>() {
            return state.cancellation_token.load(Relaxed);
        }
    }
    false
}
```

The Orchestrator's ReAct loop calls `rig.is_cancelled()` at the top of every turn and after every streamed chunk.

---

## 6. RigManager Cache

**File:** `rig_cache.rs`

### 6.1 Purpose

Before the cache, `RigManager::new()` (and thus `reqwest::Client::new()`) was called on every `chat_stream` invocation — incurring a TLS handshake, TCP connection, and DNS lookup on every message. The cache reuses the same manager (and its underlying `reqwest` connection pool) across turns when nothing has changed.

### 6.2 Cache key

```rust
pub struct RigManagerKey {
    pub provider_kind: String,   // Debug repr of ProviderKind
    pub base_url: String,
    pub model_name: String,
    pub token: String,           // API key
    pub context_size: usize,
    pub enable_tools: bool,
    pub gk_content: String,      // Concatenated knowledge-bit content
    pub model_family: Option<String>,
}
```

All eight fields must match for a cache hit.

### 6.3 Invalidation strategy

- **Automatic:** Any change to the key fields triggers `Cache MISS` and a rebuild (user switches provider, rotates key, toggles Auto Mode, changes knowledge bits, etc.)
- **Explicit:** `cache.invalidate().await` is available for factory-reset scenarios
- **Thread-safety:** `tokio::sync::Mutex` — `chat_stream` already holds the global generation lock, so contention is practically zero

```rust
pub async fn get_or_build<F>(&self, key: RigManagerKey, build_fn: F) -> RigManager
where F: FnOnce() -> RigManager
```

---

## 7. Orchestrator — the ReAct Loop

**File:** `rig_lib/orchestrator.rs`

The `Orchestrator` is the **primary execution engine** for all production chat requests. It replaces Rig's built-in agent loop with a custom, streaming-first, Rhai-based ReAct cycle that supports multi-turn tool use, context window management, and MCP integration.

### 7.1 Role and position

```
chat_stream()
    └── Orchestrator::new_with_mcp(Arc<RigManager>, McpConfig)
            └── orchestrator.run_turn(messages, permissions, project_id, persona, conv_id)
                    ├── [Token Check + Auto-Summarisation]
                    ├── Manual Mode  →  provider.stream_raw_completion()
                    └── Auto Mode    →  build_sandbox() → run_sandbox_loop()
```

### 7.2 McpOrchestratorConfig

```rust
pub struct McpOrchestratorConfig {
    pub mcp_base_url: Option<String>,    // e.g. "https://api.scrappy.dev"
    pub mcp_auth_token: Option<String>,  // JWT bearer
    pub sandbox_enabled: bool,           // true only when URL + flag both set
}
```

### 7.3 ToolPermissions

```rust
pub struct ToolPermissions {
    pub allow_web_search: bool,   // Auto mode OR legacy web icon
    pub force_web_search: bool,   // Legacy web icon only — LLM must search
    pub allow_file_search: bool,  // Auto mode OR has project/docs
    pub allow_image_gen: bool,    // Auto mode only
}
```

These permissions gate which tools are advertised in the system prompt and which are registered in the sandbox.

### 7.4 run_turn() walkthrough

`run_turn()` spawns a Tokio background task and returns a `ReceiverStream`. The stream emits `ProviderEvent` items consumed by `chat_stream`.

**Step 0 — Token Check & Auto-Summarisation**

A two-phase check:
1. Fast heuristic: ~4 chars/token over all messages
2. If within 80% of the threshold, call `provider.count_tokens()` precisely via `/tokenize`
3. Threshold = 60% of `context_window`

If over threshold, the oldest 50% of history is summarised by calling `provider.stream_raw_completion()` with a summarisation prompt at temperature 0.1. The result is injected as a `role: "system"` message with `is_summary: true`, and a `ProviderEvent::ContextUpdate` is emitted so the frontend can update its local state.

**Step 1 — Document ID collection**

All `attached_docs` from history + current message are gathered into `all_doc_ids`.

**Step 2 — Mode decision**

```
if !any_tools → Manual Mode
else          → Auto/Tool Mode
```

`any_tools = allow_web_search || allow_file_search || allow_image_gen`

### 7.5 Manual Mode path

1. RAG retrieval via `rag::retrieve_context_internal()` (if docs or project present)
2. Visual preview injection — up to 2 document previews as base64 `image_url` parts (for multimodal models)
3. Conversation assembled: `[system prompt + context] + [history] + [visual previews] + [user query]`
4. `provider.stream_raw_completion(conversation, None)` — direct streaming, **no tool loop**

Status XML emitted: `<scrappy_status type="thinking" />`

### 7.6 Auto/Tool Mode — Sandbox ReAct loop

**Image detection:** If `raw_content` is a JSON array containing `image_url` parts, the Orchestrator enters **Vision Mode** (see §7.8). Otherwise, it proceeds with the full tool pipeline.

**Sandbox creation:**
```rust
let sandbox = self.build_sandbox(&tx)
    .or_else(|| self.build_sandbox_unconditional(&tx));
```
`build_sandbox_unconditional` forces `sandbox_enabled = true` so local host tools (web_search, rag_search, read_file) are always available even without a remote MCP server.

**System prompt construction:**
- Persona instructions + current date
- `CORE RULES` section — varies by `force_web_search` / `has_mcp` / plain auto mode
- `TOOL USAGE` section — explains the `<rhai_code>…</rhai_code>` protocol with examples
- `AVAILABLE TOOLS` list — conditionally includes each tool based on permissions

**Tool invocation protocol (the model sees):**
```xml
<rhai_code>
let results = web_search("latest AI news");
results
</rhai_code>
```

**ReAct loop (max 5 turns; 2 turns when `force_web_search`; 1 turn when images present):**

```
Turn N:
  1. If last turn → inject synthesis instruction into last user message
  2. stream_raw_completion(conversation, temperature=0.1)
  3. Buffer tokens; detect <rhai_code> opening tag
  4. If code detected:
       a. collect full <rhai_code>…</rhai_code> block
       b. execute via sandbox.execute(script)
       c. inject <tool_result>output</tool_result> as next user message
       d. continue to next turn
  5. If no code (direct answer):
       forward all tokens to the tx channel
       break
```

On the **last turn**, any `<rhai_code>` tags are stripped before forwarding — the model is forced to synthesise in natural language even if it tries to call a tool.

Cancellation is checked at the top of every turn and inside the streaming loop.

Status XML tags emitted to frontend:
```xml
<scrappy_status type="thinking" />
<scrappy_status type="tool_call" name="web_search" query="..." status="running" />
<scrappy_status type="progress" pct="50" msg="..." />
<scrappy_status type="summarizing" />
<scrappy_status type="rag_search" query="..." />
```

### 7.8 Vision Mode (Image Analysis)

When the user's current message contains images (detected by `raw_content.starts_with('[') && raw_content.contains("image_url")`), the Orchestrator uses a **simplified code path** optimised for vision-language models:

1. **Compact system prompt** — Only the persona instructions + date + a concise "analyze images directly" instruction (~50 tokens). The full tool prompt (~800+ tokens of Rhai examples, tool descriptions, search rules) is omitted. This is critical for small VLMs (4B) where the tool prompt alone would consume 20%+ of the context window.

2. **Direct multimodal content** — The user's message is passed as a JSON array of `text` + `image_url` parts directly, without wrapping in tool instruction prefixes (e.g. no "Respond to this request. Only use tools if...").

3. **Single turn** — `max_turns = 1` (no ReAct loop). Image analysis doesn't need multi-step tool reasoning.

**Image history context bloat prevention** (`chat.rs`):

Base64 image data (~100KB per 1024×1024 JPEG = ~25K tokens) would fill a 32K context window by the second turn. To prevent this:
- Only the **last/current** user message embeds full base64 image data
- Older messages in history that contained images are replaced with a text placeholder: `"[User shared N image(s) in this message]"`
- This reduces per-turn history overhead from ~25K tokens to ~10 tokens

### 7.7 Summarisation pipeline

The auto-summarisation (Step 0) uses the **summarizer provider** if one is configured. The summariser is typically a smaller/faster local model running on a separate port. If no dedicated summariser is configured, the main provider is used. The summarisation request runs at temperature 0.1 for determinism.

---

## 8. Sandbox Factory

**File:** `rig_lib/sandbox_factory.rs`

### 8.1 create_sandbox()

```rust
pub fn create_sandbox<R: StatusReporter + 'static>(
    rig: Arc<RigManager>,
    mcp_config: &McpOrchestratorConfig,
    reporter: Arc<R>,
) -> Option<Sandbox>
```

Returns `None` if `sandbox_enabled = false`. Called by:
1. `Orchestrator::build_sandbox()` — for the main ReAct loop
2. `Orchestrator::build_sandbox_unconditional()` — same, but forces `sandbox_enabled = true`
3. `McpRequestHandler` — for OpenClaw IPC-driven tool requests (shared factory ensures consistent tool availability on both paths)

The `StatusReporter` bridges `ToolEvent` emissions from the sandbox to `ProviderEvent::Content(xml_tag)` items on the response stream.

### 8.2 Rhai host tools registered

Every function registered on the Rhai engine is available to the LLM via `<rhai_code>` blocks.

| Rhai function | Implementation | Note |
|---|---|---|
| `web_search(query: String)` | `rig.explicit_search(&query)` | Async bridged via `tokio::task::block_in_place` |
| `rag_search(query: String)` | `rag::retrieve_context_internal(…)` | Requires `rig.app_handle` |
| `read_file(path: String)` | `std::fs::read_to_string` | Hard cap at 20,000 chars |
| `calculator(expr: String)` | `calculator_tool::evaluate(&expr)` | Pure sync, no async needed |
| `search_tools(query: String)` | `tool_discovery::search_all_tools(…)` | Returns JSON of host+skill+remote tools |
| `list_skills()` | `SkillManager::list_skills()` | Returns JSON array of skill metadata |
| `run_skill(id, args_json)` | `SkillManager::prepare_script()` + eval | Executes skill script in nested Rhai scope |
| `save_skill(id, script, desc)` | `SkillManager::save_skill(…)` | Persists a new user-defined skill |

### 8.3 Remote MCP typed bindings

When `mcp_base_url` is configured, additional typed Rhai functions are registered using a shared `McpClient`:

| Rhai function | Remote tool |
|---|---|
| `mcp_call(tool_name, args_json)` | Generic passthrough — any remote tool by name |
| `finance::get_stock_price(symbol)` | Finance module |
| `news::get_news(category, limit)` | News module |
| `news::search_news(query)` | News search |
| `news::get_headlines(country, limit)` | Headlines |
| `knowledge::rag_query(query)` | Remote RAG |
| `economics::get_economic_data(country)` | Economics module |
| `models::get_model_catalog()` | Model catalog |
| `health::search_medical_research(query, limit)` | Health/medical |
| `ai_tools::summarize_text(text, length)` | Remote LLM summarisation |

---

## 9. Tool Discovery

**File:** `rig_lib/tool_discovery.rs`

```rust
pub fn get_host_tools_definitions() -> Vec<ToolInfo>
```

Single source of truth for host tool names, descriptions, and JSON schemas. Used by:
- `ToolRouter::host_tool_names()` — auto-populates the dispatch registry
- `search_all_tools()` — for `search_tools()` responses shown to the LLM

**Registered host tools:**
```
web_search   — "Full text search of web content via DuckDuckGo"
rag_search   — "Search local documents and knowledge base (vector embeddings)"
read_file    — "Read file contents (sandbox restricted, read-only)"
calculator   — "Evaluate mathematical expressions…"
```

```rust
pub async fn search_all_tools(
    query: &str,
    mcp_client: Option<&McpClient>,
    skill_manager: Option<&SkillManager>,
    include_host: bool,
) -> SearchResult
```

Searches across all three tool tiers — host, skills, remote MCP — filtering by `query` substring match on name and description.

---

## 10. Tool Router

**File:** `rig_lib/tool_router.rs`

`ToolRouter` is a registry-driven dispatcher that resolves tool calls by name to their implementation tier:

```
Priority:
  1. Skills (SkillManager) — user-defined Rhai workflows
  2. Host Tools (registry from tool_discovery) — local native tools
  3. Remote MCP (McpClient) — external API tools
```

```rust
pub struct ToolRouter<'a> {
    pub mcp_client:    Option<&'a McpClient>,
    pub skill_manager: Option<&'a SkillManager>,
    pub sandbox:       Option<&'a Sandbox>,
}
```

All host tools are dispatched through the sandbox (`sb.execute()`) with a generated Rhai one-liner.

**Utility functions:**
- `summarize_result(result, max_chars)` — truncates long `content[].text` fields
- `summarize_arbitrary_json(val, max_string_len, max_array_len)` — recursive JSON truncator

Both functions have full unit test coverage in the same file.

---

## 11. Tools — Rig Native (`Tool` trait)

These structs implement `rig::tool::Tool` and are registered on the `Agent<UnifiedProvider>` via `AgentBuilder::tool()`. They are invoked by the Rig agent's internal function-calling loop in the legacy/non-streaming paths.

### 11.1 DDGSearchTool (web_search)

**File:** `rig_lib/tools/web_search.rs`

```rust
pub struct DDGSearchTool {
    pub app: Option<tauri::AppHandle>,
    pub max_total_chars: usize,
    pub summarizer: Option<UnifiedProvider>,
    pub conversation_id: Option<String>,
}
impl Tool for DDGSearchTool {
    const NAME: &'static str = "web_search";
    type Args = SearchArgs;    // { query: String }
    type Output = String;
}
```

**Execution pipeline:**

1. **DDG HTML scrape** — `GET https://duckduckgo.com/html/?q={encoded_query}`, parse `.result` elements, extract title/link/snippet (up to 20 raw results)
2. **Trusted source ranking** — `trusted_sources::is_trusted(&link)` bumps known reputable domains to the top; truncate to top 5
3. **Emit `web_search_results` event** — initial result cards to frontend
4. **Parallel Chromium scraping** — `ScrapePageTool::scrape_url()` in a dedicated `tokio::task::spawn_blocking` runtime (Chromium browser is not `Sync`); concurrency limited by `config.scrape_concurrency_limit`
5. **Map-Reduce summarisation** (when `summarizer` is present):
   - Each scraped page is chunked into `chunk_size` char blocks with 10% overlap
   - Each chunk is validated by an LLM call: returns `{ score, reasoning, summary }` JSON
   - Chunks scoring ≥ 4.0 are kept; summaries are aggregated
   - If the combined summary exceeds `chars_per_slot`, a recursive reduce LLM call is made
6. **Sort** by aggregate relevance score (descending)
7. **Emit `web_search_results`** with final summaries + `web_search_status: "generating"`
8. **Return** `ToolResult` JSON via `generate_tool_result_json()`

Cancellation is checked at multiple points (before DDG, between scraping, between summarisation chunks).

**Concurrency parameters** (from `UserConfig`):
- `scrape_concurrency_limit` — parallel browser tabs
- `search_concurrency_limit` — parallel LLM summarisation calls
- `summarization_chunk_size` — chars per chunk (0 = dynamic)
- `max_scrape_chars` — global char cap for all results

### 11.2 ScrapePageTool

**File:** `rig_lib/tools/scrape_page.rs`

```rust
pub struct ScrapePageTool {
    pub app: Mutex<Option<tauri::AppHandle>>,
}
impl Tool for ScrapePageTool {
    const NAME: &'static str = "scrape_page";
    type Args = ScrapeArgs;    // { url: String }
    type Output = String;
}
```

Uses `chromiumoxide` to launch a headless Chromium, navigate to the URL, wait for page load, and extract clean text via `html2text`. The `chromium_resolver.rs` module finds the system Chromium binary. The Mutex wrapper is required because `AppHandle` is not `Sync` and `ScrapePageTool` is shared across parallel scraping tasks.

**Used internally by** `DDGSearchTool::call()` — it is not directly exposed to the LLM as a Rhai sandbox function in normal usage (only through the DDG tool's scraping pipeline).

### 11.3 CalculatorTool

**File:** `rig_lib/tools/calculator_tool.rs`

```rust
pub struct CalculatorTool;
impl Tool for CalculatorTool {
    const NAME: &'static str = "calculator";
    type Args = CalcArgs;      // { expression: String }
    type Output = String;
}
```

Zero-size struct — no state required. Delegates to the public `evaluate(expr: &str) -> Result<f64, String>` function.

**`evaluate()` is also called directly** by the Rhai sandbox binding (`sandbox_factory.rs` line 190), making `CalculatorTool` the single implementation used by both tool paths.

**Parser details** — hand-written recursive-descent, no external dependencies:
- Grammar: `expr → term → power → unary → primary`
- Power is **right-associative**: `2^3^2 = 2^(3^2) = 512`
- `**` treated as `^` (Python style)
- Scientific notation: `1e3`, `2.5E-3`

**Supported functions:** `sqrt`, `abs`, `round`, `ceil`, `floor`, `log`/`log10`, `ln`, `log2`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `min`, `max`, `pow`, `exp`

**Supported constants:** `pi`, `e`, `tau`, `inf`/`infinity`

**Error cases:** division by zero, modulo by zero, sqrt of negative, unknown identifier, mismatched parentheses, NaN, infinite result.

Fully unit-tested (40+ tests covering arithmetic, functions, constants, scientific notation, real-world scenarios, and error cases).

### 11.4 RAGTool (knowledge_search)

**File:** `rig_lib/tools/rag_tool.rs`

```rust
pub struct RAGTool { pub app: tauri::AppHandle }
impl Tool for RAGTool {
    const NAME: &'static str = "knowledge_search";
    type Args = RAGArgs;       // { query: String }
    type Output = String;
}
```

Emits `web_search_status: "rag_searching"` to the frontend, then calls `crate::rag::retrieve_context_internal()` in a spawned task (requires `SidecarManager`, `SqlitePool`, `VectorStoreManager`, `RerankerWrapper` from Tauri state).

Returns document chunks joined by `---` separators, or `"No relevant information found in the knowledge base."`.

**Note:** In Auto/Tool Mode the Orchestrator uses `rag_search()` in the Rhai sandbox rather than this Rig-native tool. This tool is only invoked via the Rig agent loop (legacy `agent_chat` path or explicit `rag_chat()` calls).

### 11.5 ImageGenTool (generate_image)

**File:** `rig_lib/tools/image_gen_tool.rs`

```rust
pub struct ImageGenTool { pub app: tauri::AppHandle }
impl Tool for ImageGenTool {
    const NAME: &'static str = "generate_image";
    type Args = ImageGenArgs;  // { prompt: String, negative_prompt: Option<String> }
    type Output = String;
}
```

Emits `web_search_status: "generating"`, then calls `crate::image_gen::generate_image()` which communicates with the Stable Diffusion sidecar. Returns a Markdown image link:

```markdown
![Generated Image](/path/to/image.jpg)

**Generated Image ID:** <uuid>
```

---

## 12. Deterministic Router (legacy)

**File:** `rig_lib/router.rs`

`Router::plan()` is a deterministic keyword-based routing function that pre-classifies a user query into a `ToolPlan` before any LLM is involved. It was designed as a latency optimisation (no LLM classifier round-trip).

```rust
pub enum Decision { NoTool, Rag, Web, RagAndWeb, Image, Clarify }

pub struct ToolPlan {
    pub decision: Decision,
    pub reason: String,
    pub steps: Vec<ToolStep>,
    pub response_style: String,  // "brief", "normal", "detailed"
}
```

**Routing rules (in priority order):**
1. Has attachments → `NoTool` (Rig/Orchestrator handles context injection)
2. Query starts with `draw`/`generate image`/`create picture`/`drawing of` → `Image`
3. Contains `price`/`news`/`weather`/`latest`/starts with `search for` → `Web`
4. Has `project_id` AND query contains code/file/how/implement/fix/… → `Rag`
5. Less than 3 words → `NoTool` (greeting)
6. Fallback → `NoTool`

**Current status:** The `Router` struct is defined but **not called in the production `chat_stream` path**. The Orchestrator subsumes this logic via the LLM's own reasoning (guided by the system prompt's CORE RULES). This file is kept for potential future use as a pre-classifier.

---

## 13. Provider Resolution — chat.rs

**File:** `chat.rs` — `resolve_provider()`

Called at the top of `chat_stream` and `chat_completion`. Reads `user_config.selected_chat_provider` and returns a `ProviderConfig`:

| `selected_chat_provider` | `ProviderKind` | Base URL | Default model |
|---|---|---|---|
| `"anthropic"` | `Anthropic` | `https://api.anthropic.com/v1` | `claude-3-5-sonnet-latest` |
| `"openai"` | `OpenAI` | `https://api.openai.com/v1` | `gpt-4o` |
| `"openrouter"` | `OpenRouter` | `https://openrouter.ai/api/v1` | `moonshotai/kimi-k2.5` |
| `"gemini"` | `Gemini` | `https://generativelanguage.googleapis.com/v1beta/models` | `gemini-2.0-flash` |
| `"groq"` | `OpenAI` (compat) | `https://api.groq.com/openai/v1` | `llama-3.3-70b-versatile` |
| `_` (default) | `Local` | `http://127.0.0.1:{port}/v1` | `"default"` |

API keys are fetched from `OpenClawManager::get_config()` (stored in macOS Keychain, not plaintext). The local provider reads the running sidecar port, token, context size, and model family from `SidecarManager::get_chat_config()`.

---

## 14. Full chat_stream Lifecycle

```
1. [chat.rs] chat_stream() — IPC entry
   ├── acquire generation_lock (queuing concurrent requests)
   ├── reset cancellation_token
   ├── preprocess messages:
   │     ├── load base64 images for LAST user message only
   │     ├── replace older image messages with text placeholders
   │     └── filter image-generation history turns
   ├── resolve_provider() → ProviderConfig
   ├── collect gk_content (enabled knowledge bits) → preamble injection
   ├── build RigManagerKey (8-field cache key)
   ├── rig_cache.get_or_build(key, || RigManager::new(…)) → RigManager
   ├── build McpOrchestratorConfig from user_config
   ├── build ToolPermissions from payload flags + has_context
   ├── resolve persona_instructions (custom or built-in persona)
   └── Orchestrator::new_with_mcp(Arc::new(manager), mcp_config)
           └── orchestrator.run_turn(messages, permissions, …) → Stream<ProviderEvent>

2. [orchestrator.rs] run_turn() — spawns background Tokio task
   ├── Token count heuristic → precise count → maybe: summarise history
   ├── Collect all_doc_ids from history + current message
   ├── Manual Mode (no tools):
   │     ├── RAG retrieval (if docs/project)
   │     ├── visual PDF previews injection
   │     └── provider.stream_raw_completion() → ProviderEvent stream
   └── Auto/Tool Mode:
         ├── if images: Vision Mode (simplified prompt, 1 turn, direct analysis)
         ├── create_sandbox(rig, mcp_config, reporter)
         └── run_sandbox_loop():
               ├── build system prompt (persona + date + rules + tools list)
               ├── ReAct loop (max 5 turns):
               │     ├── inject synthesis instruction on last turn
               │     ├── provider.stream_raw_completion(conversation, temp=0.1)
               │     ├── buffer tokens, detect <rhai_code>
               │     ├── if code: sandbox.execute(script) → <tool_result>
               │     └── if no code: stream to frontend, break
               └── sends all ProviderEvent items to mpsc::Sender

3. [chat.rs] stream consumption — back in chat_stream():
   ├── Content events → batch-buffered (20 chars OR 30ms flush interval)
   ├── Usage events → flushed immediately with StreamChunk { usage }
   ├── ContextUpdate events → flushed immediately with StreamChunk { context_update }
   ├── cancellation_token check before each event
   └── on stream end → send StreamChunk { done: true }
```

---

## 15. Tauri State Registration

**File:** `lib.rs`

```rust
app.manage(RigManagerCache::new());
```

Registered at line 298 of `lib.rs`, immediately after `SidecarManager`, `DownloadManager`, and `ConfigManager`. This makes `State<'_, RigManagerCache>` available to all Tauri commands.

The `RigManagerCache` is the only explicit RIG-specific state. `RigManager` instances themselves are created on-demand (or cache-hit) and are not stored as permanent Tauri state — they are passed through as local variables within `chat_stream`.

---

## 16. Exposed Tauri Commands

| Command | File | Description |
|---|---|---|
| `chat_stream` | `chat.rs` | Primary streaming chat — uses Orchestrator |
| `chat_completion` | `chat.rs` | Non-streaming single completion — uses `UnifiedProvider.completion()` directly |
| `count_tokens` | `chat.rs` | Token count for a conversation — uses `LlamaProvider.count_tokens()` |
| `rig_check_web_search` | `rig_lib/mod.rs` | Debug command: directly calls `DDGSearchTool.call()` |
| `agent_chat` | `rig_lib/mod.rs` | Legacy command: creates a fresh `RigManager` and calls `manager.chat()` (Local provider only) |

---

## 17. Design Decisions and Trade-offs

### Rig as scaffolding, not the runtime

The decision to build `UnifiedProvider` implementing `CompletionModel` rather than using rig's built-in provider clients (`rig::providers::openai`, etc.) was made to support providers Rig doesn't have built-in (Gemini, Groq, local llama.cpp with model-family quirks) and to have full control over streaming SSE parsing, stop tokens, and authentication headers.

### Two parallel tool execution paths

Tools exist in two forms:
1. **Rig `Tool` trait structs** — type-safe, for the native `Agent<UnifiedProvider>` (used in legacy/non-streaming paths)
2. **Rhai engine functions** — for the Orchestrator's ReAct loop (production streaming path)

This duplication is intentional. The Rig agent loop is synchronous at the tool-call parsing level and doesn't support streaming mid-generation. The Rhai path gives us character-by-character streaming with interleaved tool execution. The shared `evaluate()` function in `CalculatorTool` is an example of sharing core logic across both paths.

### Rhai over native Rust async for the ReAct loop

The sandbox uses [`rhai`](https://rhai.rs/) (an embedded scripting engine) rather than having the LLM emit structured JSON tool calls. This was chosen because:
- Local models (llama.cpp) have inconsistent/broken native function-calling support
- Rhai provides a deterministic execution sandbox with controllable timeouts
- The LLM can compose multi-step scripts (sequential tool calls, variable bindings, string formatting) in a single turn
- This avoids N round-trips for N tool calls — a multi-step Rhai script executes in one sandbox invocation

### RigManagerCache

The "cache-or-build" pattern avoids repeated TLS handshakes and DNS lookups between conversational turns. The 8-field key ensures any meaningful configuration change triggers a rebuild, preventing stale providers from serving requests intended for a different model/key.

---

## 18. Known Limitations and Tech Debt

| Issue | Location | Notes |
|---|---|---|
| `agent_chat` creates a fresh `RigManager` on every call | `rig_lib/mod.rs` | TODO comment present; acceptable for debug/legacy use |
| `RAGTool` doesn't have easy access to `conversation_id` | `rig_lib/tools/rag_tool.rs` | Emits `id: None` in the status event |
| `Router::plan()` is implemented but never called | `rig_lib/router.rs` | Could serve as a pre-classifier to save LLM calls for obvious cases |
| `ScrapePageTool` is not directly accessible from the Rhai sandbox | `rig_lib/tools/scrape_page.rs` | Only used internally by `DDGSearchTool`; the Orchestrator can't call it directly |
| `completion_gemini` doesn't respect chat history | `rig_lib/unified_provider.rs` | The non-streaming Gemini path injects history inline into `contents` but doesn't handle multi-turn history for the agent tool loop |
| Rhai sandbox execution is synchronous (uses `block_in_place`) | `rig_lib/sandbox_factory.rs` | Requires a multi-thread Tokio runtime; incompatible with `current_thread` runtime |
| Image generation is gated behind `app_handle.expect()` in `RigManager::new()` | `rig_lib/agent.rs` | Panics if called without an `AppHandle` — only acceptable in the Tauri sidecar context |
| `trusted_sources.rs` domain list is hardcoded | `rig_lib/tools/trusted_sources.rs` | Should be user-configurable |
