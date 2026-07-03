# Metrics & Guardrails

How we know we're done, and how each invariant is kept fixed. Re-run the
`robustness-architecture-audit` workflow after each wave and update the **Current** column.

## Metrics dashboard (baseline 2026-06-29 → target)

| Metric | Baseline | Current | Target | Tracked by |
|---|---|---|---|---|
| Dependency cycles | 0 | 0 | 0 (guarded) | crate-boundary CI |
| Wrong-direction crate edges | 2 | 2 | 0 | A1, A2 |
| Files > 2,000 lines | 18 | 18 | < 5 → 0 (guarded) | A5–A8, T10 |
| Largest file (lines) | 4,577 | 4,577 | < 800 | A7 |
| Duplicate-versioned crates (root lock) | 94 | 94 | < 30 (deny-gated) | D1–D3 |
| `rand` versions in tree | 3 (root) / 4 (desktop) | 2–3 | 1 | D2 (desktop done: advisory cleared) |
| ROUTE_TABLE command coverage | 15/341 (4%) | ~16 | 341/341 (gated) | B4 |
| Commands returning `Result<_, String>` | ~149 | ~149 | 0 | B5 |
| Dead `ObserverEvent` variants | 5/10 | 5/10 | 0 | B2 |
| `StatusUpdate` `#[non_exhaustive]` | no | no | yes | B1 |
| Persistent log sink | none | none | rolling file + ring | A3 |
| `/api/health` real readiness | no | no | yes | A4 |
| `cargo-deny` workspace coverage | 1 of (2 + 28 sub) | 2 (desktop advisories) | all (advisories) | D4, T-sub |
| Desktop dependency advisories | 3 | **0** | 0 (gated) | **#130 done** |
| `--locked` CI coverage | desktop + primary root | primary root + desktop | all jobs | #129 done, T9 |
| Signed desktop release | no | no | yes | P3 |
| Production hot-path panics (audited) | ~10 sites | fixed in #128 | 0 + lint | #128, A11, B-lint |
| Blocking-in-async (audited) | 4 sites | 2 fixed (#127) | 0 + clippy gate | #127, async-clippy |
| Observer events emitted | 5/10 | 5/10 | 10/10 | B2 |

## Guardrail catalog

Each architectural invariant must be machine-enforced. Status: ✅ live · 🟡 ready, add with its task ·
⛔ blocked (prereq noted).

| Guard | Enforces | Status / prereq |
|---|---|---|
| `cargo check --locked` on all gates | lockfile drift | ✅ primary root (#129) + desktop; 🟡 extend to all jobs (T9) |
| `cargo-deny` per workspace (advisories) | unscanned supply chain | ✅ desktop advisories (#130); 🟡 sub-workspace lockfiles |
| crate-boundary grep (`use thinclaw::` / wrong edges) | dependency direction | ✅ root-import; 🟡 add db→agent + gateway→tools guards (A1, A2) |
| god-file size guard (`.rs > N lines` fails CI) | god-file regrowth | ⛔ blocked until A5–A8 bring all ≤ threshold (T10) |
| `ROUTE_TABLE` coverage test | unclassified dual-mode commands | 🟡 with B4 |
| `clippy::await_holding_lock = deny` | async std-lock deadlocks | ✅ already warn-by-default under `-D warnings` |
| `clippy::unwrap_used = warn` | new production panics | ⛔ blocked by `-D warnings` (must decouple first) |
| `deny.toml multiple-versions = deny` | duplicate-version creep | ⛔ blocked until D1/D2 dedup |
| `wit-bindgen` single-version check | WASM interface skew | 🟡 with T11 |
| bundle-reference resolution test | broken registry bundles | 🟡 with P5/T11 |
| coverage threshold (`--fail-under`, no `--lib`) | silent coverage erosion | 🟡 with T1 |
| `export_bindings` no-hand-edit + variant-coverage test | binding drift | ✅ exists; extend per B-tasks |
| MSRV verification job | accidental MSRV bump | 🟡 with T8 |

## Re-audit procedure

After each wave, re-run the parallel audit to refresh **Current** and catch regressions:

```
Workflow: robustness-architecture-audit   (10 dimensions, grounded, ~12 min)
→ diff metrics vs this table → update Current → flag any new High/Critical findings into BACKLOG.
```

The refactor is "done enough to declare extreme robustness" when: every **Target** is met, every
guard in the catalog is ✅, and a fresh audit surfaces no new Critical/High findings.
