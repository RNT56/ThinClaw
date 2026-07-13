# Desktop Performance Budgets

Last updated: 2026-07-13

These budgets are observable product contracts, not benchmark claims. Runtime
measurements are reported on the machine that runs ThinClaw; deterministic
source-level guards cover batching, virtualization, and bundle size in CI.

| Surface | Budget | Enforcement / evidence |
|---|---:|---|
| Backend ready | 8,000 ms | Measured from `run()` entry through database, secrets, RAG, tray, and shortcut initialization. Logged and exposed by `get_system_specs`; over-budget starts are warnings. |
| Renderer ready | 2,500 ms | Measured from webview navigation to the first post-setup React commit and exposed as `window.__THINCLAW_PERFORMANCE__` for browser/E2E inspection. |
| Stream delivery | 16 ms | Adjacent `AssistantDelta` events for one run/message are coalesced for one frame; other `UiEvent` variants flush pending deltas and remain ordered/immediate. A single coalesced payload is capped at 64 KiB. |
| Chat history | Windowed DOM | Direct Workbench and Agent Cockpit timelines both use `react-virtuoso`; the Agent timeline's pure preparation path is covered with 10,000 messages. |
| JavaScript chunks | 500 KiB each | `npm run build` fails through `scripts/check_frontend_bundle.mjs` when any production chunk exceeds the limit. |
| App + inference memory | User-configured GiB ceiling | `get_system_specs` separates the Desktop process from every descendant sidecar (including launcher grandchildren), reports the combined ceiling state, and the Server Resources UI raises an explicit over-budget warning. The budget is advisory at runtime: ThinClaw does not abruptly kill an active model and risk corrupting an in-flight response. The model-selection UI uses the same quota when evaluating a model/context combination. |

Run the deterministic performance contract tests with:

```bash
cd apps/desktop
npm run test:performance
npm run build
```

Cold-start and resident-memory results vary with hardware, first-run downloads,
database migration work, model size, and OS cache state. Capture whether a run
was cold or warm when using the runtime numbers as release evidence.
