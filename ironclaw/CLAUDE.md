# IronClaw Development Guide

## Project Overview

**IronClaw** is a secure personal AI assistant that protects your data and expands its capabilities on the fly.

### Core Philosophy
- **User-first security** - Your data stays yours, encrypted and local
- **Self-expanding** - Build new tools dynamically without vendor dependency
- **Defense in depth** - Multiple security layers against prompt injection and data exfiltration
- **Always available** - Multi-channel access with proactive background execution

### Features
- **Multi-channel input**: TUI, HTTP webhooks, web gateway, native (Discord, Signal, iMessage, Gmail, Nostr, Telegram, Slack), WASM channels
- **Parallel job execution** with state machine and self-repair for stuck jobs
- **Sandbox execution**: Docker container isolation with network proxy and credential injection
- **Claude Code mode**: Delegate jobs to Claude CLI inside containers
- **Skills system**: SKILL.md prompt extensions with trust model, tool attenuation, and ClawHub registry
- **Routines**: Scheduled (cron) and reactive (event, webhook) task execution
- **Web gateway**: Browser UI with SSE/WebSocket real-time streaming
- **Extension management**: Install, auth, activate MCP/WASM extensions
- **Extensible tools**: Built-in tools, WASM sandbox, MCP client, dynamic builder
- **Persistent memory**: Workspace with hybrid search (FTS + vector via RRF)
- **Prompt injection defense**: Sanitizer, validator, policy rules, leak detection, shell env scrubbing
- **Multi-provider LLM**: OpenAI, Anthropic, Ollama, OpenAI-compatible, Tinfoil, AWS Bedrock, Google Gemini, llama.cpp
- **Setup wizard**: 9-step interactive onboarding for first-run configuration
- **Heartbeat system**: Proactive periodic execution with checklist

## Build & Test

```bash
# Format code
cargo fmt

# Lint (fix ALL warnings before committing, including pre-existing ones)
cargo clippy --all --benches --tests --examples --all-features

# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run with logging
RUST_LOG=ironclaw=debug cargo run
```

## Project Structure

> Last verified: 2026-03-13 — 375 `.rs` files across 27 modules + 19 standalone files

```
src/
├── lib.rs              # Library root, module declarations
├── main.rs             # Entry point, CLI args, startup
├── app.rs              # AppBuilder (5-phase init), AppComponents
├── error.rs            # Error types (thiserror)
│
├── boot_screen.rs      # Polished CLI startup screen
├── bootstrap.rs        # .env loading, legacy config migration
├── hardware_bridge.rs  # Sensor access bridge (camera, mic, screen)
├── i18n.rs             # Internationalization
├── qr_pairing.rs       # QR code based device pairing
├── service.rs          # launchd/systemd service management
├── settings.rs         # Settings struct, DB map serialization
├── tailscale.rs        # Tailscale integration helpers
├── talk_mode.rs        # Continuous voice conversation mode
├── tauri_commands.rs   # Tauri embedding command exports
├── testing.rs          # Test utilities
├── tracing_fmt.rs      # Custom tracing formatter
├── update_checker.rs   # Binary update checker
├── util.rs             # Shared utilities
├── voice_wake.rs       # Wake word detection (cpal audio)
│
├── agent/              # Core agent logic (30 files)
│   ├── agent_loop.rs   # Main Agent struct, message handling loop
│   ├── agent_router.rs # Multi-agent workspace routing
│   ├── commands.rs     # Agent command handlers
│   ├── compaction.rs   # Context window management with turn summarization
│   ├── context_monitor.rs # Memory pressure detection
│   ├── cost_guard.rs   # Token spending limits
│   ├── cron_stagger.rs # Cron schedule staggering
│   ├── dispatcher.rs   # Skill-aware job dispatching
│   ├── global_session.rs # Global session state
│   ├── heartbeat.rs    # Proactive periodic execution
│   ├── job_monitor.rs  # Job monitoring
│   ├── management_api.rs # Management API endpoints
│   ├── presence.rs     # Presence tracking (beacons, stale pruning)
│   ├── router.rs       # MessageIntent classification
│   ├── routine.rs      # Routine types (Trigger, Action, Guardrails)
│   ├── routine_audit.rs # Routine audit logging
│   ├── routine_engine.rs # Routine execution (cron ticker, event matcher)
│   ├── runtime_behavior.rs # Runtime behavior configuration
│   ├── scheduler.rs    # Parallel job scheduling
│   ├── self_repair.rs  # Stuck job detection and recovery
│   ├── session.rs      # Session/thread/turn model with state machine
│   ├── session_manager.rs # Thread/session lifecycle management
│   ├── subagent_executor.rs # Sub-agent job execution
│   ├── submission.rs   # Submission parsing (undo, redo, compact, etc.)
│   ├── task.rs         # Sub-task execution framework
│   ├── thread_inheritance.rs # Thread context inheritance
│   ├── thread_ops.rs   # Thread operations
│   ├── undo.rs         # Turn-based undo/redo with checkpoints
│   └── worker.rs       # Per-job execution with LLM reasoning
│
├── api/                # Public REST API types (10 files)
│   ├── chat.rs         # Chat API types
│   ├── config.rs       # Config API
│   ├── extensions.rs   # Extension API
│   ├── memory.rs       # Memory API
│   ├── routines.rs     # Routines API
│   ├── sessions.rs     # Sessions API
│   ├── skills.rs       # Skills API
│   └── system.rs       # System API
│
├── channels/           # Multi-channel input (24 files + 2 subdirs)
│   ├── channel.rs      # Channel trait, IncomingMessage, OutgoingResponse
│   ├── manager.rs      # ChannelManager merges streams
│   ├── repl.rs         # Simple REPL (for testing)
│   ├── http.rs         # HTTP webhook (axum) with secret validation
│   ├── discord.rs      # Discord native channel
│   ├── gmail.rs        # Gmail channel
│   ├── gmail_wiring.rs # Gmail OAuth + IMAP wiring
│   ├── imessage.rs     # iMessage channel (macOS)
│   ├── imessage_wiring.rs # iMessage bridge wiring
│   ├── nostr.rs        # Nostr protocol channel
│   ├── signal.rs       # Signal native channel (signal-cli)
│   ├── slack.rs        # Slack channel
│   ├── telegram.rs     # Telegram native channel
│   ├── canvas_gateway.rs # Canvas hosting gateway routes
│   ├── webhook_server.rs # Generic webhook ingestion
│   ├── ack_reaction.rs # Acknowledgment reactions
│   ├── forward_download.rs # Media download forwarding
│   ├── group_priming.rs # Group chat priming
│   ├── health_monitor.rs # Channel health monitoring
│   ├── reaction_machine.rs # Reaction state machine
│   ├── self_message.rs # Self-messaging
│   ├── status_view.rs  # Channel status views
│   ├── tool_stream.rs  # Tool streaming
│   ├── web/            # Web gateway (browser UI)
│   │   ├── server.rs   # Axum router, 40+ API endpoints
│   │   ├── sse.rs      # SSE broadcast manager
│   │   ├── ws.rs       # WebSocket gateway + connection tracking
│   │   ├── auth.rs     # Bearer token auth middleware
│   │   ├── log_layer.rs # Tracing layer for log streaming
│   │   └── static/     # HTML, CSS, JS (single-page app)
│   └── wasm/           # WASM channel runtime
│       ├── bundled.rs  # Bundled channel discovery
│       └── wrapper.rs  # Channel trait wrapper for WASM modules
│
├── cli/                # CLI subcommands (25 files)
│   ├── agents.rs       # Agent management
│   ├── browser.rs      # Browser automation commands
│   ├── channels.rs     # Channel management
│   ├── completion.rs   # Shell completion generation
│   ├── config.rs       # Config management
│   ├── cron.rs         # Cron/routine management
│   ├── doctor.rs       # Diagnostics (DB, binary, LLM, Tailscale)
│   ├── gateway.rs      # Gateway start/stop/status
│   ├── logs.rs         # Log viewing
│   ├── mcp.rs          # MCP server management
│   ├── memory.rs       # Memory CLI (read, write, search, tree)
│   ├── message.rs      # Single message mode
│   ├── models.rs       # Model listing/selection
│   ├── nodes.rs        # Multi-node management
│   ├── oauth_defaults.rs # OAuth provider defaults
│   ├── pairing.rs      # DM pairing commands
│   ├── registry.rs     # Extension registry
│   ├── service.rs      # Service install/start/stop/status
│   ├── session_export.rs # Session export (md/json/csv/html)
│   ├── sessions.rs     # Session management
│   ├── status.rs       # Status diagnostics
│   ├── subagent_spawn.rs # Sub-agent spawn commands
│   ├── tool.rs         # Tool management
│   └── update.rs       # Binary update
│
├── config/             # Structured configuration (24 files)
│   ├── mod.rs          # Config struct, LlmBackend enum, from_env
│   ├── agent.rs        # Agent config
│   ├── builder.rs      # Config builder
│   ├── channels.rs     # Channel config
│   ├── database.rs     # Database config
│   ├── embeddings.rs   # Embeddings config
│   ├── heartbeat.rs    # Heartbeat config
│   ├── hygiene.rs      # Memory hygiene config
│   ├── llm.rs          # LLM provider config
│   ├── mdns_discovery.rs # Bonjour/mDNS discovery
│   ├── model_compat.rs # Model compatibility tables
│   ├── network_modes.rs # Loopback/LAN/remote modes
│   ├── provider_catalog.rs # Provider presets
│   ├── routines.rs     # Routines config
│   ├── safety.rs       # Safety layer config
│   ├── sandbox.rs      # Sandbox config
│   ├── secrets.rs      # Secrets config
│   ├── skills.rs       # Skills config
│   ├── tunnel.rs       # Tunnel config
│   ├── wasm.rs         # WASM runtime config
│   ├── watcher.rs      # Config file watcher
│   └── webchat.rs      # Web chat config
│
├── extensions/         # Extension management (11 files)
│   ├── manager.rs      # ExtensionManager
│   ├── registry.rs     # Extension registry
│   ├── clawhub.rs      # ClawHub registry client
│   ├── discovery.rs    # Extension discovery
│   ├── ext_health_monitor.rs # Extension health monitoring
│   ├── lifecycle_hooks.rs # Extension lifecycle hooks
│   ├── manifest_validator.rs # Manifest validation
│   ├── plugin_interfaces.rs # Plugin interfaces
│   ├── plugin_manifest.rs # Plugin manifest types
│   └── plugin_routes.rs # Plugin API routes
│
├── hooks/              # Lifecycle hook system (5 files)
│   ├── mod.rs          # HookEvent, HookRegistry
│   ├── hook.rs         # Hook trait
│   ├── bootstrap.rs    # Bootstrap hooks
│   ├── bundled.rs      # Bundled hook implementations
│   └── registry.rs     # Hook registry
│
├── llm/                # LLM integration (22 files)
│   ├── mod.rs          # Provider factory, LlmBackend enum
│   ├── provider.rs     # LlmProvider trait, message types
│   ├── rig_adapter.rs  # Rig framework adapter
│   ├── reasoning.rs    # Planning, tool selection, evaluation
│   ├── circuit_breaker.rs # Circuit breaker for provider failures
│   ├── retry.rs        # Retry with exponential backoff
│   ├── failover.rs     # Multi-provider failover chain
│   ├── smart_routing.rs # Cost-optimized model routing (cheap vs primary)
│   ├── routing_policy.rs # Routing policy configuration
│   ├── response_cache.rs # LLM response caching
│   ├── response_cache_ext.rs # Cache extensions
│   ├── costs.rs        # Token cost definitions
│   ├── cost_tracker.rs # Token cost tracking
│   ├── discovery.rs    # Auto model discovery (queries /v1/models)
│   ├── embeddings.rs   # Embedding provider
│   ├── extended_context.rs # Extended context handling
│   ├── provider_presets.rs # Provider configuration presets
│   ├── bedrock.rs      # AWS Bedrock provider adapter
│   ├── gemini.rs       # Google Gemini provider adapter
│   ├── llama_cpp.rs    # Native llama.cpp inference interface
│   ├── llm_hooks.rs    # LLM lifecycle hooks
│   └── llms_txt.rs     # LLMs.txt support
│
├── media/              # Media processing (12 files)
│   ├── audio.rs        # Audio processing
│   ├── video.rs        # Video pipeline
│   ├── image.rs        # Image processing
│   ├── pdf.rs          # PDF extraction
│   ├── sticker.rs      # Sticker handling
│   ├── tts.rs          # Text-to-speech (OpenAI)
│   ├── tts_streaming.rs # Streaming TTS
│   ├── cache.rs        # Media cache
│   ├── limits.rs       # Size/duration limits
│   ├── media_cache_config.rs # Cache configuration
│   └── types.rs        # Shared media types
│
├── observability/      # Observability (5 files)
│   ├── log.rs          # Log layer
│   ├── multi.rs        # Multi-subscriber
│   ├── noop.rs         # No-op implementation
│   └── traits.rs       # Observable trait
│
├── orchestrator/       # Internal HTTP API for sandbox containers
│   ├── api.rs          # Axum endpoints (LLM proxy, events, prompts)
│   ├── auth.rs         # Per-job bearer token store
│   └── job_manager.rs  # Container lifecycle (create, stop, cleanup)
│
├── pairing/            # Device pairing (2 files)
│   ├── mod.rs          # Pairing protocol
│   └── store.rs        # Pairing state storage
│
├── registry/           # Extension registry (6 files)
│   ├── artifacts.rs    # Artifact resolution
│   ├── catalog.rs      # Catalog cache
│   ├── embedded.rs     # Embedded registry
│   ├── installer.rs    # Extension installer
│   └── manifest.rs     # Registry manifest types
│
├── safety/             # Security layer (13 files)
│   ├── sanitizer.rs    # Pattern detection, content escaping
│   ├── validator.rs    # Input validation (length, encoding, patterns)
│   ├── policy.rs       # PolicyRule system with severity/actions
│   ├── leak_detector.rs # Secret detection (API keys, tokens, etc.)
│   ├── credential_detect.rs # Credential pattern detection
│   ├── auth_profiles.rs # Auth profile management
│   ├── dangerous_tools.rs # Dangerous tool detection
│   ├── device_pairing.rs # Device pairing security
│   ├── elevated.rs     # Elevated permission checks
│   ├── key_rotation.rs # Secret key rotation
│   ├── media_url.rs    # Media URL validation
│   └── skill_path.rs   # Skill path traversal prevention
│
├── sandbox/            # Docker execution sandbox (9 files)
│   ├── config.rs       # SandboxConfig, SandboxPolicy enum
│   ├── manager.rs      # SandboxManager orchestration
│   ├── container.rs    # ContainerRunner, Docker lifecycle
│   ├── error.rs        # SandboxError types
│   └── proxy/          # Network proxy for containers
│       ├── http.rs     # HttpProxy, CredentialResolver trait
│       ├── policy.rs   # NetworkPolicyDecider trait
│       └── allowlist.rs # DomainAllowlist validation
│
├── secrets/            # Secrets management (5 files)
│   ├── crypto.rs       # AES-256-GCM encryption
│   ├── store.rs        # Secret storage
│   └── types.rs        # Credential types
│
├── setup/              # Onboarding wizard (spec: src/setup/README.md)
│   ├── mod.rs          # Entry point, check_onboard_needed()
│   ├── wizard.rs       # 9-step interactive wizard
│   ├── channels.rs     # Channel setup helpers
│   └── prompts.rs      # Terminal prompts (select, confirm, secret)
│
├── skills/             # SKILL.md prompt extension system (7 files)
│   ├── registry.rs     # SkillRegistry: discover, install, remove
│   ├── selector.rs     # Deterministic scoring prefilter
│   ├── attenuation.rs  # Trust-based tool ceiling
│   ├── gating.rs       # Requirement checks (bins, env, config)
│   ├── parser.rs       # SKILL.md frontmatter + markdown parser
│   └── catalog.rs      # ClawHub registry client
│
├── tools/              # Extensible tool system (7 files + 4 subdirs)
│   ├── tool.rs         # Tool trait, ToolOutput, ToolError
│   ├── registry.rs     # ToolRegistry for discovery
│   ├── sandbox.rs      # Process-based sandbox (stub, superseded by wasm/)
│   ├── builtin/        # Built-in tools (25 files)
│   │   ├── echo.rs, time.rs, json.rs, http.rs
│   │   ├── file.rs     # ReadFile, WriteFile, ListDir, ApplyPatch
│   │   ├── shell.rs    # Shell command execution
│   │   ├── memory.rs   # Memory tools (search, write, read, tree)
│   │   ├── job.rs      # CreateJob, ListJobs, JobStatus, CancelJob
│   │   ├── routine.rs  # routine_create/list/update/delete/history
│   │   ├── extension_tools.rs # Extension install/auth/activate/remove
│   │   ├── skill_tools.rs # skill_list/search/install/remove tools
│   │   ├── canvas.rs   # A2UI canvas tool
│   │   ├── browser.rs  # Browser automation (headless Chrome)
│   │   ├── subagent.rs # Sub-agent delegation tool
│   │   ├── agent_control.rs # Agent control (pause/resume/status)
│   │   ├── camera_capture.rs # Camera capture (via hardware bridge)
│   │   ├── screen_capture.rs # Screen capture (via hardware bridge)
│   │   ├── device_info.rs # Device information tool
│   │   ├── location.rs # Location services
│   │   ├── html_converter.rs # HTML → markdown conversion
│   │   ├── tts.rs      # Text-to-speech tool
│   │   ├── discord_actions.rs # Discord-specific actions
│   │   ├── slack_actions.rs # Slack-specific actions
│   │   └── telegram_actions.rs # Telegram-specific actions
│   ├── builder/        # Dynamic tool building
│   │   ├── core.rs, templates.rs, testing.rs, validation.rs
│   ├── mcp/            # Model Context Protocol (HTTP client)
│   │   ├── client.rs, protocol.rs
│   └── wasm/           # Full WASM sandbox (wasmtime)
│       ├── runtime.rs, wrapper.rs, host.rs, limits.rs
│       ├── allowlist.rs, credential_injector.rs
│       ├── loader.rs, rate_limiter.rs, storage.rs
│
├── tunnel/             # Tunnel providers (6 files)
│   ├── tailscale.rs    # Tailscale serve + funnel
│   ├── cloudflare.rs   # Cloudflare Tunnel
│   ├── ngrok.rs        # ngrok tunnel
│   ├── custom.rs       # Custom tunnel
│   └── none.rs         # No tunnel
│
├── tui/                # Terminal UI (1 file)
│   └── mod.rs          # TUI framework
│
├── db/                 # Database abstraction layer
│   ├── mod.rs          # Database trait (~60 async methods)
│   ├── postgres.rs     # PostgreSQL backend (delegates to Store + Repository)
│   └── libsql_backend.rs # libSQL/Turso backend (embedded SQLite)
│
├── workspace/          # Persistent memory system (11 files)
│   ├── mod.rs          # Workspace struct, memory operations
│   ├── document.rs     # MemoryDocument, MemoryChunk, WorkspaceEntry
│   ├── chunker.rs      # Document chunking (800 tokens, 15% overlap)
│   ├── embeddings.rs   # EmbeddingProvider trait
│   ├── search.rs       # Hybrid search with RRF algorithm
│   └── repository.rs   # PostgreSQL CRUD and search operations
│
├── context/            # Job context isolation
│   ├── state.rs, memory.rs, manager.rs
│
├── estimation/         # Cost/time/value estimation
│   ├── cost.rs, time.rs, value.rs, learner.rs
│
├── evaluation/         # Success evaluation
│   ├── success.rs, metrics.rs
│
└── history/            # Persistence
    ├── store.rs        # PostgreSQL repositories
    └── analytics.rs    # Aggregation queries (JobStats, ToolStats)
```

## Key Patterns

### Architecture

When designing new features or systems, always prefer generic/extensible architectures over hardcoding specific integrations. Ask clarifying questions about the desired abstraction level before implementing.

### Error Handling
- Use `thiserror` for error types in `error.rs`
- Never use `.unwrap()` or `.expect()` in production code (tests are fine)
- Map errors with context: `.map_err(|e| SomeError::Variant { reason: e.to_string() })?`
- Before committing, grep for `.unwrap()` and `.expect(` in changed files to catch violations mechanically

### Async
- All I/O is async with tokio
- Use `Arc<T>` for shared state across tasks
- Use `RwLock` for concurrent read/write access

### Traits for Extensibility
- `Database` - Add new database backends (must implement all ~60 methods)
- `Channel` - Add new input sources
- `Tool` - Add new capabilities
- `LlmProvider` - Add new LLM backends
- `SuccessEvaluator` - Custom evaluation logic
- `EmbeddingProvider` - Add embedding backends (workspace search)
- `NetworkPolicyDecider` - Custom network access policies for sandbox containers

### Tool Implementation
```rust
#[async_trait]
impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Does something useful" }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "param": { "type": "string", "description": "A parameter" }
            },
            "required": ["param"]
        })
    }

    async fn execute(&self, params: serde_json::Value, ctx: &JobContext)
        -> Result<ToolOutput, ToolError>
    {
        let start = std::time::Instant::now();
        // ... do work ...
        Ok(ToolOutput::text("result", start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool { true } // External data
}
```

### State Transitions
Job states follow a defined state machine in `context/state.rs`:
```
Pending -> InProgress -> Completed -> Submitted -> Accepted
                     \-> Failed
                     \-> Stuck -> InProgress (recovery)
                              \-> Failed
```

### Code Style

- Use `crate::` imports, not `super::`
- No `pub use` re-exports unless exposing to downstream consumers
- Prefer strong types over strings (enums, newtypes)
- Keep functions focused, extract helpers when logic is reused
- Comments for non-obvious logic only

### Review & Fix Discipline

Hard-won lessons from code review -- follow these when fixing bugs or addressing review feedback.

**Fix the pattern, not just the instance:** When a reviewer flags a bug (e.g., TOCTOU race in INSERT + SELECT-back), search the entire codebase for all instances of that same pattern. A fix in `SecretsStore::create()` that doesn't also fix `WasmToolStore::store()` is half a fix.

**Propagate architectural fixes to satellite types:** If a core type changes its concurrency model (e.g., `LibSqlBackend` switches to connection-per-operation), every type that was handed a resource from the old model (e.g., `LibSqlSecretsStore`, `LibSqlWasmToolStore` holding a single `Connection`) must also be updated. Grep for the old type across the codebase.

**Schema translation is more than DDL:** When translating a database schema between backends (PostgreSQL to libSQL, etc.), check for:
- **Indexes** -- diff `CREATE INDEX` statements between the two schemas
- **Seed data** -- check for `INSERT INTO` in migrations (e.g., `leak_detection_patterns`)
- **Semantic differences** -- document where SQL functions behave differently (e.g., `json_patch` vs `jsonb_set`)

**Feature flag testing:** When adding feature-gated code, test compilation with each feature in isolation:
```bash
cargo check                                          # default features
cargo check --no-default-features --features libsql  # libsql only
cargo check --all-features                           # all features
```
Dead code behind the wrong `#[cfg]` gate will only show up when building with a single feature.

**Zero clippy warnings policy:** Fix ALL clippy warnings before committing, including pre-existing ones in files you didn't change. Never leave warnings behind — treat `cargo clippy` output as a zero-tolerance gate.

**Mechanical verification before committing:** Run these checks on changed files before committing:
- `cargo clippy --all --benches --tests --examples --all-features` -- zero warnings
- `grep -rnE '\.unwrap\(|\.expect\(' <files>` -- no panics in production
- `grep -rn 'super::' <files>` -- use `crate::` imports
- If you fixed a pattern bug, `grep` for other instances of that pattern across `src/`

## Configuration

Environment variables (see `.env.example`):
```bash
# Database backend (default: postgres)
DATABASE_BACKEND=postgres               # or "libsql" / "turso"
DATABASE_URL=postgres://user:pass@localhost/ironclaw
LIBSQL_PATH=~/.ironclaw/ironclaw.db    # libSQL local path (default)
# LIBSQL_URL=libsql://xxx.turso.io    # Turso cloud (optional)
# LIBSQL_AUTH_TOKEN=xxx                # Required with LIBSQL_URL

# LLM provider (default: openai_compatible)
LLM_BACKEND=openai_compatible           # openai, anthropic, ollama, openai_compatible, tinfoil
LLM_BASE_URL=https://openrouter.ai/api/v1  # For OpenAI-compatible
LLM_API_KEY=sk-...                      # API key for the provider
LLM_MODEL=anthropic/claude-sonnet-4-20250514

# Agent settings
AGENT_NAME=ironclaw
MAX_PARALLEL_JOBS=5

# Embeddings (for semantic memory search)
OPENAI_API_KEY=sk-...                   # For OpenAI embeddings
EMBEDDING_MODEL=text-embedding-3-small  # or text-embedding-3-large

# Heartbeat (proactive periodic execution)
HEARTBEAT_ENABLED=true
HEARTBEAT_INTERVAL_SECS=1800            # 30 minutes
HEARTBEAT_NOTIFY_CHANNEL=tui
HEARTBEAT_NOTIFY_USER=default

# Web gateway
GATEWAY_ENABLED=true
GATEWAY_HOST=127.0.0.1
GATEWAY_PORT=3001
GATEWAY_AUTH_TOKEN=changeme           # Required for API access
GATEWAY_USER_ID=default

# Docker sandbox
SANDBOX_ENABLED=true
SANDBOX_IMAGE=ironclaw-worker:latest
SANDBOX_MEMORY_LIMIT_MB=512
SANDBOX_TIMEOUT_SECS=1800
SANDBOX_CPU_LIMIT=1.0                  # CPU cores per container
SANDBOX_NETWORK_PROXY=true             # Enable network proxy for containers
SANDBOX_PROXY_PORT=8080                # Proxy listener port
SANDBOX_DEFAULT_POLICY=workspace_write # ReadOnly, WorkspaceWrite, FullAccess

# Claude Code mode (runs inside sandbox containers)
CLAUDE_CODE_ENABLED=false
CLAUDE_CODE_MODEL=claude-sonnet-4-20250514
CLAUDE_CODE_MAX_TURNS=50
CLAUDE_CODE_CONFIG_DIR=/home/worker/.claude

# Routines (scheduled/reactive execution)
ROUTINES_ENABLED=true
ROUTINES_CRON_INTERVAL=60            # Tick interval in seconds
ROUTINES_MAX_CONCURRENT=3

# Skills system
SKILLS_ENABLED=true
SKILLS_MAX_TOKENS=4000                 # Max prompt budget per turn
SKILLS_CATALOG_URL=https://clawhub.dev # ClawHub registry URL
SKILLS_AUTO_DISCOVER=true              # Scan skill directories on startup

# Tinfoil private inference
TINFOIL_API_KEY=...                    # Required when LLM_BACKEND=tinfoil
TINFOIL_MODEL=kimi-k2-5               # Default model
```

### LLM Providers

IronClaw supports multiple LLM backends via the `LLM_BACKEND` env var: `openai`, `anthropic`, `ollama`, `openai_compatible` (default), and `tinfoil`.

**OpenAI-compatible** (default) -- Any endpoint that speaks the OpenAI API (vLLM, LiteLLM, OpenRouter, etc.). Configure with `LLM_BASE_URL`, `LLM_API_KEY` (optional), `LLM_MODEL`. Set `LLM_EXTRA_HEADERS` to inject custom HTTP headers into every request (format: `Key:Value,Key2:Value2`), useful for OpenRouter attribution headers like `HTTP-Referer` and `X-Title`.

**Tinfoil** -- Private inference via `https://inference.tinfoil.sh/v1`. Runs models inside hardware-attested TEEs so neither Tinfoil nor the cloud provider can see prompts or responses. Uses the OpenAI-compatible Chat Completions API. Configure with `TINFOIL_API_KEY` and `TINFOIL_MODEL` (default: `kimi-k2-5`).

**AWS Bedrock** (`src/llm/bedrock.rs`) -- Adapts AWS Bedrock to the OpenAI-compatible format. Uses standard AWS credentials.

**Google Gemini** (`src/llm/gemini.rs`) -- Adapts Google Gemini API (via AI Studio) to the OpenAI-compatible format.

**llama.cpp** (`src/llm/llama_cpp.rs`) -- Native inference interface for local model loading via llama.cpp FFI bindings.

**Smart Routing** (`src/llm/smart_routing.rs`) -- Cost-optimized routing that sends simple tasks to cheap models and complex tasks to primary models. Enabled via `SMART_ROUTING_ENABLED=true`.

## Database

IronClaw supports two database backends, selected at compile time via Cargo feature flags and at runtime via the `DATABASE_BACKEND` environment variable.

**IMPORTANT: All new features that touch persistence MUST support both backends.** Implement the operation as a method on the `Database` trait in `src/db/mod.rs`, then add the implementation in both `src/db/postgres.rs` (delegate to Store/Repository) and `src/db/libsql_backend.rs` (native SQL).

### Backends

| Backend | Feature Flag | Default | Use Case |
|---------|-------------|---------|----------|
| PostgreSQL | `postgres` (default) | Yes | Production, existing deployments |
| libSQL/Turso | `libsql` | No | Zero-dependency local mode, edge, Turso cloud |

```bash
# Build with PostgreSQL only (default)
cargo build

# Build with libSQL only
cargo build --no-default-features --features libsql

# Build with both backends available
cargo build --features "postgres,libsql"
```

### Database Trait

The `Database` trait (`src/db/mod.rs`) defines ~60 async methods covering all persistence:
- Conversations, messages, metadata
- Jobs, actions, LLM calls, estimation snapshots
- Sandbox jobs, job events
- Routines, routine runs
- Tool failures, settings
- Workspace: documents, chunks, hybrid search

Both backends implement this trait. PostgreSQL delegates to the existing `Store` + `Repository`. libSQL implements native SQLite-dialect SQL.

### Schema

**PostgreSQL:** `migrations/V1__initial.sql` (351 lines). Uses pgvector for embeddings, tsvector for FTS, PL/pgSQL functions. Managed by `refinery`.

**libSQL:** `src/db/libsql_migrations.rs` (consolidated schema, ~480 lines). Translates PG types:
- `UUID` -> `TEXT`, `TIMESTAMPTZ` -> `TEXT` (ISO-8601), `JSONB` -> `TEXT`
- `VECTOR(1536)` -> `F32_BLOB(1536)` with `libsql_vector_idx`
- `tsvector`/`ts_rank_cd` -> FTS5 virtual table with sync triggers
- PL/pgSQL functions -> SQLite triggers

**Tables (both backends):**

**Core:**
- `conversations` - Multi-channel conversation tracking
- `agent_jobs` - Job metadata and status
- `job_actions` - Event-sourced tool executions
- `dynamic_tools` - Agent-built tools
- `llm_calls` - Cost tracking
- `estimation_snapshots` - Learning data

**Workspace/Memory:**
- `memory_documents` - Flexible path-based files (e.g., "context/vision.md", "daily/2024-01-15.md")
- `memory_chunks` - Chunked content with FTS and vector indexes
- `heartbeat_state` - Periodic execution tracking

**Other:**
- `routines`, `routine_runs` - Scheduled/reactive execution
- `settings` - Per-user key-value settings
- `tool_failures` - Self-repair tracking
- `secrets`, `wasm_tools`, `tool_capabilities` - Extension infrastructure

Database configuration: see Configuration section above.

### Current Limitations (libSQL backend)

- **Workspace/memory system** not yet wired through Database trait (requires Store migration)
- **Secrets store** not yet available (still requires PostgresSecretsStore)
- **Hybrid search** uses FTS5 only (vector search via libsql_vector_idx not yet implemented)
- **Settings reload from DB** skipped (Config::from_db requires Store)
- No incremental migration versioning (schema is CREATE IF NOT EXISTS, no ALTER TABLE support yet)
- **No encryption at rest** -- The local SQLite database file stores conversation content, job data, workspace memory, and other application data in plaintext. Only secrets (API tokens, credentials) are encrypted via AES-256-GCM before storage. Users handling sensitive data should use full-disk encryption (FileVault, LUKS, BitLocker) or consider the PostgreSQL backend with TDE/encrypted storage.
- **JSON merge patch vs path-targeted update** -- The libSQL backend uses RFC 7396 JSON Merge Patch (`json_patch`) for metadata updates, while PostgreSQL uses path-targeted `jsonb_set`. Merge patch replaces top-level keys entirely, which may drop nested keys not present in the patch. Callers should avoid relying on partial nested object updates in metadata fields.

## Safety Layer

All external tool output passes through `SafetyLayer`:
1. **Sanitizer** - Detects injection patterns, escapes dangerous content
2. **Validator** - Checks length, encoding, forbidden patterns
3. **Policy** - Rules with severity (Critical/High/Medium/Low) and actions (Block/Warn/Review/Sanitize)
4. **Leak Detector** - Scans for 15+ secret patterns (API keys, tokens, private keys, connection strings) at two points: tool output before it reaches the LLM, and LLM responses before they reach the user. Actions per pattern: Block (reject entirely), Redact (mask the secret), or Warn (flag but allow)

Tool outputs are wrapped before reaching LLM:
```xml
<tool_output name="search" sanitized="true">
[escaped content]
</tool_output>
```

### Shell Environment Scrubbing

The shell tool (`src/tools/builtin/shell.rs`) scrubs sensitive environment variables before executing commands, preventing secrets from leaking through `env`, `printenv`, or `$VAR` expansion. The sanitizer (`src/safety/sanitizer.rs`) also detects command injection patterns (chained commands, subshells, path traversal) and blocks or escapes them based on policy rules.

## Skills System

Skills are SKILL.md files that extend the agent's prompt with domain-specific instructions. Each skill is a YAML frontmatter block (metadata, activation criteria, required tools) followed by a markdown body that gets injected into the LLM context when the skill activates.

### Trust Model

| Trust Level | Source | Tool Access |
|-------------|--------|-------------|
| **Trusted** | User-placed in `~/.ironclaw/skills/` or workspace `skills/` | All tools available to the agent |
| **Installed** | Downloaded from ClawHub registry | Read-only tools only (no shell, file write, HTTP) |

### SKILL.md Format

```yaml
---
name: my-skill
version: 0.1.0
description: Does something useful
activation:
  patterns:
    - "deploy to.*production"
  keywords:
    - "deployment"
  max_context_tokens: 2000
metadata:
  openclaw:
    requires:
      bins: [docker, kubectl]
      env: [KUBECONFIG]
---

# Deployment Skill

Instructions for the agent when this skill activates...
```

### Selection Pipeline

1. **Gating** -- Check binary/env/config requirements; skip skills whose prerequisites are missing
2. **Scoring** -- Deterministic scoring against message content using keywords, tags, and regex patterns
3. **Budget** -- Select top-scoring skills that fit within `SKILLS_MAX_TOKENS` prompt budget
4. **Attenuation** -- Apply trust-based tool ceiling; installed skills lose access to dangerous tools

### Skill Tools

Four built-in tools for managing skills at runtime:
- **`skill_list`** -- List all discovered skills with trust level and status
- **`skill_search`** -- Search ClawHub registry for available skills
- **`skill_install`** -- Download and install a skill from ClawHub
- **`skill_remove`** -- Remove an installed skill

### Skill Directories

- `~/.ironclaw/skills/` -- User's global skills (trusted)
- `<workspace>/skills/` -- Per-workspace skills (trusted)
- `~/.ironclaw/installed_skills/` -- Registry-installed skills (installed trust)

### Testing Skills

- `skills/web-ui-test/` -- Manual test checklist for the web gateway UI via Claude for Chrome extension. Covers connection, chat, skills search/install/remove, and other tabs.

Skills configuration: see Configuration section above.

## Docker Sandbox

The `src/sandbox/` module provides Docker-based isolation for job execution with a network proxy that controls outbound access and injects credentials.

### Sandbox Policies

| Policy | Filesystem | Network | Use Case |
|--------|-----------|---------|----------|
| **ReadOnly** | Read-only workspace mount | Allowlisted domains only | Analysis, code review |
| **WorkspaceWrite** | Read-write workspace mount | Allowlisted domains only | Code generation, file edits |
| **FullAccess** | Full filesystem | Unrestricted | Trusted admin tasks |

### Network Proxy

Containers route all HTTP/HTTPS traffic through a host-side proxy (`src/sandbox/proxy/`):
- **Domain allowlist** -- Only allowlisted domains are reachable (default: package registries, docs sites, GitHub, common APIs)
- **Credential injection** -- The `CredentialResolver` trait injects auth headers into proxied requests so secrets never enter the container environment
- **CONNECT tunnel** -- HTTPS traffic uses CONNECT method; the proxy validates the target domain against the allowlist before establishing the tunnel
- **Policy decisions** -- The `NetworkPolicyDecider` trait allows custom logic for allow/deny/inject decisions per request

### Zero-Exposure Credential Model

Secrets (API keys, tokens) are stored encrypted on the host and injected into HTTP requests by the proxy at transit time. Container processes never have access to raw credential values, preventing exfiltration even if container code is compromised.

Sandbox configuration: see Configuration section above.

## Testing

Tests are in `mod tests {}` blocks at the bottom of each file. Run specific module tests:
```bash
cargo test safety::sanitizer::tests
cargo test tools::registry::tests
```

Key test patterns:
- Unit tests for pure functions
- Async tests with `#[tokio::test]`
- No mocks, prefer real implementations or stubs

## Current Limitations / TODOs

1. **Integration tests** - Need testcontainers setup for PostgreSQL
2. **MCP stdio transport** - Only HTTP transport implemented
3. **WIT bindgen integration** - Auto-extract tool description/schema from WASM modules (stubbed)
4. **Capability granting after tool build** - Built tools get empty capabilities; need UX for granting HTTP/secrets access
5. **Tool versioning workflow** - No version tracking or rollback for dynamically built tools
6. **Webhook trigger endpoint** - Routines webhook trigger not yet exposed in web gateway
7. **Full channel status view** - Gateway status widget exists, but no per-channel connection dashboard

## Tool Architecture

**Keep tool-specific logic out of the main agent codebase.** The main agent provides generic infrastructure; tools are self-contained units that declare their requirements through `capabilities.json` files (API endpoints, credentials, rate limits, auth setup). Service-specific auth flows, CLI commands, and configuration do not belong in the main agent.

Tools can be built as **WASM** (sandboxed, credential-injected, single binary) or **MCP servers** (ecosystem of pre-built servers, any language, but no sandbox). Both are first-class via `ironclaw tool install`. Auth is declared in capabilities files with OAuth and manual token entry support.

See `src/tools/README.md` for full tool architecture, adding new tools (built-in Rust and WASM), auth JSON examples, and WASM vs MCP decision guide.

## Adding a New Channel

1. Create `src/channels/my_channel.rs`
2. Implement the `Channel` trait
3. Add config in `src/config/channels.rs`
4. Wire up in `main.rs` channel setup section

## Debugging

```bash
# Verbose logging
RUST_LOG=ironclaw=trace cargo run

# Just the agent module
RUST_LOG=ironclaw::agent=debug cargo run

# With HTTP request logging
RUST_LOG=ironclaw=debug,tower_http=debug cargo run
```

## Module Specifications

Some modules have a `README.md` that serves as the authoritative specification
for that module's behavior. When modifying code in a module that has a spec:

1. **Read the spec first** before making changes
2. **Code follows spec**: if the spec says X, the code must do X
3. **Update both sides**: if you change behavior, update the spec to match;
   if you're implementing a spec change, update the code to match
4. **Spec is the tiebreaker**: when code and spec disagree, the spec is correct
   (unless the spec is clearly outdated, in which case fix the spec first)

| Module | Spec File |
|--------|-----------|
| `src/setup/` | `src/setup/README.md` |
| `src/workspace/` | `src/workspace/README.md` |
| `src/tools/` | `src/tools/README.md` |

## Workspace & Memory System

OpenClaw-inspired persistent memory with a flexible filesystem-like structure. Principle: "Memory is database, not RAM" -- if you want to remember something, write it explicitly. Uses hybrid search combining FTS (keyword) + vector (semantic) via Reciprocal Rank Fusion.

Four memory tools for LLM use: `memory_search` (hybrid search -- call before answering questions about prior work), `memory_write`, `memory_read`, `memory_tree`. Identity files (AGENTS.md, SOUL.md, USER.md, IDENTITY.md) are injected into the LLM system prompt.

The heartbeat system runs proactive periodic execution (default: 30 minutes), reading `HEARTBEAT.md` and notifying via channel if findings are detected.

See `src/workspace/README.md` for full API documentation, filesystem structure, hybrid search details, chunking strategy, and heartbeat system.
