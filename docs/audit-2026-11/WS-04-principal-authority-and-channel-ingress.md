# WS-04 — Principal Authority & Channel Ingress

| | |
|---|---|
| **Priority** | P0 (Critical C2) |
| **Effort** | L |
| **Status** | ⏳ Not started |
| **Depends on** | WS-01 · decisions **D1**, **D2** |
| **Findings** | **C2** (SEC-1, SEC-2, AGT-3), SEC-3, SEC-4, SEC-6, SEC-7, SEC-8, SEC-9, AGT-5, GW-5 |
| **Canonical docs** | `docs/CHANNEL_ARCHITECTURE.md`, `docs/TELEGRAM_SETUP.md`, `docs/GMAIL_SETUP.md`, `docs/SURFACES_AND_COMMANDS.md`, `src/NETWORK_SECURITY.md`, `docs/SECURITY.md`, `channels-docs/*`, `src/setup/README.md` (wizard copy), `FEATURE_PARITY.md` |

## Goal

Only the owner (or principals the owner explicitly elevates) can make the agent run non-trivial tools or approve them. Every channel applies a deny-by-default sender policy enforced by the host, not just by each guest.

## Current sender policy (audit snapshot)

| Channel | Ingress verification | Unknown-sender default |
|---|---|---|
| Telegram (WASM) | secret token header | DM: pairing · **groups: open** when no `owner_id` |
| Slack (WASM) | v0 HMAC + 5-min window | DM: pairing · **channel mentions: open** |
| Discord (WASM) | Ed25519 (no freshness) | DM: pairing · **guilds: open** · display-name bypass |
| WhatsApp (WASM) | `X-Hub-Signature-256` | pairing · **display-name bypass** |
| Twilio SMS / LINE / Twitch | platform HMAC | **open — no option exists** |
| Teams, Google Chat, Feishu, DingTalk, Mattermost, Matrix, QQ, WeCom, Weixin | static `X-Webhook-Secret` | **open — no option exists** |
| Native Discord | gateway WS | **open** (guild/channel filter only) |
| iMessage / Apple Mail / Gmail / BlueBubbles | local DB / Pub/Sub / password | **open**; email `From` spoofable |
| Signal | local signal-cli | pairing; groups denied ✓ |
| Nostr | NIP-04 / gift-wrap | owner pubkey only ✓ |
| HTTP | body secret | single configured user ✓ |

## Tasks

- [ ] **4.1 — Owner/operator authority tier (C2/AGT-3, D1).**
  Add an authority level to `ResolvedIdentity` (owner / elevated / external). Non-owner principals default to `ToolProfile::Restricted` at `src/agent/dispatcher/loop.rs:~673-675` and the chat-lane preflight (`src/agent/dispatcher/tool_execution.rs:~226-243`). Approval of any non-`Never` tool requested by a non-owner must come from the owner: extend `same_requesting_actor` / `accepts_identity` (`crates/thinclaw-agent/src/session.rs:~386-401`, `crates/thinclaw-agent/src/thread_ops.rs:~321-348`) with an owner-approval path, and route the approval prompt to the owner's surface. Keep the existing requester binding for owner-initiated calls.

- [ ] **4.2 — Host-side sender gate for every WASM channel (C2/SEC-2).**
  Enforce `allow_from` / pairing / owner on every guest `emit_message` in the host (`crates/thinclaw-channels/src/wasm/wrapper/store.rs:~887`, `wrapper/callbacks.rs:~1290-1350`), independent of guest logic. Add the config surface for the shared webhook channels (`channels-src/shared_webhook_channel/src/impl.rs:333-389` emits unconditionally today) and owner settings for Slack/Discord/WhatsApp in host settings (today only in capabilities JSON).

- [ ] **4.3 — Telegram groups (C2/SEC-1, D2).**
  `channels-src/telegram/src/media.rs:~60-75, ~149-180`: when no `owner_id`, apply the sender policy to group senders, or default `groups_enabled` to false (`telegram.capabilities.json:~62-67`, `crates/thinclaw-settings/src/channels.rs:~395-397`). Fix the wizard copy (`src/setup/channels/telegram.rs:~500`) and add an `owner_id` field to the desktop Channel Center (`apps/desktop/backend/src/thinclaw/commands/channel_settings.rs`). Note `TELEGRAM_ALLOW_FROM` is parsed (`crates/thinclaw-config/src/channel_config.rs:~508`) but never reaches the WASM host config — wire it.

- [ ] **4.4 — Slack channel mentions & Discord guilds (C2/SEC-1).**
  `channels-src/slack/src/lib.rs:~580` (`if !is_dm { return true; }`) and `channels-src/discord/src/lib.rs:~552` — apply the sender policy outside DMs (Slack Connect externals and guests included).

- [ ] **4.5 — Match only stable IDs (SEC-4).**
  Remove display-name allowlist matches: `channels-src/whatsapp/src/handlers.rs:~1052` (`contacts[].profile.name`) and `channels-src/discord/src/lib.rs:~575` (`global_name`).

- [ ] **4.6 — Native channels deny-by-default (SEC-3).**
  Empty `allow_from` must mean "nobody" (or pairing), not "everyone": Gmail (`crates/thinclaw-channels/src/gmail_wiring.rs:~233`), native Discord (`crates/thinclaw-channels/src/discord.rs:~1014-1024` filters only guild/channel, never the user; DMs accepted when `guild_id` unset; schema text at `:~637` says "users" but holds channel IDs), iMessage (`crates/thinclaw-channels/src/imessage.rs`), BlueBubbles (`crates/thinclaw-channels/src/bluebubbles*`), Apple Mail (`crates/thinclaw-channels/src/apple_mail.rs:~681`). Wizard prompts "blank = all" (`src/setup/wizard/channels_step.rs:~913, ~1104`). For email: honor `Authentication-Results` (DKIM/SPF/DMARC pass for the allowed domain) before matching `From`, or document clearly that email allowlists are spoofable.

- [ ] **4.7 — Make the blocklist effective (SEC-6).**
  WASM guests (Telegram, Slack, Discord, WhatsApp) call `pairing_read_allow_from` (`crates/thinclaw-channels/src/wasm/wrapper/store.rs:~984-990`), which ignores the block list. Filter blocked entries in the host function (or switch guests to `pairing_is_allowed`), so the CLI's "blocklist takes precedence" claim (`src/cli/pairing.rs:~138`) holds. Host-side gate in 4.2 should also consult it.

- [ ] **4.8 — Enforce command authorization (AGT-5).**
  `minimum_authorization` on slash commands (`crates/thinclaw-types/src/slash_commands.rs:~158+`) is declared but never read. Enforce it in the parser/dispatcher. `/quit`, `/restart`, and the shutdown path (`src/agent/agent_loop/runtime.rs:~149-166`, `message_handling.rs:~209-231`, `agent_loop/mod.rs:~1904-1907`) must only act from local or owner-authenticated surfaces; on messaging surfaces `/quit` should end the client session, not the process. Align `docs/SURFACES_AND_COMMANDS.md`.

- [ ] **4.9 — Pairing queue can't be jammed (SEC-9).**
  `PAIRING_PENDING_MAX = 3` (`crates/thinclaw-channels/src/pairing.rs:~16`; upsert drops new requests when full at `:~519-524`). Evict oldest / rate-limit per sender, and surface pending requests to the owner.

- [ ] **4.10 — Gateway `Operator` role containment (GW-5).**
  Operators get `Chat` on `/api/chat`, `/api/jobs`, `/api/subagents` (`crates/thinclaw-gateway/src/web/rbac.rs:~54-60`) and self-approve; only memory/learning/external-memory tools read `principal_admin` (`crates/thinclaw-gateway/src/web/submission.rs:~26-27`). Map gateway roles onto the 4.1 authority tier so `principal_admin = false` gets the Restricted profile and owner-routed approvals.

- [ ] **4.11 — Docs that overclaim (SEC-7).**
  `docs/TELEGRAM_SETUP.md` (groups bypass pairing; the wizard does not bind owner from a DM), `docs/GMAIL_SETUP.md:~235` (spoofing caveat), `docs/CHANNEL_ARCHITECTURE.md` ("production" status = webhook auth only, not sender auth — add a sender-policy column), `src/NETWORK_SECURITY.md:~18`.

- [ ] **4.12 — Discord Ed25519 freshness (SEC-8).**
  `crates/thinclaw-channels/src/wasm/router.rs:~537-578`: enforce ±5 min on `X-Signature-Timestamp`; consider persisting the replay cache (`:~107-142`) across restarts for LINE/WhatsApp/Twilio, whose protocols carry no timestamp.

- [ ] **4.13 — Regression tests.**
  Per channel: unknown sender in a group/channel/guild is dropped by the host gate; display-name equal to an allowed ID is rejected; blocked sender is dropped; non-owner request for `shell` cannot be approved by the non-owner; `/quit` from a channel does not stop the process.

## Verification (workstream exit)

```bash
cargo test --locked --workspace
./scripts/build-all.sh   # rebuild WASM channels touched by 4.3–4.5
```

Manual: with a test Telegram bot and no `owner_id`, add the bot to a group from a second account → no agent turn is produced (or pairing is requested, per D2).
