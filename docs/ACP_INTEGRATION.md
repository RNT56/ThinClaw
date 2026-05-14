# ACP Integration

ThinClaw can run as an Agent Client Protocol (ACP) agent for editor-native clients.
The implementation is buildable and uses the official ACP v1 flow. It should
remain marked as compatibility-hardening work until real-editor smoke tests are
completed against Zed and VS Code.

## Transport

- Binary: `thinclaw-acp`
- Feature: `acp`
- Transport: stdio JSON-RPC 2.0
- Protocol version: `1`
- Core methods: `initialize`, `authenticate`, `session/new`, `session/prompt`, `session/cancel`, `session/close`, `session/list`, `session/load`, `session/set_mode`

The implementation follows the public ACP flow: initialize, create or load a session, send prompt turns, stream `session/update` notifications, request client permissions with JSON-RPC, and return a `stopReason`.
Zed and VS Code behavior are the priority compatibility targets; editor quirks
must stay isolated to compatibility tests or `_meta` extensions.

## Running

```bash
cargo run --features acp --bin thinclaw-acp -- --workspace /absolute/project/path
```

Useful flags:

- `--config /path/to/settings.toml`
- `--workspace /absolute/project/path`
- `--model model-name`
- `--no-db`
- `--debug`

Logs go to stderr so stdout stays valid JSON-RPC.

## Tool Profile

ACP sessions use `ToolProfile::Acp`, which allows editor-appropriate tools:

- File/code tools: `read_file`, `write_file`, `list_dir`, `apply_patch`, `grep`, `search_files`, `shell`, `process`, `execute_code`
- Context tools: `memory_*`, `external_memory_*`, `session_search`, `skill_*`
- Agent/editor tools: `browser`, `vision_analyze`, LLM selection tools

Messaging, cron/routine, broad channel-control, and sub-agent tools are intentionally excluded from the ACP profile.

## Compatibility Notes

`session/prompt` accepts ACP text and resource blocks. Resource text is inlined; resource links are passed as context references. ThinClaw emits `session/update` notifications for assistant chunks, reasoning/thought chunks, structured plan and usage updates when the core produces them, tool starts/results, status fallbacks, sub-agent status fallbacks, mode changes, session info changes, and approval-needed states. Stop reasons are serialized through typed ACP wire models; current core support covers `end_turn`, `cancelled`, `max_tokens`, `max_turn_requests`, and mapped provider refusal/length errors.

When a tool requires approval, ThinClaw sends an ACP `session/request_permission` client request and waits for the result before completing the prompt turn. The response is bridged back into the existing ThinClaw pending-approval flow, so allow-once, allow-always, reject, cancelled, and timeout outcomes share the normal approval/cancel machinery. `session/list` and `session/load` use persisted ThinClaw conversation metadata when the database is enabled; with `--no-db`, load/list are limited to sessions active in the current ACP process. A session can have one active prompt turn at a time; concurrent turns for the same session are rejected instead of overwriting cancellation or approval waiters.

ThinClaw's ComfyUI `image_generate` tools are available in the standard runtime profile, but ACP still advertises image/audio prompt support as disabled until media artifacts are deliberately bridged into ACP editor clients. If a client advertises filesystem or terminal support, `read_file`, `write_file`, and `shell` route through the typed ACP client APIs; otherwise they fall back to host-side tools when ThinClaw policy allows it. ACP terminal wait client errors are surfaced as bridge errors, while actual wait timeouts trigger `terminal/kill` before output/release cleanup. ACP stdio MCP server descriptors are translated into session-scoped ThinClaw MCP configs and activated through the extension manager; HTTP/SSE MCP transports are rejected because this ACP build does not advertise them.

`session/resume` is retained as a compatibility handler, but it is not advertised as an ACP v1 core capability.

## Acceptance Coverage

CI runs `cargo check --features acp --bin thinclaw-acp`, ACP unit acceptance tests, and a subprocess stdio transcript smoke. Unit coverage validates emitted public messages against ACP v1 schema fixtures, typed content/resource blocks, typed update variants, client filesystem/terminal bridges, MCP descriptor translation, and session list/load replay. The subprocess transcript covers parse errors, `initialize`, no-auth `authenticate`, invalid params, unsupported MCP transport rejection, `session/new`, `session/list`, `session/load`, method-not-found errors, stdout NDJSON cleanliness, prompt streaming from a real test agent runtime, permission approve/reject/cancel/timeout flows, real `session/cancel`, and real `session/close`. Shared agent dispatcher tests also prove that turn cancellation drops in-flight provider and tool futures promptly.

## Remaining Acceptance Gates

- Real Zed and VS Code ACP smoke tests. These are release-blocking manual/live
  checks because they validate the editor integration layer outside the stdio
  harness. Do not call ACP complete until both editors have run the same flow:
  initialize, create a new session, load an existing session, stream a prompt
  response, read and write a file through the editor bridge, approve and reject
  a permission request, cancel an active turn, and close the session.
- Record the exact editor version, ThinClaw commit, workspace path, and pass/fail
  notes in the release checklist. If `zed` or `code` is unavailable in the test
  environment, mark the gate as blocked rather than substituting subprocess
  smoke coverage.
