# Rebuilding the Pi Agent Orchestrator in Rust

Rebuilding a "chat agent" is easy. You send a prompt to an LLM, wait for it to generate a tool call, run the tool, and send the result back.

However, the OpenClaw **Pi Agent Orchestrator** is exceptionally robust. It is designed to handle the messy reality of production AI: rate limits, context window overflows, provider outages, and real-time streaming to human users.

This document details all the advanced capabilities required to rebuild the Pi Agent natively in Rust on top of `rig-core`.

---

## 1. The Core Loop vs The Production Loop

### The Standard `rig-core` Agent Loop

```rust
// Basic rigid execution
let agent = openai_client.agent("gpt-4o").build();
let response = agent.chat("Search the web for X").await?;
// The user waits 15 seconds, sees nothing, then gets the final answer.
```

### The Pi Agent Production Loop

To match OpenClaw, your Rust agent loop must handle:

1. **Tool Streaming:** Emitting `delta` events while the AI is _deciding_ to use a tool, so the UI can show "Thinking..." typing indicators immediately.
2. **Provider Failover:** If Anthropic throws a `529 Overloaded`, seamlessly retrying the prompt against OpenAI's GPT-4o.
3. **Auth Rotation:** If an OpenAI API key hits a `429 Rate Limit`, marking that key as "cooldown" for 60 seconds and instantly retrying with a backup API key.
4. **Context Compaction:** If the total conversation history + new tool results exceeds the model's 128k context window, intelligently summarizing or truncating older messages before submitting the prompt.

---

## 2. Advanced Feature Breakdown & Architecture

### A. Provider Fallbacks & The Model Router

OpenClaw doesn't just bind an agent to a single model. It binds it to a "Profile" which defines fallbacks.

**Rust Implementation Sketch:**
You need a `ModelRouter` that wraps `rig-core` models and implements a retry loop.

```rust
pub struct ModelRouter {
    primary: rig::providers::anthropic::Client,
    fallback: rig::providers::openai::Client,
    // Local inference (MLX, llama.cpp, Ollama) is accessed via its
    // OpenAI-compatible HTTP server — NOT via a native Rust crate.
    // See INFERENCE_PLACEMENT_RS.md for the full explanation.
    local_fallback: rig::providers::openai::Client,  // points to http://localhost:8080/v1
}

impl ModelRouter {
    pub fn new(config: &Config) -> Self {
        Self {
            primary: rig::providers::anthropic::Client::new(&config.auth.anthropic_key),
            fallback: rig::providers::openai::Client::new(&config.auth.openai_key),
            // The local inference URL comes from config — could be MLX, llama.cpp, or Ollama.
            // All speak the OpenAI-compatible API, so the client is identical.
            local_fallback: rig::providers::openai::Client::from_url(
                "local",  // local servers accept any non-empty key
                &config.inference.endpoint,  // e.g. "http://localhost:8080/v1"
            ),
        }
    }

    pub async fn chat_with_fallback(&self, prompt: &str) -> Result<ChatResponse> {
        // Try Primary (Claude)
        match self.primary.completion(prompt).await {
            Ok(res) => return Ok(res),
            Err(e) if is_transient_or_failover(&e) => {
                tracing::warn!("Claude overloaded, falling back to GPT-4o");
            }
            Err(e) => return Err(e),
        }

        // Try Fallback (OpenAI)
        match self.fallback.completion(prompt).await {
             Ok(res) => return Ok(res),
             Err(e) if is_transient_or_failover(&e) => {
                 tracing::warn!("OpenAI failed, falling back to local inference");
             }
             Err(e) => return Err(e),
        }

        // Last resort: local inference engine (MLX / llama.cpp / Ollama)
        self.local_fallback.completion(prompt).await
    }
}
```

### B. Auth Profile Rotation (The "Keychain")

A large portion of the OpenClaw agent (`src/agents/auth-profiles.ts`) manages API keys. It tracks "last good usage" and "cooldown windows".

**Requirements:**

1. A global state manager for API keys (`RwLock<HashMap<Provider, Vec<AuthProfile>>>`).
2. Tracking HTTP 429 and 401 errors.
3. A priority queue: Always prefer the key that has the highest quota remaining, or round-robin if none have hit limits.

**Rust Design:**

```rust
pub struct AuthKeychain {
    pub keys: Vec<AuthProfile>,
}

impl AuthKeychain {
    pub fn get_next_available_key(&mut self, provider: Provider) -> Option<&AuthProfile> {
        // 1. Filter out keys currently in "cooldown"
        // 2. Select the one with the lowest recorded usage rate
        // 3. Return the token string
    }

    pub fn mark_failure(&mut self, key_id: &str, error: &ApiError) {
        if error.status_code == 429 {
            // Put in cooldown for 60s
        }
    }
}
```

### C. Context Compaction (Overflow Prevention)

Standard agents crash when text + tool results > `max_tokens`. Pi agent actively monitors token usage and triggers a "compaction" pre-flight step.

**OpenClaw's Strategy:**

1. **Calculate:** Prompt tokens + System tokens + History tokens + projected Tool Result tokens.
2. **Detect overflowing:** If > 90% of context window...
3. **Tool Guard:** First, aggressive truncate giant tool results (e.g., HTML from `web-fetch` over 40k chars becomes `<truncated due to length>`).
4. **History Summarization:** Run a _secondary, cheap agent_ (`gpt-4o-mini`) to summarize the first 50% of the conversation history into a single dense summary block, replacing the raw messages.

**Rust Implementation in RIG:**
RIG does not do this natively. You must write a custom `AgentBuilder` wrapper that intercepts the `chat()` call, counts tokens, and rewrites the `Vec<Message>` before passing it to the underlying `rig-core` LLM client.

> ⚠️ **Token Counter Caveats:**
> - **OpenAI / GPT models:** Use `tiktoken-rs` for accurate client-side token counting.
> - **Anthropic / Claude:** Anthropic returns token counts in API response headers (`anthropic-tokens-remaining`). Use these rather than estimating client-side, since Claude uses a different tokenizer.
> - **Local models / Ollama:** Use a conservative character-based estimate (1 token ≈ 4 chars) or query the `/v1/models` endpoint which sometimes reports `context_window`.

**Compaction Model Config (G3 — Remote Mode):**
The cheap secondary LLM used for summarization should be a distinct, hardcoded config value so it works in Remote Mode regardless of which primary model the user chose:
```toml
[agent]
# The compaction model is used to summarize history when context overflows.
# Should be a cheap, fast model — not the user's primary model.
compaction_model = "gpt-4o-mini"  # or "claude-haiku-3"
```

### D. Tool Output Streaming

When an agent decides to call a tool, it outputs a JSON blob. Standard agents wait until the whole JSON blob is generated, execute it, and then stream the final answer.

Pi Agent intercepts the raw delta stream so the UI can show "Agent is calling the Browser Tool..." _while_ the JSON is still being generated by the AI block layer.

## 3. Implementation Plan & Complexity

Replacing the Pi Agent is a **High Complexity (🔴)** task.

| Sub-system                | Rig Support              | Required Custom Rust                                 | Effort    |
| :------------------------ | :----------------------- | :--------------------------------------------------- | :-------- |
| **Tool Execution**        | Excellent (`Tool` trait) | None, works out of the box                           | Low       |
| **Streaming Output**      | Good                     | Custom stream interceptors for tool delta            | Medium    |
| **System Prompts**        | Excellent                | None, pass via `agent.build()`                       | Low       |
| **Model Router/Fallback** | None                     | Custom retry wrapper around `CompletionModel` traits | High      |
| **Auth Profile Rotation** | None                     | Custom global state + HTTP interceptors              | High      |
| **Context Compaction**    | None                     | Pre-flight token counting + history mutation logic   | Very High |

### Recommended Build Order

If you are replacing the Node.js Pi agent with Rust, do so incrementally:

1. **Phase 1: The Standard Agent**
   - Bind a RIG agent to your Chat UI.
   - Get plain text chatting and streaming working.
2. **Phase 2: RIG Tools**
   - Port the 4 most important tools (`web-search`, `web-fetch`, `bash`, `memory`) to RIG `Tool` traits and bind them to the agent.
3. **Phase 3: The Router Wrapper**
   - Build the `ModelRouter` to handle Anthropic → OpenAI fallbacks.

4. **Phase 4: Robustness (The Hard Stuff)**
   - Add the `tiktoken-rs` layer for context window compaction.
   - Build the `AuthKeychain` for API key rotation.

By following this order, you can have a working, useful agent rapidly, while saving the production-grade error handling (which took OpenClaw ~4,000 lines of code) for later.

---

## 4. Dependencies Mapping (Node.js -> Rust)

A major advantage of migrating the Pi Agent to Rust is that almost all heavy dependencies used in OpenClaw are already native Rust or C/C++ libraries wrapped for Node.js.

Using them directly in Rust eliminates binding compilation issues, native-addon headaches, and provides true multi-threading rather than blocking the Node.js event loop.

### A. Core AI & Logic

| OpenClaw Dependency (TS)            | Purpose                 | Rust Equivalent                   | Status                                                                                 |
| :---------------------------------- | :---------------------- | :-------------------------------- | :------------------------------------------------------------------------------------- |
| **`@mariozechner/pi-ai`**           | Core agent loop & logic | **`rig-core`**                    | ✅ Excellent. RIG handles this natively, though fallback/routing requires custom code. |
| **`@sinclair/typebox`** / **`zod`** | JSON Schema generation  | **`schemars`** & **`serde_json`** | ✅ Perfect. RIG auto-generates schemas directly from Rust `struct`s.                   |
| **`@aws-sdk/client-bedrock`**       | Hitting AWS AI models   | **`aws-sdk-bedrockruntime`**      | ✅ Official AWS crate.                                                                 |
| **`croner`**                        | Scheduled tasks         | **`tokio-cron-scheduler`**        | ✅ Native async Rust cron.                                                             |

### B. Document & Web Parsing (The "Eyes")

Rust is significantly faster at parsing large documents and HTML than Node.js.

| OpenClaw Dependency (TS)   | Purpose                   | Rust Equivalent                  | Status                                                                  |
| :------------------------- | :------------------------ | :------------------------------- | :---------------------------------------------------------------------- |
| **`@mozilla/readability`** | Extracting article text   | **`readability`** (crate)        | ✅ Direct Rust port of Mozilla's library.                               |
| **`linkedom`**             | HTML DOM parsing          | **`scraper`**                    | ✅ Extremely fast HTML parsing in Rust.                                 |
| **`pdfjs-dist`**           | Reading PDF text          | **`pdf-extract`** or **`lopdf`** | 🟡 Good. Solid, though slightly less flexible than Mozilla's JS viewer. |
| **`file-type`**            | Detecting file signatures | **`infer`** or **`tree_magic`**  | ✅ Standard native crates.                                              |
| **`markdown-it`**          | Markdown parsing          | **`pulldown-cmark`**             | ✅ The gold standard Rust markdown parser.                              |

### C. Media & Heavy Computation

This is where the TS agent struggles, and Rust shines. In Node.js, these operations block the main thread unless moved to workers. In Rust, `tokio` handles them asynchronously.

| OpenClaw Dependency (TS) | Purpose                      | Rust Equivalent                                | Status                                                                                                |
| :----------------------- | :--------------------------- | :--------------------------------------------- | :---------------------------------------------------------------------------------------------------- |
| **`sharp`**              | Image resizing/compression   | **`image`**                                    | ✅ Pure Rust. Eliminates the need for `libvips` C-bindings.                                           |
| **`sqlite-vec`**         | Vector memory for embeddings | **`rusqlite` + `sqlite-vec`**                  | ✅ Load the exact same SQLite native extension via `rusqlite`.                                        |
| **`playwright-core`**    | Browser automation           | **`playwright-rust`** or **`headless_chrome`** | 🟡 Good. `headless_chrome` is fast, but slightly less robust than Microsoft's official JS Playwright. |
| **`node-edge-tts`**      | Text to speech               | **`edge-tts`**                                 | ✅ Direct Rust port exists.                                                                           |
| **`jszip`**              | Zipping files                | **`zip`**                                      | ✅ Standard compression crate.                                                                        |

**The Verdict**: 100% of the Pi Agent's dependencies have robust, production-ready Rust equivalents. The only missing pieces are the custom business logic layers (context compaction, fallback routing) mapped out in Section 2 above.
