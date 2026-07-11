# Best Practices & Pitfalls — Global Engineering Guardrails (CC-A)

> **Workstream:** CC-A (cross-cutting) · **Status:** authoritative for the whole remediation effort.
> Every other workstream doc (`WS-*` / numbered `0N-*.md`) inherits these rules. When a task in another
> workstream conflicts with a rule here, this doc wins unless the task explicitly justifies the exception in writing.
>
> This is a **guardrails** doc, not a task list. It has no `cargo` tasks of its own beyond the Quality Gate.
> Its job is to keep future work from re-introducing the exact failure modes the
> 2026-06-23 audit (`AUDIT-FINDINGS.md`) found.
>
> **Note on the audit findings cited below:** every specific bug this doc references (empty-token
> bypass, FTS5 divergence, `split_message` panic, Discord verification, self-repair `with_builder`,
> WASM table/instance limits, the `cargo deny` advisory, CI clippy `--all-targets`, the god-files)
> has since been **RESOLVED** by the remediation stack (merged to `main`). Those citations are kept
> as the *rationale* for each rule (the lesson), not a live defect. Where a claim was written in the
> present tense ("currently red", "stubbed", "never injected"), it has been corrected below to say
> what shipped. The **rules themselves remain in force.** File:line anchors were originally verified
> on 2026-06-23 (`main` @ `9e707985`); the load-bearing ones have been re-pointed to the current
> tree, but some illustrative anchors still name a file that has since been decomposed into a
> directory module or deleted, and are kept as historical references.

---

## How To Use This Doc

1. Before writing any remediation change, skim the section that matches the area you touch.
2. Every PR description must state **which build profiles it touches** (see Feature-Flag Discipline) and
   **which DB backends it touches** (see Dual DB Backend Parity).
3. Run the full Quality Gate before opening a PR; run `/code-review high` before requesting merge.
4. If you change behavior, update the canonical doc in the **same PR** (see Doc-Update Triggers).

---

## Engineering Principles

These are the four load-bearing ideas from `CLAUDE.md` → *Core Design Ideas* and `docs/CRATE_OWNERSHIP.md`.
Remediation must **realize** these, not erode them.

- **Control over convenience.** The operator chooses where ThinClaw runs, which models it uses, and which
  integrations are trusted. Do not add a "just works" default that silently widens trust. The canonical
  anti-example is finding #1: an empty `gateway_auth_token` became `Some("")` and silently disabled all
  `/api` auth — convenience (don't force a token) defeated control (the auth boundary). Prefer
  fail-closed defaults (the sandbox allowlist's "empty = deny all", `src/sandbox/proxy/allowlist.rs`) over
  fail-open ones (the shell scanner's documented fail-open default, `AUDIT-FINDINGS.md §8`).

- **Security as architecture.** Safety is split across sandboxing, tool policy, secret injection, network
  controls, and trust-boundary decisions — not bolted on. `src/NETWORK_SECURITY.md` is the authoritative
  per-listener reference; treat its *Review Checklist for Network Changes* as mandatory for any
  network-facing PR. A guarantee that is documented but never fires (findings #6/#7: HTTPS credential
  injection that no shipped default mapping can trigger) is worse than no guarantee — it lies to the next
  reviewer. Either wire it so it fires, or delete it and correct the doc.

- **Ports & adapters (hexagonal).** This is the spine of the crate split. Reusable policy/algorithms/DTOs
  and **port traits** live in `thinclaw-*` crates; concrete host side-effects (DB, LLM, tool, channel,
  Docker, filesystem) live in root `src/*` as adapters. Cross a root/crate boundary **only through a narrow
  port trait**, never by pulling a concrete root service into a crate. Examples to copy: `thinclaw-agent`
  self-repair behind context/store/**builder** ports; `thinclaw-tools` shell runtime behind sandbox/ACP/
  smart-approval ports; `thinclaw-db` backends behind persistence traits. When you "wire" a built-but-dead
  capability (the operator's REALIZE-THE-VISION directive), wire it by **injecting the existing adapter into
  the existing port** — e.g. the once-dead self-repair rebuild path was exactly a missing `with_builder`
  injection, now shipped at `src/agent/agent_loop/mod.rs:672`
  (`repair = repair.with_builder(builder, self.deps.tools.clone())`), not a missing feature.

- **Hybrid extensibility.** Native Rust where persistent connections / local system access matter; WASM
  where hot reload + credential isolation matter; MCP for external tool ecosystems. Do not collapse these
  into one mechanism. MCP servers are **operator-trusted external processes** (no SSRF guard by design,
  `src/tools/mcp/client.rs`); WASM guests are **untrusted sandboxed code** (allowlist + fuel/epoch/memory
  limits). Never apply WASM-tier sandboxing assumptions to MCP, or MCP-tier trust to WASM.

---

## Architecture Hygiene Rules

Verbatim intent from `CLAUDE.md` → *Architecture Hygiene*. **God files are architectural debt, not style.**

- **Façade `mod.rs`.** A `mod.rs` declares submodules, re-exports the stable public API with `pub use`, and
  holds only narrowly-shared glue. It must not accrete behavior. Good shape: `thinclaw-tools/src/wasm/`
  splits into `loader.rs`, `error.rs`, `oauth.rs`, `credential_injector.rs`, `storage.rs`, `limits.rs`.
- **`pub use` re-exports preserve public paths.** When you decompose a module, keep `thinclaw::agent`,
  `thinclaw::tools`, `thinclaw::db`, etc. stable via `pub use` (root rule in `CRATE_OWNERSHIP.md §Rule Of
  Thumb`). Callers must not have to change imports because you split a file.
- **Narrow visibility.** Keep internal cross-module visibility at `pub(super)` or `pub(in crate::...)`.
  **Do not widen an API to `pub` just to make a split compile** — that is a documented audit pitfall
  (see Common Pitfalls Catalog). If a split needs wider visibility, that is a design smell; re-cut the
  boundary instead.
- **Add behavior to the narrowest owning submodule.** Do not grow façades, coordinators, or catch-all
  helpers because they are convenient. Repeated unrelated edits to one file = signal to extract first.
- **No vague buckets.** No new `misc`/`common`/`utils` modules unless genuinely cross-cutting and small.
  Name modules for the domain they own.
- **Block new god-file growth in review.** A PR that adds substantial unrelated behavior to an
  already-broad file must split the module first or justify why it cannot be separated safely yet.

The god-files the audit called out (`AUDIT-FINDINGS.md §5`) have all been decomposed into directory
modules (WS-10). **No committed `.rs` file now exceeds 2,000 lines anywhere in the repo**, and a CI guard
(`scripts/ci/check-file-sizes.sh`, `MAX_LINES=2000`, run at `ci.yml:64`) keeps it that way. The rule still
stands: **do not grow a module back toward that limit.** The largest surviving modules to keep an eye on
(do not grow; split further when you must touch them): `crates/thinclaw-channels/src/gmail.rs` (1,999L),
`crates/thinclaw-db/src/libsql_migrations.rs` (1,980L), `crates/thinclaw-secrets/src/store.rs` (1,974L),
and `src/agent/agent_loop/mod.rs` (1,968L). Any structural re-cut of a still-large module should be
its own focused change, not a rider on unrelated work.

---

## Crate Dependency-Direction Rules

Summary of `docs/CRATE_OWNERSHIP.md`. These are CI-enforced (code-style job runs the import/package guards).

- **Internal crates import each other directly as `thinclaw_*`.** New internal code imports the extracted
  crate, not the root facade.
- **Internal crates MUST NOT import the root `thinclaw` package.** The following searches must return
  **zero matches** (run them before pushing a crate-boundary change):

  ```bash
  rg "use thinclaw::" crates
  rg '^\s*thinclaw\s*=|^\s*\[.*\.thinclaw\]' crates -g Cargo.toml
  rg 'package\s*=\s*"thinclaw"' crates -g Cargo.toml
  ```

- **Root = facade + app wiring.** Root `src/*` preserves public paths (`thinclaw::agent`, `thinclaw::tools`,
  `thinclaw::channels`, `thinclaw::db`, `thinclaw::workspace`) and owns concrete DB/secrets/LLM/tool/
  channel/gateway wiring, Docker orchestration, `AppBuilder`, binaries, and host side-effects. It is a
  **compatibility boundary, not proof of extraction** — do not mark a runtime path "extracted" just because
  an adjacent DTO/helper moved to a crate (`CRATE_OWNERSHIP.md §Root-Owned Runtime Still In Root`).
- **Direction of new ports.** When extracting more behavior, the trait goes in the crate, the concrete
  adapter stays in root. The root-owned runtime list in `CRATE_OWNERSHIP.md` is the backlog of what still
  needs this treatment — consult it before deciding where new code lives.
- **Beware near-duplicate half-migrations.** The audit's canonical example — `src/history/store/` vs
  `crates/thinclaw-db/src/postgres_store/` near-byte-for-byte duplication (`AUDIT-FINDINGS.md §5`) — was
  resolved: `src/history/store/` was deleted and history consolidated onto `thinclaw-db` (`src/history/` is
  now a facade `mod.rs`). The rule still applies to any future fork: when a runtime path exists in both a
  crate and root, do not maintain two copies; pick the crate-owned one and make root a thin adapter.

---

## Feature-Flag Discipline

Authority: `docs/BUILD_PROFILES.md`. Profile composition:

```
edge     = libsql
light    = edge + postgres + wasm-runtime + gateway + html-to-markdown + document-extraction + timezones
desktop  = libsql + html-to-markdown + document-extraction + repl + timezones
full     = light + acp + repl/tui + tunnel + docker-sandbox + browser + nostr
```

- **Every PR states which profiles it touches.** A change to `#[cfg(feature = "...")]`-gated code must name
  the profiles (edge/light/desktop/full + all-features) it affects in the PR description. The CI feature
  matrix (`docs/BUILD_PROFILES.md §CI/CD`) compiles + clippies + test-compiles all of: edge, light, full,
  all-features, desktop, minimal-libsql, minimal-postgres. Assume any of these can break and check the gate
  for the profile you touched.
- **`edge` is the canary.** It must avoid the heavy runtime set (no Wasmtime, Postgres, browser, Docker,
  Nostr, document-extraction). If your change pulls a heavy dep into a code path reachable from `edge`,
  the matrix will fail — gate it.
- **No flag enabled by no profile.** A feature that no profile turns on is dead weight masquerading as
  capability. The audit's example was the `voice` feature (and `voice_wake.rs` + `cpal`), enabled by **no**
  profile. Its resolution was **WIRE** (not erase): `voice_wake` is now connected into the dispatcher
  (wake-word utterances route through `capture_and_transcribe` → dispatch), while the `voice` feature stays
  deliberately **opt-in**: it is not in `full` and is only reachable via an explicit `--features …,voice`
  combo, documented as opt-in in `BUILD_PROFILES.md`. When adding a flag, either add it to a profile in the
  same PR or document in `BUILD_PROFILES.md` why it is opt-in (the `voice`/`bedrock`/`bundled-wasm`/
  `integration` table in `§full vs --all-features` is the template).
- **`desktop` omits `wasm-runtime`.** WASM tools/channels are not available under the desktop profile
  (`AUDIT-FINDINGS.md §2 WASM-tools row`). Do not assume a WASM runtime exists in desktop builds; gate
  accordingly or document the omission.
- **Feature unification is real.** `cargo` unifies features across the workspace; a transitive enable can
  link code you didn't intend (see the AWS legacy-TLS webpki note in `deny.toml:11-24`). Verify with the
  exact profile, not just `--all-features`.

---

## Dual DB Backend Parity Rules

ThinClaw ships **both** Postgres (`light`/`full`) and libSQL (`edge`/`desktop`, the **desktop default**).
They must behave identically. Authority: `crates/thinclaw-db/`, workstream `02-database-correctness-and-parity.md`.

- **Every query-shape change is tested on both backends.** The `db_contract` test runs against Postgres
  and libSQL (`BUILD_PROFILES.md §CI/CD`). If you change a query, a projection, ordering, NULL handling, or
  a search path, it must pass on both — not just the one you ran locally.
- **Sanitize before FTS5 `MATCH`.** This was the audit's divergence bug (finding #3): libSQL transcript
  search fed **raw user input** to FTS5 `MATCH`, so a query containing `:`/`"`/`-` threw where Postgres
  tolerates it. It is now **fixed**: transcript search sanitizes via the shared
  `super::fts::sanitize_fts5_match(query)` before `MATCH` at
  `crates/thinclaw-db/src/libsql/conversations/mod.rs:574` (the file was decomposed into a directory module).
  The rule stands: any new FTS5 `MATCH` path must run through `sanitize_fts5_match`; **reuse that shared
  sanitizer, do not invent a second one.**
- **Postgres tolerance is not a license for libSQL fragility.** Whenever Postgres "just works" with raw
  input, ask whether libSQL's FTS5/SQL dialect does too. Divergence is a correctness bug on the
  desktop-default backend, which is where most real users run.
- **Parity tests must check behavior, not just shape.** The `schema_divergence` parity test currently only
  compares **column names** (`AUDIT-FINDINGS.md §3, §4`). Strengthening it is owned by
  `02-database-correctness-and-parity.md`; until then, do not rely on it to catch a value-level divergence —
  add a `db_contract` case that exercises the actual query on both backends.

---

## Secret & Crypto Rules

Authority: `crates/thinclaw-secrets/`, `src/NETWORK_SECURITY.md §Authentication Mechanisms Summary` and
`§Credential Handling`. The existing stack is strong — match it, don't reinvent it.

- **Constant-time compares for every secret/token/HMAC.** Use `subtle::ConstantTimeEq` (`ct_eq`), the
  pattern used by gateway bearer, webhook secret, per-job token (`src/channels/web/auth.rs`,
  `src/channels/http.rs`, `src/orchestrator/auth.rs`). **Historical bad example (never reintroduce it):**
  the deleted `src/qr_pairing.rs` compared a one-time pairing token with a plain `==`
  (`self.info.pairing_token == token`) — timing-leaky. That module was ERASED in the dead-code sweep; if any
  token-compare path is ever re-added, it must use `ct_eq` from the start, never `==`.
- **Crypto is AES-256-GCM + HKDF-SHA256 + AAD.** Encryption derives a per-secret key via
  `Hkdf::<Sha256>::extract+expand` (`crates/thinclaw-secrets/src/crypto.rs:154-158`) and binds context with
  AAD via `encrypt_in_place(&nonce, aad, ...)` / `decrypt_in_place(nonce, aad, ...)`
  (`crypto.rs:96,144`; tamper test `test_aad_tamper_fails` at `crypto.rs:200`). New secret-bearing data must
  use `encrypt_with_aad`/`decrypt_with_aad` with a meaningful AAD (e.g. `user|key`), not the bare
  `encrypt`/`decrypt`.
- **Redacted `Debug`, always.** Any type holding secret material implements `Debug` to print `[REDACTED]`,
  like `Secret` (`crates/thinclaw-secrets/src/types.rs:58-65`) and `DecryptedSecret`
  (`types.rs:173-175`, regression test `test_decrypted_secret_redaction` at `types.rs:385`). Never derive
  `Debug` on a struct that holds a plaintext secret, token, or key.
- **Never log secret values.** The audit found **no** secret-value logging (`AUDIT-FINDINGS.md §8`) — keep
  it that way. Log secret *names*/lengths, never contents. Credential injection happens at the host boundary
  so guest/WASM/container code never sees the value (`src/tools/wasm/credential_injector.rs`,
  `src/sandbox/proxy/http.rs`).
- **Master key from OS keychain.** The secrets master key is sourced from the OS keychain
  (`crates/thinclaw-secrets/src/keychain.rs`); env-var master keys require an explicit allow-flag
  (`SecretsMasterKeySource::Env` + `allow_env_master_key`, `crates/thinclaw-config/src/secrets.rs`). Do not
  add a code path that reads a master key from disk or env without that explicit opt-in.

---

## WASM Guest Rules

Authority: `crates/thinclaw-tools/src/wasm/`, `crates/thinclaw-channels/src/wasm/`,
`src/NETWORK_SECURITY.md §Egress Controls / WASM Tool HTTP Requests`. Guests are **untrusted**.

- **No panic that traps the guest — char-aware slicing.** This was finding #5: `split_message` panicked on
  a multibyte UTF-8 boundary in three shipped channels (a byte-index slice `&remaining[..max_len]` that
  panics when `max_len` lands mid-codepoint). It is now **fixed**: slack and discord build chunks with
  `char_indices()` (`channels-src/slack/src/lib.rs:1055`, `channels-src/discord/src/lib.rs:778`) and
  `split_message` is a shared helper, matching the char-aware `whatsapp` implementation the fix was ported
  from. The rule stands: any string slicing in guest code must use
  `char_indices()`/`is_char_boundary()`/`floor_char_boundary`, never a raw byte index derived from a length
  budget.
- **Allowlist + path-traversal hardening for egress.** WASM HTTP goes through the host allowlist
  (`src/tools/wasm/allowlist.rs`): host exact/wildcard, path-prefix, method restriction, **HTTPS by default**,
  reject userinfo (`user:pass@host`), normalize-and-block `../` / `%2e%2e/`, reject invalid percent-encoding
  (`NETWORK_SECURITY.md §Egress Controls`). New guest egress paths must route through this validator, not a
  bespoke check.
- **Enforce declared resource limits.** Fuel + epoch + memory limits exist, and the table/instance limits
  that were once stubbed are now **enforced** via a wasmtime `ResourceLimiter` in
  `crates/thinclaw-tools/src/wasm/limits.rs` (`table_growing` returns `Ok(false)` past
  `max_tables`/`max_table_elements`; `instances()` caps at `max_instances`); the old
  `#[allow(dead_code)] // Reserved` counters are gone. The rule stands: if you add a new declared limit,
  enforce it in the limiter in the same PR — do not leave a declared-but-unenforced knob.
- **Broken-auth declarations are worse than none.** Finding #4: Discord WASM declared webhook signature
  verification but implemented none. It is now **fixed**: Ed25519 verification is performed host-side
  (`verify_discord_ed25519_signature`, `crates/thinclaw-channels/src/wasm/router.rs:316`), dispatched via
  `WebhookSecretValidation::DiscordEd25519` (`schema.rs:556`, `router.rs:700`); the WASM guest only *declares*
  the requirement. The rule stands: do not declare a security control in a `*.capabilities.json`/README that
  the host does not actually perform — implement it or remove the claim.
- **Shared SDK over copy-paste.** `json_response` / `split_message` / `conversation_scope_id` /
  `external_conversation_key` / `url_encode_path` / `validate_input_length` are copy-pasted across WASM
  channel crates, and that is **exactly why the `split_message` fix landed in only 1 of 4 copies**
  (`AUDIT-FINDINGS.md §5`). Prefer a shared WASM channel SDK helper; if you must fix a duplicated helper,
  grep for and fix **every** copy in the same PR (see Common Pitfalls Catalog).

---

## Error Taxonomy

- **`thiserror` per crate.** Each crate defines its own error enum with `#[derive(thiserror::Error)]` and a
  `#[from]`/`#[source]` chain. Existing good examples: `crates/thinclaw-secrets/src/types.rs` (`SecretError`),
  `crates/thinclaw-tools/src/wasm/error.rs`, `crates/thinclaw-tools/src/registry.rs`,
  `crates/thinclaw-db` persistence errors. New crate-level fallible code adds variants here, not a
  catch-all `anyhow` at a boundary that should be typed.
- **Do not flatten everything to `Internal`.** `crates/thinclaw-gateway/src/web/api.rs:14` defines a
  `GatewayApiErrorKind::Internal → 500` (`api.rs:54`). `src/api/experiments.rs` has **139 `map_err` sites**
  (audit counted ~106 flattening to `Internal`) — that collapses validation errors, not-found, conflicts,
  and genuine internals into one opaque 500, losing the cause and the right status code. When you add or
  touch a handler in that file: map to the **specific** kind (BadRequest/NotFound/Conflict/etc.), not
  `Internal`. (The structural cleanup of this god-file is owned by `10-architecture-overhaul.md` /
  `07-experiments-research-completion.md`; new edits must not add to the flattening.)
- **Preserve the cause.** Keep the source error in the chain (`#[source]` / `.map_err(Into::into)` /
  `#[from]`), and log it at the boundary. Never `.map_err(|_| Error::Internal)` — the dropped error is the
  thing the next operator needs.

---

## Async Pitfalls

- **Never hold a `std`/blocking lock across `.await`.** `clippy::await_holding_lock` catches this. The one
  historical instance in `crates/thinclaw-config/src/secrets.rs` (a test holding the `lock_env()` guard
  across an `.await`) is now carried as an explicit `#[allow(clippy::await_holding_lock)]` with a justifying
  comment, and CI now runs clippy with `--all-targets` so any *new* occurrence is caught (see Quality Gate).
  Use `tokio::sync::Mutex` if you must hold across `.await`, or scope the `std` guard so it drops before the
  await point — do not reach for a blanket `#[allow]` without a written justification.
- **Correct cancellation.** Background listeners must drain on shutdown via `oneshot` + `tokio::select!`
  (gateway/webhook/orchestrator all do this; `NETWORK_SECURITY.md §Graceful Shutdown` per listener). Note
  the documented gap: the sandbox proxy `stop()` does **not** await a join handle, so in-flight connections
  are not drained (`proxy/http.rs §Graceful Shutdown`) — match the gateway pattern (await the join) when
  adding a new listener, don't copy the proxy's fire-and-forget.
- **Bounded retries — always.** Finding (P2): the repo-project supervisor has an **unbounded merge-retry
  loop** at `src/api/repo_projects.rs` / supervisor `pipeline.rs:532` (`AUDIT-FINDINGS.md §4 P2`). Every
  retry loop needs a max-attempts cap and backoff. Never `loop { … retry … }` without an exit count.
- **Per-item error isolation in batch loops.** Finding: routine event dispatch does break-on-first-error,
  deferring the **whole batch** when one item fails (`src/agent/routine_engine.rs:898`). In a batch loop,
  isolate per-item failures (log + continue / collect errors), do not abort the batch on the first error
  unless the batch is genuinely all-or-nothing.
- **Computed-but-dropped async work.** Don't compute a value/future and then drop it — see the LLM
  CheapSplit cascade computed and discarded (`src/llm/route_planner.rs:565`, `AUDIT-FINDINGS.md §4 P1`).
  Either use the result or don't compute it.

---

## Testing Conventions

- **`db_contract` is dual-backend.** Query-shape changes get a contract case that runs on Postgres **and**
  libSQL (see Dual DB Backend Parity). A test that only runs on the backend you developed against is not
  sufficient.
- **`schema_divergence` should fail, not skip.** The parity test must fail loudly on missing
  `DATABASE_URL`/schema drift, not silently skip (`AUDIT-FINDINGS.md §4 P1: "fail-not-skip on missing
  DATABASE_URL"`). A skipped parity test is a false green. (Strengthening owned by
  `02-database-correctness-and-parity.md`.)
- **Snapshots for rendered output.** Use snapshot tests for prompt assembly, profile/identity rendering,
  and message formatting where exact output is load-bearing (the dual profile renderers drift precisely
  because there is no shared snapshot, `AUDIT-FINDINGS.md §5`).
- **`#[ignore]` policy + a nightly `--ignored` job.** There are 15 `#[ignore]` tests (Docker E2Es, live
  smokes) and **none run in CI** (`AUDIT-FINDINGS.md §9`). New `#[ignore]` must (a) carry a one-line reason
  comment, and (b) be runnable by a nightly `cargo test -- --ignored` job. **Do not** use `#[ignore]` to
  quarantine a flaky test without filing the underlying bug — the `autonomous_campaign_..._end_to_end`
  quarantine (`src/api/experiments.rs:5060`, commit `64b9572f`) masks an unfixed worktree/Docker race; that
  is the anti-pattern. Quarantine + tracked fix, not quarantine-and-forget.
- **Keep tests next to the module.** Use `tests.rs`/`test_support.rs` close to the code (CLAUDE.md hygiene),
  not a distant dumping ground.

---

## The Quality Gate

Run **all** of these before opening a PR. The `ship` skill (`.claude/commands/ship.md`) runs the core gate.

```bash
cargo fmt --all -- --check
cargo clippy --all --benches --tests --examples --all-features -- -D warnings
cargo test
```

- **`--all-targets` / `--all` matters.** CI's clippy now runs with `--all-targets --all-features`:
  `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` (`.github/workflows/ci.yml:66`),
  and the feature-matrix leg also passes `--all-targets` (`ci.yml:135`), so test/example/bench code is held
  to `-D warnings` in CI. Still run the full `--all --benches --tests --examples` form locally (the CLAUDE.md
  command) so you catch failures before the gate does.
- **`cargo deny check`** must pass, and it is **green on `main`**. The audit's RUSTSEC-2026-0182 advisory is
  resolved: `wasmtime-wasi` is now `36.0.12` in `Cargo.lock` and root `deny.toml` carries
  `[advisories] ignore = []` (no stale ignores). Do not add a new `ignore` to `deny.toml` without a
  web-verified, dated justification comment in the established style.
- **Profile matrix.** Before merge, verify the profiles your change touches compile (see Feature-Flag
  Discipline); the CI feature matrix will gate this but catch it locally first for any `#[cfg(feature)]`
  edit.
- **`/code-review high` before merge.** Run the high-effort review on the diff before requesting merge;
  use `/code-review ultra` for security-boundary or crate-boundary changes.

---

## Doc-Update Triggers

From `CLAUDE.md §Common Update Triggers` — the canonical doc is updated in the **same PR** as the behavior
change, code-adjacent spec doc first, broad overview second.

| If you change… | Update in the same PR |
|---|---|
| Onboarding / setup | `src/setup/README.md` + user-facing setup refs |
| Identity packs / `/personality` / memory-growth / cross-surface vocab | `docs/IDENTITY_AND_PERSONALITY.md`, `docs/MEMORY_AND_GROWTH.md`, `docs/SURFACES_AND_COMMANDS.md` |
| Experiments / research / runners / GPU cloud | `docs/RESEARCH_AND_EXPERIMENTS.md` |
| Delivery architecture | `docs/CHANNEL_ARCHITECTURE.md` + affected channel guides |
| Channel formatting behavior | owning native channel (`Channel::formatting_hints()`) **or** WASM `*.capabilities.json` first, then `docs/CHANNEL_ARCHITECTURE.md` |
| Extension flows | `docs/EXTENSION_SYSTEM.md`, `src/tools/README.md`, affected tool docs |
| Crate boundaries | `docs/CRATE_OWNERSHIP.md` + `CLAUDE.md` repo-shape notes |
| Security boundaries | `src/NETWORK_SECURITY.md` + top-level trust/safety wording |
| Tracked feature behavior | `FEATURE_PARITY.md` |

- **Doc claims must not run ahead of code.** The audit's `§7 Doc vs Code Drift` is a catalog of this failure:
  `CRATE_OWNERSHIP.md` listed 22 of 26 crates; `provider_catalog.rs:4` claims "20+ providers" vs 16 in the
  registry; Discord WASM `README.md:106` claims verification that does not exist; `deny.toml:3` points at a
  non-existent `code_style.yml`. When you fix the code, fix the claim. When you fix a claim, verify the code.
- **Avoid brittle counts / "default forever".** `CLAUDE.md §Documentation Rules`: no hardcoded inventories
  or counts unless the code guarantees them (`FEATURE_PARITY.md §20` hardcoded dated counts are the
  anti-example).

---

## Common Pitfalls Catalog

The concrete failure modes the audit found. Each remediation PR should self-check against this list.

1. **Fix lands in 1 of N duplicated copies.** The `split_message` UTF-8 fix landed only in `whatsapp`, not
   `telegram`/`slack`/`discord` (`AUDIT-FINDINGS.md §5`). **Rule:** when fixing a duplicated helper, grep for
   every copy and fix all of them in the same PR; prefer extracting a shared helper so there is one copy.

2. **Empty string treated as "present".** Finding #1: `gateway_auth_token: ""` → `Some("")` → empty `Bearer`
   authenticated. Now **fixed**: the `auth_token` builder trims then
   `.filter(|token| !token.trim().is_empty())` at `crates/thinclaw-config/src/channel_config.rs:200`,
   mirroring the `GatewayAccess` empty-token filter (`src/platform/gateway_access.rs:29`). **Rule:** treat
   empty/whitespace-only as **absent** for any credential, token, or required-presence value;
   `.filter(|s| !s.trim().is_empty())` after the trim.

3. **Computed-but-dropped values.** The CheapSplit cascade is computed then discarded
   (`src/llm/route_planner.rs:565`); native-streaming `finish_reason` is computed as always-`Stop` even with
   tool calls (`crates/thinclaw-llm/src/rig_adapter.rs:1611`). **Rule:** if you compute it, use it or delete
   the computation.

4. **Orphaned wired-looking code.** The audit's canonical examples (desktop cloud-sync, self-repair rebuild
   via `with_builder`, the native dynamic-library plugin pipeline, and observability `create_observer`) all
   *looked* connected but had zero runtime callers. Every one was subsequently **wired** (cloud-sync via
   `migrate_to_cloud`/`start_live_sync`; `with_builder` at `src/agent/agent_loop/mod.rs:672`;
   `ExtensionKind::NativePlugin` signature-gated dispatch; `create_observer` from `AppBuilder` at
   `src/app.rs:1717`). **Rule (REALIZE-THE-VISION):** prefer wiring such code via its existing port; if it is
   genuinely drifted cruft, ERASE it explicitly and update docs. Do not leave half-wired.

5. **Doc claims ahead of code.** See Doc-Update Triggers. **Rule:** a security/feature claim with no
   implementation is a bug, not just stale docs.

6. **Unbounded retry.** Repo-project merge-retry loop has no cap (`pipeline.rs:532`). **Rule:** every retry
   loop is bounded with backoff (see Async Pitfalls).

7. **God-file growth.** Adding unrelated behavior to a 3000–5700L file (`thread_ops.rs`,
   `api/experiments.rs`, `wasm/wrapper.rs`). **Rule:** split first or justify; never grow.

8. **Widening visibility to make a split compile.** Bumping a field/fn to `pub` so a decomposition compiles
   leaks internals (Architecture Hygiene). **Rule:** keep `pub(super)`/`pub(in crate::...)`; re-cut the
   boundary rather than widen the API. Preserve external paths with `pub use` re-exports only.

9. **Fail-open where fail-closed is expected.** The external shell scanner defaults fail-open
   (`AUDIT-FINDINGS.md §8`); the sandbox proxy credential injection only fires for plaintext HTTP while every
   shipped default is HTTPS, so the guarantee never fires (findings #6/#7). **Rule:** security defaults
   fail-closed; a guarantee that cannot fire must be wired to fire or deleted with the doc corrected.

10. **Backend-specific fragility.** Raw user input into libSQL FTS5 `MATCH` throws where Postgres tolerates
    (finding #3). **Rule:** sanitize for the stricter backend; reuse the existing sanitizer
    (`libsql/workspace.rs:677-693`); test on both backends.
