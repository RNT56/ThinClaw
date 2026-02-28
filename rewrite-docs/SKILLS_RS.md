# The OpenClaw Skill System (Managed Executables)

You've made a crucial distinction: "Plugins" (like MCP and WASM) are entirely separate from **OpenClaw's "Skill" system.**

In OpenClaw, a "Skill" is **not** a piece of code that loads inside the AI engine.

A Skill in OpenClaw is simply **a standalone bash tool or system binary that the agent has permission to run.**

---

## 1. What is an OpenClaw Skill?

If you look at the `src/agents/skills-install.ts` and `src/plugin-sdk/run-command.ts` files, you'll see exactly how the "Skill" system operates under the hood.

When the agent wants to "learn a new skill" (e.g., using `uv`, a Python package manager, or installing a `brew` formula):

1. **The Request:** The LLM outputs a tool call to install a skill: `{"tool": "install_skill", "name": "uv"}`
2. **The Rust Orchestrator:** The backend receives this and runs a standard `bash` script to download and install the binary (e.g., `brew install uv` or `pnpm install ...`).
3. **The `skills/` Directory:** The resulting binary is often placed in a dedicated config folder, like `~/.thinclaw/config/skills/`.

**A "Skill" is a pre-packaged terminal command.** It does not use the `@plugin-sdk` directly; it uses the standard OS execution layer.

## 2. Why OpenClaw calls it a "Skill" (Security & Discoverability)

If it’s just a bash command, why did OpenClaw build a massive "Skill System" instead of just letting the agent run `bash`?

- **Sandboxing:** OpenClaw runs in a Docker container. In a Docker container, you don't have tools like `wget`, `curl`, Python, or Node.js by default. The "Skill System" was built to allow the agent to explicitly request permission to install those tools into its sterile Docker container.
- **Discoverability:** The Orchestrator reads a list of installed binary paths and injects them into the prompt: _"You have the following skills installed: `uv`, `rg`, `fd`."_

## 3. Rebuilding the Skill System in Rust

Because ThinClaw runs natively on macOS (not inside a blank Docker container), your user already has access to tools like `brew`, `python`, and `node`.

However, to maintain the exact autonomy of OpenClaw, the agent still needs the ability to write its own scripts and declare them as tools.

We don't need WASM or MCP for this; we just need our standard **Host Execution / Sandbox** functionality (detailed in `SANDBOX_RS.md`).

**The "Create a Skill" Loop:**

1. **The Goal:** The User asks the agent: _"Create a skill that scrapes the latest news from TechCrunch and summarizes it."_
2. **The Write Phase:** The LLM writes a Python script (`scrape.py`) to the hard drive using the `write_file` tool.
3. **The Registration:** The LLM outputs a tool call: `{"tool": "register_skill", "name": "techcrunch_news", "command": "python3 /path/to/scrape.py"}`.
4. **The Rust Orchestrator:** Saves that metadata to a file (`skills.toml`).
5. **The Next Chat:** The Orchestrator automatically loads `skills.toml` and injects a dynamic tool into the RIG Agent called `techcrunch_news`. When the LLM decides to call that tool, the Rust Orchestrator simply uses `std::process::Command` to execute the Python script natively.

## Summary

- **MCP & WASM Plugins:** Used for deep integrations written by third-party developers that require JSON-RPC communication (e.g., connecting a live Jira workspace).
- **The "Skill" System:** Used by the _Agent itself_ to write simple bash/python scripts, save them to the hard drive, and tell the Orchestrator: _"Hey, add this script as a button to my toolbelt so I can run it later."_

Because the Rust Orchestrator controls the toolbelt, handling skills is just a matter of dynamically mapping LLM JSON outputs to `std::process::Command` execution.

---

## 4. Security Requirements for Skill Registration (G5)

The `register_skill` action is powerful and must not be silently auto-approved. A compromised LLM could attempt to register a malicious script as a permanent tool.

### Rule 1: Skill Registration Always Requires User Approval

Even in `Autonomous Mode` (where normal tool calls are auto-approved), registering a *new* skill must always pop a Tauri UI approval dialog:

```
┌────────────────────────────────────────────────────────┐
│ 🧩  Agent Wants to Register a New Skill                │
│                                                        │
│  Name:    techcrunch_news                              │
│  Command: python3 ~/.thinclaw/skills/scrape.py         │
│                                                        │
│  This will add a new permanent tool to the agent.      │
│  Review the script before approving:                   │
│  [View Script]  [Deny]  [Approve & Register]           │
└────────────────────────────────────────────────────────┘
```

The "View Script" button opens the script contents for the user to review before granting permission.

### Rule 2: Registered Skills Run in the Deno Sandbox

When a registered skill is subsequently *called* by the LLM, it does not run as raw `std::process::Command`. It runs through the **Deno sandbox** (see `SANDBOX_RS.md`), inheriting the same `--deny-net` and `--allow-read=/tmp/sandbox_dir` restrictions:

```rust
// Correct: skill runs in the Deno sandbox
let output = std::process::Command::new("deno")
    .arg("run")
    .arg("--allow-read=/tmp/sandbox_dir")
    .arg("--deny-net")
    .arg(skill.command_path)
    .output()?;
```

This prevents skills from exfiltrating data even if the LLM later decides to invoke one for malicious purposes.

### Rule 3: Skill Auditability Commands

The user must be able to review and remove registered skills from any channel using slash commands:

- **`/skills list`** — displays all registered skills and their commands.
- **`/skills remove [name]`** — permanently removes a skill from `skills.toml` after a confirmation prompt.

These commands bypass the LLM entirely (they are handled by the Command Router in `CHAT_COMMANDS_RS.md`).
