# CR-01 — CLI Contract, Safety, and Bootstrap

- **Priority:** P0
- **Depends on:** none
- **Blocks:** all other workstreams

## Scope and ownership

Primary targets:

- `src/main.rs`
- `src/cli/mod.rs`
- `src/async_main/command_dispatch.rs`
- `crates/thinclaw-app/src/runtime.rs`
- `src/terminal_branding.rs`
- `src/setup/wizard/summary.rs`
- `src/setup/wizard/{flow,persistence,infrastructure,channels_step,presentation}.rs`
- `crates/thinclaw-app/src/setup.rs`
- `src/tui/mod.rs`
- CLI gateway consumers under `src/cli/`
- new focused modules under `src/cli/` for context, output, termination, and gateway access

## CR-01.1 — Typed invocation context and terminal outcome

**Covers:** INV-01, INV-04, INV-05, INV-09.

Implement:

1. Add `src/cli/context.rs` with a `CliContext` built once per invocation. It owns resolved output/color policy, debug/verbosity settings, environment/config source metadata, and lazy accessors for shared DB/gateway services. Do not store mutable process-global presentation state in it.
2. Add `src/cli/output.rs` with:
   - `OutputFormat::{Human, Json, Jsonl}`;
   - `ColorChoice::{Auto, Always, Never}`;
   - an `Output` writer that has explicit `data`, `diagnostic`, and `progress` channels;
   - serialization helpers that receive typed values, not preformatted ANSI strings.
3. Add the typed termination/result types `CliOutcome` and `CliError` and map them at the binary boundary. Use stable process classes:
   - `0`: requested operation completed and all required checks passed;
   - `1`: invocation/operational/internal error (invalid request, I/O, auth, unavailable required dependency);
   - `2`: parser/usage error emitted by Clap;
   - `3`: a complete `doctor` or `status --live` report was produced, but one or more checks required by the selected readiness profile are unhealthy;
   - `130`: user interrupt for supported foreground CLI/runtime flows.
   - help/version retain Clap's successful exit; internal supervisor restart code `75` is not a public command result and remains owned by `RuntimeShutdownPlan`.
4. Convert `run_terminal_command` from `Option<anyhow::Result<()>>` to a typed dispatch result that preserves whether the command was handled and its exit classification. Do not call `std::process::exit` inside handlers.

Acceptance:

- a handler cannot report an unhealthy/failed typed result and accidentally exit zero;
- machine renderers cannot call `TerminalBranding`;
- every command outcome has one top-level exit mapping;
- runtime/restart/service behavior retains its existing special exit handling.

Tests:

- unit-test all termination mappings;
- black-box a successful command, invalid command input, and unhealthy doctor result;
- assert stdout and stderr independently.

## CR-01.2 — One output/color contract

**Covers:** INV-04 and all machine-output portions of INV-16…INV-49.

Implement:

1. Add global flags:

   ```text
   --output-format human|json|jsonl    default: human
   --color auto|always|never    default: auto
   --quiet                      suppress nonessential human diagnostics/progress
   --verbose                    add human diagnostics; conflict with --quiet
   --debug                      tracing/debug diagnostics
   ```

2. Do not add a global `--output`: backup/config/model/browser/trajectory/conversation commands already use it as an artifact path. Canonical path options become `--out`; retain every existing path-valued leaf `--output`/`-o` as a hidden alias through 0.18.
3. `auto` enables color only for a terminal destination. `NO_COLOR` forces no color unless the user explicitly supplied `--color always`. An empty `NO_COLOR` value still counts as set. Machine modes force no color regardless of `--color`; reject `--output-format json|jsonl --color always` as a usage conflict rather than contaminating output.
4. Move banners behind the human output path and show them only for interactive runtime/setup surfaces, never immediate data commands or non-TTY output.
5. Classify every leaf in the matrix as one of:
   - **record:** human table/prose or one JSON document; JSONL is allowed only for a natural sequence;
   - **stream:** human lines or JSONL events; reject JSON for an unbounded `--follow` invocation;
   - **artifact:** exact domain bytes/text to stdout or `--out`; when written to a file, `--output-format` controls only the status record. Reject machine presentation when it would share stdout with an artifact.
   - **parser information:** `-h/--help`, `help ...`, and `-V/--version` retain deterministic Clap-style raw text and successful short-circuit behavior. They do not bootstrap/load state and are not wrapped by `--output-format`; this exception is documented and snapshotted rather than accidentally falling through a handler.
6. Replace command-local presentation `--format table|json|text` flags with the shared output policy. Preserve domain artifact formats such as trajectory `jsonl|json|sft|dpo`, conversation `markdown|json`, and browser `text|html`; name those `--artifact-format` where collision is possible. Keep hidden parsing shims where compatibility requires them and reject contradictory old/new flags.
7. Use fixed machine envelopes:

   ```json
   {"schema_version":1,"command":"agents.list","data":[]}
   ```

   JSONL objects use `schema_version`, `command`, `type`, and `data`. Successful stdout contains data only. Operational machine errors leave stdout empty and emit an equivalent error envelope on stderr. `doctor`/live-status unhealthy outcomes still emit the complete report on stdout and exit `3`.
8. Completion scripts and stdout artifacts are raw and never wrapped. Human deprecations/warnings/progress go to stderr. Ensure empty record results serialize as the documented empty collection/object, not branded prose.
9. Redact secret-like values in typed data before serialization so changing output format cannot bypass masking.

Acceptance:

- `NO_COLOR=1 thinclaw status` contains no escape byte;
- `thinclaw --output-format json agents list` is valid JSON even when empty;
- piping human output suppresses banners and uses plain text;
- deprecations/progress never appear on stdout;
- `--quiet` does not suppress requested data or errors.

Tests:

- add process-level tests that search output bytes for `\x1b`;
- parse every JSON empty/nonempty fixture with `serde_json`;
- test TTY policy as a pure function rather than depending only on CI terminal state;
- snapshot human tables without ANSI.
- test record, bounded sequence, stream, stdout artifact, file artifact, completion script, and error envelopes as separate fixtures.
- test help/version before and after global flags as raw, banner-free, state-free parser information, including the documented machine-format exception.

## CR-01.3 — Centralize environment bootstrap

**Covers:** INV-01, INV-22, and the immediate commands currently missing bootstrap.

Current omission set includes tool, config, registry, repository projects, backup, MCP, memory, pairing, devices, service, ComfyUI, completion, browser, trajectory, update, and some internal entrypoints.

Implement:

1. In `src/main.rs`, classify the parsed command with `runtime_command_intent` and execute `RuntimeEnvBootstrapPlan` once, before configuration or terminal dispatch.
2. Remove every per-match-arm call to `execute_env_bootstrap_plan` from `run_terminal_command`.
3. Make bootstrap idempotence explicit and testable. No command handler calls dotenv/bootstrap again.
4. Build `CliContext` only after bootstrap so environment and explicit config inputs resolve consistently.
5. Keep service-runtime policy explicit: if service entrypoints intentionally skip dotenv, preserve that in `RuntimeCommandIntent` and test it.

Acceptance:

- every `ImmediateCli` invocation receives the same bootstrapped environment;
- bootstrap is called once, not zero or twice;
- commands observe values from ThinClaw environment overlays consistently;
- internal/service policy remains intentional and documented in code.

Tests:

- table-drive every `Command` variant through `runtime_command_intent`;
- instrument a bootstrap executor in tests and assert call count/policy;
- black-box at least config, memory, tool, backup, status, and update using an isolated `THINCLAW_HOME`.

## CR-01.4 — Scope flags to the commands that honor them

**Covers:** INV-02, INV-03, INV-16, INV-18, INV-19.

Implement:

1. Remove `cli_only`, `no_db`, `message`, and `no_onboard` from the global `Cli` shape. Keep `config: Option<PathBuf>` truly global, resolve/validate it in `CliContext`, and make every state-loading command honor it. Help/version may short-circuit before context construction, as expected.
2. Add canonical `Ask { text, ...runtime selection... }` and `Send { text, channel, ...gateway selection... }` variants.
3. Place `--no-db`, canonical `--skip-setup-check`, and `--channels` only on `run`, `tui`, or `ask` where the implementation consumes them. Keep `--no-db` explicitly testing/diagnostic-only and hide it from normal help. Keep `--no-onboard` as a hidden forwarding flag through 0.18. `setup` owns only setup-specific flags. `--config PATH` remains global and must be honored uniformly by every state-loading command.
4. Define `--channels` parsing:
   - `none`: disable all nonlocal ingress, including native, WASM, and web gateway listeners;
   - `configured`: existing configured activation policy;
   - comma-separated names: only the named, configured channels; unknown names are usage errors.
5. Preserve hidden compatibility parsing:
   - global `-m/--message` maps to `ask` for one minor release and conflicts with an explicit subcommand;
   - `--cli-only` maps exactly to `--channels none` for two minor releases;
   - warning goes to human stderr only.
6. Update `NativeChannelActivationPlan` and runtime assembly so `none` really disables native, gateway, and WASM ingress. Local REPL/TUI operation remains.

Acceptance:

- no ignored flag is accepted;
- `thinclaw status --no-db` is a Clap usage error;
- `thinclaw run --channels none` opens no nonlocal listener and registers no external channel;
- `ask` is local one-shot; `send` is gateway injection; help makes the distinction explicit.

Tests:

- exhaustive Clap parse matrix for before/after-subcommand placement, global `--config`, conflicts, aliases, and feature gates;
- runtime activation tests for none/configured/selected channel plans;
- black-box old `-m` and `--cli-only` warning behavior in human and JSON modes.

## CR-01.5 — Harden shared gateway access

**Covers:** INV-09, INV-19, INV-27, INV-42, INV-43.

Add `src/cli/gateway_client.rs` with:

- one URL resolver with precedence documented as explicit CLI argument, environment overlay, resolved settings, then safe local default;
- token lookup through explicit argument/environment/settings secret reference, never by logging the value;
- HTTPS/HTTP policy matching deployed local/remote requirements;
- `reqwest` client configured with redirects disabled and typed `GatewayRequestBudget`. Preserve any endpoint's existing stricter bound; otherwise control-plane defaults are a 5-second connect timeout, 30-second total request timeout, and 8 MiB response cap. A route must opt into a different named cap (for example the existing 4 MiB job-file/device contracts) and test both boundary values. Do not use an ambient proxy unless the existing product policy explicitly requires one;
- authenticated request helpers and typed API error decoding;
- `/health`/readiness probe helper;
- URL joining that prevents attacker-controlled path replacement;
- redacted debug representation;
- typed bounded pagination/event-page support where job APIs need it; do not invent reconnect streaming until the server contract exists.

Migrate `message send` first and prove compatibility. CR-02/CR-04 then migrate devices, experiments, runtime web, routine triggering, and jobs.

Acceptance:

- no gateway consumer independently reads/prints the access token;
- redirects cannot forward credentials;
- oversized bodies fail with a typed error;
- authentication and non-2xx API messages are preserved without leaking response secrets;
- debug logs display origin and path, never query tokens or authorization headers.

Tests:

- local mock server: auth header, no redirect follow, timeout, body limit, structured error, malformed response, URL joining;
- snapshot redacted diagnostics;
- successful `send` returns the accepted message/request ID where the API provides it.

## CR-01.6 — Remove unsafe onboarding PATH mutation

**Covers:** INV-12, INV-13, INV-14.

Implement in `src/setup/wizard/summary.rs`:

1. Delete `offer_path_setup`, `try_symlink`, `path_contains`, their tests, and the invocation from the setup summary.
2. Keep only non-destructive installation guidance, selected by package/source-install context where that context is reliably known. Do not print `sudo ln -sf` as the primary recommendation.
3. Setup must not create `~/.local/bin`, remove an existing path, invoke sudo, or mutate shell startup files.
4. A future `system install-shell` is out of this execution scope. If later added, it must inspect the exact target, refuse non-ThinClaw ownership, require explicit confirmation, and support a dry-run.

Acceptance:

- setup performs zero filesystem writes outside ThinClaw-owned configuration/state roots;
- a sentinel file named `thinclaw` on a simulated PATH remains byte-identical;
- summary still gives concise install/package-manager guidance when the binary is not discoverable.

Tests:

- run the summary with an isolated PATH containing a sentinel target and assert metadata/content unchanged;
- source-level deny test or lint asserting setup code does not call symlink/remove-file/sudo helpers;
- platform-specific guidance snapshots.

## CR-01.7 — Remove raw TUI shell execution

**Covers:** INV-50.

Implement in `src/tui/mod.rs`:

1. Delete parsing/execution of `!command`, its ten-minute timeout, child-process launch, and four-megabyte capture path.
2. If input begins with `!`, show a local nonfatal explanation: raw shell escape was removed; use an approved ThinClaw tool or a separate terminal.
3. Do not silently reinterpret it as chat text.
4. Do not add a nominal feature flag that restores the same bypass. Any future operator shell needs a separate security design using the safety scanner, approval policy, and configured sandbox backend.

Acceptance:

- no TUI input path spawns a shell/process directly;
- `!whoami` never executes and leaves the conversation usable;
- approved runtime shell/tool capabilities remain unaffected.

Tests:

- TUI state test for `!command` notice;
- source/call-path assertion covering direct `tokio::process::Command` use in input handling;
- regression test that normal messages and slash commands still submit.

## CR-01.8 — Hide internal entrypoints, remove secret argv protocols, and tier help

**Covers:** INV-10, INV-11, INV-28, INV-41, INV-49.

Implement:

1. Mark Worker, ClaudeBridge, CodexBridge, and ExperimentRunner hidden in Clap under every feature set.
2. Preserve NetworkRelay, AutonomyShadowCanary, and WindowsServiceRuntime hidden status.
3. Ensure completion generation excludes internal commands.
4. Keep `completion` available but omit it from the common command synopsis; it appears in expert help/install docs.
5. Add a build-feature help snapshot matrix so enabling `docker-sandbox`, `repl`, or Windows-specific code cannot expose internals.
6. Treat hidden entrypoints as authenticated process protocols, not merely hidden help. Delete `ExperimentRunner { token: String, ... }` and `build_bootstrap_command`, which currently place the lease bearer in process listings, shell history, SSH/Slurm command text, Kubernetes/provider bootstrap payloads, serializable action responses, and the Web UI. `ExperimentLeaseAuthentication` becomes a non-`Clone`, non-`Serialize`, redacted-`Debug` secret type; ordinary launch/action DTOs cannot contain it or a token-bearing command.
7. The replacement internal parser is exact:

   ```text
   experiment-runner --gateway-url URL [--workspace-root PATH]
                     (--auth-stdin | --auth-file FILE)
   ```

   Exactly one auth source is required. Both carry a versioned `RunnerAuthEnvelope { schema_version: 1, lease_id, token }` capped by `MAX_RUNNER_AUTH_ENVELOPE_BYTES = 4096`. `--auth-stdin` rejects an interactive TTY; `--auth-file` uses a shared private-input reader that rejects relative paths, symlinks, nonregular/multi-link files, wrong owner, group/other Unix permissions, broad Windows ACLs, and replacement between open/metadata check. A writable one-use file is unlinked immediately after a successful read; a read-only managed mount is accepted only through the declared managed-launch adapter. Bytes and parsed token live in zeroizing storage and never enter debug/errors/events. Preserve `--workspace-root` as a non-secret orchestrator control, but require an absolute canonical non-symlink directory and contain every lease worktree/artifact path below it.
8. Replace `RunnerLaunchOutcome.bootstrap_command` and any response `lease` bearer with a credential-free `RunnerLaunchInstruction { command, auth_delivery, expires_at, operator_action }`. The command may contain only the gateway origin and an auth-source selector/path. `auth_delivery` is one of `stdin_pipe`, `private_file`, `kubernetes_secret_mount`, `provider_secret`, or `manual_private_artifact`; it never contains the secret payload. OpenAPI, generated clients, desktop, static Web UI, provider templates, trial summaries, and audit events receive this safe type only.
9. Delivery is backend-specific and fail-closed: local controllers use an anonymous stdin pipe; SSH/Slurm stream the envelope over stdin into a remote mode-`0600` one-use file and launch by path without putting it in the remote command; Kubernetes creates a namespaced Secret with owner reference/expiry cleanup and pipes its mounted value to the runner; RunPod/Vast/Lambda use a provider-native secret facility only. A backend without a reviewed secret channel is `manual_only` and cannot auto-launch or auto-advance a campaign. Never fall back to argv, environment text in a generated command, cloud-init/user-data, a ConfigMap, or a logged template. Every initial, resumed, reissued, and automatically advanced remote trial passes an explicit `RunnerAuthSink`; no helper may create a bearer and then decide how to expose it.
10. Canonical campaign actions that can issue a manual lease are `labs experiments campaigns start ... [--auth-out FILE] [--yes]`, `resume ID [--auth-out FILE] [--yes]`, and `reissue-lease ID [--auth-out FILE] [--yes]`. A manual backend requires a local server-side invocation plus `--auth-out`; without it preflight returns `secure_delivery_required` before creating a lease or changing campaign state. Write the envelope as a staged private file, commit the lease/state, then atomically publish; publish failure revokes the just-created lease and returns typed partial recovery without a success claim. Managed delivery consumes the bearer inside the service and rejects an unnecessary `--auth-out`. When a manual campaign would auto-advance, persist `awaiting_secure_delivery` without creating a token/lease and require explicit `resume --auth-out`. Gateway-only callers that request manual delivery get `secure_delivery_unavailable`, not the token. Reissue revokes the prior lease before publishing one replacement and never returns a replayable prior credential. All action results are metadata-only. The artifact is single-use, expires with the existing named lease TTL, is excluded from backup, and is deleted by the runner after consumption.
11. Remove the other short-lived secret argv protocol in this task: canonical sender approval is `access senders approve CHANNEL REQUEST_ID [--actor ID|--name NAME]`, operating on the stable pending-request ID already owned by the store. `--actor` and `--name` are mutually exclusive: the former links an existing actor without renaming it, the latter creates a new actor, and an existing actor is renamed only through `access identities rename`. Omitting both approves the sender without identity linking. Pending list/JSON/result/error DTOs omit the pairing code. Remove `pairing approve CHANNEL CODE` immediately; the hidden root group forwards only ID-based canonical forms. Approval remains exact-channel/owner-scoped, rate-limited, audited, and non-mutating for unknown/expired/already-consumed IDs.
12. Delete the two `THINCLAW_GATEWAY_INSTANCE_TOKEN` child-environment writes in `src/cli/gateway.rs`: the audited child never reads that variable. Keep the random correlation nonce only in parent memory and the private versioned PID record used for ownership-safe cleanup. Redact its `Debug`, bound/validate it on read, and never confuse it with the gateway bearer-auth token. Spawn tests capture the complete child argv/environment and prove neither token class is injected unless an explicitly documented runtime-auth source requires it.

Acceptance:

- public `--help`, expert help, generated completion, and CLI reference contain no internal process entrypoint;
- orchestrator invocations continue to parse and execute;
- hidden does not become security: internal endpoints keep their existing auth/lease controls;
- known token sentinels are absent from `/proc`/process-list argv fixtures, shell/SSH/Slurm text, provider/Kubernetes non-Secret manifests, typed JSON/OpenAPI/generated clients, Web UI DOM, summaries, events, errors, and logs;
- stdin/private-file/managed-secret success, 4-KiB boundaries, TTY refusal, file/ACL/replacement defenses, workspace-root containment, single-use cleanup, expiry/replay denial, manual-remote refusal, provider-without-secret-channel refusal, and reissue revocation/atomic-artifact behavior have exact tests.
- pending sender results contain stable request IDs but no code, ID-based approval has exact owner/channel/state behavior, and a known code sentinel never appears in argv/history/output/error fixtures.
- gateway start/reload child-spawn fixtures contain no unused instance-token or bearer-token argv/environment value, while private PID-record ownership/cleanup still rejects stale or mismatched processes.

## CR-01.9 — Make full reset scoped, inspectable, and exclusive

**Covers:** INV-15, INV-80, INV-83.

Replace the all-or-nothing `ResetCommand { yes }` contract with:

```text
setup reset [--scope all|database|files|credentials]... [--dry-run] [--yes]
```

Rules:

1. With no `--scope`, default to `all`. `all` conflicts with any additional scope; repeated non-`all` scopes are deduplicated. `files` means only the resolved ThinClaw-owned home/state tree. `credentials` means encrypted secret rows, purpose-scoped source records/bindings, known ThinClaw OS credential accounts, and direct Phase-A secret-reference fields (never unrelated environment variables or operator-owned source files). `database` means all selected-backend ThinClaw tables/data; its plan explicitly notes that this superset also removes encrypted secret/source rows. Scope overlap is deduplicated.
2. `--dry-run` performs no writes and returns a typed plan containing resolved backend, canonical paths, table/resource and secret/source/binding counts where safely queryable, redacted source IDs/kinds and OS account labels (never values/locations), active-runtime/lease state, scope overlap, and per-scope `ready|blocked|skipped` status.
3. Execution requires `--yes` when stdin is not a TTY. Interactive execution requires the existing confirmation plus the exact `RESET` phrase. JSON/JSONL execution without `--yes` is a usage error; machine mode never prompts.
4. Acquire an exclusive runtime-operation lease before any mutation. Refuse while a ThinClaw runtime/service owns the lease; do not rely on a warning. Release the lease on success/error. The reset does not stop/uninstall a service and does not remove the installed binary.
5. Resolve and validate every target before mutating. Never follow symlinks, never accept a home/root path as the ThinClaw state target, and reuse bounded/private filesystem helpers. Report partial failure per scope and exit `1`; do not print “reset complete” after a skipped/failed selected scope.
6. libSQL deletion and PostgreSQL truncation remain transactional/backend-aware and use the compile-aware backend resolver. For credentials-only reset, transactionally clear mutable bindings and encrypted/source rows, atomically scrub direct Phase-A refs, then delete owned OS accounts; operator-owned env variables/private files are never deleted. For `all`, deduplicate rows already removed by the database scope before OS/file cleanup. Output records every completed target/scope so recovery is explicit.
7. Keep hidden root `reset` forwarding to the same request through 0.18. `config reset KEY` remains a separate single-setting operation.

Acceptance tests:

- dry-run and execution select the same exact targets and dry-run leaves bytes/rows/credentials unchanged;
- active runtime lease blocks every mutating scope;
- `all`/repeat/conflict parsing, TTY/non-TTY confirmation, cancellation, partial failure, and empty target behavior;
- libSQL/PostgreSQL parity and path/symlink/root-target adversarial fixtures;
- service definition and binary sentinel remain untouched.

## CR-01.10 — Rebuild setup as an explicit draft/review/apply flow

**Covers:** INV-12, INV-13, INV-14, INV-83, INV-87, INV-88, INV-89. The exact entry and 27-page disposition is mandatory in `08-setup-flow-page-and-secret-matrix.md`.

Implement in this order:

1. Replace `pause_after_completion: bool` with `SetupInvocation`/`SetupContinuation`. Canonical explicit `setup` uses `Exit`; `setup --run` uses `Run`; automatic first-run preserves the parsed `Run`, `Tui`, or full `AskRequest`. Setup `--ui` controls only the renderer. Remove unreachable pause branches and update handoff helpers/tests.
2. Add canonical `setup --mode quick|advanced` and `setup edit TOPIC`. Advanced traverses all applicable target sections. `edit` replaces the misleading one-topic “Advanced Setup.” Map hidden `onboard`, `--guide`, and `--channels-only` through typed compatibility adapters; legacy `onboard` retains its old continue-to-runtime result only until removal.
3. Introduce `SetupDraft`, `SetupPlan`, `SetupAction`, and `SetupApplyReport` in `thinclaw-app` as secret-free neutral types. Keep concrete DB/secure-store/extension adapters in the root. Both CLI and TUI render the same types.
4. Delete `auto_configure_quick_runtime_defaults` as a mutating pre-page operation. Profile/quick helpers return draft defaults/diffs only. Database inspection uses a no-auto-migrate connection path; new DB creation and migrations become planned Apply actions.
5. Delete `persist_after_step`. Page handlers mutate only the draft and run explicitly labeled bounded read-only probes. Replace live `Settings` checkpoints with typed draft navigation. Provider/channel/worker auth values live only in one top-level setup-controller-owned, non-`Clone`, non-`Serialize`, redacted-`Debug` `SetupSecretDraft` of zeroizing `SecretString` slots keyed from the secret-free draft; only source/reference/presence/verification metadata enters checkpoints or plans.
6. Consolidate all 27 current `SetupWizardStepId` entries into the ten target sections in file 08 and implement its complete seven-current/six-retained setup-profile matrix. Provider/channel pages consume the prerequisite shared catalogs and typed secret-reference field metadata; they may not introduce setup-only service/driver lists or credential schemas. Keep exhaustive legacy-ID/value-to-section/profile mapping tests. Quick may collapse optional details but starts visibly on `balanced` and its Review displays every inferred value/action; Advanced is comprehensive and can use No preset. Delete the hidden builder/coding Quick default.
7. Implement Review & Apply and the exact headless contract in file 08. Show the baseline revision, non-secret settings diff, DB backend/path and migrations, secure-store account/reference names, extension sources/digests, filesystem targets, listeners/binds, external requests/costs, warnings, blockers, continuation, and plan digest. Interactive default is Cancel. Machine input is versioned/secret-free/bounded, dry-run precedes digest-pinned non-TTY `--yes`, and invalid/conflicting input cannot mutate. Setup Apply is CR-02.1 `stopped_exclusive`: dry-run/review may inspect an active runtime, but Apply refuses an owned active or ambiguously stale instance before any write and gives the canonical stop/status remediation. Once stopped, acquire the exclusive runtime-operation lease, revalidate the baseline/digest, stage/validate all actions, and apply in the documented order. `setup_completed` is the last marker.
8. Return typed failure/partial results. No persistence error is debug-only; no success/continuation occurs after a failed selected action. Cancel before Apply exits `130` with no mutation. A renderer fallback cannot reset draft state or change continuation.
9. Replace setup credential storage with the shared `SecretSourceId`/`SecretSourceRecord`/`SecretBinding` contract introduced by the prerequisite CR-02.6 foundation. The master key and any pre-DB host credential use the OS credential store or an operator-owned env/private-file Phase-A reference; masked/generated provider/channel/gateway credentials default to the existing encrypted `SecretsStore`, then receive purpose-scoped source records. An explicitly selected env/private-file source remains external. Mutable settings contain only source IDs/purposes. Env/private-file mode accepts an operator-supplied source and never generates, prints, shells, copies, or persists its value. The existing `SecretRef` remains encrypted-store metadata and never masquerades as an OS-keychain handle. Remove master keys, tokens, passwords, credential-source locations, credential-bearing database URLs, sender codes, and experiment lease bearers from `.env`, DB/TOML mutable settings, command text, plans, and summary data.
10. Add a verified one-time plaintext-settings migration: secure-store write → retrieval verification → transactional reference replacement → old-source scrub. On failure, preserve the old source and report only its schema key. Remove setup calls to tokenized `GatewayAccessInfo` formatting; only CR-02.15 may reveal a gateway token.
11. Apply CR-01.6 at the new summary boundary: no PATH/symlink/directory mutation outside the reviewed ThinClaw-owned action set. Replace decorative/repeated page copy with the layout rules in file 08.

Acceptance:

- explicit setup/edit exits; `--run` and first-run Run/TUI/Ask continuations are exact;
- all 27 legacy page IDs have one disposition and target section;
- Quick/full Advanced/all topics/profiles/renderers/profile gates produce deterministic secret-free plan fixtures;
- headless input schema/bounds, dry-run, required non-TTY plan digest, conflicts, and digest/baseline races are deterministic and side-effect-free until accepted Apply;
- cancel/back at every boundary leaves files, schema/data, secure-store metadata, extensions, and process environment unchanged;
- apply ordering, baseline conflict, migration/store/settings/artifact failure, recovery, and retry have typed fixtures;
- active/stale/wrong runtime-instance Apply attempts are zero-mutation failures; a stopped instance obtains the exclusive lease and a later `--run` continuation starts only after successful commit;
- known secret sentinels never appear in output, trace/TUI buffers, files/DB maps, debug/panic values, or arguments;
- env-key mode never generates/writes a key, local mode never falls back to plaintext, and no setup path emits a tokenized URL;
- setup help, summary, first-run prompts, source comments, and canonical docs use `setup` paths and truthful continuation language.

## CR-01 definition of done

- [ ] CR-01.1 typed context/outcome landed.
- [ ] CR-01.2 output/color contract applied to representative commands and available to all handlers.
- [ ] CR-01.3 bootstrap is centralized and proven once-per-invocation.
- [ ] CR-01.4 flag scope and ask/send/channel semantics are enforced.
- [ ] CR-01.5 gateway client is hardened and `send` is migrated.
- [ ] CR-01.6 onboarding PATH mutation is absent.
- [ ] CR-01.7 raw TUI shell escape is absent.
- [ ] CR-01.8 internal help exposure is closed, sender approval is request-ID/code-free, and experiment runner auth has no argv/bootstrap/result bearer path.
- [ ] CR-01.9 reset is scoped, dry-runnable, lease-exclusive, and backend-safe.
- [ ] CR-01.10 setup is typed, side-effect-free before Apply, credential-safe, page-complete, and continuation-correct.
- [ ] `cargo fmt --all -- --check`, targeted tests, `cargo check --features full --bin thinclaw`, and `git diff --check` pass.
