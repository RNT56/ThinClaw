# ThinClaw Desktop Environment Requirements

Last updated: 2026-05-15

This document captures the environment needed to run, test, and smoke ThinClaw Desktop in local and remote modes.

## Required Toolchain

| Requirement | Version / Notes |
| --- | --- |
| Rust | Stable toolchain. Use `rustup` and keep `cargo` on `PATH`. |
| Node.js | 22.x or newer. |
| npm | npm 10.x or newer. |
| Tauri prerequisites | Platform-specific WebView and build tools from `setup.md`. |
| Git | Required for repository work and skill publish dry-runs. |
| GitHub CLI | Required only for confirmed `skill_publish` remote-write flows. Dry-runs do not require it. |

## Desktop Build Profiles

Run commands from `apps/desktop` unless noted.

| Mode | Command |
| --- | --- |
| llama.cpp dev | `npm run tauri:dev:llamacpp` |
| MLX dev | `npm run tauri:dev:mlx` |
| vLLM dev | `npm run tauri:dev:vllm` |
| Ollama dev | `npm run tauri:dev:ollama` |
| Cloud-only build | `npm run tauri:build:cloud` |

Desktop backend remains outside the root Cargo workspace unless a later CI change intentionally moves it. Use `apps/desktop/backend` for desktop backend checks.

## Required Setup Commands

```bash
cd apps/desktop
npm install
npm run setup:all
```

For MLX/vLLM:

```bash
cd apps/desktop
bash scripts/setup_uv.sh
```

For llama.cpp-only refresh:

```bash
cd apps/desktop
bash scripts/setup_llama.sh
```

## Generated Runtime Variables

ThinClaw Desktop generates these for the embedded gateway/engine process. Do not set `THINCLAW_HOME` for Desktop.

| Variable | Owner | Notes |
| --- | --- | --- |
| `THINCLAW_STATE_DIR` | Desktop | Points at the app data runtime state directory. |
| `THINCLAW_ENGINE_CONFIG` | Desktop | Runtime config path. |
| `THINCLAW_CONFIG_PATH` | Desktop | Runtime config path. |
| `THINCLAW_GATEWAY_PORT` | Desktop | Local gateway port. |
| `THINCLAW_GATEWAY_TOKEN` | Desktop | Local gateway auth token. |
| `THINCLAW_LOCAL_INFERENCE_ENABLED` | Desktop | Mirrors local inference toggle. |
| `THINCLAW_EXPOSE_INFERENCE` | Desktop | Mirrors network exposure toggle. |
| `THINCLAW_ENABLED_CLOUD_PROVIDERS` | Desktop | Provider allowlist sent to the runtime. |
| `THINCLAW_CUSTOM_LLM_ENABLED` | Desktop | Enables custom OpenAI-compatible provider path. |
| `THINCLAW_CUSTOM_LLM_URL` | User/Desktop | Custom provider base URL. |
| `THINCLAW_CUSTOM_LLM_MODEL` | User/Desktop | Custom provider model. |
| `THINCLAW_CUSTOM_LLM_KEY` | Secret-backed | Injected only when granted. |

## Optional User Variables

| Variable | Purpose |
| --- | --- |
| `ANTHROPIC_API_KEY`, `ANTHROPIC_API_KEYS` | Legacy/env fallback for Anthropic. Prefer Settings > Secrets. |
| `OPENAI_API_KEY`, `OPENAI_API_KEYS` | Legacy/env fallback for OpenAI. Prefer Settings > Secrets. |
| `GEMINI_API_KEY`, `GOOGLE_AI_API_KEY`, `GOOGLE_API_KEY` | Legacy/env fallback for Gemini. |
| `GROQ_API_KEY` | Legacy/env fallback for Groq. |
| `BRAVE_SEARCH_API_KEY` | Legacy/env fallback for Brave Search. |
| `HF_TOKEN`, `HUGGINGFACE_TOKEN` | Hugging Face gated model access. |
| `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`, `AWS_DEFAULT_REGION` | Bedrock credential fallback. Prefer Settings > Secrets. |
| `AWS_BEARER_TOKEN_BEDROCK`, `BEDROCK_API_KEY`, `BEDROCK_PROXY_API_KEY` | Bedrock bearer/proxy fallback paths. |
| `THINCLAW_REMOTE_TOKEN` | Legacy remote token fallback. Prefer Gateway settings. |
| `THINCLAW_MCP_URL`, `THINCLAW_MCP_TOKEN` | Legacy FastAPI MCP sandbox URL/token shown in Settings > MCP Server. |
| `THINCLAW_GENERATED_MEDIA_ROOTS` | Additional generated-media roots allowed for outbound media lookup. |

## Remote Gateway Requirements

Remote mode expects a ThinClaw gateway with:

- Authenticated HTTP API access.
- SSE event stream available to Desktop.
- `/api/chat`, `/api/sessions`, `/api/memory`, `/api/providers`, `/api/routines`, `/api/skills`, `/api/extensions`, `/api/mcp`, `/api/jobs`, `/api/autonomy`, `/api/experiments`, and `/api/learning` routes where the matching UI surface is enabled.
- Provider vault support for save/delete/status only. Raw secret reads are intentionally unavailable.

Unsupported remote capabilities must return an explicit unavailable reason in the Desktop UI.

## Final Gate Commands

```bash
cd apps/desktop && npm run lint:ts
cd apps/desktop && npm test
cd apps/desktop && npm run build
cd apps/desktop/backend && cargo check --locked
cd apps/desktop/backend && cargo test --locked --lib -- --skip web_search
cd /Users/mt/Programming/Schtack/ThinClaw/thinclaw-desktop && cargo test --workspace
cd apps/desktop && npx tauri info
```
