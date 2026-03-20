OpenClaw’s architecture separates the long‑running gateway from the agent runtime, model inference layer and the tools/skills ecosystem.  The following overview explains where each component lives and how they communicate.

1. Gateway: the central hub
	•	Location and role – Each host runs exactly one gateway daemon.  It owns all messaging surfaces (WhatsApp via Baileys, Telegram via grammY, Slack, Discord, Signal, iMessage, WebChat, etc.), maintains provider connections and is the only process that opens a WhatsApp session ￼.  The gateway exposes a typed WebSocket API; clients and nodes connect to it on 127.0.0.1:18789 (default) or via a VPN/Tailscale tunnel ￼ ￼.
	•	Clients vs nodes – Control‑plane clients (macOS companion app, CLI, web UI) open a WebSocket connection, send requests (health, status, agent, etc.) and subscribe to events (tick, agent, presence) ￼.  Nodes (Mac/iOS/Android/headless) connect with role: node, present a device identity for pairing and declare their capabilities (canvas, camera, screen recording, system commands) ￼.  Nodes expose commands like canvas.*, camera.* and system.run which the agent can call through the gateway.
	•	Handshake and pairing – On connection the client sends a connect frame; the gateway replies with hello-ok and begins streaming presence/tick events.  New devices require pairing approval; local connections (loopback) can auto‑approve, while remote connections require signing a challenge ￼.
	•	Events and RPC – The gateway validates all frames against JSON schemas and emits events such as agent, chat, presence, health, heartbeat and cron ￼.  RPC requests (agent, send, etc.) and server events are JSON messages over WebSocket ￼.

2. Agent runtime: context, workspace and loop
	•	Workspace and bootstrap – Each agent has a workspace directory.  When a run starts, OpenClaw resolves and (if needed) creates the workspace, loads bootstrap files (AGENTS.md, SOUL.md, TOOLS.md, IDENTITY.md, USER.md, etc.) and injects them into the system prompt so the model has persistent context ￼.
	•	System prompt – For every run, the gateway builds a custom system prompt containing:
– A tool list and short descriptions;
– Safety reminders;
– A skills section listing available skills with file paths; the prompt instructs the model to use the read tool to load a skill’s SKILL.md ￼ ￼;
– Self‑update hints (config.apply/update.run);
– Workspace path and documentation directory;
– Current date/time;
– Sandbox settings (if enabled) ￼.
The prompt includes local docs and points the model to the docs directory for help ￼.
	•	Agent loop – When a client calls the agent RPC, the gateway immediately returns {runId, acceptedAt} and then executes agentCommand.  This resolves the model and thinking/verbose defaults, loads the skills snapshot, and calls runEmbeddedPiAgent ￼.  runEmbeddedPiAgent serializes runs per session (to avoid concurrent tool calls), builds a pi‑agent session with the selected model and auth profile, subscribes to event streams, enforces timeouts and returns payloads plus usage metadata ￼.  Events are streamed back as assistant deltas, tool events and lifecycle events ￼ ￼.

3. Model inference: local and remote models
	•	Model selection – OpenClaw supports multiple providers (Anthropic, OpenAI, MiniMax, etc.) and local services such as Ollama or LM Studio.  You configure a primary model and optional fallbacks in openclaw.json.  During agentCommand, the gateway resolves the model and auth profile; if the primary model fails or is unavailable, it falls back to the next model in the list (this requires models.mode: "merge" when mixing local and hosted models).
	•	Local inference – When using a local service (e.g., LM Studio), the gateway sends inference requests to the local /v1 endpoint.  Local models are useful for reducing cost and latency but have smaller context windows; the docs warn that heavily quantized models increase prompt‑injection risk ￼, so you should run them in a sandbox and keep sensitive tools disabled.
	•	Remote inference – For cloud providers the gateway uses the respective API (OpenAI, Anthropic, etc.).  Authentication happens during onboarding.  The gateway streams assistant deltas from the provider to the client through agent events ￼.  The provider choice and fallback chain can be changed via CLI or the Control UI.

4. Tools: built‑in functions available to the agent
	•	Built‑in tools – The agent can call built‑in tools to perform actions.  Some key tools include:
– read: read files or skill descriptions;
– apply_patch: edit files using unified diff;
– exec: run shell commands (optionally sandboxed or with approval);
– browser/web_search/web_fetch: browse the web, click links and extract content;
– canvas: draw or manipulate images;
– message: send messages to chat channels;
– cron: schedule jobs;
– sessions and agents tools for session management.
The gateway enforces tool policy (allowlist/denylist per agent, channel or provider) and rejects calls to unauthorized tools ￼.
	•	Tool invocation – During an agent run, the model returns JSON with a tool name and arguments.  pi-agent-core streams this as a tool event; the gateway validates it against the tool schema and executes it on the host or node.  Tool results are streamed back to the model as part of the agent loop ￼.

5. Skills: code and instructions that extend capabilities
	•	Skill format and location – A skill is a directory containing a SKILL.md file with YAML front‑matter (name, description, optional metadata) and instructions for the model.  Skills live in three places: (1) bundled with OpenClaw; (2) managed locally at ~/.openclaw/skills; (3) inside each agent’s workspace <workspace>/skills ￼.  Precedence is workspace → local → bundled ￼.
	•	Loading and gating – At the start of a session, OpenClaw loads skills from these directories, applies gating rules (only eligible OS, required binaries or environment variables) and constructs the skills list.  Per‑agent configuration can disable skills or provide API keys/env values ￼.  The system prompt includes a compact XML list of available skills with names, descriptions and file paths ￼.  The model then calls read to load the skill’s SKILL.md for detailed instructions.
	•	Skills vs tools – Skills can provide their own tool definitions (written in Node.js or other languages) or act as high‑level instructions that call built‑in tools.  The agent can also write its own skills by editing files under the workspace; these are loaded on the next session.

6. Communication flow summary
	1.	Message arrives (from Telegram, Slack, etc.) → the gateway receives the chat event and queues it.
	2.	Client requests agent run → a control client (or auto‑routing logic) calls the agent method on the gateway with the session key; the gateway returns a runId.
	3.	Run preparation → the gateway resolves the session, workspace and skills, builds the system prompt and selects the model.
	4.	Model inference → runEmbeddedPiAgent sends the assembled context to the model (local or remote).  The model returns assistant deltas and tool calls; these are streamed back to the gateway.
	5.	Tool execution → when the model calls a tool, the gateway executes it on the host or node (subject to policy) and streams results back into the model loop.
	6.	Reply streaming → as the model produces text or tool outputs, the gateway streams assistant/tool events to clients.  When the run finishes or errors, a lifecycle end event is emitted ￼.
	7.	Persistence → the session transcript and tool results are stored in the workspace and the gateway’s session store for recall in future runs.

In essence, the gateway acts as the orchestrator and communications hub; the agent runtime builds context and drives the model loop; the model inference layer produces reasoning and tool calls using local or remote providers; and tools/skills expose functions and high‑level instructions that let the agent interact with the outside world.