# 2026-11 Audit — Findings Registry

Baseline: `main` @ `5bebe1c`. Line anchors are as of that commit; they will drift as fixes land — follow the symbol, not the number.

**Severity** — judged on the default configuration with stated preconditions. *Critical*: untrusted input → host compromise, credential theft, or full control-plane takeover in a default or near-default setup. *High*: same impact with realistic opt-in preconditions, or a bypass of a stated security gate. *Medium*: plausible defense-in-depth gap or a security doc that overclaims. *Low*: hardening. *Blocker*: build/CI/release breakage.

**Verified** — **✔** re-traced in code during audit consolidation; **R** traced by one reviewer, not independently re-traced; **L** reviewer marked "Likely" (code path traced, trigger or runtime behavior inferred).

**Status** — `Open` until a PR lands; then `Fixed (<PR link>)`, `Won't fix (<decision>)`, or `Deferred (<reason>)`.

"Injection" below means the model following instructions embedded in content it reads (web page, email, document, tool/MCP output, channel message). It is the standing threat model for an agent, not an exotic precondition.

---

## 1. Registry

### Blockers — build, CI, release

| ID | Title | Verified | Task | Status |
|---|---|---|---|---|
| CI-1 | `tests/device_pairing_integration.rs` calls `.lock()` on `Arc<PendingApprovalsStore>` → all test-compile jobs fail (Windows ×3, macOS) | ✔ | WS-01.1 | Open |
| CI-2 | New CLI leaf `extensions mcp server isolation` uses an unreviewed verb → `tests/cli_coverage_manifest.rs:77` fails all 12 profile jobs + Code Style | ✔ | WS-01.2 | Open |
| CI-3 | Coverage job (`--all-features`) needs `cargo-component 0.21.1` for `bundled-wasm` (`build.rs:55-75`) — not installed on runner | ✔ | WS-01.3 | Open |
| CI-4 | `clippy::too_many_arguments` in `crates/thinclaw-types/src/subagent.rs:93` (+ ~8 other warnings) fails `-D warnings` | ✔ | WS-01.4 | Open |
| CI-5 | Unit test `channels::web::ws::tests::test_handle_client_approval_invalid_uuid` fails (error text changed) | ✔ | WS-01.5 | Open |
| CI-6 | Unit test `url_guard::tests::upgrades_http_when_requested` needs live DNS (non-hermetic) | ✔ | WS-01.6 | Open |
| CI-7 | All 4 vendored patches past `review_on: 2026-09-15` → `check-vendored-patches.py` fails | ✔ | WS-01.7 | Open |
| CI-8 | Credential-consumer ledger drift (11 new fields from 21a0b6c) → `check-credential-consumers.py` fails | ✔ | WS-01.8 | Open |
| CI-9 | Integration job fails at "Require large integration build storage" | ✔ (CI log) | WS-01.9 | Open |
| CI-10 | Desktop Companion "Frontend browser E2E" fails | ✔ (CI log) | WS-01.10 | Open |
| OPS-1 | v0.15.0 and v0.16.0 published with **zero assets**; installer URL 404; GHCR `:latest` == v0.14.0 | ✔ (v0.16.0 assets=[]) / R (GHCR digest) | WS-02.1 | Open |
| OPS-2 | Release assembly no longer ships `scripts/thinclaw-installer.sh` (regressed in bfdc25e); cargo-dist installer lacks documented flags and lists forbidden darwin targets | L | WS-02.2 | Open |

### Critical

| ID | Title | Precondition | Verified | Task | Status |
|---|---|---|---|---|---|
| **C1** | Injection → unapproved host shell / file write / HTTP exfil through `spawn_subagent` (aliases TOOL-1, AGT-1) | default | ✔ | WS-03.1–3.3 | Open |
| **C2** | Strangers drive the agent through channels and self-approve tool calls (aliases SEC-1, SEC-2, AGT-3) | channel enabled; Telegram owner unset (wizard default) | ✔ | WS-04.1–4.4 | Open |
| **C3** | Web UI XSS → gateway bearer-token theft via agent-controlled canvas `panel_id`; no CSP (aliases UI-1, UI-2, UI-3) | web UI open + injection | ✔ | WS-05.1–5.4 | Open |
| **C4** | WASM extension manifest can name **any** operator secret for injection into requests to its own allowlisted host (alias EXT-1) | malicious/compromised extension installed | ✔ | WS-06.1–6.4 | Open |

### High

| ID | Title | Precondition | Verified | Task | Status |
|---|---|---|---|---|---|
| H1 | `build_software` / self-repair builder execute registry tools directly (no approval, policy, hooks, plan mode); self-repair `remove_dir_all(build_dir.join(<LLM-derived name>))` (aliases TOOL-3, AGT-2) | default | ✔ deletion path / L trigger | WS-03.5, 3.7 | Open |
| H2 | Unapproved side-effect/exfil tools: WASM tools default `Never`; un-annotated MCP tools `Never`; `extract_document` fetches any HTTPS URL without approval or leak scan (aliases TOOL-4, TOOL-5) | default (WASM side-effects need an installed tool) | R | WS-06.9, WS-08.1 | Open |
| H3 | Ambient-auth gateway modes: WS Origin unchecked; Tailscale Serve headers trusted from any loopback peer, no Host check; trusted-proxy mode trusts loopback by default and always grants Admin; job-written files served same-origin as HTML (aliases GW-1, GW-2, GW-3) | Tailscale identity / `TRUSTED_PROXY_HEADER` (opt-in) | ✔ GW-1/GW-2 · R GW-3 | WS-07.1–7.5 | Open |
| H4 | `create_agent`/`update_agent` (no approval) write a global, unowned agent registry; default-agent prompt override applies to every principal and channel (alias AGT-4) | default | R | WS-03.4 | Open |
| H5 | WASM manifest `hooks` registered agent-wide without operator consent; can rewrite inbound text, tool params, outbound replies (alias EXT-2) | extension installed | R | WS-06.6 | Open |
| H6 | Repo-project merge gate: empty checks count as green in review + gate event keyed per task not per SHA; approvals counted from any GitHub login (aliases DATA-1, DATA-2) | supervisor + `maintainer_auto_merge` + `auto_merge` | ✔ | WS-09.1–9.2 | Open |
| H7 | Release workflow: all jobs `contents: write`/`packages: write`, `GH_TOKEN` in compile jobs; unverified `curl \| sh`; no binary signatures/provenance (aliases OPS-3, OPS-4, OPS-5) | dependency or upstream compromise | R | WS-02.3–2.5 | Open |
| SEC-3 | Native Discord, iMessage, BlueBubbles, Apple Mail, Gmail accept all senders by default; email allowlists match spoofable `From` | channel enabled | R | WS-04.6 | Open |
| SEC-4 | WhatsApp & Discord (WASM) allowlists also match attacker-chosen display names | channel enabled | R | WS-04.5 | Open |
| TOOL-2 | Builder registers host `shell`/`write_file`/`apply_patch` even when "local tools stay off", with `safety: None` (no external scanner) | builder enabled (default) + no Docker sandbox | R | WS-03.6 | Open |

### Medium

| ID | Title | Verified | Task | Status |
|---|---|---|---|---|
| GW-4 | TLS listener always binds `0.0.0.0` and auto-starts on first pairing, ignoring `GATEWAY_HOST` (`src/channels/web/tls.rs:411`) | ✔ | WS-07.6 | Open |
| GW-5 | RBAC `Operator` role reaches shell / `tool_install` via chat; only memory/learning tools read `principal_admin` | R | WS-04.10 | Open |
| GW-6 | `/api/logs/*` (DEBUG, all principals) readable by `ReadOnly`; logs SSE lacks passwordless re-auth guard | R | WS-07.7 | Open |
| AGT-5 | Any admitted sender can `/quit`/`/restart` the whole process; `minimum_authorization` never enforced | ✔ | WS-04.8 | Open |
| AGT-6 | `routine_create`/`routine_update` need no approval; injection can rewrite an operator routine that carries tool grants | R | WS-08.7 | Open |
| AGT-7 | Background subagent results re-enter as an unframed *user* turn under the parent identity | R | WS-03.8 | Open |
| TOOL-6 | Shell `Always` classification and path-escape detection are substring heuristics; `docs/SECURITY.md:42` overclaims workspace confinement | R | WS-08.3–8.4 | Open |
| TOOL-7 | Plan mode + `disabled_tools` only enforced on first dispatcher pass (not after approvals, not in subagent/`tool_rpc`/builder) | R | WS-03.3 | Open |
| TOOL-8 | Arg-policy glob `*` matches shell metacharacters; hand-rolled URL host parser disagrees with reqwest | R / L | WS-08.5 | Open |
| TOOL-9 | "Autonomous" wizard mode auto-approves writes to `~/.ssh`, `~/.thinclaw/.env`, user-tool TOMLs; "Full Auto" writes identical settings | R | WS-08.6 | Open |
| SEC-5 | Webhook secrets (incl. Twilio account Auth Token) injected into untrusted guest config and persisted in plaintext workspace JSON | R | WS-06.5 | Open |
| SEC-6 | Pairing blocklist ignored by WASM guests (`pairing_read_allow_from` returns raw allow list) | R | WS-04.7 | Open |
| SEC-7 | Channel docs overclaim access control (Telegram groups, Gmail allowlist spoofing, "production" = webhook auth only) | R | WS-04.11 | Open |
| EXT-3 | Agent/gateway install path drops registry `sha256` and artifact-host allowlist; silent overwrite | R | WS-06.7 | Open |
| EXT-4 | Native-plugin gates are DB settings flippable via `PUT /api/settings`; `tool_activate` (no approval) can `dlopen` a registered plugin; docs say operator-only | R | WS-06.8 | Open |
| EXT-5 | `skill_install` honors an LLM-supplied `approve_risky`; trust promotion / trusted taps only `UnlessAutoApproved` | R | WS-06.10 | Open |
| EXT-6 | Community skill attenuation bypassed through `skill_read` lazy loading | L | WS-06.11 | Open |
| EXT-7 | Strict MCP isolation read-only-binds the executable's grandparent (e.g. exposes `~/.cargo/credentials.toml`) | R | WS-06.12 | Open |
| UI-4 | AppleScript injection in macOS desktop bridge turns an approved `keypress`/`select_menu` into unsandboxed shell | R | WS-05.9 | Open |
| UI-5 | All ~350 Tauri commands exposed to every window (incl. Spotlight, which renders model output); raw secret getters over IPC | L | WS-05.8 | Open |
| DATA-3 | Read-only repo reviewer jobs receive the upstream push PAT in maintainer modes | R | WS-09.3 | Open |
| DATA-4 | Postgres HNSW search post-filters scope after an `ef_search`-bounded scan → empty per-user recall | L | WS-10.1 | Open |
| DATA-5 | Supervisor sandbox dispatch fails with default paths (`repo_projects` dir outside allowed `projects` mount base); in-container git push likely broken | R / L | WS-09.4 | Open |
| DATA-6 | Merge-gate contract doc claims checks the code doesn't perform (commit statuses, base branch, installation id) | R | WS-09.5 | Open |
| DATA-7 | Autonomous experiment campaigns have no default trial/cost/runtime ceiling | R | WS-09.8 | Open |
| OPS-6 | (= CI-7 + CI-8) Code Style job red on expired patch review + credential ledger drift | ✔ | WS-01.7–1.8 | Open |
| OPS-7 | Root `docker-compose.yml` publishes Postgres on `0.0.0.0:5432` with password `thinclaw` | R | WS-02.7 | Open |

### Low

| ID | Title | Task |
|---|---|---|
| GW-7 | Tailscale whois doesn't check the connection arrived on the tailnet interface; no negative cache → peer can exhaust whois permits | WS-07.8 |
| GW-8 | Pairing lockout is global (anyone can deny pairing) | WS-07.9 |
| GW-9 | Routine-webhook HMAC has no timestamp/nonce (replay); fallback fences payload without escaping backticks | WS-07.10 |
| GW-10 | `/metrics` unauthenticated when Prometheus backend enabled (documented) | WS-07.11 |
| SEC-8 | Discord Ed25519 has no timestamp freshness window; replay cache in-memory only | WS-04.12 |
| SEC-9 | Pairing queue (`PAIRING_PENDING_MAX = 3`) can be jammed | WS-04.9 |
| SEC-10 | Secret AAD lacks length prefixes; derived/master key material not zeroized | WS-10.3 |
| EXT-8 | Online discovery turns top-starred GitHub repo homepages into installable `validated: true` MCP entries; approval card hides URL | WS-06.13 |
| EXT-9 | WASM HTTP response headers and non-UTF-8 bodies not leak-scanned | WS-06.14 |
| TOOL-10 | Stale security docs/comments (http tool, reckless desktop approvals, scanner default) | WS-08.9 |
| TOOL-11 | Sandboxed-job exfil paths: default proxy allowlist includes `github.com`/`api.github.com`; `tool_rpc` auto-approves `http` GET / browser navigate | WS-03.9 |
| AGT-8 | One detected injection pattern drops SOUL/AGENTS/IDENTITY together | WS-08.8 |
| UI-6 | Web UI canvas iframe never renders; sandbox `allow-scripts allow-same-origin` would be void if it did | WS-05.6 |
| UI-7 | Cloud-sync object keys use unkeyed `sha256(path)`/`sha256(plaintext)` | WS-05.10 |
| UI-8 | Bearer tokens in SSE/log URLs; webhook canvas embeds secret in HTML | WS-05.5 |
| UI-9 | `expose_inference` toggle is a no-op; stale divergent unserved copy of web static assets | WS-05.7, 5.11 |
| UI-10 | Remote deploy uses SSH `StrictHostKeyChecking=accept-new` | WS-05.12 |
| DATA-8 | Embedded `"` in a keyword breaks libSQL memory FTS and drops vector results too | WS-10.2 |
| DATA-9 | SSH/Slurm launch args not checked for leading `-` (currently unreachable) | WS-09.9 |
| DATA-10 | Some experiment / repo-project records are global (readable by any authenticated principal); desktop session delete has no owner check | WS-09.12 |
| OPS-8 | Release image not digest-pinned; no `HEALTHCHECK`; apt not snapshot-pinned | WS-02.7 |
| OPS-9 | Native systemd unit unhardened; legacy unit pulls mutable `:latest` with host networking | WS-02.7 |
| OPS-10 | Unpinned `cargo install` (cargo-deny, cargo-fuzz); TestFlight lane has no `environment:` gate | WS-02.7 |
| OPS-11 | Integration-test "evidence" check is a substring match satisfied by a comment | WS-02.8 |

Completion gaps (`COMP-*`) and doc drift (`DRIFT-*`) are enumerated in [WS-11](./WS-11-half-wired-and-doc-drift.md).

---

## 2. Critical findings — detail

### C1 — Approval classes don't survive delegation

- `crates/thinclaw-tools/src/builtin/subagent.rs:287` — `spawn_subagent::requires_approval` returns `ApprovalRequirement::Never`.
- `crates/thinclaw-agent/src/subagent.rs:~982` — allowed-tools merge: `(None, Some(requested)) => requested`. When the parent thread has no routed allowlist (the default), the model-requested list is taken verbatim; an explicit list also satisfies the `explicit_only` profile (`crates/thinclaw-tools/src/registry.rs:~1592`).
- `src/agent/subagent_executor/mod.rs:~1618` — subagent tool calls are prepared with `ToolApprovalMode::Autonomous` and `hooks: None`.
- `crates/thinclaw-tools/src/execution.rs:~281` — `approval_required(_, Autonomous)` only returns true for `ApprovalRequirement::Always`.
- **Effect:** every `UnlessAutoApproved` tool — regular shell commands, `write_file`, `http`, `send_message` — runs in a subagent with no operator prompt. BeforeToolCall hooks and `disabled_tools` are also skipped. A second route with the same effect is `create_agent` + `message_agent` (`crates/thinclaw-tools/src/builtin/agent_management.rs`, all default `Never`), which is also persistent (H4).
- **Amplifier (TOOL-2):** with the builder on (default), host dev tools are registered even when the wizard reports "local tools stay off", with `safety: None` (`crates/thinclaw-app/src/assembly.rs:326`, `src/tools/registry.rs:1173`).
- **Fix direction:** WS-03 / decision D3.

### C2 — No owner tier; strangers reach the agent and approve their own calls

- Telegram (WASM) `channels-src/telegram/src/media.rs:~73` — DM policy / pairing applies only in the `else if is_private` branch; group delivery defaults on (`:155`, `.unwrap_or("true")`), `owner_id` defaults null (`telegram.capabilities.json`, `crates/thinclaw-settings/src/channels.rs:395-397`); the wizard invites leaving the owner blank (`src/setup/channels/telegram.rs:500`). In a group, a mention or a `/command` from anyone is emitted.
- Slack `channels-src/slack/src/lib.rs:~580` (`if !is_dm { return true; }`) and Discord `channels-src/discord/src/lib.rs:~552` follow the same pattern for channel mentions / guild interactions.
- `channels-src/shared_webhook_channel/src/impl.rs:333-389` — emits every payload; no allowlist/pairing option exists for Twilio SMS, LINE, Twitch, Teams, Google Chat, Feishu, DingTalk, Mattermost, Matrix, QQ, WeCom, Weixin (R).
- The tool profile is a single global `main_tool_profile` (`src/agent/dispatcher/loop.rs:~674`); nothing narrows it for non-owners.
- Approvals are bound to the **requester** (`crates/thinclaw-agent/src/thread_ops.rs:~340` → `accepts_identity`; `crates/thinclaw-agent/src/session.rs:~386`). Binding prevents hijacking the owner's approvals, but is not an owner gate: the stranger who triggered a call can approve it.
- **Fix direction:** WS-04 / decisions D1, D2.

### C3 — Web UI XSS → token theft

- Sink: `crates/thinclaw-gateway/src/web/static/app.js:~10365` builds `onclick="dismissCanvasPanel('…')"` inside `card.innerHTML` using `escapeJsString` (`:~10426`), which escapes backslash, single quote, CR, LF — **not** `"`. A `"` in `panel_id` therefore terminates the attribute and lets the value add arbitrary attributes, including event handlers.
- Source: the canvas tool is `ApprovalRequirement::Never` and registered by default (`crates/thinclaw-tools/src/builtin/canvas.rs:~278`); `validate_panel_id` (`:~331`) only bounds length and control chars. `panel_id` flows verbatim via `StatusUpdate::CanvasAction` → `crates/thinclaw-gateway/src/web/status.rs:~625` (`SseEvent::CanvasUpdate`) → `handleCanvasUpdate` → `showCanvasPanel`.
- Impact: the main UI sends no `Content-Security-Policy` (only the plaintext export page has one, `crates/thinclaw-gateway/src/web/chat.rs:~551`); the bearer token is a top-level `let` (`app.js:3`) and in `sessionStorage` — an injected handler can read and exfiltrate it. The token is full gateway control.
- Systemic: `escapeHtml` (`app.js:~10420`, textContent→innerHTML) does not escape quotes either, and is used in ~76 attribute / inline-JS contexts (e.g. routine name in `onclick` at `:~5726`, routine names being model-creatable without approval). The denylist `sanitizeRenderedHtml` (`:~1960`) keeps remote `<img>` → zero-click exfil through rendered markdown.
- **Fix direction:** WS-05.1–5.4.

### C4 — Unscoped secret injection for WASM extensions

- `crates/thinclaw-tools/src/wasm/capabilities_schema.rs:~237` — `http.credentials[*].secret_name` is only checked to be an identifier; `host_patterns` only against the extension's own HTTP allowlist.
- `crates/thinclaw-tools/src/wasm/wrapper.rs:~1652` — `resolve_host_credentials` calls `store.get_for_injection(user_id, &mapping.secret_name, …)` for every mapping; no namespace / owner / provider check (`crates/thinclaw-secrets/src/store.rs:~386`).
- Variant: `auth.env_var` (`capabilities_schema.rs:~436`) copies an arbitrary host env var into the extension's secret (`src/extensions/manager/wasm.rs:~82`); `tool_activate` is `Never` and auto-runs auth (`crates/thinclaw-tools/src/builtin/extension_tools.rs:~698`) (R).
- WASM channels: `setup.required_secrets` names are unscoped (`crates/thinclaw-channels/src/wasm/schema.rs:~283`, `runtime_config.rs:~364`) (R).
- Doc overclaim: `src/NETWORK_SECURITY.md:~524` says secrets must be in the tool's `allowed_secrets` list — that check (`CredentialInjector`) is test-only.
- **Fix direction:** WS-06.1–6.4.

---

## 3. High findings — detail pointers

High-finding evidence, scenarios, and fix sketches live with their tasks:
H1 → [WS-03](./WS-03-delegation-and-approval-integrity.md) §3.5/3.7 · H2 → [WS-06](./WS-06-extension-trust-boundary.md) §6.9 and [WS-08](./WS-08-tool-policy-hardening.md) §8.1 · H3 → [WS-07](./WS-07-gateway-remote-access.md) §7.1–7.5 · H4 → WS-03 §3.4 · H5 → WS-06 §6.6 · H6 → [WS-09](./WS-09-repo-projects-and-experiments.md) §9.1–9.2 · H7 → [WS-02](./WS-02-release-and-distribution.md) §2.3–2.5 · SEC-3/SEC-4 → [WS-04](./WS-04-principal-authority-and-channel-ingress.md) §4.5–4.6 · TOOL-2 → WS-03 §3.6.

---

## 4. Re-verified June fixes and sound controls

**June 2026 fixes — still fixed:** empty `gateway_auth_token` bypass (filtered in `crates/thinclaw-config/src/channel_config.rs`, `src/platform/gateway_access.rs`; `GatewayChannel::new` replaces missing/empty with a random token); OAuth `state` generated and compared in constant time on every caller, PKCE S256, loopback-only listeners; DNS-rebinding TOCTOU in `http` / `extract_document` (pinned resolution + redirect re-validation); `execute_code` host execution only in `unrestricted` mode and then `Always`; external shell scanner defaults `fail_closed` (but see TOOL-2: the builder path never configures it); libSQL transcript FTS sanitization; Discord Ed25519 host-side verification; char-aware `split_message`.

**Controls verified sound (do not regress):**
- Gateway default mode: constant-time bearer; RBAC classifier fails closed; device scopes are exact method+path allowlists; pairing secrets 256-bit / 40-bit code, hashed, single-use, 15-min TTL; owner-bound approvals; 1 MB body limit; CORS limited to localhost/bind origins.
- Secrets: AES-256-GCM, random nonce + per-secret HKDF salt, AAD binds user/name/provider/version, legacy rows rejected, OS keychain master key, env master key opt-in only, private 0600 atomic writes.
- Webhooks: every WASM channel path fails closed when its secret is unset; constant-time compares; Slack 5-min window; 1 MiB / 64-concurrency caps; signature headers stripped before the guest.
- WASM sandbox: WHATWG allowlist parse (userinfo rejected, traversal normalized, trailing-dot fails closed), HTTPS-only, DNS pinned, no redirects, no WASI FS/env, fuel/epoch/memory limits.
- Tool execution: approvals evaluated on post-hook args; shell scans the string it executes; host children get a cleared env; Docker sandbox `cap_drop ALL`, no-new-privileges, read-only rootfs, non-root, no Docker socket, fails closed without Docker.
- SQL: no injection found — all `format!` SQL interpolation is constant; experiment/job queries owner-scoped.
- Desktop: strict CSP, `react-markdown` without raw HTML, sidecar keys via 0600 file, HF downloads hash-verified.
- iOS: Secure Enclave device key, SPKI pinning fails closed, no ATS exceptions, content-free APNs.
- CI: all Actions SHA-pinned; `pull_request_target` never checks out PR head; worker image digest-pinned with SBOM + Trivy.
