# Repository Layout

ThinClaw is the agent runtime repository. The root package owns the CLI, TUI,
WebUI/gateway runtime, native channels, routines, tools, memory, and release
artifacts.

## Canonical Roots

- `src/`: root runtime facade, binary entrypoints, and concrete host wiring.
- `crates/`: extracted reusable runtime crates. These crates should import each
  other directly and must not import the root `thinclaw` package.
- `apps/desktop/`: ThinClaw Desktop companion app. Desktop is an app-level
  surface, not part of the WebUI folder.
- `src/channels/web/` and `crates/thinclaw-gateway/`: WebUI and gateway runtime
  code. This is the browser/gateway surface for the agent runtime, not the
  Desktop app home.
- `channels-src/` and `tools-src/`: standalone WASM component source crates.
- `registry/`, `channels-docs/`, and `tools-docs/`: packaged component metadata
  and docs.
- `patches/`: canonical vendored dependency patches shared by the workspace and
  apps.

## Workspace Hygiene

Do not keep scratch clones, merge copies, or generated comparison trees beside
active clones in `/Users/mt/Programming/Schtack/ThinClaw`. Archive them outside
the project folder or convert intentional parallel checkouts into Git worktrees
with clear names.

The active Desktop parity branch lives in `thinclaw-desktop`. The sibling
`ThinClaw` checkout is the main branch clone and should not be modified during
Desktop parity cleanup unless a sync step explicitly calls for it.
