# CR-02 — Persistent Wiring, Capabilities, and Health

- **Priority:** P0/P1
- **Depends on:** CR-01 typed context/output/client
- **Blocks:** CR-03 runtime state, CR-04 canonical grouping

## Scope and ownership

Primary targets:

- `src/cli/{agents,sessions,cron,channels,config,memory,backup,models,gateway}.rs`
- new `src/cli/{conversations,jobs,skills,learning,capabilities}.rs` focused modules
- `src/async_main/command_dispatch.rs`
- `src/async_main.rs`
- `src/boot_screen.rs`
- `crates/thinclaw-tools/src/registry.rs` and `src/tools/registry.rs`
- `crates/thinclaw-agent/src/{agent_registry,session_manager,routine_engine}.rs`
- `crates/thinclaw-db/src/lib.rs` and backend implementations only where a missing neutral query is required
- `crates/thinclaw-app/src/` for neutral capability/report types
- `src/channels/web/handlers/{routines,jobs}.rs` and `crates/thinclaw-gateway` response types for API contract additions
- `src/api/extensions.rs`, `src/channels/web/handlers/extensions.rs`, `crates/thinclaw-gateway/src/web/extensions.rs`, and desktop extension proxy/contracts for kind-aware runtime lifecycle actions
- `src/api/learning.rs`, `src/channels/web/handlers/learning.rs`, `crates/thinclaw-gateway/src/web/learning.rs`, learning DB traits/backends, and desktop learning contracts

## CR-02.1 — Shared durable service construction

**Covers:** INV-36, INV-37, INV-95 and prevents future disposable-state or split-brain handlers.

Implement:

1. Add lazy, fallible service construction to `CliContext`:
   - resolved `Arc<dyn Database>`/store using the same backend, migrations, path/URL, and credential rules as the runtime;
   - durable `ConversationStore` access;
   - `AgentRegistry` construction using an `AgentRouter` plus that database;
   - `GatewayClient` from CR-01 for runtime-owned state/actions;
   - one `RuntimeMutationCoordinator` backed by the private owned-instance/PID record, operation lease, authenticated runtime identity, and selected local/remote profile. It never decides that a runtime is stopped from a failed port probe alone.
2. Reuse the existing `crate::db::connect_from_config` factory. Remove command-local backend construction where it duplicates this helper; do not create a second CLI database factory.
3. Add neutral shared types and include their metadata in the CLI contract manifest/generated reference:

   ```text
   MutationExecutionPolicy = embedded_runtime | durable_immediate |
                             active_coordinated | runtime_required |
                             stopped_exclusive | owned_process_lifecycle |
                             external_direct
   MutationApplication = durable_applied | applied_live | restart_required |
                         runtime_not_running | process_lifecycle_applied |
                         external_effect_applied
   MutationRequest { request_id: UUID, expected_durable_revision?, payload }
   MutationReceipt {
     request_id, policy, domain, resource_id?, durable_revision?,
     runtime_instance_id?, runtime_revision?, external_operation_id?,
     application, restart_reasons[], partial, recovery?
   }
   ```

   `embedded_runtime` covers `run`/`tui`/`ask` and emits lifecycle/turn events rather than pretending a separate admin owner. `durable_immediate` is valid only where the store is authoritative and no stale live cache/object can be affected; an in-use resource still returns a conflict. `active_coordinated` covers agent registry CRUD/defaults, routine definitions, mutable/model/channel settings and credential bindings, skill/tap/catalog mutations, installed-extension lifecycle, and any other runtime-cached domain. `runtime_required` covers send, routine trigger, active jobs/devices, live probes/evaluation/provider activation, extension activation, and skill reload. `stopped_exclusive` covers setup Apply, reset, backup import/master rotation/update replacement, and cross-domain migration. `owned_process_lifecycle` covers web/service/managed-sidecar start/stop/reload with its private ownership records. `external_direct` covers explicitly requested bounded external/file effects that own no cached ThinClaw state; if such an effect also changes a catalog/settings revision, it is instead active-coordinated. Exact leaf metadata—not handler guesswork—selects the policy.
4. Route by owned runtime state:
   - when no owned runtime is active, `durable_immediate` and `active_coordinated` call the same shared service directly and return `runtime_not_running`; `runtime_required` fails precisely; `stopped_exclusive` acquires the lease and proceeds;
   - when an owned runtime is active, `active_coordinated` and `runtime_required` use its authenticated service. If that service is unreachable, wrong-instance, or the selected remote profile cannot perform the operation, fail before a write with `runtime_coordination_unavailable`; never instantiate a local fallback. `stopped_exclusive` fails with stop/remediation. `durable_immediate` may remain direct only after its domain's in-use policy proves no cached object is affected;
   - a bootstrap/config change that is safe to persist but cannot hot-apply is still sent through the active runtime coordinator and returns `restart_required` with exact keys/reasons. It is not called live merely because persistence succeeded.
5. The runtime-side shared service performs authorization and optimistic revision validation, persists, applies/reconciles live state, and publishes its new revision as one failure-aware operation. On live-apply failure it either rolls persistence back to the prior revision or returns `partial=true`, exact durable/live revisions, `restart_required`, and a recovery action; it never emits `applied_live`. A successful response is observable through a subsequent read/snapshot at the returned revisions.
6. Make retries concrete. The outermost client creates one `MutationRequestId`; compatibility adapters preserve it. After replacing any secret input with its staged source/ephemeral ID, compute a keyed canonical request fingerprint using host-held key material—never store a raw payload, raw/unkeyed secret hash, credential locator/value, prompt/note text, or file content. Add backend-parity `mutation_receipts` storage keyed by `(principal, domain, request_id)` containing only fingerprint, state, safe receipt, and timestamps. A same-ID/same-fingerprint retry returns the original receipt/resource ID; same ID with a different fingerprint is `idempotency_conflict`. `MUTATION_RECEIPT_TTL` is 24 hours and `MAX_MUTATION_RECEIPTS_PER_PRINCIPAL_DOMAIN` is 10,000; transactional pruning removes terminal oldest/expired rows without deleting an in-progress claim. Concurrent claims execute once. External operations without provider idempotency are never automatically replayed after an unknown outcome and return an exact reconciliation action.
7. Use the same policy from CLI, hidden aliases, gateway/Web/desktop adapters, setup, and agent admin tools. A compatibility path cannot choose a different state owner. Desktop remote selection preserves D-46: it never mutates the embedded local runtime as fallback. Finite operations return `MutationReceipt`; long-running `embedded_runtime`/process surfaces emit matching start/accepted/terminal events with request/instance/operation IDs.
8. Every request includes explicit principal/user and applicable channel/workspace scope with stable defaults; output includes effective scope plus the `MutationReceipt` in verbose or machine metadata. Human mutation output states `applied live`, `saved; restart required`, `saved while runtime stopped`, `process lifecycle applied`, or `external effect applied` as applicable.
9. Database initialization/migration and runtime-coordination errors are operational exit `1`, never an empty successful list or a downgrade to direct state.

Acceptance:

- no terminal handler constructs a fresh state container merely to satisfy its API;
- CLI and runtime use the same configured DB factory;
- stopped-runtime direct operations work through the shared service and return `runtime_not_running`;
- active-coordinated operations mutate the live service/revision or fail before writing, and runtime-required operations fail precisely when stopped;
- persisted/live revision mismatch is explicit `restart_required`/partial state, never a successful-live claim.

Tests:

- unit-test source/backend selection;
- black-box two-process tests with an isolated ThinClaw home;
- active-runtime versus stopped-runtime routing, wrong/stale instance records, unreachable active gateway, remote-profile no-local-fallback, revision CAS, same/different-digest/concurrent request-ID replay, receipt retention bounds, persist-then-live failure/rollback, external unknown-outcome, and receipt/snapshot revision tests;
- backend contract tests for libSQL and PostgreSQL where CI supplies the latter.

## CR-02.2 — Persist agent administration

**Covers:** INV-37.

Current defect: `run_terminal_command` creates `AgentRouter::new()`, calls `run_agents_command`, prints success, and exits; the next process sees nothing. The runtime already builds `AgentRegistry::new(router, db)` and calls `load_from_db()` in `src/async_main.rs`.

Implement:

1. Change `run_agents_command` to consume a shared agent-admin service/`AgentRegistry`, return `Result<AgentCommandResult, CliError>`, and emit no output itself.
2. Extract agent CRUD request/result types into `thinclaw-agent` so terminal CLI, runtime tools, and other surfaces share validation and persistence logic.
3. For every invocation, construct the registry with the durable DB and call `load_from_db()` before read/mutation, or provide an equivalent registry factory that guarantees loading.
4. `add` validates unique ID/name, workspace path policy, model/profile, tool/channel allowlists, and persists before reporting success.
5. `remove` and `set-default` persist transactionally. Refuse removal of a default/in-use agent unless the domain service already has a safe reassignment rule; return a precise conflict.
6. `show <missing>` returns exit `1` with a typed not-found error. List returns `[]` in JSON when empty.
7. Add canonical `agents update ID` so the CLI reaches the registry's existing update capability. Keep `set-default` as a convenience adapter over the same transactional update service. Do not add an `agents message` admin leaf: the existing `message_agent` tool is an active-runtime, identity-scoped inter-agent action, not durable registry CRUD; user-originated runtime messaging remains `send`/`ask` with explicit agent selection.
8. Apply CR-02.1's `active_coordinated` policy. With a running runtime, CRUD/default mutations call that exact in-process `AgentRegistry` through its authenticated admin port so persistence and router state share one revision; an unreachable active instance fails before mutation. With no runtime, construct the durable registry directly and return `runtime_not_running`. Reads may use the durable store but include live revision/drift when a runtime is active.
9. Runtime startup reads changes made by a previous terminal process without any alternate storage format.

Acceptance tests:

1. process A: `agents add`; process B: `agents show/list` observes it;
2. process B: set default; process C/runtime load observes the default;
3. process C: remove; process D list no longer includes it;
4. duplicate add and missing show/remove are nonzero and parseable in JSON;
5. empty list contains no prose/ANSI.
6. an active runtime observes add/update/default/remove at the returned revision immediately; stopped mode persists for the next load; stale/unreachable active-instance fixtures neither write nor create a second router.

## CR-02.3 — Replace sessions with durable conversations

**Covers:** INV-36, INV-59.

Implement canonical `data conversations` over the existing `ConversationStore` APIs, including `list_conversations_with_preview`, `get_conversation_metadata`, message pagination/window queries, counts/search, and deletion.

Canonical commands:

```text
data conversations list [--principal ID] [--channel NAME] [--limit N] [--cursor TOKEN]
data conversations show ID [--messages N] [--before MESSAGE_ID]
data conversations search QUERY [--principal ID] [--channel NAME] [--limit N]
data conversations export ID --artifact-format markdown|json [--out FILE]
data conversations delete ID [--dry-run] [--yes]
data conversations prune --older-than DURATION [--principal ID] [--channel NAME] --dry-run
data conversations prune --older-than DURATION [scope...] --yes
```

Rules:

1. Never call these records “active sessions.” List output shows conversation ID, channel, principal, updated time, message count, and preview.
2. `show`/`export` use real ordered persisted messages, with roles/timestamps/tool records represented consistently. `export --artifact-format markdown|json` is an artifact command: stdout contains only the artifact, while `--out` writes only the artifact and emits an optional typed status record under the global presentation contract.
3. Destructive selection is server/store-side and scoped. `prune` defaults to dry-run; execution requires `--yes`. A zero-match prune is successful with count zero. Invalid/unknown conversation IDs are not success.
4. If store APIs lack cursor-based list/delete-by-scope, add backend-neutral trait methods and parity implementations/tests rather than pulling all records into memory.
5. Keep `SessionManager` for genuine in-process coordination only. Remove its use from terminal CLI.
6. Hidden `sessions` compatibility paths translate to durable conversation requests and warn. Do not retain the old in-memory behavior.

Acceptance tests:

- process A/runtime stores a multi-message conversation; process B list/show/export sees exact messages;
- pagination has no duplicates/gaps under a stable fixture;
- dry-run lists/counts exactly what execution deletes;
- execution without `--yes` is refused nonzero;
- libSQL/PostgreSQL contract parity;
- JSON export round-trips and Markdown export has deterministic ordering.

## CR-02.4 — Make routine triggering real

**Covers:** INV-25.

Current defect: `src/cli/cron.rs` `Trigger` prints a gateway POST instruction instead of performing it. The gateway already has `POST /api/routines/{id}/trigger`, and `RoutineEngine::fire_manual` returns the created run UUID, but the handler currently discards it.

Implement:

1. Extend the additive gateway response for manual trigger to include `run_id` and initial `status` while retaining `routine_id`. Capture the UUID returned by `fire_manual`.
2. Update the shared gateway response type/OpenAPI/tests. Existing clients tolerate the additive fields.
3. Implement CLI trigger through `GatewayClient`; require the running runtime because execution belongs to its engine and registered tool/agent context.
4. Print/serialize routine ID, run ID, and accepted/initial status. Do not ship `--wait` in this refinement: the audited trigger contract does not provide a dedicated terminal run-status polling contract. A future additive polling task is separate.
5. Disabled/missing/foreign routine, auth failure, and stopped runtime are nonzero typed errors.
6. Keep list/runs/lint as authoritative durable reads. Mark add/edit/remove `active_coordinated`: when the routine engine is running, call its shared definition service, persist at an expected revision, refresh the engine schedule/event cache, and return the live revision; when stopped, write through the same service for the next load. An unreachable owned runtime is not a direct-DB fallback. Trigger remains `runtime_required`.
7. Route all results through the typed output boundary and rename the public group in CR-04.

Acceptance tests:

- mock/in-process gateway trigger creates one real routine run and returns its UUID;
- CLI output run ID resolves through routine run history;
- repeated invocation creates distinct runs;
- no success text is emitted when gateway execution is unavailable;
- JSON response has stable IDs/status and no instruction prose;
- active add/edit/remove changes the engine's next schedule at the returned revision, stopped mutation loads on next startup, and live-refresh failure rolls back or returns an exact partial/restart-required receipt.

## CR-02.5 — Expose actual job operations

**Covers:** INV-27, INV-79.

Add typed `automation jobs` adapters over the existing authenticated routes:

```text
automation jobs list [--state STATE] [--backend direct|sandbox] [--limit N] [--cursor TOKEN]
automation jobs summary
automation jobs show ID
automation jobs cancel ID [--yes]
automation jobs restart ID [--yes]
automation jobs prompt ID TEXT
automation jobs events ID [--limit N] [--after EVENT_ID]
automation jobs files list ID [PATH]
automation jobs files read ID PATH [--out FILE]
```

Rules:

1. Use `GatewayClient`; do not duplicate HTTP/auth configuration.
2. Preserve ownership-based 404 behavior and server-side authorization. CLI confirmation is defense in depth, not authorization.
3. `cancel`/`restart` require confirmation or `--yes`; noninteractive invocation without `--yes` fails before mutation.
4. `prompt` clearly reports when a job is not interactive.
5. Extend `GET /api/jobs` additively with bounded server-side `state`, `backend`, `limit`, and opaque `cursor` query fields. Default/max limits are `50`/`200`; sort deterministically by creation time and ID. Do not pull an unbounded list into the CLI and paginate locally.
6. Extend `GET /api/jobs/{id}/events` with `limit` (default `100`, max `1000`) and `after` (exclusive event ID), backed by parity DB queries. Return `next_after`/`has_more`. This release is a bounded ordered snapshot only: there is no `--follow`, SSE reconnect, or synthetic cursor behavior.
7. File reads preserve the endpoint's actual contract: bounded UTF-8 text in `ProjectFileReadResponse`. `--out` writes that UTF-8 content atomically; stdout artifact mode prints only the content. Binary/non-UTF-8 files return a precise unsupported error. Preserve server path containment and never execute/open artifacts.
8. Do not expose internal worker/bridge commands as a substitute for job administration.

Acceptance tests:

- list/show across direct and sandbox job projections;
- mutation confirmation and state conflicts;
- prompt accepted/rejected states;
- event ordering, exclusive `after`, limits, and pagination without gaps/duplicates;
- file path traversal rejection and exact UTF-8 output; binary input is rejected;
- auth, 404 ownership masking, body bound, and JSON output.

## CR-02.6 — One source-aware configuration and credential boundary

**Covers:** INV-22, INV-23, INV-24, INV-26, INV-28, INV-83, INV-92.

Current defect: `config` describes DB-to-disk fallback while `load_settings(None)` returns defaults in paths that do not implement that claim; global `--config` is not consistently honored; get/set output may reveal secrets. The same audit found no complete process boundary: raw launch APIs and inherited environments are dispersed across runtime, desktop, tools, channels, media, and service code, including generated local-inference bearers in desktop argv/env. This task defines the resolver, source, and environment-policy primitives; CR-02.20 applies them exhaustively to process launches.

Implement:

1. Add the schema/merge engine to `crates/thinclaw-settings/src/resolver.rs`. It accepts source maps/typed documents without depending on `thinclaw-db`; a root adapter fetches DB maps. Every schema key declares one class: `BootstrapInfrastructure`, `MutablePreference`, `SecretReference`, or `RuntimeOnly`.
2. Resolve in two explicit phases to avoid a database bootstrap cycle:
   - **Phase A — bootstrap:** compiled defaults → compatibility JSON (migration input only) → selected/default TOML → bootstrapped environment → explicit leaf overrides. It resolves database backend/location/credentials, ThinClaw home, selected TOML, gateway bind, and other values required before DB connection. DB values are prohibited for this class.
   - **Phase B — hydrated runtime:** compiled defaults/legacy migration → persisted DB mutable preferences → selected/default TOML → environment → explicit leaf overrides. Bootstrap fields are copied from Phase A and cannot be replaced by DB rows. Runtime-only values are never persisted.
3. Record `ConfigSource` and resolution phase per field. Human `config get/list --verbose` and all JSON results include source/phase metadata; ordinary human output omits it. Secret values are never included. Environment provenance is captured after the once-only dotenv/ThinClaw-env bootstrap.
4. An explicit global `--config PATH` must be read, validated, and honored or fail clearly; a missing/unreadable explicit path never silently falls back. `run`, `status`, and immediate commands consume the same resolved context.
5. Add schema metadata identifying secret/sensitive paths. General `config get/set/list` refuses secret values and points to `config secrets`; do not add `--show-secret` to general config. Define these distinct neutral types before CR-01.10 consumes them:

   ```text
   SecretSourceId(UUID)
   SecretSourceRef = EncryptedStore { secret_name } |
                     OsCredential { account } |
                     Environment { variable } |
                     PrivateFile { path }
   SecretSourceRecord {
     source_id, label, owner_principal, allowed_purposes[], source, revision
   }
   SecretBinding { source_id, purpose }
   ```

   Mutable runtime settings persist `SecretBinding`, never the source location or value. `SecretSourceRef` and the resolved domain record have manual redacted `Debug` and are not general API-`Serialize` types; persistence uses a private storage DTO, while public reads use a separate safe metadata DTO. Add backend-parity durable source-record methods to `thinclaw-secrets`; records are loaded through the shared service and authorization never relies on an ID being unguessable. In storage, keep source ID, owner, kind, sorted canonical purposes, and revision queryable, but encrypt and authenticate the serialized locator (`secret_name`, account, variable, or path) with those columns as associated data so rows cannot be swapped or repurposed. `EncryptedStore` points to an existing encrypted `SecretsStore` entry protected by the resolved master key. The existing `thinclaw_secrets::SecretRef` remains returned metadata for that encrypted record and is not an OS-keychain handle. `OsCredential` is restricted to schema-declared host/bootstrap purposes. Phase-A values needed before database connection may persist a direct redacted `SecretSourceRef` only in the private local bootstrap TOML; they do not consult the source-record table, and gateway/desktop/agent APIs can neither create nor select those bootstrap fields. Any TOML containing a direct ref is atomically written with owner-only Unix mode (`0600`) or the equivalent current-user/SYSTEM/Administrators Windows ACL; an existing broad-permission target fails closed with remediation rather than being read or silently chmodded. Parent directories receive owner-only permissions and symlink/reparse-point traversal is rejected.

   IDs are UUIDs; labels are unique per owner principal; labels/account/secret names are nonempty printable identifiers capped at 256 bytes; environment names match `[A-Za-z_][A-Za-z0-9_]{0,127}`. `SecretPurpose` is a closed tagged type serialized with these catalog-derived families: `database`, `gateway:<slot>`, `llm:<provider>:<slot>`, `embedding:<provider>:<slot>`, `channel:<service>:<field>`, `external-memory:<provider>:<slot>`, `extension:<kind>:<source-id>:<slot>`, `repo-project:<slot>`, `experiment:provider:<backend>:<slot>`, `experiment:runner-env:<name>`, `media:<provider>:<slot>`, `integration:<service>:<slot>`, `coding-worker:<worker>:<slot>`, and `tunnel:<provider>:<slot>`. Providers/services/kinds/slots come from compiled schemas, a validated signed dynamic manifest, or an owner-local typed registration record such as an ad-hoc MCP server's exact validated secret-env slot; remote DTOs cannot create such schemas. Environment names use the grammar above and dynamic source IDs use their canonical digest/ID. Arbitrary family names, unregistered free-form slots, prefix authorization, and wildcards are invalid. Sort/deduplicate purposes before authenticated storage and comparison. Source resolution rechecks owner, exact purpose, caller authorization, binding/source revision, and current consumer schema on every use.

   Reuse/generalize the `thinclaw-platform` private-input reader introduced for the experiment runner in CR-01.8: require an absolute path; reject symlinks, multiple hard links, nonregular files, and values over 64 KiB (or a stricter consumer cap); open then re-check metadata to resist replacement. On Unix require the effective user as owner and no group/other permission bits. On Windows require the current-user owner SID and reject broad read grants such as Everyone/Authenticated Users/Builtin Users while allowing current user, SYSTEM, and Administrators; inability to inspect the ACL fails closed. Validate at creation and every use. Values are held in zeroizing wrappers and never enter persisted settings, source records, plans, ordinary API DTOs, debug output, or caches. The deliberately local `config secrets --from-stdin` creation ingress and CR-01.8 runner-auth sink are narrow secret-bearing inputs with non-serializing types; remote consumer/configuration DTOs remain ID-only.
6. Define canonical writes exactly:
   - `config set KEY VALUE [--source database|toml]` defaults by schema class: mutable preference → database; bootstrap infrastructure → TOML. A database source for a bootstrap key and either source for secret/runtime-only keys is rejected.
   - `config reset KEY [--source database|toml]` follows the same routing and removes only that source's override.
   - TOML mutation requires an existing selected/default file (or a prior `config init`), preserves unrelated/comments where the chosen TOML editor supports it, validates the fully resolved result before atomic replace, and refuses symlink/non-regular targets.
   - DB mutation is transactional and only reports success after commit.
   - schema metadata declares `hot_apply | restart_required | stopped_exclusive` for every writable key. Through CR-02.1, a running runtime owns mutable/model/channel/binding writes and either applies/publishes the revision or returns exact restart-required keys; the CLI never bypasses its reload hooks. Phase-A database/backend/master-key changes are at least restart-required, and a migration/key-rotation action may be stopped-exclusive. Results always include `MutationReceipt`.
7. `config path` reports Phase A/Phase B source locations in precedence order; `config init --out PATH` refuses overwrite without `--force` and an interactive confirmation or noninteractive `--yes`.
8. Keep legacy `settings.json` read-only, migrate it once through the existing migration path, then rename it `.migrated`. It is never a new write target.
9. Refine `config secrets` without introducing a second value store:
   - `config secrets set LABEL [--from-stdin|--from-env VAR|--from-file FILE] --for PURPOSE... [--principal ID] [--replace]` defaults to a masked TTY value written to the existing encrypted `SecretsStore`; with no source flag in a non-TTY it fails with usage guidance rather than reading an implicit pipe. The three source flags are mutually exclusive. `--from-stdin` is explicit, bounded to 64 KiB, trims at most one terminal newline, and never echoes. Env/file forms create only a source record after validating/resolving the source once; no value is copied into the DB. At least one catalog-valid purpose is required. The effective principal is the authenticated/local CLI principal; `--principal` is an admin-only override and never grants authority by itself. A new label returns a generated source ID. Replacing an existing label requires interactive confirmation or noninteractive `--replace`, retains its ID, increments revision, and atomically revalidates every in-use binding before changing the backing source/purposes.
   - Remove the current `--value` option immediately rather than retaining an argument/history leak. Hidden `--provider` maps to the exact primary-provider purpose through 0.18; hidden `--user` maps to `--principal`. Conflicting old/new selectors are usage error `2`.
   - `status`, `list`, and a new `show ID` return source ID/label/kind/purposes/in-use/configured/resolvable metadata but never an encrypted-store name, OS account, environment name, file path, or value to remote callers. Local `--verbose` may return separately typed redacted location metadata, never the value.
   - `delete ID [--yes]` refuses an in-use source; consumers must be rebound/disabled first. Creating an unbound source is `durable_immediate`; replacing an in-use source or changing a binding is `active_coordinated` so owning services re-resolve at the returned revision. `rotate-master [--yes]` is `stopped_exclusive` and available only when the active Phase-A master-key source is the writable OS credential store; an operator-managed env/file source returns `operator_managed_source` before mutation. It re-encrypts both encrypted secret rows and every source-locator ciphertext before publishing the new OS-held master key, and reports that external env/file/OS **credential values** were not rotated. Rotation is all-or-rollback across both backends and restores the prior OS key if publication/commit fails. All source-record and encrypted-secret writes/deletes are transactional or return a typed partial result with no false success.
10. Migrate existing encrypted secrets and consumers without guessing. For each legacy setting with a known schema purpose, decrypt/retrieve the named `SecretsStore` record, create one owner/purpose source record, verify resolution through the new service, transactionally replace the consumer field with `SecretBinding`, then scrub only the legacy binding field. Existing encrypted records with no unambiguous consumer/purpose remain intact as `legacy_unbound` metadata and are not remotely selectable; `config secrets list` gives safe rebind guidance. Duplicate labels or provider metadata never authorize a purpose by themselves.
11. Check in `tests/fixtures/credential_consumer_manifest.json`, generated/verified from settings schemas, Clap arguments, HTTP/WebSocket routes and DTOs, OpenAPI/generated clients, agent-tool schemas, gateway/static-Web and desktop/Tauri/Specta forms, dynamic extension manifests, and process descriptors. The generator emits every schema-annotated sensitive field plus every secret-like candidate found by the scanner; checked records use this closed disposition:

    ```text
    SensitiveFieldDisposition =
      SourceBound { purpose, binding_field, local_ingress, resolver, legacy_fields } |
      BootstrapDirect { purpose, local_ref_field, resolver, restart_policy, legacy_fields } |
      EphemeralInternal { owner, generator, sink, ttl, replay_policy, cleanup } |
      ProtocolSensitive { protocol, authenticated_scope, storage, retention, rotation, revocation } |
      DeliberateReveal { protocol, guard, single_use_or_rotation } |
      NonSecretSemantic { semantic_type, evidence }
    CredentialConsumerRecord {
      id, surface, field_path, disposition, redacted_projection, proof_id
    }
    ```

    `SourceBound` is the only class allowed for mutable operator credentials and must name an exact catalog-valid `SecretPurpose` plus durable binding. `BootstrapDirect` is restricted to schema-declared pre-DB Phase-A infrastructure, persists only the redacted direct ref in the private local bootstrap file, is never remotely selectable, and declares restart/stopped behavior. `EphemeralInternal` cannot persist or cross an ordinary DTO; its named private transport and lifecycle test are mandatory. `ProtocolSensitive` exists only where the external protocol itself supplies/consumes the value and requires least-privilege route scope, encrypted/private storage where retained, bounded lifetime/retention, revocation, and redaction. `DeliberateReveal` is limited to the fixed gateway/device-pairing cases and their dedicated guards. `NonSecretSemantic` covers proven cursors/page tokens/model token counts and needs a typed semantic field plus test; it is not a free-form suppression. OAuth authorization codes/verifiers are protocol-sensitive and transient, while retained access/refresh material for a configured integration becomes source-bound after server-side exchange. Device bearer/push registrations, worker/job/sandbox/browser capabilities, and pagination-like fields must each be classified by semantics rather than spelling. There is no wildcard/regex ignore record, and no UI field or handwritten JavaScript request escapes classification merely because it is outside a Rust settings struct. The initial audited consumers and mandatory migrations are:

    | Consumer | Current unsafe/ambiguous path | Required binding/input contract |
    |---|---|---|
    | Bootstrap database/master/tunnel | Secret-bearing URLs, token fields, `.env`, and mixed secure-store fallback | Direct Phase-A OS/env/private-file refs only; strip URL userinfo; no DB source-registry dependency. |
    | LLM, embeddings, media, integrations, and coding workers | Multiple `SecretString`/name/env conventions, gateway `ProviderKeyRequest { api_key }`, Web provider-key forms, and desktop key fields | Schema slot → purpose-bound source ID; only the in-memory resolver yields a value. Desktop/gateway writes are ID-only and remote forms select authorized source metadata rather than accepting raw values. |
    | Utility/shell/build/sandbox/bridge subprocesses | Raw launch sites can inherit the entire ThinClaw environment, including unrelated provider/database tokens; preserving real `HOME`, shared temp, or unvalidated parent `PATH` would still expose credential files or executable-resolution control | One `ChildEnvironmentPolicy` calls `env_clear`; one `ExecutionContextPolicy` supplies a validated absolute executable or allowlisted normalized search path, launcher-owned private home/temp for untrusted work, contained cwd/filesystem roots, explicit network/isolation mode, and only named non-secret locale/terminal fields. Trusted runtime/service utilities may select an explicitly reviewed real-home or system-temp policy only when their operation genuinely owns that state. A toolchain/executor descriptor may add an enumerated validated non-secret path such as `CARGO_HOME`, `RUSTUP_HOME`, or `SDKROOT`; explicit job env follows the same schema. `LD_*`, `DYLD_*`, proxy/auth-agent variables, and every other ambient field are absent unless a separately reviewed descriptor declares the exact validated non-secret path/network field or exact credential slot. Only a typed consumer such as a coding-worker or MCP slot may resolve/inject its exact authorized binding; built-in shell/process/execute-code never inherit the agent's credential set. Arbitrary approved host mode is reported as `host_unconfined`, requires the existing high-risk approval policy, and is not called a sandbox; a fixed `reviewed_direct_host` consumer is separately identified and limited to its exact declared sink. |
    | Runtime/service/gateway/dedicated-user re-exec | Child runtimes inherit ambient state; gateway additionally writes an unused instance token, and dedicated-user launchers inject database URL/token fields directly | Pass only the selected config/home/runtime-mode controls and the exact Phase-A source variables required by the resolved bootstrap descriptor. Credential URLs are split into non-secret endpoint metadata plus an authorized source. Parent correlation nonces remain parent/PID-record state. Prefer an inherited pipe/private bootstrap artifact where the same ThinClaw binary controls both ends; never copy the complete parent environment. |
    | Desktop local-inference sidecars | vLLM and llama.cpp get generated bearer values in `--api-key`; MLX gets one in `THINCLAW_MLX_API_KEY`; managed STT writes `WHISPER_HTTP_TOKEN` into process-global bridge environment; backend structs and renderer-facing DTOs carry cloneable `String` token fields | Generate a per-launch `EphemeralSidecarAuth` in zeroizing/redacted backend state and deliver it through a reviewed inherited pipe or owner-private auth file. A pinned llama.cpp file-auth interface may receive only the private path; ThinClaw-owned vLLM/MLX launch adapters read an inherited descriptor/file and start the server without token argv/env. A typed in-memory local-endpoint registry replaces global bridge env. Delete token fields from every `Serialize`/Specta/renderer/runtime-contract DTO. No proven adapter means `auth_transport_unsupported`, not plaintext fallback. |
    | Channels | Plain token/password fields, `/api/nostr/key` plus `NostrPrivateKeyRequest`, gateway/static-Web secret inputs, and the disconnected desktop binding flag | CR-02.7 typed channel field bindings; replace special plaintext routes/forms with source-ID binding and preserve `secret_binding_available=false` until wired. |
    | MCP and extension credentials | Stdio `--env KEY=VALUE`, desktop `mcp_auth_token` form/config fields, OAuth/name conventions, WASM credential maps, and gateway/WebSocket/free-form chat token submission | `--env` is non-secret only; add repeatable `--secret-env KEY=SOURCE_ID`; reject credential-bearing URLs/command args. OAuth exchanges server-side into a namespaced source; manual auth binds an existing ID or creates one through local masked/stdin/file ingress, without activation. Delete ordinary remote/chat token DTOs and raw desktop fields. Signed manifests or an owner-local typed MCP registration declare exact slots. At spawn, remove ambient secret-shaped variables and inject only declared resolved slots directly—never through a shell. |
    | Repository projects | CLI `--value`, agent/gateway plaintext `value`, settings secret-name strings | Four catalog slots (`github-token`, `github-fork-token`, `github-app-private-key`, `github-webhook-secret`) use `repo-project:<slot>` bindings. Local masked/stdin/env/file creation only; agent/gateway accept source ID. |
    | Experiments | GPU `payload_json` API-key channel, secret-name references, arbitrary `env_grants_json`, and runner token bootstrap | Provider connect accepts `--credential-source ID`; runner secrets use `--secret-env NAME=ID`; generic config/env files reject secret-shaped content; CR-01.8 owns ephemeral lease-auth delivery. |
    | External memory | Inline API/embedding/header keys and arbitrary config | CR-02.19 distinct primary/embedding/header bindings and exhaustive legacy-map scrub. |

    Canonical repository-project credential creation is `automation projects set-credential SLOT [--from-stdin|--from-env VAR|--from-file FILE] [--replace]`; it delegates to the same local source-creation service, assigns the exact slot purpose, and atomically writes the binding. With no source flag it uses a masked TTY and fails in a non-TTY. Remove its current `--value` immediately. `repo_project_set_credential` becomes a binding tool with `{slot, secret_source_id}`; `repo_project_request_credential` may request a declared client's out-of-band secure-source UI but credential text never returns through the model/tool transcript. If the active client/runtime lacks that secure ingress, return `secure_input_unavailable` with the local `config secrets`/`automation projects set-credential` remediation and do not accept chat text as fallback. The gateway credential route accepts `{slot, secret_source_id}` only. Existing name fields migrate only when the named record and slot are unambiguous.

    Canonical experiment runner-profile writes replace inline `--backend-config-json`, `--gpu-requirements-json`, `--env-grants-json`, and `--cache-policy-json` with bounded non-symlink regular `--backend-config-file`, `--gpu-requirements-file`, `--env-file`, and `--cache-policy-file`; hidden old flags survive through 0.18 only after the same secret-shape/size/schema validation and cannot carry credentials. `--secret-reference NAME` becomes repeatable `--secret-env ENV=SOURCE_ID`. GPU provider connect is exactly `labs experiments providers connect PROVIDER --credential-source ID`; validate has no body, and launch-test accepts typed non-secret options only. Remove the current generic `--payload-json` immediately because its connect envelope does not match the gateway DTO and can put an API key in argv. The typed CLI/GatewayClient request must match the path DTO byte-for-byte.

    Target link/update replaces inline `--metadata-json` with `--metadata-file FILE`. Define one tagged `ExperimentTargetMetadata` catalog for all ten `ExperimentTargetKind` values, including exact common/system-populated fields and kind-specific fields; gateway, CLI, OpenAPI, generated clients, DB validation, and result projection use the same type. An optional namespaced annotations map accepts only bounded non-secret scalar/list values. Use `MAX_EXPERIMENT_METADATA_BYTES = 64 * 1024`, maximum depth `8`, maximum nodes `2_048`, key length `128`, and string length `4_096`; reject duplicate/unknown top-level fields, control characters, URL userinfo, sensitive query keys, and credential/header/cookie/private-key fields at any depth. The hidden `--metadata-json` alias may survive through 0.18 only after parsing through this exact schema. Non-object JSON is an error rather than silently becoming `{}`. A legacy row that fails validation is projected as `metadata_state=legacy_blocked` with field names only, cannot be linked/launched, and remains available for explicit replacement through `targets update --metadata-file`; never echo its unvalidated values.

Acceptance tests:

- table-driven precedence for representative scalar/list/nested fields;
- explicit missing config is nonzero;
- DB/TOML/env changes are seen identically by `run`, `status`, and immediate commands;
- active-runtime writes execute the declared hot-apply/restart/stopped policy, expose durable/live revisions, and never fall back to direct DB/TOML mutation after a coordination failure;
- secret-key get/set/list never emit the secret in human/JSON/debug/error output;
- concurrent/failed writes do not truncate configuration.
- bootstrap DB keys never depend on DB settings; phase-A and phase-B provenance fixtures match runtime behavior.
- libSQL/PostgreSQL source-record parity, locator-ciphertext tamper/swap detection, owner/purpose authorization, ID/label ambiguity, revision race, in-use deletion, OS-backed whole-store master-key rotation/rollback, and env/file-master refusal fixtures pass;
- `--value` is rejected before dispatch and known sentinels are absent from process arguments, shell history fixtures, output, records, settings, and failure diagnostics;
- masked/stdin/env/file creation resolves exact kind/purpose behavior, boundary caps, and generated source IDs; mutable settings contain only bindings, while pre-DB direct refs remain local-only and never enter remote DTOs.
- the credential-consumer manifest has no unclassified sensitive settings/Clap/HTTP/WS/OpenAPI/tool/desktop/Web-UI/dynamic-manifest field or generic JSON/env escape hatch; every source-bound record has an exact purpose/ID-only consumer, every other sensitive disposition has its complete lifecycle contract, and every `NonSecretSemantic` exemption has typed evidence. Adding a raw provider key, Nostr private key, MCP/extension token, or other sensitive form/DTO without a valid disposition and proof fails CI;
- repository-project CLI/agent/gateway and experiment provider/runner/target-metadata migrations exercise exact slots, source authorization, non-TTY behavior, legacy ambiguity, DTO parity, schema boundaries, blocked legacy projection, suspicious generic-field rejection, and sentinel absence from argv/transcripts/API/settings;
- MCP non-secret env versus secret-env classification, credential-bearing URL/arg rejection, OAuth/source binding, signed-manifest slot namespaces, and absence of HTTP/WebSocket/chat raw-token auth are exact fixtures.

## CR-02.7 — Split channel configuration checks from live probes

**Covers:** INV-29, INV-34, INV-90. File 07 contains the exhaustive current native/lifecycle/bundled/dynamic channel inventory.

Implement:

1. Create one typed `ChannelCatalog`/descriptor source consumed by setup, CLI, runtime activation, web settings, capability snapshots, and generated docs. It combines local surfaces; root-native drivers; the four native-lifecycle descriptors; all 16 embedded registry channel manifests; and validated installed WASM manifests. Delete `src/cli/channels.rs::KNOWN_CHANNELS`.
2. Model stable service ID separately from driver ID (`native:*`, `lifecycle:*`, `wasm:*`, `local:*`). Migrate overlapping `discord`/`matrix` configurations to an explicit selected driver, preferring an already-configured native driver; never activate two same-service drivers implicitly.
3. Rename static `validate` behavior to `check-config`. It checks build/platform availability, selected driver, enabled/configured state, schema, credential **references**, and local dependencies without claiming a connection succeeded or reading secret values.
4. Add `probe [name|--all]` using bounded, side-effect-minimized adapters declared by the catalog. Prefer a driver health method; where a service lacks a safe probe, return `not_supported` rather than sending a message.
5. Report independent compiled/platform/configured/selected-driver/registered/dependency/exposed/approval/health facts plus last probe time, origin, and reason. Local `repl`/`tui` are surfaces; remove the fictional aggregate registered ID `cli`.
6. Fix runtime selection so `run|tui|ask --channels none` disables root-native, lifecycle, gateway listeners, and WASM ingress. `configured` activates configured selected drivers; CSV names resolve exact service IDs and return typed unknown/ambiguous/unconfigured errors. Local REPL/TUI remain usable.
7. Keep hidden `validate` alias mapped to `check-config` with a human-stderr deprecation warning. Hot install/activate/remove increments the shared catalog/snapshot revision transactionally.
8. Land the neutral `FactState`, `DependencyState`, `HealthState`, `ProbeOutcome`, and applicable reason-code primitives in the target low-level capability module as part of this foundation, then use them in channel results. Do not build a channel-only readiness vocabulary. CR-02.10 later adds the aggregate `CapabilityItem`/`CapabilitySnapshot`, collectors, revision semantics, and final cross-origin reconciliation; this task does not collect or present an early snapshot.
9. Make every credential-bearing channel schema field a typed `SecretBinding`. Local CLI/setup may create a source through the CR-02.6 service and select its `SecretSourceId`; gateway, agent, and desktop mutation DTOs accept only that opaque ID and the target runtime revalidates owner plus the exact `channel:<service>:<field>` purpose. Reads return source ID/kind/configured/resolvable state, never the source location or value. Delete special plaintext mutations such as `/api/nostr/key`/`NostrPrivateKeyRequest` and their gateway/static-Web input; replace them with the same authorized source-ID binding route/form rather than maintaining a channel exception. Migrate a legacy plaintext channel credential only after encrypted-store write/read verification plus source-record creation, then swap the binding and scrub it transactionally; on failure retain the legacy source, disable that channel, and return field-name-only remediation. The current desktop `secret_binding_available=false` response remains the truthful gate until this adapter is wired, after which availability is derived from the runtime contract rather than hard-coded. Non-password channel settings continue to round-trip independently.
10. Channel configuration/driver/credential mutations are `active_coordinated`. The running `ChannelManager` validates and persists the expected catalog/config revision, then hot-restarts only the affected selected driver when its descriptor declares safe reload; otherwise it returns `restart_required`. Failure rolls back the persisted revision or returns the exact partial receipt. With no runtime, persist through the same service for next startup. An owned but unreachable runtime is never bypassed, and a selected remote desktop profile never mutates the embedded local channel manager.

Acceptance tests:

- static checks make zero network calls;
- probes honor timeouts and distinguish auth, network, unavailable dependency, and unsupported probe;
- configured-but-not-registered and registered-but-unhealthy are distinct;
- JSON reports deterministic dimension fields and timestamps appropriate to test fixtures;
- catalog identity fixtures contain every source named in file 07 and no hard-coded CLI-only exception;
- overlapping native/WASM driver migration and selection are deterministic and never double-register;
- `--channels none` opens no nonlocal native/lifecycle/gateway/WASM listener in every supported profile.
- desktop/gateway channel credentials accept only an authorized source ID, never a raw password/token/env name/path; unavailable secret binding disables only credential mutation and cannot silently persist plaintext;
- verified legacy credential migration is atomic and failure leaves the original source intact, the channel inactive, and all output secret-free;
- active hot-reload and restart-required driver fixtures return exact durable/live/catalog revisions; failed/unreachable/remote-profile coordination never writes through a local fallback.

## CR-02.8 — Enrich memory search provenance

**Covers:** INV-35.

`SearchResult` already carries `path`, `document_id`, and `chunk_id`; the CLI currently drops them.

Implement:

1. Return rank, score, path, document ID, chunk ID, and a bounded excerpt in human and JSON modes.
2. Define a stable citation string (`path#chunk=<id>` or the repository's existing citation convention) produced by a typed helper.
3. Keep full content opt-in (`--full`) and apply safe terminal truncation by display width/line count in human mode; JSON returns the documented excerpt/content field without ANSI.
4. `read` and `search` identifiers interoperate: an ID printed by search must be accepted by the relevant read path or be clearly labeled metadata-only.

Acceptance tests:

- every fixture result preserves all available provenance;
- stable rank ordering/tie behavior;
- filenames/control characters cannot inject terminal escapes;
- empty result is valid JSON/list output.

## CR-02.9 — Add skills administration without duplicating the runtime

**Covers:** INV-33.

Implement `extensions skills` over the same registry/catalog/remote-hub/quarantine/settings lifecycle used by the 17 registered runtime skill tools. Extract a shared `SkillAdminService`; the CLI must not fabricate a `JobContext` or bypass the runtime tools' policy/scan/provenance logic.

```text
extensions skills list [--source SOURCE]
extensions skills search QUERY [--source local|catalog|remote]
extensions skills inspect NAME
extensions skills read NAME
extensions skills check (--path PATH|--url URL|--stdin)
extensions skills install NAME [--url URL|--from FILE] [--force] [--approve-risky] [--yes]
extensions skills update NAME [--approve-risky] [--yes]
extensions skills audit [NAME]
extensions skills snapshot [--out FILE]
extensions skills publish NAME --target-repo OWNER/REPO [--execute] [--approve-risky] [--yes]
extensions skills remove NAME [--yes]
extensions skills reload [NAME|--all]
extensions skills trust NAME --to installed|trusted [--yes]
extensions skills taps list [--health]
extensions skills taps add OWNER/REPO [--path PATH] [--branch BRANCH] [--trust-level LEVEL] [--replace] [--yes]
extensions skills taps remove OWNER/REPO [--path PATH] [--branch BRANCH] [--yes]
extensions skills taps refresh [OWNER/REPO] [--path PATH]
```

Rules:

1. These commands map one-for-one to `skill_inspect`, `skill_read`, `skill_list`, `skill_search`, `skill_check`, `skill_install`, `skill_update`, `skill_audit`, `skill_snapshot`, `skill_publish`, four tap operations, `skill_remove`, `skill_reload`, and `skill_trust_promote`. Do not add `enable`/`disable` or a quarantine-release state: no such durable lifecycle exists in the audited service.
2. Network/content input remains bounded, HTTPS/source/provenance rules are preserved, and every install/update passes the quarantine scanner. Risk acceptance requires both `--approve-risky` and the normal mutation confirmation.
3. Publish defaults to dry-run. Remote GitHub write requires `--execute --yes`, a configured matching tap, and the existing draft-PR flow. `--execute` cannot be inferred from `--yes`.
4. Snapshot is an artifact command. Trust promotion, remove, install/update overwrite, tap replacement/removal, and remote publish use the common mutation confirmation policy and audit trail.
5. Install/update/remove/trust/tap/publish mutations are `active_coordinated`: a running runtime uses its exact `SkillAdminService`, updates the durable catalog/artifact state, reconciles the live registry where applicable, and publishes the returned revision. With no runtime, the same service persists for startup. `reload` is `runtime_required` and never constructs a throwaway registry. An owned unreachable runtime fails before mutation; source scanning/download staging may occur before the final coordinator check only in a private disposable area and is cleaned without publish.

Acceptance:

- CLI and agent tool views return the same skill identity/status;
- mutations persist and are observed after process restart;
- active mutations are observed by the live skill registry at the returned revision, stopped mutations load on startup, and reload/unreachable-active cases cannot succeed against an ephemeral service;
- all 17 runtime operations have an intentional CLI disposition and shared policy tests;
- no second catalog, manifest parser, or policy implementation is introduced.

## CR-02.10 — Define the multidimensional capability snapshot

**Covers:** INV-06, INV-07, INV-20, INV-21, INV-29, INV-34, INV-55, INV-61, INV-81, INV-82, INV-84, INV-86, INV-90, INV-95.

Complete the neutral serializable types in `crates/thinclaw-app/src/capabilities.rs`, building on the fact/probe primitives landed with CR-02.7, and re-export them from that crate. Keep collection in the root runtime, so `thinclaw-app` does not gain dependencies on database, agent, channel, or tool implementation crates. Do not model readiness as one ordered enum:

```text
FactState = yes | no | unknown | not_applicable
DependencyState = ready | missing | degraded | unknown | not_applicable
HealthState = healthy | unhealthy | unknown | not_probed | not_supported
ApprovalPolicy = never | risk_based | always | unknown
CapabilityReasonCode = not_compiled | platform_unsupported | not_configured |
                       configuration_invalid | disabled_by_operator |
                       driver_unselected | driver_ambiguous | dependency_missing |
                       dependency_degraded | secret_unresolvable | auth_missing |
                       auth_failed | policy_denied | approval_required |
                       runtime_initializing | runtime_stopped |
                       runtime_coordination_unavailable | restart_required |
                       revision_drift | partial_apply | registration_failed |
                       name_conflict | probe_timeout | probe_unreachable |
                       probe_protocol_error | probe_failed | not_probed |
                       probe_not_supported
ProbeOutcome = healthy | unhealthy | not_probed | not_supported | timeout |
               unreachable | auth_failed | policy_denied | protocol_error
CapabilityItem {
  id, kind, compile, configuration, registration, dependency,
  exposure, health, approval_policy, reasons[], provenance, last_checked_at
}
BuildProfile = edge | light | desktop | full | minimal-libsql | minimal-postgres |
               all-features | custom
BuildFingerprint { build_profile, enabled_runtime_features, target_os, target_arch, database_backends }
RuntimeRevisionState {
  domain, runtime_instance_id?, durable_revision, applied_revision?,
  drift = none | restart_required | partial, reasons[]
}
CapabilitySnapshot {
  schema_version, generated_at, build, runtime, database, llm,
  revisions: RuntimeRevisionState[], agents, tools, channels, routines, gateway
}
```

Rules:

1. Types contain facts, not colored strings or UI labels.
2. Runtime collection occurs at the explicit preparation boundary after all startup registrations, including final scheduler-bound jobs, send-message/Nostr, subagents, LLM/advisor initial state, persistent agents, routines, channels, and configured dynamic extensions. Add a `ToolCapabilityDescriptor`/registry snapshot with stable name, origin (`core|memory|dev|job|extension-admin|skill|learning|repo-project|media|desktop|hardware-bridge|channel|subagent|llm|agent|routine|wasm|mcp|user-tool|native-plugin`), source ID/digest where dynamic, registry revision, compile/runtime predicates, active-profile exposure, approval policy, and unavailability reasons. Never infer capability from a hard-coded expected count.
3. Static snapshots do not perform network I/O. Live snapshots add bounded probes and record timestamps.
4. A database is `configured` when enabled/settings resolve, `healthy` only after a successful connection/query.
5. Tool totals are derived from the exact identity ledger in file 07 and broken down by independent dimension, origin, and active-profile exposure. Channel totals come from the variant-aware catalog and distinguish config, selected driver, registration, dependency, and health. “approval required” is policy, not progress.
6. Gateway origin is credential-free. Tokens are never snapshot fields.
7. Unknown/not-probed survives serialization; it is not coerced into false/healthy. Build fingerprints expose only a compile-time allowlist of non-secret feature names; test-only `integration`/`schema-divergence` are never runtime capabilities.
8. `CapabilityReasonCode` and `ProbeOutcome` are closed and machine-stable for schema version 1. Human remediation is separate safe text. Multiple reasons may coexist, but producers must use the most specific applicable code: platform exclusion is not `not_compiled`, invalid configuration is not `not_configured`, ambiguous/unselected drivers are not generic configuration failure, an unresolvable secret is distinct from rejected credentials, a registry collision is not generic `registration_failed`, and timeout/transport/protocol/unsupported probes are distinct. `approval_required` describes a pending execution gate while `ApprovalPolicy` describes standing policy. Map a nonhealthy `ProbeOutcome` to `HealthState` plus the corresponding reason code without discarding the probe outcome in detailed reports.
9. Desktop fleet/capability views, local/remote route gates, and other shared clients consume this snapshot or a lossless versioned projection. They do not derive capabilities from gateway connection counts or a hand-maintained list. If the runtime supplies no task identity, `current_task` remains absent; initialized/connected is not “Ready” and is not an active task. Preserve `thinclaw_spawn_session` and its desktop-managed list/update operations as `LocalOnly`: when a remote profile is active, return the typed capability gate before any local session mutation. This desktop route is distinct from the registered `spawn_subagent`/`list_subagents`/`cancel_subagent` agent tools.
10. Include CR-02.1's durable/applied revisions and drift reasons for every runtime-cached mutable domain. Static stopped-runtime status may report a durable revision with no runtime instance; an active runtime never substitutes its startup revision for a newer durable revision. `restart_required` and `partial` are explicit degraded facts until the applied revision catches up. Sensitive field names/values do not enter reasons.

Acceptance tests:

- a fixture with late registrations reports final counts;
- all 124 static catalog IDs have one typed descriptor/disposition, while each profile snapshot contains exactly its actual subset;
- MCP/WASM/user/native dynamic entries retain source provenance and do not shadow a static identity;
- disabled DB does not say connected; configured DB with failed probe is not healthy;
- no token/secret field serializes;
- status, doctor, boot, TUI, `/tools`, and channel views render the same revisioned fixture without changing facts.
- every negative/unknown fact emitted by a fixture uses a declared reason code; unknown codes fail deserialization only when the negotiated schema version requires strict decoding.
- desktop local/remote fixtures preserve the same capability identities/revision, keep an absent task absent, and prove a remote-selected desktop child-session request cannot fall through to the embedded runtime.
- active/stopped/restart-required/partial fixtures expose exact durable/applied revisions consistently across CLI/gateway/desktop; no stale runtime is rendered as current after a direct-store revision changes.

## CR-02.11 — Rebuild status and doctor on typed reports

**Covers:** INV-20, INV-21.

Implement:

1. Replace the Linux-named CLI type with a neutral `ReadinessProfile::{Server, Remote, Desktop, PiOsLite64, AllFeatures}` in the shared capability contract. Canonical `--readiness-profile` values are `server|remote|desktop|pi-os-lite-64|all-features`, default `server`. Preserve hidden `--profile` and `desktop-linux`/`desktop-gnome` aliases through 0.18 with conflict rejection and human-stderr deprecation only. Extract common checks plus one application-root-owned `PlatformReadinessProvider`; adapt the existing `src/platform/linux_readiness.rs` probes rather than duplicating them, and add macOS/Windows providers for their service manager, filesystem/runtime prerequisites, desktop permissions/dependencies, and feature prerequisites. Every check descriptor declares supported OS/arch and which profiles require it. `desktop` dispatches the current platform provider; `pi-os-lite-64` is required only on Linux aarch64 and remains parseable elsewhere, where it yields a required `platform_unsupported` check instead of disappearing from scripts. Do not reuse setup's `OnboardingProfile` or the compiled `BuildFingerprint.build_profile` field.
2. `status` defaults to a fast, compact static snapshot: runtime/service state if locally discoverable, configured DB/LLM, active agent/readiness profile, compiled build profile, and dimension-qualified counts. Static status exits `0` unless snapshot collection itself fails.
3. Make tool inventory a real nested subcommand instead of an overloaded flag:

   ```text
   status [--readiness-profile PROFILE] [--live]
   status [--readiness-profile PROFILE] tools [NAME] [--all] [--match GLOB]
          [--origin ORIGIN] [--compiled FACT] [--configured FACT]
          [--registered FACT] [--dependency DEPENDENCY] [--exposed FACT]
          [--approval POLICY] [--health HEALTH] [--live]
   ```

   Model this as `StatusArgs { scope: StatusScopeArgs, area: Option<StatusArea> }`, where `StatusArea::Tools(ToolStatusArgs)` owns only inventory-specific arguments. Mark the shared `StatusScopeArgs` fields global within the `status` subtree so `--readiness-profile` and `--live` parse either before or after `tools`, normalize to one value, and reject conflicting duplicates. They are not root-global options. Generated help uses the canonical before-subcommand spelling shown above.

   `FACT` is `yes|no|unknown|not-applicable`; dependency, approval, and health values are the hyphenated CLI spellings of the shared enums. Origin and fact filters are repeatable: values within one dimension are OR, different dimensions are AND. `NAME` is an exact case-sensitive stable ID capped at 256 bytes and conflicts with `--match`; the match expression is capped at 256 bytes, rejects control characters/brackets/backslash escaping, and supports only literal characters plus `*`/`?`. Invalid syntax is usage error `2`. Default population is entries with `registration=yes`; exact `NAME` searches the full static plus installed-dynamic descriptor catalog; `--all` and `--match` populate that full catalog. Unknown exact names exit `1` with `not_found`; a valid multi-result filter with no matches exits `0` and returns `[]`. Sort by `(origin, name, source_id)`. Do not add a synthetic `--state`, `ready`, or `unavailable` filter: consumers inspect/filter the independent facts. JSON returns every fact and the one snapshot revision; human summary counts are derived and never replace identities.
4. `status --live` performs bounded probes (DB query, selected LLM discovery/metadata readiness with no chat generation, gateway health, safely probeable channels/MCP). Use named defaults of 5 seconds per probe and 15 seconds for the whole concurrently scheduled report; preserve an existing stricter probe cap. Once the global deadline expires, unfinished checks become typed `timeout` results in the complete report. `status tools --live` probes only the selected safely probeable descriptors and leaves the others `not_probed`/`not_supported`.
5. `doctor` is already the active diagnostic command; do not add `--live`. It runs bounded local/external checks appropriate to `--readiness-profile` and returns `DoctorReport { schema_version, readiness_profile, build_profile, checks, required_failed, optional_failed, skipped, remediation }`. Checks declare why they are required for the selected readiness profile and avoid billable generation.
6. Exit mapping: healthy required checks `0`; completed report with required failures `3`; inability to execute diagnosis `1`; Clap usage `2`. Optional failures alone remain exit `0` but produce degraded state. `status --live` also exits `3` when a check required by its selected readiness profile is unhealthy.
7. Both commands support the global human/JSON contract. JSON schema has a version field.
8. `doctor` never mutates state unless a future explicit `--fix` command is separately added; no `--fix` in this scope.

Acceptance tests:

- required failure exits `3` while emitting complete parseable report;
- optional-only failure exits `0` with degraded summary;
- canonical/legacy readiness flags and values map identically during the window, conflict when mixed, and remain distinct from setup/build profile fields;
- `desktop` selects current-platform desktop checks and unsupported `pi-os-lite-64` produces a complete required platform failure rather than a parser omission;
- probe timeout is classified and bounded;
- static status performs no network request;
- live status never labels unprobed components healthy;
- shared status-scope flags parse before or after `tools`, normalize identically, reject conflicting duplicate values, and remain invalid outside the `status` subtree;
- `status tools`, `/tools`, boot, and TUI fixtures use identical sorted identities/facts for a given revision, including exact lookup, zero-result multi-filters, all-catalog population, and independent fact filters.

## CR-02.12 — Make boot output truthful and compact

**Covers:** INV-06, INV-08, INV-61, INV-82, INV-86.

Implement:

1. Split runtime startup into `assemble` → `prepare` → `seal/snapshot` → `present` → `run`. Before sealing, construct the final scheduler/job host, subagent executor, LLM/advisor initial state, agent registry, and routine engine; register each tool once against its final service. Move routine construction/registration out of the first portion of `Agent::run` into preparation without starting its loops.
2. Load/activate configured startup channels and dynamic MCP/WASM/native extensions before the seal. Reconcile intentional entries, compare the sorted live registry identity set to its descriptors, assign startup revision `N`, and construct boot input from that exact credential-free `CapabilitySnapshot`.
3. Remove duplicated mission/readiness/health blocks. Default human REPL boot is one 80-column/8-nonblank-line card at most: version/agent/model on one line; conversation/workspace; database; dimension-qualified tool/channel summary with revision; one degraded/next-action line when needed; credential-free web origin only if active. Omit absent optional rows rather than placeholders. Detailed inventories live in `status`, `status tools`, or `/status`, not startup.
4. Mark degraded/unknown facts with their exact dimension/reason. Do not equate `!no_db` with connected.
5. Never print a tokenized gateway URL. Only the explicit guarded access command may print a credential; boot cannot.
6. Do not show the large boot screen for noninteractive or machine invocations. REPL gets compact boot; TUI receives the same snapshot as a startup card/status header.
7. Start prepared background loops/watchers only after presentation. Advisor/extension/channel hot changes publish revision `N+1` and update consumers; they do not silently mutate a snapshot labeled final.
8. Clarify `runtime web` wording as full headless runtime/web ingress, not gateway-only.

Acceptance tests:

- golden snapshots for healthy, degraded, no-DB, no-channels, and non-TTY modes;
- scan all boot outputs for known token fixture and URL query credentials;
- final tool registration fixture count matches runtime registry count;
- startup ordering test proves boot occurs after final job/routine/static/dynamic identity reconciliation and before prepared loop execution;
- post-start hot activation produces a later revision without rewriting the captured startup facts;
- 80/100/120-column goldens meet the eight-line budget and do not duplicate/wrap credential content.

## CR-02.13 — Make build profiles and backend selection truthful

**Covers:** INV-44, INV-48, INV-70, INV-71, INV-72, INV-73, INV-82.

Implement:

1. Add a compile-time `CompiledCapabilitySet` in a low-level/root-neutral module. It reports enabled runtime features/backends/target facts from `cfg!` and classifies the normalized runtime set as `edge|light|desktop|full|minimal-libsql|minimal-postgres|all-features|custom`. Normalize out empty compatibility aliases and test-only `integration`/`schema-divergence` before classification. A named aggregate plus any additional real runtime feature is `custom`; the exact minimal and all-runtime-feature sets retain their supported identities. Keep the raw compile-time test facts internal and expose only the allowlisted runtime-feature set.
2. Replace unconditional `DatabaseBackend::default()` selection with:
   - libSQL only → `libsql`;
   - PostgreSQL only → `postgres`;
   - both → explicit configured backend; only in 0.17/0.18, a missing selection resolves to a typed `postgres` compatibility fallback plus human-stderr/config-status deprecation (never machine stdout). Setup/config writes the chosen backend explicitly. In 0.19, missing selection on a fresh/unmigrated dual-backend install is `configuration_invalid`; do not silently persist from a read path;
   - neither → retain the existing compile error requiring at least one backend.
   A requested uncompiled backend fails with feature/profile remediation and never falls back.
3. Decouple `src/cli/service.rs`, `Command::Service`, and the Windows SCM dispatcher from `#[cfg(feature = "repl")]`. Gate only on supported host OS where necessary; service management must exist in edge/light/minimal/desktop/full host builds. Preserve hidden Windows dispatcher naming and security.
4. Treat local REPL and TUI as supported host entry modes in edge/light/desktop/full because their channels compile unconditionally. Move compact boot presentation out of the `repl` gate along with service/SCM code. Remove `#![cfg(feature = "web-gateway")]` from `tests/host_runtime_smoke.rs` and run that smoke under the real always-available gateway/profile matrix; an empty compatibility alias must not alter test discovery. In 0.17, remove `repl`, `web-gateway`, and `timezones` from every aggregate; retain only empty deprecated declarations for downstream `--features` compatibility through 0.18, with removal documented for 0.19. No command, test target, module, dependency, generated metadata, or capability fact may depend on them. Add source/Cargo assertions that no `cfg` references any of those three feature names and that enabling each alias independently yields the same test discovery, canonical help, and static capability metadata as its base feature set.
5. Separate `dev browser` readiness (external Chrome/Chromium/Brave/Edge binary, bounded subprocess) from the Cargo `browser` capability (agent chromiumoxide backend). Likewise, tool/registry administration remains present without `wasm-runtime`, while local WASM execution reports `not_compiled`.
6. Ensure `voice`, `bedrock`, `bundled-wasm`, `nostr`, Docker, TLS/mDNS, ACP, tunnel, document extraction, desktop/local tools, and platform-native channels each have a descriptor/predicate; the temporary empty compatibility aliases and `integration`/`schema-divergence` test flags do not create user capabilities.

Acceptance tests:

- help/capability snapshots for every profile in the matrix document;
- build-profile classifier fixtures cover all seven named identities, custom real-feature deltas, and prove empty/test-only flags cannot alter the public fingerprint;
- fresh edge/desktop/minimal-libSQL selects libSQL; minimal-Postgres selects PostgreSQL; dual backend retains compatibility default; uncompiled explicit selection fails;
- dual-backend missing-selection fixtures warn/fall back only in 0.17/0.18, explicit migration persists through restart, and the 0.19 policy fixture fails without mutating configuration;
- `repl`, `web-gateway`, and `timezones` are absent from aggregate lists in 0.17, have no `cfg`/dependency/metadata effect while retained empty, and are release-note/test scheduled for declaration removal in 0.19;
- service parses on Linux/macOS edge/light/full and Windows supported profiles; Windows internal token remains hidden;
- browser/WASM installed-versus-executable distinctions are visible and actionable.

## CR-02.14 — Harden backup artifacts and restore coordination

**Covers:** INV-38, INV-74, INV-76, INV-80, INV-83.

Canonical surface:

```text
data backup export [--out FILE] [--no-database|--require-database] [--force]
                   [--passphrase-file FILE]
data backup import FILE [--dry-run] [--restore-database] [--yes]
                   [--passphrase-file FILE]
data backup inspect FILE [--passphrase-file FILE]
```

Rules:

1. Passphrase precedence is explicit private regular file → `THINCLAW_BACKUP_PASSPHRASE` → masked TTY prompt. Preserve/name the current 4-KiB `MAX_BACKUP_PASSPHRASE_BYTES`, trim one terminal newline from files, reject symlink/non-regular/empty input, and never log/debug/serialize it. Remove current `--passphrase` immediately; it is an unsafe process-argument/history channel and is not eligible for a hidden compatibility alias.
2. Export status always includes `workspace_included`, `database_requested`, `database_included`, `credential_values_included=false`, `secret_sources_included=false`, backend, section sizes/counts, artifact path/hash, and exclusions. A default DB failure may produce a deliberate partial bundle with a human stderr warning and typed `database_included=false`; `--require-database` aborts before publishing any final artifact. `--no-database` is intentional, not partial.
3. Refuse an existing `--out` unless `--force`; publish atomically and keep encrypted secret rows, source records/locations, OS credentials, `.env`, logs, live DB/WAL files, volatile captures, and experiment runner-auth envelopes excluded. Sanitize/remove `SecretBinding` fields from exported settings instead of restoring dangling IDs; include only field-name/count remediation metadata so import reports those consumers unconfigured. Stdout is not used for the sealed binary bundle: `--out` is required or the safe timestamp default is used.
4. `import` without `--yes` remains a dry-run; add explicit `--dry-run` for clarity and reject `--dry-run --yes`. Before mutation, acquire the same exclusive runtime lease as reset, validate the complete manifest/targets, and prove path containment. PostgreSQL remains an extracted/manual restore unless a separately reviewed transactional executor is implemented; report that state honestly.
5. PostgreSQL export removes password/userinfo from the connection URL and never sets or inherits `PGPASSWORD`. When a password is required, write one exact libpq-escaped line to a private temporary `PGPASSFILE` using the shared owner/mode/ACL-safe writer, pass only that path plus a sanitized explicit connection URL, and delete/zeroize the file after the bounded child exits or spawn fails. Before spawn, remove inherited `PGPASSWORD`, `PGPASSFILE`, `PGSERVICE`, and `PGSERVICEFILE`; connection/TLS environment is accepted only after the Phase-A schema resolves it deliberately. Escape `:` and `\\` per libpq rules; reject newline/NUL and unsupported secret-bearing query parameters. Child argv/environment/error fixtures may contain host/user/database and the private path, never the password. PostgreSQL manual restore guidance likewise contains placeholders/source-ID guidance only, not a credential URL or reusable shell-history value.

Acceptance tests cover partial/required DB behavior, no-final-artifact on failure, overwrite policy, passphrase leakage scans, corrupted bundles, libSQL lease/sidecars, PostgreSQL private-pgpass success/escape/boundary/spawn-failure cleanup and argv/environment sentinel scans, PostgreSQL manual outcome, traversal/symlink fixtures, and exact manifest metadata.

## CR-02.15 — Guard explicit gateway credential reveal

**Covers:** INV-06, INV-43, INV-61, INV-78, INV-83.

1. Canonical `runtime web access` prints credential-free origins, bind/port/auth state, health state, and tunnel guidance. Rename `--show-token` to `--reveal-token`; keep the former as a hidden alias through 0.18.
2. Reveal requires stdout to be a TTY plus explicit confirmation (`--yes` may skip the prompt but not the TTY requirement). Reject with `--output-format json|jsonl`, `--quiet`, redirected stdout, or unavailable auth. The reveal is never placed in a typed result, history, trace span, error, clipboard, boot snapshot, or TUI state.
3. Centralize redaction in `GatewayAccessInfo`/presentation types so tokenized URLs cannot be formatted accidentally. Preserve PID instance-token ownership and lifecycle-lock tests independently from bearer-token display.

Acceptance scans stdout/stderr/log/debug/boot/TUI/JSON fixtures with a known token and proves only the explicit confirmed TTY reveal contains it.

## CR-02.16 — Make model network/cost behavior opt-in and explicit

**Covers:** INV-24, INV-77, INV-83.

1. `config models verify` performs discovery/capability metadata requests only by default. Replace the inverted `--discovery-only` UX with explicit `--chat-probe`; the old flag is a hidden no-op compatibility alias through 0.18.
2. `--chat-probe` states that it can incur provider cost and requires a TTY confirmation or noninteractive `--yes`. It sends one bounded minimal prompt per selected provider/model, honors global/per-request timeouts, and reports request count without secret/header/body leakage.
3. `config models test MODEL` is already an explicit live action; its help must say remote/billable, it performs exactly one bounded minimal request, and noninteractive use is accepted as explicit intent. `sync` performs discovery/catalog ingestion only and uses `--out` for its artifact destination.
4. Doctor/status live LLM checks reuse non-billable discovery/readiness probes and never call chat generation.

Acceptance uses mock providers to assert zero chat requests by default, one per explicit probe/test, confirmation behavior, timeout/body bounds, and clean machine output.

## CR-02.17 — Make tool registration deterministic and provenance-complete

**Covers:** INV-82, INV-84, INV-85, INV-86. File 07 section 4 is the normative registry contract.

1. Replace `ToolRegistry`'s separate tool/built-in maps with entries containing name, `ToolOrigin`, source ID/digest, revision, registration time, and tool object. The exact origin vocabulary is the one in CR-02.10/file 07.
2. Generate static reservations/descriptors from one `StaticToolCatalog` containing all 124 audited static IDs and their compile/runtime predicates. Delete manual `PROTECTED_TOOL_NAMES` as an independent source; compatibility exports, if temporarily needed, are derived from the catalog.
3. Eliminate `register_sync`'s silent two-`try_write` path. Use an infallible startup builder or short synchronous metadata lock, then an async dynamic registry; every insert returns `RegistrationOutcome::{Inserted, Rebound, Unchanged, Rejected}` and callers propagate/log a typed result.
4. Reject name collisions by default, including dynamic/dynamic and built-in/built-in. Allow replacement only for an explicitly requested same-origin/same-source rebind or an authorized uninstall/install transaction. Include both safe origins/source IDs in conflicts.
5. Preflight and reserve every final name for an MCP activation before publishing any proxy; roll back the whole set on failure. Apply equivalent atomic name reservation to WASM, user-tool, and native-plugin activation. Never partially activate around a collision.
6. Remove the provisional job-tool registration and register the complete job group exactly once after the final scheduler/event store/prompt queue are constructed. Startup must not use a `Rebound` exception for jobs. Treat advisor add/remove and hot extension/channel changes as revisioned registry transactions.
7. `seal_startup()` sorts and compares live `(name, origin, source_id, revision)` identities to capability descriptors before boot. A mismatch is startup failure, not a warning or count adjustment.

Acceptance tests include forced lock contention/no omission, every collision pairing, explicit same-source rebind, concurrent register/unregister, complete MCP rollback, WASM/user/native conflicts, job/advisor lifecycle, all-124 catalog coverage, and per-profile final registry/snapshot identity equality.

## CR-02.18 — Add only the operator-worthy capability parity gaps

**Covers:** INV-30, INV-33, INV-35, INV-84, INV-94. File 07 is the per-tool disposition authority.

Extract domain ports; do not invoke agent-tool `execute` or fabricate a `JobContext`.

1. Normalize the existing extension lifecycle before adding a leaf:
   - introduce one neutral `ExtensionSelector { name, kind? }` and include `NativePlugin` in the portable kind enum/mapping/schema; `tool_list` can filter all four kinds, while unsupported native-plugin network installation returns a typed policy error;
   - make manager/port auth, activate, and remove consume the selector. A supplied kind must exist exactly; an omitted ambiguous kind returns `ambiguous_kind { matches }` without mutation;
   - replace JSON/string result plumbing with typed `ExtensionAuthResult`, `ExtensionActivationResult`, and `ExtensionRemovalResult` carrying kind, exact identities/effects, source, revision, auth/readiness, and safe error code. Activation branches on typed `AuthRequired`, never error substrings;
   - introduce runtime-owned `ExtensionAuthSessionStore` and `ExtensionAuthSession { id, selector, owner_principal, purpose, mode, state, expires_at, revision }` with a 128-bit random URL-safe ID, 10-minute TTL, one-use OAuth state plus S256 PKCE verifier, exact authenticated identity binding, and rate/audit controls. The verifier is zeroizing/non-serializing; the store is in-memory, restart-invalidating, and never persisted into conversation snapshots. `state` is the closed CAS-driven machine `pending → exchanging → bound|failed`, with `cancelled|expired` terminal from `pending`; terminal states are immutable and concurrent callback/complete/cancel can produce at most one binding transaction. Modes are `oauth_pkce`, `bind_existing_source`, or `local_secure_create`; the catalog declares supported modes and exact output slots. Session/read results contain authorization URL/state metadata and source configured/resolvable facts only, never verifier/code/token/source location;
   - delete `POST /api/chat/auth-token`, `AuthTokenRequest`, WebSocket `AuthToken`, and the pending-auth path that interprets the next free-form incoming message as a token. This includes both gateway/static-Web copies of `app.js`/styles, route registration and device-scope tests, WebSocket codecs, agent message interception, and persisted `PendingAuthMode::ManualToken` snapshots; no dormant renderer or compatibility decoder may retain a token field. Replace `/api/chat/auth-cancel` with session-ID cancellation on the extension-admin surface. Plain chat while auth is pending returns safe secure-input guidance and never calls `ExtensionManager::auth`. Replacement gateway/WS completion binds `{auth_session_id, secret_source_id}` only, is never grantable to a device token, and uses the authenticated extension-admin policy. Cancel uses session ID and exact owner. A remote/Web UI caller without OAuth or a pre-created authorized source returns `secure_input_unavailable` and local CLI remediation;
   - make local manual auth exact: `extensions tools auth NAME [--kind KIND] [--credential-source ID | --from-stdin | --from-env VAR | --from-file FILE]` (and the equivalent category-specific MCP/channel adapter). Source/creation selectors are mutually exclusive; absent selectors use a masked TTY only for a catalog-declared manual-token mode and fail in non-TTY. Creation delegates to CR-02.6 with exact `extension:<kind>:<source-id>:<slot>` purpose. No `--token`/`--value` flag exists;
   - OAuth authorization URL/PKCE state is created server-side; the bounded callback verifies one-use state/session/redirect origin, atomically claims `pending → exchanging`, exchanges the code without logging/serializing it, and stages every catalog-declared access/refresh slot through the shared encrypted source service. Source writes, read-back verification, binding publication, and `bound` transition are all-or-rollback; on failure, delete staged rows and revoke newly issued provider material where supported before returning a safe terminal error. Refresh later runs only in the owning adapter and atomically rotates the same source revision/binding. OAuth codes/tokens never enter browser query history beyond the provider callback protocol, ordinary API results, or frontend state. Poll/retry/cancel/expiry are idempotent and do not activate; restart invalidates unfinished sessions and cleans their zeroizing state;
   - migrate existing extension/MCP token rows only after read-back verification, source-record creation, and binding swap; ambiguous shared-auth names remain disabled with field-name-only remediation. Remove `tool_auth`'s implicit post-auth activation. Auth has one effect and returns `next_action=activate`; `tool_activate` may return the typed auth session because activation was explicitly requested, but retries only after that same authorized session completes;
   - make `tool_auth`, `tool_activate`, and `tool_remove` accept optional kind. Preserve the tool IDs, but native-plugin removal must report its exact current effect (`unloaded`, operator manifest/artifact retained) and never claim durable uninstall. Existing category-specific CLI auth/remove paths pass an explicit kind and converge on these services instead of retaining parallel policy;
   - add generic `extensions activate NAME [--kind mcp-server|wasm-tool|wasm-channel|native-plugin]` over that service. Activation is a **running-runtime/GatewayClient** operation because its registry/channel effects are process-owned. Extend `POST /api/extensions/{name}/activate` additively with an optional validated kind (and the desktop/local adapter identically), return a typed/versioned result containing kind, exact registered identities, revision, auth/readiness, and conflict, and preserve compatible legacy response fields during the API window. A stopped runtime fails precisely; the CLI must not construct a throwaway manager or imply “active on next start.” Do not add a fictional generic deactivate command; the audited manager has no durable symmetric operation.
2. Add `data memory delete PATH [--dry-run] [--yes]` through the same memory policy/host service as `memory_delete`, including protected identity paths, principal/workspace containment, index cleanup, and bootstrap-completion semantics. Dry-run resolves the exact target; mutation requires confirmation.
3. Keep `prompt_manage`, `skill_manage`, external-memory recall/export, outbound `send_message`, Nostr actions, subagents, sensors, desktop autonomy, and execution primitives agent-only. `extensions skills` remains the safe operator skill lifecycle instead of exposing generic skill file mutation. Root `send` remains inbound prompt injection and its help explicitly says it is not outbound `send_message`.

Acceptance proves CLI/agent/services/API/desktop adapters return the same identities/policy outcomes; all-four-kind list/filter and selector behavior; auth-session owner/kind/purpose/TTL/replay/rate behavior; state-machine CAS races, restart invalidation, zeroizing cleanup, OAuth PKCE callback/source-write rollback/refresh rotation, and local masked/stdin/env/file/source-ID matrices; no raw token HTTP/WebSocket/chat/argv/DTO/history path; auth has no activation side effect; activation uses typed auth sessions; cross-kind auth/activate/remove ambiguity is non-mutating; legacy token migration is verified; native removal reports retained artifacts; a real running runtime observes activation and revision `N+1`, while stopped-runtime CLI activation is nonzero and side-effect-free; protected memory deletion/dry-run parity; mutation confirmation; persistence across processes; and absence of any additional raw execution CLI.

## CR-02.19 — Reconcile the complete learning administration surface

**Covers:** INV-83, INV-84, INV-91. This task covers both the five learning/external-memory agent admin tools selected in file 07 and the broader existing `/api/learning/*` operator surface.

Current facts that the implementation must not lose:

- gateway routes already expose status, event/evaluation history, candidates, artifact versions, feedback list/submit, provider health, code-proposal list/review, outcome list/detail/review/evaluate-now, and rollback list/record;
- list responses can say `has_more` but accept no cursor, so a caller cannot retrieve the next page;
- status/provider health performs live HTTP probes; it cannot serve as the static default;
- code-proposal approval can write a bundle and publish/promote through Git, while outcome evaluation invokes an LLM;
- the rollback POST records a learning-ledger event—it does not restore an artifact;
- `external_memory_setup` accepts inline `api_key` and arbitrary stringified config that can be serialized into settings.

Create one `LearningAdminService` and typed request/result DTOs used by CLI, gateway/API, desktop, and the relevant agent-tool adapters. Do not call an agent tool or fabricate `JobContext`.

Canonical CLI:

```text
data learning status [--recent N] [--live]
data learning history events|evaluations|candidates|artifact-versions|feedback|rollbacks|proposals|all
                      [kind-specific filters] [--limit N] [--cursor TOKEN]
data learning outcomes list [--status S] [--contract-type T] [--source-kind K]
                            [--thread ID] [--limit N] [--cursor TOKEN]
data learning outcomes show ID
data learning outcomes review ID --decision confirm|dismiss|requeue
                              [--verdict positive|neutral|negative] [--yes]
data learning outcomes evaluate-now [--yes]
data learning feedback submit TARGET_TYPE TARGET_ID --verdict VERDICT
                              [--note TEXT] [--metadata-file FILE]
data learning proposals list [--status S] [--limit N] [--cursor TOKEN]
data learning proposals show ID
data learning proposals review ID --decision approve|reject [--note TEXT]
                               [--dry-run] [--yes]
data learning rollbacks record ARTIFACT_TYPE ARTIFACT_NAME --reason TEXT
                               [--version ID] [--metadata-file FILE] [--yes]
data learning external-memory status [--live]
data learning external-memory configure PROVIDER [provider-typed options] [--enabled BOOL]
                                    [--credential-source ID]
                                    [--embedding-credential-source ID]
                                    [--secret-header-source HEADER=ID]
data learning external-memory activate PROVIDER [--yes]
data learning external-memory deactivate [--yes]
```

The provider schema catalog is normative, not a loose `HashMap` replacement:

| CLI provider / stored key | Required or default configuration | Allowed provider-specific non-secret fields | Operational disposition |
|---|---|---|---|
| `honcho` / `honcho` | `base_url` required; API credential optional according to deployment | `cadence`, `depth`, `user_modeling_enabled` | Recall/export/user-modeling supported with strict subject scope. Endpoints remain fixed. |
| `zep` / `zep` | `base_url` required; API credential optional according to deployment | none beyond common enable/auth fields | Recall/export supported with strict subject scope. Endpoints remain fixed. |
| `mem0` / `mem0` | Cloud base defaults to `https://api.mem0.ai` and requires an API credential; self-hosted base may omit it | provider user prefix, `agent_id`, bounded same-origin search/sync paths, `rerank`, `export_role`, `user_modeling_enabled` | Recall/export/user-modeling supported with strict subject scope. |
| `openmemory` / `openmemory` | Base defaults to `http://localhost:8888`; credential optional | provider user prefix, `agent_id`, bounded same-origin search/sync paths, `export_role` | Recall/export supported with strict subject scope. |
| `letta` / `letta` | Cloud base defaults to `https://api.letta.com`; `agent_id` required; cloud API credential required | bounded same-origin search/sync paths and bounded tags | **Configuration and health only in this refinement.** Activation returns `policy_denied` because the audited adapter ignores ThinClaw's subject ID and `supports_strict_subject_scoping()` is false. Do not advertise recall/export until a separately tested per-subject adapter exists. |
| `chroma` / `chroma` | Base defaults to `http://localhost:8000`; `collection_id` and `embedding_url` required | tenant, database, bounded same-origin query/sync paths, embedding model/shape and embedding credential source | Recall/export supported with strict subject scope; provider and embedding credentials are distinct references. |
| `qdrant` / `qdrant` | Base defaults to `http://localhost:6333`; `collection` and `embedding_url` required | bounded same-origin query/sync paths, embedding model/shape and embedding credential source | Recall/export supported with strict subject scope; provider and embedding credentials are distinct references. |
| `custom-http` / `custom_http` | Either `base_url` or both explicit recall/sync URLs; API credential optional | bounded recall/sync URLs; bounded non-secret headers; separately referenced secret headers | Recall/export supported only through the existing strict subject-bearing request contract. Unknown response shapes remain typed provider errors. |

Common URL fields are bounded absolute HTTP(S) URLs without userinfo/fragments. Relative/path overrides must stay on the configured origin and interpolate only allowlisted non-secret identifiers. IDs/prefixes/tags/model names are bounded and reject control characters. Provider options that the adapter does not consume are usage errors, not inert persisted settings. `url` remains a read-only migration alias for `base_url`; new writes use only canonical keys.

Rules:

1. `history` kind values map exactly to the existing event/evaluation/candidate/version/feedback/rollback/proposal records. Kind-specific filters are rejected on incompatible kinds rather than ignored: actor/channel/thread for events, candidate type/risk tier for candidates, artifact type/name for versions and rollbacks, target type/ID for feedback, and status for proposals. `all` is a bounded dashboard returning a `next_cursor` per kind; it rejects a single input cursor.
2. All single-kind lists and outcomes/proposals use backend-neutral keyset pagination ordered by `(created_at DESC, id DESC)`, default `50`, maximum `200`, with an opaque versioned cursor bound to principal, query kind, and filters. Extend libSQL/PostgreSQL traits and the gateway query/response/OpenAPI types. `has_more` without a usable continuation is removed only after all consumers migrate.
3. Static `status` reads settings, evaluator metadata, counts, and last stored probe only. Configure-only external-memory writes also perform zero network requests and return the persisted configured/enabled/active facts rather than calling `provider_health`. `--live` and `external-memory status --live` schedule at most eight provider probes concurrently, with a named 5-second per-provider timeout and 10-second aggregate deadline. Unfinished probes become typed `timeout` results; unconfigured providers are `not_configured`, unsupported probes are `not_supported`, and scope-unsafe providers use `policy_denied`. No remote request occurs in static mode.
4. Route by state ownership:
   - direct durable service: static reads, history/outcome/proposal detail, feedback submit, outcome review, rollback record, and external-provider configure-only;
   - running runtime through `GatewayClient`: outcome `evaluate-now`, proposal approval/publish, external-provider activation, and shutdown of a live provider. If a runtime is active, use its service for all overlapping mutations to avoid split-brain cache/state; if stopped, only the explicitly direct operations above are available.
5. Proposal review first returns an effect plan containing target files, bundle directory, current publish mode, branch/remote intent, validation results, and rollback note. Reject is a durable/audited mutation. Approve defaults to dry-run for noninteractive invocation and requires `--yes` to write/publish; it uses workspace containment, the runtime-operation lease, and returns bundle/branch/PR plus partial/failure state. Never call a failed bundle/publish “applied.”
6. Outcome review validates the exact decision/verdict matrix (`confirm` requires one of `positive|neutral|negative`; `dismiss|requeue` reject `--verdict`), is principal-scoped/audited, and requires confirmation. `evaluate-now` is a bounded, potentially billable LLM action through the runtime; disclose model/request bound and require TTY confirmation or `--yes`.
7. `rollbacks record` is deliberately named `record`: it appends an observation and outcome hook only. Help/result contains `artifact_restored=false`. It validates an optional version belongs to the named artifact/principal, accepts only a non-symlink regular JSON-object metadata file capped at 1 MiB, and requires confirmation because the record can affect learning outcomes. Do not ship `rollbacks apply` until a real restoration service exists.
8. `feedback submit` accepts an optional non-symlink regular JSON-object metadata file capped at 1 MiB; target type/verdict use shared typed vocabularies or a documented extensible-value validator. It never treats arbitrary metadata as a settings or secret channel.
9. Implement the exact provider table above as typed catalog metadata shared by CLI, agent schema, gateway/OpenAPI, desktop, settings validation, runtime status, and generated docs. Unknown fields, unsupported/inert options, and secret-shaped keys are rejected; the generic arbitrary `config` object is not a secret escape hatch. Preserve the configured/enabled/active/healthy/scope-safe facts independently.
10. Reuse the `SecretSourceId`/record/binding service and private-secret reader introduced by CR-02.6; do not add a provider-local lookalike. Inject one authorization-aware value resolver into provider adapters and remove direct `config.api_key`/`std::env::var` reads from provider request helpers. Sources are created separately by local `config secrets` or interactive setup. CLI, gateway, desktop, and agent provider-mutation DTOs accept only opaque IDs; the runtime resolves/revalidates owner, revision, and the exact `external-memory:<provider>:<slot>` purpose. They cannot submit a host environment-variable name, filesystem path, encrypted-store name, or OS account. Responses expose only source ID/kind/configured/resolvable status unless a local authorized diagnostic explicitly requests redacted location metadata. Re-check private-file ownership/type/ACL/permissions at use; values are bounded/zeroized and never cached in snapshots.
11. Remove inline `api_key`, `embedding_api_key`, and arbitrary credential-bearing headers/config from agent, CLI, gateway, OpenAPI, and desktop DTOs immediately. Primary, embedding, and allowlisted custom secret-header credentials use distinct `SecretBinding` slots; source reuse across slots is permitted only when the record explicitly carries every exact purpose. Migrate every secret-like legacy map entry (including API/embedding keys, authorization/cookie/token headers, URL userinfo or token query values, and unknown key/token/password/secret-shaped fields) by encrypted/OS-store write→retrieve verification, source-record creation, and binding swap before transactional scrub. If a value cannot be mapped safely, leave the legacy source intact, disable activation, and report only the field name/remediation. Sentinel values must not enter arguments, settings maps after migration, source records, API DTOs, traces, errors, or snapshots.
12. `external-memory configure` is durable and configure-only; it never probes, activates, deactivates, recalls, or exports. Omitting `--enabled` preserves an existing provider's value and defaults a newly configured provider to `true`. An exact idempotent rewrite is allowed, but any material update to the active provider—including `--enabled false`—returns `active_provider_conflict` and directs the operator through `deactivate → configure → activate`; configuration never hides a live rebind or shutdown. `activate PROVIDER` is a distinct confirmed running-runtime/GatewayClient action that requires configured+enabled+scope-safe state and returns the selected provider plus snapshot revision. Re-activating the same live provider is an idempotent typed success; activating a different provider while one is active returns `active_provider_conflict` and does not switch implicitly. `deactivate` truthfully clears only the active selection, shuts down the runtime-owned provider when present, and preserves configuration/enabled state; when the runtime is stopped it may clear a stale persisted selection and reports `live_shutdown=not_running`. The existing agent tool ID `external_memory_setup` becomes a thin typed adapter to configure/explicit activate operations, while `external_memory_off` maps to deactivate. No path silently exports or recalls memory.
13. Every read/mutation derives its effective principal and actor from the authenticated gateway identity or local `CliContext`; the learning CLI in this release has no caller-controlled `--principal`/`--user-id` selector. Cross-principal gateway administration is permitted only through the existing authenticated admin-override policy and its audit path. A caller-controlled body/query `user_id` is never sufficient. Result schemas are versioned and preserve safe source/audit IDs.
14. `prompt_manage`, `skill_manage`, external-memory recall/export remain agent-only. The CLI is an administration/ledger surface, not a way to inject learning payloads or bypass contextual evidence and tool approval.

Acceptance includes exact CLI↔service↔gateway↔desktop↔agent result parity; libSQL/PostgreSQL cursor traversal with no gaps/duplicates; invalid/filter-bound cursor rejection; zero network in static status and configure-only operations; bounded eight-provider live timing; proposal dry-run/containment/publish partiality; billable evaluation gating; outcome decision matrix; truthful rollback recording; exact accepted/rejected option fixtures for all eight providers; Letta activation denial until strict scoping exists; configured/enabled/active state transitions; distinct primary/embedding/header secret resolution; exhaustive legacy secret-map migration; principal/actor isolation; runtime stopped/running routing; and two-process persistence.

## CR-02.20 — Seal every subprocess and local-sidecar boundary

**Covers:** INV-08, INV-28, INV-48, INV-50, INV-76, INV-83, INV-92.

**Depends on:** CR-01.7/CR-01.8, CR-02.6, and CR-02.13. Execute this task immediately after those foundations and before setup Apply or any later process-owning refactor.

Implement:

1. Add one typed `ProcessLaunchDescriptor` registry and check in its generated `tests/fixtures/process_launch_manifest.json`. Define `ProcessLaunchId`, `ProcessClass`, `ChildEnvironmentPolicy`, `ExecutablePolicy::{AbsolutePinned, ValidatedSearchPath}`, `HomePolicy::{LauncherPrivate, WorkspaceScoped, ReviewedOperatorHome}`, `TempPolicy::{LauncherPrivate, ReviewedSystemTemp}`, `FilesystemPolicy`, `NetworkPolicy`, `IsolationPolicy::{HostUnconfined, ReviewedDirectHost, WorkspaceSandbox, Container, DedicatedUser}`, `ProcessIoPolicy`, `ProcessLifetimePolicy`, and `CredentialSlot`. Each stable launch ID records owner/source symbol, executable locator and digest/search policy, argument schema, cwd/filesystem roots, exact environment additions, home/temp policy, credential slots/sinks, network/isolation, stdin/stdout/stderr caps, total timeout, process-tree ownership, shutdown/reap/cleanup, availability predicates, and proof ID. Descriptor metadata is non-secret and deterministic across a given build profile.
2. Route every production `std::process::Command`, `tokio::process::Command`, Tauri sidecar, and shell-wrapper launch through `thinclaw-platform::ProcessLauncher` or a thin desktop adapter enforcing the same descriptor. The launcher begins with `env_clear`; constructs private home/temp rather than inheriting them for untrusted classes; applies only validated descriptor fields; validates executable/search path/cwd/filesystem arguments; captures bounded redacted diagnostics; enforces one aggregate deadline; and owns/reaps the descendant tree on success, cancellation, timeout, and caller drop. Private home/temp directories use owner-only permissions/ACLs, reject symlink/reparse traversal, and are removed by the same exactly-once cleanup path. A secret-typed value cannot implement the ordinary argument/display conversion. Shell use requires an explicit fixed-script descriptor and can never receive a resolved secret or user-built command string.
3. There is no universal inherited baseline. The minimum platform substrate contains only required OS process-start fields (for example Windows `SystemRoot`/`WINDIR`/`ComSpec` where necessary) plus explicitly selected non-secret locale/terminal identity. Real `HOME`/`USERPROFILE`, `TMP*`, parent `PATH`/`PATHEXT`, toolchain roots, dynamic-loader paths, service-manager session fields, proxy/auth-agent fields, and network controls are separately named, validated, and justified per descriptor. Search paths contain bounded canonical absolute directories, reject empty/current-directory segments and writable-untrusted locations unless the declared sandbox owns them, and executable identity is rechecked at spawn. Wildcard env prefixes and arbitrary parent pass-through are invalid. Consumer-specific credentials are resolved only inside the launcher after owner/exact-purpose authorization and ambient clearing. Built-in shell/process/execute-code receive no general credential set. Their filesystem/network reach is governed by `IsolationPolicy`; a requested hardened policy that is unavailable fails `isolation_unavailable`. Explicitly approved arbitrary `HostUnconfined` remains visibly high risk and cannot receive a secret slot. `ReviewedDirectHost` is limited to a fixed executable/argument schema such as a database exporter or provider daemon, may receive only its one declared private sink/slot, and is reported as direct host execution rather than sandboxing. Runtime/service/gateway/dedicated-user re-exec receives the selected config/home/mode controls plus only exact Phase-A source variables required by the resolved bootstrap descriptor; it never copies the parent environment or embeds credential URL userinfo.
4. Make the launch catalog exhaustive across: no-secret OS/media/browser/service/install/probe/Git/toolchain utilities; job/sandbox/agent execution; MCP/extensions/channels/tunnels/coding workers; runtime/service/gateway/dedicated-user re-exec; database export; experiment runners; and desktop llama.cpp/vLLM/MLX chat, embedding, summarizer, and STT sidecars. `scripts/ci/check-process-launches.py` rejects a production raw constructor outside the platform implementation, a missing/stale descriptor, an undeclared env key, a sensitive argument, or manifest drift. Tests/build scripts have a separate explicit scope and cannot become a production allowlist. Migrate and lock the scanner class-by-class in this order: platform/shell execution → workers/jobs → runtime/service/setup → channels/tunnels/MCP/extensions → media/browser/probes → backup/experiments → desktop sidecars.
5. Keep specialized transports with their owning tasks but enforce them through descriptors: CR-01.8 owns experiment runner auth; CR-02.14 owns private `PGPASSFILE`; CR-01.8 removes the gateway instance-token env writes. All initial/resume/reissue/reload/retry paths use the same descriptor and auth sink. A compatibility alias may forward to a migrated launcher but cannot preserve an unsafe process path.
6. For desktop local inference, define `EphemeralSidecarAuth`, an opaque `LocalEndpointId`, and `SidecarAuthSink::{InheritedPipe, PrivateFile}`. The secret type is non-`Clone`, non-`Serialize`, zeroizing, and redacted in `Debug`; a backend-only `ManagedLocalEndpointRegistry` owns it, and only the in-memory HTTP client can borrow it for an authenticated loopback request. Endpoint/process structs exchange the opaque ID rather than token strings. llama.cpp uses a pinned private key-file interface and receives only the file path. If the current artifact lacks it, update version, hashes, provenance, and every platform packaging fixture to the first audited supporting build before enabling that engine. ThinClaw-owned vLLM and MLX Python launch adapters read an inherited pipe/file before constructing the server in-process. They never exec or reformat a token into argv/env. A platform lacking the audited artifact/adapter reports `auth_transport_unsupported` and does not weaken auth.
7. Auth files use the shared owner-private, symlink/reparse/hard-link-safe atomic writer. Remove them immediately after the backend's pinned contract proves the credential was read; always clean them on spawn/readiness failure, cancellation, timeout, or process exit. Anonymous pipes/handles are non-inheritable except for the intended child endpoint and close in both processes after transfer. Treat readiness publication and secret-artifact cleanup as one failure-aware transaction so no half-started endpoint is advertised.
8. Delete `token`/`api_key` from `ChatServerConfig`, `EngineStartResult`, sidecar events/status, Specta/generated TypeScript, frontend state, OpenAPI, and runtime contracts rather than serializing an empty placeholder. Replace managed STT's `inject_bridge_vars`/`WHISPER_HTTP_TOKEN` path with dependency injection from `ManagedLocalEndpointRegistry`; starting/stopping a managed endpoint leaves the process-global environment byte-identical. Explicit operator-configured remote STT credentials remain ordinary purpose-bound sources and are not conflated with this ephemeral handle. Internal endpoint state is not a general DTO. Regenerate desktop contracts and run every engine-profile gate in the same checkpoint. Logs/errors/process snapshots may contain only endpoint/launch ID, safe backend, PID, and redacted diagnostic fields.

Acceptance:

- the generated process manifest covers every production launch exactly once for each supported profile, and the raw-constructor/env-key/sensitive-argument scanner has no unexplained exception;
- Unix/macOS/Windows capture fixtures for every launch class prove unrelated ambient credential sentinels are absent, untrusted work receives private home/temp and validated executable resolution, cwd/filesystem/network/isolation policies are enforced or fail closed, exact authorized slots work only in eligible classes, and argv/cwd/output/error data contain no credential;
- for launcher-private/sandbox/container/dedicated/direct policies, adversarial parent `HOME`/`USERPROFILE`, `TMP*`, `PATH`/`PATHEXT`, credential-helper files, current-directory path entries, symlinked temp/home, and writable executable shadowing do not cross the declared boundary. `HostUnconfined` cannot promise filesystem containment; its fixtures instead prove approval gating, zero secret-slot injection, and an explicit non-isolated label rather than passing sandbox assertions;
- timeout, output-cap, cancellation, spawn failure, readiness failure, caller drop, and normal exit all reap the complete process tree and run cleanup exactly once;
- runtime/service/gateway/dedicated-user re-exec works with OS/env/private-file Phase-A sources and never inherits an unrelated sentinel or parent-only correlation nonce;
- llama.cpp/vLLM/MLX chat, embedding, summarizer, and STT fixtures prove private auth delivery, descriptor/handle permission rules, no token argv/child-or-global-environment/DTO/schema field, authenticated readiness/use, byte-identical process environment across managed start/stop, and cleanup at every failure boundary;
- a backend missing its pinned auth mechanism is unavailable with typed remediation; no feature/profile/platform or compatibility path falls back to `--api-key`, a token env variable, shell text, or an empty serialized token field;
- process descriptors and safe policy identities appear in diagnostic capability output, while secret slots/values and private artifact paths do not.

## CR-02.21 — Consolidate desktop runtime assembly and remove its secret overlay

**Covers:** INV-47, INV-54, INV-55, INV-83, INV-90, INV-92, INV-93.

**Depends on:** CR-02.1, CR-02.6/CR-02.7, and CR-02.20. Complete it before CR-01.10 consumes the desktop/shared runtime constructors.

Implement:

1. Prove the compiled module graph first. The active path is `apps/desktop/backend/src/thinclaw/runtime_builder.rs`; the tracked `runtime_builder/environment.rs`, `background_tasks.rs`, `event_forwarders.rs`, and `sandbox.rs` are currently not declared and therefore cannot affect behavior. Compare each extracted function with its active duplicate and record the mapping in a temporary test fixture before editing behavior.
2. Finish the existing extraction rather than maintaining two sources: declare the four submodules, route `build_inner` through their typed functions, move the remaining shared types/constants to the parent, and delete each corresponding duplicate block from `runtime_builder.rs` as soon as its module compiles/tests. If an extracted block is stale and cannot be proven equivalent, keep the active implementation and delete that orphan file instead; never merge two variants by guesswork. End with one compiled definition per behavior and focused files small enough to review independently.
3. Replace the `HashMap<String, String>` `bridge_config`/`BRIDGE_VARS` assembly with typed `DesktopRuntimeInputs` containing only non-secret settings/overrides plus opaque credential handles. The non-secret type has explicit fields for DB location/backend, heartbeat, workspace/tool policy, model/provider selection, base endpoints, gateway bind/port, and local endpoint IDs. Unknown env-like keys are impossible. Existing external environment values enter through the CR-02.6 resolver with provenance, not a second desktop precedence system.
4. Do not materialize resolved credentials in that input object. Inject the shared authorization-aware `SecretResolver` into each owning service and retain only exact-purpose bindings/opaque handles: Gmail access/refresh under `integration:gmail:*`, gateway bearer under `gateway:*`, provider keys under `llm:*`/`embedding:*`, MCP/channel/media/coding-worker slots under their catalog purposes, and operator-configured remote STT under its media/integration slot. Resolve at use into zeroizing values with revision/owner checks. CR-02.20's `ManagedLocalEndpointRegistry` owns generated local sidecar auth separately.
5. Remove every secret insertion/read through `inject_bridge_vars`, `optional_env`, generic runtime settings maps, or raw desktop config clones. `BRIDGE_VARS` may remain only as a temporary non-secret compatibility adapter through 0.18, backed by an allowlisted typed projection; reject secret-shaped keys at compile/schema boundaries and delete the adapter in 0.19. Never log the projection's values wholesale. Migrate legacy desktop keychain values with the verified source-record/binding transaction from CR-02.6 before scrubbing their old names.
6. Make `build_inner(DesktopRuntimeInputs, SharedServices, ProcessLauncher, ManagedLocalEndpointRegistry)` the sole local assembly entry. Remote-profile routes continue to use their typed gateway proxy and preserve the `LocalOnly` child-session gate. The builder constructs no live provider twice, starts no probe while translating inputs, and emits the capability snapshot only after all final services/tools/channels are registered.
7. Replace source-string tests such as `include_str!("runtime_builder.rs")` field matching with compile-time typed constructors and behavior fixtures. Add `scripts/ci/check-desktop-runtime-builder.py` to fail when a tracked Rust file below `thinclaw/runtime_builder/` is not in the module graph, when an assembly symbol has multiple definitions, or when secret field names are inserted into generic bridge/env maps. Run Rustfmt, backend Clippy/tests, generated Specta contract drift, frontend TypeScript/tests/build, and every engine profile after the extraction.

Acceptance:

- module-graph evidence shows all retained runtime-builder files compiled and every removed duplicate/orphan absent; one behavior definition and one test owner exist for environment inputs, sandbox, event forwarding, and background tasks;
- Gmail, gateway, LLM/embedding, MCP, channel, media/STT, coding-worker, and local-sidecar sentinels never appear in `BRIDGE_VARS`, `optional_env` results for injected desktop state, `DesktopRuntimeInputs`, settings, DTOs, logs, or snapshots; each exact owning service can resolve only its authorized handle;
- legacy desktop secret migration is write→read verified and transactional; an ambiguous/failing migration keeps the old source, disables only the consumer, and emits field-name-only remediation;
- local start/restart builds the same final service/tool/channel identities and revision through the modular path; remote profile fixtures retain no embedded-runtime mutation or credential fallback;
- configuration translation has zero network/process side effects, and final assembly starts each live resource once with failure-aware cleanup;
- the module-topology/generic-secret-map checker, desktop generated-contract gate, all backend engine profiles, and frontend/backend suites pass.

## CR-02 definition of done

- [ ] Agents persist across processes through `AgentRegistry`.
- [ ] Every mutating leaf has one D-54 execution policy; active/stopped routing, durable/live/external/process identities, restart requirements, partial rollback, and no-direct-fallback behavior are proven.
- [ ] Public conversations operate on durable messages; in-memory sessions are not exposed as durable.
- [ ] Manual routine trigger creates and returns a real run ID.
- [ ] Job APIs have a safe regular CLI.
- [ ] Config resolution, encrypted-locator secret-source handling, semantic classification of every sensitive-field candidate, checked credential-consumer/process-launch coverage, ID-only durable bindings, and private ephemeral/protocol auth transports are single-source and tested.
- [ ] Channel static/live semantics are separated.
- [ ] Memory search preserves provenance.
- [ ] Skills administration reuses the runtime service.
- [ ] One multidimensional registry-derived capability snapshot drives status, doctor, boot, and later TUI state.
- [ ] Build profiles/backend defaults/service gates match the supported matrix.
- [ ] Backup, token reveal, and model probes follow their artifact/secret/cost contracts.
- [ ] Tool registration has no silent omission/overwrite and final identities match the exact catalog/live registry.
- [ ] Extension authentication has no raw HTTP/WebSocket/chat/desktop-token path or implicit activation; activation and memory deletion share domain services; all other tool parity decisions match file 07.
- [ ] Complete learning ledger/admin API and selected agent tools converge on `LearningAdminService`, with cursors, truthful effects, runtime ownership, and secret-reference-only provider setup.
- [ ] Required doctor/live-status failures exit `3`; usage errors remain `2`; operational errors exit `1`.
- [ ] Targeted, backend-parity, JSON, and two-process tests pass.
