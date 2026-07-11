# ThinClaw Desktop — Feature Status & Verified Gaps

> **Snapshot:** re-verified 2026-07-10 (originally 2026-06-28) · point-in-time,
> code-grounded. This is intentionally *thinly scoped* (per the repo doc rules): it does
> not re-inventory every feature. For the roadmap and parity ledger use the canonical docs
> below; this file only records **verified open gaps** that are easy to lose track of.
> Several earlier gaps (tool-policy deny-list, per-channel stream-mode, Gmail label filter,
> the sidecar/dashboard status rows, the web-search probes) have since been closed and were
> removed from the table.

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
| Skills | Per-skill enable/disable toggle is a deliberate no-op (the SkillRegistry has no enable/disable). | `rpc_skills.rs` `thinclaw_skills_toggle` (comment: "SkillRegistry doesn't support enable/disable") | UI-dishonest |
| Channels | WhatsApp QR-login modal is unreachable — the runtime always emits `qr_code: None`. | `event_mapping.rs` (`qr_code: None`), `ThinClawChannels.tsx` | Dead UI |
| Voice | Cloud TTS playback decodes responses as raw Int16 PCM @22050, but OpenAI/ElevenLabs return MP3 — likely garbled. | `MessageBubble.tsx` Read-Aloud handler | Likely bug (needs app run) |
| Cloud sync | App Nap suppression is a no-op on macOS (atomic counter; never calls `NSProcessInfo.beginActivity`). | `cloud/app_nap.rs` (macOS `begin()` only increments `APP_NAP_GUARD_COUNT`) | Platform no-op |
| Models | Dead remote-catalog backend commands (`update_/get_remote_model_catalog`) — their only client (a `localhost:8000` fetch) was removed; the commands remain registered. | `model_manager.rs:886,918`; still registered in `setup/commands.rs` | Dead backend (registered) |

## Notes

- Items above marked "dead backend (registered)" require regenerating `bindings.ts`
  (specta) and updating the bindings contract test, so they are Rust-gated, not
  frontend-only cleanups.
- Several items (WhatsApp QR, per-skill toggle) are product calls — wire real data vs.
  remove the control — rather than mechanical fixes.
