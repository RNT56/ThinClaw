# ThinClaw Full Codebase Audit Plan

> **Created:** 2026-04-14  
> **Scope:** Complete audit of the ThinClaw Rust codebase (~250K LOC, 454 source files)  
> **Goal:** Identify architectural debt, security gaps, quality issues, and hardening opportunities across every subsystem

---

## Table of Contents

1. [Audit Objectives](#1-audit-objectives)
2. [Codebase Profile](#2-codebase-profile)
3. [Audit Domain 1: Architecture & Design](#3-audit-domain-1-architecture--design)
4. [Audit Domain 2: Code Quality & Maintainability](#4-audit-domain-2-code-quality--maintainability)
5. [Audit Domain 3: Security & Trust Boundaries](#5-audit-domain-3-security--trust-boundaries)
6. [Audit Domain 4: Performance & Scalability](#6-audit-domain-4-performance--scalability)
7. [Audit Domain 5: Testing & Coverage](#7-audit-domain-5-testing--coverage)
8. [Audit Domain 6: Documentation](#8-audit-domain-6-documentation)
9. [Audit Domain 7: CI/CD & DevOps](#9-audit-domain-7-cicd--devops)
10. [Audit Domain 8: Dependency Management](#10-audit-domain-8-dependency-management)
11. [Audit Domain 9: Database & Migrations](#11-audit-domain-9-database--migrations)
12. [Audit Domain 10: Feature Completeness & Consistency](#12-audit-domain-10-feature-completeness--consistency)
13. [Audit Domain 11: Operational Readiness](#13-audit-domain-11-operational-readiness)
14. [Audit Domain 12: WASM & Extension Ecosystem](#14-audit-domain-12-wasm--extension-ecosystem)
15. [Execution Plan](#15-execution-plan)
16. [Deliverables](#16-deliverables)
17. [Known Healthy — Verified Strengths](#17-known-healthy--verified-strengths)
18. [Remediation Plans](#18-remediation-plans)
19. [Prior Audit Work](#19-prior-audit-work)

---

## 1. Audit Objectives

### Primary Goals

1. **Structural integrity** — verify the codebase compiles cleanly, has no dead code paths, and the module graph matches the intended architecture
2. **Security hardening** — identify trust boundary violations, secret exposure risks, and injection surfaces
3. **Production readiness** — assess error handling, panic safety, graceful degradation, and operational observability
4. **Maintainability** — flag over-coupled modules, mega-files, unclear ownership, and undocumented invariants
5. **Test confidence** — evaluate coverage gaps, test quality, and edge-case handling

### Non-Goals

- Full rewrite or major refactoring proposals (unless critical)
- Performance benchmarking with production workloads
- Auditing the Scrappy (Tauri) desktop app codebase
- Auditing WASM channel/tool guest source code (`channels-src/`, `tools-src/`)

---

## 2. Codebase Profile

| Metric | Value |
|--------|-------|
| Total `.rs` source files | 454 |
| Total lines of Rust | ~250,000 |
| Cargo features | 13 (default: `light`) |
| Database backends | 2 (PostgreSQL, libSQL/Turso) |
| Messaging channels | 14+ (native + WASM) |
| Built-in tools | 43 |
| Migration files | 17 |
| Integration tests | 9 test files |
| Benchmarks | 2 |
| CI workflows | 6 |

### Mega-Files (>1,500 LOC) — Priority Audit Targets

| File | LOC | Concern |
|------|-----|---------|
| `src/channels/web/server.rs` | 7,923 | Web gateway — blast radius of any bug is enormous |
| `src/api/experiments.rs` | 4,548 | Research platform API — complex state machine |
| `src/channels/wasm/wrapper.rs` | 4,173 | WASM channel runtime — trust boundary |
| `src/agent/dispatcher.rs` | 3,877 | Core message dispatch — highest fan-out |
| `src/history/store.rs` | 3,580 | Session history — data integrity critical |
| `src/extensions/manager.rs` | 2,753 | Extension lifecycle — plugin trust boundary |
| `src/channels/signal.rs` | 2,743 | Signal channel — external process IPC |
| `src/settings.rs` | 2,712 | Settings — every subsystem depends on this |
| `src/agent/learning.rs` | 2,661 | Learning loop — autonomous code changes |
| `src/llm/runtime_manager.rs` | 2,520 | LLM runtime — provider orchestration |
| `src/agent/thread_ops.rs` | 2,429 | Thread operations — complex state transitions |
| `src/llm/reasoning.rs` | 2,053 | Prompt assembly — security-sensitive |
| `src/agent/worker.rs` | 1,944 | Worker runtime — concurrency management |
| `src/db/postgres.rs` | 1,909 | PostgreSQL backend — SQL injection surface |
| `src/workspace/workspace_core.rs` | 1,821 | Memory/workspace — identity file handling |

---

## 3. Audit Domain 1: Architecture & Design

### 3.1 Module Dependency Graph

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Circular dependency check** | P0 | `src/lib.rs`, all `mod.rs` files | Are there any circular `use` paths between top-level modules? |
| **Feature gate correctness** | P0 | `Cargo.toml`, `src/lib.rs`, all `#[cfg(feature = ...)]` | Does every feature-gated module compile correctly when its feature is disabled? Do `light`, `desktop`, `full` profiles each produce valid builds? |
| **Dead module detection** | P1 | All `mod.rs` files, `pub use` re-exports | Are there modules declared but never used? Empty directories? Orphaned files? |
| **Layering violations** | P1 | `src/agent/dispatcher.rs`, `src/tools/registry.rs` | Does the dispatcher reach into layers it shouldn't? Do tools depend on agent internals? |

### 3.2 Core Architecture Contracts

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Prompt assembly ownership** | P0 | `src/workspace/workspace_core.rs`, `src/llm/reasoning.rs`, `src/llm/provider.rs` | Is there a clear contract for which layer owns identity, context, channel, runtime, and provider metadata injection? Are there overlapping responsibilities? |
| **Session lifecycle** | P1 | `src/agent/session.rs`, `src/agent/session_manager.rs`, `src/agent/global_session.rs` | What are the invariants for session creation, cutover, pruning, and cross-channel continuity? Are they enforced or merely assumed? |
| **Channel trait surface** | P1 | `src/channels/channel.rs`, all channel implementations | Is the `Channel` trait minimal and complete? Are there methods that only one channel implements? |
| **Tool registration contract** | P1 | `src/tools/registry.rs`, `src/tools/tool.rs`, `src/tools/toolset.rs` | Are all tools registered consistently? Is the registry thread-safe? Are tool dependencies (services, configs) injected cleanly? |
| **Error propagation model** | P1 | `src/error.rs`, grep for `unwrap()`, `.expect()`, `panic!()` | Is the error type hierarchy consistent? Are there panics in library code that should be `Result`s? |

### 3.3 State Management

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Global mutable state inventory** | P0 | All `static`, `lazy_static`, `OnceLock`, `OnceCell` usage | What global state exists? Is it necessary? Is it properly synchronized? |
| **Config hot-reload safety** | P1 | `src/config/`, `src/settings.rs` | Can a hot-reload produce an inconsistent snapshot? Are subscribers notified atomically? |
| **Session state consistency** | P1 | `src/agent/session.rs` | What happens if two channels write to the same session concurrently? |
| **Subagent state isolation** | P1 | `src/agent/subagent_executor.rs` | Do subagents properly isolate their state? Can a child contaminate the parent's session? |

---

## 4. Audit Domain 2: Code Quality & Maintainability

### 4.1 Code Health Metrics

| Item | Priority | Scope | Questions |
|------|----------|-------|-----------|
| **Clippy compliance** | P0 | Full codebase | `cargo clippy --all-targets --all-features -- -D warnings` — zero warnings? |
| **Format compliance** | P0 | Full codebase | `cargo fmt --check` — zero deviations? |
| **Dead code analysis** | P1 | Full codebase | `cargo +nightly udeps` — unused dependencies? `#[allow(dead_code)]` count? |
| **Complexity hotspots** | P1 | Mega-files listed above | Functions >100 LOC? Cyclomatic complexity outliers? Deeply nested match/if arms? |
| **Clone audit** | P2 | Agent and LLM modules | Unnecessary `.clone()` on large types? Arc vs clone patterns? |
| **String allocation audit** | P2 | Hot paths (prompt assembly, tool dispatch) | Excessive `String::from()`, `format!()`, `.to_string()` on hot paths? |

### 4.2 API Design

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Builder pattern consistency** | P1 | `src/app.rs`, configs, tool builders | Are complex constructors using builders consistently? |
| **Type safety for IDs** | P1 | Session IDs, actor IDs, agent IDs | Are IDs typed (newtype wrappers) or raw strings/UUIDs? Could a session ID be confused with an agent ID at compile time? |
| **Trait coherence** | P1 | `src/db/mod.rs`, `src/channels/channel.rs`, `src/tools/tool.rs` | Are trait implementations complete? Any `todo!()` or `unimplemented!()` in trait impls? |
| **Public API surface** | P2 | All `pub` exports via `src/lib.rs` | Is the public API minimal? Are internal types leaking? |

### 4.3 Code Smells

| Item | Priority | Scope | What to Look For |
|------|----------|-------|-----------------|
| **God objects** | P1 | `src/agent/dispatcher.rs`, `src/settings.rs` | Files with too many responsibilities. Can they be decomposed? |
| **Stringly-typed APIs** | P1 | Settings, config keys, tool names | Are there string constants that should be enums? |
| **Copy-paste duplication** | P2 | Channel implementations, tool implementations | Shared patterns that should be extracted to helpers? |
| **Commented-out code** | P2 | Full codebase | Significant blocks of commented code that should be removed or tracked as issues? |
| **TODO/FIXME/HACK inventory** | P2 | Full codebase | `grep -rn "TODO\|FIXME\|HACK\|XXX\|SAFETY"` — categorize and prioritize |

---

## 5. Audit Domain 3: Security & Trust Boundaries

> [!CAUTION]
> This is the highest-stakes audit domain. Security issues found here should be flagged immediately, not batched.

### 5.1 Trust Boundary Validation

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **WASM sandbox escape paths** | P0 | `src/sandbox/`, `src/channels/wasm/`, `src/tools/wasm/` | Can WASM guests access host filesystem, network, or secrets beyond their capability grants? |
| **MCP server trust** | P0 | `src/tools/mcp/` | Are MCP responses treated as untrusted? Can a malicious MCP server inject tool calls? |
| **Shell command injection** | P0 | `src/tools/builtin/shell.rs`, `src/tools/builtin/shell_security.rs` | Are all command constructions safe from injection? Does ANSI/Unicode normalization cover all known bypass techniques? |
| **SQL injection** | P0 | `src/db/postgres.rs`, `src/db/libsql_migrations.rs`, `src/db/mod.rs` | Are all queries parameterized? Any string interpolation in SQL? |
| **Path traversal** | P0 | `src/tools/builtin/file.rs`, `src/safety/skill_path.rs`, `src/workspace/` | Can agents read/write outside allowed directories? Are `..` traversals blocked consistently? |
| **SSRF** | P0 | `src/tools/builtin/http.rs`, `src/safety/media_url.rs` | Are all outbound HTTP requests validated against SSRF? IPv6 transition bypass? Cloud metadata endpoints? |

### 5.2 Secret Management

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Secret exposure in logs** | P0 | `src/safety/leak_detector.rs`, all `tracing::` calls | Can API keys, tokens, or PII appear in log output? Are secrets redacted before logging? |
| **Secret exposure in prompts** | P0 | `src/llm/reasoning.rs`, `src/workspace/workspace_core.rs` | Are raw user IDs, phone numbers, or API keys ever sent to LLM providers in prompts? Status of PII redactor? |
| **Env var handling** | P1 | `.env.example`, `src/config/`, `src/bootstrap.rs` | Are all secret env vars documented? Are defaults safe? |
| **Keychain integration** | P1 | `security-framework`, `secret-service` usage | Are keychain operations properly scoped? Can unrelated apps access ThinClaw secrets? |
| **Credential sync security** | P1 | `src/llm/credential_sync.rs` | Are watched auth files validated before consumption? Could a symlink attack compromise auth sync? |

### 5.3 Input Validation

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Prompt injection defense** | P0 | `src/safety/sanitizer.rs` | What injection patterns are detected? Any known bypasses? Coverage of invisible Unicode, HTML comments, hidden divs? |
| **Context file scanning** | P0 | `src/workspace/workspace_core.rs` | Are AGENTS.md, SOUL.md, USER.md, .cursorrules scanned for injection before prompt inclusion? |
| **Webhook payload validation** | P1 | `src/channels/http.rs`, `src/channels/webhook_server.rs` | Are incoming webhooks authenticated? Signature verification for HMAC? Body size limits? |
| **Media validation** | P1 | `src/media/`, `src/document_extraction/` | Are media files validated before processing? Can a crafted PDF/image cause OOM or code execution? |
| **WebSocket message validation** | P1 | `src/channels/web/server.rs` | Are WS messages validated? Max message size? Rate limiting? |

### 5.4 Authentication & Authorization

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Gateway auth model** | P0 | `src/channels/web/server.rs`, `src/tailscale.rs` | Is every API endpoint authenticated? Are there unauthenticated endpoints beyond health checks? |
| **Token validation** | P1 | Bearer token handling | Constant-time comparison? Token expiration? Revocation? |
| **Pairing security** | P1 | `src/safety/device_pairing.rs`, `src/pairing/` | Is the pairing protocol resistant to replay and brute-force? |
| **Multi-actor isolation** | P1 | `src/identity/mod.rs`, actor-scoped queries | Can Actor A access Actor B's sessions, routines, or memory? |

### 5.5 Cryptographic Practices

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Algorithm choices** | P1 | `Cargo.toml` crypto deps, `src/secrets/` | AES-256-GCM, SHA-256/BLAKE3, Ed25519 — all current and appropriate? |
| **Key derivation** | P1 | `hkdf` usage | Are KDF parameters appropriate? Salt handling? |
| **Random number generation** | P1 | `rand` usage | Is `OsRng` or `ThreadRng` used (not a weak PRNG)? Are nonces unique? |
| **Constant-time operations** | P2 | `subtle` crate usage | Are all token/HMAC comparisons constant-time? |

---

## 6. Audit Domain 4: Performance & Scalability

### 6.1 Resource Usage

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Memory allocation patterns** | P1 | Agent loop, prompt assembly, session storage | Are there unbounded buffers? Can a long conversation OOM the process? |
| **Connection pooling** | P1 | `deadpool-postgres`, HTTP clients | Are DB connections pooled with appropriate limits? Are HTTP clients reused? |
| **Tokio runtime configuration** | P1 | `src/main.rs` | Is the runtime configured appropriately? Thread pool size? |
| **Channel backpressure** | P1 | `tokio::sync::mpsc`, `broadcast` channels | Are channels bounded? What happens on overflow? |

### 6.2 Hot Path Analysis

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Prompt assembly performance** | P1 | `src/llm/reasoning.rs` | How many string allocations per prompt build? Can it be reduced? |
| **Tool dispatch latency** | P2 | `src/agent/dispatcher.rs`, `src/tools/registry.rs` | Is tool lookup O(1)? Any unnecessary cloning? |
| **Session lookup** | P2 | `src/agent/session_manager.rs` | Is session lookup efficient? LRU size appropriate? |
| **SSE broadcast fan-out** | P2 | `src/channels/web/server.rs` | Does broadcasting to many clients block the agent loop? |

### 6.3 Concurrency

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Lock contention** | P1 | All `Mutex`, `RwLock`, `parking_lot` usage | Are locks held across async boundaries? Any potential deadlocks? |
| **Subagent concurrency limits** | P1 | `src/agent/subagent_executor.rs` | Is the concurrency limit (default 5) enforced correctly? What happens when exceeded? |
| **Spawn leak detection** | P1 | All `tokio::spawn` calls | Are spawned tasks tracked? Can they leak? Are JoinHandles collected? |
| **Graceful shutdown** | P1 | `src/main.rs`, signal handlers | Does shutdown wait for in-flight work? Are connections drained? |

---

## 7. Audit Domain 5: Testing & Coverage

### 7.1 Test Infrastructure

| Item | Priority | Key Files | Questions |
|------|----------|-------|-----------|
| **Unit test coverage** | P0 | All `#[cfg(test)]` modules | Which modules have zero tests? Which have only trivial tests? |
| **Integration test coverage** | P0 | `tests/` directory (9 files) | What end-to-end flows are tested? What critical paths are untested? |
| **Test harness quality** | P1 | `src/testing.rs` | Is the test harness comprehensive? Mock providers, DB backends, channels? |
| **Snapshot tests** | P1 | `tests/snapshots/`, `insta` usage | Are snapshots up to date? Covering the right outputs? |
| **Fuzz testing** | P2 | `fuzz/` directory | What inputs are fuzzed? Are security-critical parsers fuzzed (prompt injection patterns, shell commands, WASM inputs)? |

### 7.2 Critical Path Coverage

These paths **must** have test coverage. Flag any that don't:

| Critical Path | Expected Coverage | Key Files |
|---------------|-------------------|-----------|
| Agent message → tool call → response | Integration | `src/agent/dispatcher.rs`, `src/agent/thread_ops.rs` |
| Shell command → security check → approval → execution | Unit + Integration | `src/tools/builtin/shell.rs`, `src/tools/builtin/shell_security.rs` |
| WASM tool load → sandbox → execute → return | Integration | `src/tools/wasm/`, `src/sandbox/` |
| Multi-provider failover chain | Unit + Integration | `src/llm/failover.rs`, `src/llm/runtime_manager.rs` |
| Session create → persist → restore → prune | Integration | `src/agent/session_manager.rs`, `src/db/` |
| Context monitor → compaction → post-compaction injection | Integration | `src/agent/context_monitor.rs`, `src/agent/compaction.rs`, `src/context/` |
| Webhook inbound → auth → route → respond | Integration | `src/channels/http.rs`, `src/channels/web/server.rs` |
| Database migration → schema validation | Integration | `src/db/libsql_migrations.rs`, `migrations/` |
| Routine schedule → trigger → execute → audit log | Integration | `src/agent/routine_engine.rs`, `src/agent/routine_audit.rs` |
| Learning loop → evaluation → candidate → apply/reject | Integration | `src/agent/learning.rs` |

### 7.3 Test Quality

| Item | Priority | Scope | Questions |
|------|----------|-------|-----------|
| **Flaky test audit** | P1 | All tests | Are there tests with timing dependencies, network calls, or shared state? |
| **Test isolation** | P1 | Integration tests | Do tests clean up after themselves? Can they run in parallel? |
| **Negative testing** | P1 | Security-critical paths | Are error cases tested? Malicious inputs? Boundary conditions? |
| **Benchmark validity** | P2 | `benches/` | Are benchmarks stable? Do they measure the right things? |

---

## 8. Audit Domain 6: Documentation

### 8.1 Code Documentation

| Item | Priority | Scope | Questions |
|------|----------|-------|-----------|
| **Public API docs** | P1 | All `pub` items in `src/lib.rs` exports | Do all public types, traits, and functions have doc comments? |
| **Module-level docs** | P1 | All `mod.rs` files | Does each module have a doc comment explaining its purpose, invariants, and relationship to other modules? |
| **Safety comments** | P0 | All `unsafe` blocks | Does every `unsafe` block have a `// SAFETY:` comment explaining why it's sound? |
| **Invariant documentation** | P1 | Complex state machines | Are state machine invariants documented? Session states? Extension lifecycle states? |

### 8.2 Project Documentation

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **README accuracy** | P1 | `README.md` | Does the README match current capabilities? Quick start still works? |
| **CLAUDE.md accuracy** | P1 | `CLAUDE.md` | Does the development guide match current repo shape? |
| **Canonical docs currency** | P1 | All files in `docs/` | Are canonical docs up to date with the code? Any stale references? |
| **FEATURE_PARITY.md accuracy** | P0 | `FEATURE_PARITY.md` | Does every ✅ entry actually work? Are any 🚧 entries actually complete? |
| **CHANGELOG completeness** | P2 | `CHANGELOG.md` | Does the changelog cover all significant changes? Version numbering consistent? |
| **.env.example completeness** | P1 | `.env.example` | Are all env vars documented? Defaults safe? Any secrets in the example? |
| **Cross-reference integrity** | P2 | All docs with `[link](path)` | Do all internal doc links resolve? Any broken references? |

### 8.3 Architecture Documentation

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Agent_flow.md accuracy** | P1 | `Agent_flow.md` | Does the flow diagram match the actual boot and runtime sequence? |
| **agent_flows_architecture.md** | P1 | `agent_flows_architecture.md` | Is this current or stale? |
| **Network security model** | P0 | `src/NETWORK_SECURITY.md` | Does the documented trust model match the implementation? |
| **Stale audit docs** | P2 | `docs-audit/`, `rewrite-docs/`, `agent_skill_system_audit.md` | Are these historical artifacts or active docs? Should any be cleaned up? |

---

## 9. Audit Domain 7: CI/CD & DevOps

### 9.1 CI Pipeline

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **CI workflow completeness** | P0 | `.github/workflows/ci.yml` | Does CI run clippy, fmt, test, and build for all feature combinations? |
| **Feature matrix testing** | P0 | CI config | Are `light`, `desktop`, `full`, individual features tested in CI? **Current gap:** CI only runs `--all-features` and default features. Per-profile builds (`light`, `desktop`, `full`, `--no-default-features`) are not tested — a compilation error in the default `light` profile could ship undetected. |
| **Per-feature cargo check** | P0 | CI config | Add matrix steps: `cargo check --features light`, `cargo check --features desktop`, `cargo check --features full`, `cargo check --no-default-features`. Also lint with `cargo clippy --all-targets --all-features`. |
| **Security scanning** | P1 | CI config | Is `cargo-audit` or `cargo-deny` in CI? OSV scanning? |
| **Fuzz CI** | P2 | `.github/workflows/fuzz.yml` | Is fuzzing running regularly? What targets? |

### 9.2 Release Pipeline

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Release workflow** | P1 | `.github/workflows/release.yml` | Cross-compilation targets correct? Signing configured? |
| **Installer generation** | P1 | `release.yml`, `cargo-dist` config | Shell/PowerShell/NPM/MSI installers tested? |
| **Version management** | P1 | `Cargo.toml`, `release-plz.toml` | Automated version bumps? Changelog generation? |

### 9.3 Build System

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Build script** | P1 | `build.rs` (14K LOC) | What does the build script do? Is it necessary? Can it fail silently? |
| **Compile time** | P2 | `Cargo.toml` dependencies | Are there heavy proc-macro deps? Can compile time be reduced? |
| **Binary size** | P2 | Release profile | LTO settings appropriate? Debug info stripped? |
| **Feature flag hygiene** | P1 | `Cargo.toml` features section | Are feature definitions minimal? Any unintentional feature unification? |
| **Mixed TLS stacks** | P1 | `Cargo.toml` TLS deps | `reqwest` uses `rustls` while `tokio-tungstenite` uses `native-tls` — two TLS implementations in the same binary. See `tls_unification_plan.md` for one-line fix. |

---

## 10. Audit Domain 8: Dependency Management

### 10.1 Dependency Health

| Item | Priority | Tool | Questions |
|------|----------|------|-----------|
| **Known vulnerabilities** | P0 | `cargo audit` | Any CVEs in the dependency tree? |
| **License compliance** | P0 | `cargo deny check licenses` | Any copyleft dependencies? Any license conflicts with MIT/Apache-2.0? |
| **Outdated dependencies** | P1 | `cargo outdated` | How many deps are behind? Any with breaking updates pending? |
| **Unused dependencies** | P1 | `cargo +nightly udeps` | Dependencies declared but never imported? |
| **Dependency tree depth** | P2 | `cargo tree` | Excessive transitive dependencies? Duplicate versions? |

### 10.2 Critical Dependency Audit

| Dependency | Version | Risk | Questions |
|------------|---------|------|-----------|
| `wasmtime` | 36 | High — sandbox boundary | Is this the latest stable? Any known sandbox escapes? |
| `rig-core` | 0.30 | High — LLM abstraction | Pre-1.0 — breaking changes expected? API surface stable? |
| `chromiumoxide` | 0.9.1 | Medium — browser automation | Is it maintained? Security patches? |
| `nostr-sdk` | 0.44.1 | Medium — crypto protocol | NIP compliance? Audit status? |
| `reqwest` | 0.12 | Medium — HTTP client | rustls configured correctly? Certificate validation? |
| `tokio-tungstenite` | 0.26 | Medium — WS + native-tls | Why native-tls instead of rustls? Mixed TLS stacks? |
| `libsql` | 0.6 | Medium — embedded DB | Stability? Replication reliability? |

---

## 11. Audit Domain 9: Database & Migrations

### 11.1 Schema Integrity

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Migration ordering** | P0 | `migrations/V1__initial.sql` through `V17__*.sql` | Are migrations idempotent? Can they be replayed safely? |
| **Schema consistency** | P0 | `src/db/mod.rs`, `src/db/postgres.rs`, `src/db/libsql_migrations.rs` | Do PostgreSQL and libSQL schemas diverge? Are they tested equivalently? **Current gap:** PostgreSQL backend has 0 tests; libSQL has 19. No shared contract tests exist. See `database_divergence_plan.md` for full remediation plan. |
| **Backend contract tests** | P0 | `src/db/tests/` (proposed) | All 11 sub-traits (~130 methods) must have shared contract tests that run against both backends. See `database_divergence_plan.md`. |
| **Foreign key constraints** | P1 | Migration files | Are all relationships properly constrained? Orphan row risks? |
| **Index coverage** | P1 | Migration files | Are frequently queried columns indexed? FTS indexes present and maintained? |
| **Data integrity** | P1 | `src/db/` | Are transactions used for multi-table writes? Can writes leave inconsistent state? |

### 11.2 Query Safety

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **SQL injection surface** | P0 | All SQL string construction | Any `format!()` with user input in SQL? All parameters bound? |
| **Query performance** | P2 | Complex queries in `src/db/` | Any N+1 queries? Missing pagination? Unbounded `SELECT *`? |
| **Connection handling** | P1 | `deadpool-postgres` usage, `libsql` usage | Are connections returned to pool on error? Timeout configuration? |

### 11.3 Migration Robustness

| Item | Priority | Scope | Questions |
|------|----------|-------|-----------|
| **Rollback strategy** | P1 | All migrations | Can each migration be rolled back? Are down migrations provided? |
| **Zero-downtime migration** | P2 | Schema-altering migrations | Can migrations run while the agent is serving? |
| **Migration testing** | P1 | CI/test | Are migrations tested against both PostgreSQL and libSQL? |

---

## 12. Audit Domain 10: Feature Completeness & Consistency

### 12.1 FEATURE_PARITY.md Verification

| Item | Priority | Scope | Questions |
|------|----------|-------|-----------|
| **Status accuracy** | P0 | All ✅ entries | Spot-check 20-30 random ✅ entries — does the code actually implement them? |
| **🚧 entries assessment** | P1 | All 🚧 entries | What's the actual completion percentage? Are any stale? |
| **Cross-reference integrity** | P1 | File path references | Do all `[file](path)` links in FEATURE_PARITY.md resolve? |

### 12.2 Upgrade Roadmap Alignment

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **thinclaw_upgrade.md status** | P1 | `thinclaw_upgrade.md`, source code | Which of the 19 upgrade vectors have been partially or fully implemented? Does FEATURE_PARITY.md reflect the current state? |
| **Phase 0 completion** | P1 | Prompt ownership, command routing, settings ownership | Were the Phase 0 integration-prep deliverables completed? |

### 12.3 Feature Consistency

| Item | Priority | Scope | Questions |
|------|----------|-------|-----------|
| **CLI ↔ WebUI ↔ TUI parity** | P1 | Features exposed across interfaces | Are there features available in CLI but not WebUI, or vice versa? |
| **Settings ↔ .env.example ↔ docs alignment** | P1 | Configuration surfaces | Are all settings documented in all three places? |
| **Slash command completeness** | P2 | `src/agent/commands.rs`, `src/agent/submission.rs` | Are all slash commands parsed, documented, and have help text? |

---

## 13. Audit Domain 11: Operational Readiness

### 13.1 Error Handling & Recovery

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Panic audit** | P0 | Full codebase (`unwrap`, `expect`, `panic!`, `unreachable!`) | Are there panics on recoverable errors? In async contexts? In library code? |
| **Error context** | P1 | `anyhow::Context` usage | Do errors carry enough context for debugging? Or just "connection failed"? |
| **Graceful degradation** | P1 | Provider failover, channel reconnect, optional features | Does the system degrade gracefully when components fail? |
| **Self-repair mechanisms** | P1 | `src/agent/self_repair.rs` | What does self-repair cover? Is it safe? Can it make things worse? |

### 13.2 Observability

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Structured logging** | P1 | `src/tracing_fmt.rs`, all `tracing::` calls | Is logging structured? Consistent span hierarchy? Appropriate log levels? |
| **Metrics** | P2 | Cost tracking, token usage, latency | Are metrics collected? Exportable? |
| **Health endpoints** | P1 | `/api/health`, `/api/gateway/status` | Do health checks cover all dependencies (DB, LLM, channels)? |
| **Error reporting** | P2 | Error aggregation | Are errors surfaced to the operator? Repeated errors rate-limited? |

### 13.3 Deployment Safety

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Docker configuration** | P1 | `Dockerfile`, `Dockerfile.worker`, `docker-compose.yml` | Are images minimal? Non-root user? Health checks? |
| **Service integration** | P1 | `src/service.rs` | launchd/systemd integration robust? PID file cleanup? |
| **Upgrade path** | P1 | `src/cli/update.rs`, `src/update_checker.rs` | Is the update flow safe? Rollback works? |
| **Backup/restore** | P2 | Database, workspace, config | Is there a documented backup procedure? Can the system be restored from backup? |

---

## 14. Audit Domain 12: WASM & Extension Ecosystem

### 14.1 WASM Runtime

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Sandbox boundaries** | P0 | `src/sandbox/`, wasmtime config | Are memory limits enforced? CPU time limits? Fuel metering? |
| **WIT interface surface** | P1 | `wit/` directory | Is the WIT interface minimal? Any overly broad capability grants? |
| **WASM validation** | P1 | `wasmparser` usage | Are uploaded WASM modules validated before instantiation? |
| **Hot-reload safety** | P1 | `src/channels/wasm/channel_watcher.rs` | Can a hot-reload leave the system in an inconsistent state? |

### 14.2 Extension Security

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **Manifest validation** | P1 | `src/extensions/manifest_validator.rs` | What does manifest validation check? Can it be bypassed? |
| **Capability escalation** | P0 | `src/extensions/manager.rs` | Can an extension request capabilities it wasn't granted? |
| **Extension isolation** | P1 | Extension manager | Can one extension interfere with another? |
| **ClawHub security** | P1 | `src/extensions/clawhub.rs` | Is the registry trusted? Signature verification? |

### 14.3 MCP Integration

| Item | Priority | Key Files | Questions |
|------|----------|-----------|-----------|
| **OAuth 2.1 implementation** | P1 | `src/tools/mcp/` | Is the OAuth flow standards-compliant? Token storage secure? |
| **Transport security** | P1 | stdio + HTTP transport | Are HTTP transports TLS-only? stdio isolation? |
| **Tool result sanitization** | P0 | MCP response handling | Are MCP tool results treated as untrusted input? Sanitized before use? |

---

## 15. Execution Plan

### Phase 1: Automated Scans (Day 1)

Run all automated tooling first to establish a baseline:

```bash
# 1. Compile check all feature combinations
cargo check --features light
cargo check --features desktop
cargo check --features full
cargo check --no-default-features

# 2. Lint
cargo clippy --all-targets --all-features -- -D warnings

# 3. Format
cargo fmt --check

# 4. Tests
cargo test --all-features

# 5. Security scan
cargo audit
cargo deny check

# 6. Dead code / unnecessary deps
# cargo +nightly udeps --all-features  # requires nightly

# 7. Code metrics
find src -name "*.rs" -exec wc -l {} + | sort -rn | head -30
grep -rn "unwrap()" src/ --include="*.rs" | wc -l
grep -rn "expect(" src/ --include="*.rs" | wc -l
grep -rn "panic!" src/ --include="*.rs" | wc -l
grep -rn "todo!()" src/ --include="*.rs" | wc -l
grep -rn "unimplemented!()" src/ --include="*.rs" | wc -l
grep -rn "unsafe" src/ --include="*.rs" | wc -l
grep -rn "TODO\|FIXME\|HACK\|XXX" src/ --include="*.rs" | wc -l
grep -rn "#\[allow(dead_code)\]" src/ --include="*.rs" | wc -l
```

### Phase 2: Security Deep Dive (Days 2–3)

Manual review of all P0 security items:

1. Shell command injection paths
2. SQL injection surface
3. Path traversal in file tools
4. SSRF in HTTP tools
5. WASM sandbox configuration
6. Secret exposure in logs and prompts
7. Authentication bypass paths
8. MCP response sanitization

### Phase 3: Architecture & Design Review (Days 4–5)

1. Module dependency graph validation
2. Prompt assembly ownership audit
3. State management review
4. Concurrency and lock analysis
5. Error propagation model review
6. **`server.rs` decomposition** — Execute `server_decomposition_plan.md` (7,923 LOC → ~16 modules)
7. **Database backend divergence** — Execute `database_divergence_plan.md` (shared contract tests)
8. **TLS stack unification** — Execute `tls_unification_plan.md` (one-line Cargo.toml change)

### Phase 4: Quality & Testing (Days 6–7)

1. Test coverage gap analysis
2. Critical path coverage verification
3. Test quality and flakiness audit
4. Dead code and unused dependency cleanup
5. Code smell inventory

### Phase 5: Documentation & Ops (Day 8)

1. Documentation accuracy spot-checks
2. FEATURE_PARITY.md verification
3. CI/CD pipeline review
4. Operational readiness assessment
5. Dependency health check

### Phase 6: Synthesis & Reporting (Days 9–10)

1. Consolidate all findings
2. Prioritize issues (P0 → P3)
3. Create actionable remediation plan
4. Update FEATURE_PARITY.md if needed
5. File tracking issues

---

## 16. Deliverables

| Deliverable | Format | Audience |
|-------------|--------|----------|
| **Audit findings report** | Markdown | Developer |
| **Security findings** | Markdown (restricted) | Developer / Security |
| **Remediation backlog** | GitHub Issues or tracking doc | Developer |
| **Updated FEATURE_PARITY.md** | In-repo update | All |
| **Test coverage gap map** | Markdown table | Developer |
| **Dependency health report** | `cargo audit` + `cargo deny` output | Developer |

### Finding Severity Scale

| Severity | Definition | SLA |
|----------|-----------|-----|
| **P0 — Critical** | Security vulnerability, data loss risk, or architectural flaw that blocks production use | Fix before next release |
| **P1 — High** | Significant quality issue, missing test coverage on critical path, or operational risk | Fix within 2 weeks |
| **P2 — Medium** | Code smell, documentation gap, or minor inconsistency | Fix within 1 month |
| **P3 — Low** | Cosmetic issue, optimization opportunity, or nice-to-have improvement | Backlog |

---

## 17. Known Healthy — Verified Strengths

> [!TIP]
> These areas have been manually verified and confirmed sound. Auditors can skip deep-diving these unless a specific concern arises — focus effort on the open items above.

| Area | Verification Date | Verdict | Evidence |
|------|-------------------|---------|----------|
| **Error hierarchy** | 2026-04-14 | ✅ Excellent | `src/error.rs` (438 LOC): Clean `thiserror` enum hierarchy, structured context in every variant, feature-gated variants. Zero stringly-typed errors. |
| **Builder initialization** | 2026-04-14 | ✅ Excellent | `src/app.rs` `AppBuilder`: 5-phase init (`database → secrets → llm → tools → extensions`), each phase independently testable, test harness can construct components without channels. |
| **Trait abstractions** | 2026-04-14 | ✅ Strong | `Tool` trait (rich defaults), `Channel` trait (minimal required surface), `Database` supertrait (11 sub-traits). All trait impls complete — zero `todo!()` or `unimplemented!()`. |
| **Feature gate architecture** | 2026-04-14 | ✅ Good | 13 features (`light`, `desktop`, `full`, etc.). `lib.rs` clean and organized. Feature-gated modules verified during structural audit. |
| **Code health metrics** | 2026-04-14 | ✅ Strong | 0 `todo!()`, 0 `unimplemented!()`, 1 `TODO/FIXME/HACK`, 0 unwraps in dispatcher, 64 `unsafe` blocks (all in WASM FFI + shell test fixtures). |
| **Identity bridge pattern** | 2026-04-14 | ✅ Sound | `IdentityStore` (string-based) ↔ `IdentityRegistryStore` (UUID-based) with blanket impl bridge. Clean dual-trait pattern for CLI vs internal consumers. |
| **Cost guard pre-check** | 2026-04-14 | ✅ Sound | Checked before each LLM call in the agentic loop. Seeds daily totals from `CostTracker` DB at boot. |
| **Memory flush pattern** | 2026-04-14 | ✅ Sound | Pre-compaction flush at 80% context cap. Reset logic correctly fires *after* hard cap truncation (Bug 9 fix verified). |
| **Stuck loop detection** | 2026-04-14 | ✅ Sound | Tracks consecutive identical tool calls with warn threshold (3) and force threshold (5). Injects finalization prompt on stuck detection. |
| **CI pipeline** | 2026-04-14 | ✅ Solid | `fmt + clippy + cargo-deny` on every PR, postgres service container, coverage via `cargo-llvm-cov`, Codecov upload, concurrency control. **Gap:** Missing per-feature build matrix. |

---

## 18. Remediation Plans

Detailed, end-to-end execution plans for the three highest-priority architectural issues:

| Plan | File | Priority | Estimated Effort | Summary |
|------|------|----------|------------------|---------|
| **server.rs decomposition** | `server_decomposition_plan.md` | P1 | 8–11 hours | Split 7,923 LOC / 123 handlers into ~16 focused modules. 4-phase extraction with zero behavior change. |
| **Database backend divergence** | `database_divergence_plan.md` | P1 | 19–26 hours | Shared contract test suite (70+ tests) for all 11 sub-traits across both backends. Schema divergence audit. CI integration. |
| **Mixed TLS stack** | `tls_unification_plan.md` | P2 | 30 minutes | One-line `Cargo.toml` change: switch `tokio-tungstenite` from `native-tls` to `rustls-tls-native-roots`. |

---

## 19. Prior Audit Work

The following past work should be ingested at the start of the audit to avoid duplication:

| Prior Work | Date | Scope | Status |
|------------|------|-------|--------|
| ThinClaw Codebase Structural Audit | 2026-04-13 | Structural flaws, feature gating, stray files | Completed — syntax errors, empty dirs, feature gates fixed |
| ThinClaw Codebase Audit Verification | 2026-04-13 | Follow-up verification of structural audit | Completed — tunnel feature gate, migration runner, trait abstraction verified |
| Hermes Agent Tool Parity Audit | 2026-04-13 | Tool feature gaps vs Hermes | Completed — findings in `thinclaw_upgrade.md` |
| Agent Skill System Audit | 2026-04-07 | Skill system architecture | Completed — findings in `agent_skill_system_audit.md` |
| Secret Leak Investigation | 2026-04-12 | `leak_detector.rs` false positives | Completed — high_entropy_hex patterns confirmed as benign |
| Documentation Suite Audit | 2026-04-12 | README, CLAUDE.md, .env.example | Completed — rebranding cleanup done |
| Architecture Assessment | 2026-04-14 | Codebase soundness, maintainability, code health metrics | Completed — B+/A- grade, 3 remediation plans created |

> [!IMPORTANT]
> This audit plan is designed to be executed incrementally. Start with Phase 1 (automated scans) and Phase 2 (security deep dive) as they have the highest ROI and shortest time-to-value. Phases 3–5 can be parallelized across multiple reviewers if available.
