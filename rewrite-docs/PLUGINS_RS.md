> ⛔ **ARCHIVED** — This is a historical migration guide from the OpenClaw→IronClaw rewrite (early 2026). It does NOT reflect the current codebase. See [`docs/EXTENSION_SYSTEM.md`](../docs/EXTENSION_SYSTEM.md) for the current extension architecture, and [`docs/CHANNEL_ARCHITECTURE.md`](../docs/CHANNEL_ARCHITECTURE.md) for the hybrid channel design.

---

# Extensibility: Plugins, Skills, & MCP

One massive difference between a Node.js app (OpenClaw) and a Rust app (ThinClaw) is how they handle **Extensibility**.

If a user wants to add a new tool to OpenClaw (like a tool that searches their company's Jira board), they can just write a quick JavaScript file (`my-jira-skill.js`) and drop it in a folder. Because Node.js is interpreted, OpenClaw can simply `require()` that file at runtime.

**Rust is a compiled language.** You cannot just drop a `.rs` file into a folder and expect the `ThinClaw.app` binary to magically load it without recompiling the entire application.

So, how do we make the Rust agent extensible? We have two modern, incredibly powerful architectural options.

---

## 1. The Standard: Model Context Protocol (MCP)

This is the industry standard created by Anthropic, and it is natively supported by OpenClaw (via the `@agentclientprotocol/sdk` or MCP).

Instead of writing a "plugin" that loads _inside_ your application, developers write standalone **MCP Servers**.

**How it works:**

1. A developer writes a tiny Python or Node script that connects to their Jira board.
2. This script implements the **MCP Server** specification, exposing a `search_jira` tool.
3. The user configures ThinClaw to connect to this MCP Server:
   ```toml
   [mcp_servers]
   jira_helper = { command = "node", args = ["/path/to/jira-mcp.js"] }
   ```
4. When ThinClaw boots, the Rust Orchestrator spawns that node process in the background. It communicates with it over `stdio` (Standard Input/Output) using JSON-RPC.
5. The Rust Orchestrator reads the available tools from the MCP server, and automatically adds the `search_jira` tool to your RIG Agent's toolkit.

**Why this is the best approach:**
It is language-agnostic. Your users can write extensions for your Rust agent using Python, Go, Node, or anything else. You simply build an MCP Client into your Rust Orchestrator. The official Rust MCP SDK crate is **`rmcp`** (published by the Anthropic/MCP community). Integrate it with:
```toml
[dependencies]
rmcp = { version = "0.1", features = ["client", "transport-child-process"] }
```

---

## 2. The Native Rust Sandbox: WebAssembly (WASM) Plugins

If you want a tightly integrated plugin system where users _drop files into a folder_ just like OpenClaw, the state-of-the-art approach in Rust is **WebAssembly (WASM) Plugins**.

Using a crate like **`extism`** or building on your existing sandbox crate **`wasmtime`**, you can create a true plugin system in a compiled binary.

**How it works:**

1. You define a Plugin Trait in your Rust application (e.g., `execute_tool(input: String) -> String`).
2. A developer writes a plugin in Rust, Go, or TypeScript, and compiles it to a single `.wasm` file (e.g., `github_tool.wasm`).
3. They drop `github_tool.wasm` into `~/.thinclaw/plugins/`.
4. Your Rust app detects the `.wasm` file, dynamically loads it into memory instantly using `wasmtime`, and wires it up to the RIG agent's toolkit.

**Why this is amazing:**
WASM plugins are phenomenally safe. They are perfectly sandboxed. You can load a downloaded community plugin and explicitly deny it network access or filesystem access, and it runs at near-native C speeds.

---

## 3. Webhooks & REST Endpoints (The Simple Way)

If your users just want standard integrations (e.g., receiving a Telegram message when a long agent task finishes, or triggering the agent from a Shortcut on their iPhone), you don't need a plugin system.

Because we established in `REMOTE_AND_PII_RS.md` that you can wrap your Rust core in an `axum` web server, you can literally just expose a `/webhook` API route.

Users can use Zapier, Make, or Apple Shortcuts to `POST` to `http://localhost:port/webhook` to dynamically feed the agent new tasks or data.

## Summary Recommendation for ThinClaw

If you want ThinClaw to have the exact same massive ecosystem of integrations that OpenClaw currently has, **build an MCP (Model Context Protocol) Client** into your Rust Orchestrator.

Because Anthropic and the open-source community are building thousands of MCP servers (for GitHub, Slack, Jira, Postgres, etc.), if your Rust agent speaks MCP, your users instantly get access to every single one of those tools without you having to write a single line of Rust code for them.
