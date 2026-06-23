# WS-11 — Dead-Code Sweep & Vision Decisions

> **Status:** Not started · **Priority:** P2 · **Risk:** low · **Effort:** L
> **Depends on:** WS-05 (native plugins / self-repair `with_builder` & `RepairTask`), WS-10 (god-file decomposition & `src/history/store` dedup — declares `src/safety/*.rs` orphan deletion as **WS-11**'s) · **Blocks:** none
> **Owns (symbols/files):** `src/safety/auth_profiles.rs`, `src/safety/credential_detect.rs`, `src/safety/dangerous_tools.rs`, `src/safety/device_pairing.rs`, `src/safety/elevated.rs`, `src/safety/key_rotation.rs`, `src/safety/leak_detector.rs`, `src/safety/media_url.rs`, `src/safety/osv_check.rs`, `src/safety/pii_redactor.rs`, `src/safety/policy.rs`, `src/safety/sanitizer.rs`, `src/safety/skill_path.rs`, `src/safety/validator.rs` (the 14 uncompiled orphan siblings of `src/safety/mod.rs`); `src/cli/nodes.rs`, `src/cli/subagent_spawn.rs`, `src/cli/session_export.rs` (declared-but-unwired CLI modules); `build.rs::build_telegram_channel`; `crates/thinclaw-gateway/src/web/sse.rs::SseManager::subscribe` (test-only); `crates/thinclaw-channels/src/self_message.rs` + `src/channels/self_message.rs` facade; `src/extensions/manager.rs::install_bundled_channel_from_artifacts`; `src/boot_screen.rs::redact_gateway_url`; `src/cli/secrets.rs::_secret_cli_access_context`; `src/voice_wake.rs` + `voice` feature + `cpal` dep; `src/tailscale.rs` (`TailscaleDiscovery`, the discovery module — **NOT** `src/tunnel/tailscale.rs::TailscaleTunnel`); `src/qr_pairing.rs`.
> **Explicitly NOT owned by WS-11:** `crates/thinclaw-agent/src/self_repair.rs::RepairTask` and the self-repair `with_builder` wiring (WS-05 decides wire-vs-erase; WS-11 only flags it). `src/safety/mod.rs` itself (the live façade — keep, never delete).

## Vision & Goal

The audit confirmed ThinClaw is a mature platform whose dead code is *concentrated and identifiable* rather than pervasive. This workstream removes the genuinely drifted duplicate cruft that taxes every reader and risks "fix lands in only one of N copies" bugs (the audit caught exactly this with `src/safety/sanitizer.rs` having drifted regex ordering vs the live crate), and makes a deliberate wire-or-erase call on three orphaned vision features (`voice_wake`, `tailscale` discovery, `qr_pairing`) per the operator's realize-the-vision directive. The product win is a smaller, honest trust surface: no shadow safety code that *looks* authoritative but never runs, and no half-built capability silently rotting.

## Scope

**In scope:**
- ERASE bucket: delete the 14 orphan `src/safety/*.rs` files (façade `mod.rs` stays); delete the 3 unwired CLI modules + their `pub mod` lines; delete `build_telegram_channel`; delete the test-only `subscribe()`; delete the dead `self_message` module (both crate + root facade) **unless** the WIRE decision below carries; delete `install_bundled_channel_from_artifacts`, `redact_gateway_url`, `_secret_cli_access_context`.
- DECIDE bucket: resolve wire-vs-erase for `src/voice_wake.rs` (+ `voice` feature + `cpal`), `src/tailscale.rs` (`TailscaleDiscovery`), `src/qr_pairing.rs`; if wired, fix `qr_pairing`'s non-constant-time token compare and hand-rolled base64.

**Out of scope (and which WS owns it):**
- `src/history/store/**` dedup, all god-file decompositions, `src/media` extraction — **WS-10**.
- `self_repair.rs::RepairTask`, self-repair `with_builder`, native dynamic-library plugin pipeline (`src/extensions/native.rs`) — **WS-05** (flag/coordinate only).
- Behavior fixes *inside* the live `thinclaw-safety` crate (e.g. the sanitizer regex ordering itself is correct in the crate; WS-11 only deletes the stale root copy, it does not edit crate behavior).

## Current State (verified)

**`src/safety/*` — drifted dead (façade + 14 uncompiled orphans):**
- `src/safety/mod.rs` is a 4-line **live façade**: `pub use crate::tools::builtin::{...}; pub use thinclaw_safety::*;` — it declares **zero** `mod x;` for its sibling files and there is no `#[path]` trick (`grep "#\[path" src/safety/ src/lib.rs` → empty). The 14 sibling `.rs` files are therefore not in the module tree and never compile.
- `src/lib.rs:86` `pub mod safety;` points at that façade only.
- The crate `crates/thinclaw-safety/src/lib.rs` is the live implementation (`pub mod skill_path;` line 15, `pub mod auth_profiles;`, etc.). `crate::safety::skill_path::SkillPathConfig` used at `src/skills/registry.rs:423` resolves *through the façade glob* to `thinclaw_safety::skill_path`, **not** the root orphan — confirmed because the root has no `mod skill_path;` declaration.
- **Drift is real** between root orphans and the live crate (so they are stale, not just duplicate): `src/safety/sanitizer.rs` has different injection-pattern table ordering and pre-rustfmt test bodies vs `crates/thinclaw-safety/src/sanitizer.rs`; `src/safety/pii_redactor.rs` has the old `|value| true` keep-closure signature vs the crate's `|value, range| {...}` with `has_user_id_label_prefix` user-id guard; `src/safety/osv_check.rs` imports a now-removed `crate::config::helpers::lock_env`; `src/safety/skill_path.rs` uses `crate::platform::state_paths()` where the crate uses `dirs::home_dir()`. These would not even compile if reconnected.

**Dead CLI modules (declared, never matched):**
- `src/cli/mod.rs:32 pub mod nodes;`, `:41 pub mod session_export;`, `:44 pub mod subagent_spawn;` — 265 + 375 + 374 = 1014 lines. `grep "nodes::|subagent_spawn::|session_export::" src/` returns only their own definitions; never referenced from a command match.

**`build.rs::build_telegram_channel` (build.rs:42):** `#[allow(dead_code)]`, never called — `main()` only calls `embed_registry_catalog` and (under `bundled-wasm`) `build_all_wasm_extensions`. ~90 lines.

**`SseManager::subscribe` (`crates/thinclaw-gateway/src/web/sse.rs:88`):** Only caller is its own test at `sse.rs:254`. Production uses `subscribe_raw` (`sse.rs:61`) everywhere — `src/channels/web/ws.rs:90`, `src/channels/web/handlers/chat.rs:313,834,936`, `src/channels/web/mod.rs:639`. The `Sse`/`Event`/`KeepAlive` imports (`sse.rs:8`) and `Infallible` (`sse.rs:3`) are used **only** by `subscribe()`; removing it removes those imports.

**`self_message` (`crates/thinclaw-channels/src/self_message.rs`, 251 lines):** `crates/thinclaw-channels/src/lib.rs:28 pub mod self_message;`; root facade `src/channels/self_message.rs` (44 bytes: `pub use thinclaw_channels::self_message::*;`) declared at `src/channels/mod.rs:68` and re-exported `pub use self_message::{SelfMessageConfig, TrustedMetadata};` at `src/channels/mod.rs:99`. Outside the definition + that one re-export, `SelfMessageConfig`/`TrustedMetadata`/`is_self_message`/`filter_messages` have **zero production callers** (`grep` across `crates/ src/ apps/` shows only the module + the re-export line). The module doc *claims* an anti-loop guarantee ("Prevents the agent from processing its own messages... infinite loops") that nothing enforces.

**Confirmed-dead helpers:**
- `src/extensions/manager.rs:1678 install_bundled_channel_from_artifacts` — `#[allow(dead_code)] // Reserved...`, no callers.
- `src/boot_screen.rs:243 redact_gateway_url` — no callers besides definition.
- `src/cli/secrets.rs:278 _secret_cli_access_context` — `#[allow(dead_code)]`, leading-underscore dead name.

**DECIDE — `src/voice_wake.rs` (749 lines):** `src/lib.rs:109 pub mod voice_wake;`. Zero Rust callers (`grep "voice_wake::|VoiceWake|WakeBackend"` across `src/ crates/ apps/` → empty). Gated behind `voice` feature (`Cargo.toml:339 voice = ["dep:cpal", "thinclaw-config/voice"]`, `cpal` at `Cargo.toml:223`). **`voice` is in NO build profile** — not `edge` (`Cargo.toml:258`), `light` (`:261`), `desktop` (`:271`), or `full` (`:296`). So it only compiles under explicit `--features voice` and even then is unreachable. The desktop app has its **own** TS/Tauri voice path (`apps/desktop/frontend/src/hooks/use-voice-wake.ts`, `VoiceWakeOverlay.tsx`) — the module doc itself says "in desktop mode (Tauri), Scrappy owns the microphone." So this Rust module targets *headless/remote* mode exclusively.

**DECIDE — `src/tailscale.rs` (331 lines):** `src/lib.rs:96 pub mod tailscale;`. Defines `TailscaleDiscovery` (queries `localhost:41112` Tailscale local API to auto-find the headless orchestrator from the Tauri thin client). **Zero callers** (`grep "TailscaleDiscovery"` ex-self → empty). Distinct from the **live, wired** `src/tunnel/tailscale.rs::TailscaleTunnel` (`src/tunnel/mod.rs:24 pub use tailscale::TailscaleTunnel;`) — that one is a tunnel provider, fully used. The root `src/tailscale.rs` is the orphaned *discovery* helper.

**DECIDE — `src/qr_pairing.rs` (329 lines):** `src/lib.rs:83 pub mod qr_pairing;`. Defines `PairingInfo`, `PairingSession`, `render_qr_terminal` for a Tailscale-less QR pairing fallback. **Zero Rust callers** of `PairingSession`/`validate_token`/`generate_pairing_token` (the `validate_token` hits in `src/cli/tool.rs:679,864,935` are an unrelated OAuth helper). The desktop frontend's `qrCode` state (`ThinClawChannels.tsx:197`) is separate TS. **Two security defects if ever wired:** (1) non-constant-time token compare `if self.info.pairing_token == token` (`qr_pairing.rs:219`); (2) hand-rolled `base64_encode` (`qr_pairing.rs:94`) instead of the `base64` crate the repo already uses (`URL_SAFE_NO_PAD` at `crates/thinclaw-tools/src/wasm/oauth.rs:4`). The **live** pairing path is `src/pairing/store.rs::PairingStore` (wired at `src/main_helpers.rs:320`, `src/main.rs:709`) — a different, persistent device-pairing store. `qr_pairing.rs` is a parallel, never-connected mechanism.

## Decision Points

1. **`src/safety/*` 14 orphans — WIRE vs ERASE.** Options: (a) ERASE the stale root copies, keep the façade. (b) Reconnect them as the live impl. → **Recommend ERASE.** They are drifted *duplicates* of the live `thinclaw-safety` crate (won't even compile against current `crate::` paths). This is textbook "genuinely drifted duplicate cruft," not vision. WS-10 already disclaims them to WS-11.

2. **`src/cli/{nodes,subagent_spawn,session_export}.rs` — WIRE vs ERASE.** These are unwired CLI surfaces (node management, subagent spawn CLI, session export). Per realize-the-vision they *could* be wired into the command match. → **Recommend ERASE.** (a) The functionality already exists via live surfaces: subagent spawning runs through `SpawnSubagentTool`/the agent runtime, session/transcript export through the gateway and history store; these CLI modules are an abandoned earlier attempt, not the canonical path. (b) Wiring them means designing + documenting 3 new operator commands in `docs/CLI_REFERENCE.md` and `docs/SURFACES_AND_COMMANDS.md`, far beyond a P2 dead-code sweep. **Escalation note:** if the operator wants any of these as real commands, spin a dedicated WS — do not smuggle command design into this sweep.

3. **`self_message` — WIRE vs ERASE.** It documents an anti-loop guarantee it does not enforce; zero callers. Options: (a) ERASE. (b) WIRE: call `SelfMessageConfig::from_env()` + `filter_messages` in the channel ingest path before dispatch. → **Recommend ERASE.** Native channels already filter self-echo at the transport layer (each channel knows its bot id), and the gateway is the trusted control plane; a second, unwired filter that *claims* a guarantee is worse than none (false assurance). If a future audit shows a real echo loop, WIRE is a clean ~1-day follow-up against this same module — but ship the erase now. (Low confidence this is vision vs cruft; defaulting to erase because the doc-vs-behavior lie is itself a hazard.)

4. **`src/voice_wake.rs` + `voice` feature + `cpal` — WIRE vs ERASE.** → **Recommend WIRE (gate into a feature, do not auto-enable in a profile).** This is genuine half-built vision (energy-detector implemented, Sherpa-ONNX scaffolded) explicitly scoped to *headless/remote* mode, which the desktop TS path does not cover. ERASE would delete a real capability the product roadmap implies. **But** it must NOT enter `light`/`desktop`/`full` by default (mic capture is a privacy + dependency-weight decision; `cpal` pulls platform audio libs). Wire it behind the existing `voice` feature with a documented opt-in and a real spawn point in the headless runtime. Provide the ERASE plan too in case the operator de-scopes headless voice. *Effort asymmetry: WIRE = M (needs a spawn site + event plumbing into the agent), ERASE = S.*

5. **`src/tailscale.rs` (`TailscaleDiscovery`) — WIRE vs ERASE.** → **Recommend WIRE into deployment auto-discovery (feature-gated under `tunnel`).** Mesh auto-find of the headless orchestrator from the thin client is real deployment value and complements the already-wired `TailscaleTunnel`. Wire `TailscaleDiscovery` into the deployment/onboarding path so the Tauri client can locate the orchestrator without manual IP. Provide ERASE as the fallback if the operator standardizes on manual/QR pairing only. *Effort: WIRE = M (call site in deploy/onboarding + a thin-client consumer), ERASE = S.*

6. **`src/qr_pairing.rs` — WIRE vs ERASE.** → **Recommend ERASE.** This is the one DECIDE item I push toward erase despite the realize-the-vision default, because the **live** pairing mechanism is `src/pairing/store.rs::PairingStore` and `qr_pairing.rs` is a *parallel, never-connected* fallback with two security defects. Wiring it means maintaining two pairing systems and hardening this one (constant-time compare + drop hand-rolled base64) for a path the live store already covers. If the operator wants QR-over-the-PairingStore, that is a feature on top of `PairingStore`, not a revival of this module. **If the operator overrides to WIRE:** the security fixes are mandatory and specified in T-WIRE-QR below.

## Tasks

Ordered so the safe, no-decision erases land first; DECIDE items follow once the operator signs off (see Decision Points). Tasks touching files another WS owns are flagged.

- [ ] **T1: Erase the 14 drifted `src/safety/*.rs` orphans (keep the façade).**
  - **Files:** delete `src/safety/{auth_profiles,credential_detect,dangerous_tools,device_pairing,elevated,key_rotation,leak_detector,media_url,osv_check,pii_redactor,policy,sanitizer,skill_path,validator}.rs`. Keep `src/safety/mod.rs` exactly as-is.
  - **Change:** `git rm` the 14 files. Do not touch `mod.rs`, `crates/thinclaw-safety/**`, or any `crate::safety::*` import (they resolve through the façade to the crate).
  - **Acceptance:** `src/safety/` contains only `mod.rs`; `cargo check --features light` and `--no-default-features --features edge` compile with zero new warnings; `grep -rn "crate::safety::" src/ | grep -v src/safety/mod.rs` still resolves (it always pointed at the crate).
  - **Effort:** S
  - **Verification:** `cargo check --no-default-features --features edge && cargo check --features light && cargo clippy --all --benches --tests --examples --all-features -- -D warnings`

- [ ] **T2: Erase the 3 unwired CLI modules.**
  - **Files:** delete `src/cli/nodes.rs`, `src/cli/subagent_spawn.rs`, `src/cli/session_export.rs`; remove `pub mod nodes;` (`src/cli/mod.rs:32`), `pub mod session_export;` (`:41`), `pub mod subagent_spawn;` (`:44`).
  - **Change:** `git rm` + delete the 3 `pub mod` lines. Confirm no re-export elsewhere first (`grep -rn "nodes::|subagent_spawn::|session_export::" src/`).
  - **Acceptance:** files gone; `grep` returns nothing; CLI command match unchanged; build green.
  - **Effort:** S
  - **Verification:** `cargo check --features full` (CLI surfaces live under `repl`/`full`) `&& cargo clippy --all --tests -- -D warnings`

- [ ] **T3: Erase `build_telegram_channel` from `build.rs`.**
  - **Files:** `build.rs` — remove `fn build_telegram_channel` (lines 41–131, including the `#[allow(dead_code)]` and doc comment).
  - **Change:** delete the function. `main()` does not call it.
  - **Acceptance:** `build.rs` has no `build_telegram_channel`; a clean `cargo build` and `cargo build --features bundled-wasm` both succeed (bundled-wasm path is `build_all_wasm_extensions`, untouched).
  - **Effort:** S
  - **Verification:** `cargo build --features light && cargo build --release --features bundled-wasm` (the second requires `rustup target add wasm32-wasip2` + `wasm-tools`; if the WASM toolchain is unavailable in the runner, at minimum `cargo build --features light` must pass and a reviewer confirms `main()` is unchanged).

- [ ] **T4: Erase test-only `SseManager::subscribe` (gateway).**
  - **Files:** `crates/thinclaw-gateway/src/web/sse.rs` — remove `pub fn subscribe` (lines 85–~125, the `Sse`-returning fn) and its lone test caller at `sse.rs:254`; remove the now-unused imports `Sse`, `Event`, `KeepAlive` (`sse.rs:8`) and `Infallible` (`sse.rs:3`). Keep `subscribe_raw` and all its tests.
  - **Change:** delete the convenience SSE wrapper; production already wraps `subscribe_raw` into `Sse` in `src/channels/web/*`. Verify the deleted test at 254 only exercised `subscribe()` (the over-limit assertion has a `subscribe_raw` twin at `sse.rs:253`).
  - **Acceptance:** no `pub fn subscribe(` in `sse.rs`; `subscribe_raw` callers (`ws.rs:90`, `handlers/chat.rs:313,834,936`, `web/mod.rs:639`) compile unchanged; no unused-import warnings.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw-gateway --features <gateway profile>` and `cargo clippy -p thinclaw-gateway --all-targets -- -D warnings`. (Gateway compiles under `web-gateway`/`full`; run with `--features full` from the root package or the crate's own default if standalone.)

- [ ] **T5: Erase 3 confirmed-dead helpers.**
  - **Files:** `src/extensions/manager.rs` (remove `install_bundled_channel_from_artifacts`, ~lines 1678–1700+ — read the full fn body before cutting); `src/boot_screen.rs` (remove `redact_gateway_url`, line 243 + body); `src/cli/secrets.rs` (remove `_secret_cli_access_context`, lines 278–281).
  - **Change:** delete each fn and its `#[allow(dead_code)]`/doc. `manager.rs` is a god-file owned by **WS-10** for *decomposition*; this is a pure deletion of one dead method, not a structural move — **coordinate the cut with WS-10's manager.rs decomposition to avoid a merge collision** (land WS-11's deletion first or rebase). If WS-10's decomposition has already moved this method, drop this sub-item and note it resolved.
  - **Acceptance:** the 3 symbols gone; `grep -rn "install_bundled_channel_from_artifacts|redact_gateway_url|_secret_cli_access_context" src/` empty; build green.
  - **Effort:** S
  - **Verification:** `cargo check --features full && cargo clippy --all --tests -- -D warnings`

- [ ] **T6: [DECISION 3] Erase `self_message` (default) — gate on operator sign-off.**
  - **Files:** delete `crates/thinclaw-channels/src/self_message.rs`; remove `pub mod self_message;` (`crates/thinclaw-channels/src/lib.rs:28`); delete root facade `src/channels/self_message.rs`; remove `pub mod self_message;` (`src/channels/mod.rs:68`) and the re-export `pub use self_message::{SelfMessageConfig, TrustedMetadata};` (`src/channels/mod.rs:99`).
  - **Change:** ERASE per Decision Point 3. **If operator chooses WIRE instead:** keep the module and add a call site in the channel ingest path (`crates/thinclaw-channels` manager runtime) that builds `SelfMessageConfig::from_env()` once and runs `filter_messages` before dispatch — then this task becomes a WIRE task and updates `docs/CHANNEL_ARCHITECTURE.md`.
  - **Acceptance (erase):** no `self_message` module in either location; `grep -rn "SelfMessageConfig|TrustedMetadata" crates/ src/` empty; `thinclaw-channels` and root build green under `full`.
  - **Effort:** S (erase) / M (wire)
  - **Verification:** `cargo check --features full && cargo test -p thinclaw-channels && cargo clippy --all --tests -- -D warnings`

- [ ] **T7: [DECISION 4] WIRE `voice_wake` behind the `voice` feature (recommended) OR erase.**
  - **WIRE plan — Files:** `src/voice_wake.rs` (keep, gate cleanly), the headless runtime spawn site (the non-Tauri agent runtime entry; coordinate with the owning runtime WS for the exact spawn module — do not edit `agent_loop.rs`/`runtime_manager.rs` god-files, which **WS-10** owns for decomposition; add only a thin `#[cfg(feature = "voice")]` spawn call), `Cargo.toml` (keep `voice`/`cpal`), `docs/RESEARCH_AND_EXPERIMENTS.md` or a headless-mode doc + `docs/BUILD_PROFILES.md`.
    - **Change:** add a `#[cfg(feature = "voice")]` spawn of the wake-word loop in the headless runtime that emits a "wake detected" event into the agent (mirror an existing background-task spawn pattern). Keep `voice` opt-in; document in `docs/BUILD_PROFILES.md` that `voice` is headless-only and not in any default profile. Do **not** add `voice` to `light`/`desktop`/`full`.
    - **Acceptance:** `cargo build --features voice` compiles and the loop is reachable from the headless runtime; `cargo build --features light` (no voice) still excludes `cpal`; the module is no longer `#[allow(dead_code)]`-reachable-only.
    - **Effort:** M
  - **ERASE plan — Files:** delete `src/voice_wake.rs`; remove `pub mod voice_wake;` (`src/lib.rs:109`); remove the `voice` feature (`Cargo.toml:339`), the `cpal` optional dep (`Cargo.toml:223`), and `thinclaw-config/voice`; drop any `voice` references in docs.
    - **Acceptance:** no `voice_wake`, `voice` feature, or `cpal` in the tree; all profiles build green.
    - **Effort:** S
  - **Verification:** WIRE → `cargo build --features voice && cargo build --features light`; ERASE → `cargo check --no-default-features --features edge && cargo check --features full && cargo deny check` (confirm `cpal` removal does not strand a transitive advisory).

- [ ] **T8: [DECISION 5] WIRE `tailscale` discovery into deployment (recommended) OR erase.**
  - **WIRE plan — Files:** `src/tailscale.rs` (keep), a deployment/onboarding call site (coordinate with the setup/deploy owner — likely `src/setup/**` or `src/tunnel/mod.rs`; do not duplicate the live `TailscaleTunnel`), `docs/DEPLOYMENT.md`.
    - **Change:** feature-gate `TailscaleDiscovery` under `tunnel` and call it from the deploy/onboarding path so the thin client can auto-locate the headless orchestrator; surface discovered peers to the existing pairing/connect flow. Reuse the `reqwest::Client` pattern already in the file.
    - **Acceptance:** `cargo build --features tunnel` reaches `TailscaleDiscovery::*`; documented in `docs/DEPLOYMENT.md`.
    - **Effort:** M
  - **ERASE plan — Files:** delete `src/tailscale.rs`; remove `pub mod tailscale;` (`src/lib.rs:96`). Leave `src/tunnel/tailscale.rs` and `src/tunnel/mod.rs:24` untouched (that is the live `TailscaleTunnel`).
    - **Acceptance:** root `src/tailscale.rs` gone; `TailscaleTunnel` still wired; `grep -rn "TailscaleDiscovery" src/` empty; build green.
    - **Effort:** S
  - **Verification:** WIRE → `cargo build --features tunnel`; ERASE → `cargo check --features full` (ensure no confusion with `tunnel::tailscale`).

- [ ] **T9: [DECISION 6] Erase `qr_pairing` (recommended) OR wire-with-hardening.**
  - **ERASE plan — Files:** delete `src/qr_pairing.rs`; remove `pub mod qr_pairing;` (`src/lib.rs:83`). Live pairing stays in `src/pairing/store.rs`.
    - **Acceptance:** `src/qr_pairing.rs` gone; `grep -rn "qr_pairing::|PairingSession" src/` empty (the `validate_token` in `src/cli/tool.rs` is unrelated and remains); `PairingStore` path unchanged; build green.
    - **Effort:** S
  - **WIRE-with-hardening plan (only if operator overrides) — Files:** `src/qr_pairing.rs` + a pairing call site over `PairingStore`.
    - **Mandatory security fixes before wiring:** (1) replace `if self.info.pairing_token == token` (`qr_pairing.rs:219`) with constant-time compare using `subtle::ConstantTimeEq::ct_eq` — copy the pattern from `crates/thinclaw-gateway/src/web/auth.rs:120` / `src/orchestrator/auth.rs:61`; (2) replace the hand-rolled `base64_encode` (`qr_pairing.rs:94`) with `base64::engine::general_purpose::URL_SAFE_NO_PAD` (pattern at `crates/thinclaw-tools/src/wasm/oauth.rs:4`); (3) back token state with `PairingStore` instead of an in-memory `PairingSession.used` bool so one-time-use survives restarts.
    - **Acceptance:** constant-time compare + `base64` crate in use; tokens validated through `PairingStore`; QR flow reachable from a real surface; `docs/DEPLOYMENT.md` updated.
    - **Effort:** L
  - **Verification:** ERASE → `cargo check --features full && cargo clippy --all --tests -- -D warnings`; WIRE → add the security regression tests + `cargo test --features full`.

- [ ] **T10: Coordinate `RepairTask` with WS-05 (no edit here).**
  - **Files:** `crates/thinclaw-agent/src/self_repair.rs:325 RepairTask` (re-exported `src/agent/mod.rs:99`, zero constructors found). **WS-11 does not delete or wire this.**
  - **Change:** none — record the finding in WS-05's plan: `RepairTask` has no constructor and is dead today; its fate is tied to the self-repair `with_builder` wiring decision (WIRE if self-repair-rebuild is realized, ERASE if not). Add a back-reference in WS-05.
  - **Acceptance:** WS-05 doc lists `RepairTask` under its self-repair decision; WS-11 makes no `self_repair.rs` edit.
  - **Effort:** S (doc-only)
  - **Verification:** n/a (cross-WS note).

## Best Practices (workstream-specific)

- **Façade discipline (CLAUDE.md):** when erasing the `src/safety` orphans, the façade `mod.rs` (4 lines, `pub use thinclaw_safety::*;`) is the *correct* shape — leave it. Do not "fix" it by re-declaring submodules; that would resurrect the dead code.
- **Delete one, never "fix both copies":** mirror WS-10's rule. For drifted duplicates (`src/safety/*`), the cure is deletion of the stale copy, not syncing it — the audit's sanitizer/pii_redactor drift proves syncing is a perpetual tax.
- **Constant-time + real crypto crates:** if any token compare is touched (qr_pairing WIRE), copy `subtle::ConstantTimeEq::ct_eq` from `crates/thinclaw-gateway/src/web/auth.rs:120`, `src/orchestrator/auth.rs:61`, or `src/hooks/webhook_signing.rs:28`; copy `base64` usage from `crates/thinclaw-tools/src/wasm/oauth.rs:4`. Never hand-roll either.
- **Feature-gate, don't profile-default, for hardware/privacy features:** `voice_wake` must stay opt-in (`voice`), consistent with how `cpal`/mic capture is scoped to headless mode in the module doc and `Cargo.toml:337-339`.
- **Crate dependency direction (docs/CRATE_OWNERSHIP.md):** erases here are root-package and crate-local; no new root→crate or crate→root edges. Do not move behavior across the boundary in this sweep.
- **Verify the module tree, not file mtimes,** to prove deadness: a sibling `.rs` with no `mod`/`#[path]` declaration is unreachable regardless of timestamp.

## Common Pitfalls

- **Mistaking the live `src/tunnel/tailscale.rs::TailscaleTunnel` for the dead `src/tailscale.rs::TailscaleDiscovery`.** They share a name and a topic; only the root-level `src/tailscale.rs` is dead. Deleting the tunnel would break `src/tunnel/mod.rs:24`.
- **Assuming `src/safety/*.rs` is compiled because `crate::safety::skill_path::SkillPathConfig` is used at `src/skills/registry.rs:423`.** That path resolves through the façade glob to the **crate** (`thinclaw_safety::skill_path`), not the root orphan. Deleting the root files does not break it. Re-confirm with `grep "mod skill_path" src/safety/mod.rs` (empty) before doubting.
- **Removing `subscribe()` but leaving its now-unused imports** (`Sse`, `Event`, `KeepAlive`, `Infallible` at `sse.rs:3,8`) → `-D warnings` clippy failure. Strip them in the same edit.
- **Colliding with WS-10 on `src/extensions/manager.rs`** (T5) — it is a god-file WS-10 decomposes. Land the dead-helper deletion before/around WS-10's move, or rebase; do not both touch the same hunk.
- **Treating the DECIDE items as obvious erases.** `voice_wake` and `tailscale` are real vision per the realize-the-vision directive — default to WIRE and get sign-off before deleting. Only `qr_pairing` is recommended for erase among the three, and only because `PairingStore` already covers it.
- **Self_message false-comfort:** leaving the module wired-but-unfiltered (or deleting the doc but keeping the code) preserves a *claimed* guarantee with no enforcement — worse than a clean erase. Resolve it fully one way.
- **The `experiments.rs:5060` `#[ignore]`'d flaky test and other quarantined tests are NOT in scope** — do not "clean them up" here.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - **Wave A (parallel, no decisions, low risk):** four independent subagents run T1 (safety orphans), T2 (CLI modules), T3 (build.rs), T4 (gateway sse) simultaneously — disjoint files, no shared hunks.
  - **Wave B (after operator sign-off on Decision Points 3–6):** T5 (dead helpers — sequence around WS-10's manager.rs), T6 (self_message), T7 (voice_wake), T8 (tailscale), T9 (qr_pairing) as parallel subagents; each touches its own files. T10 is a doc-only note handed to WS-05.
  - **Sequencing constraint:** Wave B starts only after WS-10 has landed (or explicitly deferred) its `src/extensions/manager.rs` and decomposition work, to avoid T5 hunk collisions.
- **Isolation:** Wave A needs **no** worktree isolation (fully disjoint files). Wave B's T5 needs either a shared branch ordered after WS-10 or a git worktree if run truly concurrently with WS-10's manager.rs decomposition. T6–T9 are disjoint and can share one worktree.
- **Workflow shape:** `implement (fan-out Wave A) → verify-gate → review → operator sign-off on Decision Points → implement (fan-out Wave B) → verify-gate → review → fix`. Each DECIDE task (T7/T8) carries both a WIRE and ERASE branch; the orchestrator picks the branch from the sign-off and discards the other.
- **Verification gate (run per wave, all must pass):**
  - `cargo fmt --all -- --check`
  - `cargo clippy --all --benches --tests --examples --all-features -- -D warnings` (the audit flagged CI omits `--all-targets`; run the full target set locally to catch test/example regressions like the `await_holding_lock` in `secrets.rs:144`).
  - `cargo check --no-default-features --features edge` and `cargo check --features light` and `cargo check --features full` (profile matrix).
  - `cargo test -p thinclaw-gateway` (T4), `cargo test -p thinclaw-channels` (T6), root `cargo test` for CLI/extension touches.
  - `cargo deny check` after T7 ERASE (removing `cpal`) to confirm no stranded advisory and to verify the sweep does not regress the RUSTSEC state.
  - `cargo build --features voice` (T7 WIRE), `cargo build --features tunnel` (T8 WIRE), `cargo build --release --features bundled-wasm` (T3, if WASM toolchain present).
  - Run `/ship` for the consolidated Rust quality gate and `/code-review` (high) on the final diff.
  - **DB/Docker prerequisites:** none for the erases. None of WS-11's tasks require Postgres/libSQL/Docker. (Per CLAUDE.md, broader Postgres-backed integration tests need migrations applied, but they are out of scope here.)
- **Fan-out detail:** Wave A is pure deletion → safe to fan out 4-wide and merge sequentially; each subagent re-runs the gate on its own branch before merge to keep `main` green.

## Definition of Done

- [ ] T1–T5 landed: `src/safety/` is `mod.rs`-only; the 3 CLI modules, `build_telegram_channel`, `SseManager::subscribe` (+ its dead imports), and the 3 dead helpers are gone.
- [ ] Decision Points 3–6 explicitly resolved with operator sign-off; the chosen branch (WIRE or ERASE) of T6–T9 is implemented and the other discarded.
- [ ] `self_message` resolved one way (erased, or wired with a real ingest call site + `docs/CHANNEL_ARCHITECTURE.md` update).
- [ ] `voice_wake` resolved: either wired behind `voice` with a headless spawn site + `docs/BUILD_PROFILES.md` note, or fully erased (module + `voice` feature + `cpal`).
- [ ] `tailscale` discovery resolved: wired into deployment under `tunnel` + `docs/DEPLOYMENT.md`, or erased (root module only; `TailscaleTunnel` untouched).
- [ ] `qr_pairing` resolved: erased (recommended), or wired with constant-time compare + `base64` crate + `PairingStore`-backed one-time-use.
- [ ] T10 cross-note delivered to WS-05; WS-11 made no `self_repair.rs` edit.
- [ ] Verification gate green on the full profile matrix: `cargo fmt --check`, `cargo clippy --all --benches --tests --examples --all-features -D warnings`, `cargo check` for edge/light/full, targeted `cargo test`, `cargo deny check` (post-T7).
- [ ] `grep` confirms zero remaining references to every erased symbol.
- [ ] Docs updated for any WIRE (`BUILD_PROFILES.md`, `DEPLOYMENT.md`, `CHANNEL_ARCHITECTURE.md` as applicable); no stale references to erased modules remain.
- [ ] `/ship` and `/code-review` (high) pass on the final diff.
