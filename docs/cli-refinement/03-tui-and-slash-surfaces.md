# CR-03 — TUI and Slash Surfaces

- **Priority:** P0 for approvals/event identity; P1 for rendering/layout
- **Depends on:** CR-01 safety/output, CR-02 capability snapshot and durable conversations
- **Blocks:** CR-04 final help/completion migration

## Scope and ownership

Primary targets:

- `src/tui/mod.rs` and focused extracted TUI modules
- `crates/thinclaw-channels/src/tui.rs`
- `crates/thinclaw-channels-core/src/channel.rs` (`StatusUpdate`)
- all tool-status producers and transport adapters found with `rg 'StatusUpdate::Tool'`
- `crates/thinclaw-agent/src/{command_registry,command_catalog,submission}.rs`
- `crates/thinclaw-channels/src/repl.rs`
- TUI startup construction across `crates/thinclaw-channels/src/tui.rs::TuiRuntime::start`, `src/channels/tui_channel.rs::{RootTuiRuntime,TuiChannel::new}`, and `src/async_main.rs`

## CR-03.1 — Preserve tool invocation identity end to end

**Covers:** INV-52, INV-53.

Current defect: `StatusUpdate` tool events have names/arguments but no stable invocation ID; the TUI adapter drops arguments; results mutate only the most recent message. Concurrent/interleaved tools can therefore display the wrong result.

Implement:

1. Add a `ToolInvocationId` newtype (string/UUID-compatible, serializable) in the lowest shared type crate.
2. Extend `StatusUpdate::ToolStarted`, `ToolResult`, and `ToolCompleted` with the same `invocation_id`. Allocate it once at the tool-execution boundary before the start event and reuse it for every result/completion event. If a provider supplies a tool-call ID, preserve it through a namespaced/validated representation rather than generating a second identity.
3. Extend every producer/consumer found by `rg 'StatusUpdate::(ToolStarted|ToolResult|ToolCompleted)'`: portable status payloads, SSE/client/OpenAPI wire types, desktop contracts, ACP/WASM/native adapters, fixtures, and tests. Serialized transports add the field compatibly with an explicit schema-version/update note; no adapter may synthesize a different ID.
4. Replace lossy `TuiUpdate` variants with:

   ```text
   ToolStarted { invocation_id, name, parameters, started_at }
   ToolOutput { invocation_id, preview, artifacts }
   ToolCompleted { invocation_id, name, success, result_preview, duration_ms }
   ```

5. TUI state stores an ordered activity list plus a map keyed by `ToolInvocationId`. Completion updates the matching entry, never “last message.” Unknown completion IDs create a diagnostic orphan record rather than corrupting another tool.
6. Keep completed tool records visible with success/failure, duration, arguments summary, result preview, and artifact count. Redact first, then cap each parameter/result preview at 8 KiB through one typed presentation helper; mark `truncated=true` and retain an artifact/detail handle when available.

Acceptance tests:

- start A, start B, result A, complete B, complete A produces two correct cards;
- duplicate/replayed completion is idempotent;
- unknown ID is visible as a diagnostic and does not panic;
- parameters and previews are redacted/bounded before rendering;
- successful completion remains visible and does not collapse to generic `Ready`.

## CR-03.2 — Make approvals request-ID-bound and explicit

**Covers:** INV-51.

Current defect: the adapter discards `request_id` and `parameters`; UI state is a boolean; text input can dismiss the visible prompt while the runtime remains pending.

Implement:

1. Preserve all fields from `StatusUpdate::ApprovalNeeded` in:

   ```text
   ApprovalPrompt {
       request_id,
       tool_name,
       description,
       redacted_parameters,
       received_at,
   }
   ```

2. Replace `pending_approval: bool` with an ordered collection keyed by request ID. If runtime guarantees only one pending approval, still model the identity explicitly and reject a second conflicting ID with a visible diagnostic.
3. Add `TuiEvent::ApprovalResponse { request_id, decision }`, where decision is `ApproveOnce`, `ApproveForSession`, or `Deny`. Map it to the existing explicit `Submission::ExecApproval { request_id, approved, always }`; do not use implicit “current pending” `ApprovalResponse` for TUI.
4. Render a focused approval modal/card containing tool, description, redacted arguments, and exact keys/buttons. Default focus is deny/cancel. Session approval states its scope and uses the existing `always` semantics only for this tool/session.
5. Free-form text while approval is pending does not clear, approve, deny, or replace the prompt. Either keep it in the composer or submit it as an ordinary queued user message according to the agent-loop policy; the approval stays visible until a matching terminal decision/event.
6. Reject response IDs that do not match a pending request and log/display the mismatch without forwarding it.
7. Interrupt/exit explicitly deny or cancel pending approvals according to runtime policy before shutdown; never strand an approval waiter.

Acceptance tests:

- prompt retains ID/tool/description/parameters;
- approve once maps `always=false`; session maps `always=true`; deny maps `approved=false`;
- unrelated text and navigation keys do not dismiss the prompt;
- wrong/stale ID is not forwarded;
- queued approvals are resolved in explicit order;
- token/password-shaped parameter values are redacted.

## CR-03.3 — Preserve structured runtime activity

**Covers:** INV-54, INV-56.

Implement:

1. Stop converting plan, usage, context pressure, compaction, advisor, self-repair, lifecycle, and canvas updates into one `Status(String)` lane.
2. Add typed `TuiUpdate` variants mirroring structured `StatusUpdate` fields. Empty/unknown variants must not blank status.
3. Preserve subagent ID/name/task on spawn, progress, and completion. The current progress adapter must not emit `name: String::new()`.
4. State layout:
   - `transient_status`: the current short-lived operation;
   - `activities`: persistent keyed tool/subagent/job/auth/self-repair records;
   - `plan`: current structured plan entries;
   - `usage`: token/model/cost totals and last turn;
   - `context`: capacity/compaction state;
   - `canvas`: typed panel state;
   - `diagnostics`: bounded errors/warnings.
5. Apply monotonic sequence IDs where provided. Ignore stale out-of-order state replacements while retaining append-only activity events. Retain at most 500 completed activities and 200 diagnostics in memory; pending approvals and nonterminal activities are never evicted. Eviction is oldest-completed-first and visible in diagnostics/metrics, not silent.

Acceptance tests:

- a usage update cannot erase a plan or error;
- subagent progress updates the correct named/id card under interleaving;
- empty lifecycle/status events do not blank the UI;
- context and cost values survive later tool updates;
- bounded activity/diagnostic retention does not grow without limit.

## CR-03.4 — Hydrate authoritative startup and conversation state

**Covers:** INV-17, INV-55, INV-59, INV-61, INV-66.

Implement:

1. Add a `TuiBootstrap` value assembled by runtime after configuration and registration:
   - selected agent ID/name;
   - resolved model/provider;
   - conversation/thread identity;
   - workspace/profile;
   - `CapabilitySnapshot`;
   - paginated initial conversation messages;
   - usage/context values when known;
   - credential-free gateway origin/state.
2. Pass it into TUI construction. Remove hard-coded `default` model and `main` agent labels.
3. Produce actual model-change updates at the model-switch/route-change boundary, including model/provider and effective-from turn. The UI updates assistant-message attribution using the model active for that message, not a global label retroactively.
4. Hydrate the latest 100 durable messages through `ConversationStore`; load older pages in batches of 100 on upward scroll with a stable cursor and a server/store-enforced maximum request of 500. Preserve roles, timestamps, tool activity references, and conversation identity.
5. Reconnect/resume continues the same durable conversation. A new conversation is an explicit command/action.
6. TUI `/status` uses the shared runtime/capability report, not locally synthesized placeholders.
7. Never include a gateway token in bootstrap state or copyable display text.
8. Reuse the runtime's identity-scoped primary direct-conversation selection used during agent hydration. On default TUI startup, continue that primary conversation and page its latest messages; `/new` explicitly creates/switches to a new durable conversation. Do not introduce a second TUI-only thread index or silently choose an arbitrary recent conversation.
9. Thread `TuiBootstrap` through the concrete constructor chain named in Scope and ownership. Change the trait signature and all fixtures together so a default/empty bootstrap cannot be substituted at an adapter boundary.
10. Bootstrap carries the sealed capability revision `N` and the TUI/REPL receive a read-only revision feed or snapshot handle from the runtime. Apply hot `N+1` replacements atomically; reject stale/replayed revisions and never merge independently sampled counts into an older descriptor set.
11. Make `/tools` a local presentation route on REPL and TUI over the latest complete `CapabilitySnapshot`, while the authorized agent-message route may remain a runtime query. Its compact default groups exact tool identities by origin and reports registered/exposed/ready counts with the revision; `/tools NAME` shows provenance, source, independent fact states, approval policy, safe reasons, and remediation. `/tools --all` includes unavailable catalog entries. Do not render only `ToolRegistry::list()` or a bare count.

Acceptance tests:

- selected nondefault agent/model appears at first render;
- model switch updates future attribution and leaves older messages correct;
- history pagination has no duplicate or reordered messages;
- stopped/disabled gateway is not shown as enabled;
- token fixtures are absent from serialized state, debug output, and rendered buffers.
- startup `/tools` identities exactly match sealed revision `N`; one hot activation replaces the view with `N+1`, while a replay of `N` is ignored;
- `/tools`, `/status`, terminal `status tools`, and boot fixtures agree on identities and facts at the same revision.

## CR-03.5 — Add safe Markdown and viewport-aware scrolling

**Covers:** INV-58.

Implement:

1. Extract transcript rendering from `src/tui/mod.rs` into focused modules such as `src/tui/render/{markdown,transcript}.rs` and `src/tui/state.rs`.
2. Use a maintained CommonMark parser added at workspace dependency level after license/advisory review. Render only terminal-safe semantics: paragraphs, emphasis, headings, lists, block quotes, links as label plus sanitized URL, inline code, and fenced code. Ignore raw HTML and terminal escape/control sequences.
3. Wrap based on display width using Unicode width rules, accounting for borders/padding. Never slice UTF-8 by byte offset.
4. Derive scrolling from rendered visual rows, not message count or source newlines. Preserve bottom-follow while streaming only if the user was already at bottom; manual upward scroll disables auto-follow until returned to bottom.
5. Cache rendered rows by message ID/content hash/width/theme and invalidate on content or viewport changes.
6. Bound code blocks and oversized unbroken tokens; provide horizontal-safe truncation or wrapping without terminal control injection.

Acceptance tests:

- golden buffers for headings, lists, code, Unicode, links, long tokens, and narrow/wide terminals;
- wrapped-row up/down/page/top/bottom behavior;
- stream append at bottom vs while scrolled up;
- malicious ANSI/OSC/raw HTML fixture cannot alter terminal state;
- resize preserves a valid logical anchor.

## CR-03.6 — Typed navigation and a cleaner chat-first layout

**Covers:** INV-60, INV-62.

Adopt this default layout:

```text
┌ agent · model · conversation · context ─ capability/degraded indicator ┐
│                                                                        │
│                         conversation transcript                         │
│              inline compact tool/subagent/job activity                  │
│                                                                        │
├ composer ──────────────────────────────────────────────────────────────┤
│ transient status · shortcuts                                           │
└────────────────────────────────────────────────────────────────────────┘
```

Rules:

1. Large logo/hero is off by default after startup. A skin may influence color and compact mark, not consume the conversation viewport.
2. Replace branded labels with plain task language: `Activity`, `Input`, `Details`, `Plan`, `Usage`, `Approval`.
3. Details/plan/help/approval are typed overlays or drawers represented by `ViewMode`, not synthetic chat messages.
4. `/back` and Ctrl+B close the active overlay/drawer; if none is open they do nothing and do not delete transcript/activity records.
5. Errors and approvals remain visible until acknowledged/resolved. Transient success messages expire without removing durable activity.
6. Narrow terminals collapse metadata deliberately; they do not duplicate headings or hide the composer.
7. Skin support remains, but skin metadata (“sigil”, prompt symbol, skin name) belongs in theme/settings help, not repeated runtime chrome.
8. Use fixed responsive breakpoints based on usable cell dimensions: wide `>=100` columns shows agent/model/conversation/context plus capability badge; compact `60–99` keeps one truncated identity line and the badge; narrow `40–59` keeps conversation plus degraded/approval indicator and makes drawers full-screen overlays. At `12–23` rows, collapse footer help and completed inline activity before shrinking transcript/composer; below `40x12`, render a deterministic resize view that still exposes pending approval deny/approve keys and quit. The composer is always at least three rows when the terminal meets `40x12`.
9. The transcript is the only permanently expanding pane. Show at most three active/recent activity summaries inline; completed detail, plan, usage, diagnostics, and full tool output open in drawers. Initial focus is the composer unless an approval modal arrives; closing a drawer returns focus to its prior owner.

Acceptance tests:

- view-state transition table for open/close/back/escape;
- Ctrl+B never mutates transcript length;
- golden buffers immediately below/at/above 40/60/100-column and 12/24-row breakpoints retain composer/approval/quit access and correct focus restoration;
- optional hero setting defaults false and does not affect machine output.

## CR-03.7 — One executable slash-command registry

**Covers:** INV-57, INV-63, INV-64, INV-65, INV-66, INV-67, INV-68.

Extend `CommandSpec` so metadata describes behavior, not only booleans:

```text
CommandSpec {
  name, aliases, argument_schema, help,
  repl: SurfaceRoute,
  tui: SurfaceRoute,
  agent_message: SurfaceRoute,
  capability: CapabilityPredicate,
  minimum_authorization: AuthorizationRequirement,
  visibility: CommandVisibility,
}

SurfaceRoute = Local(LocalCommand) | Forward(SystemCommandRoute) | Unsupported
CommandVisibility = Common | Expert | Hidden | Removed
```

Implement:

1. Replace `tui_forwarded`/`tui_autocomplete` booleans with explicit routes and derive availability/autocomplete/help from those routes.
2. Define local handler enums and exhaustive `match` dispatch for REPL and TUI. Adding a local enum variant without a handler must fail compilation.
3. `/debug` is a real local command on both REPL and TUI that toggles only that client's diagnostic/event-detail presentation. It does not mutate the process-wide tracing filter in this refinement; help says exactly that.
4. `/skin` is explicitly local on both surfaces.
5. `/status` is local presentation over the shared snapshot/report on both surfaces.
6. `/tools [NAME] [--all]` is local on REPL/TUI over the revisioned snapshot described in CR-03.4. Its agent-message route is independently declared and authorization-checked. Argument parsing is typed; it is not an `ExactOnly` route after this change.
7. Generate REPL `SLASH_COMMANDS`, hint/autocomplete, help table, TUI autocomplete/help, agent-message routing help, and surface docs from the registry. Delete the static list in `crates/thinclaw-channels/src/repl.rs`, the hard-coded `src/channels/repl.rs::print_help` table, and duplicated TUI lists/help in `crates/thinclaw-agent/src/command_catalog.rs`.
8. Ensure `/rewind`, `/plan`, `/restart`, and every other registered/supported command appear on the correct surfaces.
9. Remove `/think` from help, completion, routing, and docs until an implemented and policy-approved reasoning view exists. Input `/think` returns a concise removed-command explanation, not chat submission. This does not remove or rename the distinct registered agent tool `agent_think`; file 07 keeps that execution primitive and its policy visible.
10. Delete the `!<command>` line and `/think` line from `command_catalog.rs::tui_help_text`, all autocomplete arrays, REPL help, docs, and tests. Retain only the safe removed-command notice in the TUI input handler.
11. Normalize the message-level job vocabulary in the registry: `/job create DESCRIPTION`, `/job list [FILTER]`, `/job status [ID]`, `/job cancel ID`, and `/job help ID`. Keep the router's legacy `/job DESCRIPTION`, `/create`, `/list`, `/jobs`, `/status ID`, `/cancel ID`, and `/help ID` as hidden message aliases through 0.18. Bare `/status` and `/help` remain unambiguously the shared system commands. Delete `TUI_ONLY_FORWARDED`/`TUI_ONLY_AUTOCOMPLETE` after these routes land.
12. Capability predicates hide unavailable commands (feature/backend/runtime dependent) and explain unavailable explicit invocations. Agent-message routes additionally enforce identity/role authorization; local admin-only commands cannot become remotely executable merely because they share registry metadata.

Acceptance tests:

- every registry entry marked local/forwarded has a real matching handler;
- every handler has exactly one canonical registry entry;
- aliases resolve identically;
- help and autocomplete are subsets of supported commands for that surface;
- `/debug`, `/skin`, `/status`, `/rewind`, `/plan`, and `/restart` route correctly;
- `/tools`, `/tools NAME`, and `/tools --all` render the latest whole snapshot revision with exact origin/readiness/policy facts;
- canonical and legacy job command forms resolve without `/status`/`/help` ambiguity;
- `/think` is absent and cannot toggle dead state;
- `!<command>` is absent from every generated/help surface and cannot launch a process;
- remote/agent-message routing rejects local-only/admin-only commands without the declared authorization;
- snapshots are generated from the registry, not hand-maintained arrays.

## CR-03.8 — Accessibility, resilience, and performance pass

Implement:

1. Do not rely on color alone for tool/error/approval state; include symbols/text.
2. Restore terminal mode and cursor on panic/error using the existing guard pattern; the panic-path cleanup test is mandatory on Unix and runs in the Windows CI terminal fixture.
3. Keep input responsive under rapid status streams through a 1,024-entry typed channel. Coalesce replaceable usage/context/status updates by key before enqueue; terminal tool/approval/completion events apply backpressure and are never dropped. Tests saturate the queue and prove both behaviors.
4. Enforce the 500-activity/200-diagnostic/8-KiB-preview/history-page limits above. Large tool output lives in artifacts/detail views, not the main transcript buffer.
5. Make key help discoverable and consistent with the actual event loop.

Acceptance:

- no unbounded state vector/channel under continuous updates;
- terminal cleanup executes on normal exit, error, interrupt, and panic guard;
- approval/tool terminal events are never coalesced away;
- UI remains operable in no-color and narrow-terminal fixtures.

## CR-03 definition of done

- [ ] Tool events carry and preserve invocation IDs and parameters.
- [ ] Approval events preserve request ID/arguments and use explicit scoped decisions.
- [ ] Subagent, plan, usage, context, job, auth, and repair updates remain structured.
- [ ] TUI starts with authoritative model/agent/conversation/capability facts and durable history.
- [ ] `/tools` and `/status` consume one atomically revisioned snapshot; hot updates cannot mix revisions.
- [ ] Markdown, wrapping, and scrolling are visual-row-correct and terminal-safe.
- [ ] Navigation cannot delete transcript/activity by accident.
- [ ] Default layout is compact and chat-first.
- [ ] One registry generates slash routing/help/completion for REPL and TUI.
- [ ] The same registry explicitly governs authorized agent-message routes without exposing local admin behavior.
- [ ] `/think` and the raw shell escape are absent.
- [ ] Interleaving, approval, history, rendering, and registry consistency tests pass.
