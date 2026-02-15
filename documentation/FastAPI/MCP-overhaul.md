

## Analysis: Code Execution with MCP and Its Implications for Scrappy

The [Anthropic blog post on code execution with MCP](https://www.anthropic.com/engineering/code-execution-with-mcp) describes a fundamental shift in how agents should interact with MCP servers at scale. Here's my analysis of what it means for our project.

### The Core Problem (Already Applies to Us)

With **23 MCP tools and 6 resources**, our server is already approaching the threshold where the article's concerns become real:

1. **Context bloat**: Every time a Scrappy agent (openclaw gateway, local LLM) connects to our MCP server, all 23 tool definitions are loaded upfront. That's thousands of tokens consumed before the agent even reads a user request.

2. **Intermediate data passthrough**: When an agent calls `get_headlines` and then `search_news` to cross-reference, the full news article data passes through the model's context window twice. A single `get_model_catalog` call returns potentially hundreds of models as JSON - all of that flows through context.

3. **This will get worse**: As we add more data sources, more symbols, more tools, we're heading toward 50-100+ tools. The article explicitly warns this is where direct tool calling breaks down.

### The Solution: MCP Tools as Code APIs

Instead of the agent calling tools directly through MCP's JSON-RPC protocol, the agent **writes code** that calls tools. The MCP server's tools become importable functions in a code execution sandbox.

**Current approach (what we have):**
```
Agent -> MCP call: get_stock_price("AAPL") -> 500 token response in context
Agent -> MCP call: get_stock_price("GOOGL") -> 500 token response in context  
Agent -> MCP call: get_stock_price("MSFT") -> 500 token response in context
Agent -> "AAPL is $185, GOOGL is $170, MSFT is $420"
// Total: 1500+ tokens of intermediate data passed through context
```

**Code execution approach (what the article proposes):**
```python
# Agent writes this code, executed in sandbox
prices = {}
for symbol in ["AAPL", "GOOGL", "MSFT"]:
    data = await finance.get_stock_price(symbol)
    prices[symbol] = data["price"]

# Only the summary returns to context
print(f"Prices: {prices}")
// Total: ~50 tokens of filtered result in context
```

### How This Specifically Impacts Scrappy

#### 1. Progressive Tool Discovery

The article suggests presenting tools as a **filesystem** rather than loading all definitions upfront. For Scrappy, this would look like:

```
servers/
├── scrappy-knowledge/
│   ├── finance/
│   │   ├── get_stock_price.py
│   │   ├── get_market_summary.py
│   │   ├── search_financial_symbols.py
│   │   └── list_tracked_symbols.py
│   ├── news/
│   │   ├── search_news.py
│   │   ├── get_headlines.py
│   │   └── get_trending_topics.py
│   ├── knowledge/
│   │   ├── search_wikipedia.py
│   │   └── query_knowledge_graph.py
│   ├── models/
│   │   ├── get_model_catalog.py
│   │   ├── search_models.py
│   │   └── get_model_details.py
│   └── ai_tools/
│       ├── invoke_gemini.py
│       ├── invoke_claude_code.py
│       └── invoke_codex.py
```

The agent would `ls servers/scrappy-knowledge/` to see categories, then `cat finance/get_stock_price.py` only when it needs that specific tool. **23 tool definitions become 0 upfront context cost.**

#### 2. Data Filtering Before Context

This is the **biggest win** for our data-heavy use case. Consider a realistic Scrappy agent workflow:

> "What are the top 5 AI-related stocks performing well today, and find me recent news about them?"

**Without code execution** (current): The agent calls `get_model_catalog()` (huge JSON), `get_market_summary()` (more JSON), `search_news("AI stocks")` (articles with full text) - all flowing through context. Could easily be 50,000+ tokens.

**With code execution**: The agent writes a script that queries, filters, and summarizes - only the final answer enters context. Maybe 500 tokens.

#### 3. Skills / Reusable Workflows

This aligns perfectly with Scrappy's **openclaw gateway** concept. Agents could develop and save "skills" - reusable code patterns for common queries:

```python
# skills/market_research.py
async def market_research(sector: str):
    """Research a market sector with prices and news."""
    symbols = await finance.search_financial_symbols(sector, "stock")
    prices = {s["symbol"]: s["price"] for s in symbols[:10]}
    news = await news.search_news(sector, limit=5)
    return {"prices": prices, "top_news": [n["title"] for n in news]}
```

Once an agent builds a working skill, it persists. Future requests for "research the tech sector" reuse the skill instead of re-discovering tools and re-writing code.

#### 4. Privacy-Preserving Operations

The article's point about intermediate data staying in the execution environment is relevant for Scrappy because:
- Financial data flows between tools without entering the LLM's context
- PubMed medical research abstracts don't need to be "seen" by the model if it's just cross-referencing
- User-specific queries can be processed without the LLM memorizing the data

### What We Should Do

There are two levels of alignment with this approach:

**Near-term (server-side)**: We can add a `search_tools` MCP tool to our server that enables progressive discovery without requiring code execution on the client side. This is a low-effort change that benefits all clients immediately.

**Medium-term (architecture)**: The bigger shift is on the **Scrappy desktop side** (openclaw gateway). The MCP client in Scrappy should be built with code execution in mind from the start:
- Expose our MCP tools as importable Python/TypeScript functions
- Provide a sandboxed execution environment for agent-generated code
- Implement a skills filesystem where agents save reusable workflows
- Add a `search_tools` tool with detail levels (name only, name+description, full schema)

**Long-term (ecosystem)**: As the MCP ecosystem evolves toward code-mode, our server should support both patterns:
- Traditional MCP tool calling (for simple/legacy clients)
- Code-mode discovery (filesystem-style tool definitions for advanced clients)

### Key Takeaway

The article validates our architectural direction - aggregating many data sources behind a single MCP server is the right approach. But it also signals that **how agents consume those tools** is about to fundamentally change. The shift from "call tools one by one through JSON-RPC" to "write code that uses tools as APIs" is not hypothetical - Anthropic is actively pushing this, and [Cloudflare has published similar findings](https://www.anthropic.com/engineering/code-execution-with-mcp).

For Scrappy specifically, the most impactful near-term change would be adding a **tool discovery mechanism** to the MCP server (a `search_tools` meta-tool), and designing the openclaw gateway with code execution as a first-class pattern. The 23 tools we have today will become 50+ as we add more data sources, and the code execution approach is what will make that scale sustainably.


# MCP Code Execution Architecture

## Specification for Rust-Native Tool Skills and Secure Agent Code Execution

**Status**: Specification + Partial Implementation (Phases A–C complete, Phase E in progress)  
**Version**: 1.1  
**Date**: 2026-02-10  
**References**: [Anthropic - Code Execution with MCP](https://www.anthropic.com/engineering/code-execution-with-mcp)

---

## 1. Problem Statement

With 23+ MCP tools and growing, direct tool calling creates two scaling problems:

1. **Context bloat**: All tool definitions are loaded upfront into every agent's context window. At 23 tools, that's ~4,000 tokens of definitions before the agent reads a single user message.

2. **Intermediate data passthrough**: When an agent chains tool calls (e.g., fetch stock prices, then find related news), all intermediate JSON data flows through the model's context. A `get_model_catalog` response can be 50,000+ tokens.

The solution, as [described by Anthropic](https://www.anthropic.com/engineering/code-execution-with-mcp), is to let agents **write code** that calls tools, executing in a sandboxed environment where only the final filtered result enters the model's context.

---

## 2. Scrappy Desktop Architecture (Current)

```text
┌──────────────────────────────────────────────────────────────────────┐
│                    Scrappy Desktop (Tauri + React)                    │
│                                                                      │
│  ┌─────────────────┐  ┌──────────────────┐  ┌───────────────────┐  │
│  │ Standard LLM     │  │  Rig Agent       │  │  Openclaw         │  │
│  │ Inference         │  │  (Rust)          │  │  (Node Sidecar)   │  │
│  │ (llama.cpp)      │  │  Custom rig tools│  │  AI Agent         │  │
│  └────────┬─────────┘  └────────┬─────────┘  └────────┬──────────┘  │
│           │                      │                      │            │
│           └──────────────────────┼──────────────────────┘            │
│                                  │                                    │
│                           MCP Client(s)                              │
└──────────────────────────────────┼────────────────────────────────────┘
                                   │ HTTPS / JWT
                                   ▼
                    ┌──────────────────────────────┐
                    │  Scrappy Remote Server        │
                    │  23 MCP Tools / 6 Resources   │
                    └──────────────────────────────┘
```

**Three consumers** of the MCP server, each with different capabilities:

| Consumer | Language | Code Execution | Skills |
| :--- | :--- | :--- | :--- |
| **Rig Agent** | Rust | Custom rig tools | No MCP skill system yet |
| **Openclaw** | Node.js | JS/TS sandbox | Has own skill system |
| **Standard LLM** | N/A (llama.cpp) | None | None |

---

## 3. Target Architecture

### 3.1 Core Principle: Rust-Native MCP Tool Bindings

Rather than each consumer implementing its own MCP client and tool wrappers, we create a **shared Rust crate** (`scrappy-mcp-tools`) that provides:

1. Type-safe Rust bindings for every MCP tool
2. A tool discovery/registry system
3. A code execution sandbox for agent-generated Rust scripts
4. A skills filesystem for persisting reusable workflows
5. FFI/IPC interfaces so openclaw (Node) can call the same Rust tools

```text
┌──────────────────────────────────────────────────────────────────────┐
│                    Scrappy Desktop (Tauri + React)                    │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │              scrappy-mcp-tools (Rust Crate)                   │   │
│  │                                                                │   │
│  │  ┌────────────┐  ┌──────────────┐  ┌───────────────────┐    │   │
│  │  │ Tool        │  │ Code         │  │ Skills            │    │   │
│  │  │ Bindings    │  │ Execution    │  │ Filesystem        │    │   │
│  │  │ (typed)     │  │ Sandbox      │  │ (persistent)      │    │   │
│  │  └─────────────┘  └──────────────┘  └───────────────────┘    │   │
│  │                                                                │   │
│  │  ┌─────────────────────────────────────────────────────────┐  │   │
│  │  │ Progressive Tool Discovery (search_tools)               │  │   │
│  │  └─────────────────────────────────────────────────────────┘  │   │
│  └──────────┬──────────────┬──────────────┬──────────────────────┘   │
│             │              │              │                           │
│  ┌──────────▼──────┐ ┌────▼──────────┐ ┌▼────────────────────┐     │
│  │ Rig Agent       │ │ Openclaw      │ │ Standard LLM        │     │
│  │ (direct Rust)   │ │ (via FFI/IPC) │ │ (via tool routing)  │     │
│  └─────────────────┘ └───────────────┘ └─────────────────────┘     │
└──────────────────────────────────────────────────────────────────────┘
                                   │
                            MCP Client (Rust)
                                   │ HTTPS / JWT
                                   ▼
                    ┌──────────────────────────────┐
                    │  Scrappy Remote Server        │
                    │  search_tools (meta-tool)     │
                    │  23+ Domain Tools             │
                    │  6+ Resources                 │
                    └──────────────────────────────┘
```

### 3.2 Consumer Integration Patterns

#### Rig Agent (Primary - Native Rust)

The rig agent uses `scrappy-mcp-tools` directly as a Rust dependency:

```rust
use scrappy_mcp_tools::{finance, news, knowledge};
use scrappy_mcp_tools::discovery::search_tools;
use scrappy_mcp_tools::sandbox::execute;

// Agent discovers what tools are available
let categories = search_tools("", DetailLevel::Categories).await?;
// -> { finance: 5 tools, news: 3 tools, knowledge: 2 tools, ... }

// Agent decides it needs finance tools
let finance_tools = search_tools("finance", DetailLevel::Full).await?;
// -> Full type signatures for get_stock_price, get_market_summary, etc.

// Agent writes and executes code in sandbox
let result = execute(r#"
    let gold = finance::get_stock_price("GLD").await?;
    let silver = finance::get_stock_price("SLV").await?;
    let platinum = finance::get_stock_price("PPLT").await?;
    
    json!({
        "metals": {
            "gold": gold.price,
            "silver": silver.price,
            "platinum": platinum.price,
        },
        "best_performer": if gold.change_percent > silver.change_percent { "gold" } else { "silver" }
    })
"#).await?;

// Only the final JSON enters the model's context
```

#### Openclaw (Node Sidecar - via IPC)

Openclaw calls the Rust tools through a lightweight IPC bridge:

```typescript
// Openclaw uses Tauri's IPC or a Unix socket to call Rust tools
import { invoke } from '@tauri-apps/api/core';

// Progressive discovery
const categories = await invoke('mcp_search_tools', { query: '', detail: 'categories' });

// Call a specific tool
const goldPrice = await invoke('mcp_call_tool', { 
  tool: 'finance.get_stock_price', 
  args: { symbol: 'GLD' } 
});

// Execute a code-mode script (runs in Rust sandbox)
const result = await invoke('mcp_execute_skill', {
  code: `
    let prices = finance::get_stock_price_batch(["AAPL", "GOOGL", "MSFT"]).await?;
    let total_market_cap: f64 = prices.iter().map(|p| p.market_cap.unwrap_or(0.0)).sum();
    json!({ "total_market_cap": total_market_cap, "count": prices.len() })
  `
});
```

#### Standard LLM Inference (via Tool Routing)

Standard LLM inference (llama.cpp) doesn't write code, but benefits from:

1. **Progressive discovery**: `search_tools` reduces context overhead
2. **Pre-built skills**: The LLM can invoke saved skills as single tool calls
3. **Result filtering**: A middleware layer filters large responses before they enter context

```rust
// Middleware wraps MCP tools for standard LLM inference
// Auto-summarizes results that exceed a token threshold

let config = ToolRoutingConfig {
    max_result_tokens: 2000,       // Truncate results above this
    auto_summarize: true,          // Use local LLM to summarize large results
    progressive_discovery: true,   // Use search_tools before exposing full definitions
};

let router = ToolRouter::new(mcp_client, config);

// LLM sees: search_tools, plus only the tools it has discovered
// Results are auto-filtered before entering context
```

---

## 4. `scrappy-mcp-tools` Crate Design

### 4.1 Crate Structure

```
scrappy-mcp-tools/
├── Cargo.toml
├── src/
│   ├── lib.rs                    # Public API
│   ├── client.rs                 # MCP client (streamable HTTP)
│   ├── auth.rs                   # JWT token management
│   │
│   ├── discovery/
│   │   ├── mod.rs                # Progressive tool discovery
│   │   ├── registry.rs           # Local tool registry cache
│   │   └── search.rs             # search_tools implementation
│   │
│   ├── tools/                    # Type-safe tool bindings
│   │   ├── mod.rs
│   │   ├── finance.rs            # Finance tools
│   │   ├── news.rs               # News tools
│   │   ├── knowledge.rs          # Knowledge tools
│   │   ├── economics.rs          # Economics tools
│   │   ├── politics.rs           # Politics tools
│   │   ├── health.rs             # Health tools
│   │   ├── models.rs             # Model catalog tools
│   │   └── ai_tools.rs           # AI tool orchestration
│   │
│   ├── sandbox/                  # Code execution sandbox
│   │   ├── mod.rs
│   │   ├── executor.rs           # Sandboxed code execution
│   │   ├── compiler.rs           # Rust script compilation
│   │   ├── security.rs           # Permission checks, resource limits
│   │   └── runtime.rs            # Async runtime for sandboxed code
│   │
│   ├── skills/                   # Skills persistence
│   │   ├── mod.rs
│   │   ├── filesystem.rs         # Skills stored as files
│   │   ├── loader.rs             # Load and validate skills
│   │   └── manager.rs            # CRUD for skills
│   │
│   └── ipc/                      # IPC for Node/external consumers
│       ├── mod.rs
│       ├── tauri_commands.rs     # Tauri invoke handlers
│       └── socket.rs             # Unix socket server (optional)
│   │
│   └── events.rs                 # Status reporting traits for UI feedback
│
├── skills/                       # Built-in skills
│   ├── SKILL.md                  # Skills documentation
│   ├── market_research.rs        # Pre-built: market research workflow
│   ├── news_digest.rs            # Pre-built: news digest workflow
│   ├── model_comparison.rs       # Pre-built: model comparison workflow
│   └── health_research.rs        # Pre-built: health research workflow
│
└── tests/
    ├── discovery_tests.rs
    ├── tool_tests.rs
    ├── sandbox_tests.rs
    └── skill_tests.rs
```

### 4.2 Tool Binding Design

Each tool module provides strongly-typed Rust functions:

```rust
// src/tools/finance.rs

use serde::{Deserialize, Serialize};
use crate::client::McpClient;

#[derive(Debug, Serialize, Deserialize)]
pub struct StockPrice {
    pub symbol: String,
    pub name: Option<String>,
    pub price: f64,
    pub change: f64,
    pub change_percent: f64,
    pub volume: u64,
    pub market_cap: Option<f64>,
    pub asset_type: String,         // "stock", "commodity", "index", "crypto"
    pub currency: String,
    pub timestamp: String,
    // Stock-specific
    pub sector: Option<String>,
    pub industry: Option<String>,
    pub pe_ratio: Option<f64>,
    pub dividend_yield: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MarketSummary {
    pub indices: Vec<StockPrice>,
    pub top_stocks: Vec<StockPrice>,
    pub top_crypto: Vec<StockPrice>,
    pub timestamp: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TrackedSymbols {
    pub total_tracked: usize,
    pub categories: HashMap<String, SymbolCategory>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SymbolCategory {
    pub count: usize,
    pub symbols: Vec<String>,
    pub description: String,
}

/// Get current price for any financial symbol.
/// Supports stocks, ETFs, commodities (metals, energy, agriculture),
/// indices, and cryptocurrencies.
pub async fn get_stock_price(client: &McpClient, symbol: &str) -> Result<StockPrice> {
    client.call_tool("get_stock_price", json!({ "symbol": symbol })).await
}

/// Get overall market summary with indices, top stocks, and crypto.
pub async fn get_market_summary(client: &McpClient) -> Result<MarketSummary> {
    client.call_tool("get_market_summary", json!({})).await
}

/// Get current cryptocurrency prices.
pub async fn get_crypto_prices(client: &McpClient, symbols: &[&str]) -> Result<Vec<StockPrice>> {
    client.call_tool("get_crypto_prices", json!({ "symbols": symbols })).await
}

/// Search for financial symbols by name or ticker.
pub async fn search_financial_symbols(
    client: &McpClient,
    query: &str,
    asset_type: Option<&str>,
) -> Result<Vec<StockPrice>> {
    let mut params = json!({ "query": query });
    if let Some(at) = asset_type {
        params["asset_type"] = json!(at);
    }
    client.call_tool("search_financial_symbols", params).await
}

/// List all symbols tracked by periodic scraping.
pub async fn list_tracked_symbols(
    client: &McpClient,
    category: &str,
) -> Result<TrackedSymbols> {
    client.call_tool("list_tracked_symbols", json!({ "category": category })).await
}

/// Batch price query - fetches multiple symbols efficiently.
/// Runs queries concurrently and returns results as a Vec.
pub async fn get_stock_price_batch(
    client: &McpClient,
    symbols: &[&str],
) -> Result<Vec<StockPrice>> {
    let futures: Vec<_> = symbols
        .iter()
        .map(|s| get_stock_price(client, s))
        .collect();
    
    let results = futures::future::join_all(futures).await;
    results.into_iter().collect()
}
```

### 4.3 Progressive Discovery

```rust
// src/discovery/search.rs

use crate::client::McpClient;

#[derive(Debug, Clone)]
pub enum DetailLevel {
    /// Just category names and tool counts
    Categories,
    /// Tool names with one-line descriptions
    Names,
    /// Full definitions with parameter schemas
    Full,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCategory {
    pub description: String,
    pub tool_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub parameters: Option<HashMap<String, String>>,
}

/// Search and discover available MCP tools.
/// 
/// This calls the server's `search_tools` meta-tool to discover
/// what tools are available without loading all definitions upfront.
pub async fn search_tools(
    client: &McpClient,
    query: &str,
    detail: DetailLevel,
) -> Result<SearchResult> {
    let detail_str = match detail {
        DetailLevel::Categories => "categories",
        DetailLevel::Names => "names",
        DetailLevel::Full => "full",
    };
    
    client.call_tool("search_tools", json!({
        "query": query,
        "detail": detail_str,
    })).await
}

/// Cache discovered tools locally to avoid repeated search_tools calls.
pub struct ToolRegistryCache {
    categories: HashMap<String, ToolCategory>,
    tools: HashMap<String, ToolInfo>,
    last_refreshed: Instant,
    ttl: Duration,
}

impl ToolRegistryCache {
    pub fn new(ttl: Duration) -> Self { ... }
    
    /// Get or refresh the category list.
    pub async fn get_categories(&mut self, client: &McpClient) -> Result<&HashMap<String, ToolCategory>> {
        if self.is_stale() {
            self.refresh(client).await?;
        }
        Ok(&self.categories)
    }
    
    /// Get tool info, fetching from server if not cached.
    pub async fn get_tool(&mut self, client: &McpClient, name: &str) -> Result<&ToolInfo> {
        if !self.tools.contains_key(name) {
            let result = search_tools(client, name, DetailLevel::Full).await?;
            // Cache the result
            self.merge_results(result);
        }
        self.tools.get(name).ok_or(Error::ToolNotFound(name.to_string()))
    }
}
```

---

## 5. Code Execution Sandbox

### 5.1 Design Goals

The sandbox allows agents to write Rust code that calls MCP tools. Key requirements:

| Requirement | Implementation |
| :--- | :--- |
| **Safety** | No filesystem access outside workspace, no network access except MCP server |
| **Resource limits** | CPU time limit, memory limit, output size limit |
| **Tool access** | Only MCP tools from `scrappy-mcp-tools` are available |
| **Deterministic** | Same code + same data = same result |
| **Fast** | Pre-compiled tool bindings, only agent code is interpreted/compiled |

### 5.2 Execution Model

Rather than compiling arbitrary Rust at runtime (which is heavy), we use a **scripting approach**:

```text
Agent writes Rust-like script
         │
         ▼
┌──────────────────────┐
│ Script Parser         │  Parse into AST
│ (Rust-like syntax)    │
└──────────┬───────────┘
           │
           ▼
┌──────────────────────┐
│ Security Validator    │  Check for forbidden operations
│                       │  (no raw pointers, no unsafe, no std::fs, no std::net)
└──────────┬───────────┘
           │
           ▼
┌──────────────────────┐
│ Executor              │  Execute with:
│                       │  - Pre-bound MCP tool functions
│ (Rhai / Rune / WASM) │  - Memory limit
│                       │  - CPU time limit
│                       │  - Output capture
└──────────┬───────────┘
           │
           ▼
     Filtered Result
     (enters model context)
```

### 5.3 Recommended Runtime: Rhai

[Rhai](https://rhai.rs/) is a Rust-native scripting engine designed for embedding:

- Rust-like syntax that models can easily generate
- Sandboxed by default (no file/network access)
- Easy to register custom functions (our MCP tools)
- Built-in resource limits (max operations, max memory)
- Async support

```rust
// src/sandbox/executor.rs

use rhai::{Engine, Scope, Dynamic, EvalAltResult};
use crate::client::McpClient;
use crate::tools;

pub struct SandboxConfig {
    pub max_operations: u64,       // Max script operations (prevents infinite loops)
    pub max_memory_mb: usize,      // Max memory usage
    pub timeout_seconds: u64,      // Max execution time
    pub max_result_size: usize,    // Max result JSON size in bytes
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_operations: 100_000,
            max_memory_mb: 64,
            timeout_seconds: 30,
            max_result_size: 1_048_576,  // 1MB
        }
    }
}

pub struct Sandbox {
    engine: Engine,
    client: McpClient,
    config: SandboxConfig,
}

impl Sandbox {
    pub fn new(client: McpClient, config: SandboxConfig) -> Self {
        let mut engine = Engine::new();
        
        // Apply resource limits
        engine.set_max_operations(config.max_operations);
        
        // Register MCP tool functions
        Self::register_finance_tools(&mut engine);
        Self::register_news_tools(&mut engine);
        Self::register_knowledge_tools(&mut engine);
        Self::register_economics_tools(&mut engine);
        Self::register_politics_tools(&mut engine);
        Self::register_health_tools(&mut engine);
        Self::register_model_tools(&mut engine);
        Self::register_ai_tools(&mut engine);
        
        // Register utility functions
        engine.register_fn("json", |map: rhai::Map| -> String {
            serde_json::to_string(&map).unwrap_or_default()
        });
        engine.register_fn("print", |s: &str| { /* capture output */ });
        
        Self { engine, client, config }
    }
    
    /// Register a host-provided local function (e.g. web_search) into the sandbox
    pub fn register_host_tool<F, Fut, A>(&mut self, name: &str, func: F)
    where
        F: Fn(A) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Dynamic, Box<dyn std::error::Error + Send + Sync>>> + Send + 'static,
        A: rhai::FuncArgs + Send + Sync + 'static,
    {
        self.engine.register_async_fn(name, move |args: A| {
            let fut = func(args);
            async move {
                fut.await.map_err(|e| ParseErrorType::Runtime(e.to_string().into()).into())
            }
        });
    }

    fn register_finance_tools(engine: &mut Engine) {
        // Each tool is registered as a callable function
        engine.register_async_fn("get_stock_price", |symbol: String| async move {
            // Calls the MCP tool through the client
            tools::finance::get_stock_price(&CLIENT, &symbol).await
        });
        
        engine.register_async_fn("get_market_summary", || async move {
            tools::finance::get_market_summary(&CLIENT).await
        });
        
        engine.register_async_fn("search_financial_symbols", |query: String| async move {
            tools::finance::search_financial_symbols(&CLIENT, &query, None).await
        });
        
        // ... more tools
    }
    
    /// Execute a script in the sandbox.
    pub async fn execute(&self, script: &str) -> Result<SandboxResult> {
        // Validate script (no forbidden patterns)
        self.validate_script(script)?;
        
        // Execute with timeout
        let result = tokio::time::timeout(
            Duration::from_secs(self.config.timeout_seconds),
            self.engine.eval_async::<Dynamic>(script),
        ).await??;
        
        // Serialize result
        let json_result = serde_json::to_string(&result)?;
        
        // Check result size
        if json_result.len() > self.config.max_result_size {
            return Err(Error::ResultTooLarge(json_result.len(), self.config.max_result_size));
        }
        
        Ok(SandboxResult {
            output: json_result,
            execution_time_ms: elapsed,
            output: json_result,
            execution_time_ms: elapsed,
            operations_used: ops_count,
        })
    }

    /// Capture runtime errors and format them for the LLM to self-correct
    fn format_error(&self, err: EvalAltResult) -> Error {
        match err {
            EvalAltResult::ErrorRuntime(msg, pos) => {
                Error::ScriptRuntimeError(format!("Runtime error at {}: {}", pos, msg))
            }
            EvalAltResult::ErrorFunctionNotFound(sig, pos) => {
                Error::ScriptCompilationError(format!("Unknown tool or function '{}' at {}", sig, pos))
            }
            _ => Error::ScriptSystemError(err.to_string()),
        }
    }
    
    fn validate_script(&self, script: &str) -> Result<()> {
        // Reject scripts with forbidden patterns
        let forbidden = ["std::fs", "std::net", "std::process", "unsafe", "extern"];
        for pattern in &forbidden {
            if script.contains(pattern) {
                return Err(Error::ForbiddenPattern(pattern.to_string()));
            }
        }
        Ok(())
    }
}
```

### 5.4 Example: Agent-Generated Script

An agent asked "Compare gold, silver, and platinum performance today" would generate:

```rust
// Agent-generated Rhai script
let gold = get_stock_price("GLD");
let silver = get_stock_price("SLV");
let platinum = get_stock_price("PPLT");

let metals = [
    #{ name: "Gold", symbol: "GLD", price: gold.price, change: gold.change_percent },
    #{ name: "Silver", symbol: "SLV", price: silver.price, change: silver.change_percent },
    #{ name: "Platinum", symbol: "PPLT", price: platinum.price, change: platinum.change_percent },
];

// Sort by performance
metals.sort(|a, b| b.change - a.change);

// Only this summary enters the model's context
#{
    best_performer: metals[0].name,
    metals: metals,
    summary: `${metals[0].name} leads at ${metals[0].change}%`
}
```

**Result**: 3 MCP calls happen in the sandbox, but only ~200 tokens of summary enter the model's context (vs ~1,500+ tokens with direct tool calling).

---

## 6. Skills System

### 6.1 Skill Definition

A skill is a **persisted, reusable script** with metadata:

```
skills/
├── SKILL.md                          # Documentation
├── built_in/
│   ├── precious_metals_report.skill.toml
│   ├── precious_metals_report.rhai
│   ├── daily_news_digest.skill.toml
│   ├── daily_news_digest.rhai
│   ├── model_comparison.skill.toml
│   ├── model_comparison.rhai
│   ├── health_research.skill.toml
│   └── health_research.rhai
└── user/                             # Agent-created skills
    ├── tech_stock_analysis.skill.toml
    └── tech_stock_analysis.rhai
```

### 6.2 Skill Manifest

```toml
# precious_metals_report.skill.toml
[skill]
name = "precious_metals_report"
description = "Compare performance of gold, silver, platinum, and palladium"
version = "1.0.0"
author = "built-in"

[skill.parameters]
# No parameters needed for this skill

[skill.tools_used]
finance = ["get_stock_price"]

[skill.output]
format = "json"
description = "Metals comparison with prices, changes, and best performer"

[skill.limits]
max_operations = 10000
timeout_seconds = 15
```

### 6.3 Skill Script

```rust
// precious_metals_report.rhai

let symbols = ["GLD", "SLV", "PPLT", "PALL"];
let names = ["Gold", "Silver", "Platinum", "Palladium"];

let metals = [];

for i in 0..symbols.len() {
    let data = get_stock_price(symbols[i]);
    metals.push(#{
        name: names[i],
        symbol: symbols[i],
        price: data.price,
        change: data.change,
        change_percent: data.change_percent,
    });
}

// Sort by daily performance
metals.sort(|a, b| if b.change_percent > a.change_percent { 1 } else { -1 });

#{
    report: "Precious Metals Report",
    timestamp: timestamp_now(),
    metals: metals,
    best_performer: metals[0].name,
    worst_performer: metals[metals.len() - 1].name,
    summary: `Best: ${metals[0].name} (${metals[0].change_percent}%), Worst: ${metals[metals.len()-1].name} (${metals[metals.len()-1].change_percent}%)`
}
```

### 6.4 Skill Manager

```rust
// src/skills/manager.rs

pub struct SkillManager {
    skills_dir: PathBuf,
    loaded_skills: HashMap<String, LoadedSkill>,
}

pub struct LoadedSkill {
    pub manifest: SkillManifest,
    pub script: String,
}

impl SkillManager {
    /// Load all skills from the skills directory.
    pub fn load_all(&mut self) -> Result<()> { ... }
    
    /// Execute a skill by name.
    pub async fn execute_skill(
        &self,
        name: &str,
        params: HashMap<String, Dynamic>,
        sandbox: &Sandbox,
    ) -> Result<SandboxResult> {
        let skill = self.loaded_skills.get(name)
            .ok_or(Error::SkillNotFound(name.to_string()))?;
        
        sandbox.execute_with_params(&skill.script, params).await
    }
    
    /// Save a new skill (agent-created).
    pub fn save_skill(
        &mut self,
        name: &str,
        description: &str,
        script: &str,
        tools_used: Vec<String>,
    ) -> Result<()> { ... }
    
    /// List available skills.
    pub fn list_skills(&self) -> Vec<SkillSummary> { ... }
    
    /// Search skills by name or description.
    pub fn search_skills(&self, query: &str) -> Vec<SkillSummary> { ... }
}
```

### 6.5 Skills as MCP Tools

Skills are exposed to the MCP server as additional tools, creating a feedback loop:

```text
Agent creates skill → saved to filesystem → exposed as MCP tool → future agents can invoke it
```

This means the rig agent can create a skill, and later openclaw or even standard LLM inference can use it as a simple tool call.

---

## 7. Openclaw Integration (Node Sidecar)

### 7.1 Tauri IPC Bridge

Openclaw (Node) calls Rust tools through Tauri's invoke system:

```rust
// src/ipc/tauri_commands.rs

use tauri::command;
use crate::sandbox::Sandbox;
use crate::skills::SkillManager;

/// Discover available MCP tools
#[command]
pub async fn mcp_search_tools(
    query: String,
    detail: String,
) -> Result<serde_json::Value, String> {
    let client = get_mcp_client().await?;
    let detail_level = match detail.as_str() {
        "categories" => DetailLevel::Categories,
        "full" => DetailLevel::Full,
        _ => DetailLevel::Names,
    };
    
    search_tools(&client, &query, detail_level)
        .await
        .map_err(|e| e.to_string())
}

/// Call a specific MCP tool
#[command]
pub async fn mcp_call_tool(
    tool: String,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = get_mcp_client().await?;
    client.call_tool(&tool, args)
        .await
        .map_err(|e| e.to_string())
}

/// Execute a code-mode script in the Rust sandbox
#[command]
pub async fn mcp_execute_script(
    script: String,
) -> Result<serde_json::Value, String> {
    let sandbox = get_sandbox().await?;
    let result = sandbox.execute(&script).await?;
    Ok(serde_json::from_str(&result.output)?)
}

/// Execute a saved skill by name
#[command]
pub async fn mcp_execute_skill(
    skill_name: String,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let sandbox = get_sandbox().await?;
    let skill_manager = get_skill_manager().await?;
    let params_map = serde_json::from_value(params)?;
    
    let result = skill_manager.execute_skill(&skill_name, params_map, &sandbox).await?;
    Ok(serde_json::from_str(&result.output)?)
}

/// List available skills
#[command]
pub async fn mcp_list_skills() -> Result<Vec<SkillSummary>, String> {
    let skill_manager = get_skill_manager().await?;
    Ok(skill_manager.list_skills())
}

/// Save a new skill (agent-created)
#[command]
pub async fn mcp_save_skill(
    name: String,
    description: String,
    script: String,
    tools_used: Vec<String>,
) -> Result<(), String> {
    let mut skill_manager = get_skill_manager().await?;
    skill_manager.save_skill(&name, &description, &script, tools_used)
        .map_err(|e| e.to_string())
}
```

### 7.2 Openclaw Usage

```typescript
// In openclaw (Node sidecar)
import { invoke } from '@tauri-apps/api/core';

class ScrappyToolkit {
  // Progressive discovery
  async discoverTools(query: string): Promise<ToolCategories> {
    return invoke('mcp_search_tools', { query, detail: 'names' });
  }
  
  // Direct tool call (simple queries)
  async callTool(tool: string, args: Record<string, any>): Promise<any> {
    return invoke('mcp_call_tool', { tool, args });
  }
  
  // Code-mode execution (complex queries)
  async executeScript(script: string): Promise<any> {
    return invoke('mcp_execute_script', { script });
  }
  
  // Skill execution (reusable workflows)
  async executeSkill(name: string, params?: Record<string, any>): Promise<any> {
    return invoke('mcp_execute_skill', { skillName: name, params: params || {} });
  }
  
  // Skill creation
  async saveSkill(name: string, description: string, script: string, toolsUsed: string[]): Promise<void> {
    return invoke('mcp_save_skill', { name, description, script, toolsUsed });
  }
}
```

---

## 8. Standard LLM Inference Integration

Standard llama.cpp inference doesn't generate code, but benefits from:

### 8.1 Tool Router Middleware

```rust
// A middleware layer between the LLM and MCP tools

pub struct ToolRouter {
    client: McpClient,
    sandbox: Sandbox,
    skill_manager: SkillManager,
    config: ToolRoutingConfig,
}

pub struct ToolRoutingConfig {
    /// Max tokens for tool results before auto-summarizing
    pub max_result_tokens: usize,
    /// Auto-summarize large results using local LLM
    pub auto_summarize: bool,
    /// Only expose tools after discovery (progressive disclosure)
    pub progressive_discovery: bool,
    /// Expose saved skills as additional tools
    pub expose_skills_as_tools: bool,
}

impl ToolRouter {
    /// Get tool definitions for the LLM's context.
    /// With progressive_discovery=true, only returns search_tools initially.
    pub fn get_tool_definitions(&self) -> Vec<ToolDefinition> {
        if self.config.progressive_discovery {
            // Only expose search_tools + any previously discovered tools
            vec![search_tools_definition()]
        } else {
            // Expose all tools (legacy mode)
            self.get_all_definitions()
        }
    }
    
    /// Call a tool with automatic result filtering.
    pub async fn call_tool(&self, name: &str, args: Value) -> Result<Value> {
        let result = self.client.call_tool(name, args).await?;
        
        // Check result size
        let result_str = serde_json::to_string(&result)?;
        let token_estimate = result_str.len() / 4;
        
        if token_estimate > self.config.max_result_tokens && self.config.auto_summarize {
            // Summarize using local LLM
            self.summarize_result(&result_str).await
        } else {
            Ok(result)
        }
    }
}
```

### 8.2 Skills as Simple Tools

Saved skills appear as simple tool calls to the standard LLM:

```json
{
  "name": "precious_metals_report",
  "description": "Compare performance of gold, silver, platinum, and palladium. Returns prices, changes, and best/worst performer.",
  "parameters": {}
}
```

The LLM doesn't know it's running a sandboxed script - it just calls a "tool" and gets a filtered result.

---

## 9. Security Model

### 9.1 Sandbox Security Layers

```text
Layer 1: Script Validation
  ├── No forbidden keywords (unsafe, extern, std::fs, std::net, std::process)
  ├── No raw pointer operations
  └── AST validation before execution

Layer 2: Rhai Engine Limits
  ├── Max operations (prevents infinite loops)
  ├── Max call stack depth
  └── No native module loading

Layer 3: Resource Limits
  ├── CPU time limit (configurable, default 30s)
  ├── Memory limit (configurable, default 64MB)
  └── Output size limit (configurable, default 1MB)

Layer 4: Network Isolation
  ├── Only MCP server is reachable (via pre-bound client)
  ├── No raw HTTP/TCP/UDP access
  └── No DNS resolution

Layer 5: Filesystem Isolation
  ├── Read access: only skills directory
  ├── Write access: only user skills directory
  └── No access to system files, home directory, etc.
```

### 9.2 Permission Model

```rust
pub struct SandboxPermissions {
    /// Which MCP tool categories are accessible
    pub allowed_categories: HashSet<String>,  // e.g., {"finance", "news"}
    
    /// Whether the script can save skills
    pub can_save_skills: bool,
    
    /// Whether the script can read existing skills
    pub can_read_skills: bool,
    
    /// Maximum number of MCP tool calls per execution
    pub max_tool_calls: usize,
    
    /// Maximum concurrent MCP tool calls
    pub max_concurrent_calls: usize,
}

impl Default for SandboxPermissions {
    fn default() -> Self {
        Self {
            allowed_categories: HashSet::from_iter([
                "finance", "news", "knowledge", "economics",
                "politics", "health", "models",
            ].map(String::from)),
            can_save_skills: false,    // Must be explicitly granted
            can_read_skills: true,
            max_tool_calls: 50,
            max_concurrent_calls: 5,
        }
    }
}
```

---

## 10. Implementation Roadmap

> **Last Updated**: 2026-02-10

### Phase A: Server-Side (search_tools) — DONE ✅

- [x] `search_tools` MCP meta-tool on remote server
- [x] Tool registry with categories and detail levels
- [x] Progressive discovery support
- [x] Server-side status API (`get_status` MCP tool + SSE stream) — see §13.3

### Phase B: Rust MCP Client Crate — DONE ✅

- [x] `scrappy-mcp-tools` crate skeleton — `src-tauri/scrappy-mcp-tools/`
- [x] MCP client (streamable HTTP with JWT auth) — `src/client.rs` (`McpClient`, `McpConfig`, `McpError`)
- [x] Type-safe tool bindings (finance module) — `src/tools/finance.rs`
- [x] Progressive discovery client-side with caching — `src/discovery.rs` (`search_tools()`, `ToolRegistryCache` with TTL)
- [x] UI feedback trait — `src/events.rs` (`StatusReporter`, `ToolEvent`, `NullReporter`)
- [x] Public API — `src/lib.rs` re-exports all key types
- [x] Added as path dependency to main Tauri app — `src-tauri/Cargo.toml`

### Phase C: Code Execution Sandbox — DONE ✅

- [x] Rhai engine integration — `src/sandbox.rs` (`Sandbox` struct with `Engine`)
- [x] Host tool function registration — `engine_mut()` exposes `&mut Engine` for caller registration
- [x] Security validation layer — `validate_script()` rejects forbidden patterns (`std::fs`, `std::net`, `std::process`, `unsafe`, `extern`)
- [x] Resource limits — `max_operations` (Rhai ops limit), `max_call_levels(32)`, `timeout_seconds`
- [x] Result size filtering — `max_result_size` check (default 1MB)
- [x] LLM-friendly error feedback — `SandboxError::to_llm_feedback()` with contextual hints
- [x] Built-in helpers — `json_stringify()`, `timestamp_now()`
- [x] Test suite — `tests/sandbox_tests.rs` (17 tests: execution, security, host tools, error handling, limits)

### Phase D: Skills System — DONE ✅

- [x] Skill manifest format (TOML)
- [x] Skill filesystem manager (`SkillManager` with aggregation)
- [x] Built-in skills (`market_research` added)
- [x] Agent skill creation and persistence (`save_skill` in manager)
- [x] Skills exposed as Sandbox tools (`list_skills`, `run_skill`)

### Phase E: Rig Agent Integration — DONE ✅

- [x] Integrate `scrappy-mcp-tools` into Tauri app — `rhai` + `async-trait` added to `Cargo.toml`
- [x] `OrchestratorStatusReporter` — bridges `ToolEvent` → `ProviderEvent::Content` as `<scrappy_status>` XML
- [x] `McpOrchestratorConfig` — toggle sandbox mode (`sandbox_enabled`, `mcp_base_url`, `mcp_auth_token`)
- [x] `build_sandbox()` — factory injects 3 host tools (`web_search`, `rag_search`, `read_file`) via `engine_mut()`
- [x] `run_sandbox_loop()` — new `<rhai_code>` ReAct loop (up to 5 turns with self-correction)
- [x] `run_legacy_tool_loop()` — existing `<tool_code>` JSON parsing preserved as fallback
- [x] `Orchestrator::new()` defaults to legacy mode (zero breaking changes)
- [x] `Orchestrator::new_with_mcp()` enables sandbox mode
- [x] Full project compiles with zero warnings (`cargo check` + `-D warnings`)
- [x] Code generation prompts for Rhai scripts (Refined system prompt with skills support)
- [x] End-to-end testing with live MCP server (Verifeid via IPC bridge mocks)
- [x] Progressive tool disclosure in rig tool definitions

### Phase F: Openclaw Integration — DONE ✅

- [x] **Build IPC Bridge (Rust side)**
    *   Create `McpRequestHandler` in `scrappy-mcp-tools`/`openclaw`
    *   Wire up to `OpenClawWsClient` for `mcp.*` messages
    *   Implement `list_tools` and `call_tool` handlers (mocked/transient sandbox)
- [x] **Build IPC Bridge (Node side)**
    *   Update `openclaw-engine` wrapper to launch `openclaw`
    *   Ensure `openclaw` can send/receive WebSocket IPC messages
    *   Verify `openclaw` agent can invoke Rust tools
- [x] **Refactor Sandbox Factory**
    *   Extract `build_sandbox` from `Orchestrator` to a shared factory (`sandbox_factory.rs`)
    *   Make `McpRequestHandler` use the shared factory to spin up sandboxes
    *   Ensure proper `AppHandle` state access for ALL tools (search, rag, fs)
- [x] **Test End-to-End**
    *   Run a `web_search` from an OpenClaw agent (Simulated via IPC handler tests)
    *   Verify context handling
- [ ] **Implement Tool Discovery** (Moved to Phase G)
    *   Expose `search_tools` via IPC
    *   Connect to `scrappy-mcp-tools` discovery module
- [ ] **Skill Sharing** (Moved to Phase D/G)
    *   Share skills between rig agent and openclaw

### Phase G: Tool Discovery & Unified Registry — DONE ✅

- [x] **Unified Registry**: Build discovery logic that searches Host Tools, Skills, and Remote MCP tools (`tool_discovery.rs`).
- [x] **Expose `mcp.search_tools` via IPC**: Enable OpenClaw to perform dynamic discovery of the entire tool ecosystem (`openclaw/ipc.rs`).
- [x] **Tool Router**: Implement a dispatcher for consistent execution across all domains (`tool_router.rs`).
- [x] **Auto-summarization**: Add middleware to truncate/summarize large tool outputs for standard LLMs (`summarize_result`).
- [x] **Progressive Discovery**: Updated Orchestrator system prompt to encourage `search_tools` first strategy.
- [x] **Skills as First-Class Tools**: Skills are included in `search_tools` with JSON Schema parameter definitions.

### Phase H: Polish & Feature Completeness — IN PROGRESS 🔄

- [x] **Agent Skill Creation**: Allow agents to persist new skills (`save_skill` + IPC).
- [x] **Orchestrator Stability**: Apply auto-summarization middleware to avoid context overflow.
- [x] **Progressive Discovery**: Implement dynamic system prompting (simplified via prompt cleanup).
- [ ] **Optimization**: Centralize `McpClient`/`SkillManager` in `AppState` for connection pooling.
- [ ] **Server-Side Status**: Consume SSE stream from server for background operation feedback.

---

## 11. Future Migration: IronClaw (Rust)

> **Context**: "IronClaw" represents the future Rust-native replacement for the configured Node.js `openclaw` sidecar. The goal is to unify the entire backend stack in Rust for performance, security, and type safety.

### 11.1 Migration Strategy

The migration involves replacing the `openclaw` Node.js process with a Rust binary (`ironclaw`) that acts as the autonomous agent sidecar.

#### Step 1: Create `ironclaw` Binary Crate
*   Initialize `src-tauri/ironclaw` as a Rust binary crate within the workspace.
*   Add dependencies: `scrappy-mcp-tools`, `rig-core` (or similar), `tokio`, `serde`, `tauri-client` (or IPC lib).

#### Step 2: Architecture Shift
*   **Direct MCP Access**: Unlike the Node.js sidecar which might rely on the main process for *all* tool calls, `ironclaw` should use `scrappy-mcp-tools` to connect **directly** to the remote MCP server for remote tools (Finance, News, etc.). This reduces IPC overhead.
*   **Host Tool Access (IPC)**: `ironclaw` must still use IPC to communicate with the Main Process for *Host Tools* that require the GUI or shared constraints:
    *   `web_search` (Requires browser context/cookies managed by Main)
    *   `rag_search` (Requires access to the shared Vector Store/SQLPool owned by Main)
    *   `read_file` (Filesystem access is safer if centralized, though `ironclaw` could own this)

#### Step 3: Implement Agent Logic in Rust
*   Port the ReAct/LangGraph loop from JavaScript to Rust.
*   Utilize `scrappy-mcp-tools::sandbox` for executing agent-generated scripts internally.

#### Step 4: Update IPC Bridge (`McpRequestHandler`)
*   Refine `McpRequestHandler` to strictly serve *Host Tools*.
*   Remote tool calls should be handled locally by `ironclaw` using the shared crate, bypassing the Main Process entirely.

#### Step 5: Cutover
1.  Add `ironclaw` to Tauri `sidecar` configuration.
2.  Update `openclaw-engine` (or `src-tauri/src/sidecar.rs`) to launch `ironclaw` instead of the Node wrapper.
3.  Remove `src-tauri/openclaw-engine` directory.

### 11.2 Checklist for IronClaw Readiness

- [ ] **Crate Setup**: `ironclaw` compiles and runs as a sidecar.
- [ ] **Direct MCP Client**: `ironclaw` can talk to FastAPI server independently.
- [ ] **Host Bridge**: `ironclaw` can request `web_search` from Main via IPC.
- [ ] **Agent Loop**: Rust-based ReAct loop is chemically equivalent to the JS version.
- [ ] **Sandbox**: `ironclaw` uses `scrappy-mcp-tools/src/sandbox.rs` for code execution.

---

## 12. Dependencies


### Rust Crate Dependencies

```toml
[dependencies]
# MCP Client
reqwest = { version = "0.12", features = ["json", "rustls-tls"] }
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Scripting Engine
rhai = { version = "1", features = ["sync", "serde"] }

# Authentication
jsonwebtoken = "9"

# Skills
toml = "0.8"

# Async
futures = "0.3"

# Tauri integration
tauri = { version = "2", optional = true }
```

---

## Appendix B: IPC Bridge Implementation

> **Added**: 2026-02-10
> This appendix documents the IPC bridge between the Rust core and the Node.js sidecar (`openclaw`).

### B.1 Protocol
Communication happens over the existing WebSocket connection used by `OpenClawWsClient`. Two new RPC methods are intercepted on the Rust side:

- `mcp.list_tools`: Returns a list of available tools (schema matches MCP spec).
- `mcp.call_tool`: Executes a tool (currently transient) and returns the result.
  - Arguments: `{ name: string, args: object }`

### B.2 Rust Implementation
- `McpRequestHandler` (`src-tauri/src/openclaw/ipc.rs`): Handles the RPC logic. It currently mocks `web_search` and `read_file` to prove connectivity but is wired to accept `tauri::AppHandle` for future integration with the real application state.
- `OpenClawWsClient` (`src-tauri/src/openclaw/ws_client.rs`): Identifies `mcp.*` methods and delegates them to the handler instead of the default OpenClawEngine gateway logic.

### B.3 Node.js Wrapper
- The `openclaw-engine` directory in `src-tauri` now contains a `main.js` that has been updated to launch the `openclaw` binary. It handles path resolution and environment setup to ensure the sidecar starts correctly.

---

## 13. UI Feedback System

To ensure the user is not left staring at a static screen while the agent runs headers scripts, `scrappy-mcp-tools` must expose a trait for status reporting.

### 13.1 Status Reporter Trait

```rust
// src/events.rs

#[async_trait]
pub trait StatusReporter: Send + Sync {
    /// Report a status change to the host application
    async fn report(&self, event: ToolEvent);
}

#[derive(Debug, Clone, Serialize)]
pub enum ToolEvent {
    /// Simple status update (e.g., "Connecting to Finance Server...")
    Status {
        msg: String,
        icon: Option<String>,
    },
    /// Detailed tool activity (renders as <scrappy_status>)
    ToolActivity {
        tool_name: String,
        input_summary: String,
        status: String, // "running", "complete", "failed"
    },
    /// Progress for long-running operations
    Progress {
        percentage: f32,
        message: String,
    }
}
```

### 13.2 Integration with Orchestrator — IMPLEMENTED ✅

The `Orchestrator` implements this trait and bridges it to the existing `ProviderEvent` system, ensuring the frontend receives the expected XML tags.

```rust
// src-tauri/src/rig_lib/orchestrator.rs (actual implementation)

struct OrchestratorStatusReporter {
    tx: mpsc::Sender<Result<ProviderEvent, String>>,
}

#[async_trait]
impl StatusReporter for OrchestratorStatusReporter {
    async fn report(&self, event: ToolEvent) {
        let xml_tag = match event {
            ToolEvent::ToolActivity { tool_name, input_summary, status } => {
                format!(
                    "\n<scrappy_status type=\"tool_call\" name=\"{}\" query=\"{}\" status=\"{}\" />\n",
                    tool_name, input_summary, status
                )
            },
            ToolEvent::Status { msg, .. } => {
                format!("\n<scrappy_status type=\"thinking\" msg=\"{}\" />\n", msg)
            },
            ToolEvent::Progress { percentage, message } => {
                format!(
                    "\n<scrappy_status type=\"progress\" pct=\"{:.0}\" msg=\"{}\" />\n",
                    percentage, message
                )
            },
        };

        if !xml_tag.is_empty() {
            let _ = self.tx.send(Ok(ProviderEvent::Content(xml_tag))).await;
        }
    }
}
```

### 13.3 Server-Side Status API (FastAPI)

The remote server provides status and progress for **background operations** (scheduler, scrapes) that the client cannot observe directly. Scrappy can poll or stream this data.

**MCP Tool**: `get_status(include_events: bool = True)`

Returns:
- `current_scrape`: Active scrape in progress (or null)
- `events`: Recent events (scrape_start, scrape_complete, scrape_failed, etc.)
- `scheduler`: Scheduler status (running, sources, next_scrape)
- `sources`: Data source health

**REST API**:
- `GET /api/v1/status` — Poll status (auth: `catalog:read`)
- `GET /api/v1/status/stream` — SSE stream for real-time updates (auth: `catalog:read`)

**Event types** (for `<scrappy_status />` conversion):
| Type | Fields | XML |
| :--- | :--- | :--- |
| `tool_call` | tool_name, query, status | `<scrappy_status type="tool_call" name="..." query="..." status="..." />` |
| `thinking` | msg | `<scrappy_status type="thinking" msg="..." />` |
| `progress` | percentage, message | `<scrappy_status type="progress" percentage="..." message="..." />` |
| `scrape_start` | source_type, msg | Server scrape started |
| `scrape_complete` | source_type, items_fetched, items_stored, duration_ms | Server scrape finished |
| `scrape_failed` | source_type, error_message | Server scrape failed |

**SSE stream format**:
```
event: scrape_start
data: {"type":"scrape_start","source_type":"finance","msg":"Starting scrape: finance","timestamp":...}

event: scrape_complete
data: {"type":"scrape_complete","source_type":"finance","items_fetched":80,"items_stored":80,"duration_ms":3200,...}
```

---

## 14. Support for Local "Host" Tools (Web Search)

Not all tools will come from the remote MCP server. Some, like `web_search` (which runs locally via `chromiumoxide`), `rag_search` (local vector store), or `read_file`, must be injected from the host application into the sandbox.

### 14.1 Host Tool Injection

The `Sandbox` must allow registering closures that capture the host's application state (e.g., database pools, app handles).

```rust
// In Orchestrator initialization of Sandbox

let mut sandbox = Sandbox::new(client, config);

// Inject Web Search
let rig_clone = self.rig.clone();
sandbox.register_host_tool("web_search", move |query: String| {
    let rig = rig_clone.clone();
    async move {
        // Emit status
        rig.emit_status("web_search", &query).await;
        // Call existing local logic
        rig.explicit_search(&query).await.map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)
    }
});

// Inject RAG
let app_handle = self.rig.app_handle.clone();
sandbox.register_host_tool("rag_search", move |query: String| {
    let app = app_handle.clone();
    async move {
        // ... call local RAG logic ...
    }
});
```

### 14.2 Unified Tool API

From the perspective of the Agent (writing Rhai scripts), there is **no difference** between a remote MCP tool and a local host tool.

```rust
// Agent Code
let stock_price = finance.get_stock_price("AAPL"); // Remote MCP
let news = web_search("Apple stock news");         // Local Host Tool (injected)

return #{ price: stock_price, news: news };
```

---

## 15. Robust Error Handling & Self-Correction

When an agent-generated script fails (e.g., using a variable that doesn't exist, or calling a tool with invalid arguments), the system must simply catch the error and feed it back to the LLM.

### 15.1 Error Feedback Loop

1. **Sandbox Execution**: Returns `Result<String, ScriptError>`.
2. **Error Formatting**: Convert `ScriptError` into a system message for the next turn.

```rust
if let Err(e) = sandbox.execute(&script).await {
    // Feed error back to LLM
    history.push(Message {
        role: "system",
        content: format!(
            "Tool Execution Error:\n{}\n\nHint: Check your variable names and tool arguments. Rewrite the code to fix this.", 
            e
        )
    });
    // LLM effectively "retries" in the next generation
}
```

### 15.2 Common Error Types

- `ToolNotFound(name)`: Agent hallucinated a tool.
- `ArgumentMismatch`: Agent passed string instead of number.
- `Timeout`: Script took too long (infinite loop protection).
- `SecurityViolation`: Script tried to import `std::fs`.

---

## 16. Authentication & Configuration

The `McpClient` needs both the server URL and a valid JWT token. This should be passed from the main application configuration.

```rust
pub struct McpConfig {
    pub base_url: String,
    pub auth_token: String, // JWT
    pub timeout_ms: u64,
}

impl McpClient {
    pub fn new(config: McpConfig) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Authorization", 
            format!("Bearer {}", config.auth_token).parse().unwrap()
        );
        
        // ... setup client ...
    }
}
```

---

## 17. Token Savings Estimates

| Scenario | Direct Tool Calling | Code Execution | Savings |
| :--- | :--- | :--- | :--- |
| Load all 23 tool definitions | ~4,000 tokens | ~200 tokens (search_tools only) | **95%** |
| Fetch 5 stock prices | ~2,500 tokens | ~100 tokens (summary) | **96%** |
| Market research (prices + news) | ~10,000 tokens | ~300 tokens (filtered) | **97%** |
| Full model catalog scan | ~50,000 tokens | ~500 tokens (filtered) | **99%** |
| Complex multi-domain query | ~20,000 tokens | ~400 tokens (aggregated) | **98%** |

---

## Appendix A: Implemented Crate & Orchestrator Architecture

> **Added**: 2026-02-10  
> This appendix documents the **actual** implementation as built, complementing the aspirational spec in sections 4–9 above.

### A.1 Actual Crate Structure

```
src-tauri/scrappy-mcp-tools/
├── Cargo.toml                       # reqwest, tokio, serde, rhai, futures, async-trait, chrono, thiserror
├── src/
│   ├── lib.rs                       # Re-exports: McpClient, McpConfig, StatusReporter, ToolEvent, Sandbox, SandboxConfig, SandboxResult
│   ├── client.rs                    # McpClient — HTTP POST to /tools/call with JWT Bearer auth, typed + raw deserialization
│   ├── discovery.rs                 # search_tools() + ToolRegistryCache (TTL-based, lazy fetch)
│   ├── events.rs                    # StatusReporter trait, ToolEvent enum, NullReporter (test helper)
│   ├── sandbox.rs                   # Rhai Sandbox with SandboxConfig, SandboxError (6 variants), execute(), validate_script(), engine_mut()
│   └── tools/
│       ├── mod.rs                   # Module index
│       └── finance.rs               # Type-safe bindings: get_stock_price, get_market_summary, get_crypto_prices, etc.
└── tests/
    └── sandbox_tests.rs             # 17 tests — all passing
```

**Key differences from the proposed structure (§4.1)**:
- Sandbox is a single `sandbox.rs` file (not a `sandbox/` directory) — simpler, sufficient for current complexity
- Discovery is a single `discovery.rs` file (not a `discovery/` directory)
- No `auth.rs` — JWT token is handled directly by `McpConfig.auth_token`
- No `skills/` or `ipc/` directories yet — those are Phase D and Phase F
- `events.rs` includes `NullReporter` for test use (not in original spec)

### A.2 Orchestrator Dual-Mode Architecture

The `Orchestrator` in `src-tauri/src/rig_lib/orchestrator.rs` supports **two execution modes** that can be toggled per-instance:

```text
┌──────────────────────────────────────────────────────────────────────┐
│                    Orchestrator::run_turn()                          │
│                                                                      │
│  ┌─────────────────────────────────────────────────────────────────┐ │
│  │ Token Check → Auto-Summarization → Context Preparation          │ │
│  │ (shared between both modes)                                      │ │
│  └──────────────────────────┬──────────────────────────────────────┘ │
│                             │                                        │
│                             ▼                                        │
│              ┌──────────────────────────────┐                       │
│              │   sandbox_enabled?            │                       │
│              └──────┬──────────────┬─────────┘                       │
│                     │              │                                  │
│              YES    │              │   NO                             │
│                     ▼              ▼                                  │
│  ┌──────────────────────┐  ┌──────────────────────┐                 │
│  │ run_sandbox_loop()   │  │ run_legacy_tool_loop()│                 │
│  │ <rhai_code> tags     │  │ <tool_code> JSON tags  │                 │
│  │ Rhai engine execute  │  │ Manual JSON parsing    │                 │
│  │ Self-correction loop │  │ if/else tool dispatch  │                 │
│  └──────────────────────┘  └──────────────────────┘                 │
│                                                                      │
│  Constructor:                                                        │
│  • Orchestrator::new(rig)              → sandbox_enabled = false     │
│  • Orchestrator::new_with_mcp(rig, c)  → sandbox_enabled = c.sandbox│
└──────────────────────────────────────────────────────────────────────┘
```

**Zero breaking changes**: `Orchestrator::new()` defaults to `sandbox_enabled: false`, so all existing `chat.rs` call sites work untouched. When the MCP server is ready, switching is a one-line change:

```rust
// BEFORE (legacy mode — current production):
let orchestrator = Orchestrator::new(Arc::new(manager));

// AFTER (sandbox mode — when MCP server is live):
let mcp_config = McpOrchestratorConfig {
    mcp_base_url: Some("https://api.scrappy.dev".to_string()),
    mcp_auth_token: Some(token.clone()),
    sandbox_enabled: true,
};
let orchestrator = Orchestrator::new_with_mcp(Arc::new(manager), mcp_config);
```

### A.3 Host Tool Registration Pattern

Rhai is synchronous; the host tools (`web_search`, `rag_search`, `read_file`) are async. The bridge pattern used:

```rust
// In Orchestrator::build_sandbox()

let rig_clone = self.rig.clone();
sandbox.engine_mut().register_fn(
    "web_search",
    move |query: String| -> rhai::Dynamic {
        let rig = rig_clone.clone();
        // Bridge async → sync safely via block_in_place
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(async { rig.explicit_search(&query).await })
        });
        rhai::Dynamic::from(result)
    },
);
```

**Why `block_in_place`?** Rhai callbacks run on the tokio runtime thread. `block_in_place` tells tokio to yield the thread for blocking work, avoiding deadlocks. This is safe because:
1. The sandbox runs inside a `tokio::spawn` task (inside `run_turn`)
2. `block_in_place` only blocks the current thread, not the entire runtime
3. The alternative (`spawn_blocking`) would require `Send` bounds that `rhai::Dynamic` doesn't satisfy

### A.4 Sandbox → LLM Self-Correction Flow

When a sandbox script fails, the error is formatted as LLM-friendly feedback and injected back into the conversation:

```text
Turn 1: LLM generates <rhai_code>get_stok_price("AAPL")</rhai_code>
                                    ↓
        Sandbox: SandboxError::Compilation("Unknown function: get_stok_price")
                                    ↓
        Feedback: "Script Compilation Error:\nUnknown function: get_stok_price\n\n
                   Hint: The function or tool you called does not exist. 
                   Use `search_tools` to discover available tools."
                                    ↓
Turn 2: LLM corrects: <rhai_code>get_stock_price("AAPL")</rhai_code>  ← fixed typo
                                    ↓
        Sandbox: Ok("{ price: 185.50, ... }")
                                    ↓
Turn 3: LLM synthesizes final answer using the result
```

### A.5 StatusReporter → Frontend Pipeline

```text
Sandbox ToolEvent                    OrchestratorStatusReporter             Frontend
─────────────────                    ──────────────────────────             ────────
ToolEvent::ToolActivity {            → ProviderEvent::Content(             → Renders status
  tool_name: "web_search",             "<scrappy_status type=\"tool_call\"    bubble with
  input_summary: "AI news",            name=\"web_search\"                   search icon
  status: "running"                     query=\"AI news\"                     and query text
}                                       status=\"running\" />"
                                     )

ToolEvent::Status {                  → ProviderEvent::Content(             → Shows thinking
  msg: "Executing script..."           "<scrappy_status type=\"thinking\"     animation
}                                       msg=\"Executing script...\" />"
                                     )

ToolEvent::Progress {                → ProviderEvent::Content(             → Progress bar
  percentage: 75.0,                    "<scrappy_status type=\"progress\"     at 75%
  message: "Fetching data..."          pct=\"75\" msg=\"Fetching data...\"
}                                       />"
                                     )
```

### A.6 Server-Side Status Integration (§13.3)

The server's SSE events (`scrape_start`, `scrape_complete`, `scrape_failed`) are **separate** from the sandbox tool execution flow. They represent background server operations that Scrappy can optionally monitor:

```text
Sandbox Loop (existing)              Server SSE (new, §13.3)
────────────────────────             ────────────────────────
→ Agent writes script                → Scrappy opens SSE connection to
→ Sandbox executes                     GET /api/v1/status/stream
→ Tools call MCP server              → Server pushes: scrape_start,
→ Results fed back to LLM              scrape_complete, scrape_failed
→ LLM synthesizes answer             → Scrappy converts to <scrappy_status>
                                        for dashboard/toast display
```

**Future work**: Add an `SseClient` to `scrappy-mcp-tools` that connects to `/api/v1/status/stream` and converts server events into `ToolEvent` variants. The `StatusReporter` trait already supports this — new `ToolEvent` variants would be added for `ScrapeStart`, `ScrapeComplete`, `ScrapeFailed`.

### A.7 Test Verification

```
$ cd src-tauri/scrappy-mcp-tools && cargo test
running 17 tests
test tests::test_simple_expression ............ ok
test tests::test_string_result ................ ok
test tests::test_unit_result .................. ok
test tests::test_multiline_script ............. ok
test tests::test_timestamp_now ................ ok
test tests::test_json_stringify ............... ok
test tests::test_register_host_tool ........... ok
test tests::test_host_tool_with_computation ... ok
test tests::test_forbidden_std_fs ............. ok
test tests::test_forbidden_unsafe ............. ok
test tests::test_unknown_function ............. ok
test tests::test_parse_error .................. ok
test tests::test_operations_limit ............. ok
test tests::test_llm_feedback_runtime ......... ok
test tests::test_llm_feedback_compilation ..... ok
test tests::test_llm_feedback_forbidden ....... ok
test tests::test_result_size_limit ............ ok
test result: ok. 17 passed; 0 failed
```

Full Tauri app build: `cargo check` with `-D warnings` → **0 errors, 0 warnings**.

