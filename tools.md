# 🛠️ Scrappy Agent Tools: Implementation & Registry

Scrappy utilizes a **dual-engine architecture** for its AI agents. Depending on the task, tools can be implemented either as high-performance **Native Rust Tools (Rig)** or as flexible **OpenClaw Node.js Skills**.

---

## 🏗️ 1. Native Rust Agent (Rig Engine)

The Native agent is built on `rig-core`. It is designed for high-performance, complex orchestration tasks like deep web research and RAG.

### 📋 Available Tools

| Tool Name | Class | Description | Implementation |
| :--- | :--- | :--- | :--- |
| `web_search` | `DDGSearchTool` | Multi-stage search: 1. DuckDuckGo lookup, 2. Trusted source ranking, 3. Parallel deep scraping via Chromium, 4. LLM-based summarization of results. | `web_search.rs` |
| `scrape_page` | `ScrapePageTool` | Directly visits a URL using a bundled Chromium sidecar to extract clean markdown content. | `scrape_page.rs` |
| `rag_tool` | `RAGTool` | Hooks into the local USearch vector store to retrieve relevant document snippets based on embeddings. | `rag_tool.rs` |
| `generate_image` | `ImageGenTool`| Interface for the Stable Diffusion (`sd`) sidecar to generate visual assets from textual prompts. | `image_gen_tool.rs` |

### 🛠️ How to Create & Register a Native Tool

#### 1. Implement the `Tool` Trait
Create a new file in `src-tauri/src/rig_lib/tools/your_tool.rs`:

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
Modify `src-tauri/src/rig_lib/agent.rs` inside `RigManager::new`:

```rust
let agent = builder
    .tool(MyTool) // Add your tool here
    .tool(RAGTool { ... })
    .build();
```

---

## 📡 2. OpenClaw Node.js Agent (OpenClaw Engine)

The OpenClaw engine (formerly Moltbot) is a Node.js-based runtime designed for autonomous tasks and interactive tool usage (HITL).

### 📋 Built-in Capabilities

*   **💻 Shell Execution (`exec`)**: Runs commands in the local shell with human-in-the-loop (HITL) safety approvals.
*   **📂 File I/O**: Operations for reading, writing, and managing files in the workspace.
*   **🌐 Browser Control**: Full Puppeteer/Chromium automation for web tasks.
*   **🎨 Skill Extensions**: Custom JS scripts that add specific capabilities like "Search News" or "Control Spotify".

### 🛠️ How to Create & Register an OpenClaw Skill

Tools in OpenClaw are called **Skills**. They are located in the `moltbot` workspace directory.

#### 1. Create a Skill Definition (`skill.json`)
Each skill requires a JSON definition that follows the **Agent Control Protocol (ACP)** specification.

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
The implementation is safe-eval'd inside the Node host.

```javascript
export async function execute({ city }) {
  const resp = await fetch(`https://api.weather.com/...&q=${city}`);
  const data = await resp.json();
  return `The weather in ${city} is ${data.temp}°C.`;
}
```

#### 3. Deployment
*   Skills are automatically discovered if placed in the `Clawdbot/skills/` directory within your App Data folder.
*   You can toggle them on/off in the **Settings > OpenClaw** tab in the Scrappy UI.

---

## 🧬 Summary: Which Engine to Use?

*   Use **Native Rust (Rig)** if your tool requires intensive CPU/Memory (like Image Gen), direct access to the Vector DB, or if you want to leverage Rust's safety for complex data processing.
*   Use **OpenClaw (Node.js)** if you want to quickly build autonomous scripts, need easy access to the NPM ecosystem (e.g., specialized APIs), or want your agent to perform "Thinking Loops" with tool reflections.
