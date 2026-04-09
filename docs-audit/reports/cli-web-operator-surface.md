# CLI / Web UI / Operator Surface Audit

## Executive Summary

ThinClaw’s operator surface is coherent in code: a `clap` CLI front door, a web gateway control plane, and a browser UI built around the same runtime state. The docs are broadly directionally correct, but they spread the operator story across `README.md`, `docs/DEPLOYMENT.md`, and `FEATURE_PARITY.md` in ways that make the canonical workflows harder to see.

The right documentation split is simple: keep the README high-level, make `docs/DEPLOYMENT.md` the operator reference, and treat the CLI and gateway code as the source of truth for command names and routes.

## Actual Operator Surface

- The CLI root in [`src/cli/mod.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/mod.rs#L100) exposes `run`, `onboard`, `config`, `cron`, `gateway`, `channels`, `tool`, `registry`, `mcp`, `memory`, `message`, `models`, `pairing`, `agents`, `sessions`, `service`, `doctor`, `status`, `logs`, `browser`, `update`, and `completion`.
- `gateway` is an explicit operator command family in [`src/cli/gateway.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/gateway.rs#L11) with `start`, `stop`, and `status`.
- `tool` is the WASM tool manager in [`src/cli/tool.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/tool.rs#L27) with `install`, `list`, `remove`, `info`, and `auth`.
- `mcp` is the remote tool-server manager in [`src/cli/mcp.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/mcp.rs#L21) with `add`, `remove`, `list`, `auth`, `test`, and `toggle`.
- `message` is a direct gateway injection path in [`src/cli/message.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/message.rs#L10).
- `models`, `logs`, `sessions`, `agents`, and `browser` are all real operator subcommands with distinct operational purposes.
- The gateway server in [`src/channels/web/server.rs`](/Users/vespian/coding/ThinClaw-main/src/channels/web/server.rs#L228) exposes the real control-plane API: chat, memory, jobs, logs, extensions, gateway management, pairing, routines, skills, provider vault, settings, costs, and the OpenAI-compatible endpoints.
- The browser UI in [`src/channels/web/static/index.html`](/Users/vespian/coding/ThinClaw-main/src/channels/web/static/index.html#L41) is organized around Chat, Memory, Jobs, Logs, Routines, Extensions, Skills, Providers, Costs, and Settings.

## Current Doc Accuracy Assessment

- [`README.md`](/Users/vespian/coding/ThinClaw-main/README.md#L109) is a good front door, but it is trying to be product pitch, setup guide, architecture summary, and reference all at once.
- [`docs/DEPLOYMENT.md`](/Users/vespian/coding/ThinClaw-main/docs/DEPLOYMENT.md#L945) is the strongest operator reference and is close to code, especially for gateway behavior and the OpenAI-compatible API.
- [`FEATURE_PARITY.md`](/Users/vespian/coding/ThinClaw-main/FEATURE_PARITY.md#L38) is useful for engineering coordination, but it is too dense and status-oriented to serve as operator documentation.
- [`docs/BUILDING_CHANNELS.md`](/Users/vespian/coding/ThinClaw-main/docs/BUILDING_CHANNELS.md) is developer-oriented and should stay out of the operator-docs path.

## Contradictions and Drift

- `thinclaw message send` posts to `/api/chat` in [`src/cli/message.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/message.rs#L51), but the gateway route table and the deployment guide document `/api/chat/send` in [`src/channels/web/server.rs`](/Users/vespian/coding/ThinClaw-main/src/channels/web/server.rs#L230) and [`docs/DEPLOYMENT.md`](/Users/vespian/coding/ThinClaw-main/docs/DEPLOYMENT.md#L952). This should be reconciled before any operator docs are finalized.
- The `channels` command treats `gateway` as a channel in [`src/cli/channels.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/channels.rs#L40), but the runtime UI and server treat it as the control plane. Docs should describe it as the operator surface, not just another channel.
- The README and deployment docs mix local defaults (`127.0.0.1:3000`) with remote/Scrappy deployment settings (`0.0.0.0:18789`) without clearly separating standalone defaults from remote-host mode.
- The README’s big feature matrix and parity-style narrative are informative, but they are too noisy to be the canonical operator guide.

## Canonical Reference Topics

- `thinclaw gateway start/stop/status`
- `thinclaw tool install/list/remove/info/auth`
- `thinclaw mcp add/list/auth/test/toggle`
- `thinclaw models list/info/test/verify`
- `thinclaw logs tail/search/show/levels`
- `thinclaw sessions list/show/prune/export`
- `thinclaw agents list/add/remove/show/set-default`
- `thinclaw browser open/screenshot/links/check`
- Web gateway tabs and capabilities: chat, memory, jobs, logs, routines, extensions, skills, providers, costs, settings
- Web gateway API groups: chat, memory, jobs, logs, extensions, gateway, pairing, routines, skills, providers, settings, costs, health, and OpenAI-compatible endpoints

## Rewrite Recommendations

- Keep the README concise: what ThinClaw is, how to start it, and where the operator reference lives.
- Make `docs/DEPLOYMENT.md` the canonical deployment and gateway reference.
- Use the CLI and gateway code as the authoritative command and route source.
- Move route tables, command catalogs, and parity/status material out of the README and into reference docs.
- Fix the `message` route mismatch first, then update the docs to match the actual wire path.
- Treat `FEATURE_PARITY.md` as internal status, not user-facing operator documentation.

## Evidence Pointers

- [`src/cli/mod.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/mod.rs#L100)
- [`src/cli/gateway.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/gateway.rs#L11)
- [`src/cli/tool.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/tool.rs#L27)
- [`src/cli/mcp.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/mcp.rs#L21)
- [`src/cli/message.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/message.rs#L10)
- [`src/cli/channels.rs`](/Users/vespian/coding/ThinClaw-main/src/cli/channels.rs#L40)
- [`src/channels/web/server.rs`](/Users/vespian/coding/ThinClaw-main/src/channels/web/server.rs#L228)
- [`src/channels/web/static/index.html`](/Users/vespian/coding/ThinClaw-main/src/channels/web/static/index.html#L41)
- [`docs/DEPLOYMENT.md`](/Users/vespian/coding/ThinClaw-main/docs/DEPLOYMENT.md#L945)
- [`README.md`](/Users/vespian/coding/ThinClaw-main/README.md#L109)
- [`FEATURE_PARITY.md`](/Users/vespian/coding/ThinClaw-main/FEATURE_PARITY.md#L38)
