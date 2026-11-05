# WS-11 — Half-Wired Features & Doc Drift

| | |
|---|---|
| **Priority** | P2 (trails every wave) |
| **Effort** | M (spread across waves) |
| **Status** | ⏳ Not started |
| **Depends on** | runs alongside each WS; items owned by another WS are listed there and only cross-referenced here |
| **Findings** | COMP-1 … COMP-24, DRIFT-1 … DRIFT-16, TEST-1 … TEST-5, ARCH-1 … ARCH-3 |
| **Canonical docs** | per item; `FEATURE_PARITY.md` reconciliation is the exit artifact |

## Goal

Every capability is either working end-to-end or honestly marked as not shipped, and every canonical doc matches code. This is the "fully realized vs. aspirational" ledger for the 2026-11 snapshot.

## 11.A — Built but not working / half-wired (COMP)

Items owned by another workstream link there; do the work in that WS.

- [ ] **COMP-1** Self-repair cannot register a rebuilt tool (`registered: false` hard-coded, `src/tools/builder/core.rs:~399, ~450, ~499`; no hot-reload) → **WS-03.7**
- [ ] **COMP-2** Remote experiment auto-launch disabled for every backend; launch code after the guard is dead; validation says `launch_ready` → **WS-09.10**
- [ ] **COMP-3** Repo-project sandbox dispatch fails with default paths; in-container push likely broken → **WS-09.4**
- [ ] **COMP-4** Repo-project webhook payloads not mapped to state transitions; `unresolved_threads` and commit statuses never collected → **WS-09.2, 9.5, 9.7**
- [ ] **COMP-5** The 9 "equals"-secret beta WASM channels (Teams, Google Chat, Feishu, DingTalk, Mattermost, Matrix, QQ, WeCom, Weixin) can't receive real platform traffic — no platform sends `X-Webhook-Secret`; Feishu URL verification can't complete. Implement platform-native verification (Bot Framework JWT, Google Chat JWT, Feishu/DingTalk signatures, Mattermost token) or mark unsupported in `docs/CHANNEL_ARCHITECTURE.md` + registry metadata.
- [ ] **COMP-6** `TELEGRAM_ALLOW_FROM` parsed but never reaches the WASM host (`crates/thinclaw-config/src/channel_config.rs:~508`); native Slack/Telegram config structs resolved but unused → **WS-04.3**
- [ ] **COMP-7** Twitch replies likely broken: template sends `sender_id: {user_id}` (chatter) where Helix needs the bot id. Verify with a live test; fix.
- [ ] **COMP-8** Shared webhook guest renders `{content}` before metadata placeholders, so `{chat_id}` in agent text is substituted; any `{UPPER_CASE}` in a reply makes the host reject delivery as an unresolved credential (`crates/thinclaw-channels/src/wasm/wrapper/store.rs:~43, ~444`). Render metadata first / escape content.
- [ ] **COMP-9** Routine webhooks unusable for tool-created routines: `Trigger::Webhook.secret`/`allow_unsigned_webhook` hard-coded `None`/`false` (`crates/thinclaw-agent/src/routine_tools.rs:~302-305`) → every `/hooks/routine/{id}` call 403s. Add a secret-provisioning surface.
- [ ] **COMP-10** Web UI "Browse Files" navigates without a token → 401 in token mode → **WS-07.5**
- [ ] **COMP-11** Randomly generated gateway token never printed/persisted → **WS-07.12**
- [ ] **COMP-12** Web UI canvas iframe never renders → **WS-05.6**
- [ ] **COMP-13** Desktop `expose_inference` toggle is a no-op → **WS-05.11**
- [ ] **COMP-14** Command `minimum_authorization` decorative → **WS-04.8**
- [ ] **COMP-15** `disabled_tools` enforced only in chat lane and only for `local_user`; tool definitions not filtered → **WS-03.3**
- [ ] **COMP-16** WASM tool `workspace_read` never wired (`crates/thinclaw-tools/src/wasm/capabilities_schema.rs:~164`, `reader: None`) — wire a `WorkspaceReader` or document as unsupported.
- [ ] **COMP-17** Tool-side `{PLACEHOLDER}` credential substitution (`StoreData::credentials` / `with_credentials`) never populated in production — delete or wire host-bound.
- [ ] **COMP-18** Sandbox proxy plain-HTTP forwarding calls `set_host(ip)` with an unbracketed IPv6 literal → IPv6 targets fail (closed). Fix.
- [ ] **COMP-19** Desktop cloud sync: non-macOS keychain functions are error stubs (`apps/desktop/backend/src/cloud/encryption.rs:~380-400`); iCloud/Google Drive/WebDAV/SFTP are migration-only, no live sync (`StorageTab.tsx:~440-512`). Ship or label per platform.
- [ ] **COMP-20** Desktop backlog still open: TDO-113 (benchmarks/runtime smoke), TDO-120 (secret binding, gated), TDO-121/122 (iMessage, Nostr, long-tail channel configs), TDO-123, TDO-130, TDO-131, TDO-140 (repo-projects quarantined in Desktop), Phase 2 TDO-200…206, Phase 3 TDO-300…305; reconcile TDO-141/142 rows (code exists). Source: `apps/desktop/documentation/OVERHAUL_BACKLOG.md`.
- [ ] **COMP-21** iOS: real APNs / Live Activity / TestFlight delivery, live-LAN mDNS run, phone↔watch round trip on hardware — unverified; device-key proof-of-possession not enforced (v2) → **WS-07.13** for the gateway side.
- [ ] **COMP-22** Dead code: Gemini `?key=` URL builders (test-only; would leak key into errors if wired), Gmail `webhook_path` setting (validated, unused), stateless `wait_for_callback` (no callers), NEAR AI `/auth/callback` doc comment (flow gone). Delete.
- [ ] **COMP-23** F-09 shared WASM SDK `include!` extraction still partial (`docs/remediation/FOLLOWUPS.md`). Carry.
- [ ] **COMP-24** `FEATURE_PARITY.md` real gaps (❌): Linq (P3), HTTP path registration, auth/memory/provider plugin surfaces (re-check memory/provider after 639e5ab — see DRIFT-3); 🚧: linked DM continuity, iOS/watch/widgets/APNs client completion.

## 11.B — Doc vs code drift (DRIFT)

Fix in the same PR as the owning code change where one exists; otherwise a docs-only PR.

| ID | Doc claim | Code truth | Owner |
|---|---|---|---|
| DRIFT-1 | `FEATURE_PARITY.md:3` header: Tailscale passwordless identity "removed" | Re-added in 50d3731; row `:~595` is ✅ | WS-11 |
| DRIFT-2 | `FEATURE_PARITY.md:~213` `/subagents spawn`: "No such CLI/slash command" | 6aa06d8 added `src/cli/subagents.rs` (spawn/list/status/cancel) | WS-11 |
| DRIFT-3 | `FEATURE_PARITY.md:~381` memory plugins: "no surface" | 639e5ab added signed manifest memory/context providers | WS-11 |
| DRIFT-4 | `FEATURE_PARITY.md` source anchors (e.g. `src/channels/signal.rs`) | Now one-line re-export facades into `crates/thinclaw-channels` | WS-11 |
| DRIFT-5 | `docs/BUILD_PROFILES.md:~74` says `light` excludes REPL/TUI | Contradicts `:~241` and `Cargo.toml` (`repl = []` alias) | WS-11 |
| DRIFT-6 | `docs/ROBUSTNESS_AND_ARCHITECTURE_PLAN.md:~72, ~254` extension sub-lockfiles not under cargo-deny | `ci.yml` extension-dependency-policy audits all 28 | WS-11 |
| DRIFT-7 | `docs/remediation/FOLLOWUPS.md` F-16: DDG search test runs in nightly | `nightly.yml` never runs desktop-backend `--ignored` tests | WS-11 (+ TEST-2) |
| DRIFT-8 | `docs/refactor/BACKLOG.md` T9 `--locked` everywhere: open | Done except llvm-cov and fuzz | WS-11 |
| DRIFT-9 | `src/NETWORK_SECURITY.md` WebSocket Origin "required, exact match" | Any http(s) origin or none accepted | WS-07.1 |
| DRIFT-10 | `src/NETWORK_SECURITY.md` http tool HTTPS-only / no redirects / always approval; `allowed_secrets` enforced; responses scanned | See TOOL-10, C4, EXT-9 | WS-08.9, WS-06 |
| DRIFT-11 | `docs/SECURITY.md:~42` shell confined to workspace by default | Kernel confinement off by default | WS-08.4 |
| DRIFT-12 | `docs/PROCESS_ISOLATION.md` ambient operator data blocked for MCP | Grandparent + PATH dirs bound | WS-06.12 |
| DRIFT-13 | `docs/REPO_PROJECT_SUPERVISOR.md` merge-gate contract | Several checks not performed | WS-09.5 |
| DRIFT-14 | `docs/RELEASING.md` GITHUB_TOKEN + signed desktop updater; README install one-liners | App token; Apple disabled; installers 404 | WS-02.6, 2.9 |
| DRIFT-15 | `docs/TELEGRAM_SETUP.md`, `docs/GMAIL_SETUP.md`, `docs/CHANNEL_ARCHITECTURE.md` access-control wording | Groups open; `From` spoofable; "production" = webhook auth only | WS-04.11 |
| DRIFT-16 | Channel counts ("17 WASM channels") in overview docs | 16 channels + `shared_webhook_channel` library + 12 tools; canonical docs should avoid counts | WS-11 |

- [ ] DRIFT-1 … DRIFT-8, DRIFT-16 fixed in a docs-only PR
- [ ] DRIFT-9 … DRIFT-15 confirmed fixed by their owning WS PRs

## 11.C — Test posture (TEST)

- [ ] **TEST-1** `desktop_autonomy_live_smoke` ×6 `#[ignore]` with no reason and run nowhere — give reasons and schedule, or delete (see WS-02.8).
- [ ] **TEST-2** Desktop `hf_hub` ×2 and `web_search` live-network tests not in nightly — add a nightly desktop `--ignored` lane.
- [ ] **TEST-3** WebUI provider-discovery live diagnostic `#[ignore]` runs nowhere — schedule or delete.
- [ ] **TEST-4** Doctests and bin tests not covered by the lib coverage job; no Miri on `unsafe` sites (`crates/thinclaw-secrets/src/keychain.rs`, `src/extensions/native.rs`, `crates/thinclaw-platform/src/process.rs`) — add a Miri/sanitizer lane for the pure-Rust unsafe sites.
- [ ] **TEST-5** Security regression tests from WS-03 … WS-08 collected into a named suite (e.g. `tests/security_regressions.rs` or per-crate `security_*` modules) run on every PR.

## 11.D — Architecture hygiene (ARCH)

- [ ] **ARCH-1** God-file guard passes but 30 files sit at 1,800–1,999 lines (top: `crates/thinclaw-tools/src/mcp/client.rs` 1993, `apps/desktop/backend/src/cloud/migration.rs` 1990, `src/agent/worker.rs` 1988, `src/repo_projects/github.rs` 1987, `crates/thinclaw-secrets/src/store.rs` 1978). Decompositions are stopping at the cap, not at domain boundaries. Lower the guard in steps (e.g. 1,800 → 1,500) with a time-boxed allowlist, per `CLAUDE.md` Architecture Hygiene.
- [ ] **ARCH-2** 82 duplicated crate versions vs. `<30` target (`docs/ROBUSTNESS_AND_ARCHITECTURE_PLAN.md`); refactor backlog D3 `multiple-versions = deny`, T11 two wit-bindgen versions. Make progress measurable in CI (warn-count ratchet).
- [ ] **ARCH-3** Refactor backlog open items (A6/A8/A9/A11 partial; B6, D1/D2/D5, T1–T7, P1–P5; `clippy::unwrap_used` blocked on ~2,200 sites) — re-prioritize against this audit; `unwrap_used` can start as a ratchet on new code.

## Exit artifact

A `FEATURE_PARITY.md` reconciliation PR that (a) updates the "Last reconciled" header for this audit, (b) fixes DRIFT-1…4, and (c) reflects every COMP item's final disposition (shipped / marked unsupported / deleted).
