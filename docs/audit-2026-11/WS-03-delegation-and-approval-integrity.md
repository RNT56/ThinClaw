# WS-03 — Delegation & Approval Integrity

| | |
|---|---|
| **Priority** | P0 (Critical C1) |
| **Effort** | L |
| **Status** | ⏳ Not started |
| **Depends on** | WS-01 · decisions **D3**, **D4** |
| **Findings** | **C1** (TOOL-1/AGT-1), H1 (TOOL-3/AGT-2), H4 (AGT-4), TOOL-2, TOOL-7, TOOL-11, AGT-7 |
| **Canonical docs** | `src/tools/README.md`, `docs/SECURITY.md`, `src/NETWORK_SECURITY.md`, `docs/LOOP_HARDENING_STATUS.md`, `FEATURE_PARITY.md` (sub-agent, plan-mode, self-repair rows) |
| **Owns shared file** | `src/tools/execution.rs` (`prepare_tool_call`) — WS-08 rebases on this |

## Goal

An operator approval requirement means the same thing in every execution lane. Delegation (subagents, routed agents, builder, self-repair, `tool_rpc`) can never run a tool with a weaker approval than the parent would have needed.

## Background

See [FINDINGS §2 C1](./FINDINGS.md#c1--approval-classes-dont-survive-delegation). Lanes that currently execute tools with weaker gates than the interactive chat lane:

| Lane | Entry | Approval mode today | Hooks | `disabled_tools` / plan mode |
|---|---|---|---|---|
| Subagent | `src/agent/subagent_executor/mod.rs:~1608` | `Autonomous` (only `Always` gated) | none | no |
| Routed agent via `message_agent` | `src/agent/dispatcher/tool_execution.rs:~908-990` | `Autonomous` | none | no |
| Deferred calls after an approval | `src/agent/thread_ops/approval.rs:~475-490` | Interactive | yes | **no** |
| `execute_code` `tool_rpc` | `src/tools/builtin/execute_code.rs:~39-70`, `crates/thinclaw-tools/src/builtin/execute_code.rs:~1317` | auto-approves `http` GET / browser navigate | — | no |
| `build_software` / self-repair | `src/tools/builder/core.rs:~602-609` | **none** — direct `tool.execute()` | none | no |

## Tasks

- [ ] **3.1 — `spawn_subagent` inherits the strictest approval of what it grants (C1, D3).**
  `crates/thinclaw-tools/src/builtin/subagent.rs:287` → compute `requires_approval(params)` as the max approval class across the requested tools (resolved through the registry with the same profile/lane rules), `Always` if any granted tool is `Always`. Do the same for `message_agent` / `create_agent` grants (`crates/thinclaw-tools/src/builtin/agent_management.rs`). The approval card must list the tools being granted.

- [ ] **3.2 — Clamp subagent grants (C1, D3).**
  `crates/thinclaw-agent/src/subagent.rs:~982` `(None, Some(requested)) => requested` → when the parent has no routed allowlist, clamp to the set the parent could run *without* approval, plus anything approved in 3.1. Never let an explicit list bypass `explicit_only` without that approval (`crates/thinclaw-tools/src/registry.rs:~1592`).

- [ ] **3.3 — Single enforcement point (C1, TOOL-7).**
  Move `disabled_tools` (today only `src/agent/dispatcher/tool_execution.rs:101-140`, read from a hardcoded `local_user` setting), plan-mode escalation (`:~222`), and BeforeToolCall hooks into `prepare_tool_call` (`src/tools/execution.rs:~150-200`) so subagent, deferred-approval, `tool_rpc`, builder, and routine lanes all get them. Pass plan-mode state in the request. Remove `hooks: None` from `subagent_executor`. Also filter disabled tools out of tool *definitions* so the model doesn't see them.

- [ ] **3.4 — Agent registry: approval + ownership (H4).**
  `create_agent` / `update_agent` / `remove_agent` / `message_agent` currently default to `Never`. Make create/update/remove require approval (`Always` when setting `is_default`, a `system_prompt`, or tool grants). Add `principal_id` to `AgentWorkspaceRecord` (`crates/thinclaw-agent/src/agent_registry.rs:~163-186`) with migrations for both backends; scope CRUD and `agent_router::route` (`crates/thinclaw-agent/src/agent_router.rs:~137-191`) by principal; the global default-agent override (`src/agent/dispatcher/prompt_context.rs:~178-190`) becomes admin-only.

- [ ] **3.5 — Builder goes through the gate (H1 / TOOL-3).**
  `src/tools/builder/core.rs:~602-609` — replace direct `tool.execute(params, &JobContext::default())` with `prepare_tool_call` (Interactive, no bypass, parent's identity). Restrict callable names to the advertised build tool set; reject anything else.

- [ ] **3.6 — Don't register host dev tools behind the operator's back (TOOL-2).**
  `crates/thinclaw-app/src/assembly.rs:326` (`builder_workspace = builder_enabled && (allow_local_tools || !sandbox_enabled)`) and `src/tools/registry.rs:~1173` (`register_dev_tools_with_runtime(…, None /*safety*/, …)`): gate builder dev tools on `allow_local_tools`, always pass `Some(&config.safety)` so the external scanner / smart approval apply. Fix the wizard copy "local tools stay off" (`src/setup/wizard/agent.rs:~489`) to match reality.

- [ ] **3.7 — Self-repair safety (H1 / AGT-2, D4).**
  - Validate tool names against the registry and `[a-z0-9_]` before `record_tool_failure` (`src/agent/agent_loop/mod.rs:~760-774, ~867-880`; `src/agent/worker.rs:~931-934, ~1426-1431`).
  - In `build()` (`src/tools/builder/core.rs:~694-699`) canonicalize `project_dir` and require it to stay under `build_dir` before any `remove_dir_all`.
  - Treat `last_error` as untrusted when composing the repair requirement (`src/agent/self_repair.rs:~209-223`) — wrap with `wrap_external_content`.
  - Per D4: disable automatic repair by default until rebuilt tools can actually be registered (`registered: false` is hard-coded at `src/tools/builder/core.rs:~399, ~450, ~499`), or implement hot registration.

- [ ] **3.8 — Frame subagent results as untrusted (AGT-7).**
  `src/async_main.rs:~1052-1069` injects `"[Sub-agent result from X]\n\n{summary}"` as an `IncomingMessage` with the parent's identity → parsed as `UserInput`. Inject as a tool/system-event message wrapped with `wrap_external_content`.

- [ ] **3.9 — Close delegated egress paths (TOOL-11).**
  `tool_rpc` auto-approves `http` GET and browser navigate (`crates/thinclaw-tools/src/builtin/execute_code.rs:~1317`) — apply parent approval semantics. Review the default sandbox proxy allowlist (`crates/thinclaw-types/src/sandbox.rs:~139-141`: `github.com`, `api.github.com` accept writes, e.g. gists) and per-job credential grants.

- [ ] **3.10 — Regression suite.**
  Tests that fail on `5bebe1c` and pass after: (a) model requests `spawn_subagent{tools:[shell]}` → approval required, and subagent `shell` call with a non-`Never` command → not executed unattended; (b) `message_agent` to an agent with `shell` grant → same; (c) builder cannot call a non-build tool and cannot bypass approval; (d) plan mode holds for calls deferred after an approval; (e) `disabled_tools` blocks in subagent and `tool_rpc` lanes; (f) self-repair with tool name `..` or an absolute path does not touch the filesystem outside `build_dir`.

- [ ] **3.11 — Docs.**
  `src/tools/README.md` (approval classes per lane), `docs/SECURITY.md` ("policy and validation layers around dangerous tools" — state the lane guarantees), `FEATURE_PARITY.md` (sub-agent budget/fairness, plan mode, self-repair rows), `docs/LOOP_HARDENING_STATUS.md`.

## Verification (workstream exit)

```bash
cargo test --locked --workspace
cargo test --locked --features full --test '*' -- subagent approval builder
```

Plus a manual injection drill: a fetched page instructs the agent to spawn a subagent with `shell` and run a benign marker command; the operator must see an approval prompt.
