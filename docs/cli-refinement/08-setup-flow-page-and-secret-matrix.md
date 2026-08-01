# Setup Flow, Page, Mutation, and Secret Matrix

> **Temporary execution input.** This is the complete setup/onboarding state-machine audit for the 0.16.0 baseline. CR-01.10 owns the refactor. Delete this file after the shipped setup reference, security documentation, and contract tests contain these facts.

## 1. Current entry and continuation behavior

| Entry | Current behavior | Defect | Required end state |
|---|---|---|---|
| Explicit `thinclaw onboard` | Builds `SetupConfig` with `pause_after_completion=false`; after setup it always enters the runtime selected from the wizard UI mode. | A settings command unexpectedly becomes a long-running runtime. There is no public way to reach the existing stop-after-setup branch. | Canonical `thinclaw setup` applies settings and exits `0`. `setup --run` explicitly continues to the normal REPL runtime. Wizard renderer (`--ui`) does not silently choose a different runtime. |
| `onboard --channels-only` | Three-step channel plan, then also continues into the runtime. | A focused settings edit cannot simply return to the shell. | `setup edit channels` applies and exits. Hidden legacy form forwards to `setup edit channels --run` only during its compatibility window. |
| `onboard --guide[=TOPIC]` | Opens one selected topic plus Summary, then continues into the runtime. | “Advanced Setup” is not a comprehensive setup; it is a one-topic editor with surprising continuation. | `setup --mode advanced` is the complete advanced path. `setup edit TOPIC` is the explicit one-topic editor and exits unless `--run` was given. |
| Explicit `--profile` | Skips the Quick/Advanced entry choice and applies profile/quick defaults before the first planned page. | Flag selection can authorize DB migrations/key generation without a mutation preview. | Profile preselects draft values only. Nothing durable occurs until Review & Apply. |
| First run from bare/`run` | Automatic setup, then normal runtime. | Continuation is implicit but expected; origin is not represented as a typed request. | First-run setup receives `Continuation::Run` and resumes the exact originating runtime only after successful apply. |
| First run from `tui` | Automatic setup, then uses resolved wizard UI mode to set runtime mode. | The setup renderer can change the requested runtime surface. | Preserve `Continuation::Tui` regardless of whether setup falls back from TUI rendering to CLI prompts. |
| First run from `ask TEXT` after CR-01 | Not yet represented. | One-shot intent and text could be lost or accidentally become an interactive runtime. | Preserve `Continuation::Ask { request }`; after setup execute exactly one turn and exit. |
| Cancel/back before completion | In-memory checkpoint restores settings, but earlier per-step DB/`.env`/secure-store/extension writes are not rolled back. | UI appears reversible while durable side effects are not. | Back is purely draft navigation. Cancel before Apply returns `130` and leaves DB/files/secure store/extensions byte-for-byte unchanged. |

Remove `pause_after_completion` and replace it with a non-boolean typed continuation:

```text
SetupContinuation = Exit | Run | Tui | Ask(AskRequest)
SetupInvocation = Initial { mode, profile, continuation } |
                  Edit { topic, continuation } |
                  FirstRun { origin, continuation }
```

Canonical parser:

```text
thinclaw setup [--mode quick|advanced] [--profile PROFILE]
               [--ui auto|cli|tui] [--skip-provider-auth] [--run]
               [--input FILE] [--dry-run|--yes]
               [--expect-plan-digest SHA256]
thinclaw setup edit ai|channels|agent|tools|automation|runtime|appearance
               [--ui auto|cli|tui] [--run]
               [--input FILE] [--dry-run|--yes]
               [--expect-plan-digest SHA256]
thinclaw setup reset ...
```

`--ui` controls setup rendering only. `--run` means the normal REPL runtime; users who want the TUI can run `thinclaw tui`, while automatic first-run from `tui` resumes TUI exactly. Rename current `--skip-auth` to `--skip-provider-auth`; keep a hidden forwarding alias through 0.18.

Headless/machine contract:

1. `--input FILE` reads a non-symlink regular UTF-8 JSON document capped at 1 MiB with `schema_version: 1`, explicit invocation mode/topic, optional retained preset, and typed section selections. Unknown/duplicate fields, incompatible feature/platform choices, control characters, and values outside shared bounds are errors; there is no permissive string map.
2. The document is secret-free and may select only an existing, purpose-authorized opaque `SecretSourceId`. It cannot create an environment/private-file source because a plan digest must not authorize a new host source implicitly; create it first with local `config secrets set --from-env|--from-file`. Inline tokens/passwords/keys, credential-bearing URLs/headers, source locations, and sentinel placeholders are schema errors. Interactive local setup may stage a new masked/encrypted or env/file source explicitly and shows its generated ID/kind/purpose in Review without the value/location.
3. `--input` does not imply mutation. `--dry-run` renders/returns the complete `SetupPlan` and digest with exit `0`. `--dry-run` conflicts with `--yes` and `--run`. A non-TTY invocation without `--dry-run` or `--yes` is usage error `2` before mutation.
4. `--yes` authorizes only the plan computed from that input and current baseline. `--expect-plan-digest SHA256` is optional with interactive confirmation but required with non-TTY `--yes`; mismatch exits `1` with a fresh credential-free plan/digest and makes zero writes. The apply path still acquires the lease and revalidates the baseline immediately before the first commit.
5. `--run` is allowed only for an interactive human-output invocation after successful Apply; it conflicts with machine output and headless `--input --yes`. Automated/service workflows run `setup` and `runtime service`/`run` as separate operations.

### 1.1 Setup profile catalog and target presets

Profiles are setup recommendation overlays, not compiled build profiles, readiness profiles, or permanent runtime modes. On the audited Quick path, `auto_configure_quick_runtime_defaults` silently selects `builder-coding` when no profile was supplied, before the UI can show the nominal `balanced` default. Remove that behavior.

| Canonical setup value (current aliases) | Actual 0.16 mutations/recommendations | Disposition | Target secret-free draft overlay |
|---|---|---|---|
| `balanced` | Skills/routines/log observability on; smart routing on and primary-only changes to cheap-split; heartbeat off. Balanced extension bundle. | **KEEP / MAKE EXPLICIT; canonical default.** | Propose those non-secret values only after the Profile page and show every changed field in Review. Do not replace an already selected model/provider/channel. |
| `local-private` (`local-and-private`) | Skills/routines/log on; defaults missing LLM to Ollama, primary-only routing, Ollama `nomic-embed-text` recommendation, heartbeat off. Safe extension bundle. | **KEEP / HARDEN LOCAL CLAIM.** | Propose only compiled/local providers discovered without network. “Private” is blocked if any selected provider/embedding/channel requires remote egress; Review lists every egress exception. It never implies data locality from a label alone. |
| `builder-coding` (`builder-and-coding`) | Skills/routines/log on; advisor/executor routing; advisor max calls raised to at least four; heartbeat off. Power extension bundle and coding-worker bias. It is also the current hidden Quick default. | **KEEP / REMOVE HIDDEN DEFAULT.** | Propose coding tools, declared sandbox/backend, advisor/executor and maximum-call/cost implications. Nothing installs or enables before Apply. Quick defaults to `balanced`, never silently to this profile. |
| `channel-first` | Skills/routines/log and cheap-split routing; channel/notification steps prioritized; heartbeat recommendation depends on a preferred notification channel. Balanced extension bundle. | **KEEP / MAKE DEPENDENCIES EXPLICIT.** | Propose no heartbeat until an egress destination is configured and a probe succeeds. Display selected service+driver, listener/ingress exposure, credential source, and notification target in Review. |
| `remote` (`remote-server`) | Headless helper enables gateway on loopback/port 3000, generates an auth token, disables current CLI channel flag, recommends libSQL, skills/routines/log/cheap-split, heartbeat off, env-key compatibility, SSH guidance. | **KEEP / SECURITY OVERHAUL.** | Local REPL/TUI remain available surfaces. Propose loopback web ingress, explicit SSH/tunnel guidance, compile-valid DB choice, and a secure-store token action at Apply. No token URL, plaintext setting, env-key generation, listener start, DB create, or migration before Apply. |
| `pi-os-lite-64` (`raspberry-pi-os-lite`, `pi`) | Remote defaults plus desktop autonomy off and Pi-specific follow-up text. | **KEEP / PLATFORM-GATE.** | Extend the remote draft with desktop/local-sensor policy off and resource-conscious choices. Require Linux aarch64/Pi OS readiness evidence; an explicit incompatible host produces a Review blocker and cannot Apply invalid platform defaults. |
| `custom` (`custom-advanced`) | Applies no profile defaults and exists mainly to enter/talk about Advanced setup. | **REMOVE AS A DISTINCT PRESET / KEEP ALIAS.** | Canonical behavior is `setup --mode advanced` with “No preset.” Hide `--profile custom` through 0.18 as an adapter to that invocation. `--mode quick --profile custom` is a usage conflict; remove the value in 0.19. |

Profile rules:

1. `setup --profile` accepts the six retained canonical preset values only; existing aliases remain hidden through 0.18. The interactive Advanced page additionally offers **No preset** without serializing a fictional profile.
2. Selecting/changing a preset computes a typed `ProfileRecommendation { profile_id, base_revision, proposed_values, inferred_values, blockers, warnings }`. It does not mutate live `Settings` or `SetupSecretDraft`; operator-entered values win and are never silently reset.
3. Quick starts on `balanced` but displays the selection before deriving anything. Headless/machine setup uses the versioned input contract above and supplies a retained profile or complete explicit selections; it never inherits the old hidden builder default.
4. Every proposed field identifies why it changed, whether it is local/networked/billable/listener-opening, and the compile/platform predicate. Unsupported recommendations are blockers or explicit alternatives, never inert settings.
5. Tests freeze the exact source-to-target mapping, all current value aliases, Quick/Advanced interactions, existing-value precedence, every feature/platform blocker, and the absence of mutation before Apply.

## 2. Current 27-step/page inventory

Route notation: `Q` = current 12-step quick plan; `AI`, `CH`, `AG`, `TL`, `AU`, `RT` = current guided topics; `CO` = channels-only. “Hidden Q” means the behavior runs inside another quick step or before the plan and is not represented as its own page.

| Current `SetupWizardStepId` | Current route/content and side effects | Disposition | Target page/content |
|---|---|---|---|
| `CliSkin` | Q, AG. Chooses CLI/onboarding skin and changes the process-wide runtime skin immediately; persisted after step. | **MERGE / MOVE LATE** | Optional Appearance section with Web UI. Do not lead setup with decoration; preview locally without durable/global mutation until Apply. |
| `Profile` | Q only. Selects one of seven profiles and immediately applies many defaults. Not persisted at this step, but quick defaults were already applied before it. | **KEEP / OVERHAUL** | First substantive choice. Show an exact diff of profile defaults and feature/platform blockers; modify draft only. Advanced mode may still use a profile as a starting point. |
| `Database` | RT only. Selects/tests backend, connects, runs migrations, then reloads DB settings. Q instead auto-connects/migrates before page 1. | **KEEP / MOVE EARLY** | Runtime Foundation page in both Quick and Advanced. Read-only probe existing stores; show backend/path/remote host and migration plan. Create/migrate only during Apply under a setup/runtime lease. |
| `Security` | RT only. Selects OS secure store, env mode, or disabled; can store a key immediately or print a generated key. Q auto-generates/stores before page 1. | **KEEP / REBUILD** | Runtime Foundation security section in every initial setup. Local default is OS secure store. Env/file mode accepts an operator-supplied secret source and never generates/prints/persists the key. |
| `InferenceProvider` | Q, AI. Selects provider and may collect/store auth. | **KEEP / MERGE** | AI page: provider plus authentication source/status. The secret value stays only in the top-level setup-controller-owned `SetupSecretDraft`; the serializable draft holds redacted source/status metadata. Optional bounded probe is explicit; durable store/reference write occurs only during Apply. |
| `ModelSelection` | Q, AI. Selects primary model. In Q it also silently applies embeddings defaults and runs smart-routing setup. | **KEEP / MERGE** | AI page: primary model visibly followed by optional Routing/Embeddings advanced sections. No hidden mutation in a different step. Show network/cost implications. |
| `SmartRouting` | AI page; also invoked invisibly from Q ModelSelection. Selects fast executor model/routing. | **MERGE** | AI > Routing expandable section. Summary lists advisor/executor choices separately. |
| `FallbackProviders` | AI only. Selects fallbacks and may require/provider-auth follow-up. | **MERGE / CONDITIONAL** | AI > Fallbacks; shown after a primary exists, optional in Quick, fully reviewable in Advanced. |
| `Embeddings` | AI only; Q writes inferred defaults invisibly. | **MERGE / MAKE EXPLICIT** | AI > Memory Search. Show enabled/provider/model, local/remote, dimensions/backend compatibility, and possible cost. Quick may accept a recommended default only after it appears in Review. |
| `AgentIdentity` | Q, AG. Agent name and personality pack. | **KEEP / MERGE** | Identity & Locale page with timezone. Explain exact files/metadata seeded; preview only. |
| `Timezone` | AG only; Q auto-detects and stores invisibly before page 1. | **MERGE / MAKE EXPLICIT** | Identity & Locale. Show detected IANA zone in Quick review; allow correction in Advanced/edit. |
| `Channels` | Q uses a reduced primary-channel flow; CH/CO use the full flow. Collects tokens and may write secure-store/settings values during the page. | **KEEP / REBUILD FROM CATALOG** | Channels page driven by the one typed catalog in file 07. Select service and driver, collect secret references, show ingress/listener/egress scope, and stage all writes. |
| `ChannelContinuity` | CH only. Explanatory page about cross-channel sessions; no durable setting. | **REMOVE AS BLOCKING PAGE** | Inline help on Channels and a canonical docs link. Do not spend a progress step on static prose. |
| `ChannelVerification` | Q, CH, CO. Runs checks and records follow-ups; Q also silently chooses notification/heartbeat defaults. | **KEEP / SPLIT FACTS** | Verify page after configuration. Show static config and explicit bounded probe results separately. It must not mutate notifications. |
| `Notifications` | CH only. Chooses proactive delivery destination; reads credentials/readiness. Q assigns defaults inside verification. | **MERGE / MAKE EXPLICIT** | Automation & Delivery page. Only verified egress destinations are recommended; Quick review shows its selected default. |
| `Extensions` | TL only. Reads registry and immediately installs selected artifacts into `~/.thinclaw/tools`, with bundled/source fallback. | **KEEP / DEFER MUTATION** | Tools & Safety > Extensions. Stage selected artifacts, source/digest/auth/risk and disk plan; download/validate to a private staging area and atomically publish during Apply. |
| `DockerSandbox` | Q, TL. Probes Docker and changes sandbox setting. | **KEEP / MERGE** | Tools & Safety > Execution Boundary. Probe is read-only; explain host vs container behavior and unavailable profile reasons. |
| `CodingWorkers` | Q. Combined enablement prompt for Claude Code/Codex after Docker step. | **MERGE / CONDITIONAL** | Tools & Safety > Coding Workers. One selector, then show provider-specific detail only for selected workers. |
| `ClaudeCode` | TL only. Provider/auth/sandbox details, possible secret reuse/write. | **MERGE / CONDITIONAL** | Conditional Claude Code subsection under Coding Workers. One deliberate consent for secret reuse; stage references only. |
| `CodexCode` | TL only. Provider/auth/sandbox details, possible secret reuse/write. | **MERGE / CONDITIONAL** | Conditional Codex subsection under Coding Workers, same policy as Claude Code. |
| `ToolApproval` | Q, TL. Selects autonomy level and local-tool policy. | **KEEP / PROMOTE** | Tools & Safety first section. Show exact enabled execution lanes, approval default, containment, and high-risk warning; no euphemistic labels without effects. |
| `Routines` | AU only. Enable/disable scheduled automation. Q sets enabled invisibly before page 1. | **MERGE / MAKE EXPLICIT** | Automation & Delivery. Quick review lists enabled state; Advanced can edit. Distinguish definition support from live scheduler readiness. |
| `Skills` | AU only. Enable/disable skills. Q sets enabled invisibly before page 1. | **MERGE / MAKE EXPLICIT** | Automation & Delivery (or Tools & Safety summary) with trust/catalog sources and no claim that skills are loaded before runtime. |
| `Heartbeat` | AU only. Enables cadence/notification; Q initially disables then verification can silently change it. | **MERGE / MAKE EXPLICIT** | Automation & Delivery conditional on routines plus verified notification destination. Never enable as a verification side effect. |
| `WebUi` | Q, AG. Appearance plus remote bind/access choice; can generate a gateway token and currently prints a token URL. | **MERGE / SECURITY OVERHAUL** | Appearance & Access. Appearance is optional; listener bind/exposure/auth is a separate security subsection. Show credential-free origin only. |
| `Observability` | RT only. Chooses log/none/Prometheus. Q sets log invisibly before page 1. | **KEEP / MERGE** | Runtime Operations advanced section; Quick review shows log default. Validate that selected backend/endpoint is actually compiled. |
| `Summary` | All paths. Marks onboarding complete, persists settings, writes bootstrap `.env`, prints readiness/config/token information, and offers PATH mutation. | **KEEP / REBUILD AS REVIEW & APPLY** | First show complete diff/action/security/listener/cost/follow-up plan with Apply defaulting to Cancel. After apply, show typed results and credential-free next commands. No PATH mutation or secret value. |

All 27 current IDs therefore have an explicit final disposition. The target UI has fewer meaningful screens without deleting underlying configuration:

1. Profile;
2. Runtime Foundation (database + secret protection);
3. AI & Memory;
4. Identity & Locale;
5. Channels;
6. Tools & Safety;
7. Automation & Delivery;
8. Appearance & Access;
9. Verify;
10. Review & Apply.

Quick mode uses progressive defaults and may collapse optional sections, but its final review must display every value it will write or capability it will enable. Advanced mode traverses all applicable sections rather than only one topic. `setup edit TOPIC` opens the relevant section plus Verify/Review & Apply.

## 3. Current mutation defects

1. `prepare_run_mode` calls `auto_configure_quick_runtime_defaults` before the first planned page. That function selects the Builder/Coding profile by default, applies timezone/Web UI/observability/routine/skills/heartbeat values, connects to the database, runs migrations, and configures secrets.
2. After most successful pages, `persist_after_step` writes bootstrap `.env` and DB settings. Both failures are logged at debug and ignored, so later Summary can obscure earlier persistence loss.
3. TUI checkpoints clone in-memory wizard state only. Back cannot undo migrations, secure-store writes, provider/channel secret writes, extension installs, directories, or earlier DB/`.env` persistence.
4. Summary is both review and commit: `onboard_completed=true`, DB write, `.env` write, output, PATH mutation prompt, and continuation are interleaved.
5. `Settings::to_db_map` flattens serialized settings, and legacy channel token/password fields are serializable. Debug redaction does not prevent plaintext DB/TOML/bootstrap persistence.

## 4. Target plan/apply architecture

Replace page-owned mutation with typed planning:

```text
SetupDraft { baseline_revision, invocation, selections, secret_slots, probes }
SetupSecretDraft { zeroizing values keyed by secret slot; non-serializable, non-debug }
SetupAction = StoreMasterKey | MigrateDatabase | StoreEncryptedSecret |
              CreateSecretSource | WriteSettings | PublishBootstrap |
              InstallExtension | SeedIdentityFiles | MarkSetupComplete
SetupPlan { schema_version, baseline, diff, actions, listeners, costs, warnings, blockers }
SetupApplyReport { plan_digest, action_results, committed, partial, recovery }
```

Execution rules:

1. Resolve compiled capabilities, selected config, and an existing database in **read-only/no-auto-migrate** mode. Build defaults into `SetupDraft`; do not call profile mutation helpers on live `Settings`.
2. Pages update the serializable, secret-free `SetupDraft` and may run explicitly labeled bounded read-only probes. Secret slot state records only `SecretSourceId`/kind/purpose/presence/verification metadata; a staged new source preallocates its opaque ID so the reviewed plan digest is stable. Actual values live in one top-level setup-controller-owned, non-`Clone`, non-`Serialize`, redacted-`Debug` `SetupSecretDraft` of `SecretString`/zeroizing inputs and never appear in checkpoints, plans, events, debug, or UI buffers after submission.
3. Before any durable change, render Review & Apply with the resolved backend/path, settings diff (secret bindings only), migrations, files, secret source IDs/kinds/purposes, extension source/digests, listeners/binds, external requests/cost, and blockers. Never show encrypted-store names, OS accounts, environment names, or file paths in a remotely renderable plan. Human default is Cancel. Machine/headless setup follows the input/dry-run/expected-digest/`--yes` contract above and never prompts.
4. On Apply, enforce CR-02.1 `stopped_exclusive`: an owned active runtime, an ambiguous/stale ownership record, or a selected remote runtime that cannot prove safe coordination blocks before mutation with exact stop/status remediation. Dry-run remains available. Once stopped, acquire the same exclusive runtime-operation lease used by reset/backup import and verify the baseline revision has not changed.
5. Stage private files/artifacts first. Validate all manifests/config and obtain every required confirmation before the first commit.
6. Commit in a documented order: create a new OS master-key handle if required; connect/create and migrate the DB; write/verify encrypted secrets; create purpose-scoped source records; commit mutable settings containing only bindings; atomically publish non-secret/direct-ref Phase-A bootstrap TOML with owner-only mode/ACL and symlink-safe parents; identity files; atomic extension publishes; `setup_completed` marker last. Delete a newly created unused master key/source record during compensation; never delete a pre-existing one. Report any non-rollbackable external outcome precisely.
7. Never swallow persistence failures. Any failed selected action makes the report `committed=false` or `partial=true`, exits `1`, omits “complete,” leaves a credential-free recovery plan, and does not continue to runtime.
8. Cancel/back before Apply performs no durable writes. Tests compare file trees, DB schema/data, secure-store account metadata, process environment, and extension directories before/after.
9. Remove the claim that progress is saved after each page. If resumable drafts are later required, make it an explicit encrypted, versioned, secret-free draft feature—not best-effort production settings writes.

## 5. Secret and credential contract

Current high-impact defects:

- interactive environment mode generates and prints a full `SECRETS_MASTER_KEY` shell command, but does not install that value into wizard crypto state;
- quick-mode secure-store failure generates a master key, sets the process environment, records it in `generated_env_master_key`, and later writes it to `~/.thinclaw/.env`;
- channel/provider/gateway credential fields can remain in serializable `Settings` and enter DB/TOML/bootstrap output;
- remote Web UI setup calls `token_url(true)` and the summary prints a token URL; current follow-up text advertises `--show-token`;
- masked input protects keystrokes but not subsequent storage/serialization.

Required end state:

1. Local setup generates the master key only inside the OS secure store and receives an opaque key handle/loaded crypto object. It never converts the key to printable UI text.
2. Headless/env mode requires an operator-provided `SECRETS_MASTER_KEY` or private regular `--master-key-file`; setup validates availability without reading it into output. It does not generate an env key, edit shell profiles, or write the key to `.env`.
3. Masked/generated provider keys, channel tokens/passwords, and gateway bearer tokens default to the existing encrypted `SecretsStore`; its master key comes from the direct Phase-A source. An operator may instead bind one of those runtime slots to an explicitly created environment/private-file source record. Pre-DB database credentials use a direct local OS credential account or explicitly operator-owned environment/private-file Phase-A source because the durable source registry is unavailable before connection. Durable source records encrypt/authenticate their exact locator at rest and expose only safe ID/owner/kind/canonical-purpose/revision metadata; mutable settings contain only `SecretBinding { source_id, purpose }`. The existing `SecretRef` is encrypted-record metadata, not an OS-keychain reference. Remove new writes to legacy plaintext fields.
4. Add a one-time migration: discover legacy plaintext fields, write each to its final encrypted/OS store, verify retrieval, create the source record, transactionally replace the setting with a binding, then scrub the old DB/TOML/bootstrap value. Back up only non-secret migration outcome metadata; never log values or source locations. A migration failure leaves the old source intact, disables the consumer, and reports the affected key name only.
5. `GatewayAccessInfo` exposes credential-free origins/bind/tunnel text by default. Setup, boot, TUI, logs, debug, JSON, errors, and summaries cannot call a token-URL formatter. Only guarded `runtime web access --reveal-token` owns deliberate TTY reveal.
6. Setup summaries show only source ID/kind/status (`encrypted_store`, `os_credential`, `environment`, `private_file`, `disabled`) and allowed purpose, never an encrypted-store name, OS account, variable, path, value/prefix, tokenized URL, command containing a value, or QR embedding a long-lived token.
7. Zeroize and drop `SetupSecretDraft` after Apply/Cancel/error and when replacing a slot. Never clone it for Back/history. Test with known sentinel values across stdout, stderr, tracing capture, TUI buffers, serialized draft/plan/events, files, DB maps, panic/debug formatting, and child-process arguments.

## 6. Page-content and layout rules

- Use plain section names. Remove “Humanist Cockpit,” “cockpit lane,” and other decorative copy where it displaces the current choice, consequence, or remediation.
- One header, `Step N of M`, concise purpose, current/recommended value, dependency/readiness, and Back/Continue controls. Do not repeat phase title, description, “why this matters,” and recommendation as four separate blocks.
- Show conditional fields only after the enabling choice. Keep advanced provider/channel/worker fields collapsed in Quick.
- Every choice states whether it is local, external/networked, billable, listener-opening, privileged, or destructive.
- Review groups changes as Runtime, Credentials, AI, Channels, Tools, Automation, and Experience; unchanged values are collapsed.
- Summary distinguishes `applied`, `needs_attention`, and `not_configured`. “Ready” requires the selected readiness profile, not a count of completed pages.
- TUI and line-oriented CLI render the same `SetupDraft`/`SetupPlan`; renderer fallback cannot change selections, continuation, or mutation semantics.
- Terminal resize/back/cancel tests operate on typed navigation; no page is identified only by display text.

## 7. Required acceptance fixtures

1. Each of the 27 legacy step IDs maps to one target section/disposition and no handler is orphaned.
2. Quick, full Advanced, every `edit` topic, channels compatibility, each profile, CLI/TUI/auto renderer, and first-run Run/TUI/Ask continuations have plan snapshots.
3. Explicit `setup` exits after successful Apply; `setup --run` enters REPL; first-run TUI/Ask resumes its exact request; legacy `onboard` retains its temporary compatibility continuation.
4. Cancel at every page and immediately before Apply produces zero durable mutation. Back across every boundary preserves only the current draft.
5. Apply action ordering, baseline conflict, DB migration failure, secure-store failure, settings failure, extension failure, partial recovery, and retry are deterministic and typed.
6. libSQL-only, PostgreSQL-only, dual-backend, desktop, edge, full, and unsupported feature/platform pages show only valid choices and correct defaults.
7. No secret sentinel appears outside the authorized secure-store fixture; no token URL appears anywhere in setup.
8. Environment secret mode never generates/prints/writes a key. Local mode never falls back to plaintext `.env` automatically.
9. Setup PATH/symlink sentinel tests from CR-01.6 remain mandatory.
10. Generated setup help/reference and every “what next” command use canonical paths and actual continuation semantics.
11. Input parsing at 1 MiB/1 MiB+1, unknown/duplicate fields, inline secret-shaped fields, dry-run/yes/run conflicts, non-TTY missing authorization, expected-digest match/mismatch, and baseline-change races have exact fixtures with zero mutation on every rejected path.
12. Active/stale/wrong/remote runtime ownership fixtures cannot Apply or create a direct-store fallback; stopped-runtime Apply returns durable revisions and any requested continuation starts only afterward.
