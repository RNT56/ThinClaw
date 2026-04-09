# Security, Safety, and Trust Report

## Executive Summary

ThinClaw has a real, multi-layer security model, not just aspirational wording. The code enforces output truncation, prompt-injection filtering, secret-leak scanning, endpoint allowlists, Docker sandboxing, non-root container execution, and constant-time token checks in network-facing paths. The strongest single source for that model is `src/NETWORK_SECURITY.md`, backed by the `src/safety/`, `src/sandbox/`, `src/secrets/`, and `src/tools/` modules.

The docs are directionally good, but the top-level narrative still overstates the trust envelope in a few places. In particular, `README.md` and `CLAUDE.md` blur the line between sandboxed components and unsandboxed ones, and some docs describe MCP as if it were just another safe extension path. It is not: MCP servers are operator-controlled external processes or remote services, and the code does not apply the same sandbox or SSRF controls there.

The biggest missing piece is a single canonical security/trust overview that explains ThinClaw’s trust boundaries in plain language: what is sandboxed, what is merely policy-controlled, what is operator-trusted, and what is intentionally outside the threat model. That doc should be the hub for all other security claims.

## Actual Security / Trust Model

ThinClaw’s security posture is layered and boundary-based:

- `src/config/safety.rs` only exposes two tunables: `SAFETY_MAX_OUTPUT_LENGTH` and `SAFETY_INJECTION_CHECK_ENABLED`. That is important because the public docs should not imply a large, user-tunable security surface when the actual config is intentionally narrow.
- `src/safety/mod.rs` combines truncation, leak detection, policy enforcement, and prompt-injection sanitization. Tool output is truncated when it exceeds the configured cap, scanned for secret leakage, checked against policy rules, and optionally sanitized before reaching the LLM.
- `src/safety/policy.rs` is not generic “AI safety”; it has explicit default rules for system file access, crypto key patterns, shell injection, encoded exploit payloads, obfuscated strings, and excessive URL spam.
- `src/sandbox/config.rs` makes the sandbox default to `ReadOnly`, routes network through an allowlisted proxy, and supports `WorkspaceWrite` and `FullAccess`. `FullAccess` is a deliberate escape hatch, not a hidden secure mode.
- `src/sandbox/mod.rs` and `src/NETWORK_SECURITY.md` show the intended sandbox properties clearly: container isolation, proxy-mediated egress, credential injection at the proxy boundary, non-root execution, read-only rootfs, capability dropping, and cleanup after execution.
- `src/secrets/mod.rs` and `src/config/secrets.rs` show a host-boundary secret model: secrets are AES-256-GCM encrypted, derived per-secret with HKDF-SHA256, sourced from env or OS keychain, and injected by the host at request time so the WASM tool never sees the raw token.
- `src/tools/policy.rs` adds a second trust layer that is easy to miss in the docs: tool access can be scoped globally, per channel, or per group with allowlist/denylist semantics.
- `src/tools/wasm/allowlist.rs` and `src/NETWORK_SECURITY.md` enforce request-level egress control for WASM tools: HTTPS-only by default, host/path/method matching, URL normalization, userinfo rejection, and path traversal blocking.
- `src/tools/mcp/client.rs` and `src/tools/mcp/config.rs` make the MCP story explicit: stdio MCP servers are spawned as child processes, HTTP MCP servers are connected over Streamable HTTP, and remote HTTPS servers are authenticated separately. That path is not sandboxed in the same way as WASM tools.

The network-security reference is especially strong because it names the trust boundaries directly: local user, browser client, Docker containers, and external services. It also documents specific protections like bearer-token auth, Origin validation, CORS allowlisting, rate limits, body limits, response-size caps, and no-redirect behavior in the web gateway and HTTP tools.

## Current Doc Accuracy Assessment

- `src/NETWORK_SECURITY.md` is the most accurate and useful security reference. It is detailed, concrete, and mostly aligned with the code paths I checked.
- `README.md` is broadly accurate about the existence of sandboxing, allowlisting, and prompt-injection defense, but it compresses too many trust boundaries into broad claims like “nothing leaves your control.” That is stronger than the implementation actually guarantees.
- `CLAUDE.md` is conceptually right about “defense in depth,” but it is stale in places and still describes the channel model and setup flow in older terms. Its security language needs to be rewritten to match the current native/WASM/MCP split.
- `src/tools/README.md` is honest about the WASM-vs-MCP tradeoff. Its strongest security statement is also the most important one: MCP servers are external processes with full system access and no sandbox, so they are not equivalent to WASM tools.
- `docs/EXTENSION_SYSTEM.md` is useful but slightly over-trusts MCP by presenting it as a generic extension bucket. It needs a sharper warning that operator-controlled configuration is the trust boundary, not a sandbox boundary.
- `docs/EXTERNAL_DEPENDENCIES.md` is good on the “optional only” framing. It correctly says core ThinClaw is self-contained and lists external tools as feature unlocks rather than hard runtime requirements.

## Contradictions and Drift

- `README.md` says ThinClaw is “security-hardened” and that WASM tools and channels run in a sandboxed runtime. That is only partially true. WASM components are sandboxed, but native channels and MCP servers are not.
- `README.md` says “nothing leaves your control.” That overstates the guarantee. External services, webhook endpoints, tunnel providers, and remote MCP servers can all receive data when configured.
- `CLAUDE.md` lists Telegram and Slack as “native” channels, which conflicts with the current hybrid channel model documented in `docs/CHANNEL_ARCHITECTURE.md` and `FEATURE_PARITY.md`.
- `docs/EXTENSION_SYSTEM.md` treats MCP as a single discovery/installation plane, but the code distinguishes between sandboxed WASM tools and unsandboxed MCP servers. The security documentation should make that distinction the headline, not a footnote.
- `docs/EXTENSION_SYSTEM.md` and `src/tools/mcp/config.rs` both show that MCP config can live in `~/.thinclaw/mcp-servers.json`, but the broader docs should stop implying that file is a security boundary. It is operator-trusted config.
- The current security docs do not surface `ToolPolicyManager` enough. Per-channel and per-group tool scoping is a real security control and deserves mention.

## Canonical Security Topics

1. Trust boundaries and threat model: local user, browser client, Docker sandbox, native channels, WASM tools, MCP servers, and external services.
2. Secret handling: master key sourcing, encryption, host-boundary injection, and what the tool runtime never sees.
3. Sandbox model: `ReadOnly`, `WorkspaceWrite`, and `FullAccess`, plus the proxy, allowlist, and cleanup behavior.
4. Prompt-injection defense: sanitizer, leak detector, policy rules, output truncation, and external-content wrapping.
5. Tool governance: global/channel/group policy overrides, dangerous-tool warnings, and approval surfaces.
6. Network security: gateway auth, Origin validation, CORS, webhook secrets, rate limiting, body limits, redirects, and SSRF boundaries.
7. Extension trust model: WASM tools/channels versus MCP servers, with explicit “sandboxed vs operator-trusted” language.
8. Operator assumptions: single-user local machine, loopback defaults, and where ThinClaw intentionally relies on operator-controlled configuration.

## Rewrite Recommendations

- Create one canonical `docs/overview/security.md` or equivalent and make every other security claim point back to it.
- Rewrite `README.md` security wording so it distinguishes between sandboxed components, policy-controlled components, and external trust dependencies.
- Rewrite `CLAUDE.md` security sections to reflect the current native/WASM/MCP split and remove stale channel assumptions.
- Update `docs/EXTENSION_SYSTEM.md` so MCP is described as trusted external execution, not as another sandboxed plugin path.
- Add a brief but explicit section for `ToolPolicyManager` and dangerous-tool handling, because that is a real control surface that is currently easy to miss.
- Keep `src/NETWORK_SECURITY.md` as the deep reference, but link to it from the new canonical overview instead of duplicating its full detail everywhere.
- Avoid brittle claims like “nothing leaves your control” or “fully sandboxed” unless the doc is naming the exact component and constraint.

## Evidence Pointers

- [README.md](/Users/vespian/coding/ThinClaw-main/README.md)
- [CLAUDE.md](/Users/vespian/coding/ThinClaw-main/CLAUDE.md)
- [src/NETWORK_SECURITY.md](/Users/vespian/coding/ThinClaw-main/src/NETWORK_SECURITY.md)
- [src/config/safety.rs](/Users/vespian/coding/ThinClaw-main/src/config/safety.rs)
- [src/safety/mod.rs](/Users/vespian/coding/ThinClaw-main/src/safety/mod.rs)
- [src/safety/policy.rs](/Users/vespian/coding/ThinClaw-main/src/safety/policy.rs)
- [src/sandbox/config.rs](/Users/vespian/coding/ThinClaw-main/src/sandbox/config.rs)
- [src/sandbox/mod.rs](/Users/vespian/coding/ThinClaw-main/src/sandbox/mod.rs)
- [src/secrets/mod.rs](/Users/vespian/coding/ThinClaw-main/src/secrets/mod.rs)
- [src/config/secrets.rs](/Users/vespian/coding/ThinClaw-main/src/config/secrets.rs)
- [src/tools/policy.rs](/Users/vespian/coding/ThinClaw-main/src/tools/policy.rs)
- [src/tools/README.md](/Users/vespian/coding/ThinClaw-main/src/tools/README.md)
- [src/tools/mcp/config.rs](/Users/vespian/coding/ThinClaw-main/src/tools/mcp/config.rs)
- [src/tools/mcp/client.rs](/Users/vespian/coding/ThinClaw-main/src/tools/mcp/client.rs)
- [src/tools/wasm/allowlist.rs](/Users/vespian/coding/ThinClaw-main/src/tools/wasm/allowlist.rs)
- [docs/EXTENSION_SYSTEM.md](/Users/vespian/coding/ThinClaw-main/docs/EXTENSION_SYSTEM.md)
- [docs/EXTERNAL_DEPENDENCIES.md](/Users/vespian/coding/ThinClaw-main/docs/EXTERNAL_DEPENDENCIES.md)
