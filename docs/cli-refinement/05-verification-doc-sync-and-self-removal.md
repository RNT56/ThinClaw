# CR-05 — Verification, Canonical Documentation, and Self-Removal

- **Priority:** release blocker
- **Depends on:** CR-01…CR-04 complete
- **Final action:** delete `docs/cli-refinement/`

## CR-05.1 — Build a black-box CLI contract harness

Add integration tests using `std::process::Command` with `CARGO_BIN_EXE_thinclaw` (or the repository's established binary harness). Each test receives an isolated temporary ThinClaw home/config/database and controlled environment. Never read or mutate the developer's real ThinClaw state.

Required test layout:

```text
tests/cli_contract.rs                parser, output, exit, help, aliases
tests/cli_persistence_libsql.rs      two-process agents/conversations/config
tests/cli_gateway_contract.rs        send/routines/jobs/devices client behavior
tests/cli_safety_contract.rs         setup PATH, token/ANSI/control leakage
tests/cli_setup_contract.rs          27-page mapping, draft/apply, continuation, secret safety
tests/tool_registry_contract.rs      static catalog, collisions, provenance, final identity parity
tests/process_launch_contract.rs     descriptor coverage, environment/auth transport, bounds/cleanup
tests/cli_docs_contract.rs           help/reference/completion drift
tests/cli_coverage_manifest.rs       inventory/entity/proof-ID reconciliation
tests/fixtures/cli_contract_manifest.json
                                      durable proof ID → exact test/case and entity coverage
tests/fixtures/credential_consumer_manifest.json
                                      durable secret consumers and exact purpose/binding contracts
tests/fixtures/process_launch_manifest.json
                                      every production child launch and process-security descriptor
scripts/ci/check-cli-contract-manifest.py
                                      validates test discovery and runs manifest reconciliation
scripts/ci/check-process-launches.py  rejects raw/unclassified production process launches
scripts/ci/check-desktop-runtime-builder.py
                                      rejects orphan/duplicate builder code and generic secret maps
```

Harness requirements:

- construct the binary command in one helper;
- clear ThinClaw-related environment before applying fixture values;
- create an isolated `THINCLAW_HOME` and explicit libSQL location;
- capture stdout/stderr as bytes and text separately;
- support global `--output-format`/color/config flags before and after canonical subcommands where Clap allows them;
- parse JSON/JSONL rather than matching substrings alone;
- enforce timeouts for commands that can contact a runtime;
- use a local mock server for gateway tests; no public network dependency;
- redact fixture secrets in assertion failure output.

The checked manifest has `schema_version`, unique `proof_id` records, an exact Cargo integration-test target plus test name (and a fixture `case_id` for parameterized tests), and covered entity IDs. Entity IDs include `INV-01`…`INV-95`, canonical leaf IDs with their D-54 execution policy, all 124 static tool IDs, allowed dynamic-origin classes, channel service/driver IDs, all 27 setup step IDs, every current/target setup-profile value, all readiness-profile values, supported build/release profile IDs, and every entry in `credential_consumer_manifest.json` and `process_launch_manifest.json`. Credential entries use exactly the D-53 dispositions and the checker rejects a wildcard/regex ignore, an incomplete class-specific lifecycle, or an unproved `non_secret_semantic` exemption. Proof IDs may contain letters, digits, `_`, and `-` only; `*`, ranges, placeholder suffixes, and “existing tests” are invalid.

`check-cli-contract-manifest.py` must discover compiled tests with `cargo test --locked --test TARGET -- --list --format terse`, fail when a named target/test/case is absent or ignored without a named mandatory CI job, and invoke `cli_coverage_manifest`. Before CP-6, that test parses files 00 and 06–08 plus both checked consumer/process manifests and proves every row/identity references a manifest proof. After CP-6, it validates the same durable entity sets from generated Clap/catalog/setup/credential-schema/process-descriptor metadata and `docs/CLI_REFERENCE.md`; deleting the dossier therefore cannot delete the regression contract. The ordinary test matrix executes every referenced target, so existence alone is not accepted as proof. `check-process-launches.py` additionally scans production Rust launch constructors/wrappers, rejects any site not owned by the typed platform/desktop adapter and descriptor registry, and compares the generated launch manifest byte-for-byte. `check-desktop-runtime-builder.py` verifies the compiled module graph, unique assembly symbols, and absence of generic secret-bearing bridge inserts.

Land the shared helper, manifest schema, and validator skeleton in CP-1a, then add exact proof records beside each behavior change. CR-05.1 closes and audits the accumulated harness at order 27; it is not the first point at which black-box tests are written.

## CR-05.2 — Required test matrix

### Parsing, information architecture, and migration

- canonical root/common/category/leaf paths parse;
- every CR-04.2 legacy path maps to the same typed request and exit/result as canonical;
- deprecated paths warn once on human stderr and remain silent in JSON/JSONL stdout;
- invalid global/runtime-only flag placement fails through Clap;
- `-m` conflicts with explicit subcommands and follows its one-release lifecycle;
- `--cli-only` is exactly `--channels none` while retained;
- unsafe `secrets set --value`, repository-project `set-credential --value`, extension `--token`/chat-token submission, backup `--passphrase`, sender-approval positional codes, experiment provider `--payload-json`, and internal experiment-runner `--token` are rejected immediately with secret-free migration guidance; retained adapters cannot reintroduce a plaintext argument path;
- default help, category help, and `help --all` have the intended visibility;
- `help PATH` is metadata-only and matches `PATH --help`; `help --all PATH` conflicts, unknown/compatibility/internal paths are rejected, and no help form loads state or contacts a runtime;
- help/version remain deterministic raw parser information before/after global options, contain no banner/progress/deprecation text, exit 0, and follow the documented no-JSON-wrapper exception;
- internal and compatibility tokens are absent from help and completions in every feature snapshot.

### Output, color, secrets, and exit classes

- JSON empty/nonempty outputs parse for agents, conversations, memory, channels, status, doctor, routines, jobs, skills, and configuration;
- JSONL is one valid object per nonempty line;
- no banner, ANSI, warning, or progress text appears in machine stdout;
- `NO_COLOR`, piped/non-TTY auto mode, explicit never/always policy, quiet/verbose conflicts;
- secrets/gateway/experiment-lease/pairing-code/desktop-sidecar sentinels are absent from human, JSON, debug, error, boot, TUI, OpenAPI/generated clients, Web UI DOM, process argv, ambient child environment, generated shell/provider text, and non-Secret deployment manifests except the separately tested guarded gateway/device display paths, private experiment-auth artifact, and descriptor-declared private pipe/file/native-secret bytes;
- operational failure exits `1`; Clap usage exits `2`; required doctor/live-status failure exits `3`; valid degraded optional state exits `0`; foreground interrupt exits `130`.
- record JSON uses the versioned envelope, stream JSONL uses versioned event envelopes, and raw artifact/completion stdout is never wrapped.
- `--readiness-profile server|remote|desktop|pi-os-lite-64|all-features` is canonical/defaulted exactly, hidden `--profile`/desktop aliases map identically through 0.18, mixed old/new flags conflict, and readiness/setup/build profiles never overwrite one another in JSON or UI state;

### Durable agents and conversations

- two independent processes add/show/set-default/remove an agent and a runtime registry load observes the same DB state;
- missing agent operations fail nonzero;
- a persisted conversation written in one process is listed/shown/exported in another;
- message ordering, pagination cursors, roles, timestamps, tool records, and previews are stable;
- conversation prune dry-run and execution select the same IDs/count, with `--yes` required;
- hidden `sessions` paths call durable conversation behavior, never `SessionManager::new()`.

### Mutation ownership and live coherency

- every mutating canonical/compatibility/Web/desktop/agent adapter resolves the same generated `MutationExecutionPolicy`; no leaf lacks or overrides the domain policy;
- `MUTATION-policy-routing` has one discoverable parameterized `case_id` per finite mutating leaf plus exact `run`/`tui` lifecycle cases; the coverage checker rejects family-only or wildcard claims;
- `embedded_runtime`, `durable_immediate`, `active_coordinated`, `runtime_required`, `stopped_exclusive`, `owned_process_lifecycle`, and `external_direct` fixtures cover runtime stopped, active/healthy, active/unreachable, stale/wrong instance record, selected remote profile, restart-required, process/external IDs, and partial apply;
- active agent/routine/channel/model/config/credential-binding/skill/extension mutations are observable through the live service and snapshot at the returned revision; stopped writes load on next startup;
- an active cached-state owner that is unreachable causes zero durable/artifact mutation and no local router/manager construction; retry request IDs are idempotent;
- same-ID/same-keyed-fingerprint retry returns the original receipt/resource, different-fingerprint reuse conflicts, concurrent claims execute once, the 24-hour/10,000-entry receipt bounds prune only terminal rows, and journals contain no raw/unkeyed secret hash, prompt/note/file content, locator, or request payload;
- persist-then-live failure either restores the previous durable revision or returns exact durable/applied revisions, `partial=true`, `restart_required`, and recovery—never `applied_live`;
- setup Apply, reset, backup import, and master-key rotation reject active/ambiguous ownership before mutation and serialize under the same exclusive lease;
- human/JSON receipts agree on `applied_live`, `restart_required`, or `runtime_not_running`, and status/doctor/boot/desktop expose the same revision drift without secret field names.

### Gateway-backed actions

- `send` uses resolved auth and returns an accepted ID;
- routine trigger creates one real run, returns `run_id`, and that ID appears in history;
- job list/summary/show/cancel/restart/prompt/events/files cover direct and sandbox projections, bounded list/event pagination, and no advertised follow mode;
- devices and experiments use the same client after migration; experiment provider connect/validate/launch-test requests exactly match their path DTOs and contain no generic payload wrapper;
- redirects are not followed, tokens are not forwarded, response bodies are bounded, timeouts are bounded, and API errors retain safe context;
- named gateway/probe/file budgets pass at exactly the cap and return typed failures one unit over; endpoint-specific stricter caps remain stricter and no retry resets the aggregate deadline;
- ownership/auth failures and stopped runtime are nonzero, never simulated success.

### Configuration, channels, memory, skills, and capabilities

- two-phase config precedence/provenance across defaults/legacy migration/DB/TOML/env/CLI overrides, including proof that bootstrap DB keys never depend on DB rows;
- explicit missing config fails; atomic write failure preserves the old file/value;
- sensitive config paths are refused/masked in every output mode;
- channel `check-config` performs no network I/O; `probe` reports supported/unsupported/auth/network/timeout distinctly;
- one channel catalog contains local/native/lifecycle/all-16-bundled/installed sources; overlapping drivers select once; `--channels none` disables native/lifecycle/gateway/WASM ingress;
- memory search returns rank, score, path, document/chunk IDs, and safe citation;
- memory delete dry-run/mutation uses the shared protected-path policy and confirmation;
- all 17 skills CLI operations and runtime tools report the same identity/provenance/trust/scan outcomes; no fictitious enable/disable state appears;
- portable/manager extension kinds include MCP, WASM tool, WASM channel, and native plugin; list filters all four; auth/activate/remove selectors resolve an explicit kind and reject omitted ambiguity before mutation;
- extension auth sessions enforce owner/kind/purpose binding, 128-bit identity, ten-minute expiry, one-use state/S256-PKCE, CAS state transitions, restart invalidation, replay/rate/audit controls, source-write rollback/refresh rotation, and mutually exclusive OAuth/pre-authorized-source/local-secure-create modes; `/api/chat/auth-token`, `AuthTokenRequest`, WebSocket `AuthToken`, and free-form pending-auth token capture are absent from production/generated contracts; remote completion is source-ID-only and clients without secure ingress fail `secure_input_unavailable`;
- extension auth has no activation side effect and returns `next_action=activate`; activation handles a typed auth challenge without error-string matching; CLI, agent, gateway, and desktop activation produce the same live registered identities/revision; stopped-runtime CLI activation constructs no manager and mutates nothing; native removal reports unload plus retained operator artifacts;
- learning status/history/candidates/versions/feedback/proposals/outcomes/rollback records and external-memory administration share one principal-scoped service across CLI, agent adapters, gateway, OpenAPI, and desktop clients;
- learning CLI principal/actor identity comes from authenticated/local context and exposes no caller-controlled cross-principal selector; gateway cross-principal operations require the existing admin override plus audit path;
- every learning list uses the declared stable keyset cursor (including one cursor per kind for `history all`), rejects mismatched filters/principals, and never reports an unusable `has_more`;
- learning status, external-memory status, and configure-only provider writes make zero provider calls by default; `--live` probes all eight providers with declared concurrency/time bounds and typed partial failures;
- proposal approval previews exact file/Git/publish effects and is dry-run by default noninteractively; outcome `evaluate-now` discloses and bounds billable LLM work; outcome review enforces its decision/verdict matrix;
- learning rollback creation is named `record`, returns `artifact_restored=false`, and never claims to restore an artifact;
- all eight external-memory providers have exact accepted/rejected option schemas and independent configured/enabled/active/scope-safe facts; Letta activation is denied while strict isolation is absent;
- external-memory configure preserves an omitted existing `enabled` value, defaults a new provider to enabled without probing, rejects material changes to an active provider, and requires explicit deactivate/configure/activate for a switch; same-provider activation is idempotent and cross-provider activation is non-mutating conflict;
- inline primary/embedding/header secrets and arbitrary secret-bearing config are rejected, mutable settings use `SecretBinding`/`SecretSourceId` end to end, and all legacy secret-like map fields are verified/migrated without appearing in logs/output/snapshots;
- source-record validation covers generated-ID/label bounds, owner/exact-purpose/revision authorization, encrypted/authenticated locator storage with row-swap/tamper rejection, encrypted-store metadata versus OS-account distinction, environment-name grammar, and absolute/symlink/hard-link/size/owner/Unix-mode/Windows-ACL file defenses with open-time recheck; remote/agent DTOs can select only pre-authorized opaque source IDs and cannot inject encrypted-store names, OS accounts, host paths, or environment names;
- `config secrets set --value` is absent immediately; masked/stdin/env/file creation, mutual-exclusion and non-TTY behavior, admin-only principal override, 64-KiB boundaries, in-use deletion refusal, OS-backed whole-store master-key rotation/rollback plus operator-managed-master refusal, source ID output, and libSQL/PostgreSQL source-record parity are exact fixtures;
- the checked credential-consumer manifest classifies every settings/Clap/HTTP/WS/OpenAPI/agent/desktop/Web-UI/dynamic-manifest/process secret-like candidate under the exact D-53 taxonomy; mutable repository-project, MCP/extension, experiment, LLM/embedding/media/integration/coding-worker/tunnel, channel, and external-memory credentials persist bindings and use only authorized resolvers; pre-DB infrastructure alone uses private local `bootstrap_direct`; ephemeral/protocol/reveal/non-secret records satisfy their distinct contracts with no generic JSON/env/URL/argument escape hatch or keyword-only exemption;
- the compiled desktop runtime builder contains no generic secret-bearing bridge/env map: Gmail/gateway/LLM/embedding/MCP/channel/media/coding-worker credentials reach only exact owning services through authorized handles, and legacy keychain migration follows the source-record transaction;
- repository-project local creation uses exact four slots and masked/stdin/env/file rules; its agent tool and gateway bind a source ID only, the out-of-band request tool never returns user credential text to the model transcript, and a client without secure ingress fails `secure_input_unavailable` without treating chat text as a secret;
- experiment runner profiles use bounded secret-free file schemas plus `--secret-env NAME=ID`; GPU providers use `--credential-source ID`; target link/update uses the shared ten-kind metadata schema from a bounded file; invalid legacy metadata is blocked/redacted; legacy secret names migrate only when unambiguous;
- experiment runner authentication proves the 4-KiB stdin/private-file/managed-secret matrix, owner/mode/ACL/replacement defenses, single-use/expiry/replay behavior, initial/resume/reissue/auto-advance sink coverage, previous-lease revocation, staged private atomic `--auth-out` with publish-failure revocation, `awaiting_secure_delivery`, managed-backend delivery, and fail-closed manual/provider-without-secret-channel outcomes;
- MCP `--env` accepts only non-secret entries, `--secret-env` requires a purpose-authorized ID, credential-bearing URL/command arguments are rejected, and OAuth/manual credentials use the same source registry without auto-activation;
- the checked process-launch manifest accounts for every production std/Tokio/Tauri/shell launch and records exact program/digest-or-search, argv, cwd/filesystem, env, home/temp, auth, network/isolation, I/O/deadline/tree/cleanup policy; its scanner rejects raw or stale sites;
- process captures across utility, shell/build/sandbox/bridge, runtime/service re-exec, channel/tunnel/MCP/coding-worker, backup, experiment, and desktop-sidecar classes on Unix/Windows strip unrelated ambient credential sentinels and parent home/temp/search-path hazards, and inject only an exact approved consumer slot into an eligible isolated/direct consumer without argv/shell/log duplication;
- adversarial executable shadowing/current-directory `PATH`, symlinked or broad-permission home/temp, credential-helper files, unavailable required isolation, and host-unconfined labeling/approval fixtures prove sanitized environment is not misrepresented as containment;
- llama.cpp/vLLM/MLX chat, embedding, summarizer, and STT fixtures deliver generated auth only through the proven private pipe/file adapter and in-memory endpoint registry, leave process-global environment unchanged, contain no token field in any serializable/Specta/frontend/runtime DTO, clean up at every spawn/readiness/exit failure boundary, and fail closed when the pinned backend lacks its transport;
- capability fixtures distinguish compile/configuration/registration/dependency/exposure/approval-policy/health/unknown/not-probed/not-supported independently;
- all 124 static catalog IDs have one descriptor/disposition; each profile contains exactly its compiled/registered subset;
- registry contention cannot omit a tool; every static/dynamic collision is rejected or an explicit same-source rebind; MCP activation is atomic;
- late tool/agent/routine registrations appear in startup revision `N`, and hot changes produce `N+1`;
- `status tools` and `/tools [NAME] [--all]` render the same population and sorted identities/facts for revision `N`; exact unknown versus empty multi-filter exits differ as specified; every independent fact filter is exercised; hot `N+1` replaces atomically and stale/replayed revisions are ignored.
- status-scope flags parse before and after `tools`, normalize to the same request, reject conflicting duplicates, and are rejected outside the `status` subtree; generated help prints only the canonical before-subcommand spelling.
- no source or test `cfg` consumes `repl`, `web-gateway`, or `timezones`; the existing host-runtime smoke is discovered/run under the real gateway profile rather than an empty alias; none appears in any 0.17 aggregate; enabling each retained empty alias independently produces identical test discovery and byte-identical generated help/static capability metadata to the base set; declarations are removal-gated for 0.19;

### Reset, backup, gateway reveal, model cost, and artifacts

- reset scope parsing/deduplication, exact dry-run parity, active-runtime lease refusal, double confirmation, backend transactions, symlink/root-path defense, and untouched service/binary sentinels;
- backup passphrase-source/leak scans, PostgreSQL private-pgpass/escape/cleanup and no-child-password scans, partial versus `--require-database`, no final artifact on required-section failure, overwrite policy, import lease, and manifest/path validation;
- gateway credential fixture appears only in confirmed TTY `--reveal-token`, never in machine/redirected/quiet/log/debug/boot/TUI output;
- gateway start/reload does not place its parent-only PID correlation nonce or bearer token in child argv/environment, and stale/mismatched private PID records remain non-mutating;
- model verify makes zero chat requests by default and an exact bounded count only under confirmed `--chat-probe`; `test`/`sync` behavior is classified correctly;
- every artifact leaf obeys stdout-versus-`--out`, overwrite, atomic-write, and domain-format rules from the leaf matrix.

### Onboarding and TUI safety/state

- onboarding cannot create directories/symlinks or change/remove a sentinel PATH target;
- all 27 current setup step IDs map to a tested target section; every current setup-profile value/alias maps to the six retained presets or Advanced/No-preset adapter; Quick visibly defaults balanced rather than hidden builder-coding; full Advanced/every edit topic/profile/renderer has a deterministic plan fixture;
- secret-free headless setup input enforces schema/version/1-MiB cap/unknown-field rejection, dry-run and exact plan digest, non-TTY `--yes`, run/output conflicts, and zero mutation on stale digest or baseline;
- explicit setup/edit exits; `--run` and first-run Run/TUI/Ask continuations preserve exact intent across renderer fallback;
- Back/cancel before Apply leaves DB schema/data, files, secure-store metadata, extensions, and process environment byte-identical;
- Apply baseline conflict/action ordering/lease, every failure boundary, partial recovery, and retry are typed; no persistence error is swallowed;
- environment master-key mode never generates/prints/writes a key, local mode never falls back to plaintext, legacy credential migration is verified, and setup emits no token URL;
- TUI `!command` cannot launch a child process;
- interleaved tool invocation IDs update the correct records;
- approvals retain request ID/tool/description/parameters and explicit scope;
- unrelated input/wrong IDs do not dismiss or resolve approval;
- subagent names/IDs survive progress events;
- desktop-managed child-session spawn/list/update stays local-only: selecting a remote profile returns the typed capability gate before mutation, while local mode retains its existing behavior; no adapter translates it into an agent-tool call;
- desktop fleet fixtures consume the shared capability revision and preserve absent task identity instead of inventing “Ready,” connection-count tasks, or heuristic capabilities;
- desktop/gateway channel configuration round-trips non-secret fields, exposes only authorized credential-source metadata, and keeps credential mutation disabled while secret binding is unavailable; raw password/token values never enter DTOs or settings;
- desktop engine/sidecar start/status results expose endpoint readiness metadata only; local bearer material remains a redacted non-serializing backend value and never appears as an empty placeholder field in generated bindings;
- every retained `thinclaw/runtime_builder/*.rs` file is in the compiled module graph, no assembly symbol/block has a duplicate active/orphan definition, and modular local startup preserves final service/tool/channel identities plus remote `LocalOnly` routing;
- structured plan/usage/context/status updates do not overwrite each other;
- authoritative startup model/agent/conversation/capabilities and durable history hydrate correctly;
- Markdown/control sanitization, Unicode wrapping, resize, visual-row scrolling, and stream-follow behavior;
- navigation closes typed views without deleting transcript/activity.
- 1,024-entry event-channel saturation coalesces only replaceable state, never drops terminal events, and applies backpressure; the 501st completed activity, 201st diagnostic, 8-KiB preview, and 100/default–500/max history-page boundaries evict, truncate, or reject exactly as specified and report that fact;

### Slash registry

- every declared local route has an exhaustive handler;
- every forwarded route parses to the expected `Submission`/system route;
- help/autocomplete contain only commands supported on that surface;
- aliases are unique and exact-or-space matching does not swallow chat text;
- `/debug`, `/skin`, `/status`, `/rewind`, `/plan`, and `/restart` match their declared strategy;
- `/tools`, `/tools NAME`, and `/tools --all` use the local revisioned snapshot on REPL/TUI, while any agent-message route is separately authorized;
- `/think` is absent from registry/help/completion/docs and returns removed-command guidance if invoked.

## CR-05.3 — Backend, platform, and feature coverage

Run locally where supported and require CI for unavailable environments:

| Dimension | Required coverage |
|---|---|
| DB | libSQL black-box persistence on every PR; PostgreSQL contract parity in service-backed CI. |
| Features | exact profile rows from `06-feature-command-and-output-matrix.md`: edge, light/default, desktop, full, minimal-libSQL, minimal-PostgreSQL, all-features, the three empty-compatibility deltas separately/together, and selected real-feature deltas. Each gets compile, parser/help/completion, and static capability snapshots. |
| Release | cargo-dist full artifacts on all configured macOS/Linux GNU/Linux musl/Windows targets plus explicit edge GNU/musl x86_64/aarch64 artifacts; smoke their packaged `--help`, `status`, backend default, and hidden-entrypoint contract. |
| OS | Linux and macOS CLI/setup/service tests; Windows parser/service/hidden-dispatch/help tests in Windows CI. Platform-native channels/tools get present/absent capability fixtures. |
| Terminal | no-color, non-TTY, narrow/wide TUI golden buffers. |
| Runtime state | stopped, starting/unavailable, healthy, degraded, and auth-failed gateway. |
| Configuration | no config, explicit valid/invalid path, DB values, environment override, legacy JSON migration. |

Tests requiring PostgreSQL or platform services may be gated by explicit CI environment availability, but their absence blocks release sign-off. They do not silently pass as “skipped.” `integration` and `schema-divergence` remain test gates and must never appear in user capability output.

Keep `.github/workflows/ci.yml`'s existing profile matrix as the floor rather than replacing it with a smaller custom job. Add the help/capability/CLI-contract commands to each row. Continue to build/test the excluded `channels-src/*` and `tools-src/*` workspaces through their existing matrices, and run the desktop backend/frontend contract gates whenever shared CLI/runtime/SSE/OpenAPI types change.

## CR-05.4 — Full quality gate

Preflight disk again before the heavy gate. Require 20 GiB free and install/verify the documented all-feature prerequisites (`libasound2-dev` on Linux, `wasm32-wasip2`, and `wasm-tools`) before claiming this gate:

```bash
df -h .
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
cargo deny check
cargo build --locked --features full --bin thinclaw
   git diff --check
```

Then run focused binaries/snapshots with an isolated state root:

```bash
cargo test --locked --test cli_contract
cargo test --locked --test cli_persistence_libsql --features libsql
cargo test --locked --test cli_gateway_contract --features full
cargo test --locked --test cli_safety_contract --features full
cargo test --locked --test cli_setup_contract --features full
cargo test --locked --test tool_registry_contract --features full
cargo test --locked --test cli_docs_contract --features full
cargo test --locked --test cli_coverage_manifest --features full
python3 scripts/ci/check-cli-contract-manifest.py
```

Run the supported profile checks exactly (CI may parallelize them):

```bash
cargo check --locked --workspace
cargo check --locked --workspace --no-default-features --features edge
cargo check --locked --workspace --features full
cargo check --locked --workspace --all-features
cargo check --locked --workspace --no-default-features --features desktop
cargo check --locked --workspace --no-default-features --features libsql
cargo check --locked --workspace --no-default-features --features postgres
```

For every row, run `cli_docs_contract` with that row's feature arguments so help/completion/capability snapshots are profile-aware. Also run:

```bash
scripts/build-all.sh
cargo run --locked --example export-openapi -- check
cargo check --locked --manifest-path apps/desktop/backend/Cargo.toml
```

When shared runtime/SSE/OpenAPI/status/channel/subagent contracts or a desktop adapter changed, run the consumer gate explicitly rather than relying on the single backend check:

```bash
cargo test --locked -p thinclaw-runtime-contracts
cargo test --locked -p thinclaw-config provider_catalog
(cd apps/desktop && npm ci)
(cd apps/desktop && npm run contracts:check)
(cd apps/desktop && npm run validate:naming)
(cd apps/desktop && npm run lint:agent-styles)
(cd apps/desktop && npm run lint:ts)
(cd apps/desktop && npm test)
(cd apps/desktop && npm run build)
cargo clippy --locked --manifest-path apps/desktop/backend/Cargo.toml --all-targets -- -D warnings
scripts/ci/desktop-fixture-acceptance.sh
```

The CI desktop job additionally runs browser E2E, every supported backend engine profile, dependency policy/advisories, Tauri metadata, and the unsigned cloud build; those remain mandatory release gates. If OpenAPI changed, regenerate/check the committed gateway specification first, then run `apps/ios/scripts/check-generated-drift.sh` and the iOS `generated-drift` CI job so the vendored spec/generated Swift client cannot lag. If runtime-contract DTOs changed and `swiftc` is present, type-check `clients/swift/ThinClawRuntimeContracts/Sources/ThinClawRuntimeContracts/RuntimeContracts.swift`; otherwise the matching CI job is required.

The bundled-WASM script requires its documented target/tool prerequisites. Keep the focused test filenames above so commands remain copy/paste-valid. A missing tool, unavailable platform, grace-period generator skip, or skipped command is recorded as blocked—not passed—and must be satisfied by its named mandatory CI gate before release.

## CR-05.5 — Generate and update canonical documentation

Add `src/bin/export-cli-reference.rs`, a deterministic developer generator that walks Clap command metadata, CLI availability/D-54 mutation metadata, the slash registry, static tool catalog, channel catalog, setup section metadata, credential-consumer catalog, and safe process-descriptor metadata. Because `autobins = false`, register it explicitly as a non-default `[[bin]]` in `Cargo.toml`. A full/all-feature invocation updates marked generated regions in `docs/CLI_REFERENCE.md` containing the canonical tree, leaf options, output/exit/mutation-effect contract, profile availability, operator-versus-agent capability dispositions, channel drivers, setup flow, and non-secret process/credential policy identities; per-profile tests construct that build's metadata and compare help/completion/capability snapshots. `cli_docs_contract` fails when checked-in output differs. The generator must exclude compatibility/internal commands, secret source locations/values, force color off, and never probe runtime/network state.

Update these durable documents after code is final:

| Document | Required update |
|---|---|
| `README.md` | Canonical quick-start examples (`setup`, default/run, `tui`, `ask`, `send`, `status`) and links. |
| `docs/CLI_REFERENCE.md` | Generated command tree/options/output/exit contract; direct-DB vs running-runtime requirements; compatibility note. |
| generated capability reference/section in `docs/CLI_REFERENCE.md` | Exact live/static tool descriptor vocabulary, agent-only versus shared CLI decisions, dynamic provenance, channel driver variants, and `status tools`; never a universal count claim. |
| `docs/SURFACES_AND_COMMANDS.md` | Registry-generated REPL/TUI slash matrix; remove `/think` claims; explain local vs forwarded handlers. |
| `docs/README.md` | Ensure navigation points to canonical CLI/surface docs; do not link this temporary directory. |
| `src/setup/README.md` | Canonical setup/edit/mode/continuation paths, ten target sections, draft/review/apply boundary, renderer fallback, secret sources, and explicit statement that setup does not install/symlink the binary. |
| `docs/TERMINAL_SKINS.md` | Color auto policy, `NO_COLOR`, no styling in JSON/JSONL, compact TUI chrome. |
| `docs/DEPLOYMENT.md` and `docs/deploy/**` | `runtime web` full-headless semantics, `runtime service` paths, package-owned installation/PATH. |
| `docs/SECURITY.md` | No raw TUI shell escape, request-ID approvals, redacted gateway access, setup/source-record encryption and credential-consumer policy, no pre-Apply mutation, no plaintext CLI/extension-chat/internal-runner/local-sidecar bearer arguments, source-bound extension OAuth/manual-auth sessions, process executable/home/temp/filesystem/network/isolation policy and honest `host_unconfined` boundary, secure experiment and desktop-sidecar auth delivery, direct DB/gateway trust boundaries, active-runtime mutation ownership/no-fallback/idempotency, and high-risk tool origins. |
| `docs/AGENT_ENV.md` | Actual config precedence, scoped runtime flags, channel selection, deprecated environment/flags. |
| `docs/BUILD_PROFILES.md` | Correct REPL/TUI/service availability, compile-aware backend defaults, CLI-browser versus agent-browser distinction, and profile capability table. |
| `docs/EXTENSION_SYSTEM.md` | Canonical activation/channels/tools/registry/MCP/skills paths, source-bound OAuth/manual auth sessions with no raw remote token form or implicit activation, collision/provenance behavior, one channel catalog/driver model, and multidimensional capability vocabulary. |
| `docs/SKILLS_ECOSYSTEM.md` | Skills admin surface and shared catalog/quarantine lifecycle. |
| `docs/MEMORY_AND_GROWTH.md` | Durable conversations, memory citation/delete policy, and `data learning` administration versus agent-only learning operations. |
| `docs/REPO_PROJECT_SUPERVISOR.md` | Canonical `automation projects/jobs` commands, four credential slots, local source creation, and ID-only agent/gateway binding. |
| `docs/RESEARCH_AND_EXPERIMENTS.md` | Canonical `labs experiments` path, typed provider/profile/ten-kind target-metadata inputs, source-ID secret env, blocked-legacy remediation, runner-auth delivery matrix, and manual-only backend remediation. |
| `docs/RELEASING.md` | 0.17 introduction, 0.18/0.19 alias removal checks, generated help/reference gate. |
| installer/service error strings and examples | Replace `onboard`, `service`, `gateway`, and other moved paths with canonical commands. |
| OpenAPI/client docs | Add manual routine trigger `run_id` and any job client types changed. |
| packaged completions/manpages/installers | Regenerate from canonical metadata and smoke each release profile; no compatibility/internal tokens. |

Run a stale-reference audit after updates:

```bash
rg -n "thinclaw (onboard|cron|gateway|service|message send|sessions|repo-projects|trajectory|pairing|tool|registry|mcp|memory|backup|browser|comfy|secrets|models|channels|identity|devices|experiments|logs|update)" README.md docs src scripts
rg -n "thinclaw secret( |$)|--no-onboard|--skip-auth|pause_after_completion|KNOWN_CHANNELS" README.md docs src scripts crates
rg -n "(/think|!command|cli-only|Single message mode|Manage active sessions|cron jobs)" README.md docs src scripts
rg -n -- "--output (human|json|jsonl)|doctor --live|trajectories inspect|skills (enable|disable)" README.md docs src scripts
rg -n -- "--value|--passphrase|--token|bootstrap_command|payload_json|secret_reference|env_grants_json" README.md docs src scripts crates apps/desktop
rg -n -- "--api-key|THINCLAW_MLX_API_KEY|PGPASSWORD|THINCLAW_GATEWAY_INSTANCE_TOKEN|metadata_json|/api/chat/auth-token|AuthTokenRequest|NostrPrivateKeyRequest|ProviderKeyRequest|mcp_auth_token" apps/desktop src crates README.md docs scripts
```

Every match must be one of:

- an explicit versioned migration note;
- a credential-free internal invocation or an explicitly private test fixture;
- a test fixture proving compatibility;
- corrected before completion.

Do not keep aspirational statements. Examples are copied from passing help/black-box commands.

## CR-05.6 — Coverage reconciliation

| Inventory | Owning implementation | Required proof |
|---|---|---|
| INV-01…INV-05 | CR-01.1…CR-01.4 | bootstrap/parser/output/exit black-box suite |
| INV-06…INV-08 | CR-02.10…CR-02.12, CR-04 | capability/boot golden and runtime-web help tests |
| INV-09…INV-11 | CR-01.5, CR-01.8 | gateway security and feature help snapshots |
| INV-12…INV-15 | CR-01.6, CR-04.1/CR-04.4 | setup sentinel plus reset dry-run/confirmation tests |
| INV-16…INV-19 | CR-01.4, CR-04 | run/tui/ask/send parser and local-vs-gateway tests |
| INV-20…INV-21 | CR-02.11 | status/doctor static/live/exit/schema tests |
| INV-22…INV-24 | CR-02.6, CR-04 | config precedence/secrets/models migration tests |
| INV-25…INV-28 | CR-01.8, CR-02.4/CR-02.5/CR-02.6, CR-04 | routine run ID, job API, project credential bindings, experiment DTO/runner-auth security, and projects/experiments mapping |
| INV-29…INV-34 | CR-02.7/CR-02.9/CR-02.10, CR-04 | channel static/live, skills parity, extension mappings |
| INV-35…INV-39 | CR-02.2/CR-02.3/CR-02.8, CR-04 | memory provenance, two-process state, data mappings |
| INV-40…INV-49 | CR-01.5/CR-01.8, CR-04 | access/runtime/media/dev/completion mapping and help tests |
| INV-50…INV-53 | CR-01.7, CR-03.1/CR-03.2 | no-shell, interleaved tool, request-ID approval tests |
| INV-54…INV-62 | CR-03.3…CR-03.6 | structured activity/startup/history/render/navigation goldens |
| INV-63…INV-69 | CR-03.7, CR-05.5 | registry consistency and generated canonical docs |
| INV-70…INV-73 | CR-02.10/CR-02.13, CR-05.3 | profile help/capability snapshots, backend defaults, service gates, execution readiness |
| INV-74…INV-75 | CR-01.1/CR-01.2, CR-04.2 | output-option/artifact matrix and stable exit-class black-box tests |
| INV-76…INV-80 | CR-01.9, CR-02.5/CR-02.14…CR-02.16 | backup/model/token/job/reset security and behavior suites |
| INV-81…INV-83 | CR-01.8, CR-02.6/CR-02.10/CR-02.13, CR-05.2/CR-05.3 | multidimensional snapshot, final registry inventory, exhaustive leaf and credential-consumer contracts |
| INV-84 | CR-02.10/CR-02.18, CR-04 | all-124 static identity catalog, dynamic-source dispositions, selective CLI parity, generated capability reference |
| INV-85 | CR-02.17 | contention/collision/rebind/atomic-activation/provenance tests and registry-to-snapshot identity parity |
| INV-86 | CR-02.10/CR-02.12/CR-02.17 | prepare/seal ordering, final job/routine registration, startup revision and hot-update tests |
| INV-87…INV-89 | CR-01.10 | 27-page mapping, quick/advanced/edit plans, continuation, zero-mutation cancel, apply failures, and secret-sentinel suite |
| INV-90 | CR-01.4/CR-02.7 | full channel-source catalog, driver collision/migration, all-ingress `none`, setup/runtime/CLI identity parity |
| INV-91 | CR-02.19 | full learning API/CLI/service parity, cursors, static/live probes, effect/cost gates, truthful rollback records, provider schemas, and secret-reference migration |
| INV-92 | CR-01.8/CR-02.6/CR-02.14/CR-02.20 | checked process-launch descriptors, ambient-environment stripping, validated executable/home/temp/filesystem/network/isolation policy, honest host-unconfined reporting, runtime/service re-exec policy, private desktop-sidecar auth transport/no-token DTOs, gateway nonce removal, and PostgreSQL private-pgpass proofs |
| INV-93 | CR-02.6/CR-02.21 | compiled desktop module topology, duplicate/orphan deletion, typed non-secret runtime inputs, purpose-bound service credential resolution, legacy migration, and local/remote runtime parity |
| INV-94 | CR-02.6/CR-02.18 | delete HTTP/WebSocket/chat raw-token paths, owner/kind/purpose-bound expiring auth-session tests, OAuth PKCE/state/replay coverage, local secure-source ingress, source-ID-only remote completion, no implicit activation, and legacy token migration |
| INV-95 | CR-01.9/CR-01.10, CR-02.1/CR-02.2/CR-02.4/CR-02.6/CR-02.7/CR-02.9/CR-02.10/CR-02.18/CR-02.19 | generated mutation-policy coverage, active/stopped/unreachable/stale/remote routing, durable/live revision receipts, bounded idempotency/concurrency, hot-apply/restart/partial rollback, no local fallback, exclusive cross-domain operations, and snapshot drift parity |

Completion requires every inventory range above to have both implementation evidence and a passing proof. A test marked ignored without a mandatory CI job does not count. Every leaf row in the cross-cutting matrix must also name its concrete proof ID; range coverage alone cannot hide an untested leaf. `tests/fixtures/cli_contract_manifest.json`, both checked consumer/process manifests, `cli_coverage_manifest`, and the two CI checkers jointly prove that every proof ID names an existing executed test or fixture case and that no inventory/tool/channel/page/profile/process entity disappears when this temporary dossier is deleted.

## CR-05.7 — Adversarial review checklist

Before deleting the dossier, review the final diff specifically for:

- success messages preceding durable commit/accepted API response;
- accidental construction of `AgentRouter::new()` or `SessionManager::new()` in terminal dispatch;
- direct DB/TOML/artifact mutation of an `active_coordinated` domain while an owned runtime is active or ambiguously stale; a compatibility/desktop/remote path bypassing the coordinator; success without a durable/live revision receipt; or persisted/live drift rendered as current;
- command handlers printing directly instead of using the output boundary;
- secrets in `Debug`, URLs, JSON, boot/TUI buffers, errors, or snapshots;
- redirects, unbounded bodies/timeouts, or duplicated auth client construction;
- static checks labeled healthy/connected;
- event IDs/parameters dropped by any adapter;
- approvals resolvable without an explicit matching request ID;
- compatibility aliases with copied business logic;
- internals exposed under a less-common feature set;
- service/TUI/REPL incorrectly disappearing behind `repl`, or a libSQL-only build defaulting to PostgreSQL;
- presentation `--output-format` colliding with artifact `--out`/domain formats;
- paid model chat probes, partial backups, credential reveal, or destructive reset occurring without the declared opt-in/lease/confirmation;
- plaintext values/pairing codes/extension tokens/lease or desktop-sidecar bearers in argv, shell/provider bootstrap strings, free-form chat, generic JSON/env/desktop bridge maps, ambient child environments, API/OpenAPI/client/Specta DTOs, Web UI, trial summaries, or logs; a surviving `/api/chat/auth-token`, WebSocket `AuthToken`, or pending-message secret capture; manual experiment auth without a private single-use artifact, managed launches without a reviewed secret-delivery adapter, an empty serialized token placeholder masquerading as redaction, or a secret-like field waived by name/regex without a complete D-53 disposition and proof;
- raw production process constructors, unmanifested launch IDs/env keys, unbounded child output/deadlines, orphanable descendants, secret-bearing shell templates, and local inference backends falling back from private pipe/file auth to `--api-key` or environment credentials;
- tracked desktop runtime-builder fragments absent from the Rust module graph, duplicate assembly functions/blocks, source-string tests that bless one copy, or a secret inserted/read through `BRIDGE_VARS`/`optional_env` rather than an owning service resolver;
- unbounded job list/events or fictitious follow/cursor behavior;
- silent tool insertion/overwrite, incomplete reserved-name catalogs, incomplete extension-kind enums, name-only lifecycle ambiguity, auth-triggered activation, error-string auth classification, ambiguous channel names, or registry/snapshot identity drift;
- setup DB/secure-store/file/extension mutation before Apply, swallowed persistence failure, renderer-driven continuation, plaintext settings secrets, generated env keys, or tokenized setup URLs;
- docs/examples generated from memory rather than passing commands;
- pre-existing user changes accidentally included.

Use `rg` and review all matches, not only expected modules:

```bash
rg -n "AgentRouter::new|SessionManager::new|print_banner|println!|eprintln!" src/cli src/async_main/command_dispatch.rs
rg -n "ApprovalNeeded|ToolStarted|ToolResult|ToolCompleted|SubagentProgress" src crates
rg -n "GATEWAY.*TOKEN|token=|Authorization|bearer" src/cli src/boot_screen.rs src/tui
rg -n -- "--api-key|THINCLAW_MLX_API_KEY|PGPASSWORD|THINCLAW_GATEWAY_INSTANCE_TOKEN" src crates apps/desktop
rg -n "std::process::Command|tokio::process::Command|Command::new|\.sidecar\(" src crates channels-src tools-src apps/desktop/backend
rg -n "inject_bridge_vars|bridge_config\.insert|GMAIL_OAUTH_TOKEN|GATEWAY_AUTH_TOKEN|LLM_API_KEY|include_str!\(\"runtime_builder\.rs\"\)" apps/desktop/backend/src/thinclaw crates/thinclaw-config/src
rg -n "try_write|PROTECTED_TOOL_NAMES|register_sync|\.insert\(.*tool|KNOWN_CHANNELS" crates/thinclaw-tools/src/registry.rs src/tools src/cli/channels.rs
rg -n "persist_after_step|auto_configure_quick_runtime_defaults|generated_env_master_key|SECRETS_MASTER_KEY|token_url" src/setup crates/thinclaw-app/src/setup.rs
```

Direct printing may remain inside the centralized renderer and interactive REPL/TUI drawing code; each other match needs justification.

## CR-05.8 — Mandatory self-removal

Only after CR-05.1…CR-05.7 pass:

1. Confirm every checklist in `README.md` and CR-01…CR-05 is complete.
2. Confirm canonical docs contain all durable behavior and migration timelines.
3. Confirm no canonical doc links to this directory.
4. Delete every file in `docs/cli-refinement/` and the directory itself in CP-6. Use a scoped patch/deletion; do not run a broad recursive delete against `docs/`.
5. Re-run doc-link/reference checks, format/clippy/tests/deny, and `git diff --check` after deletion.
6. Verify:

   ```bash
   test ! -e docs/cli-refinement
   rg -n "docs/cli-refinement|CLI Refinement — Temporary Execution Dossier" README.md docs src scripts
   ```

   The first command must succeed. The second must return no matches.
7. The final implementation handoff lists the canonical docs, test results, compatibility deadlines, and confirms the dossier was deleted. Do not recreate an execution summary in its place.

## CR-05 definition of done

- [ ] Black-box harness isolates state and separates stdout/stderr.
- [ ] Required matrix passes across state/output/safety/gateway/TUI/registry concerns.
- [ ] libSQL and PostgreSQL contracts are proven in required environments.
- [ ] Default/full/all-feature and platform help surfaces are covered.
- [ ] Full quality gate passes without a disk-capacity failure.
- [ ] Canonical docs and generated CLI/slash references match shipped code.
- [ ] The checked coverage manifest contains no wildcard/placeholder proof and resolves every entity to an existing executed test or parameterized case.
- [ ] All 95 inventory entries, every leaf row and mutation policy, all 124 static tool IDs/dynamic origins/channel sources, all 27 setup step IDs, and every credential-consumer/process-launch manifest entry have implementation and proof.
- [ ] Adversarial review finds no unresolved high-impact issue.
- [ ] `docs/cli-refinement/` no longer exists.
