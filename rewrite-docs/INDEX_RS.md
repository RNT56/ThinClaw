# ThinClaw Rewrite Documentation Index

This directory contains the complete architectural blueprint for rewriting the OpenClaw Node.js agent in Rust using the `rig-core` library.

## Document Index

1. **`ARCHITECTURE.md`:** Explains why the RIG Agent belongs inside the Tauri frontend application instead of running as a separate isolated web server.
2. **`AGENT_RS.md`:** The technical guide on how to build the advanced resilient agent loop (fallback routing, token counting, history compaction, rate limiting) in Rust.
3. **`CONFIG_RS.md`:** How to map the massive TS configuration state to strongly typed `serde` and `TOML` structs in Rust.
4. **`SECRETS_RS.md`:** The critical security architecture for saving API keys via Tauri commands into the encrypted macOS Keychain (`SecretStore`) instead of plaintext config files. Explains why OpenClaw's Node.js environment variables were "insecure by default."
5. **`SANDBOX_RS.md`:** Strategies for executing untrusted agent code natively in Rust, including Docker (`bollard`), WASM (`wasmtime`), and Host Execution via Tauri UI approvals with network restriction (`deno`).
6. **`KNOWLEDGE_BASE_RS.md`:** How to build a Secure RAG system embedded directly in the Tauri binary using `rusqlite`, `sqlite-vec`, `sqlcipher`, and local embedding models.
7. **`LEAST_PRIVILEGE_RS.md`:** Conceptually separating the Rust Orchestrator from the airgapped Generative LLM to physically prevent the LLM from taking autonomous unauthorized action.
8. **`VECTOR_SEARCH_RS.md`:** The mathematical mechanics behind RAG, showing how the Orchestrator finds answers _without_ invoking the Generative LLM.
9. **`AUTONOMY_RS.md`:** How the Orchestrator safely provides the LLM with tools (Browser, Email, Code Execution) via a continuous execution loop and "Auto-Approve" trust scopes.
10. **`REMOTE_AND_PII_RS.md`:** The plan for compiling the Rust core as both a local Tauri app and a headless cloud Linux server. Details the PII (Personally Identifiable Information) scrubbing pipeline for securing cloud LLM requests.
11. **`REWRITE_TRACKER.md`:** The master checklist of all the 14 agent tools, 6 prioritized chat channels, and 8 device capabilities moving from Node.js to Rust.
12. **`INTERNAL_SYSTEMS_RS.md`:** Explains OpenClaw's "Personality" and "Proactivity" mechanisms—including `SOUL.md`, `BOOTSTRAP.md`, and Cron-driven Heartbeat prompts—and how to recreate them natively in Rust.
13. **`PLUGINS_RS.md`:** Architectural plan for Extensibility in compiled Rust apps. Replaces Node.js script loading with the industry-standard Model Context Protocol (MCP) and WebAssembly (WASM) plugins.
14. **`SKILLS_RS.md`:** Clarifies the difference between Plugins and Skills. Shows how to allow the agent to write its own bash/python scripts and register them as dynamic Tools via `std::process::Command`.
15. **`CHAT_COMMANDS_RS.md`:** Details the implementation of a Command Router to intercept `/slash` commands (like `/reset` and `/status`) in external headless channels like Discord and Telegram.
16. **`TRIGGER_MECHANICS_RS.md`:** Explains how the Rust Orchestrator handles group chats, thread bindings, and eavesdropping buffers to prevent infinite replies in noisy channels.
17. **`MULTIMODAL_RS.md`:** Explains how the Agent accepts images (via VLM/Cloud API) and audio files (via local Whisper) from messaging channels before processing them into text for the core LLM loop.
18. **`TAURI_RELAY_RS.md`:** Explains how the local Tauri desktop app stays alive in the macOS/Windows System Tray to act as an "Always-On" background relay powering remote chatbots.
19. **`CLIENT_SERVER_MODE_RS.md`:** Architectural blueprint for running the Tauri app purely as a Thin Client (Dumb UI). Explains how treating the remote machine as the Host eliminates bidirectional hardware bridging and massively improves security.
20. **`MODEL_DISCOVERY_RS.md`:** Explains how the Rust Orchestrator dynamically searches the Hugging Face Hub (for Local Inference like MLX/GGUF) and fetches active model lists from Cloud Providers (OpenAI/Anthropic/OpenRouter).
21. **`INFERENCE_PLACEMENT_RS.md`:** The 2×2 matrix for Orchestrator vs. Inference Engine placement (Local/Remote). Shows how local engines like MLX expose an OpenAI-compatible HTTP API so a remote Orchestrator can call them over a private VPN (Tailscale), giving users full flexibility over where computation lives.
22. **`NETWORKING_RS.md`:** Defines the complete Tailscale-first network topology for Remote Mode. Covers the three discovery methods (Tailscale MagicDNS, QR-Code pairing, manual), the WebSocket protocol envelope, API key transmission from Tauri UI to remote Orchestrator, and the auto-update/version-sync strategy.
23. **`HARDWARE_BRIDGE_RS.md`:** The implementation plan for the opt-in hardware bridge, allowing a Remote Orchestrator to request camera frames, audio clips, and screenshots from the user's local Tauri Companion App via a secure RPC protocol with mandatory user approval dialogs.
24. **`BROWSER_TOOL_RS.md`:** The complete browser automation architecture — migrating from Playwright to `chromiumoxide` (CDP), accessibility tree snapshot extraction with ref IDs, navigation guards, Chrome profile persistence, and RIG tool integration for agent-driven web browsing.
25. **`CRON_RS.md`:** The full scheduled tasks system — cron expressions, interval timers, one-shot reminders, session isolation per job, delivery targets (send results to specific channels), catchup logic, heartbeat integration, and agent-created reminder tools.
26. **`HOOKS_RS.md`:** The lifecycle event bus and external trigger system — internal hooks (agent bootstrap, message sent/received), Gmail Pub/Sub integration, generic webhook endpoints, and the relationship between hooks (reactive) and cron (proactive).
27. **`SUBAGENT_RS.md`:** Multi-agent orchestration — spawning child agents with different models/tools, run vs session modes, depth limits, result announcement back to parent, thread binding, registry persistence, and timeout/orphan cleanup.
28. **`CANVAS_RS.md`:** The Canvas and a2UI (Agent-to-UI) system — agent-generated interactive web UIs served via a local HTTP server, the a2UI bridge script for bidirectional communication, live reload via WebSocket, Tauri WebView integration, user action flow, and security boundaries.
29. **`CLI_RS.md`:** The full command-line interface specification — 25+ top-level commands with nested subcommands (config, models, agents, sessions, memory, cron, channels, hooks, plugins, security, daemon, etc.), `clap` derive implementation, shared command pattern for CLI+Tauri IPC, global flags, shell completions, and Orchestrator communication.
30. **`TUI_RS.md`:** The interactive terminal chat interface — `ratatui` layout (header, chat area, input, footer), streaming token assembly and display, slash commands, overlay selector system (model/agent/session pickers), tool call rendering, local shell execution (`!` prefix), input history, and Ctrl+C handling.
31. **`SETUP_WIZARD_RS.md`:** The first-run onboarding wizard — 8-step flow (security ack, mode selection, provider/key setup, model selection, identity, channels, networking, review+launch), Tauri UI screens with ASCII mockups, terminal wizard variant using `inquire`, shared finalization logic, re-onboarding support, and dangerous command denylist.
