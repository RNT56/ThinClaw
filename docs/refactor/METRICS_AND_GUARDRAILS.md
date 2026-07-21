# Metrics & Guardrails

How we know we're done, and how each invariant is kept fixed. Re-run the
`robustness-architecture-audit` workflow after each wave and update the **Current** column.

## Metrics dashboard (baseline 2026-06-29 → target)

**Current** column verified 2026-07-11 against the merged loop, channel, and dependency hardening stack.
✅ = target met.

| Metric | Baseline | Current (2026-07-11) | Target | Tracked by |
|---|---|---|---|---|
| Dependency cycles | 0 | 0 ✅ | 0 (guarded) | crate-boundary CI |
| Wrong-direction crate edges | 2 | 0 (removed + CI-guarded) ✅ | 0 | A1, A2 done |
| Files > 2,000 lines | 18 | 0 ✅ | < 5 → 0 (guarded) | A5–A8, T10 done |
| Largest file (lines) | 4,577 | 1,999 (`crates/thinclaw-channels/src/gmail.rs`) | < 800 | A8 (< 800 not met) |
| Duplicate-versioned crates (root lock) | 94 | 82 (`cargo deny` duplicate diagnostics; +4 vs immediate pre-upgrade `main`, -12 vs the audit baseline, still above target) | < 30 (deny-gated) | D1–D3 |
| `rand` versions in tree | 3 (root) / 4 (desktop) | 3 (root: 0.8.6 / 0.9.4 / 0.10.2); desktop advisory cleared | 1 | D2 |
| ROUTE_TABLE command coverage | 15/341 (4%) | 346/346 (100%, test-enforced) ✅ | 100% (gated) | B4 done |
| Commands returning `Result<_, String>` | ~149 (undercounted) | 313/342 | 0 | B5 |
| Dead `ObserverEvent` variants | 5/10 | 0 ✅ | 0 | B2 done |
| `StatusUpdate` `#[non_exhaustive]` | no | yes ✅ | yes | B1 done |
| Persistent log sink | none | rolling daily file + ring ✅ | rolling file + ring | A3 done |
| `/api/health` real readiness | no | yes ✅ | yes | A4 done |
| `cargo-deny` workspace coverage | 1 of (2 + 28 sub) | root + desktop (incl. advisories); `channels-src/` + `tools-src/` sub-workspace lockfiles NOT scanned | all (advisories) | D4, T-sub |
| Desktop dependency advisories | 3 | 0 ✅ (CI runs `cargo deny check advisories` on the desktop backend) | 0 (gated) | #130 done |
| `--locked` CI coverage | desktop + primary root | primary root + desktop | all jobs | #129 done, T9 |
| Signed desktop release | no | no | yes | P3 |
| Production hot-path panics (audited) | ~10 sites | fixed in #128 | 0 + lint | #128, A11, B-lint |
| Blocking-in-async (audited) | 4 sites | 2 fixed (#127) | 0 + clippy gate | #127, async-clippy |
| Observer events emitted | 5/10 | 10/10 ✅ | 10/10 | B2 done |

## Guardrail catalog

Each architectural invariant must be machine-enforced. Status: ✅ live · 🟡 ready, add with its task ·
⛔ blocked (prereq noted).

| Guard | Enforces | Status / prereq |
|---|---|---|
| `cargo check --locked` on all gates | lockfile drift | ✅ primary root (#129) + desktop; 🟡 extend to all jobs (T9) |
| `cargo-deny` per workspace (advisories) | unscanned supply chain | ✅ root (incl. advisories) + desktop advisories (#130); 🟡 `channels-src/` + `tools-src/` sub-workspace lockfiles still unscanned |
| crate-boundary grep (`use thinclaw::` / wrong edges) | dependency direction | ✅ root-import; ✅ db→agent + gateway→tools guards LIVE (A1, A2; `ci.yml` "Check crate boundaries") |
| god-file size guard (`.rs > N lines` fails CI) | god-file regrowth | ✅ LIVE at 2,000 (`scripts/ci/check-file-sizes.sh`, `MAX_LINES=2000`, `ci.yml:64`); a stricter threshold (< 2,000) is future work |
| `ROUTE_TABLE` coverage test | unclassified dual-mode commands | ✅ LIVE (`all_registered_commands_are_classified`, `bridge.rs:764`) |
| `clippy::await_holding_lock = deny` | async std-lock deadlocks | ✅ already warn-by-default under `-D warnings` |
| `clippy::unwrap_used = warn` | new production panics | ⛔ NOT enabled; set to `"allow"` (`Cargo.toml:466`); still blocked by `-D warnings` (must decouple first) |
| `deny.toml multiple-versions = deny` | duplicate-version creep | ⛔ still `"warn"` (`deny.toml:38`); blocked until D1/D2 dedup |
| `wit-bindgen` single-version check | WASM interface skew | 🟡 with T11; still 2 versions (0.51.0, 0.57.1) |
| bundle-reference resolution test | broken registry bundles | 🟡 with P5/T11 |
| coverage threshold (`--fail-under`, no `--lib`) | silent coverage erosion | 🟡 partial: CI enforces the measured 38% project floor and 70% changed-line coverage; expanding beyond `--lib` remains |
| `export_bindings` no-hand-edit + variant-coverage test | binding drift | ✅ exists; extend per B-tasks |
| MSRV/toolchain synchronization | accidental MSRV bump | ✅ `check-msrv-sync.py` runs in CI; the pinned toolchain equals package MSRV 1.94 |

## Re-audit procedure

After each wave, re-run the parallel audit to refresh **Current** and catch regressions:

```
Workflow: robustness-architecture-audit   (10 dimensions, grounded, ~12 min)
→ diff metrics vs this table → update Current → flag any new High/Critical findings into BACKLOG.
```

The refactor is "done enough to declare extreme robustness" when: every **Target** is met, every
guard in the catalog is ✅, and a fresh audit surfaces no new Critical/High findings.
