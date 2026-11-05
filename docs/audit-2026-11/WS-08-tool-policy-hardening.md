# WS-08 — Tool Policy Hardening

| | |
|---|---|
| **Priority** | P1 |
| **Effort** | M |
| **Status** | ⏳ Not started |
| **Depends on** | WS-03 (unified `prepare_tool_call`) · decision **D9** |
| **Findings** | H2 (TOOL-5 part), TOOL-6, TOOL-8, TOOL-9, TOOL-10, AGT-6, AGT-8 |
| **Canonical docs** | `src/tools/README.md`, `docs/SECURITY.md`, `src/NETWORK_SECURITY.md`, `docs/DESKTOP_AUTONOMY.md`, `docs/PROCESS_ISOLATION.md`, `src/setup/README.md`, `FEATURE_PARITY.md` (argument-scoped permissions row) |

## Goal

Once WS-03 guarantees every lane goes through one gate, make the gate's *classification* robust: outbound channels require approval, shell classification is structural rather than substring-based, argument policies can't be smuggled past, and sensitive host paths are never auto-approved.

## Tasks

- [ ] **8.1 — `extract_document` is an outbound channel (H2/TOOL-5).**
  No `requires_approval` override (→ `Never`), registered by default (`crates/thinclaw-tools/src/registry.rs:~842`, `light` includes `document-extraction`), GETs any public HTTPS URL (`crates/thinclaw-tools/src/builtin/extract_document.rs:~140-190`) with no outbound leak scan (unlike `http.rs:~601`). Make it `UnlessAutoApproved` (or `Never` only for allowlisted hosts), run the leak scanner on the URL, cap query length.

- [ ] **8.2 — Outbound-capability inventory.**
  Enumerate every tool that can cause network egress or third-party side effects (built-ins, WASM via WS-06.9, MCP, browser, `send_message`, `tool_rpc`) and assert in a test that none is `Never` unless explicitly allowlisted. Reads like `memory_read` / `memory_search` / `session_search` (`src/tools/builtin/memory.rs:~902-1040`) may stay `Never`; the invariant is that *egress* is gated.

- [ ] **8.3 — Structural shell classification (TOOL-6).**
  `requires_explicit_approval` (`crates/thinclaw-tools/src/builtin/shell_security.rs:~1414`) lower-cases the command and substring-matches `NEVER_AUTO_APPROVE_PATTERNS` (e.g. `rm -rf`), so it misses `rm -r -f`, `find ~ -delete`, `git push origin +main`; `detect_path_escape` (`:~1319`) only inspects tokens starting with `/` or containing `..` (misses `~/.ssh/...`, `$HOME/.aws/...`, interpreter one-liners). Tokenize with a real shell lexer, classify per command segment (pipes, `;`, `&&`, subshells, `$()`), expand `~`/`$HOME` for path checks, and treat interpreters (`python3 -c`, `node -e`), `env`, `xargs`, `find -exec` as wrappers whose payload is classified recursively.

- [ ] **8.4 — Kernel FS confinement by default (TOOL-6, D9).**
  `docs/SECURITY.md:~42` says shell paths are "confined to the workspace by default"; real confinement (`THINCLAW_HOST_FS_SANDBOX`) is off by default. Per D9, enable it by default in sandboxed workspace mode (bwrap/seatbelt), or correct the doc.

- [ ] **8.5 — Argument-policy bypasses (TOOL-8).**
  `crates/thinclaw-settings/src/arg_policy.rs:~155`: glob `*` matches `;`, `|`, newline → `Allow shell "npm run *"` admits `npm run x; <anything>`. Reject shell metacharacters (or match per parsed segment from 8.3) before glob matching. `url_host` (`:~121`) is hand-rolled and disagrees with reqwest/`url` on backslash-userinfo, percent-encoded hosts, and trailing dots — parse with `url::Url`, normalize (IDNA, lowercase, strip trailing dot) before allow/deny.

- [ ] **8.6 — Protected host paths + honest wizard modes (TOOL-9).**
  Wizard "Autonomous" (`src/setup/wizard/agent.rs:~493`) sets `auto_approve_tools=true` + `workspace_mode=unrestricted`; `write_file` (`crates/thinclaw-tools/src/builtin/file.rs:~486-620`) has no deny-list. Add a non-overridable `Always` (even under auto-approve) for writes to `~/.ssh/**`, `~/.thinclaw/**` (config, `.env`, `user-tools/*.toml`), shell rc files, `~/Library/LaunchAgents/**`, systemd user units, and the same paths via shell (8.3). Make "Autonomous" and "Full Auto" actually differ, and describe them accurately ("auto-approve safe operations" is not what it does).

- [ ] **8.7 — Routine tools require approval (AGT-6).**
  All routine tools default to `Never` (`crates/thinclaw-agent/src/routine_tools.rs:~113, ~531, ~611-634`). Make `routine_create` / `routine_update` / `routine_delete` `UnlessAutoApproved`; require approval (`Always`) when an update changes the prompt or grants of a routine that has `allowed_tools`/`tool_profile` grants (those run autonomously via `execute_as_subagent`, `src/agent/routine_engine/execution.rs:~72-112`). Add a per-principal routine cap and a minimum cooldown.

- [ ] **8.8 — Per-segment prompt sanitization (AGT-8).**
  `src/agent/dispatcher/prompt_context.rs:~205-216`: one Critical-severity pattern anywhere in the assembled workspace prompt replaces SOUL, AGENTS and IDENTITY together. Sanitize per source segment; never drop trusted control files because an untrusted overlay (e.g. a routed-agent override, WS-03.4) matched.

- [ ] **8.9 — Correct stale tool-security docs (TOOL-10).**
  `src/NETWORK_SECURITY.md:~541-549` (http tool: actually upgrades `http`→`https`, follows same-origin redirects with re-validation, is `UnlessAutoApproved`); `docs/DESKTOP_AUTONOMY.md:~26` (reckless mode still returns `UnlessAutoApproved`, `desktop_autonomy.rs:~325`); comment at `crates/thinclaw-tools/src/builtin/shell.rs:~281` (scanner default is `fail_closed`).

## Verification (workstream exit)

- Table-driven tests for shell classification (≥50 evasions: flag splitting, quoting, wrappers, `~`/`$HOME`, interpreters) and arg policies (metachar smuggling, URL-parser differentials).
- Under `auto_approve_tools=true`, a write to `~/.ssh/authorized_keys` via `write_file` or `shell` still prompts.
