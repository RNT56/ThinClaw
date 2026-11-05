# WS-01 — CI Recovery & Dependency Backlog

| | |
|---|---|
| **Priority** | P0 — gates every other workstream |
| **Effort** | S–M |
| **Status** | ⏳ Not started |
| **Depends on** | — |
| **Findings** | CI-1 … CI-10, OPS-6 |
| **Canonical docs** | `docs/VENDORED_PATCHES.md`, `docs/DEVELOPMENT.md` (if the local gate changes) |

## Goal

Return `main` to green and keep it there, then drain the frozen dependency backlog. Baseline: CI run [32000242050](https://github.com/RNT56/ThinClaw/actions/runs/32000242050) on `5bebe1c` failed 20+ jobs; locally `cargo test --workspace --lib` = 5,008 pass / 2 fail, clippy `-D warnings` fails.

## Tasks

- [ ] **1.1 — Fix `device_pairing_integration` compile (CI-1).**
  `tests/device_pairing_integration.rs:814, 846, 874` call `.lock()` on `state.pending_approvals`, now `Arc<PendingApprovalsStore>` (API changed by 50d3731 / 6aa06d8). Port the test to the store's API.
  *Accept:* `cargo test --locked --features full --test device_pairing_integration` compiles and passes; Windows/macOS "Compile tests (no run)" jobs green.

- [ ] **1.2 — Resolve the unreviewed CLI verb (CI-2).**
  `tests/cli_coverage_manifest.rs:77` → "canonical CLI leaf 'extensions mcp server isolation' uses unreviewed final verb 'isolation'". Either rename the leaf to a reviewed verb (preferred if a fitting one exists, e.g. a `show`/`status` form) or add `isolation` to the reviewed-verb set with a justification comment; regenerate the CLI contract artifacts.
  *Accept:* `cargo test --test cli_coverage_manifest`, `python3 scripts/ci/check-cli-contract-manifest.py`, `python3 scripts/ci/check-cli-command-drift.py` pass; all 12 "Profile:" jobs green. Update `docs/CLI_REFERENCE.md` if the leaf is renamed.

- [ ] **1.3 — Unblock the coverage job (CI-3).**
  `build.rs:55-75` asserts `cargo-component == release config cargo_component_version (0.21.1)` when `bundled-wasm` is enabled; the "Run Tests" llvm-cov job uses `--all-features`. Either install the pinned cargo-component in that job (mirroring the release workflow) or run coverage with an explicit feature list excluding `bundled-wasm`.
  *Accept:* "Run Tests" green; coverage ratchet baseline unchanged or re-baselined with a note.

- [ ] **1.4 — Clippy clean (CI-4).**
  `crates/thinclaw-types/src/subagent.rs:93` `SubagentRunRecord::new_running` takes 8 args (threshold 7 in `clippy.toml`). Refactor (params struct / builder) rather than `#[allow]`. Fix the remaining warnings surfaced by the command below (≈8: `type_complexity`, `needless_return`, `question_mark`, `items_after_test_module`, …).
  *Accept:* `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` passes.

- [ ] **1.5 — Fix `test_handle_client_approval_invalid_uuid` (CI-5).**
  `src/channels/web/ws.rs:~1121-1147` expects the error to contain "Invalid request_id"; the message changed in the 2026-08-13 commits. Decide whether behavior or test is wrong (an invalid UUID should still produce a precise, non-leaky error).
  *Accept:* `cargo test -p thinclaw --lib channels::web::ws` passes.

- [ ] **1.6 — Make `url_guard::upgrades_http_when_requested` hermetic (CI-6).**
  `crates/thinclaw-tools-core/src/url_guard.rs:~316-320` resolves `example.com`. Inject a resolver or test the pure parse/upgrade step.
  *Accept:* passes with networking disabled.

- [ ] **1.7 — Re-review vendored patches (CI-7).**
  `patches/manifest.json` — all four entries expired `review_on: 2026-09-15`: libsql double-close fix (upstream PRs #2251/#2261 were open at audit time), parser CVE-2025-47736 backport, glib RUSTSEC-2024-0429 fix, glass_pumpkin. For each: check upstream, drop if landed, else re-justify and bump `review_on`.
  *Accept:* `python3 scripts/ci/check-vendored-patches.py --verify-upstream` passes; `docs/VENDORED_PATCHES.md` updated.

- [ ] **1.8 — Reconcile the credential-consumer ledger (CI-8).**
  21a0b6c added `bot_token`×2, `app_token`, `signing_secret` and 7 `*_configured` flags in `apps/desktop/backend/src/thinclaw/commands/channel_settings.rs` without reviewed entries (314 detected vs 303 in `tests/fixtures/credential_consumer_manifest.json`). Review each classification (several were auto-classified `ephemeral_internal`), then `--write`.
  *Accept:* `python3 scripts/ci/check-credential-consumers.py` passes.

- [ ] **1.9 — Integration job storage step (CI-9).**
  "Execute classified workspace integration targets" fails at "Require large integration build storage". Fix runner selection / disk requirement.
  *Accept:* job runs all hermetic integration targets.

- [ ] **1.10 — Desktop Companion E2E (CI-10).**
  Triage "Frontend browser E2E" failure in the Desktop Companion job.
  *Accept:* job green.

- [ ] **1.11 — Land or close the open CI-repair PR.**
  [RNT56/ThinClaw#455](https://github.com/RNT56/ThinClaw/pull/455) "fix(ci): repair main iOS build and worker image CA bootstrap" (open since 2026-08-17). Rebase on the WS-01 fixes and merge, or fold into this workstream.

- [ ] **1.12 — Drain the Dependabot backlog (~47 PRs).**
  Order: security-relevant first — rustls 0.23.46 ([#529](https://github.com/RNT56/ThinClaw/pull/529), [#530](https://github.com/RNT56/ThinClaw/pull/530)), undici ([#528](https://github.com/RNT56/ThinClaw/pull/528)), dompurify ([#525](https://github.com/RNT56/ThinClaw/pull/525)), zip 9 ([#531](https://github.com/RNT56/ThinClaw/pull/531)); then grouped bumps ([#533](https://github.com/RNT56/ThinClaw/pull/533), [#532](https://github.com/RNT56/ThinClaw/pull/532), actions [#526](https://github.com/RNT56/ThinClaw/pull/526)); then per-extension `serde`/`serde_json`/`base64`/`getrandom`/`grammers-crypto` and `wit-bindgen` bumps (coordinate with refactor backlog T11 — unify on one wit-bindgen). Re-run `cargo deny check` (root, desktop, all 28 extension lockfiles) — the audit could not run the advisory scan.
  *Accept:* no Dependabot PR older than 14 days; `cargo deny check` green everywhere.

- [ ] **1.13 — Keep it green.**
  Add a lightweight guard so a red `main` is noticed within a day (e.g. scheduled workflow that opens/updates an issue when the latest `main` CI run failed), and make the cargo-free checks (`check-cli-*`, `check-credential-consumers`, `check-vendored-patches`) part of the local `/ship` gate so expiry/drift is caught before push. Add a calendar reminder or CI warning N days before any `review_on` date.

## Verification (workstream exit)

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace
python3 scripts/ci/check-vendored-patches.py --verify-upstream
python3 scripts/ci/check-credential-consumers.py
cargo deny check
```

Exit criterion: a full green CI run on `main`, linked in the README tracker.
