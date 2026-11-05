# WS-05 — Web UI & Client Surfaces

| | |
|---|---|
| **Priority** | P0 (5.1–5.4, Critical C3) · P1 (5.5, 5.8, 5.9) · P2 (rest) |
| **Effort** | M |
| **Status** | ⏳ Not started |
| **Depends on** | WS-01 |
| **Findings** | **C3** (UI-1, UI-2, UI-3), UI-4 … UI-10 |
| **Canonical docs** | `src/NETWORK_SECURITY.md`, `docs/MOBILE_SECURITY.md` (if gateway headers change), `apps/desktop/documentation/security-and-dependencies.md`, `docs/DESKTOP_AUTONOMY.md`, `FEATURE_PARITY.md` |
| **Owns shared file** | `crates/thinclaw-gateway/src/web/static/app.js` — WS-07 rebases on this |

## Goal

Nothing the model, a channel, or a tool produces can execute script in the operator's browser or desktop webview; the gateway token is not reachable from page script even if it could.

Note: the served UI is `crates/thinclaw-gateway/src/web/static/*` (`include_str!`). `src/channels/web/static/*` is a stale, divergent copy (5.7).

## Tasks

- [ ] **5.1 — Canvas `panel_id` sink (C3/UI-1).**
  `app.js:~10359-10372` builds the panel header with string concatenation into `card.innerHTML`, including an inline `onclick` using `escapeJsString` (`:~10426`, does not escape `"`). Build the header with `createElement` / `textContent` / `addEventListener` (or `data-panel-id` + delegated handler). Server side, restrict `panel_id` to `[A-Za-z0-9_-]{1,128}` in `validate_panel_id` (`crates/thinclaw-tools/src/builtin/canvas.rs:~331`).
  *Accept:* a test that drives a canvas `show` with a hostile `panel_id` (quote, angle brackets, backtick) produces inert text and no extra attributes.

- [ ] **5.2 — Fix escaping and remove inline handlers (C3/UI-2).**
  `escapeHtml` (`app.js:~10420`) must escape `&<>"'`. Replace every string-built `onclick="…"` with event delegation — known sinks include `:~5726` (routine name), `:~3178` (memory path), `:~5159`, `:~5164`, `:~5715`, `:~5815` (raw ids), and `title="…"` attributes at `:~8534`, `:~8561`, `:~8705`, `:~7997`. Add a CI grep guard that fails on new `onclick="` / `onerror="` string building in `static/*.js`.

- [ ] **5.3 — Strict CSP for the gateway UI (C3/UI-3).**
  `src/channels/web/server.rs:~1776-1785` sets only nosniff and `X-Frame-Options`. After 5.2 removes inline handlers, send `Content-Security-Policy: default-src 'none'; script-src 'self' <SRI-pinned marked host>; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'` (adjust for any inline styles). Add a test asserting the header on `/`.

- [ ] **5.4 — Allowlist HTML sanitizer for rendered markdown (C3/UI-3).**
  `sanitizeRenderedHtml` (`app.js:~1960-1985`) is a denylist that keeps remote `<img>`. Vendor DOMPurify (SRI) with an allowlist config; drop or proxy remote images. Applies to assistant messages (`:~1812`), memory files (`:~3130`), job descriptions (`:~5323`).

- [ ] **5.5 — Get the token out of page reach and URLs (UI-8).**
  Token is a top-level `let` (`app.js:3`) and in `sessionStorage`; SSE and logs use `?token=` (`:~828`, `:~3243`); the webhook canvas embeds the gateway/webhook secret in HTML (`src/channels/canvas_gateway.rs:~515, ~592`). Introduce short-lived, scope-limited stream tickets for SSE/WS; keep the bearer in a closure, not a global. (WS-07.12 fixes URL-decoding of query tokens in the meantime.)

- [ ] **5.6 — Canvas iframe (UI-6).**
  `app.js:~10368` iframes `/canvas/*` with `sandbox="allow-scripts allow-forms allow-same-origin"`, but `/canvas/*` lives only on the webhook server and the gateway sends `X-Frame-Options: DENY`, so it never renders. Decide: serve canvas from a separate origin and drop `allow-same-origin`, or render natively. Either way never combine `allow-scripts` + `allow-same-origin` for same-origin content.

- [ ] **5.7 — Remove the stale static copy (UI-9).**
  `src/channels/web/static/*` diverges from the served copy by ~550 lines and is still `include_str!`-ed by root tests. Point tests at the served copy and delete the duplicate.

- [ ] **5.8 — Tauri command scoping and secret getters (UI-5).**
  `apps/desktop/backend/build.rs:~88` calls plain `tauri_build::build()` — no `AppManifest::commands`, so all ~350 commands in `setup/commands.rs` are callable from every window, including Spotlight, which renders model output (`SpotlightBar.tsx:~559`). Declare an app manifest with per-window command sets (Spotlight: minimal). Replace raw-secret getters (`thinclaw_get_{anthropic,openai,…}_key`, `thinclaw_get_bedrock_credentials` at `commands/keys.rs:~280`, `thinclaw_reveal_gateway_token` at `gateway.rs:~304`) with "is set"/masked responses, or require an explicit OS-auth reveal. Fix `documentation/security-and-dependencies.md:~14`.

- [ ] **5.9 — AppleScript injection in the macOS bridge (UI-4).**
  `desktop-sidecars/ThinClawDesktopBridge.swift:~396` interpolates an unknown `key` raw into `keystroke "…"`; `menuClickScript` (`:~419-438`) interpolates `menuPath[i]`; reached from `keypress`/`chord` (`:~890-910`) and `select_menu` (`:~953-960`). Apply `appleScriptString()` to every interpolation, reject unknown keys, prefer AppleEvent parameters. Add payload validation in `src/desktop_autonomy/manager_core.rs:~315-321` (like `validate_numbers_payload`). Extend `desktop-sidecars/test_*.py` boundary tests.

- [ ] **5.10 — Keyed cloud-sync object names (UI-7).**
  `apps/desktop/backend/src/cloud/manifest.rs:~576-583` uses `objects/v1/{sha256(path)}/{sha256(plaintext)}/{uuid}.enc`. Switch to `HMAC(master_key, …)` under a `v2` prefix with migration; update the "provider never sees plaintext" claim (`cloud/encryption.rs:~4`) to be precise about metadata.

- [ ] **5.11 — `expose_inference` is a no-op (UI-9).**
  `thinclaw/config/engine.rs:~306-310` only changes the client `baseUrl`; `llama-server` always binds `127.0.0.1` (`sidecar/lifecycle.rs:~591`). Implement deliberately (with sidecar auth required and a warning) or remove the toggle (`config/types.rs:~436`).

- [ ] **5.12 — SSH host-key pinning for remote deploy (UI-10).**
  `apps/desktop/backend/src/thinclaw/deploy.rs:~354-363` uses `StrictHostKeyChecking=accept-new` and then sends the Tailscale auth key and gateway token over stdin. Show and confirm the host-key fingerprint on first deploy, then pin it.

## Verification (workstream exit)

- Browser test (gateway UI) feeding hostile `panel_id`, routine names, memory paths, and markdown with remote images: no script execution, no outbound image fetch, CSP header present.
- Desktop: `cargo test` in `apps/desktop/backend`; Spotlight window cannot invoke secret or config commands.
- `python3 -m unittest discover -s desktop-sidecars -p 'test_*.py'`.
