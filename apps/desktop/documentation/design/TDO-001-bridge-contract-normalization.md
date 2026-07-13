# Design — Bridge & Command-Surface Normalization (TDO-001…006)

> **Status:** TDO-001–006 implemented · **Created:** 2026-06-27 · Epic: TDO-EP1 (WS-1)
> **Parent:** [`../OVERHAUL_PLAN.md`](../OVERHAUL_PLAN.md) §4 WS-1 · **Backlog:** [`../OVERHAUL_BACKLOG.md`](../OVERHAUL_BACKLOG.md)
> This is the foundation issue — TDO-EP2/EP3 and most of Phase 1 depend on it.

## 1. Problem (grounded in code)

The desktop↔runtime bridge is **three artifacts that can silently drift**:

1. **~337 `#[tauri::command]`s** registered in `apps/desktop/backend/src/setup/commands.rs` (`tauri_specta::collect_commands!`).
2. The hand-written **`apps/desktop/frontend/src/lib/thinclaw.ts`** (2,534 LoC) of `invoke<T>(cmd, args)` wrappers.
3. The **generated `apps/desktop/frontend/src/lib/bindings.ts`** (`commands.*`, 317 fns) from `tauri-specta`.

Three concrete defects:

- **Two calling conventions coexist.** Components call both `thinclaw.foo()` (hand-written) and `commands.foo()` (generated). The hand-written layer can reference a command that was renamed/removed, and nothing fails until runtime.
- **Inconsistent "unavailable" handling.** `rpc_jobs_autonomy.rs:8` returns `Err(local_unavailable(...))` (an error string); `rpc_experiments_learning.rs:394` returns `Ok(unavailable(...))` (a success JSON with `available:false`). The frontend cannot reliably distinguish "gated, here's why" from "failed". This is the root of the *silent-unavailable* parity class.
- **Ad-hoc event handling.** ~30 `UiEvent` variants (`ui_types.rs:18`, `#[serde(tag="kind")]`) are consumed by scattered `listen('thinclaw-event')` calls (e.g. `SubAgentPanel.tsx`, `ThinClawChatView.tsx`), each re-deriving the discriminated union by hand.

## 2. Goals / non-goals

**Goals**
- One enforceable contract: every command has {a registered handler, a typed binding, a declared route behavior, a typed gated-reason}.
- A CI **bridge linter** that fails on drift.
- Code-generated `remote-gateway-route-matrix.md` and a single typed `UiEvent` union.
- Collapse to one calling convention without a flag-day rewrite.

**Non-goals**
- Changing what commands *do* (behavior changes live in Phase-1 issues).
- Touching the remote gateway's HTTP API shape.

## 3. The `RouteBehavior` model (TDO-001)

Introduce a typed, normalized outcome for gating, replacing the ad-hoc `String`/JSON split.

```rust
// apps/desktop/backend/src/thinclaw/bridge/route.rs  (new)

/// How a command behaves across the dual-mode runtime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, specta::Type)]
pub enum RouteMode {
    /// Works in both embedded and remote-gateway mode.
    LocalAndRemote,
    /// Only meaningful against a remote gateway (e.g. sandbox job restart, GPU launch).
    RemoteOnly,
    /// Only meaningful in embedded mode (e.g. local sidecar control).
    LocalOnly,
}

/// A machine-readable "this is gated, here's why + how to fix" outcome.
#[derive(Clone, Debug, serde::Serialize, specta::Type, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BridgeError {
    #[error("unavailable: {capability}: {reason}")]
    Unavailable {
        capability: String,
        reason: String,
        /// What the user must do (e.g. "connect a remote gateway", "enable reckless autonomy").
        remediation: Option<String>,
        /// Which mode WOULD satisfy it, for UI affordances.
        satisfied_by: RouteMode,
    },
    #[error("{0}")]
    Runtime(String), // genuine errors (kept distinct from gated state)
}
```

Each command declares its mode in a **central registry** (single source the linter and
route-matrix generator both read), avoiding a per-command attribute macro:

```rust
// apps/desktop/backend/src/thinclaw/bridge/registry.rs (new)
pub static ROUTE_TABLE: &[(&str, RouteMode)] = &[
    ("thinclaw_send_message",          RouteMode::LocalAndRemote),
    ("thinclaw_job_restart",           RouteMode::RemoteOnly),     // was Err string
    ("thinclaw_experiments_gpu_launch_test", RouteMode::RemoteOnly),
    ("thinclaw_learning_evaluate_outcomes",  RouteMode::RemoteOnly),
    ("direct_runtime_start_chat_server",     RouteMode::LocalOnly),
    // … every command exactly once
];
```

**Gating helper** replaces `local_unavailable` (`rpc_jobs_autonomy.rs:8`) and the
`Ok(unavailable(...))` JSON path uniformly — gated states become `Err(BridgeError::Unavailable)`,
so `Result<T, BridgeError>` is the single shape. Frontend treats `Unavailable` as a
first-class UI state (CTA), not a thrown error.

```rust
pub fn gated(capability: &str, reason: &str, remediation: &str, by: RouteMode) -> BridgeError {
    BridgeError::Unavailable { capability: capability.into(), reason: reason.into(),
        remediation: Some(remediation.into()), satisfied_by: by }
}
```

> **Migration note (breaking, allowed):** command return types move from
> `Result<T, String>` to `Result<T, BridgeError>`. Provide a `From<String> for BridgeError`
> (`Runtime`) so existing `?`/`.map_err(|e| e.to_string())` sites compile during transition,
> then sweep gated sites to `gated(...)`. Regenerate bindings.

## 4. Bridge linter (TDO-002)

Extend the existing contract test (`setup/commands.rs`, `generated_bindings_cover_phase_two_desktop_surfaces`) into a generative check:

```
For each command C registered in collect_commands!:
  assert C ∈ ROUTE_TABLE                       (mode declared exactly once)
  assert camelCase(C) present in bindings.ts   (binding generated)
  if ROUTE_TABLE[C] == RemoteOnly|LocalOnly:
      assert the handler body reaches gated(...) on the unsupported branch
        (static check: grep handler for `gated(` OR a `#[route_checked]` marker)
assert ROUTE_TABLE has no command absent from collect_commands! (no stale rows)
assert bindings.ts has no command absent from collect_commands! (no orphan bindings)
```

Failing any clause fails CI. This is the gate that makes the contract *enforced* rather
than documented.

## 5. Code-generated route matrix + UiEvent union (TDO-003, TDO-005)

`tauri-specta` already runs in `examples/export_bindings.rs` to emit `bindings.ts`. Extend
that export step to also emit:

- **`remote-gateway-route-matrix.md`** — rendered from `ROUTE_TABLE` (command | mode | remediation). Replaces the hand-maintained doc; a test asserts the committed file matches generated output (like the `bindings.ts` "must stay generated" assertion).
- **Generated `UiEvent` in `bindings.ts`** — specta derives the discriminated union
  from `ui_types.rs`. A single native listener owns the Tauri channel and fans
  typed events out to React subscribers:

```ts
// frontend/src/hooks/use-thinclaw-stream.ts  (consolidate)
const subscribers = new Set<(event: UiEvent) => void>();
listen<UiEvent>('thinclaw-event', ({ payload }) => {
  subscribers.forEach((subscriber) => subscriber(payload));
});

export function useThinClawEvents(onEvent: (event: UiEvent) => void) { /* subscribe */ }
```

All 11 subscriptions across 10 component files now switch on the generated
`event.kind`; a contract test rejects any new panel-local native listener.

## 6. One calling convention: `lib/thinclaw.ts` → `bindings.ts` (TDO-004, TDO-006)

Strangler migration (non-flag-day):

1. Make `bindings.ts` (`commands.*`) the source of truth. Keep the rich **types** that live in `thinclaw.ts` by moving them to `lib/api/types.ts` (or letting specta own them).
2. Convert `thinclaw.ts` wrappers into thin re-exports: `export const getLearningStatus = (n:number) => commands.thinclawLearningStatus(n)`. Zero component churn.
3. Split the re-export shim by domain into `lib/api/{sessions,memory,routines,learning,experiments,mcp,channels,…}.ts` (this is TDO-020 / WS-3).
4. Codemod components from `thinclaw.foo()` → `api.foo()` per domain; delete each shim once unused.
5. Retire root `src/tauri_commands.rs` facade (TDO-006): reusable service
   helpers now live in `src/desktop_api.rs`; Desktop command registration stays
   in the typed command modules. `lib.rs` retains only a deprecated source-level
   alias for downstream compatibility, and the unused pre-rename command-name
   inventory is gone.

## 7. Rollout sequence & risk

| Step | Output | Breaking? |
|---|---|---|
| `BridgeError` + `From<String>` shim | compiles, no behavior change | no |
| Sweep gated sites to `gated(...)` | consistent gating | runtime-visible (better) |
| `ROUTE_TABLE` + linter | CI gate | dev-only |
| Generate route-matrix + UiEvent union | docs/types from code | no |
| Re-export shim + domain split | one convention | no (shims) |
| Codemod + delete shims | clean surface | source-only |

**Risks:** (a) `Result<_, BridgeError>` migration is wide — mitigate with the `From<String>`
shim so it's mechanical and incremental; (b) specta type-export for `BridgeError`/`UiEvent`
must round-trip — add a sanitizer test (the harness already checks `Channel<T>`/reserved args).

**Definition of done:** linter green; route-matrix + UiEvent union generated and asserted;
`thinclaw.ts` reduced to re-exports; zero `Ok(unavailable(...))`-style ambiguity remains.
