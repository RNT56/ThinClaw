# Rust coverage ratchet

ThinClaw's enforceable Rust line-coverage floor is measured from the
`cargo llvm-cov --workspace --all-features --lib` report produced by CI. The
ratchet complements, rather than replaces, the integration-test execution
matrix and the 70% changed-line gate.

## Evidence baseline and schedule

The latest successful `main` report before the ratchet was introduced is tied
to commit `06b4cfdeb3c961da6698136af91dc52bded819e1` and workflow run
`31254931581`: 155,404 covered executable lines out of 291,945, or 53.2306%.
CI enforces the schedule in `scripts/ci/coverage-ratchet.json`:

| Effective date | Project floor | Covered-line floor |
|---|---:|---:|
| 2026-08-13 | 53.2% | 155,404 |
| 2026-10-01 | 55.0% | 155,404 |
| 2026-12-01 | 57.5% | 155,404 |
| 2027-02-01 | 60.0% | 155,404 |

The percentage can therefore never return to the old 38% allowance, and
deleting covered code cannot silently satisfy the ratio by reducing both the
numerator and denominator. Moving a scheduled date or lowering either floor is
a policy change that requires explicit review, not routine debt maintenance.

## Coverage debt only shrinks

`scripts/ci/coverage-debt.json` contains digest-bound allowances for previously
uncovered lines. The gate fails when:

- a source digest is stale;
- the manifest gains more files or lines than the ratchet permits; or
- a line that is now covered remains recorded as debt.

When tests cover existing debt, or a refactor edits a baselined file, prune the
manifest with the exact LCOV report used for review:

```sh
python3 scripts/ci/prune-coverage-debt.py coverage.lcov --write
```

For a downloaded historical artifact whose `SF:` records contain another
checkout root, pass it explicitly:

```sh
python3 scripts/ci/prune-coverage-debt.py coverage.lcov \
  --source-prefix /home/runner/work/ThinClaw/ThinClaw \
  --write
```

The pruning command cannot add or rebaseline debt. It retires the whole entry
for a changed file and removes newly covered lines from unchanged entries.
Review and commit the JSON diff with the tests or refactor that paid it down.

## Local verification

Generate a report with the same feature and target selection as CI, then run
the policy gate:

```sh
cargo llvm-cov --workspace --all-features --lib \
  --lcov --output-path coverage.lcov -- --nocapture
python3 scripts/ci/check-coverage.py coverage.lcov \
  --base "$(git merge-base HEAD origin/main)" \
  --patch-min 70 \
  --debt-baseline scripts/ci/coverage-debt.json \
  --ratchet scripts/ci/coverage-ratchet.json
```

Do not add low-value execution-only tests to chase the ratio. Prioritize
authorization/safety denials, dispatcher cancellation and error paths,
persistence rollback/recovery, and extension admission/failure isolation.
