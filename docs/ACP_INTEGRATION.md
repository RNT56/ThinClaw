# ACP Integration

ThinClaw can run as an Agent Client Protocol (ACP) agent for editor-native clients.
The implementation is buildable and uses the official ACP v1 flow, but it
should remain marked as compatibility-hardening work until the golden
transcript, schema, and real-editor smoke tests all pass.

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

`session/prompt` accepts ACP text and resource blocks. Resource text is inlined; resource links are passed as context references. ThinClaw emits `session/update` notifications for assistant chunks, tool starts/results, status/thought updates, sub-agent status fallbacks, mode changes, and approval-needed states. Stop reasons are serialized through typed ACP wire models; current core support covers `end_turn`, `cancelled`, and mapped provider refusal/length errors.

When a tool requires approval, ThinClaw sends an ACP `session/request_permission` client request and waits for the result before completing the prompt turn. The response is bridged back into the existing ThinClaw pending-approval flow, so allow-once, allow-always, reject, and cancelled outcomes share the normal approval machinery. `session/list` and `session/load` use persisted ThinClaw conversation metadata when the database is enabled; with `--no-db`, load/list are limited to sessions active in the current ACP process.

ThinClaw advertises image/audio prompt support as disabled until the media pipeline is deliberately wired into ACP. If a client advertises filesystem or terminal support, `read_file`, `write_file`, and `shell` route through the ACP client APIs; otherwise they fall back to host-side tools when ThinClaw policy allows it. ACP stdio MCP server descriptors are translated into session-scoped ThinClaw MCP configs and activated through the extension manager; HTTP/SSE MCP transports are rejected because this ACP build does not advertise them.

`session/resume` is retained as a compatibility handler, but it is not advertised as an ACP v1 core capability.

## Remaining Acceptance Gates

- Golden JSON-RPC transcripts for initialize, new, prompt, permissions, cancel, close, list, and load
- Schema validation for every emitted ACP message shape
- Real Zed and VS Code ACP smoke tests
- Native provider/cancel propagation deeper than the existing ThinClaw interrupt path
