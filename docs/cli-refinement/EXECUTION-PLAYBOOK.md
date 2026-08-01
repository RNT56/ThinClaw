# CLI Refinement Execution Playbook

> Execute this document in order. Do not begin from the inventory alone. The numbered workstream documents define task-level behavior and acceptance criteria.

## 1. Preflight and evidence capture

The executor starts by proving the target and preserving unrelated work.

```bash
pwd
git branch --show-current
git rev-parse --short HEAD
git status --short
df -h .
find .. -name AGENTS.md -print
```

Rules:

1. Work only in the ThinClaw checkout containing `Cargo.toml`, `src/cli/mod.rs`, and this directory.
2. Read any repository guidance found by the last command before editing.
3. Treat every pre-existing modification and untracked file as user-owned. Do not stage, reformat, move, or delete unrelated files.
4. If a required hot file already has unrelated edits, inspect the diff and either preserve it with a nonoverlapping patch or stop for operator coordination. Never use `git reset --hard`, `git checkout --`, or broad cleanup.
5. Require at least 20 GiB available before a clean full-release build or all-features/all-targets gate, matching `docs/BUILD_PROFILES.md`. Record `df -h` before and after heavy gates. Do not delete caches or user files to make room without explicit authority; CI is the required fallback when the local volume is below the gate.
6. Re-run source searches for named symbols; line numbers in the audit are not stable.
7. The drift-reconciled source baseline is commit `697118fb`. Before task 1, compare every source-owned audit area—not this dossier—to that commit:

   ```bash
   audit_baseline=697118fb
   git cat-file -e "${audit_baseline}^{commit}"
   git diff --name-status "$audit_baseline" -- Cargo.toml Cargo.lock .github src crates channels-src tools-src scripts apps/desktop clients apps/ios
   ```

   An empty diff means the source audit is still based on the tree being implemented. Any output is a **re-audit gate**, not an automatic failure or permission to overwrite user work: inventory the changed Clap enums, dispatch arms, Cargo features, setup-step enum, static/dynamic tool registration, channel catalogs, gateway/OpenAPI routes, and desktop shared clients; update files 00 and 06–08 plus their owner/proof rows; rerun the dossier validators; then record the new baseline commit. Do not begin CR-01 against stale counts or semantics. A commit containing only this temporary directory does not trip the scoped comparison.

Capture the baseline before behavior changes:

```bash
cargo fmt --all -- --check
cargo build --locked --features full --bin thinclaw
cargo test --locked -p thinclaw cli::
cargo test --locked -p thinclaw-agent command
cargo run --locked --features full --bin thinclaw -- --help
cargo run --locked --features full --bin thinclaw -- status
cargo run --locked --features full --bin thinclaw -- doctor
```

Do not run the full-build baseline locally when the 20 GiB gate is not met. Capture source/help evidence with existing artifacts or a lighter supported profile, mark the heavy baseline as CI-required, and do not claim it passed.

Record pre-existing failures in the implementation handoff. A known failure is not permission to broaden scope, and a new failure is never classified as pre-existing without reproducing it at the baseline commit.

## 2. Branch and commit model

Use one refinement branch. Land behavior in the following checkpoints so review and bisection stay practical:

| Checkpoint | Contents | May alter public behavior? |
|---|---|---|
| CP-1a | CR-01 typed output/exit/artifact envelope and global option contract | Yes |
| CP-1b | CR-01 once-only bootstrap, two-phase config adapter, shared gateway client | Yes |
| CP-1c | CR-01 unsafe PATH/shell removals, hidden internals, scoped reset | Yes |
| CP-1d | Two-phase config/profile/shared-service/channel-catalog foundation, credential/process manifests, private sidecar transport, single compiled typed desktop runtime builder, plus transactional setup flow/page/secret overhaul | Yes |
| CP-2a | CR-02 persistent agents/conversations/routines/jobs/config/memory | Yes |
| CP-2b | CR-02 skills/backup/model/gateway/process security and feature/backend service fixes | Yes |
| CP-2c | CR-02 exact tool/channel catalogs, deterministic registry, and operator parity gaps | Yes |
| CP-2d | CR-02 final runtime preparation, capability snapshot, doctor/status/boot | Yes |
| CP-3a | CR-03 tool/approval/activity identity and transport contracts | Yes |
| CP-3b | CR-03 bootstrap/history/render/navigation/layout | Yes |
| CP-3c | CR-03 unified authorized slash registry and generated surfaces | Yes |
| CP-4 | CR-04 command tree plus hidden compatibility forwarding | Yes |
| CP-5 | CR-05 tests, canonical docs, completion/help/profile snapshots | Documentation and guarantees |
| CP-6 | Delete `docs/cli-refinement/`; final full gate | Deletes only this temporary dossier |

Each checkpoint must pass `cargo fmt --all -- --check`, its targeted tests, `git diff --check`, and a review of `git diff --stat`/`git status --short` before proceeding. Do not combine CP-6 with unfinished code.

## 3. Required sequence

### Executable task ledger

Use this order in the next run. A row does not start until the preceding row's gate is green; tasks joined with `+` may share one checkpoint but still receive separate tests.

Testing is incremental: every behavior row adds its exact unit/integration cases and corresponding `tests/fixtures/cli_contract_manifest.json` entries in the same checkpoint. CP-1a may introduce the shared test helper, manifest schema, and validator skeleton needed by later rows. Order 27 is the CR-05.1 **closeout audit** of that accumulated harness, not permission to defer black-box coverage until after implementation.

| Order | Task IDs | Concrete gate before continuing |
|---:|---|---|
| 1 | CR-01.1 + CR-01.2 | Typed exit/output/artifact tests; no immediate handler bypasses the renderer. |
| 2 | CR-01.3 + CR-01.4 | Exhaustive command-intent/bootstrap count and global/scoped Clap matrix. |
| 3 | CR-01.5 | Mock gateway security/client suite; `send` migrated. |
| 4 | CR-01.6 + CR-01.7 | PATH sentinel and no-shell child-process tests. |
| 5 | CR-01.8 + CR-01.9 | Feature/OS hidden-help snapshots; ID-based sender approval/no-code-output; experiment-runner stdin/private/managed auth transport with no argv/API/UI/bootstrap bearer; reset dry-run/lease/symlink suite. |
| 6 | CR-02.1 + CR-02.6 + CR-02.13 | First land CR-02.1's neutral D-54 execution-policy/receipt/coordinator interfaces; then the two-phase resolver, encrypted-locator source service/direct Phase-A refs, checked credential-consumer schema, child-environment primitives, immediate secret/project/provider generic-argument removals, repository-project/MCP/experiment ID-only bindings/DTO parity, typed target metadata, and compiled-backend matrix; finally finish shared context wiring against that resolver. This foundation intentionally precedes CR-02.20 and CR-01.10. |
| 7 | CR-02.20 | Checked process-launch manifest and scanner; migrate every production child to descriptor-cleared environment/I/O/lifetime policy; private llama.cpp/vLLM/MLX auth transport and token-free desktop DTOs. |
| 8 | CR-02.7, then CR-02.21 | Land the one channel service/driver catalog and its active-coordinated mutation adapter first; only after that task is green, compile the four desktop builder modules, delete duplicated/orphaned assembly, and replace secret-bearing bridge maps with typed inputs plus exact resolver handles. Setup consumes only the resulting single path. These two tasks are serial, not an inclusive CR-02.7…CR-02.21 range. |
| 9 | CR-01.10 | All-27-page mapping against the shared catalogs; Quick/Advanced/edit plans; zero-mutation cancel; typed apply/continuation and secret-sentinel suites. |
| 10 | CR-02.17 | All-124 catalog fixture; contention/collision/rebind/atomic activation tests; no silent registry mutation. |
| 11 | CR-02.2 + CR-02.3 | Two-process durable agent/conversation tests and old-facade deny assertions. |
| 12 | CR-02.4 + CR-02.5 | Real routine run ID plus bounded job list/event/file gateway contract/OpenAPI tests. |
| 13 | CR-02.8 + CR-02.9 | Memory provenance and all-17-skills parity suites; channel behavior is already green from order 8. |
| 14 | CR-02.14 + CR-02.15 + CR-02.16 | Backup/passphrase, token reveal, and model-cost adversarial suites. |
| 15 | CR-02.18 | Four-kind/single-effect extension lifecycle, live-runtime activation, and guarded memory-delete suites. |
| 16 | CR-02.19 | Complete learning-admin/API parity, cursor, zero-network configure/static behavior, live probe/activation, effect/cost gate, truthful rollback-record, exact provider/scope-state, and exhaustive secret-source/migration suites. |
| 17 | CR-02.10 | Final registry identity parity, channel-driver facts, and multidimensional revisioned snapshot fixtures across profiles. |
| 18 | CR-02.11 + CR-02.12 | Status/doctor exit/schema/probe tests and prepare/seal/boot redaction/goldens. |
| 19 | CR-03.1 + CR-03.2 + CR-03.3 | All transport contracts compile; interleaved tool, approval-ID, and structured activity tests pass. |
| 20 | CR-03.4 | Authoritative primary-conversation bootstrap/history/model-attribution tests. |
| 21 | CR-03.5 + CR-03.6 + CR-03.8 | Markdown/security/viewport/navigation/accessibility/performance goldens. |
| 22 | CR-03.7 | Registry route exhaustiveness, authorization, canonical job vocabulary, generated REPL/TUI help. |
| 23 | CR-04.1 + CR-04.2 | Canonical tree parses; every current path/flag has one canonical or intentional removed/hidden mapping. |
| 24 | CR-04.3 | Canonical and compatibility adapters converge on the same typed requests/services with no copied business logic. |
| 25 | CR-04.4 + CR-04.5 | Naming/options/destructive semantics pass; common/expert help and canonical-only completion snapshots pass across profiles. |
| 26 | CR-04.6 + CR-04.7 | Alias warnings/sunset fixtures pass; explicit removals are absent and retained capabilities remain reachable or inventoried. |
| 27 | CR-05.1 | Isolated black-box harness proves state-root isolation, byte capture, timeouts, mock gateway use, secret-safe failures, and a checked proof/entity manifest with discoverable tests. |
| 28 | CR-05.2 | Every behavior/security/state/TUI/registry matrix case passes or has a mandatory named CI job; no silent skip. |
| 29 | CR-05.3 + CR-05.4 | Backend/platform/profile/release matrices and full workspace/excluded-workspace/desktop/OpenAPI quality gates pass. |
| 30 | CR-05.5 + CR-05.6 | Generated canonical docs/help/completions are clean; the no-wildcard coverage manifest resolves all 95 inventory entries, leaf rows/mutation policies, 124 IDs, dynamic origins, channels, profiles, 27 pages, and every credential-consumer/process-launch entry to executed tests/cases. |
| 31 | CR-05.7 | Adversarial source/diff audit has no unresolved high-impact finding. |
| 32 | CR-05.8 | Delete this directory and rerun the final no-reference and full quality gates. |

When a row changes shared OpenAPI/SSE/runtime contracts, run the desktop contract/backend checks in that row rather than deferring breakage to the final quality-gate rows.

### Wave 0 — Establish contracts and close unsafe paths

Execute the CR-01 safety foundation, then its explicit configuration dependency, in this order:

1. introduce typed command context, output mode, color policy, and exit outcome;
2. centralize environment bootstrap before dispatch;
3. scope runtime-only flags and add `ask`/channel selection parsing;
4. build the hardened `GatewayClient` and migrate one thin consumer as a contract test;
5. remove automatic PATH mutation and the raw TUI shell escape;
6. replace code-based sender approval with request-ID approval and replace experiment-runner argv/bootstrap/API bearer transport with the private auth-envelope protocol;
7. replace full reset with the scoped/dry-run/exclusive request;
8. hide internal entrypoints and freeze help snapshots;
9. land CR-02.1's neutral mutation policy/receipt/coordinator interfaces, then CR-02.6/CR-02.13's two-phase resolver, encrypted-locator secret-source contract, credential-consumer schema, repository-project/MCP/experiment ID-only migrations, typed target metadata, child-environment primitives, compile-aware backend/profile facts, and no-auto-migrate inspection path; finish shared `CliContext`/coordinator wiring against those types before continuing;
10. execute CR-02.20's descriptor/process-manifest migration across every production child, including private desktop-sidecar auth and token-free DTOs, before setup is allowed to launch any Apply-time process;
11. land CR-02.7's one channel service/driver catalog, credential-reference schema, active-coordinated mutation adapter, and selection/probe ports without starting listeners or probing during setup;
12. execute CR-02.21 against those types: wire one compiled modular desktop builder, remove duplicate/orphan source, and replace the generic secret overlay with typed non-secret inputs and purpose-bound resolver handles;
13. rebuild setup under CR-01.10 as typed draft/review/apply against those shared catalogs, with exact continuation and credential contracts.

The task-level CR-02.1/CR-02.6/CR-02.13/CR-02.20/CR-02.7/CR-02.21 exception supplies setup's mutation coordinator, resolver, backend, sanitized process launcher, service, channel, credential, and single desktop-assembly primitives before CR-01.10 closes the safety boundary. Catalog construction/input translation is side-effect-free; live activation/probes remain explicit. Every later handler still gets one context, output contract, error path, and setup/config source of truth.

### Wave 1 — Replace facades with durable/shared services

Execute the first half of CR-02:

1. reuse the already-green shared CLI/runtime service, D-54 coordinator/receipt, and channel-catalog foundations; every cached-state mutation gains its active/stopped policy and revision proof before any old facade/list is deleted;
2. replace the silent tool registry with the exact static catalog, origin-aware entries, and deterministic collision transactions;
3. wire `agents` through the DB-backed `AgentRegistry`;
4. replace public session semantics with durable conversations;
5. implement routine triggering against the same routine-run service;
6. extend and expose bounded job APIs through `GatewayClient`;
7. enrich memory-search results and add guarded deletion through the shared service;
8. align all 17 skill lifecycle operations through one shared admin service;
9. harden backup/passphrase, gateway reveal, and model-cost behavior;
10. add kind-complete extension lifecycle contracts and route generic activation through the live runtime API;
11. reconcile the complete learning gateway/agent administration surface through one `LearningAdminService`, including cursor pagination, static/live health, explicit effect/cost gates, truthful rollback-record semantics, and provider-secret migration.

Do not start command regrouping yet. Old public paths remain available during this wave, but they must call the corrected handler.

### Wave 2 — Make health and boot truthful

Finish CR-02:

1. define independent capability fact/dependency/health/approval types, `CapabilityItem`, `BuildFingerprint`, and `CapabilitySnapshot` in a crate/module usable by runtime, CLI, and TUI without reverse dependencies;
2. restructure startup into assemble/prepare/seal/present/run, including final scheduler jobs and routine registration before sealing;
3. prove sorted registry identities equal descriptors, then populate startup revision `N`; publish `N+1` for hot changes and expose per-domain durable/applied revision drift;
4. move `status`, `doctor`, boot, TUI startup, `/tools`, and channel views to the shared snapshot/catalog;
5. add bounded live probes and stable exit classification;
6. redact gateway credentials and remove duplicate boot claims.

The capability type lands before the TUI refactor. Do not let presentation modules become the source of truth.

### Wave 3 — Repair the TUI and slash-command surfaces

Execute CR-03 in this order:

1. define typed event/view-state types keyed by request or invocation ID;
2. preserve fields in the TUI channel adapter;
3. repair approval actions and unrelated-input behavior;
4. correlate interleaved tool/subagent events;
5. hydrate runtime model/agent/conversation/capability state and durable history;
6. add safe Markdown/wrapping/viewport calculations;
7. create typed drawers/modals/navigation and simplify the layout/copy;
8. extend the slash registry and generate REPL/TUI help/completion from it;
9. remove `/think` until it has real semantics.

Approval and event-correlation tests must pass before visual cleanup. A polished UI over lossy event state is not acceptable.

### Wave 4 — Apply the public information architecture

Execute CR-04 only after shared handlers exist:

1. introduce nested canonical groups without copying handler logic;
2. point old root variants at the canonical handlers as hidden aliases;
3. add deprecation metadata/warnings for human output only;
4. split common help from `help --all` expert help;
5. regenerate shell completions and help snapshots;
6. test every mapping in the migration table.

Unsafe or false implementations are not retained for compatibility: old `sessions` uses the new conversation handler, old `cron trigger` uses the real trigger, and `!command`/automatic PATH replacement remain gone.

### Wave 5 — Verify, document, and erase the plan

Execute CR-05:

1. run targeted contract and black-box tests;
2. run full workspace gates and backend/feature matrices;
3. update canonical docs and examples from actual generated help;
4. review every disposition and acceptance row from `INV-01` through `INV-95`;
5. delete `docs/cli-refinement/` with `apply_patch` or `git rm`;
6. rerun doc-link/help checks and the full gate after deletion.

## 4. Shared-file serialization

These are hot paths. Edit/review them serially in the order shown.

| Path/symbol | Work | Ordering rule |
|---|---|---|
| `src/cli/mod.rs` (`Cli`, `Command`) | flags, hidden internals, nested command tree, tests | CR-01 parsing contract first; CR-04 regrouping last. |
| `src/async_main/command_dispatch.rs` (`run_terminal_command`) | bootstrap, context, all handler forwarding | CR-01 centralizes; CR-02 replaces agents/sessions; CR-04 only rewires variants. |
| `src/async_main.rs` (`async_main`, registration sequence) | final capability snapshot and boot inputs | CR-02 only; do not mix with command-tree churn. |
| `src/setup/wizard/**`, `crates/thinclaw-app/src/setup.rs` | setup neutral types, 27-step consolidation, draft/apply, continuation, secrets | CR-02.6/02.13 resolver/backend plus CR-02.1/02.7 shared service/catalog foundations first; CR-01.10 owns behavior; CR-04 only adds canonical/legacy parsing. |
| `src/boot_screen.rs` | snapshot rendering | After final snapshot population exists. |
| `src/tui/mod.rs` | unsafe shell removal, typed state, layout/rendering | Remove shell in CP-1; defer the large event/UI refactor to CP-3. |
| `crates/thinclaw-channels/src/tui.rs` | lossless event adapter | Types first, adapter second, UI consumer third. |
| `crates/thinclaw-agent/src/command_registry.rs` | slash metadata | Extend after TUI local/forward handler contracts are explicit. |
| `crates/thinclaw-channels/src/repl.rs` | generated slash help/list | Registry first; delete static duplication afterward. |
| `src/cli/config.rs` and runtime settings loaders | source resolver | One owner; migrate consumers before deleting fallback paths. |
| `src/cli/repo_projects.rs`, `src/tools/builtin/repo_projects.rs`, repository-project gateway/API DTOs | four typed credential slots and local-create versus ID-only bind | CR-02.6 changes every adapter together; remove plaintext `value` before retaining any command alias. |
| `src/cli/experiments.rs`, `src/experiments/**`, `src/api/experiments/**`, experiment gateway/OpenAPI/Web UI/desktop clients | typed provider/profile DTOs and ephemeral runner-auth transport | CR-01.8 removes token transport/result leaks first; CR-02.6 migrates durable provider/env bindings. No intermediate commit may emit a bearer bootstrap command. |
| `src/cli/mcp.rs` and MCP config/auth consumers | non-secret env versus source-bound secret env | CR-02.6 lands the shared schema; CR-02.18 may change activation only, not credential storage. |
| `crates/thinclaw-gateway/src/web/types/{extensions,ws}.rs`, `src/channels/web/{handlers/chat.rs,ws.rs,types.rs,server.rs}`, both gateway/static-Web `app.js`/styles copies, agent pending-auth state/handlers, and desktop MCP settings/contracts | extension-auth session transport and deletion of raw token forms | CR-02.6 lands source bindings; CR-02.18 introduces the session endpoints/codecs and then removes every HTTP/WS/chat/Web/desktop token producer, decoder, persisted snapshot, and auto-activation branch in the same checkpoint. Preserve no dormant compatibility token DTO. |
| `crates/thinclaw-platform/src/process.rs`, every production `Command`/Tauri sidecar call site, and process-manifest CI | descriptor-backed launch/environment/I/O/lifetime policy | CR-02.6 lands environment/secret primitives; CR-02.20 lands the launcher/manifest and migrates one process class at a time. No raw launch or broad inherited environment remains after its class checkpoint. |
| `apps/desktop/backend/src/{engine,sidecar}/**` plus generated Specta/frontend runtime contracts | private local-inference auth transport and token-free DTOs | CR-02.20 migrates llama.cpp, vLLM, and MLX adapters with per-backend spawn captures before deleting old token fields. Update the pinned artifact when needed; never retain `--api-key`/secret env as fallback. |
| `apps/desktop/backend/src/thinclaw/runtime_builder.rs` and `runtime_builder/*.rs` | one compiled modular assembly path and typed credential inputs | CR-02.21 first proves active versus orphan symbols, then wires one extracted module at a time and deletes its duplicate block. Never make behavior/security edits only in an uncompiled fragment. |
| `src/cli/pairing.rs` and pairing store/gateway DTOs | request-ID approval and code redaction | CR-01.8 removes the code argv/output path; CR-04 only regroups the already-safe handler. |
| `src/cli/gateway.rs`, `message.rs`, `devices.rs`, `experiments.rs` | shared HTTP client migration | Add client and tests first, migrate one module at a time. |
| learning gateway routes/types, learning DB repositories, external-memory settings, desktop learning clients | one `LearningAdminService`, pagination, effects, provider schemas, and secret references | CR-02.19 owns the service/types; update gateway, agent adapters, CLI, OpenAPI, and desktop consumers in one contract checkpoint before removing old payload fields. |
| `Cargo.toml`, `crates/thinclaw-config/src/database.rs`, service cfg sites | profile/backend/service availability | CR-02.13 before capability snapshots or command regrouping; preserve release feature definitions. |
| `src/platform/*readiness*`, status/doctor CLI types, shared capability DTOs | neutral readiness profiles and per-platform check descriptors | CR-02.11 owns the rename/adapters; migrate Linux checks and add macOS/Windows providers before help/OpenAPI/desktop snapshots change. |
| `crates/thinclaw-channels-core/src/channel.rs` and every status adapter | invocation/request identity wire contract | CR-03.1/03.2; update OpenAPI/SSE/desktop fixtures atomically. |
| `crates/thinclaw-channels/src/tui.rs`, `src/channels/tui_channel.rs` | `TuiBootstrap` constructor chain | CR-03.4 after snapshot and durable conversation selection exist. |
| `crates/thinclaw-tools/src/registry.rs`, `src/tools/registry.rs`, registration call sites | static catalog, collision outcomes, provenance, final tool descriptors | CR-02.17 lands before other registration rewiring; CR-02.10/02.12 consume the sealed registry. |
| channel descriptors, `src/cli/channels.rs`, setup/web channel adapters | one service/driver catalog and activation policy | CR-02.7 owns catalog/migration; all consumers migrate before `KNOWN_CHANNELS` is deleted. |
| `apps/desktop/backend/src/thinclaw/bridge.rs`, `commands/rpc_orchestration.rs`, and desktop route consumers | local/remote child-session capability boundary | CR-02.10 preserves the typed `LocalOnly` pre-mutation gate; CR-03 transports identity; never substitute embedded local work for a selected remote profile. |
| `apps/desktop/backend/src/thinclaw/commands/rpc_channel_config.rs` and channel settings clients | channel field values and credential-source binding | CR-02.6 defines the secret source; CR-02.7 migrates backend/API/desktop together. Keep `secret_binding_available=false` until the authorized source-ID adapter is actually live. |
| `apps/desktop/backend/src/thinclaw/fleet.rs` and capability/status UI consumers | task/capability projection | CR-02.10 moves them to the shared versioned snapshot; preserve absent task identity and delete heuristic capability construction only after consumer migration. |

Do not mechanically rewrite `src/cli/mod.rs` and `command_dispatch.rs` together without compiling between steps. Clap nesting errors and lost feature-gated variants are easiest to isolate incrementally.

## 5. Implementation invariants

### 5.1 One handler per behavior

Canonical commands, hidden aliases, REPL commands, TUI commands, and gateway-backed commands may have different adapters, but must converge on the same domain service. No alias owns persistence, validation, authorization, or formatting logic.

```text
parse/adapter -> typed request -> domain service -> typed result -> output renderer
```

### 5.2 Output boundary

- stdout: requested data only;
- stderr: diagnostics, deprecations, progress, and human warnings;
- JSON: exactly one versioned command envelope;
- JSONL: one versioned typed event envelope per line;
- human: tables/prose permitted, honoring color policy;
- no handler calls branding/color helpers directly;
- durable credentials are source-ID-only before reaching ordinary request/result renderers; gateway/device deliberate display and the private experiment-auth artifact use their dedicated guarded raw-secret paths;
- process argv, generated shell/provider text, ambient/unclassified environment maps, OpenAPI/generated clients, logs, and capability/setup snapshots never carry a secret value or sender code; an exact descriptor-declared consumer env slot is injected only inside the process launcher after authorization and ambient clearing, never through an ordinary DTO/renderer;
- record, stream, and artifact commands follow the distinct stdout/stderr rules in CR-01.2 and the leaf matrix;
- `--output-format` is presentation, `--out` is an artifact path, and raw completions/artifacts are never wrapped.

### 5.3 State boundary

- direct-DB commands use the configured persistent store and explicit principal/channel scope;
- gateway commands use `GatewayClient` and return the accepted resource/run/job ID;
- process-local runtime state is described as process-local and is never presented by a fresh terminal process as durable state;
- mutating commands support explicit confirmation where consequences are destructive;
- destructive dry-runs report the exact selected objects and counts.
- every mutating leaf declares D-54 ownership. An active cached-state owner is reached through its authenticated service or the command fails before writing; stopped direct execution uses the same domain service, and results carry durable/live revisions plus exact application state.
- setup pages own a draft, not live settings; before explicit Apply, Back/Cancel cannot mutate DB schema/data, files, secure store, extensions, or process environment;
- registry insertions return typed outcomes; a collision or lock condition can never silently omit or replace a capability.
- persisted credential consumers contain only schema-purpose bindings; the checked consumer manifest and centralized resolver own every exception, while ephemeral experiment/desktop-sidecar auth is never persisted as a runtime setting. Every child launch is also present in the checked process manifest and starts from the descriptor's sanitized environment rather than inheriting ambient state.

### 5.4 Capability vocabulary

Compile inclusion, static configuration, runtime registration, dependency readiness, policy exposure, approval policy, and live health are independent dimensions. Never turn them into an ordered stage or substitute one for another. `unknown`, `not_probed`, and `not_supported` are first-class states. Every negative/unknown result uses one of the stable reason codes in CR-02.10 and may add safe remediation text.

## 6. Verification cadence

After each task cluster:

```bash
cargo fmt --all -- --check
git diff --check
cargo check --locked --features full --bin thinclaw
```

After each checkpoint:

```bash
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked -p thinclaw
cargo test --locked -p thinclaw-agent
cargo test --locked -p thinclaw-channels
cargo test --locked -p thinclaw-app
```

Before completion:

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo deny check
git diff --check
```

The workspace command does not cover excluded channel/tool workspaces or the desktop backend. Run the existing CI-equivalent matrices in CR-05.3/CR-05.4 as separate gates. `--all-features` also requires the documented ALSA, `wasm32-wasip2`, and `wasm-tools` prerequisites; absence is a blocked local gate, not permission to change features or skip CI.

If a command is unavailable because a local tool such as `cargo-deny` is not installed, install it only with operator approval or rely on the matching CI gate. Do not mark the local gate passed when it was skipped.

## 7. Recovery and rollback

- A failing checkpoint is fixed in place; later waves do not begin on a red checkpoint.
- If a compatibility alias causes parser ambiguity, keep the canonical path and implement the legacy path as a hidden explicit enum variant that forwards to the same typed request. Do not weaken canonical parsing.
- Classify probes with the shared `ProbeOutcome`: not requested → `not_probed`; no safe probe implementation → `not_supported`; attempted timeout → `timeout`; attempted DNS/connect/transport failure → `unreachable`; rejected credentials → `auth_failed`; isolation/policy refusal → `policy_denied`; malformed/unexpected response → `protocol_error`. External failure remains a complete typed report (with exit `3` only when required by the selected profile), never a panic or generic operational error.
- If one DB backend is unavailable locally, retain backend-neutral code, run the available backend tests, and leave completion blocked on CI or an environment with the missing backend.
- Revert only the executor's own checkpoint commit through normal Git history. Never erase unrelated working-tree changes.

## 8. Handoff format at every checkpoint

Record:

1. task IDs completed;
2. behavior changed and old aliases affected;
3. files changed;
4. tests run with pass/fail/skip;
5. remaining known failures;
6. whether output, state, safety, and documentation contracts were reviewed;
7. the next executable task ID.

The final handoff additionally states that canonical docs were updated and proves `docs/cli-refinement/` is absent.
