# Design — Channel-Config Framework (TDO-120…123)

> **Status:** implemented 2026-07-13 · **Created:** 2026-06-27 · Epic: TDO-EP7 (Phase 1 parity)
> **Parent:** [`../OVERHAUL_PLAN.md`](../OVERHAUL_PLAN.md) §5c · **Backlog:** [`../OVERHAUL_BACKLOG.md`](../OVERHAUL_BACKLOG.md)
> Largest single parity item (XL). Depends on TDO-001 (RouteBehavior).

## 1. Problem (grounded in code)

ThinClaw core supports **~30 channels** (native + WASM; see `FEATURE_PARITY.md §3`), but the
desktop only offers **configuration UI for Slack, Telegram, Gmail, Apple Mail, and the
Tauri-local channel**. `ThinClawChannels.tsx` (719 LoC) configures them with:

- `thinclaw.getThinClawChannelsList()` → `ChannelInfo[]` (status/enabled),
- generic `thinclaw.setSetting(key, value)` for env-style settings,
- per-channel special cases (Gmail: `startGmailOAuth()` + `getGmailStatus()` polling).

The other ~25 channels (Signal, Discord, iMessage, BlueBubbles, Nostr, Matrix, Teams, LINE,
SMS, WeCom, Feishu, Twitch, voice-call, APNs, …) can only be configured by editing
config/env files. We need a **schema-driven framework** so a channel becomes configurable
by *declaring a schema*, not by writing bespoke UI.

**Key enabler already in the codebase:** WASM channels ship a config schema today. From
`channels-src/dingtalk/dingtalk.capabilities.json`:

```json
"setup": { "required_secrets": [
  { "name": "dingtalk_access_token", "prompt": "Enter your DingTalk robot access token", "optional": false },
  { "name": "dingtalk_webhook_secret", "prompt": "Enter your DingTalk webhook secret", "optional": false }
]},
"capabilities": { "secrets": { "allowed_names": ["dingtalk_*"] }, "channel": { "allowed_paths": ["/webhook/dingtalk"], ... } }
```

So the framework can *read* WASM schemas directly; only **native channels** need a new
declaration hook.

## 2. Goals / non-goals

**Goals**
- A unified `ChannelConfigSchema` that both native and WASM channels produce.
- A command pair (mirroring the proven extension setup flow) + a generic React renderer.
- Onboard new channels by declaring a schema only — zero bespoke UI.
- Correct secret routing (secret store + grants, per `secrets-policy.md`) vs plain settings.
- Generalize Gmail OAuth into a reusable `oauth` field type.

**Non-goals**
- Implementing the channels themselves (they exist in core).
- Changing channel *runtime* behavior or the gateway `channel_setup` API.

## 3. Unified schema model

A superset of WASM `setup.required_secrets` + native env-var config + OAuth + toggles.

```rust
// crates/thinclaw-channels-core/src/config_schema.rs  (new; shared so core can declare it)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
pub struct ChannelConfigSchema {
    pub channel: String,
    pub display_name: String,
    pub transport: ChannelTransport,            // Native | Wasm | NativeLifecycle
    pub docs_url: Option<String>,
    pub fields: Vec<ChannelField>,
    pub status: ChannelConfigStatus,            // configured? paired? healthy?
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelField {
    Text   { key: String, label: String, prompt: String, optional: bool, default: Option<String> },
    Secret { key: String, label: String, prompt: String, optional: bool }, // routed to secret store
    Select { key: String, label: String, options: Vec<(String,String)>, default: Option<String> },
    Bool   { key: String, label: String, default: bool },
    Oauth  { provider: String, label: String, start_command: String },     // generalizes Gmail
    Info   { markdown: String },                                            // setup instructions
}
```

`ChannelField::Secret.key` follows the ThinClaw secret naming in `secrets-policy.md`; the
WASM `capabilities.secrets.allowed_names` glob constrains which secret names are accepted.

## 4. Schema sources

| Transport | Source of schema |
|---|---|
| **WASM** channels | Read `*.capabilities.json` `setup.required_secrets` → `Secret` fields; `capabilities.channel` → webhook/toggle info. Already exists; just map it. |
| **Native** channels | New trait method on the `Channel` trait, mirroring the existing `Channel::formatting_hints()` override pattern (the codebase already uses per-channel trait overrides for hints): |

```rust
// thinclaw-channels-core: extend the Channel trait
trait Channel {
    fn formatting_hints(&self) -> Option<FormattingHints> { None } // existing
    fn config_schema(&self) -> Option<ChannelConfigSchema> { None } // NEW — each native channel declares its fields
}
```

Each native channel (Signal, Discord, iMessage, Nostr, …) implements `config_schema()`
returning its env-keyed fields. Example (Signal, env keys from `FEATURE_PARITY §3` / channel catalog):

```rust
fn config_schema(&self) -> Option<ChannelConfigSchema> {
    Some(ChannelConfigSchema { channel: "signal".into(), display_name: "Signal".into(),
        transport: ChannelTransport::Native, docs_url: None,
        fields: vec![
            ChannelField::Text   { key: "SIGNAL_HTTP_URL".into(), label: "signal-cli URL".into(), prompt: "http://localhost:8080".into(), optional: false, default: None },
            ChannelField::Text   { key: "SIGNAL_ACCOUNT".into(), label: "Account number".into(), prompt: "+15551234567".into(), optional: false, default: None },
            ChannelField::Text   { key: "SIGNAL_ALLOWED_SENDERS".into(), label: "Allowed senders".into(), prompt: "comma-separated".into(), optional: true, default: None },
        ], status: Default::default() })
}
```

## 5. Commands (mirror the extension setup precedent)

Model exactly on `thinclaw_extension_setup_get` / `thinclaw_extension_setup_submit`
(`rpc_extensions.rs:585/634`), including the dual-mode proxy-first pattern:

```rust
// thinclaw/commands/rpc_dashboard/channels.rs  (extend existing module)

#[tauri::command] #[specta::specta]
pub async fn thinclaw_channel_config_schema(
    ironclaw: State<'_, ThinClawRuntimeState>, channel: String,
) -> Result<ChannelConfigSchema, BridgeError> {       // BridgeError from TDO-001
    if let Some(proxy) = ironclaw.remote_proxy().await {
        return proxy.get_json(&format!("/api/channels/{}/config", enc(&channel))).await;
    }
    let agent = ironclaw.agent().await?;
    agent.channel_manager().config_schema_for(&channel)  // resolves native trait OR WASM manifest
         .ok_or_else(|| gated("channel config", "unknown channel", "install the channel package", RouteMode::LocalAndRemote))
}

#[tauri::command] #[specta::specta]
pub async fn thinclaw_channel_config_submit(
    ironclaw: State<'_, ThinClawRuntimeState>, channel: String,
    secrets: HashMap<String,String>, settings: HashMap<String,String>,
) -> Result<ChannelActionResponse, BridgeError> { /* route secrets→store(+grant), settings→config; proxy in remote mode */ }

#[tauri::command] #[specta::specta]
pub async fn thinclaw_channel_oauth_start(    // generalizes thinclaw_gmail_oauth_start
    ironclaw: State<'_, ThinClawRuntimeState>, channel: String, provider: String,
) -> Result<OAuthStartResponse, BridgeError> { /* … */ }
```

Reuse the existing `thinclaw_channel_status_list` / `thinclaw_channels_list` for status, and
`thinclaw_pairing_list/approve` for the pairing step (TDO-123). Register all in
`setup/commands.rs`; they inherit the TDO-001 linter + route-matrix.

**Secret routing:** `Secret` fields → secret store via the unified secrets service (TDO-040),
auto-granting the channel; never written to plain settings. `Text/Select/Bool` → settings/env
via the existing `setSetting` path. This keeps the `secrets-policy.md` boundary intact.

## 6. Generic React renderer

One component replaces the per-channel special cases in `ThinClawChannels.tsx`:

```tsx
// frontend/src/components/thinclaw/channels/ChannelConfigForm.tsx (new)
function ChannelConfigForm({ channel }: { channel: string }) {
  const { schema } = useChannelSchema(channel);          // commands.thinclawChannelConfigSchema
  // render schema.fields by type:
  //   Text/Secret → input (Secret = password + "grant" toggle)
  //   Select → dropdown, Bool → switch, Info → markdown
  //   Oauth → button → commands.thinclawChannelOauthStart(channel, provider) + status poll
  // submit → commands.thinclawChannelConfigSubmit(channel, secrets, settings)
  // status badge from thinclaw_channel_status_list; pairing CTA when transport needs it
}
```

`ThinClawChannels.tsx` becomes a list + `<ChannelConfigForm>` host (and shrinks well under the
god-file threshold — also satisfies the WS-3 split for this component).

## 7. Rollout (TDO-120 → 121 → 122 → 123)

1. **TDO-120 framework:** schema type, `Channel::config_schema()` hook, WASM manifest mapper, the 3 commands, the renderer, linter coverage. Land with Slack/Telegram **re-expressed as schemas** (proves parity with today's bespoke UI; delete the special cases).
2. **TDO-121 first natives:** Signal, Discord, iMessage, Nostr declare `config_schema()`.
3. **TDO-122 long tail:** Matrix, Teams, LINE, SMS, BlueBubbles, Apple Mail, WeCom, Feishu, Twitch, voice-call, APNs, browser-push — each is *schema-declaration-only*.
4. **TDO-123 pairing parity:** wire the generic `pairing` status/approve step for DM-paired channels (reuse `ThinClawPairing.tsx`).

## 8. Test plan & risks

**Tests:** schema round-trips (Rust→TS→Rust); secret fields land in the store with a grant and
never in settings; one fixture per channel asserts `config_schema()` returns the expected
field set; remote-mode proxy path returns the gateway schema; `ChannelConfigForm` renders each
field type (Vitest).

**Risks:**
- *Native vs WASM divergence* — mitigate by making both produce the **same** `ChannelConfigSchema` (one renderer, no per-transport branches in UI).
- *Secret-name collisions / policy drift* — enforce the WASM `allowed_names` glob and `secrets-policy.md` naming in `config_submit`; reject out-of-namespace secrets.
- *OAuth variety* — start with the Gmail-shaped flow generalized; channels needing different OAuth declare a distinct `provider` handled server-side.
- *Doc rule* — update `CHANNEL_ARCHITECTURE.md` + `runtime-parity-checklist.md` (Channels row) in the same PR per repo policy.

**Definition of done:** every supported channel is configurable from the desktop via a
declared schema (or shows an honest "configure via host" state); Slack/Telegram special-case
code deleted; `ThinClawChannels.tsx` under the god-file threshold.
