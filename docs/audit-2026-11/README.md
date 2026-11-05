# ThinClaw 2026-11 Audit — Remediation Tracker

> **STATUS — OPEN (t.b.d.).** Audit completed 2026-11-05 against `main` @ `5bebe1c` (2026-08-13).
> No workstream has started. This directory is the live tracker: every finding has an owner workstream, every workstream has ordered tasks, and each task is closed by a PR that ticks its box here and flips the finding's status in [`FINDINGS.md`](./FINDINGS.md).
>
> This is a **new** effort. The June remediation under [`../remediation/`](../remediation/README.md) is closed and stays a historical record — do not reopen it. Where a June fix was re-verified, that is noted in `FINDINGS.md` §4.

## What this audit covered

| | |
|---|---|
| **Baseline** | `main` @ `5bebe1c` (last commit 2026-08-13; last CI run 2026-08-17) |
| **Code** | 27 crates + root package (~545k Rust LOC), Tauri desktop (~107k), iOS/watchOS (~40k Swift), 16 WASM channels + shared webhook lib, 12 WASM tools |
| **Also** | GitHub Actions, release pipeline + published artifacts, Docker/compose/deploy scripts, vendored patches, docs-vs-code drift |
| **Method** | Eight parallel read-only reviews split by trust boundary (gateway, tool execution, extensions/WASM/MCP, secrets + channel ingress, agent runtime, data + repo-projects + experiments, client surfaces, supply chain + completion). Every Critical and most Highs were then re-traced in code a second time (marked **✔** in `FINDINGS.md`). Local `cargo clippy --workspace --all-targets` and `cargo test --workspace --lib` on Rust 1.94.0; repo's cargo-free CI scripts; live GitHub CI / release / PR state. |
| **Not done** | `cargo deny` / RustSec advisory scan (advisory DB unreachable from the audit sandbox — re-run in CI once green), dynamic/exploit testing, Postgres-backed integration targets. |

## Executive summary

The core is strong — sound secret crypto, a well-built WASM HTTP sandbox, SSRF/DNS pinning, hardened Docker sandbox, MCP stdio isolation, a careful iOS client — and **every June-audit fix held**. The problems are operational decay plus a systemic gap in *who is allowed to make the agent act*:

1. **`main` is red** since the 2026-08-13 commits (4 code regressions + 2 expired CI gates), freezing ~50 dependency PRs for three months → **WS-01**.
2. **Install paths are broken:** v0.15.0 and v0.16.0 published with zero assets, installer URL 404s, `:latest` image is still v0.14.0 → **WS-02**.
3. **Approval classes don't survive delegation:** `spawn_subagent` (no approval) can grant itself `shell`/`http`/`write_file`, which then run in Autonomous mode without prompting → **WS-03**.
4. **No owner vs. stranger authority tier:** several channels admit unknown senders by default, every sender gets the same tool profile, and approvals route to whoever requested them → **WS-04**.
5. **Web UI XSS → gateway token theft** via an agent-controlled canvas `panel_id`, no CSP → **WS-05**.
6. **WASM extensions can name any operator secret** for injection into requests to their own host → **WS-06**.

## Workstreams

Priority: **P0** = blocks everything / Critical exposure; **P1** = High; **P2** = Medium/Low + completion. Effort: S ≤ 2 days, M ≤ 1 week, L ≤ 2 weeks.

| WS | Title | Pri | Effort | Depends on | Findings | Doc |
|---|---|---|---|---|---|---|
| WS-01 | CI recovery & dependency backlog | P0 | S–M | — | CI-1…CI-10 | [WS-01](./WS-01-ci-recovery.md) |
| WS-02 | Release, distribution & deploy hardening | P0 | M | WS-01 | OPS-1…OPS-11 | [WS-02](./WS-02-release-and-distribution.md) |
| WS-03 | Delegation & approval integrity | P0 | L | WS-01 | **C1**, H1, H4, TOOL-2, TOOL-3, TOOL-7, TOOL-11, AGT-2, AGT-4, AGT-7 | [WS-03](./WS-03-delegation-and-approval-integrity.md) |
| WS-04 | Principal authority & channel ingress | P0 | L | WS-01 | **C2**, SEC-1…SEC-4, SEC-6…SEC-9, AGT-3, AGT-5, GW-5 | [WS-04](./WS-04-principal-authority-and-channel-ingress.md) |
| WS-05 | Web UI & client surfaces | P0 | M | WS-01 | **C3**, UI-1…UI-10 | [WS-05](./WS-05-web-ui-and-client-surfaces.md) |
| WS-06 | Extension trust boundary | P0 | L | WS-01 | **C4**, H5, EXT-1…EXT-9, TOOL-4, SEC-5 | [WS-06](./WS-06-extension-trust-boundary.md) |
| WS-07 | Gateway remote-access hardening | P1 | M | WS-01, WS-05 (CSP) | H3, GW-1…GW-10 | [WS-07](./WS-07-gateway-remote-access.md) |
| WS-08 | Tool policy hardening | P1 | M | WS-03 (`prepare_tool_call`) | H2, TOOL-5, TOOL-6, TOOL-8…TOOL-10, AGT-6, AGT-8 | [WS-08](./WS-08-tool-policy-hardening.md) |
| WS-09 | Repo-project supervisor & experiments | P1 | M | WS-01 | H6, DATA-1…DATA-3, DATA-5…DATA-7, DATA-9, DATA-10 | [WS-09](./WS-09-repo-projects-and-experiments.md) |
| WS-10 | Data layer & crypto correctness | P2 | S | WS-01 | DATA-4, DATA-8, SEC-10 | [WS-10](./WS-10-data-layer-and-crypto.md) |
| WS-11 | Half-wired features & doc drift | P2 | M | trails every WS | COMP-*, DRIFT-* | [WS-11](./WS-11-half-wired-and-doc-drift.md) |

## Status tracker

Update this table in the same PR that changes a workstream's state. `⏳` not started · `🔧` in progress · `👀` in review · `✅` done · `⏸️` deferred (with reason).

| WS | Status | Tasks done | Tracking PR(s) | Notes |
|---|---|---|---|---|
| WS-01 | ⏳ | 0 / 13 | — | Gate for everything else |
| WS-02 | ⏳ | 0 / 12 | — | Needs WS-01 green |
| WS-03 | ⏳ | 0 / 11 | — | |
| WS-04 | ⏳ | 0 / 13 | — | |
| WS-05 | ⏳ | 0 / 12 | — | 5.1–5.4 are the Critical slice |
| WS-06 | ⏳ | 0 / 15 | — | 6.1–6.4 are the Critical slice |
| WS-07 | ⏳ | 0 / 13 | — | |
| WS-08 | ⏳ | 0 / 9 | — | |
| WS-09 | ⏳ | 0 / 13 | — | |
| WS-10 | ⏳ | 0 / 4 | — | |
| WS-11 | ⏳ | 0 / 4 groups | — | Trailing doc/completion pass |

## Waves

```text
Wave 0  WS-01 ───────────────► (main green, dependabot drained)
                │
Wave 1          ├─► WS-02  (re-publish installable release)
                ├─► WS-03 ─┐   Critical security slices, in parallel
                ├─► WS-04  │
                ├─► WS-05 ─┼─┐
                └─► WS-06  │ │
Wave 2              WS-08 ◄┘ │   (needs WS-03's unified prepare_tool_call)
                    WS-07 ◄──┘   (needs WS-05's CSP / no-inline-handler UI)
                    WS-09
Wave 3              WS-10, WS-11 (WS-11 doc pass also trails each wave)
```

### Shared-file conflict register

Serialize edits to these files across workstreams (rebase, don't co-edit in parallel branches):

| File | Workstreams |
|---|---|
| `src/tools/execution.rs` (`prepare_tool_call`) | WS-03 (owner), WS-08 |
| `src/agent/dispatcher/tool_execution.rs` | WS-03, WS-04, WS-08 |
| `crates/thinclaw-tools/src/execution.rs` (`approval_required`) | WS-03, WS-04 |
| `crates/thinclaw-gateway/src/web/static/app.js` | WS-05 (owner), WS-07 |
| `crates/thinclaw-gateway/src/web/auth.rs` | WS-07 |
| `crates/thinclaw-tools/src/wasm/wrapper.rs` | WS-06 |
| `crates/thinclaw-channels/src/wasm/**` | WS-04, WS-06 |
| `src/NETWORK_SECURITY.md`, `docs/SECURITY.md`, `FEATURE_PARITY.md` | all — append-only per PR, rebase often |

## Decision register

Each decision must be signed off by the maintainer before the dependent task starts. Record the outcome inline (date + choice).

| ID | Decision | Options | Recommendation | Blocks | Outcome |
|---|---|---|---|---|---|
| D1 | Authority model for non-owner principals | (a) Restricted profile + owner-routed approvals; (b) deny all tools; (c) per-channel config only | **(a)** — keeps group/shared use viable while making the owner the only approver of non-`Never` tools | WS-04.1 | t.b.d. |
| D2 | Group chats when no owner is bound | (a) groups off; (b) per-sender pairing in groups; (c) current (open) | **(a)** default, **(b)** opt-in | WS-04.3 | t.b.d. |
| D3 | Subagent approval semantics | (a) spawn requires strictest class of granted tools; (b) subagents run Interactive, approvals routed to parent requester; (c) clamp grants to parent's `Never` set | **(a)+(c)** now, (b) later if UX needs it | WS-03.1 | t.b.d. |
| D4 | Self-repair loop | (a) disable automatic repair until rebuilt tools can be registered; (b) fix registration + keep auto; (c) delete | **(a)** — it currently cannot restore a tool and only adds risk | WS-03.7 | t.b.d. |
| D5 | Default approval for WASM / un-annotated MCP tools | (a) `UnlessAutoApproved` + manifest read-only allowlist; (b) keep `Never` | **(a)** | WS-06.9 | t.b.d. |
| D6 | Native-plugin gates | (a) local config only (not DB/gateway settable); (b) gateway-settable by admin (current) | **(a)** — matches the documented "operator-only" trust boundary | WS-06.8 | t.b.d. |
| D7 | Trusted-proxy mode role | (a) explicit configured role, refuse empty `TRUSTED_PROXY_IPS` while a tunnel is active; (b) keep Admin | **(a)** | WS-07.4 | t.b.d. |
| D8 | Restoring installs | (a) backfill v0.16.0 assets; (b) cut v0.16.1 from green `main`; (c) hold until WS-03…06 land and ship v0.17.0 | **(b)** for install-path recovery, security fixes in v0.17.0 with an advisory | WS-02.1 | t.b.d. |
| D9 | Host FS confinement default | (a) enable `THINCLAW_HOST_FS_SANDBOX` by default in sandboxed mode; (b) keep opt-in and fix docs | **(a)** | WS-08.4 | t.b.d. |
| D10 | Disclosure | (a) handle Criticals via GitHub Security Advisories / private fork, publish after fix; (b) fix in the open | If the repository is public, **(a)** for C1–C4 and consider moving this tracker's Critical detail into the advisory | all | t.b.d. |

## How to work a task

1. Pick an unchecked task in a WS doc; note any blocking decision.
2. One focused PR per task (or tight task group). Title prefix: `fix(audit-WSxx.y): …` / `docs(audit-WSxx.y): …`.
3. Follow the repo's same-PR rule (`CLAUDE.md`): update the canonical doc(s) listed in the WS doc and `FEATURE_PARITY.md` when tracked behavior changes.
4. In the PR: tick the task box, set the finding's **Status** in `FINDINGS.md` to `Fixed (<PR link>)`, and update the status tracker row above.
5. Every security fix lands with a regression test that fails before the fix.

## Files

- [`FINDINGS.md`](./FINDINGS.md) — finding registry (IDs, severity, preconditions, verification, owner task, status) and detail for every Critical/High.
- `WS-01` … `WS-11` — one executable workstream each.
