# WS-01 — Security & CI Hardening

> **Status:** Not started · **Priority:** P0 · **Risk:** medium · **Effort:** L
> **Depends on:** none · **Blocks:** every other workstream (CI must be green for any PR to merge; the empty-token bypass and wasmtime advisory gate `main`)
> **Owns (symbols/files):** `deny.toml`; `.github/workflows/ci.yml` (clippy `--all-targets`); `crates/thinclaw-config/src/channel_config.rs` gateway `auth_token` mapping (lines 183–193); `src/channels/web/mod.rs` `GatewayChannel::new` (line 72); `crates/thinclaw-config/src/secrets.rs` tests; `src/sandbox/proxy/**` (`mod.rs`, `http.rs`, `allowlist.rs`); `crates/thinclaw-tools-core/src/url_guard.rs`; `crates/thinclaw-tools/src/wasm/wrapper.rs` `reject_private_ip`; `src/cli/oauth_defaults.rs` `wait_for_callback`; `crates/thinclaw-tools/src/builtin/execute_code.rs` `requires_approval`; `crates/thinclaw-tools/src/builtin/file.rs` `validate_path`; `crates/thinclaw-tools/src/builtin/shell.rs` scanner-health surfacing; `crates/thinclaw-tools/src/wasm/limits.rs` `WasmResourceLimiter`; **security docs** `src/NETWORK_SECURITY.md` and `docs/SECURITY.md`.
>
> Cargo.lock is shared infrastructure — WS-01 owns the wasmtime-wasi bump line specifically; coordinate any other lockfile change.

## Vision & Goal

ThinClaw's pitch is "security as architecture": trust boundaries enforced in code, not in prose. This workstream closes the gap between what `src/NETWORK_SECURITY.md` and `docs/SECURITY.md` *promise* and what the code *enforces*, and restores a green CI gate so every later workstream can land. It realizes the vision by making the half-built confinement primitives (the unused `with_credential_resolver` hook, the declared-but-unenforced WASM table/instance limits, the OAuth `state` that is generated but never checked) actually fire, rather than deleting them. The one genuinely dead default (HTTPS credential-injection mappings that can never trigger) is the only erase-candidate, and even that has a build-OOB alternative.

## Scope

**In scope:**
- CI gate restoration: bump `wasmtime-wasi` 36.0.10→36.0.11 (RUSTSEC-2026-0182), remove the 3 stale AWS-webpki ignores in `deny.toml`, fix the `deny.toml` header pointing at a nonexistent workflow, add `--all-targets` to both clippy invocations in `ci.yml`, and fix the `await_holding_lock` that surfaces.
- Empty `gateway_auth_token` auth-bypass close (config + channel constructor).
- Sandbox proxy credential confinement (SecretsStore-backed resolver + the HTTPS-injection decision).
- Security-layer coverage of `extra_public_routes` (WASM webhook routers).
- DNS-rebinding TOCTOU pin in the two URL guards.
- OAuth `state` validation on the loopback callback.
- `execute_code` host-posture hardening + documented asymmetry vs shell.
- Filesystem tool fail-closed when `base_dir` is `None`.
- Shell scanner health surfaced in `status`.
- WASM table/instance resource-limit enforcement.
- Aligning **only** `src/NETWORK_SECURITY.md` and `docs/SECURITY.md` to the code changes above.

**Out of scope (and which WS owns it):**
- libSQL FTS5 `MATCH` sanitization (Confirmed Bug #3) → **WS-02** (database).
- Discord Ed25519 verification + `split_message` UTF-8 panic in WASM channels (#4, #5) → WASM-channels workstream.
- Desktop cloud-sync wiring (#8), self-repair `with_builder` → their own workstreams.
- Inventory/drift docs (`CRATE_OWNERSHIP.md`, `FEATURE_PARITY.md`, README inventories) → **WS-12** (docs/drift). WS-01 only edits the two security docs it owns.

## Current State (verified)

- **wasmtime advisory (wired, broken).** `Cargo.lock:9518` pins `wasmtime-wasi 36.0.10`; `Cargo.lock:9260` pins `wasmtime 36.0.10`. `Cargo.toml:159-160` request `version = "36"` for both, so a precise `cargo update` is sufficient. `wasm-runtime` is enabled by the **default `light` profile** (`Cargo.toml:265`) and by `full`/`all-features`/`bundled-wasm`; `edge` (`Cargo.toml:258`, `libsql` only) does NOT pull wasmtime, so `cargo deny` is red on the default path. `cargo-deny 0.19.8` is installed locally.
- **deny.toml drift (drifted cruft).** `deny.toml:3` header: `# CI: .github/workflows/code_style.yml` — that workflow does **not exist** (`ls .github/workflows/` shows `ci.yml`, no `code_style.yml`); the real job is `codestyle` inside `ci.yml:22`. `deny.toml:22-24` ignores `RUSTSEC-2026-0098/0099/0104` with a comment claiming they are AWS-webpki transitives; the audit flags these as **stale advisory-not-detected** ignores (cargo-deny errors on ignores it can't match).
- **Empty-token bypass (half-wired; the safe path proves the bug).** `crates/thinclaw-config/src/channel_config.rs:183-193`: `auth_token` is `optional_env("GATEWAY_AUTH_TOKEN")?.or(settings...).map(|s| { trim... })` — it trims but **never filters empty**, so `""` → `Some("")`. Then `src/channels/web/mod.rs:72-81`: `GatewayChannel::new` does `config.auth_token.clone().unwrap_or_else(|| <random>)` — only `None` gets a random token; `Some("")` is kept verbatim, and the constant-time bearer compare then accepts an empty `Authorization: Bearer`. The **correct** pattern already exists at `src/platform/gateway_access.rs:27-29`: `env_string(...).or_else(...).filter(|token| !token.trim().is_empty())`. The bug is the `channel_config.rs` path failing to mirror that filter, plus the constructor not treating empty as absent.
- **`await_holding_lock` (latent; surfaces only with `--all-targets`).** `ci.yml:52` runs `cargo clippy --workspace -- -D warnings` and `ci.yml:121` runs the per-profile `cargo clippy --workspace ${{ matrix.cargo-args }} -- -D warnings` — **neither passes `--all-targets`**, so test/bench/example code escapes `-D warnings`. CLAUDE.md mandates `cargo clippy --all --benches --tests --examples --all-features`. The lint it hides: `crates/thinclaw-config/src/secrets.rs:143-162` test `env_source_uses_allowed_key` holds `let _guard = lock_env();` (a `std::sync::MutexGuard<'static,()>`, def at `crates/thinclaw-config/src/helpers.rs:25`) across `SecretsConfig::resolve(&settings).await` at lines 154-155. Same shape at `env_source_requires_explicit_allowance` (133-134) and `short_master_key_is_rejected` (176-177).
- **Sandbox proxy credential confinement (built, never wired).** `src/sandbox/proxy/mod.rs:96-99` exposes `with_credential_resolver` — but it has **zero callers** (grep confirms only the def + the two internal `Arc::new(EnvCredentialResolver)` defaults at lines 62 and 72). `NetworkProxyBuilder::from_config` (line 68) always uses `EnvCredentialResolver` (`src/sandbox/proxy/http.rs:53-60`, reads `std::env::var`). The only caller, `src/sandbox/manager.rs:165`, never injects a store-backed resolver. So the documented "resolved … from the encrypted secrets store" (NETWORK_SECURITY.md:343) is false — it reads process env. The `SecretsStore` trait (`crates/thinclaw-secrets/src/store.rs:34`, `async fn get(&self, user_id, name)`) is the intended source.
- **HTTPS credential-injection gap (dead defaults).** `src/sandbox/proxy/http.rs:350-388` (`forward_request`) injects credentials — but only on the **plaintext-HTTP forward path**. HTTPS goes through `handle_connect` (`http.rs:206-207`, `249`), which the doc-comment at `http.rs:245-248` says cannot inject ("not possible through CONNECT tunnels … without MITM"). Every shipped default mapping is HTTPS: `default_credential_mappings()` at `src/sandbox/config.rs:8-14` = `api.openai.com`, `api.anthropic.com`, `api.near.ai`. So the `AllowWithCredentials` injection at `http.rs:351` **never fires for any default** — it is reachable only for a hypothetical plaintext-HTTP host an operator manually adds.
- **`extra_public_routes` escape the layer stack (half-wired).** `src/channels/web/server.rs:1445-1460` builds `app` with `DefaultBodyLimit::max(1 MB)`, `cors`, `X_CONTENT_TYPE_OPTIONS: nosniff`, `X_FRAME_OPTIONS: DENY`. Lines 1462-1467 then `app.merge(routes)` for each `extra_public_routes` (WASM channel webhook endpoints) — **after** the `.layer(...)` calls, so axum's outermost-layer ordering means those routes inherit none of body-limit/CORS/nosniff/frame-options.
- **DNS-rebinding TOCTOU (validate-only, no pin) — two copies.** `crates/thinclaw-tools-core/src/url_guard.rs:81-92` resolves `host:port` via `to_socket_addrs()`, rejects private IPs, but **returns the original `parsed: Url`** (line 94). Callers (`crates/thinclaw-tools/src/builtin/http.rs:104`, `extract_document.rs:133`, `mcp/config.rs:253`) then hand the URL string to `reqwest`, which **re-resolves at connect** — classic TOCTOU. The second copy is `crates/thinclaw-tools/src/wasm/wrapper.rs:1510-1564` (`reject_private_ip`): same resolve-then-discard, used by the WASM HTTP host. Neither pins the validated IP.
- **OAuth `state` not validated on loopback (half-wired; good copy exists).** `src/cli/oauth_defaults.rs:142-149` `auth_url(state, code_challenge)` puts `state` in the URL, but `wait_for_callback(listener, path_prefix, param_name, display_name)` (line 303) extracts only `param_name` (e.g. `"code"`, the caller in `src/cli/tool.rs:761`; also used by `src/tauri_commands.rs:595`) and **never reads or compares `state`**. The correct pattern is right next door: `crates/thinclaw-tools/src/mcp/auth.rs:810-867` `wait_for_authorization_callback` takes `expected_state: Option<&str>` and rejects on mismatch (lines 858-867).
- **`execute_code` host posture (wired, under-protected).** `crates/thinclaw-tools/src/builtin/execute_code.rs:914-916` `requires_approval` returns `ApprovalRequirement::UnlessAutoApproved` **unconditionally**. `ExecuteCodeTool::new()` defaults `backend: LocalHostExecutionBackend::shared()` (line 292) — the **bare host**, no sandbox. `self.backend.kind()` already distinguishes `ExecutionBackendKind::{DockerSandbox, LocalHost, RemoteRunnerAdapter}` (used at lines 441, 464, 860-869). `ApprovalRequirement::Always` exists (`crates/thinclaw-tools-core/src/tool.rs:21`). The shell tool already escalates to `Always` for dangerous commands (`shell.rs:681`+, `requires_explicit_approval`), so the asymmetry is real and undocumented.
- **Filesystem fail-open when `base_dir` is None (wired, under-protected).** `crates/thinclaw-tools/src/builtin/file.rs:130-196` `validate_path`: when `base_dir` is `Some`, it enforces containment (lines 150-192). When `base_dir` is `None` (lines 142-147) it joins the path onto `current_dir()` with **no containment check at all** — the `if let Some(base)` guard at line 150 is skipped, so any absolute path or `..` traversal is allowed. `effective_base_dir` (line 225) returns `None` when neither metadata `tool_base_dir` nor a configured base is set. Every file tool (`ReadFileTool`/`WriteFileTool`/etc., constructed with optional `with_base_dir` at lines 281, 414, 583, 783, 929) inherits this.
- **Shell scanner fail-open (wired, silent).** `crates/thinclaw-tools/src/builtin/shell.rs:496-501`: when the external scanner returns `Unknown` and mode != `FailClosed`, it logs `warn!("External shell scanner unavailable in fail-open mode")` and proceeds. Default mode is `FailOpen` (`shell.rs:237`). There is no operator-visible signal that the scanner is degraded; `src/cli/status.rs:12` `run_status_command` prints subsystem health but has no scanner line.
- **WASM table/instance limits declared-but-unenforced (built, inert).** `crates/thinclaw-tools/src/wasm/limits.rs:62-78`: `WasmResourceLimiter` carries `max_tables`, `tables_created`, `max_instances`, `instances_created` — the two `_created` counters are `#[allow(dead_code)] // Reserved for … enforcement` (lines 71, 76). The `ResourceLimiter` impl enforces memory (`memory_growing`, 108-134) and a hardcoded 10_000 table-cap (`table_growing`, 136-152, **ignores `max_tables`**), and `instances()`/`tables()`/`memories()` (154-165) report caps but the counters never increment, so per-store accumulation is unenforced.

## Decision Points

1. **HTTPS credential injection — build OOB delivery vs erase dead defaults (Finding #7).**
   - *Option A (erase):* delete the three HTTPS default mappings from `src/sandbox/config.rs:12-14`, keep the HTTP-only `forward_request` path for operator-added plaintext hosts, and update NETWORK_SECURITY.md to state plainly that credentials reach containers **only** via the orchestrator `/worker/{id}/credentials` endpoint (already documented at NETWORK_SECURITY.md:223-231, 339). Lowest risk, ~30 min.
   - *Option B (build):* implement out-of-band credential delivery for HTTPS hosts (resolve the mapping at CONNECT-decision time and write the secret into the container's `/worker/{id}/credentials` response), so the documented guarantee fires for the defaults. Larger, touches the orchestrator credential-grant flow.
   - **Recommendation: A (erase the dead HTTPS defaults) for this WS, because the secure OOB path the doc already references is the real mechanism and the in-proxy HTTPS injection is architecturally impossible without MITM.** Keep `with_credential_resolver` + the HTTP `forward_request` path alive (still valuable for plaintext/internal hosts) — that is Finding #6, which we WIRE. File a follow-up note for Option B if operators ever need transparent HTTPS injection.

2. **`execute_code` `Always` vs feature-gate (Finding, §8).** Force `ApprovalRequirement::Always` when `backend.kind() == LocalHost`, vs gating bare-host execution off behind a feature.
   - **Recommendation: force `Always` on `LocalHost` (and `RemoteRunnerAdapter` if it lacks isolation), keep `UnlessAutoApproved` for `DockerSandbox`.** This realizes the capability (code execution stays available) while making bare-host runs a deliberate per-invocation operator decision, matching the shell tool's escalation pattern. Do not feature-gate — that removes a working capability.

3. **Filesystem `base_dir == None` — hard error vs default to cwd-containment (§Finding 9).**
   - **Recommendation: fail-closed — when `base_dir` is `None`, treat `current_dir()` as the implicit base and enforce containment against it (reject absolute paths and `..` escapes), rather than returning a blanket error.** This keeps the no-config dev path working (files under cwd) while removing the unbounded-traversal hole. A hard `NotAuthorized` on every call when unconfigured would break the default CLI; containment-against-cwd is the minimal fail-closed posture. Add a `with_base_dir`-style explicit opt-out only if a real caller needs full-FS access (none found).

4. **WASM limits — enforce vs delete the reserved counters (§Finding 11; audit lists this under WIRE).**
   - **Recommendation: WIRE — increment `tables_created`/`instances_created` and enforce `max_tables`/`max_instances` in `table_growing`/instance creation, removing the `#[allow(dead_code)]`.** The fields exist and the audit explicitly classifies this as built-but-disconnected. Erasing would weaken sandbox posture for no benefit.

## Tasks

Ordered so the CI gate goes green first (unblocks merging), then the auth bypass, then confinement.

- [ ] **T1: Bump wasmtime-wasi to 36.0.11 (RUSTSEC-2026-0182).**
  - **Files:** `Cargo.lock` (wasmtime-wasi entry at line 9518; bump `wasmtime` at 9260 too if the resolver requires lockstep).
  - **Change:** `cargo update -p wasmtime-wasi --precise 36.0.11`. If cargo refuses due to the shared 36.x minor across `wasmtime`, also `cargo update -p wasmtime --precise 36.0.11`. Do NOT touch `Cargo.toml` (`version = "36"` already permits it). Verify both land at 36.0.11 in the lockfile.
  - **Acceptance:** `cargo deny check advisories` no longer reports RUSTSEC-2026-0182; `light`/`full`/`all-features` profiles still compile.
  - **Effort:** S
  - **Verification:** `cargo update -p wasmtime-wasi --precise 36.0.11 && cargo deny check 2>&1 | tail -20 && cargo check --workspace`

- [ ] **T2: Clean up `deny.toml` (drift + stale ignores).**
  - **Files:** `deny.toml` (line 3 header; lines 10-25 ignore block).
  - **Change:** (a) Replace line 3 `# CI: .github/workflows/code_style.yml` with `# CI: .github/workflows/ci.yml (codestyle job)`. (b) Remove the `RUSTSEC-2026-0098/0099/0104` entries (lines 22-24) and the now-orphaned AWS-webpki comment (lines 11-21). Re-run `cargo deny` first to confirm cargo-deny no longer detects these advisories (they are flagged stale/advisory-not-detected); if any are still genuinely matched, keep only the matched ones and shrink the comment. Leave the `[licenses]`/`[bans]`/`[sources]` sections untouched.
  - **Acceptance:** `cargo deny check` passes with no `advisory-not-detected` warnings and no unused-ignore complaints.
  - **Effort:** S
  - **Verification:** `cargo deny check 2>&1 | tail -30` (expect clean exit 0).

- [ ] **T3: Add `--all-targets` to CI clippy, then fix the `await_holding_lock`.**
  - **Files:** `.github/workflows/ci.yml:52` and `:121`; `crates/thinclaw-config/src/secrets.rs:121-179` (the three lock-holding tests).
  - **Change:** (a) `ci.yml:52` → `cargo clippy --workspace --all-targets --all-features -- -D warnings` (match CLAUDE.md). `ci.yml:121` → `cargo clippy --workspace --all-targets ${{ matrix.cargo-args }} -- -D warnings`. (b) Fix `await_holding_lock`: restructure each affected test so the `lock_env()` guard does not cross `.await`. The clean fix is to drop the guard before the await once env mutation is done is **not** possible here (the env must stay set during `resolve`), so instead make `lock_env()` usage block-scoped around a synchronous setup and pass values, OR switch these tests to a `tokio::sync::Mutex`-based async env guard, OR (simplest, matching repo precedent) add a targeted `#[allow(clippy::await_holding_lock)]` on each test with a comment explaining the env-serialization invariant — choose the targeted allow only if no async-lock helper exists. Prefer converting to an async-aware serialization primitive if one is already used elsewhere in `thinclaw-config` tests; otherwise the scoped `#[allow]` with justification is acceptable for test-only code.
  - **Acceptance:** `cargo clippy --all-targets --all-features -- -D warnings` is clean across the workspace; the three secrets tests still pass.
  - **Effort:** M
  - **Verification:** `cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test -p thinclaw-config secrets`

- [ ] **T4: Close the empty `gateway_auth_token` auth bypass.**
  - **Files:** `crates/thinclaw-config/src/channel_config.rs:183-193`; `src/channels/web/mod.rs:72-81`.
  - **Change:** (a) In `channel_config.rs`, after the trim `.map(...)`, chain `.filter(|trimmed| !trimmed.is_empty())` so an empty/whitespace-only token becomes `None` — mirror `src/platform/gateway_access.rs:27-29` exactly. (b) In `GatewayChannel::new`, make empty defensive too: `config.auth_token.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| <random>)` so a `Some("")` arriving from any other path still gets a random token. Add a `tracing::warn!` when an empty token is replaced.
  - **Acceptance:** With `GATEWAY_AUTH_TOKEN=""`, the gateway generates a random token (not empty); an empty `Authorization: Bearer` is rejected. Add a unit test in `channel_config.rs` tests asserting empty → `None`, and a test in `web/mod.rs` (or wherever `GatewayChannel::new` is testable) asserting `auth_token` is non-empty.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw-config gateway && cargo test -p thinclaw web` (root crate) ; manual: `GATEWAY_AUTH_TOKEN="" cargo run -- ...` shows a generated token.

- [ ] **T5: Wire a SecretsStore-backed CredentialResolver (Finding #6).**
  - **Files:** `src/sandbox/proxy/http.rs` (new `StoreCredentialResolver` next to `EnvCredentialResolver` at lines 53-70); `src/sandbox/proxy/mod.rs` (export it; optionally a `from_config_with_store` constructor); `src/sandbox/manager.rs:165` (inject via the existing `with_credential_resolver` hook at `proxy/mod.rs:96`).
  - **Change:** Add `pub struct StoreCredentialResolver { store: Arc<dyn SecretsStore>, user_id: String }` implementing `CredentialResolver::resolve(name)` by calling `store.get(&user_id, name).await` (`crates/thinclaw-secrets/src/store.rs:43`) and returning the decrypted value (map `SecretError` → `None` with a `debug!`, never log the value). Thread the `SecretsStore` (and `user_id`) into the sandbox manager's proxy construction at `manager.rs:164-167` via `.with_credential_resolver(Arc::new(StoreCredentialResolver{...}))`. Fall back to `EnvCredentialResolver` only when no store is configured.
  - **Acceptance:** When a sandbox starts with secrets enabled, the proxy resolves credentials from the AES-256-GCM store, not process env. Unit test: a `StoreCredentialResolver` over a mock `SecretsStore` returns the stored value; with no store, the env resolver is used.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw sandbox` (or the root test path for `src/sandbox`); `cargo clippy --all-targets`.

- [ ] **T6: Resolve the HTTPS credential-injection gap (Finding #7, Decision 1).**
  - **Files:** `src/sandbox/config.rs:8-14` (`default_credential_mappings`); `src/NETWORK_SECURITY.md:339-352`.
  - **Change (recommended Option A — erase dead defaults):** Remove the three HTTPS default mappings (`api.openai.com`, `api.anthropic.com`, `api.near.ai`) since in-proxy injection cannot fire for HTTPS. Keep the HTTP `forward_request` injection path (T5's resolver still feeds it for any operator-added plaintext host). Update NETWORK_SECURITY.md to: (1) correct line 343 ("resolved … from the encrypted secrets store") so it scopes to the HTTP-only path and the store-backed resolver from T5, and (2) state that HTTPS credential delivery is via the orchestrator `/worker/{id}/credentials` OOB endpoint (already at lines 223-231, 335). Leave `forward_request`'s `AllowWithCredentials` branch intact.
  - **Acceptance:** No default mapping is unreachable; doc no longer overclaims HTTPS in-proxy injection. (If Option B is chosen instead, implement OOB delivery and document it — larger, see Decision 1.)
  - **Effort:** S (Option A) / L (Option B)
  - **Verification:** `cargo test -p thinclaw sandbox`; doc lint by reading NETWORK_SECURITY.md §5.

- [ ] **T7: Apply the security-layer stack to `extra_public_routes` (Finding, §P1).**
  - **Files:** `src/channels/web/server.rs:1445-1467`.
  - **Change:** Merge the `extra_public_routes` into the router **before** the `.layer(DefaultBodyLimit)/.layer(cors)/.layer(nosniff)/.layer(X_FRAME_OPTIONS)` chain (lines 1450-1459), so the WASM webhook routers inherit them; or, if those routers must keep distinct state, wrap them in their own `Router` that re-applies the same four layers before merging. Preserve the `.with_state()` ordering comment's intent (Router<()> compatibility) by applying layers to the combined router. Keep CORS origins logic untouched.
  - **Acceptance:** A request to a WASM webhook route over the gateway is subject to the 1 MB body limit, CORS, `X-Content-Type-Options: nosniff`, and `X-Frame-Options: DENY`. Add/extend a server test that hits an extra public route and asserts the `nosniff` header is present and an over-limit body is rejected.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw web` (server tests); `cargo clippy --all-targets`.

- [ ] **T8: Pin the validated IP in both URL guards (DNS-rebinding TOCTOU).**
  - **Files:** `crates/thinclaw-tools-core/src/url_guard.rs:16-95` (+ callers `crates/thinclaw-tools/src/builtin/http.rs:104`, `extract_document.rs:133`, `mcp/config.rs:253`); `crates/thinclaw-tools/src/wasm/wrapper.rs:1510-1564`.
  - **Change:** Have `validate_outbound_url` return the resolved `SocketAddr`(s) it already computed (lines 82-92) alongside the `Url`, and have callers build the `reqwest::Client` with `.resolve(host, pinned_addr)` (or use `ClientBuilder::resolve_to_addrs`) so reqwest connects to the IP that passed validation — eliminating the re-resolve TOCTOU. Mirror in `wrapper.rs::reject_private_ip` for the WASM HTTP host. Keep the existing private-IP rejection (`is_disallowed_ip`/`is_private_ip`) as the source of truth. Preserve the `OutboundUrlGuardOptions` API; add the pinned-addr as an additional return field rather than a breaking signature change if callers are numerous.
  - **Acceptance:** A hostname that resolves to a public IP at validation but a private IP at connect is blocked (the pinned public IP is used, and if that IP later moves there is no second resolution). Add a unit test using a fake resolver/`resolve_to_addrs` to assert the pinned address is used.
  - **Effort:** L
  - **Verification:** `cargo test -p thinclaw-tools-core url_guard && cargo test -p thinclaw-tools wasm`; `cargo clippy --all-targets`.

- [ ] **T9: Validate OAuth `state` on the loopback callback.**
  - **Files:** `src/cli/oauth_defaults.rs:303-378` (`wait_for_callback`), callers `src/cli/tool.rs:761` and `src/tauri_commands.rs:595`; `auth_url` at `:142`.
  - **Change:** Add an `expected_state: Option<&str>` parameter to `wait_for_callback` and reject (return `OAuthCallbackError::Denied`/a new `StateMismatch`) when the callback query's `state` != expected — port the comparison from `crates/thinclaw-tools/src/mcp/auth.rs:858-867`. Generate a random `state` (e.g. `uuid::Uuid::new_v4()`, as `src/extensions/manager.rs:1906` already does) at the call sites, pass it into `auth_url(state, ...)`, and pass the same value as `expected_state`. Use a constant-time compare if the repo's helper is readily available; a plain `==` matches the MCP precedent and is acceptable for a CSRF nonce.
  - **Acceptance:** A callback with a missing or mismatched `state` is rejected; the happy path still returns the code. Add a unit test mirroring the MCP auth tests.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw oauth` (root crate cli tests); manual OAuth flow smoke if a provider is configured.

- [ ] **T10: Force `Always` approval for bare-host `execute_code` (Decision 2) + document asymmetry.**
  - **Files:** `crates/thinclaw-tools/src/builtin/execute_code.rs:914-916`; `docs/SECURITY.md`.
  - **Change:** Rewrite `requires_approval` to branch on `self.backend.kind()`: `DockerSandbox` → `UnlessAutoApproved` (unchanged), `LocalHost` → `Always`, `RemoteRunnerAdapter` → `Always` unless the adapter advertises isolation (default `Always`). Add a `docs/SECURITY.md` subsection documenting that `execute_code` runs on the bare host when no sandbox backend is attached and therefore demands explicit approval, contrasting with the shell tool's per-command escalation (`shell.rs:681`+).
  - **Acceptance:** With the default `LocalHostExecutionBackend`, `requires_approval` returns `Always`; with a `DockerSandbox` backend it returns `UnlessAutoApproved`. Unit test asserting both.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw-tools execute_code`; `cargo clippy --all-targets`.

- [ ] **T11: Fail-closed filesystem tools when `base_dir` is None (Decision 3).**
  - **Files:** `crates/thinclaw-tools/src/builtin/file.rs:130-196` (`validate_path`).
  - **Change:** When `base_dir` is `None`, treat `current_dir()` as the implicit containment base: compute the joined+normalized path (as today, lines 143-146) **and** run the same `starts_with(base_canonical)` containment check (lines 150-192) against the canonical cwd, rejecting absolute paths and `..` escapes with `ToolError::NotAuthorized`. Factor the containment block so both the `Some` and `None`-fallback paths share it. Keep the explicit-`base_dir` behavior unchanged.
  - **Acceptance:** With no configured base, reading `/etc/passwd` or `../../secret` is rejected; reading `./file_in_cwd` succeeds. Unit tests for both escape and in-cwd cases.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw-tools file`; `cargo clippy --all-targets`.

- [ ] **T12: Surface shell-scanner health in `status` (fail-closed becomes deliberate).**
  - **Files:** `src/cli/status.rs:12` (`run_status_command`); read-only accessors on `crates/thinclaw-tools/src/builtin/shell.rs` (`scanner.mode()` already exists, line 263/485).
  - **Change:** Add a "Shell scanner" line to `run_status_command` reporting whether an external scanner is configured, its mode (`FailOpen`/`FailClosed`), and whether it is currently reachable (a lightweight health probe or last-known verdict). Make clear in the output that `FailOpen` means a degraded scanner does **not** block commands, so operators can choose `FailClosed` deliberately. Do not change the runtime fail-open default (that is an operator policy choice); only make it visible. If `status.rs` lacks a handle to the shell tool/config, surface the configured `external_scanner_mode` from settings instead.
  - **Acceptance:** `thinclaw status` shows scanner mode and reachability; the existing fail-open path at `shell.rs:496-501` is unchanged but now discoverable.
  - **Effort:** M
  - **Verification:** `cargo run -- status` (manual); `cargo test -p thinclaw status` if status tests exist; `cargo clippy --all-targets`.

- [ ] **T13: Enforce WASM table/instance resource limits (Decision 4, Finding #11).**
  - **Files:** `crates/thinclaw-tools/src/wasm/limits.rs:62-166`.
  - **Change:** Remove the `#[allow(dead_code)]` on `tables_created`/`instances_created` (lines 71, 76). In `table_growing`, increment `tables_created` and reject when it would exceed `max_tables` (replace/augment the hardcoded 10_000 check at lines 143-149 to honor `self.max_tables`). Enforce `max_instances` analogously where instances are created (the `ResourceLimiter::instances()` cap at 154-156 reports the limit; confirm wasmtime calls it per-instantiation — if not, track instantiation count in the store wrapper that owns the limiter). Keep the Component-Model accommodation comment (lines 83-84) — the default of 10 must remain large enough for WASI adapters.
  - **Acceptance:** A module attempting to create more than `max_tables` tables or `max_instances` instances is denied; legitimate component-model modules still load. Unit test exercising the table cap.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw-tools wasm::limits`; load a real packaged WASM tool to confirm no regression (`./scripts/build-all.sh` already builds artifacts); `cargo clippy --all-targets`.

- [ ] **T14: Final security-doc alignment + full gate.**
  - **Files:** `src/NETWORK_SECURITY.md`, `docs/SECURITY.md` (consolidate edits from T6, T10; add brief notes for T4 empty-token, T7 layer coverage, T8 IP-pinning, T9 OAuth state, T11 fs containment, T13 WASM limits where the doc makes a claim about each).
  - **Change:** Sweep both docs so every guarantee now matches enforced behavior. Do NOT touch inventory/drift docs (WS-12) or `FEATURE_PARITY.md`.
  - **Acceptance:** No security guarantee in either doc is contradicted by code; references cite real file paths.
  - **Effort:** S
  - **Verification:** Manual read-through against the changed code; `cargo fmt --all && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo deny check`.

## Best Practices (workstream-specific)

- **Mirror the already-correct sibling, don't reinvent.** Every "half-wired" finding here has a correct twin in the repo: empty-token filter → `src/platform/gateway_access.rs:27-29`; OAuth state → `crates/thinclaw-tools/src/mcp/auth.rs:858-867`; private-IP rejection → `is_private_ip` in `wrapper.rs`. Copy the proven pattern verbatim.
- **Constant-time for secret compares; plain `==` only for non-secret nonces.** The bearer/HMAC compares are already constant-time (NETWORK_SECURITY.md §1). The OAuth `state` is a CSRF nonce, not a secret, so the MCP-path `==` is acceptable; match precedent.
- **Never log secret values.** The `StoreCredentialResolver` (T5) must map errors to `None` with a `debug!` that names only the secret *name*, never the value — `Secret`'s `Debug` is already redacted (`crates/thinclaw-secrets`).
- **Respect crate dependency direction.** `thinclaw-tools-core` (T8 `url_guard`) must not gain a dep on `thinclaw-tools` or root; keep the pinned-addr type in `-core`. Resolvers that need `SecretsStore` live in root `src/sandbox` (which already depends on `thinclaw-secrets`), not pushed down into a leaf crate.
- **Feature-matrix awareness.** T1/T13 touch `wasm-runtime` (light/full/all-features/bundled, NOT edge). T5/T6 touch sandbox code present in `desktop`/`full`. Run clippy across at least `light`, `edge`, and `all-features` before declaring done.
- **Test the boundary, not just the happy path.** Each security fix needs a negative test (empty token rejected, traversal rejected, mismatched state rejected, rebinding blocked) — the audit's whole point is that positive-only tests hid these holes.

## Common Pitfalls

- **Fixing only one of N copies.** The audit explicitly calls out that the `split_message` fix landed in 1 of 4 WASM channels. Two findings here are duplicated: the DNS-rebinding TOCTOU exists in **both** `url_guard.rs` and `wrapper.rs::reject_private_ip` (T8 must hit both); the empty-token fix needs **both** `channel_config.rs` and `web/mod.rs` (T4). Grep before declaring done.
- **`--all-targets` surfaces *new* lints, not just the known one.** Adding `--all-targets` (T3) will lint all test/bench/example code workspace-wide for the first time; expect more than the single `await_holding_lock`. Run it locally and fix the full set before pushing, or CI will go red on the very change meant to harden it.
- **axum layer ordering is outermost-first and applies only to routes present when `.layer` is called.** T7's bug is precisely that `merge` after `.layer` leaves new routes unwrapped. Merge first, then layer (or wrap the extra routes separately).
- **`cargo update --precise` can drag the whole 36.x tree.** If `wasmtime` and `wasmtime-wasi` are version-locked, updating one without the other fails resolution; bump both to 36.0.11 (T1).
- **Don't delete `with_credential_resolver` (Finding #6 erase-temptation).** It is the hook that makes T5 possible; the audit's "or document + delete hook" alternative is explicitly the worse option for the vision — WIRE it.
- **Don't over-tighten the filesystem fail-close.** A blanket `NotAuthorized` when `base_dir` is `None` breaks the default CLI; contain against cwd instead (Decision 3).
- **Don't widen public APIs to compile.** T8's pinned-addr return belongs behind the existing `validate_outbound_url` surface; keep new visibility at `pub(crate)`/`pub(super)` unless a cross-crate caller truly needs it.

## Multi-Worker Execution Plan (ultracode)

- **Worker decomposition:**
  - **Sequential lead-in (single worker, must land first):** T1 → T2 → T3. These restore the CI gate and unblock every later PR. T3 depends on T1/T2 being clean so the new `--all-targets` run isn't masking unrelated red.
  - **Parallel fan-out after the gate is green (independent files, safe to parallelize):**
    - Worker A — auth/web: T4 (config + web/mod.rs), T7 (web/server.rs). Both live in gateway/web; co-locate to avoid merge churn.
    - Worker B — sandbox: T5, T6 (`src/sandbox/proxy/**`, `config.rs`, NETWORK_SECURITY.md §5). Single owner of the proxy.
    - Worker C — tools-core/url + WASM host: T8 (`url_guard.rs` + callers + `wrapper.rs`), T13 (`wasm/limits.rs`).
    - Worker D — tool builtins: T9 (oauth), T10 (execute_code), T11 (file.rs), T12 (status). Mostly disjoint files.
  - **Sequential close-out (single worker):** T14 doc alignment + full-gate sweep, after A–D merge.
- **Isolation:** Yes — use a git worktree per parallel worker (A/B/C/D) to avoid concurrent mutation of `Cargo.lock` and shared crates. The lead-in (T1-T3) must merge to the WS-01 base branch before fan-out starts, since everyone rebases onto the new lockfile + clippy config. Workers touch disjoint files by design (see decomposition); the only shared files are the two security docs, which are consolidated by T14 to avoid doc merge conflicts — earlier tasks leave inline `// DOC: ...` TODO markers instead of editing the docs directly.
- **Workflow shape:** `implement → verify → review → fix` per worker.
  1. *Implement* the worker's tasks in its worktree.
  2. *Verify* with the per-crate gate (below).
  3. *Review* via `/code-review` at `high` (security-sensitive — uncertain findings welcome).
  4. *Fix* and re-verify. Fan-out is A/B/C/D in parallel; rejoin at T14 which runs the full-workspace gate once.
- **Verification gate (exact commands):**
  - Formatting: `cargo fmt --all -- --check`
  - Lint (the whole point of T3): `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - Per-crate tests: `cargo test -p thinclaw-config`, `cargo test -p thinclaw-tools`, `cargo test -p thinclaw-tools-core`, and root `cargo test -p thinclaw <module>` for web/sandbox/cli/oauth.
  - Advisory gate: `cargo deny check`
  - Profile breadth: `cargo check --workspace --no-default-features --features edge` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` (covers light/full via all-features).
  - `/ship` to run the full fmt+clippy+test gate before each PR; `/code-review --comment` on the security-critical PRs (T4, T5, T8, T9).
  - **DB/Docker prerequisites:** None for the unit-level tests here. T13's real-WASM regression check wants `./scripts/build-all.sh` artifacts present. Sandbox proxy integration (T5/T6) does not require Docker for the unit tests (mock `SecretsStore`); only a full end-to-end sandbox smoke would need Docker — keep that manual/`#[ignore]`.

## Definition of Done

- [ ] `cargo deny check` exits 0 with no RUSTSEC-2026-0182 and no stale/unused ignores; `deny.toml` header points at the real `ci.yml` codestyle job.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean; `ci.yml:52` and `:121` both pass `--all-targets`; the `await_holding_lock` in `thinclaw-config` is resolved.
- [ ] `GATEWAY_AUTH_TOKEN=""` no longer authenticates an empty Bearer — fixed in **both** `channel_config.rs` and `web/mod.rs`, with negative tests.
- [ ] The sandbox proxy resolves credentials from the `SecretsStore` via `with_credential_resolver` (not process env); dead HTTPS default mappings are removed (or OOB delivery implemented per Decision 1) — and NETWORK_SECURITY.md §5 matches.
- [ ] WASM webhook (`extra_public_routes`) requests are covered by body-limit/CORS/nosniff/frame-options, with a test.
- [ ] Both URL guards (`url_guard.rs` and `wrapper.rs`) pin the validated IP; a rebinding test passes.
- [ ] The loopback OAuth callback validates `state`, with a mismatch-rejection test.
- [ ] `execute_code` returns `Always` on bare-host backends; `docs/SECURITY.md` documents the shell-vs-execute_code asymmetry.
- [ ] Filesystem tools reject traversal/absolute escapes when `base_dir` is `None` (contained to cwd), with tests.
- [ ] `thinclaw status` surfaces shell-scanner mode and reachability.
- [ ] WASM table/instance limits are enforced (`#[allow(dead_code)]` removed), with a cap test; real packaged tools still load.
- [ ] All Decision Points (1–4) are resolved with the recommended option recorded in the PR description.
- [ ] `src/NETWORK_SECURITY.md` and `docs/SECURITY.md` updated in the same branch; inventory/drift docs left to WS-12.
