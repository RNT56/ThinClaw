# ThinClaw Desktop — Feature Status & Verified Gaps

> **Snapshot:** 2026-06-28 · point-in-time, code-grounded. This is intentionally
> *thinly scoped* (per the repo doc rules): it does not re-inventory every feature.
> For the roadmap and parity ledger use the canonical docs below; this file only
> records **verified open gaps** that are easy to lose track of.

## Orientation

ThinClaw Desktop is a Tauri v2 app that intentionally contains two AI systems plus
shared infrastructure:

- **Direct AI Workbench** — non-autonomous local/cloud chat, RAG, voice, and image
  generation (`backend/src/chat.rs`, `rig_lib/`, `inference/`, `engine/`).
- **ThinClaw Agent Cockpit** — embeds the autonomous ThinClaw runtime in-process, or
  proxies to a remote gateway (dual-mode: embedded `inner` vs `RemoteGatewayProxy`).
- **Shared infra** — secrets, sidecars, settings, onboarding, cloud sync.

The app is **experimental / pre-1.0**. Read [`runtime-boundaries.md`](runtime-boundaries.md)
before changing either system.

### Canonical status docs (authoritative)

| Topic | Doc |
|---|---|
| Overhaul roadmap & phases | [`OVERHAUL_PLAN.md`](OVERHAUL_PLAN.md) |
| Executable backlog (TDO-###) | [`OVERHAUL_BACKLOG.md`](OVERHAUL_BACKLOG.md) |
| Runtime parity tiers | [`runtime-parity-checklist.md`](runtime-parity-checklist.md) |
| Two-system boundaries | [`runtime-boundaries.md`](runtime-boundaries.md) |
| Local/remote command behavior | [`remote-gateway-route-matrix.md`](remote-gateway-route-matrix.md) |
| Cross-surface parity ledger | [`../../../FEATURE_PARITY.md`](../../../FEATURE_PARITY.md) |

## Verified open gaps

Each row was confirmed against code at the date above. "UI-dishonest" means the UI
implies an effect the backend does not deliver.

| Area | Gap | Evidence | Class |
|---|---|---|---|
| Tool Policies | `disabled_tools` deny-list is persisted but **no runtime code reads it** — toggling a tool off does not stop the agent. | desktop writes `local_user/disabled_tools` (`rpc_extensions.rs`); no reader in `src/`/`crates/` | UI-dishonest / needs runtime wiring |
| Channels | Per-channel stream-mode toggle writes the wrong key/vocabulary. UI writes `<ID>_STREAM_MODE` with `full`/`typing_only`/`disabled`; runtime reads `channels.<name>_stream_mode` with `edit`/`status`/`chunks` (telegram+discord only). | `ThinClawChannelStatus.tsx`, `channels/wasm/runtime_config.rs` | UI-dishonest / multi-layer |
| Channels | Gmail **label filter** is shown but not savable: backend reads it only from `GMAIL_LABEL_FILTERS` env var, with **no DB-setting fallback** (unlike allowed-senders). | `rpc_dashboard/channels.rs:325` (env only); allowed-senders DB fallback at `:369-382` | Needs backend read + UI save |
| Skills | Per-skill enable/disable toggle is a deliberate no-op (the SkillRegistry has no enable/disable). | `rpc_skills.rs` `thinclaw_skills_toggle` | UI-dishonest |
| Channels | WhatsApp QR-login modal is unreachable — the runtime always emits `qr_code: None`. | `event_mapping.rs`, `ThinClawChannels.tsx` | Dead UI |
| Dashboard | "Active Instances" / "Connected Nodes" cards are always 0 (backend presence returns no such fields); version label is a hardcoded string. | `ThinClawDashboard.tsx`, `rpc_config.rs` system-presence | Dead UI |
| Sidecars | `image_running` / `tts_running` report path-presence, not a live process. | `sidecar/core.rs` `is_image_active`/`is_tts_active` | Misleading status |
| Voice | Cloud TTS playback decodes responses as raw Int16 PCM @22050, but OpenAI/ElevenLabs return MP3 — likely garbled. | `MessageBubble.tsx` Read-Aloud handler | Likely bug (needs app run) |
| Cloud sync | App Nap suppression is a no-op on macOS (atomic counter; never calls `NSProcessInfo.beginActivity`). | `cloud/app_nap.rs` | Platform no-op |
| Models | Dead remote-catalog backend commands (`update_/get_remote_model_catalog`) — their only client (a `localhost:8000` fetch) was removed; the commands remain registered. | `model_manager.rs` | Dead backend (registered) |
| Web search | Probe commands `check_web_search` / `rig_check_web_search` are registered but have no live caller. | `web_search.rs`, `rig_lib/mod.rs`, `setup/commands.rs` | Dead backend (registered) |

## Notes

- Items above marked "dead backend (registered)" require regenerating `bindings.ts`
  (specta) and updating the bindings contract test, so they are Rust-gated, not
  frontend-only cleanups.
- Several items (WhatsApp QR, dashboard cards, per-skill toggle) are product calls —
  wire real data vs. remove the control — rather than mechanical fixes.
