# WS-03 — WASM Channels & Tools Repair + Shared SDK

> **Status:** ◑ Mostly landed (2026-06-23), commit `d1c447c8` (Wave 1B: WASM channel fixes). The three real defects shipped: **T1** (char-aware `split_message` in telegram/slack/discord + unicode tests), **T2** (Discord Ed25519 verification end-to-end), and **T5** (all 12 shim READMEs + `production_status`). **Still open:** the shared-helper extraction (**T3** `tools-src`, **T4** `channels-src`) was not done, so the duplicated helpers still live in each crate, and **T6**'s CI-matrix wiring is WS-13-owned. Do not re-run the completed tasks; the remaining work is the SDK-dedup and CI matrix only.
> **Priority:** P1 · **Risk:** medium · **Effort:** L
> **Depends on:** none · **Blocks:** none (coordinates with WS-12 doc-sync, WS-13 build/CI)
> **Owns (symbols/files):**
> - `channels-src/telegram/src/lib.rs`, `channels-src/slack/src/lib.rs`, `channels-src/discord/src/lib.rs`, `channels-src/whatsapp/src/lib.rs` (the four custom-WASM channel crates)
> - `channels-src/shared_webhook_channel/src/impl.rs` and all 12 shim crates that `include!` it (`dingtalk`, `feishu_lark`, `google_chat`, `line`, `matrix`, `mattermost`, `ms_teams`, `qq`, `twilio_sms`, `twitch`, `wecom`, `weixin`)
> - `channels-src/discord/discord.capabilities.json`, `channels-src/discord/README.md` (README edit coordinated with WS-12)
> - `tools-src/github/src/lib.rs`, `tools-src/notion/src/lib.rs` (`url_encode_path`, `validate_input_length` duplication)
> - The new `WebhookSecretValidation::DiscordEd25519` variant and `verify_discord_ed25519_signature` helper in `crates/thinclaw-channels/src/wasm/{schema.rs,router.rs}`
> - Any new shared SDK crate(s) created under `channels-src/` / `tools-src/`
>
> **Note on shared files:** `crates/thinclaw-channels/src/wasm/router.rs` and `schema.rs` are host-side. This WS adds the Ed25519 variant + helper there; no other current workstream edits the WASM webhook validation path, but flag any conflict with a host-channels WS before merging.

## Vision & Goal

The externally-packaged WASM channels are ThinClaw's reach into every messaging platform an operator might run, and they are the project's frontier — the audit rated them 78% complete vs. 85–90% for the native core. This workstream closes the three real defects (a reachable multibyte-UTF-8 panic, a falsely-advertised Discord signature check, and undocumented-but-functional shim channels) and removes the structural cause of "the fix landed in only 1 of N copies" by extracting shared SDK code. Realizing this turns a mixed-confidence surface into a uniformly safe, signature-verified, honestly-documented channel fleet.

## Scope

**In scope:**
- Port WhatsApp's char-aware `split_message` (`byte_index_for_char_limit` + the unicode-safe test) into telegram, slack, discord.
- Implement real Discord interaction verification: a host-side `WebhookSecretValidation::DiscordEd25519` variant keyed on `discord_public_key`, a `channel.webhook` block in `discord.capabilities.json`, consume `req.secret_validated` (and the now-dead `require_signature_verification` flag) in the Discord WASM, and fix the false `README.md:106` claim.
- Extract shared SDK code to dedupe `json_response` / `split_message` / `conversation_scope_id` / `external_conversation_key` (channels) and `url_encode_path` / `validate_input_length` (tools).
- Audit/classify the 12 shim channels (all `include!`-based instances of `shared_webhook_channel`); finish or formally mark their production status in capabilities + a per-channel README.

**Out of scope (and which WS owns it):**
- The native (non-WASM) channel transports and `wasm/wrapper.rs` 5701L god-file refactor — owned by the native-channels / god-file workstream, not WS-03.
- The empty `gateway_auth_token` bypass and `extra_public_routes` security layering — owned by the gateway/security workstream.
- Doc-tree sync beyond the single Discord README correctness fix — `CHANNEL_ARCHITECTURE.md` and broad inventory docs are **WS-12 doc-sync**; this WS only fixes the load-bearing false claim and writes the new per-shim READMEs.
- The CI build matrix expansion to add a `wasm32-wasip2` compile gate for the 12 shims and `tools-src` to `build-all.sh` — **WS-13 build/CI** owns the workflow file; WS-03 supplies the requirement and the buildability fix.

## Current State (verified)

**(1) `split_message` multibyte panic — FIXED in all four channels:**
- `channels-src/whatsapp/src/lib.rs` was the fixed reference (counts `chars()` not `len()`, nested `fn byte_index_for_char_limit`, `test_split_message_is_unicode_safe` over `"🙂".repeat(5000)`).
- The identical char-aware fix has since been ported to the other three: telegram (`channels-src/telegram/src/dispatch.rs:117` `byte_index_for_char_limit`, slice at `:134`), slack (`channels-src/slack/src/lib.rs:1054`, slice at `:1070`, `test_split_message_is_unicode_safe` at `:1205`), and discord (`channels-src/discord/src/lib.rs:777`, slice at `:794`, `test_split_message_is_unicode_safe` at `:856`).
- No `&remaining[..max_len]` byte-slice remains in any `split_message` (`rg 'remaining\[..max_len\]' channels-src/` → 0 hits). Note: the shared-source extraction (T4) did **not** land, so the fix currently lives in four copies rather than one included module.

**(2) Discord Ed25519 verification — FIXED (implemented end-to-end):**
- The `require_signature_verification` flag is now consumed; the Discord WASM rejects requests when `!req.secret_validated` (`channels-src/discord/src/lib.rs:196,220-222`).
- `channels-src/discord/discord.capabilities.json` now carries a `capabilities.channel.webhook` block (`secret_header: X-Signature-Ed25519`, `secret_name: discord_public_key`, `secret_validation: discord_ed25519`).
- The host validation path gained the missing variant: `WebhookSecretValidation::DiscordEd25519` (`crates/thinclaw-channels/src/wasm/schema.rs:556`, serde `discord_ed25519`) and `fn verify_discord_ed25519_signature` (`crates/thinclaw-channels/src/wasm/router.rs:316`), dispatched at `router.rs:700-701`. It reads `X-Signature-Timestamp`, reconstructs `timestamp ++ body`, and verifies the `X-Signature-Ed25519` signature against the hex `discord_public_key` verifying key.
- `channels-src/discord/README.md` now accurately describes the Ed25519 flow instead of claiming validation that did not exist.

**(3) Cross-crate duplication — STILL PRESENT (T3/T4 shared-helper extraction not done):**
- `fn json_response`: discord, telegram, slack, whatsapp, shared_webhook_channel (5 copies).
- `fn split_message`: discord, telegram, slack, whatsapp (4 copies). The unicode-safe fix has now been ported into all four (T1), but they remain four separate copies — the SDK extraction (T4) that would collapse them to one source has not landed.
- `fn conversation_scope_id` / `fn external_conversation_key`: telegram (`channels-src/telegram/src/sessions.rs:13`, `:81`) + whatsapp (2 copies each).
- `fn url_encode_path` / `fn validate_input_length`: `tools-src/github/src/lib.rs` + `tools-src/notion/src/lib.rs` (2 copies each).
- **Structural constraint:** every channel/tool crate is a *standalone Cargo workspace* (each `Cargo.toml` ends with a bare `[workspace]` table) and is listed in the root `Cargo.toml` `exclude` (`Cargo.toml:3-20`). They share one WIT world via `wit_bindgen::generate!({ world: "sandboxed-channel", path: "../../wit/channel.wit" })` (`discord/src/lib.rs:21-24`).

**(4) The "12 thin shims" — wired config-driven instances, NOT scaffolds:**
- Each shim's `src/lib.rs` is **8 lines**: `wit_bindgen::generate!`, `include!("../../shared_webhook_channel/src/impl.rs")`, `export!(GenericWebhookChannel)` (verified `dingtalk/src/lib.rs`; identical shape for all 12 `include!`-based shims; `twitch`/`twilio_sms` also use the shared impl).
- The engine is `channels-src/shared_webhook_channel/src/impl.rs` (731 LoC) — a complete generic webhook channel driven by capabilities `config` (mapping, challenge, response, template values).
- All 10 China/enterprise shims have **complete config**: `mapping.text`, `response.url`, a `channel.webhook` block, and `required_secrets` (verified by parsing each capabilities.json). `feishu_lark`/`wecom`/`weixin` add `challenge` blocks.
- **The real gap is signature correctness, not missing code:** `dingtalk`, `google_chat`, `matrix`, `mattermost`, `ms_teams`, `qq`, `wecom`, `weixin`, `feishu_lark` all declare `secret_validation: "equals"` with a generic `X-Webhook-Secret` header. The actual platforms (DingTalk HMAC-SHA256 over timestamp, Feishu AES/token, QQ Ed25519, WeChat-Work/WeChat msg signature) do **not** send a plaintext shared secret in `X-Webhook-Secret`, so inbound auth is effectively `equals`-against-a-shared-secret only if the operator configures the platform to send one. `line` correctly uses `hmac_sha256_base64_body`; `twitch` uses `twitch_eventsub_hmac_sha256`; `twilio_sms` uses `twilio_request_signature` — those three are genuinely production-grade.
- **Shim READMEs + status — FIXED (T5):** all 12 `include!`-based shims now have a `README.md` and their `*.capabilities.json` carry a `production_status` field (16 capabilities files carry it in total). **Still open (T6, WS-13-owned):** the CI `channel-crates` matrix (`.github/workflows/ci.yml`) still builds/tests only telegram/slack/discord/whatsapp — the shims are not yet compiled in CI, so a broken `include!` or capabilities drift would still go uncaught until the matrix is expanded.

## Decision Points

1. **Shared SDK crate vs. shared-source `include!` (build-vs-build).**
   - *Option A — new path-dependency crates* `channels-src/thinclaw-channel-sdk` and `tools-src/thinclaw-tool-sdk`, each its own standalone workspace, depended on via `path = "../thinclaw-channel-sdk"`. Cargo path deps resolve across workspace boundaries, so this works, but every consumer crate must add the dep + bump its checked-in `Cargo.lock`, and each crate re-generates WIT bindings independently (the SDK can't reference `exports::near::...` types unless it also generates them or takes them as generics).
   - *Option B — shared-source `include!`* mirroring the existing, proven `shared_webhook_channel` pattern: put `json_response`/`split_message`/`conversation_scope_id`/`external_conversation_key` in `channels-src/shared_channel_helpers/src/helpers.rs` and `include!("../../shared_channel_helpers/src/helpers.rs")` in each channel. WIT types are already in scope at the include site, no Cargo.toml/lock churn, no cross-workspace dep wiring. Same for `tools-src/shared_tool_helpers/`.
   - **Recommendation: Option B (shared `include!` module).** It is the pattern the repo already uses for the 12 shims (`include!("../../shared_webhook_channel/src/impl.rs")`), needs no `Cargo.lock` edits across 16+ standalone workspaces, and avoids the WIT-binding-duplication trap. The audit's structural goal ("one source of truth so the fix can't land in 1 of N") is fully met by a single included `.rs`. Reserve a true SDK *crate* only if a future consumer needs the helpers outside an `include!` context. (`split_message` is pure string logic with no WIT dependency, so it could alternatively live in a tiny real crate — but keeping all four helpers in one included module is simpler and consistent.)

2. **Discord Ed25519: WIRE vs. feature-gate vs. ERASE the flag.**
   - The audit rates this **High** (a shipped channel advertising a security control it does not perform). The infrastructure (`WebhookSecretValidation` enum + router match + `secret-validated` WIT field + `ed25519-dalek` already a workspace dep at `Cargo.toml:171`, used in `src/extensions/signing.rs`) is all present.
   - **Recommendation: WIRE.** Add the `DiscordEd25519` variant, the `verify_discord_ed25519_signature` host helper, the capabilities `channel.webhook` block, and the WASM-side `req.secret_validated` check. Erasing the flag would leave Discord interactions unauthenticated (anyone who learns the endpoint can forge interactions) and contradicts the operator directive to realize the vision. This is the single highest-value task in the workstream.

3. **Shim signature validation: tighten now vs. document-as-equals.**
   - Implementing per-platform signatures (DingTalk/Feishu/QQ/WeChat) means new `WebhookSecretValidation` variants and host helpers — non-trivial and unverifiable without live platform accounts.
   - **Recommendation: classify + document now, tighten opportunistically.** Mark `line`/`twitch`/`twilio_sms` (and Discord after task T2) **production**; mark the 9 `equals`-only shims **"beta / inbound auth = shared-secret only"** in their capabilities `description` and a new README, and add a one-line `"production_status"` marker. Only `qq` (Ed25519) can reuse the Discord helper cheaply — fold that in as a stretch task. Do not silently ship them as if signature-verified.

## Tasks

- [x] **T1: Port char-aware `split_message` to telegram/slack/discord (and unicode test).**
  - **Files:** `channels-src/telegram/src/lib.rs` (replace `:2174-` body, slice bug at `:2189`), `channels-src/slack/src/lib.rs` (`:1044-`, bug at `:1058`), `channels-src/discord/src/lib.rs` (`:735-`, bug at `:750`).
  - **Change:** Replace each body with the WhatsApp implementation (`whatsapp/src/lib.rs:2096-2143`): guard on `text.chars().count()`, add nested `fn byte_index_for_char_limit`, slice via `byte_index_for_char_limit(remaining, max_len)` instead of `&remaining[..max_len]`. Add `test_split_message_is_unicode_safe` (the `"🙂".repeat(5000)` test from `whatsapp/src/lib.rs:2300`) to each `mod tests`, adjusting the expected chunk count to each channel's limit constant (`TELEGRAM_MAX_MESSAGE_LENGTH`, slack's `max_len`, `DISCORD_MAX_MESSAGE_LENGTH`). *(If T4's shared-helper extraction lands first, this collapses into editing the single included file + per-channel test — see T4.)*
  - **Acceptance:** Each of the three `cargo test --manifest-path channels-src/<c>/Cargo.toml` passes including the new unicode test; no `&str[..n]` byte-slice remains in any `split_message`.
  - **Effort:** S
  - **Verification:** `cargo test --manifest-path channels-src/telegram/Cargo.toml && cargo test --manifest-path channels-src/slack/Cargo.toml && cargo test --manifest-path channels-src/discord/Cargo.toml`

- [x] **T2: Implement Discord Ed25519 interaction verification end-to-end.**
  - **Files (host):** `crates/thinclaw-channels/src/wasm/schema.rs:519-531` (add `DiscordEd25519` enum variant, `#[serde(rename = "discord_ed25519")]`), `crates/thinclaw-channels/src/wasm/router.rs:622-645` (add the match arm + helper) — add `ed25519-dalek` to `crates/thinclaw-channels/Cargo.toml` (workspace dep already at root `Cargo.toml:171`). **Files (package):** `channels-src/discord/discord.capabilities.json` (add `capabilities.channel.webhook`), `channels-src/discord/src/lib.rs:197` (consume `req.secret_validated`), `channels-src/discord/src/lib.rs:147-149` (drop `#[allow(dead_code)]`, use the flag), `channels-src/discord/README.md:105-108` (fix false claim — coordinate with WS-12).
  - **Change:**
    1. New variant `DiscordEd25519` in `WebhookSecretValidation`.
    2. `fn verify_discord_ed25519_signature(public_key_hex: &str, headers: &HeaderMap, body: &[u8], signature_hex: &str) -> bool` modeled on `verify_twitch_eventsub_signature` (`router.rs:279-302`) for the multi-header read and on `src/extensions/signing.rs:6` for `VerifyingKey`/`Signature`/`Verifier` usage: read `X-Signature-Timestamp`, build `timestamp_bytes ++ body`, hex-decode the public key (32 bytes) and the signature (64 bytes), `VerifyingKey::from_bytes`, `.verify(msg, &sig).is_ok()`. The signature itself comes from `X-Signature-Ed25519` — set the capabilities `secret_header` to that, so router's `provided` (`router.rs:573-588`) carries the hex signature. The `expected` value (`router.rs:605-608`) becomes the `discord_public_key` secret.
    3. capabilities `channel.webhook`: `{ "secret_header": "X-Signature-Ed25519", "secret_name": "discord_public_key", "secret_validation": "discord_ed25519" }`; add `discord_public_key` to `setup.required_secrets`; allow `discord_*` already covers it (`secrets.allowed_names`).
    4. In `discord/src/lib.rs on_http_request`, after parsing the interaction, mirror whatsapp (`whatsapp/src/lib.rs:710`): if `config.require_signature_verification && !req.secret_validated` → return `json_response(401, ...)`. Read `require_signature_verification` from persisted config the same way `owner_id` is persisted in `on_start` (or gate unconditionally on `!req.secret_validated` since the endpoint sets `require_secret: true`). Remove the `#[allow(dead_code)]`.
    5. README: replace the false claim with an accurate description (host verifies Ed25519 over timestamp+body using `discord_public_key`; requests failing verification are rejected with 401 before reaching the WASM).
  - **Acceptance:** A host unit test in `router.rs` (mirror `test_router_secret_validation` at `router.rs:833`) signs a body with a known `SigningKey` (see `src/extensions/signing.rs:52` test pattern) and asserts a valid signature → `secret_validated == true`, a tampered body/sig → 401. Discord `cargo test` passes. `discord.capabilities.json` parses (loader test). README no longer claims unimplemented behavior.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw-channels wasm::router && cargo test --manifest-path channels-src/discord/Cargo.toml`

- [ ] **T3: Extract shared tool helpers (`url_encode_path`, `validate_input_length`).**
  - **Files:** new `tools-src/shared_tool_helpers/src/helpers.rs`; `tools-src/github/src/lib.rs` + `tools-src/notion/src/lib.rs` (replace the two duplicated fns with `include!("../../shared_tool_helpers/src/helpers.rs")`).
  - **Change:** Move the byte-identical `url_encode_path` and `validate_input_length` into one included module (Option B from Decision 1), mirroring `shared_webhook_channel`. Keep any tool-specific length constants local; the helper takes `max_len` as a param. Move the existing unit tests for these fns into the shared module under `#[cfg(test)]`.
  - **Acceptance:** Both tool crates compile and their existing tests pass; the two fns exist in exactly one source file. `grep -rln "fn url_encode_path" tools-src/` returns only the shared file.
  - **Effort:** S
  - **Verification:** `cargo test --manifest-path tools-src/github/Cargo.toml && cargo test --manifest-path tools-src/notion/Cargo.toml`

- [ ] **T4: Extract shared channel helpers (`json_response`, `split_message`, `conversation_scope_id`, `external_conversation_key`).**
  - **Files:** new `channels-src/shared_channel_helpers/src/helpers.rs`; `channels-src/{discord,telegram,slack,whatsapp}/src/lib.rs` (`include!` it, delete local copies). `conversation_scope_id`/`external_conversation_key` only apply to telegram + whatsapp.
  - **Change:** Move the **fixed** `split_message` (with `byte_index_for_char_limit`) and `json_response` into the shared module; for `conversation_scope_id`/`external_conversation_key`, move telegram's/whatsapp's into the shared module if byte-identical (verify first — they may differ in the `chat_id` typing; if they diverge, leave per-channel and note why). Each channel `include!`s the shared file after `wit_bindgen::generate!` so `OutgoingHttpResponse` etc. are in scope (same constraint shared_webhook_channel already satisfies). Put the unicode-safe + boundary tests in the shared module. **Do T1 first OR fold T1's fix directly into this shared `split_message`** so the fix exists once.
  - **Acceptance:** All four channel crates compile and test green; `grep -rln "fn split_message" channels-src/` returns only the shared file (plus `shared_webhook_channel` if it has its own — verify and consolidate if identical). The shared `split_message` is unicode-safe.
  - **Effort:** M
  - **Verification:** `for c in discord telegram slack whatsapp; do cargo test --manifest-path channels-src/$c/Cargo.toml || break; done`

- [x] **T5: Classify the 12 shim channels and record production status.**
  - **Files:** each shim's `*.capabilities.json` (`description` + new `"production_status"` field), new `channels-src/<shim>/README.md` (12 + reuse for twitch/twilio_sms), and a summary table appended by **WS-12** to `docs/CHANNEL_ARCHITECTURE.md` (WS-03 provides the table content; WS-12 owns the doc edit).
  - **Change:** Mark `line`, `twitch`, `twilio_sms`, `discord` (post-T2) as **production** (real signature validation). Mark the 9 `equals`-only shims (`dingtalk`, `feishu_lark`, `google_chat`, `matrix`, `mattermost`, `ms_teams`, `qq`, `wecom`, `weixin`) as **beta** with an explicit caveat: "inbound webhook auth is shared-secret `equals` only; platform-native signature verification is not yet implemented." Add a short README per shim documenting required secrets, the webhook path, and the auth caveat. **Stretch:** implement `qq` Ed25519 by reusing the T2 `verify_discord_ed25519_signature` helper generalized to a `Ed25519TimestampBody` variant.
  - **Acceptance:** Every shim capabilities.json carries an honest `production_status`; every shim has a README; the classification table exists for WS-12 to consume. No shim claims signature verification it does not perform.
  - **Effort:** M
  - **Verification:** `for f in channels-src/*/*.capabilities.json; do python3 -c "import json;json.load(open('$f'))" || echo "BAD $f"; done` (all parse); manual read of one README + one capabilities `description`.

- [ ] **T6: Supply WS-13 the buildability requirement for shims + tools-src.**
  - **Files:** none owned by WS-03 in `.github/workflows/ci.yml` or `scripts/build-all.sh` (those belong to WS-13). WS-03 deliverable: a verified list of crates that must compile to `wasm32-wasip2`.
  - **Change:** Confirm each of the 12 shims and `tools-src/*` builds to `wasm32-wasip2` locally (catches `include!` drift the current CI matrix at `ci.yml:725-751` misses, since it only covers 4 channels and runs native `cargo test`). Hand WS-13 the matrix entries to add and the `build-all.sh` gap (it iterates `channels-src/*/` but never builds `tools-src` — verified `scripts/build-all.sh:80-83`).
  - **Acceptance:** A documented, locally-verified list of all WASM crates and their `wasm32-wasip2` build status, delivered to WS-13. Any crate that fails to build is fixed here (it's WS-03-owned source) even though the CI wiring is WS-13.
  - **Effort:** S
  - **Verification:** `rustup target add wasm32-wasip2; for d in channels-src/*/ tools-src/*/; do [ -f "$d/Cargo.toml" ] && (cargo build --release --target wasm32-wasip2 --manifest-path "$d/Cargo.toml" || echo "FAIL $d"); done`

## Best Practices (workstream-specific)

- **Copy the proven implementation, don't re-derive.** `whatsapp/src/lib.rs:2096` is the canonical char-aware splitter and `whatsapp`/`telegram` are the canonical `secret_validated` consumers (`whatsapp:571,710`; `telegram:1640`). Port verbatim.
- **Host validates, WASM trusts the bit.** All webhook signature logic lives host-side in `router.rs`; the WASM only reads `req.secret_validated` (`wit/channel.wit:247`). Never implement crypto inside a WASM channel — it cannot see raw secrets by design.
- **Mirror the existing validation variants.** `verify_twitch_eventsub_signature` (`router.rs:279`) is the template for "read extra headers + reconstruct the signed payload"; `src/extensions/signing.rs:6` is the template for ed25519-dalek `VerifyingKey`/`Verifier`. Use constant-time compares where applicable (the HMAC helpers already use `subtle::ConstantTimeEq`; ed25519-dalek's `verify` is constant-time internally).
- **One source of truth via `include!`.** The repo already proves this with `include!("../../shared_webhook_channel/src/impl.rs")`. Put shared helpers in one `.rs` and include it; do not add a path-dependency crate unless a non-`include!` consumer appears.
- **Capabilities config is the channel's contract.** For shim channels, behavior changes go in `*.capabilities.json` (`mapping`, `response`, `challenge`, `channel.webhook`), not code. Keep `production_status` and `description` honest.

## Common Pitfalls

- **The exact bug this WS exists to prevent:** the WhatsApp `split_message` fix landed in 1 of 4 copies and the other three still panic. After T4, there must be exactly one `split_message` source. Re-run `grep -rln "fn split_message" channels-src/` as a gate.
- **Byte vs. char slicing.** `&s[..max_len]` panics when `max_len` lands mid-codepoint — this is *the* bug at `telegram:2189`, `slack:1058`, `discord:750`. The `.unwrap_or_else` char-boundary fallback below those lines is a red herring; the panic is on the slice above it.
- **Declaring a security control you don't perform.** Discord's `require_secret: true` + README claim + parsed-but-`#[allow(dead_code)]` flag created the illusion of verification. Wiring requires *all four* pieces: enum variant, host helper, capabilities `channel.webhook` block, and the WASM `secret_validated` check. Missing any one silently re-opens the hole.
- **`secret_name` for Discord is a public key, not a shared secret.** Do not default to `discord_webhook_secret`; the `channel.webhook` block must point `secret_name` at `discord_public_key`, and the helper must hex-decode it as a 32-byte verifying key.
- **Standalone-workspace `Cargo.lock` churn.** Each channel/tool crate has its own lockfile (`channels-src/discord/Cargo.lock` etc.). Adding a path-dependency crate (Option A) forces a lock bump in every consumer; `include!` (Option B) avoids it. If you add `ed25519-dalek` to `crates/thinclaw-channels`, that's the *host* workspace lock, which is normal.
- **Shims are invisible to CI.** The `channel-crates` matrix (`ci.yml:725-751`) covers only 4 crates and runs `cargo test` (native), so a broken `include!` in a shim or a malformed capabilities.json ships undetected. T6 + WS-13 close this; until then, build them locally before merging shim changes.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - *Parallel, independent:* **T1** (split fix in 3 channels), **T3** (tool helpers), **T5** (shim classification/docs) touch disjoint files and can run as three concurrent subagents.
  - *Sequential:* **T2** (Discord Ed25519) edits host `router.rs`/`schema.rs` + discord package; run as its own worker. **T4** (channel-helper extraction) should run *after or fused with* T1 so the shared `split_message` already carries the fix — do T1's fix directly inside T4's shared module to avoid double-editing. **T6** runs last (it verifies the whole fleet builds).
  - Recommended fan-out: Worker A = T2 (host crypto, highest risk, isolated). Worker B = T1+T4 fused (channel helpers). Worker C = T3 (tool helpers). Worker D = T5 (shim docs). Then a single integration step runs T6 across all crates.
- **Isolation:** Use **git worktree isolation**. Worker A mutates `crates/thinclaw-channels/src/wasm/*` and `channels-src/discord/*`; Worker B mutates `channels-src/{telegram,slack,whatsapp,discord}/src/lib.rs` + the new shared helper — **B and A both touch `discord/src/lib.rs`** (A adds the `secret_validated` check, B swaps `split_message`/`json_response` to the include). Sequence A→B on the discord file, or have one worker own all `discord/src/lib.rs` edits. Separate worktrees for C and D (disjoint trees).
- **Workflow shape:**
  1. *Implement* (fan-out A/B/C/D in worktrees).
  2. *Verify* per-worker: each runs its crate's `cargo test` + `wasm32-wasip2` build.
  3. *Integrate*: merge worktrees, resolve the `discord/src/lib.rs` overlap, run T6 fleet build.
  4. *Review* (`/code-review` on the combined diff, focus on the new crypto in `router.rs`).
  5. *Fix* any review findings; re-run the gate.
- **Verification gate (exact commands):**
  - `cargo fmt --all` (host workspace) and `cargo fmt --manifest-path channels-src/<c>/Cargo.toml` per touched crate.
  - `cargo clippy -p thinclaw-channels --all-targets -- -D warnings` (host changes for T2).
  - Per WASM crate: `cargo test --manifest-path channels-src/<c>/Cargo.toml -- --nocapture` and `cargo test --manifest-path tools-src/<c>/Cargo.toml`.
  - `cargo test -p thinclaw-channels wasm::router wasm::schema wasm::loader` (host validation + capabilities parsing).
  - Fleet build: `rustup target add wasm32-wasip2; for d in channels-src/*/ tools-src/*/; do [ -f "$d/Cargo.toml" ] && cargo build --release --target wasm32-wasip2 --manifest-path "$d/Cargo.toml"; done`.
  - Capabilities JSON validity: `for f in channels-src/*/*.capabilities.json; do python3 -c "import json;json.load(open('$f'))"; done`.
  - `/ship` for the host-workspace quality gate; `/code-review high` on the Ed25519 diff.
  - **Prerequisites:** none DB/Docker — WASM channel/tool crates and `thinclaw-channels` unit tests run without Postgres or Docker. `wasm-tools` only needed for `build-all.sh` packaging (WS-13), not for these tests.

## Definition of Done

- [x] `split_message` is char-aware in telegram, slack, discord; each has a passing `test_split_message_is_unicode_safe`. (The "exactly one source after T4" clause is **not** met — T4 shared-helper extraction did not land, so four copies remain.)
- [x] Discord interactions are Ed25519-verified host-side: `DiscordEd25519` variant + `verify_discord_ed25519_signature` helper land in `thinclaw-channels`, `discord.capabilities.json` has a `channel.webhook` block keyed on `discord_public_key`, the WASM rejects `!secret_validated`, and `require_signature_verification` is consumed (no `#[allow(dead_code)]`). Host unit test covers valid + tampered signatures.
- [x] `discord/README.md` no longer claims verification that didn't exist; it accurately describes the Ed25519 flow (coordinated into WS-12 doc-sync).
- [ ] `json_response`/`split_message`/`conversation_scope_id`/`external_conversation_key` (channels) and `url_encode_path`/`validate_input_length` (tools) exist in exactly one source file each; all consumer crates compile and test green. **Open (T3/T4 not landed).**
- [x] All 12 shims classified (production vs. beta) with an honest `production_status` + caveat in capabilities and a per-shim README; classification table delivered to WS-12.
- [ ] All `channels-src/*` and `tools-src/*` crates compile to `wasm32-wasip2`; the buildability list + `build-all.sh`/CI-matrix gaps are delivered to WS-13. **Open (T6, WS-13-owned).**
- [ ] Verification gate green: `cargo fmt`, `clippy --all-targets -D warnings` (host), per-crate `cargo test`, capabilities JSON parse, fleet `wasm32-wasip2` build. **Partial:** the shipped tasks (T1/T2/T5) pass their gates; the SDK-extraction gate (T3/T4) is not yet run.
- [x] Decisions 2 (WIRE Discord) and 3 (classify shims) executed. Decision 1 (SDK vs. `include!`) remains **unresolved** — the shared-helper extraction it governs (T3/T4) has not landed.
