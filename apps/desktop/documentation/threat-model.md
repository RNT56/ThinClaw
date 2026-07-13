# ThinClaw Desktop Bridge Threat Model

Last updated: 2026-07-13

This document defines the security boundary between untrusted runtime data,
the privileged Tauri host, the React webview, and a remote ThinClaw gateway. It
covers the Desktop bridge and remote deployment path; the root gateway and
agent tool-policy threat models remain authoritative for their own internals.

## Trust Boundaries

| Boundary | Trust decision |
| --- | --- |
| Local or remote runtime output -> React | Untrusted. Tool output, Markdown, SSE events, error bodies, and session metadata may be malformed, hostile, or arbitrarily large. Authentication does not make content safe. |
| React -> Tauri command | Privileged request. Generated command bindings define the shape, but Rust must still validate every host, URL, credential, confirmation, and filesystem/process argument. |
| Desktop -> remote gateway | Bearer-authenticated transport. A public gateway requires HTTPS. Plain HTTP is accepted only for loopback, private/link-local IPs, `.local`, or Tailscale hosts. |
| Remote gateway -> Desktop | Untrusted server response. Redirects are refused and response/event sizes are bounded before parsing or rendering. |
| Desktop -> SSH deployment target | User-authorized remote mutation. Host/user/key input is validated, SSH uses trust-on-first-use (`accept-new`) and rejects changed host keys, and credentials travel over stdin rather than process arguments. |
| Profile metadata -> durable state / IPC | Non-secret metadata only. Profile bearer tokens live in the encrypted Keychain envelope and are redacted from `identity.json`, status responses, discovery responses, and debug formatting. |
| Saved credentials -> React status/settings | Presence-only metadata. Remote gateway, custom LLM, profile, and Gmail OAuth credentials remain encrypted and broad status/OAuth responses never return reusable values. The local pairing handshake token is the deliberate exception described below. |

## Threats And Controls

| Threat | Control |
| --- | --- |
| Script or raw-HTML injection through runtime Markdown | React text escaping remains the default. `ReactMarkdown` is used without raw-HTML plugins. No runtime content reaches `dangerouslySetInnerHTML`. External links are parsed and limited to bounded, credential-free HTTP(S) URLs before the OS opener is invoked. |
| Render crash or memory exhaustion from hostile tool output | Remote JSON responses are capped at 8 MiB, text at 16 MiB, error bodies at 4 KiB, and an unterminated SSE event at 1 MiB. Tool cards safely stringify cyclic/non-string errors and truncate display text at 100,000 characters. Session-key extraction refuses oversized output and non-string or malformed identifiers. |
| Bearer disclosure through redirects, URL credentials, logs, or debug output | The HTTP client refuses redirects, URL userinfo/query/fragment/path input, stores a sensitive `HeaderValue` rather than a raw token string, and does not expose a token accessor. SSE payloads are not debug-logged. Error bodies are bounded and control characters are collapsed. |
| False-positive connection test with an invalid token | Health checks call the authenticated `/api/gateway/status` route. `401`/`403` returns an explicit rejected-credential result; gateway activation stops instead of starting an unauthorized SSE retry loop. |
| Bearer interception over public plaintext HTTP | Public hosts require HTTPS. HTTP remains available for private, loopback, link-local, `.local`, and Tailscale routes so local-first and tailnet deployments work. The UI uses the root gateway default port `3000`. |
| Profile tokens persisted in `identity.json` or returned to the webview | Existing plaintext profile tokens migrate to namespaced keys in the encrypted Keychain envelope. The source document is rewritten only after migration succeeds. Serialization emits `null`, status/list commands return redacted profiles, and in-memory profile tokens are zeroized on drop. |
| Saved secret erased when a redacted settings form is submitted | Gateway and custom-LLM credential inputs use patch semantics: omitted values preserve the encrypted credential and an explicit clear action deletes it. Profile selection uses the profile ID and the privileged backend credential lookup rather than resubmitting a redacted profile object. |
| OAuth credentials exposed through IPC or plaintext runtime settings | Gmail OAuth completion returns only non-secret status metadata. Access/refresh tokens are stored in the authenticated Keychain envelope, legacy plaintext settings migrate and are deleted, and runtime injection uses the in-memory bridge overlay. Token buffers are zeroized after storage. |
| SSH option/shell injection or silent host-key replacement | SSH hosts, users, and Tailscale keys use strict allowlists. Commands include `--`, `BatchMode=yes`, and `StrictHostKeyChecking=accept-new`; a changed known-host key fails. Setup credentials use a two-line stdin contract and output is redacted before `deploy-log` emission. |
| Deployment credential broadcast to unrelated listeners | `deploy-status` no longer carries credentials. The generated token is returned only as the initiating Tauri command result and is then saved through the encrypted profile/gateway secret paths. Setup output never prints it. |
| Desktop connecting to the wrong deployment port | Deployment, setup, documentation, and wizard defaults use the root ThinClaw gateway contract: port `3000`. When Tailscale is provisioned, Desktop discovers and validates the new CGNAT address before health checking it. |

## Deliberate Residual Risk

- Private-network HTTP is still plaintext. It is intended for loopback or
  operator-controlled networks; Tailscale or HTTPS is recommended whenever the
  network is not fully trusted.
- `accept-new` prevents silent replacement after first use but cannot prove the
  first host key. Operators deploying to sensitive hosts should verify the SSH
  fingerprint out of band before the first connection.
- The initiating webview necessarily holds a newly generated deployment token
  long enough to save or copy it. It is never sent through the broadcast log or
  status-event channels.
- `thinclaw_get_status` returns the local gateway handshake token because the
  pairing UI explicitly lets the local operator copy it to another device.
  This is an intentional privileged local reveal; provider, remote gateway,
  custom LLM, profile, and OAuth credentials remain presence-only.
- Durable Desktop secret persistence is currently macOS-only. Windows and Linux
  fail closed for secret writes until a real OS secure-store backend exists.
- A compromised remote gateway can return misleading but bounded content. The
  bearer credential proves access authorization, not server integrity beyond
  the TLS/SSH trust decisions above.

## Verification

Run from the repository root unless noted:

```bash
cargo fmt --all -- --check
CARGO_INCREMENTAL=0 cargo test --locked --manifest-path apps/desktop/backend/Cargo.toml --lib thinclaw::
cd apps/desktop && npm run lint:ts
cd apps/desktop/frontend && npx vitest run src/tests/components/ChatSubComponents.test.ts
```

The exhaustive generated-binding test must also remain green after any command
signature change. Remote manual acceptance must prove a valid token succeeds,
an invalid token is rejected, public HTTP is refused, HTTPS or a private route
works, and no credential appears in `deploy-log` or Desktop debug logs.
