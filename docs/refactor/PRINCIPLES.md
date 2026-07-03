# Refactor Principles & Best Practices

Read once before executing. These are the non-negotiables — they are what keep a large refactor
from becoming a large regression.

## 1. Engineering principles

1. **Never ship code that doesn't build.** Compile-verification (`cargo check`) is the floor for
   every change, always — even when full runtime verification is out of scope. A broken `main` is
   never acceptable.
2. **Fix at the source, never paper over.** Remediate advisories by upgrading the affected crate,
   not by adding a `deny.toml` ignore. Replace a false guarantee with real enforcement (or delete
   it), don't leave it. The only acceptable "ignore" is for a genuinely unfixable transitive
   (yanked with no alternative *and* no parent upgrade), and it must be documented + time-bound.
3. **One task, one focused PR.** A PR that mixes a refactor with unrelated cleanup is unreviewable.
   Split first. Pair a fix with its regression guardrail in the same PR when one exists.
4. **Decompose before you add.** If a module is already a god-file, split it *before* adding the
   new behavior — never grow a god-file because it's convenient.
5. **Preserve public paths during moves.** When relocating a type/module, re-export from the old
   location (`pub use`) so callers don't break; widen visibility to `pub(crate)`/`pub(in ...)`,
   never to `pub`, just to make a split compile.
6. **Make invariants compiler- or CI-enforced.** A rule a human has to remember will be broken at
   538k LOC. Every architectural invariant in this plan ships with a guard (a clippy lint, a CI
   grep, a structural test) so it cannot silently regress.
7. **Errors are recoverable; panics are bugs.** Production code returns typed `Result`s; `unwrap`/
   `expect`/`unreachable!`/`panic!` belong in tests or at genuinely-infallible sites with a comment
   explaining why. A panic on the agent loop, a dispatcher, a gateway handler, or a Tauri command
   kills a user-visible session.
8. **Honesty in reporting.** If a change is compile-verified but not runtime-tested, say so. If an
   item is deferred or blocked, say why. A doc claim must never run ahead of the code.

## 2. This repo's own architecture rules (from `CLAUDE.md`)

- **No god-files.** Prefer directory modules when a subsystem outgrows a focused file; `mod.rs`
  stays a façade (declares submodules, re-exports the stable API, holds only narrow glue). Split by
  responsibility: `types`, `core`/manager, orchestration phases, provider adapters,
  persistence/query, platform helpers, test support. Add new behavior to the *narrowest* submodule
  that owns it. Avoid vague buckets (`misc`/`common`/`utils`) unless genuinely cross-cutting + small.
- **Crate dependency direction is one-way.** Extracted `thinclaw-*` crates must not import the root
  `thinclaw` package (CI-enforced: `rg 'use thinclaw::' crates` must be empty). Persistence must not
  depend on the agent layer; the gateway must be lighter than the tool runtime. Leaf crates
  (`thinclaw-types`, `-platform`, `-secrets`, …) carry zero intra-workspace deps.
- **Ports/adapters at the seam.** `thinclaw-agent` owns trait *definitions* (`ports.rs`); the root
  injects concrete DB/tool/channel adapters (`src/agent/root_ports.rs`, the `Root*Port` structs).
  Keep injecting through ports rather than reaching across layers.
- **Same-PR doc rule.** A PR that changes behavior in an area with a canonical doc MUST update that
  doc in the same PR (code-adjacent spec first, broader overview second). If it changes tracked
  feature behavior, update `FEATURE_PARITY.md` too. Canonical docs: see the `CLAUDE.md` table
  (`CHANNEL_ARCHITECTURE.md`, `CRATE_OWNERSHIP.md`, `SURFACES_AND_COMMANDS.md`, `BUILD_PROFILES.md`,
  the `apps/desktop/documentation/` set, etc.).
- **Channel formatting/config ownership lives in the channel layer**, not prompt assembly. Native
  channels override trait methods (`formatting_hints`, `config_schema`); WASM channels declare in
  `*.capabilities.json`. Never reintroduce channel-name switches in `src/llm/reasoning.rs`.

## 3. Lessons learned executing Phase 0 (do not relearn the hard way)

1. **Both lockfiles, always.** A manifest change (a new dep, even a transitive-graph change) updates
   *both* the root `Cargo.lock` **and** `apps/desktop/backend/Cargo.lock`. CI runs `cargo check
   --locked`; a stale desktop lockfile is the single most common self-inflicted CI break. After any
   `Cargo.toml` edit: `cargo check` the root *and* `cargo check` in `apps/desktop/backend`, then
   stage both lockfiles.
2. **The `StatusUpdate` ripple.** `thinclaw_channels_core::StatusUpdate` is a shared, *not*
   `#[non_exhaustive]` enum. Adding a variant forces an arm in **six** exhaustive matchers across
   crates: `thinclaw-channels/src/tui.rs` (`From`), `thinclaw-gateway/src/web/status.rs` (SSE),
   `src/channels/repl.rs`, `src/channels/acp.rs` (a `=> None` group), `thinclaw-channels/src/wasm/
   wrapper/conversions.rs`, and `apps/desktop/backend/src/thinclaw/event_mapping.rs`. Several are
   feature-gated — a default `cargo check` won't see them; CI's `--all-features` will. **Backlog
   B1 makes the enum `#[non_exhaustive]` to end this tax.** Until then, verify with `cargo check`
   *and* `cargo check -p thinclaw-channels --all-features`.
3. **`#[non_exhaustive]` semantics.** On an *enum*, it forces external crates to add a `_` arm when
   matching, but it does **not** block constructing existing variants externally (like
   `std::io::ErrorKind`). So the ~50 emit sites are unaffected; only the matchers need `_`.
4. **`-D warnings` blocks warn-lints.** CI runs `cargo clippy ... -- -D warnings`. You cannot add a
   `clippy::unwrap_used = "warn"` workspace lint without it becoming a hard error on ~2,200 sites.
   `clippy::await_holding_lock` is already warn-by-default → already enforced under `-D warnings`.
5. **Verify the right instance.** A multi-version dep advisory targets a *specific* version (the
   rand advisory was on `0.8.5`, not the `0.9.4` that looked guilty). Read the advisory's
   "Solution" line; `cargo tree -i <crate>@<version>` to find the real parent.
6. **specta/serde shapes.** Internally-tagged enums (`#[serde(tag = "kind")]`) cannot have tuple
   variants — use struct variants (this broke `export_bindings` once). After any command or
   `UiEvent`/DTO change, regenerate bindings (`cargo run --example export_bindings`) and `tsc`.
7. **Generated commands vs lib wrappers.** The desktop frontend calls generated commands directly
   (`thinclawCommands.thinclawX(...)`, a type-level `Pick` of all `thinclaw*` commands) — adding a
   command needs only registration + `export_bindings`; no hand-written wrapper.
8. **Stacked-PR cascade hazard.** When a stacked PR auto-merges into its feature-branch *base*
   faster than the base merges to `main`, content can orphan in the intermediate branch. Verify the
   foundation branch actually contains each follow-up before relying on it reaching `main`.
9. **Don't pile conflicting PRs into a deep queue.** Strict-mode branch protection + a 15-PR queue
   means concurrent PRs on the same files (the matchers, `bridge.rs`, the lockfiles) thrash on
   update-branch and conflicts. Sequence: clean items now; same-file items after their in-flight
   neighbor lands. (This is why the backlog is split into Wave A / Wave B.)

## 4. Definition of done — per task

A task is done when: the change compiles (`cargo check`, plus `--all-features` if it touches
feature-gated code); the relevant tests pass; clippy is clean under `-D warnings`; both lockfiles
are synced if a manifest changed; bindings + `tsc` pass if a command/DTO changed; the owning
canonical doc + `FEATURE_PARITY.md` are updated if behavior changed; and the task's own **Verify**
and **Guardrail** items in the backlog are satisfied. See `EXECUTION_PLAYBOOK.md` for the exact
gate per change-type.
