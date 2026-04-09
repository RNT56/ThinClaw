# Final Consistency Report

## Executive Summary

ThinClaw's core docs are strong, but they are not yet internally consistent. The biggest problems are onboarding step counts, channel taxonomy, gateway defaults, command/route naming, and trust-boundary language. Code should win on all of those points, then the docs should be tightened around one canonical home per concept.

The safest path is to stop repeating the same workflows in multiple places. `README.md` should be the front door, `src/setup/wizard/mod.rs` should define onboarding truth, `docs/CHANNEL_ARCHITECTURE.md` should own channel architecture, and `docs/EXTENSION_SYSTEM.md` should own the extension trust model.

## Highest-Risk Contradictions

1. Onboarding steps are inconsistent across the repo. `src/setup/wizard/mod.rs` says the wizard has 20 steps, `src/setup/README.md` says 18, `Agent_flow.md` says 9, `CLAUDE.md` says 18, and `docs/DEPLOYMENT.md` still presents a 9-step flow. This is the highest-risk user-facing drift because the setup path is the first real ThinClaw experience.
2. The operator command and route surfaces are not aligned. `src/cli/message.rs` posts to `/api/chat`, while `src/channels/web/server.rs` and `docs/DEPLOYMENT.md` still document `/api/chat/send`. That is a concrete wire-level mismatch, not just wording drift.
3. The extension model is still described too loosely. `src/tools/README.md` and `docs/EXTENSION_SYSTEM.md` blur `thinclaw tool ...`, `thinclaw mcp ...`, registry installs, and conversational agent tools, while the completed tools audit shows those are distinct flows with different trust boundaries and command names.
4. The channel model is split between old and new assumptions. `src/channels/mod.rs` and `docs/CHANNEL_ARCHITECTURE.md` define a hybrid native/WASM model, but `CLAUDE.md` still labels Telegram and Slack as native, while `channels-docs/README.md`, `channels-docs/telegram.md`, and `channels-docs/slack.md` use older transport framing.
5. Gateway defaults are inconsistent. `src/config/channels.rs` defaults `GATEWAY_PORT` to `3000`, `README.md` mixes `3000` and `18789`, and `docs/DEPLOYMENT.md` hard-codes `18789` in its setup flow. That needs one canonical default plus explicit override guidance.
6. Build and dependency defaults are misrepresented. `Cargo.toml` defaults to `light`, which excludes tunnel and web-gateway, but `docs/EXTERNAL_DEPENDENCIES.md` says tunnel is enabled by default and auto-started. That is simply wrong for the default build.
7. Trust boundaries are overstated in the public pitch. `README.md` says "nothing leaves your control", but `docs/EXTENSION_SYSTEM.md` and `src/tools/README.md` show that MCP servers are operator-trusted external processes, not sandboxed extensions. The docs need to say that plainly.

## Canonical Boundary Problems

- `README.md` is trying to do too much. It currently mixes product pitch, setup, deployment, security, architecture, feature matrix, and channel catalog in one place.
- `Agent_flow.md` and `src/setup/README.md` both describe onboarding, but they should not both restate the full wizard flow. One should own runtime flow; the other should own setup spec.
- `docs/CHANNEL_ARCHITECTURE.md` should be the architecture authority, while `channels-docs/README.md` should be an index into per-channel guides rather than a second architecture doc.
- `docs/EXTENSION_SYSTEM.md` should clearly separate sandboxed WASM tools/channels from unsandboxed MCP servers and separate registry installs from CLI auth flows.
- `docs/DEPLOYMENT.md` should own deployment modes and remote access, but it should not duplicate setup wizard detail beyond what is needed for operators.
- `README.md` should route to deep docs instead of serving as a second operator manual or parity tracker.
- `FEATURE_PARITY.md` should remain an engineering tracker, not a source of public narrative.

## Cross-Doc Cleanup Priorities

- Normalize onboarding truth from the code outward, then update `src/setup/README.md`, `Agent_flow.md`, `CLAUDE.md`, `README.md`, and `docs/DEPLOYMENT.md` to match.
- Normalize gateway port guidance around the code default of `3000`, then make `18789` an explicit alternate or historical deployment value where needed.
- Reconcile the `thinclaw message` wire path with the gateway route docs so operator instructions and server routes match exactly.
- Rewrite the extension docs so `thinclaw tool ...`, `thinclaw mcp ...`, and registry installs are distinct and clearly labeled by trust boundary.
- Rewrite `channels-docs/README.md` into an audience-first index that cleanly separates native channels, WASM packages, and Discord Gateway vs Discord interactions.
- Update `tools-docs/` so the auth vocabulary matches the actual CLI surfaces: `thinclaw tool auth ...` and `thinclaw mcp auth ...`.
- Add one canonical security/trust overview that points to `src/NETWORK_SECURITY.md` instead of restating the whole security model in multiple places.
- Shorten `README.md` so it links out to the deep docs instead of carrying all of them inline.

## Final Consistency Recommendations

1. Make `src/setup/wizard/mod.rs` the source of truth for onboarding, then update every other setup-facing doc to match it exactly.
2. Make `src/cli/message.rs`, `src/cli/tool.rs`, and `src/cli/mcp.rs` the source of truth for command names and wire paths, then align the operator docs to them.
3. Make `docs/CHANNEL_ARCHITECTURE.md` the source of truth for channel boundaries, then align `channels-docs/` and `FEATURE_PARITY.md` to it.
4. Keep `docs/EXTENSION_SYSTEM.md` as the extension-system authority, but explicitly label MCP as operator-trusted and unsandboxed and keep registry installs separate from CLI auth flows.
5. Use `README.md` as a short, high-signal front door and move deeper detail into the canonical docs it links to.
6. Keep `rewrite-docs/` archived and out of the current-reference path.
7. When behavior changes in the code, update the relevant docs and `FEATURE_PARITY.md` in the same branch.

## Evidence Pointers

- [src/setup/wizard/mod.rs](/Users/vespian/coding/ThinClaw-main/src/setup/wizard/mod.rs#L3)
- [src/setup/README.md](/Users/vespian/coding/ThinClaw-main/src/setup/README.md#L51)
- [Agent_flow.md](/Users/vespian/coding/ThinClaw-main/Agent_flow.md#L127)
- [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md#L14)
- [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md#L26)
- [docs/CHANNEL_ARCHITECTURE.md](/Users/vespian/coding/ThinClaw-main/docs/CHANNEL_ARCHITECTURE.md#L3)
- [src/channels/mod.rs](/Users/vespian/coding/ThinClaw-main/src/channels/mod.rs#L7)
- [channels-docs/README.md](/Users/vespian/coding/ThinClaw-main/channels-docs/README.md#L24)
- [src/config/channels.rs](/Users/vespian/coding/ThinClaw-main/src/config/channels.rs#L146)
- [README.md](/Users/vespian/coding/ThinClaw-main/README.md#L38)
- [README.md](/Users/vespian/coding/ThinClaw-main/README.md#L60)
- [docs/DEPLOYMENT.md](/Users/vespian/coding/ThinClaw-main/docs/DEPLOYMENT.md#L170)
- [docs/DEPLOYMENT.md](/Users/vespian/coding/ThinClaw-main/docs/DEPLOYMENT.md#L184)
- [Cargo.toml](/Users/vespian/coding/ThinClaw-main/Cargo.toml#L215)
- [docs/EXTERNAL_DEPENDENCIES.md](/Users/vespian/coding/ThinClaw-main/docs/EXTERNAL_DEPENDENCIES.md#L66)
- [docs/EXTENSION_SYSTEM.md](/Users/vespian/coding/ThinClaw-main/docs/EXTENSION_SYSTEM.md#L132)
- [src/tools/README.md](/Users/vespian/coding/ThinClaw-main/src/tools/README.md#L105)
