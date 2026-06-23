# Best Practices & Pitfalls — Global Engineering Guardrails (CC-A)

> **Workstream:** CC-A (cross-cutting) · **Status:** authoritative for the whole remediation effort.
> Every other workstream doc (`WS-*` / numbered `0N-*.md`) inherits these rules. When a task in another
> workstream conflicts with a rule here, this doc wins unless the task explicitly justifies the exception in writing.
>
> This is a **guardrails** doc, not a task list. It has no `cargo` tasks of its own beyond the Quality Gate.
> Its job is to keep the rest of the remediation from re-introducing the exact failure modes the
> 2026-06-23 audit (`AUDIT-FINDINGS.md`) found. Citations below are real `file:line` anchors verified
> against the working tree on 2026-06-23 (`main` @ `9e707985`).

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
  the existing port** — e.g. the dead self-repair rebuild path is exactly a missing `with_builder` injection
  (`src/agent/agent_loop.rs:605`), not a missing feature.

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

**Worst current god-files (do not grow; split when you must touch them)** — from `AUDIT-FINDINGS.md §5`:
`crates/thinclaw-channels/src/wasm/wrapper.rs` (5701L), `src/api/experiments.rs` (5434L),
`crates/thinclaw-tools/src/builtin/skill.rs` (4577L) + `src/tools/builtin/skill_tools.rs` (4381L),
`src/agent/thread_ops.rs` (3032L, ~850L `process_approval`), `src/llm/runtime_manager.rs` (~3100L),
`src/extensions/manager.rs` (3343L). The structural overhaul of these is owned by workstream
`10-architecture-overhaul.md` — **do not** restructure them from another workstream; note the dependency
and make the minimal local change.

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
- **Beware near-duplicate half-migrations.** `src/history/store/` vs `crates/thinclaw-db/src/postgres_store/`
  is near-byte-for-byte duplication maintained twice (`AUDIT-FINDINGS.md §5`). When you touch either, check
  the other; do not deepen the fork. (Resolution is owned by `10-architecture-overhaul.md`.)

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
  capability. Confirmed live example: the `voice` feature (and `voice_wake.rs`, ~749L, + `cpal`) is enabled
  by **no** profile — `light` excludes voice (`BUILD_PROFILES.md §light "Excluded"`), and it's only
  reachable via the explicit `--features light,voice` custom combo. The audit's directive: **WIRE it into a
  profile or ERASE it** (`AUDIT-FINDINGS.md §6`), don't leave it orphaned. When adding a flag, add it to a
  profile in the same PR or document in `BUILD_PROFILES.md` why it is opt-in (the `voice`/`bedrock`/
  `bundled-wasm`/`integration` table in `§full vs --all-features` is the template).
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
- **Sanitize before FTS5 `MATCH`.** This is the live divergence bug (finding #3): libSQL transcript search
  feeds **raw user input** to FTS5 `MATCH` at `crates/thinclaw-db/src/libsql/conversations.rs:846`, so a
  query containing `:`/`"`/`-` throws where Postgres tolerates it. The correct, already-in-repo pattern is
  next door at `crates/thinclaw-db/src/libsql/workspace.rs:677-693`: split on non-alphanumerics and quote
  each token (`"time" "sensitive" "notes"`), falling back to empty-result when the sanitized query is empty.
  **Reuse that sanitizer; do not invent a second one.**
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
  `src/channels/http.rs`, `src/orchestrator/auth.rs`). **Bad example to never copy:** `src/qr_pairing.rs:219`
  compares a one-time pairing token with a plain `==` (`self.info.pairing_token == token`) — timing-leaky.
  (qr_pairing is an ERASE candidate; if it is instead WIRED, the `==` must become `ct_eq` first.)
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

- **No panic that traps the guest — char-aware slicing.** This is the live finding #5: `split_message`
  panics on a multibyte UTF-8 boundary in three shipped channels. **Bad:**
  `channels-src/telegram/src/lib.rs:2189` does `let search_area = &remaining[..max_len];` — a byte-index
  slice that panics when `max_len` lands mid-codepoint (same in `slack/src/lib.rs:1058`,
  `discord/src/lib.rs:750`). **Good (already shipped, port it):** `channels-src/whatsapp/src/lib.rs:2096`
  counts/iterates by `chars()` and uses `char_indices()` (`whatsapp ... :2097,2102`). Any string slicing in
  guest code must use `char_indices()`/`is_char_boundary()`/`floor_char_boundary`, never a raw byte index
  derived from a length budget.
- **Allowlist + path-traversal hardening for egress.** WASM HTTP goes through the host allowlist
  (`src/tools/wasm/allowlist.rs`): host exact/wildcard, path-prefix, method restriction, **HTTPS by default**,
  reject userinfo (`user:pass@host`), normalize-and-block `../` / `%2e%2e/`, reject invalid percent-encoding
  (`NETWORK_SECURITY.md §Egress Controls`). New guest egress paths must route through this validator, not a
  bespoke check.
- **Enforce declared resource limits.** Fuel + epoch + memory limits exist, but **table/instance limits are
  stubbed**: `crates/thinclaw-tools/src/wasm/limits.rs:71,76` carry `#[allow(dead_code)] // Reserved` on
  `tables_created`/`instances_created` — the counters are never incremented, so `max_tables`/`max_instances`
  are not enforced (`AUDIT-FINDINGS.md §6 WIRE list`). If you touch the limiter, wire these (REALIZE) rather
  than leave the reserved comment; if you add a new declared limit, enforce it in the same PR.
- **Broken-auth declarations are worse than none.** Finding #4: Discord WASM declares webhook signature
  verification (`channels-src/discord/src/lib.rs:148`, README claims it at `README.md:106`) but implements
  none. Do not declare a security control in a `*.capabilities.json`/README that the code does not perform —
  implement it (host Ed25519 verification) or remove the claim.
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

- **Never hold a `std`/blocking lock across `.await`.** `clippy::await_holding_lock` catches this — and it
  fires today on `crates/thinclaw-config/src/secrets.rs` (the test `env_source_uses_allowed_key` holds the
  `lock_env()` guard across an `.await` at ~`:144`, only surfaced because CI omits `--all-targets`, see
  Quality Gate). Use `tokio::sync::Mutex` if you must hold across `.await`, or scope the `std` guard so it
  drops before the await point.
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

- **`--all-targets` / `--all` matters.** Note: CI's clippy currently runs `cargo clippy --workspace -- -D
  warnings` **without `--all-targets`** (`.github/workflows/ci.yml:52,121`), so test/example/bench code
  escapes `-D warnings` in CI — which is how the `await_holding_lock` in `secrets.rs` survives. **Locally you
  must run the full `--all --benches --tests --examples` form** (the CLAUDE.md command) so you catch what CI
  misses. (Fixing CI to add `--all-targets` is owned by `10-architecture-overhaul.md` / build-health work.)
- **`cargo deny check`** must pass. It is **currently red on `main`** (RUSTSEC-2026-0182, wasmtime-wasi
  36.0.10, `Cargo.toml:160`); the fix is `cargo update -p wasmtime-wasi --precise 36.0.11` plus removing the
  three stale `advisory-not-detected` ignores at `deny.toml:22-24` — owned by the P0 security workstream. Do
  not add a new `ignore` to `deny.toml` without a web-verified, dated justification comment in the
  established style (`deny.toml:11-24`).
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

2. **Empty string treated as "present".** `gateway_auth_token: ""` → `Some("")` → empty `Bearer`
   authenticates (finding #1, `crates/thinclaw-config/src/channel_config.rs:183` — `.map(trim)` with **no**
   `.filter(|s| !s.is_empty())`). The correct pattern exists in the parallel `GatewayAccess` path which
   filters empties. **Rule:** treat empty/whitespace-only as **absent** for any credential, token, or
   required-presence value; `.filter(|s| !s.trim().is_empty())` after the trim.

3. **Computed-but-dropped values.** The CheapSplit cascade is computed then discarded
   (`src/llm/route_planner.rs:565`); native-streaming `finish_reason` is computed as always-`Stop` even with
   tool calls (`crates/thinclaw-llm/src/rig_adapter.rs:1611`). **Rule:** if you compute it, use it or delete
   the computation.

4. **Orphaned wired-looking code.** Subsystems that *look* connected but have zero runtime callers: desktop
   cloud-sync (`apps/desktop/backend/src/cloud/mod.rs:445` flips a flag, spawns no sync), self-repair rebuild
   (`with_builder` never injected, `src/agent/agent_loop.rs:605`), native dynamic-library plugin pipeline
   (~1500L, zero callers), observability `create_observer` (never called). **Rule (REALIZE-THE-VISION):**
   prefer wiring these via their existing port; if genuinely drifted cruft, ERASE explicitly and update docs.
   Do not leave half-wired.

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
