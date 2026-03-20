# 🛠️ Scrappy Agent Tools: Implementation & Registry

Scrappy utilizes a **dual-engine architecture** for its AI agents. Depending on the task, tools can be implemented either as high-performance **Native Rust Tools (Rig)** or as flexible **OpenClaw Skills** (via IronClaw).

---

## 🏗️ 1. Native Rust Agent (Rig Engine)

The Native agent is built on `rig-core`. It is designed for high-performance, complex orchestration tasks like deep web research and RAG.

### 📋 Available Tools

| Tool Name | Class | Description | Implementation |
| :--- | :--- | :--- | :--- |
| `web_search` | `DDGSearchTool` | Multi-stage search: 1. DuckDuckGo lookup, 2. Trusted source ranking, 3. Parallel deep scraping via Chromium, 4. LLM-based summarization of results. Features **Search Grounding** to automatically trigger for factual queries. | `web_search.rs` |
| `scrape_page` | `ScrapePageTool` | Directly visits a URL using a bundled Chromium sidecar to extract clean markdown content. | `scrape_page.rs` |
| `rag_tool` | `RAGTool` | Hooks into the local USearch vector store to retrieve relevant document snippets based on embeddings. | `rag_tool.rs` |
| `generate_image` | `ImageGenTool`| Interface for the Stable Diffusion (`sd.cpp`) sidecar. Supports multiple residents, resolved VAE/CLIP/T5 components, and real-time progress callbacks. | `image_gen_tool.rs` |
| `calculator` | `CalculatorTool` | Pure-Rust math expression evaluator. Supports arithmetic (`+`, `-`, `*`, `/`, `^`, `%`), parentheses, functions (`sqrt`, `abs`, `round`, `ceil`, `floor`, `log`, `ln`, `sin`, `cos`, `tan`, `min`, `max`), and constants (`pi`, `e`). Helps smaller models with precise calculations, currency conversions, percentages, and unit conversions. | `calculator_tool.rs` |

### 🔍 The Integrated "Scraping" Workflow (Deep Search)

In Scrappy, **Websearch and Scraping are inseparable**. When the `web_search` tool is triggered, it follows a multi-stage **Deep Search** pipeline to ensure the LLM receives high-quality, up-to-date context rather than just metadata snippets:

1.  **DuckDuckGo Search:** Performs an initial search via DuckDuckGo HTML to find relevant candidate URLs.
2.  **Ranking:** Results are filtered and ranked. "Trusted sources" (e.g., official documentation, Wikipedia, GitHub) are prioritized and moved to the top.
3.  **Automatic Scraping (The Core Trigger):** The tool automatically attempts to scrape the full content of the top results (configurable via `scrape_concurrency_limit`, usually the top 2-5 results):
    *   **Fast Path:** It first attempts a simple HTTP GET to extract text quickly.
    *   **Full Path (Chromium):** If the page requires JavaScript (detected via heuristics like "Enable JS" messages or low text density), it spawns a **Headless Chromium Browser** sidecar to render the page and extract the final text.
4.  **AI Analysis (Map-Reduce):** The scraped text is sent to a **Summarizer LLM** (a specialized local model). This model scores the content for relevancy to the user's query and compresses large pages into concise, fact-dense summaries.
5.  **Context Injection:** The summarized, scraped content is fed back to the main chat LLM as a `<tool_result>` block, allowing it to answer with "grounded" real-time information.
6.  **Search Grounding:** Intelligent logic detects factual queries (e.g., "What is the price of Bitcoin?") and automatically activates the search tool even if not explicitly requested by the user, ensuring up-to-date accuracy.

This pipeline ensures that Scrappy doesn't just "talk about" the web, but actually "reads" it before responding.


### 🛠️ How to Create & Register a Native Tool

#### 1. Implement the `Tool` Trait
Create a new file in `backend/src/rig_lib/tools/your_tool.rs`:

```rust
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Deserialize)]
pub struct MyArgs { pub input: String }

pub struct MyTool;

impl Tool for MyTool {
    const NAME: &'static str = "my_awesome_tool";
    type Error = MyError;
    type Args = MyArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> rig::completion::ToolDefinition {
        rig::completion::ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Describe what the tool does for the LLM".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // Implementation logic
        Ok(format!("Processed: {}", args.input))
    }
}
```

#### 2. Register in the Agent
Modify `backend/src/rig_lib/agent.rs` inside `RigManager::new`:

```rust
let agent = builder
    .tool(MyTool) // Add your tool here
    .tool(RAGTool { ... })
    .build();
```

---

## 📡 2. OpenClaw Agent (IronClaw Engine)

The OpenClaw agent is powered by **IronClaw**, an in-process Rust library linked directly into the Tauri binary — no Node.js sidecar or WebSocket bridge.

### 📋 Built-in Capabilities

*   **💻 Shell Execution (`exec`)**: Runs commands in the local shell with human-in-the-loop (HITL) safety approvals.
*   **📂 File I/O**: Operations for reading, writing, and managing files in the workspace.
*   **🌐 Browser Control**: Chromium automation for web tasks.
*   **🎨 Skill Extensions**: Custom scripts that add specific capabilities (e.g., "Search News", weather, API integrations).
*   **🔒 Safety Layer**: Configurable tool policies (deny/allow lists) and approval gates for sensitive operations.

### 🛠️ How to Create & Register a Skill

Tools in OpenClaw are called **Skills**. They are located in the agent's workspace directory.

#### 1. Create a Skill Definition (`skill.json`)
Each skill requires a JSON definition:

```json
{
  "name": "get_weather",
  "description": "Get current weather for a city",
  "parameters": {
    "type": "object",
    "properties": {
      "city": { "type": "string" }
    },
    "required": ["city"]
  }
}
```

#### 2. Implement the Logic (`skill.js`)

```javascript
export async function execute({ city }) {
  const resp = await fetch(`https://api.weather.com/...&q=${city}`);
  const data = await resp.json();
  return `The weather in ${city} is ${data.temp}°C.`;
}
```

#### 3. Deployment
*   Skills are automatically discovered if placed in the `OpenClaw/skills/` directory within your App Data folder.
*   You can toggle them on/off in the **Settings > OpenClaw** tab in the Scrappy UI.

---

## 🧬 Summary: Which Engine to Use?

*   Use **Native Rust (Rig)** if your tool requires intensive CPU/Memory (like Image Gen), direct access to the Vector DB, or if you want to leverage Rust's safety for complex data processing.
*   Use **OpenClaw (IronClaw)** for autonomous tasks, multi-step tool chains with human-in-the-loop approval, persistent agent memory, and interactive skill extensions.
