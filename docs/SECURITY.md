# ThinClaw Security And Trust

ThinClaw's safety model is layered, but not every surface has the same trust boundary.

This page is the short public-facing overview. For deeper network and runtime detail, use [../src/NETWORK_SECURITY.md](../src/NETWORK_SECURITY.md).

## The Core Idea

ThinClaw tries to keep control in the host runtime and make trust boundaries explicit.

That means:

- sandboxing where sandboxing makes sense
- host-managed secret handling
- policy and allowlist controls around tools and network access
- explicit operator trust decisions for external integrations

## Runtime Trust Boundaries

| Surface | Trust Model |
|---|---|
| Native runtime code | trusted host runtime |
| WASM tools | sandboxed and capability-scoped |
| WASM channels | package-based and host-managed |
| MCP servers | operator-trusted external processes or services |
| External providers and APIs | explicit data egress paths when configured |
| `desktop_autonomy.profile = "reckless_desktop"` | privileged host-level desktop control with managed code rollout/rollback |

Do not treat all integrations as if they had the same isolation guarantees.

## What ThinClaw Does

- Keeps secret injection at the host boundary rather than exposing raw values to WASM guests
- Uses policy and validation layers around dangerous tools and external content
- Adds a first-party pre-exec shell scanner ahead of approval for high-risk shell commands, with explicit fail-open or fail-closed operator control
- Supports network controls and allowlists
- Keeps execution-surface guarantees mode-aware: background `process` is disabled in restricted workspace modes, `execute_code` only runs in `sandboxed` mode when an actual Docker sandbox backend is available, and research `local_docker` trials use the same Docker-backed execution path
- Separates sandboxed extension paths from operator-trusted external paths
- Makes the gateway, channels, tools, and extension surfaces part of the security model
- Keeps reckless desktop autonomy explicit instead of implying it has the same trust profile as a normal local run

## Desktop Autonomy Trust Boundary

Desktop autonomy is intentionally a stronger trust grant than ordinary local execution.

When `desktop_autonomy.profile = "reckless_desktop"` is enabled, ThinClaw may:

- open, focus, and quit local applications
- inspect accessibility trees and visible windows
- capture screenshots and OCR evidence
- send keyboard and pointer input through the desktop automation bridge
- manipulate native productivity apps through first-class adapters
- promote and roll back managed ThinClaw builds through the local autorollout path

That means this profile should be treated as privileged operator mode, not as a sandboxed extension surface.

Important boundaries:

- desktop autonomy evidence may include screenshots, OCR text, exported files, and action metadata
- desktop autonomy code self-improvement is limited to the managed autonomy source/build tree, not arbitrary in-place mutation of the running checkout
- one-time platform permission approval is still required before full autonomy begins
- dedicated-user mode still depends on a real GUI login for that target user

## What ThinClaw Does Not Claim

ThinClaw does not claim that:

- all configured integrations are sandboxed
- all data always stays local once you configure external providers or remote services
- MCP servers have the same trust profile as WASM tools
- host-local execution with `allow_network = false` is universally the same across platforms; today hard host-local no-network enforcement is available on macOS via `sandbox-exec` and on Linux via `bwrap` when it is installed, while the Docker-backed sandbox path provides the portable hard guarantee and unsupported host-local platforms are surfaced as best-effort through runtime metadata
- reckless desktop autonomy is equivalent to standard local execution; it is materially more powerful and should be enabled only on machines and accounts you intentionally grant host control to

Those distinctions are part of the product design and should stay visible in the docs.

## Deep References

- [DESKTOP_AUTONOMY.md](DESKTOP_AUTONOMY.md)
- [../src/NETWORK_SECURITY.md](../src/NETWORK_SECURITY.md)
- [EXTENSION_SYSTEM.md](EXTENSION_SYSTEM.md)
- [CHANNEL_ARCHITECTURE.md](CHANNEL_ARCHITECTURE.md)
- [../src/tools/README.md](../src/tools/README.md)
- [../src/setup/README.md](../src/setup/README.md)
