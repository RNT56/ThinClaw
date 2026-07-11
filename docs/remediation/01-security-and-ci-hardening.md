# WS-01 — Security & CI Hardening

> **Status:** ✅ Landed (2026-06-23), commits `4f88c43e` (Wave 0: security & CI hardening) and `29188003` (activate the fixes at their runtime call sites). All tasks shipped **except T11**, which was resolved by a different decision than originally recommended: filesystem tools without a configured `base_dir` run in an explicit, warned unrestricted trusted-operator mode rather than cwd-containment (see T11 / Decision 3 below). This plan is complete; do not re-execute it.
> **Priority:** P0 · **Risk:** medium · **Effort:** L
> **Depends on:** none · **Blocks:** every other workstream (CI must be green for any PR to merge; the empty-token bypass and wasmtime advisory gated `main`)
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

## Current State (fixed)

Every item below described a bug or gap that has since been closed. The original finding is preserved for context; the resolution follows.

- **wasmtime advisory — FIXED.** Was: `wasmtime`/`wasmtime-wasi` pinned `36.0.10` (RUSTSEC-2026-0182), red on the default `wasm-runtime` path. Now: the root and extension manifests request `36.0.12`, `Cargo.lock` resolves both crates at `36.0.12`, and `cargo deny check advisories` no longer flags the advisory.
- **deny.toml drift — FIXED.** Was: `deny.toml:3` header pointed at a nonexistent `code_style.yml` and lines 22-24 ignored `RUSTSEC-2026-0098/0099/0104` as stale advisory-not-detected entries. Now: the header reads `# CI: .github/workflows/ci.yml (codestyle job)` (`deny.toml:3`) and `[advisories] ignore = []` — no stale ignores (`deny.toml:13`).
- **Empty-token bypass — FIXED in both halves.** Was: `channel_config.rs` trimmed but never filtered, so `GATEWAY_AUTH_TOKEN=""` became `Some("")` and `GatewayChannel::new` kept it verbatim, letting an empty `Authorization: Bearer` authenticate. Now: `crates/thinclaw-config/src/channel_config.rs:200` chains `.filter(|token| !token.trim().is_empty())`, and `src/channels/web/mod.rs:122` applies the same filter before the random-token fallback, with regression tests (`gateway_new_replaces_empty_auth_token_with_random`, `gateway_new_preserves_configured_auth_token`).
- **`await_holding_lock` / missing `--all-targets` — FIXED.** Was: neither CI clippy invocation passed `--all-targets`, hiding an `await_holding_lock` in the `thinclaw-config` secrets tests. Now: `ci.yml:66` and `ci.yml:135` both pass `--all-targets`, and the three lock-holding tests carry a targeted `#[allow(clippy::await_holding_lock)]` with a comment explaining the env-serialization invariant (`crates/thinclaw-config/src/secrets.rs:126,150`).
- **Sandbox proxy credential confinement — FIXED (wired).** Was: `with_credential_resolver` had zero callers; the sandbox always used `EnvCredentialResolver` reading process env. Now: `StoreCredentialResolver` (SecretsStore-backed) is implemented at `src/sandbox/proxy/http.rs:89` and injected via `.with_credential_resolver(Arc::new(StoreCredentialResolver::new(store, user_id)))` at `src/sandbox/proxy/mod.rs:96`, with tests at `http.rs:627,639`.
- **HTTPS credential-injection gap — RESOLVED (Option A, erase dead defaults).** Was: every shipped default mapping was HTTPS, which the in-proxy `AllowWithCredentials` branch can never inject through CONNECT tunnels. Now: `default_credential_mappings()` (`src/sandbox/config.rs:17`) returns an empty vec, with a doc comment stating injection applies only to the plaintext `http://` forward path and HTTPS credential delivery is out-of-band via the orchestrator `/worker/{id}/credentials` endpoint. `NETWORK_SECURITY.md` matches (credentials resolved at request time from the encrypted secrets store, `:420`).
- **`extra_public_routes` escape the layer stack — FIXED.** Was: WASM webhook routes were `.merge`d after the `.layer(...)` chain, inheriting none of body-limit/CORS/nosniff/frame-options. Now: `src/channels/web/server.rs:1592-1610` merges the extra public routes into the router **before** applying the layer stack (`DefaultBodyLimit` 1 MB, `cors`, nosniff, frame-options), with an in-code comment explaining axum's outermost-first layer ordering.
- **DNS-rebinding TOCTOU — FIXED in both guards.** Was: both `url_guard.rs` and `wrapper.rs::reject_private_ip` resolved, rejected private IPs, then discarded the address and let reqwest re-resolve at connect. Now: `validate_outbound_url_pinned` returns a `GuardedUrl { pinned_addrs }` (`crates/thinclaw-tools-core/src/url_guard.rs:32,55`) and callers pin via `ClientBuilder::resolve_to_addrs`; the WASM host mirrors this at `crates/thinclaw-tools/src/wasm/wrapper.rs:448-450` (`reject_private_ip` returns pinned addrs).
- **OAuth `state` not validated — FIXED.** Was: `wait_for_callback` extracted only the code param and never compared `state`. Now: `generate_oauth_state()` (`src/cli/oauth_defaults.rs:309`) mints a random CSRF nonce, `wait_for_callback_with_state` validates it via the constant-time `oauth_state_matches` (`:320`), and callers pass the expected value through.
- **`execute_code` host posture — FIXED.** Was: `requires_approval` returned `UnlessAutoApproved` unconditionally even on the bare-host default backend. Now: `requires_approval` (`crates/thinclaw-tools/src/builtin/execute_code.rs:914`) branches on `self.backend.kind()`, escalating to `Always` for `LocalHost` (and non-isolated backends) while keeping `UnlessAutoApproved` for `DockerSandbox`; the shell-vs-execute_code asymmetry is documented in `docs/SECURITY.md`.
- **Filesystem `base_dir == None` — RESOLVED DIFFERENTLY (still open as originally specified).** Was: `validate_path` skipped containment entirely when `base_dir` was `None`, allowing absolute paths and `..` traversal. The **shipped resolution is not** the recommended cwd-containment (Decision 3): when `base_dir` is `None`, the tool now runs in an explicit unrestricted "trusted-local-operator" mode — relative paths resolve against `current_dir()`, absolute paths are accepted as-is, and containment is enforced only when a base is configured (`crates/thinclaw-tools/src/builtin/file.rs:130-196`). Registration warns when no base is set so the unsandboxed state is observable, and untrusted callers must register the tools WITH a base. This closes the "silent unbounded traversal" surprise but does **not** implement the cwd-containment the DoD asked for — see T11.
- **Shell scanner health — FIXED (now surfaced).** Was: the fail-open scanner degraded silently with no operator-visible signal, and `status` had no scanner line. Now: `run_status_command` prints a "Shell scanner" line reporting the configured `external_scanner_mode`/path and the scanner status (`src/cli/status.rs:161-181`). The fail-open runtime default is unchanged but now discoverable.
- **WASM table/instance limits — FIXED (enforced).** Was: `tables_created`/`instances_created` were `#[allow(dead_code)]` and `table_growing` used a hardcoded 10_000 cap that ignored `max_tables`. Now: the `ResourceLimiter` increments the counters and honors `self.max_tables`/`max_instances`, with public `tables_created()`/`max_tables()`/`instances_created()` accessors and no `dead_code` allows (`crates/thinclaw-tools/src/wasm/limits.rs:118-124,166-200`). The limiter is attached to the wasmtime `Store`.

## Decision Points

1. **HTTPS credential injection — build OOB delivery vs erase dead defaults (Finding #7).**
   - *Option A (erase):* delete the three HTTPS default mappings from `src/sandbox/config.rs:12-14`, keep the HTTP-only `forward_request` path for operator-added plaintext hosts, and update NETWORK_SECURITY.md to state plainly that credentials reach containers **only** via the orchestrator `/worker/{id}/credentials` endpoint (already documented at NETWORK_SECURITY.md:223-231, 339). Lowest risk, ~30 min.
   - *Option B (build):* implement out-of-band credential delivery for HTTPS hosts (resolve the mapping at CONNECT-decision time and write the secret into the container's `/worker/{id}/credentials` response), so the documented guarantee fires for the defaults. Larger, touches the orchestrator credential-grant flow.
   - **Recommendation: A (erase the dead HTTPS defaults) for this WS, because the secure OOB path the doc already references is the real mechanism and the in-proxy HTTPS injection is architecturally impossible without MITM.** Keep `with_credential_resolver` + the HTTP `forward_request` path alive (still valuable for plaintext/internal hosts) — that is Finding #6, which we WIRE. File a follow-up note for Option B if operators ever need transparent HTTPS injection.

2. **`execute_code` `Always` vs feature-gate (Finding, §8).** Force `ApprovalRequirement::Always` when `backend.kind() == LocalHost`, vs gating bare-host execution off behind a feature.
   - **Recommendation: force `Always` on `LocalHost` (and `RemoteRunnerAdapter` if it lacks isolation), keep `UnlessAutoApproved` for `DockerSandbox`.** This realizes the capability (code execution stays available) while making bare-host runs a deliberate per-invocation operator decision, matching the shell tool's escalation pattern. Do not feature-gate — that removes a working capability.

3. **Filesystem `base_dir == None` — hard error vs default to cwd-containment (§Finding 9).**
   - **Recommendation (original): fail-closed — when `base_dir` is `None`, treat `current_dir()` as the implicit base and enforce containment against it (reject absolute paths and `..` escapes), rather than returning a blanket error.** This keeps the no-config dev path working (files under cwd) while removing the unbounded-traversal hole.
   - **Shipped (differs from the recommendation):** the code instead adopts an explicit unrestricted "trusted-local-operator" mode when `base_dir` is `None` — relative paths resolve against `current_dir()` and absolute paths are accepted as-is, with containment enforced only when a base is configured. `ToolRegistry::register_filesystem_tools` warns at registration time when no base is set, making the unsandboxed state observable, and untrusted contexts are required to register the filesystem tools WITH a base. This removes the *silent* traversal surprise but does not implement cwd-containment; the DoD item is therefore left open (T11).

4. **WASM limits — enforce vs delete the reserved counters (§Finding 11; audit lists this under WIRE).**
   - **Recommendation: WIRE — increment `tables_created`/`instances_created` and enforce `max_tables`/`max_instances` in `table_growing`/instance creation, removing the `#[allow(dead_code)]`.** The fields exist and the audit explicitly classifies this as built-but-disconnected. Erasing would weaken sandbox posture for no benefit.

## Tasks

Ordered so the CI gate goes green first (unblocks merging), then the auth bypass, then confinement.

- [x] **T1: Bump wasmtime-wasi to 36.0.11 (RUSTSEC-2026-0182).**
  - **Files:** `Cargo.lock` (wasmtime-wasi entry at line 9518; bump `wasmtime` at 9260 too if the resolver requires lockstep).
  - **Change:** `cargo update -p wasmtime-wasi --precise 36.0.11`. If cargo refuses due to the shared 36.x minor across `wasmtime`, also `cargo update -p wasmtime --precise 36.0.11`. Do NOT touch `Cargo.toml` (`version = "36"` already permits it). Verify both land at 36.0.11 in the lockfile.
  - **Acceptance:** `cargo deny check advisories` no longer reports RUSTSEC-2026-0182; `light`/`full`/`all-features` profiles still compile.
  - **Effort:** S
  - **Verification:** `cargo update -p wasmtime-wasi --precise 36.0.11 && cargo deny check 2>&1 | tail -20 && cargo check --workspace`

- [x] **T2: Clean up `deny.toml` (drift + stale ignores).**
  - **Files:** `deny.toml` (line 3 header; lines 10-25 ignore block).
  - **Change:** (a) Replace line 3 `# CI: .github/workflows/code_style.yml` with `# CI: .github/workflows/ci.yml (codestyle job)`. (b) Remove the `RUSTSEC-2026-0098/0099/0104` entries (lines 22-24) and the now-orphaned AWS-webpki comment (lines 11-21). Re-run `cargo deny` first to confirm cargo-deny no longer detects these advisories (they are flagged stale/advisory-not-detected); if any are still genuinely matched, keep only the matched ones and shrink the comment. Leave the `[licenses]`/`[bans]`/`[sources]` sections untouched.
  - **Acceptance:** `cargo deny check` passes with no `advisory-not-detected` warnings and no unused-ignore complaints.
  - **Effort:** S
  - **Verification:** `cargo deny check 2>&1 | tail -30` (expect clean exit 0).

- [x] **T3: Add `--all-targets` to CI clippy, then fix the `await_holding_lock`.**
  - **Files:** `.github/workflows/ci.yml:52` and `:121`; `crates/thinclaw-config/src/secrets.rs:121-179` (the three lock-holding tests).
  - **Change:** (a) `ci.yml:52` → `cargo clippy --workspace --all-targets --all-features -- -D warnings` (match CLAUDE.md). `ci.yml:121` → `cargo clippy --workspace --all-targets ${{ matrix.cargo-args }} -- -D warnings`. (b) Fix `await_holding_lock`: restructure each affected test so the `lock_env()` guard does not cross `.await`. The clean fix is to drop the guard before the await once env mutation is done is **not** possible here (the env must stay set during `resolve`), so instead make `lock_env()` usage block-scoped around a synchronous setup and pass values, OR switch these tests to a `tokio::sync::Mutex`-based async env guard, OR (simplest, matching repo precedent) add a targeted `#[allow(clippy::await_holding_lock)]` on each test with a comment explaining the env-serialization invariant — choose the targeted allow only if no async-lock helper exists. Prefer converting to an async-aware serialization primitive if one is already used elsewhere in `thinclaw-config` tests; otherwise the scoped `#[allow]` with justification is acceptable for test-only code.
  - **Acceptance:** `cargo clippy --all-targets --all-features -- -D warnings` is clean across the workspace; the three secrets tests still pass.
  - **Effort:** M
  - **Verification:** `cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test -p thinclaw-config secrets`

- [x] **T4: Close the empty `gateway_auth_token` auth bypass.**
  - **Files:** `crates/thinclaw-config/src/channel_config.rs:183-193`; `src/channels/web/mod.rs:72-81`.
  - **Change:** (a) In `channel_config.rs`, after the trim `.map(...)`, chain `.filter(|trimmed| !trimmed.is_empty())` so an empty/whitespace-only token becomes `None` — mirror `src/platform/gateway_access.rs:27-29` exactly. (b) In `GatewayChannel::new`, make empty defensive too: `config.auth_token.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| <random>)` so a `Some("")` arriving from any other path still gets a random token. Add a `tracing::warn!` when an empty token is replaced.
  - **Acceptance:** With `GATEWAY_AUTH_TOKEN=""`, the gateway generates a random token (not empty); an empty `Authorization: Bearer` is rejected. Add a unit test in `channel_config.rs` tests asserting empty → `None`, and a test in `web/mod.rs` (or wherever `GatewayChannel::new` is testable) asserting `auth_token` is non-empty.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw-config gateway && cargo test -p thinclaw web` (root crate) ; manual: `GATEWAY_AUTH_TOKEN="" cargo run -- ...` shows a generated token.

- [x] **T5: Wire a SecretsStore-backed CredentialResolver (Finding #6).**
  - **Files:** `src/sandbox/proxy/http.rs` (new `StoreCredentialResolver` next to `EnvCredentialResolver` at lines 53-70); `src/sandbox/proxy/mod.rs` (export it; optionally a `from_config_with_store` constructor); `src/sandbox/manager.rs:165` (inject via the existing `with_credential_resolver` hook at `proxy/mod.rs:96`).
  - **Change:** Add `pub struct StoreCredentialResolver { store: Arc<dyn SecretsStore>, user_id: String }` implementing `CredentialResolver::resolve(name)` by calling `store.get(&user_id, name).await` (`crates/thinclaw-secrets/src/store.rs:43`) and returning the decrypted value (map `SecretError` → `None` with a `debug!`, never log the value). Thread the `SecretsStore` (and `user_id`) into the sandbox manager's proxy construction at `manager.rs:164-167` via `.with_credential_resolver(Arc::new(StoreCredentialResolver{...}))`. Fall back to `EnvCredentialResolver` only when no store is configured.
  - **Acceptance:** When a sandbox starts with secrets enabled, the proxy resolves credentials from the AES-256-GCM store, not process env. Unit test: a `StoreCredentialResolver` over a mock `SecretsStore` returns the stored value; with no store, the env resolver is used.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw sandbox` (or the root test path for `src/sandbox`); `cargo clippy --all-targets`.

- [x] **T6: Resolve the HTTPS credential-injection gap (Finding #7, Decision 1).**
  - **Files:** `src/sandbox/config.rs:8-14` (`default_credential_mappings`); `src/NETWORK_SECURITY.md:339-352`.
  - **Change (recommended Option A — erase dead defaults):** Remove the three HTTPS default mappings (`api.openai.com`, `api.anthropic.com`, `api.near.ai`) since in-proxy injection cannot fire for HTTPS. Keep the HTTP `forward_request` injection path (T5's resolver still feeds it for any operator-added plaintext host). Update NETWORK_SECURITY.md to: (1) correct line 343 ("resolved … from the encrypted secrets store") so it scopes to the HTTP-only path and the store-backed resolver from T5, and (2) state that HTTPS credential delivery is via the orchestrator `/worker/{id}/credentials` OOB endpoint (already at lines 223-231, 335). Leave `forward_request`'s `AllowWithCredentials` branch intact.
  - **Acceptance:** No default mapping is unreachable; doc no longer overclaims HTTPS in-proxy injection. (If Option B is chosen instead, implement OOB delivery and document it — larger, see Decision 1.)
  - **Effort:** S (Option A) / L (Option B)
  - **Verification:** `cargo test -p thinclaw sandbox`; doc lint by reading NETWORK_SECURITY.md §5.

- [x] **T7: Apply the security-layer stack to `extra_public_routes` (Finding, §P1).**
  - **Files:** `src/channels/web/server.rs:1445-1467`.
  - **Change:** Merge the `extra_public_routes` into the router **before** the `.layer(DefaultBodyLimit)/.layer(cors)/.layer(nosniff)/.layer(X_FRAME_OPTIONS)` chain (lines 1450-1459), so the WASM webhook routers inherit them; or, if those routers must keep distinct state, wrap them in their own `Router` that re-applies the same four layers before merging. Preserve the `.with_state()` ordering comment's intent (Router<()> compatibility) by applying layers to the combined router. Keep CORS origins logic untouched.
  - **Acceptance:** A request to a WASM webhook route over the gateway is subject to the 1 MB body limit, CORS, `X-Content-Type-Options: nosniff`, and `X-Frame-Options: DENY`. Add/extend a server test that hits an extra public route and asserts the `nosniff` header is present and an over-limit body is rejected.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw web` (server tests); `cargo clippy --all-targets`.

- [x] **T8: Pin the validated IP in both URL guards (DNS-rebinding TOCTOU).**
  - **Files:** `crates/thinclaw-tools-core/src/url_guard.rs:16-95` (+ callers `crates/thinclaw-tools/src/builtin/http.rs:104`, `extract_document.rs:133`, `mcp/config.rs:253`); `crates/thinclaw-tools/src/wasm/wrapper.rs:1510-1564`.
  - **Change:** Have `validate_outbound_url` return the resolved `SocketAddr`(s) it already computed (lines 82-92) alongside the `Url`, and have callers build the `reqwest::Client` with `.resolve(host, pinned_addr)` (or use `ClientBuilder::resolve_to_addrs`) so reqwest connects to the IP that passed validation — eliminating the re-resolve TOCTOU. Mirror in `wrapper.rs::reject_private_ip` for the WASM HTTP host. Keep the existing private-IP rejection (`is_disallowed_ip`/`is_private_ip`) as the source of truth. Preserve the `OutboundUrlGuardOptions` API; add the pinned-addr as an additional return field rather than a breaking signature change if callers are numerous.
  - **Acceptance:** A hostname that resolves to a public IP at validation but a private IP at connect is blocked (the pinned public IP is used, and if that IP later moves there is no second resolution). Add a unit test using a fake resolver/`resolve_to_addrs` to assert the pinned address is used.
  - **Effort:** L
  - **Verification:** `cargo test -p thinclaw-tools-core url_guard && cargo test -p thinclaw-tools wasm`; `cargo clippy --all-targets`.

- [x] **T9: Validate OAuth `state` on the loopback callback.**
  - **Files:** `src/cli/oauth_defaults.rs:303-378` (`wait_for_callback`), callers `src/cli/tool.rs:761` and `src/tauri_commands.rs:595`; `auth_url` at `:142`.
  - **Change:** Add an `expected_state: Option<&str>` parameter to `wait_for_callback` and reject (return `OAuthCallbackError::Denied`/a new `StateMismatch`) when the callback query's `state` != expected — port the comparison from `crates/thinclaw-tools/src/mcp/auth.rs:858-867`. Generate a random `state` (e.g. `uuid::Uuid::new_v4()`, as `src/extensions/manager.rs:1906` already does) at the call sites, pass it into `auth_url(state, ...)`, and pass the same value as `expected_state`. Use a constant-time compare if the repo's helper is readily available; a plain `==` matches the MCP precedent and is acceptable for a CSRF nonce.
  - **Acceptance:** A callback with a missing or mismatched `state` is rejected; the happy path still returns the code. Add a unit test mirroring the MCP auth tests.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw oauth` (root crate cli tests); manual OAuth flow smoke if a provider is configured.

- [x] **T10: Force `Always` approval for bare-host `execute_code` (Decision 2) + document asymmetry.**
  - **Files:** `crates/thinclaw-tools/src/builtin/execute_code.rs:914-916`; `docs/SECURITY.md`.
  - **Change:** Rewrite `requires_approval` to branch on `self.backend.kind()`: `DockerSandbox` → `UnlessAutoApproved` (unchanged), `LocalHost` → `Always`, `RemoteRunnerAdapter` → `Always` unless the adapter advertises isolation (default `Always`). Add a `docs/SECURITY.md` subsection documenting that `execute_code` runs on the bare host when no sandbox backend is attached and therefore demands explicit approval, contrasting with the shell tool's per-command escalation (`shell.rs:681`+).
  - **Acceptance:** With the default `LocalHostExecutionBackend`, `requires_approval` returns `Always`; with a `DockerSandbox` backend it returns `UnlessAutoApproved`. Unit test asserting both.
  - **Effort:** S
  - **Verification:** `cargo test -p thinclaw-tools execute_code`; `cargo clippy --all-targets`.

- [ ] **T11: Fail-closed filesystem tools when `base_dir` is None (Decision 3). OPEN (resolved differently).** The shipped code does **not** implement cwd-containment; instead `validate_path` runs an explicit, warned unrestricted trusted-operator mode when no base is configured (see Decision 3 and Current State). The cwd-containment approach below was not adopted, so this task remains open as originally specified.
  - **Files:** `crates/thinclaw-tools/src/builtin/file.rs:130-196` (`validate_path`).
  - **Change:** When `base_dir` is `None`, treat `current_dir()` as the implicit containment base: compute the joined+normalized path (as today, lines 143-146) **and** run the same `starts_with(base_canonical)` containment check (lines 150-192) against the canonical cwd, rejecting absolute paths and `..` escapes with `ToolError::NotAuthorized`. Factor the containment block so both the `Some` and `None`-fallback paths share it. Keep the explicit-`base_dir` behavior unchanged.
  - **Acceptance:** With no configured base, reading `/etc/passwd` or `../../secret` is rejected; reading `./file_in_cwd` succeeds. Unit tests for both escape and in-cwd cases.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw-tools file`; `cargo clippy --all-targets`.

- [x] **T12: Surface shell-scanner health in `status` (fail-closed becomes deliberate).**
  - **Files:** `src/cli/status.rs:12` (`run_status_command`); read-only accessors on `crates/thinclaw-tools/src/builtin/shell.rs` (`scanner.mode()` already exists, line 263/485).
  - **Change:** Add a "Shell scanner" line to `run_status_command` reporting whether an external scanner is configured, its mode (`FailOpen`/`FailClosed`), and whether it is currently reachable (a lightweight health probe or last-known verdict). Make clear in the output that `FailOpen` means a degraded scanner does **not** block commands, so operators can choose `FailClosed` deliberately. Do not change the runtime fail-open default (that is an operator policy choice); only make it visible. If `status.rs` lacks a handle to the shell tool/config, surface the configured `external_scanner_mode` from settings instead.
  - **Acceptance:** `thinclaw status` shows scanner mode and reachability; the existing fail-open path at `shell.rs:496-501` is unchanged but now discoverable.
  - **Effort:** M
  - **Verification:** `cargo run -- status` (manual); `cargo test -p thinclaw status` if status tests exist; `cargo clippy --all-targets`.

- [x] **T13: Enforce WASM table/instance resource limits (Decision 4, Finding #11).**
  - **Files:** `crates/thinclaw-tools/src/wasm/limits.rs:62-166`.
  - **Change:** Remove the `#[allow(dead_code)]` on `tables_created`/`instances_created` (lines 71, 76). In `table_growing`, increment `tables_created` and reject when it would exceed `max_tables` (replace/augment the hardcoded 10_000 check at lines 143-149 to honor `self.max_tables`). Enforce `max_instances` analogously where instances are created (the `ResourceLimiter::instances()` cap at 154-156 reports the limit; confirm wasmtime calls it per-instantiation — if not, track instantiation count in the store wrapper that owns the limiter). Keep the Component-Model accommodation comment (lines 83-84) — the default of 10 must remain large enough for WASI adapters.
  - **Acceptance:** A module attempting to create more than `max_tables` tables or `max_instances` instances is denied; legitimate component-model modules still load. Unit test exercising the table cap.
  - **Effort:** M
  - **Verification:** `cargo test -p thinclaw-tools wasm::limits`; load a real packaged WASM tool to confirm no regression (`./scripts/build-all.sh` already builds artifacts); `cargo clippy --all-targets`.

- [x] **T14: Final security-doc alignment + full gate.**
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

- [x] `cargo deny check` exits 0 with no RUSTSEC-2026-0182 and no stale/unused ignores; `deny.toml` header points at the real `ci.yml` codestyle job.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` is clean; both clippy invocations (`ci.yml:66` and `:135`) pass `--all-targets`; the `await_holding_lock` in `thinclaw-config` is resolved.
- [x] `GATEWAY_AUTH_TOKEN=""` no longer authenticates an empty Bearer — fixed in **both** `channel_config.rs` and `web/mod.rs`, with negative tests.
- [x] The sandbox proxy resolves credentials from the `SecretsStore` via `with_credential_resolver` (not process env); dead HTTPS default mappings are removed (Option A) — and NETWORK_SECURITY.md matches.
- [x] WASM webhook (`extra_public_routes`) requests are covered by body-limit/CORS/nosniff/frame-options, with a test.
- [x] Both URL guards (`url_guard.rs` and `wrapper.rs`) pin the validated IP; a rebinding test passes.
- [x] The loopback OAuth callback validates `state`, with a mismatch-rejection test.
- [x] `execute_code` returns `Always` on bare-host backends; `docs/SECURITY.md` documents the shell-vs-execute_code asymmetry.
- [ ] Filesystem tools reject traversal/absolute escapes when `base_dir` is `None` (contained to cwd), with tests. **Not done as specified.** Shipped instead: an explicit, warned unrestricted trusted-operator mode when no base is configured (Decision 3, T11). Cwd-containment was not implemented.
- [x] `thinclaw status` surfaces shell-scanner mode and reachability.
- [x] WASM table/instance limits are enforced (`#[allow(dead_code)]` removed), with a cap test; real packaged tools still load.
- [x] Decision Points 1, 2, and 4 resolved as recommended; Decision 3 resolved with a **different** option (unrestricted trusted-operator mode, not cwd-containment) — see Decision 3 and T11.
- [x] `src/NETWORK_SECURITY.md` and `docs/SECURITY.md` updated in the same branch; inventory/drift docs left to WS-12.
