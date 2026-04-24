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
- Stores local secrets as AES-256-GCM ciphertext with authenticated metadata; current rows use v2 metadata and legacy rows must be re-entered
- Uses the OS secure store as the default master-key source; `SECRETS_MASTER_KEY` is ignored unless `THINCLAW_ALLOW_ENV_MASTER_KEY=1` or `secrets.allow_env_master_key = true`
- Keeps general OS keychain secret caching disabled by default; provider/API-key caching is opt-in via `THINCLAW_KEYCHAIN_CACHE=1`, and master-key caching is disabled unless explicitly enabled
- Uses policy and validation layers around dangerous tools and external content
- Adds a first-party pre-exec shell scanner ahead of approval for high-risk shell commands, with explicit fail-open or fail-closed operator control
- Supports network controls and allowlists
- Keeps execution-surface guarantees mode-aware: background `process` is disabled in restricted workspace modes, `execute_code` only runs in `sandboxed` mode when an actual Docker sandbox backend is available, and research `local_docker` trials use the same Docker-backed execution path
- Separates sandboxed extension paths from operator-trusted external paths
- Makes the gateway, channels, tools, and extension surfaces part of the security model
- Keeps reckless desktop autonomy explicit instead of implying it has the same trust profile as a normal local run

## Local Secrets

The default secrets backend is `local_encrypted`. Secret values live in the database only as encrypted blobs, while the master key is created and retrieved from the platform secure store:

- macOS: Keychain
- Windows: Credential Manager
- Linux desktop: Secret Service such as GNOME Keyring or KWallet

Headless Linux and containers may use `SECRETS_MASTER_KEY`, but only after explicitly enabling the fallback with `THINCLAW_ALLOW_ENV_MASTER_KEY=1` or `secrets.allow_env_master_key = true`. Treat this as a deployment exception: environment variables are easier to leak through process inspection, shell history, crash reports, and service managers.

The strict default settings are:

```toml
[secrets]
backend = "local_encrypted"
master_key_source = "os_secure_store"
allow_env_master_key = false
cache_ttl_secs = 0
strict_sensitive_routes = true
```

Use `thinclaw secrets status` or `thinclaw doctor` to check secure-store readiness, env-fallback risk, schema posture, and sensitive-route policy. Provider Vault writes require header/proxy authentication; query-string bearer tokens are rejected for credential write/delete routes.

Master-key rotation is exposed as `thinclaw secrets rotate-master`. The command decrypts active v2 rows with the current key, re-encrypts them with a newly generated OS-secure-store key, verifies decryptability, and advances the local key version. Existing legacy or incompatible rows are not silently decrypted after this hardening change; re-enter those credentials through Provider Vault or `thinclaw secrets set`.

Backups need both pieces: the database and the OS secure-store master key. Losing the secure-store item makes encrypted local secret values unrecoverable. A host compromise while ThinClaw is running can still access secrets that the trusted host is authorized to inject; encryption protects at-rest storage, not a fully compromised runtime.

The codebase now has a `SecretBackend` boundary for future remote backends. The first supported implementation remains `local_encrypted`; planned external implementations should store only metadata/references locally while preserving the same audit, leak-detection, Provider Vault, and sensitive-route policy path. The intended next providers are HashiCorp Vault KV v2, AWS Secrets Manager, and 1Password Connect.

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
- local encryption protects secrets from a compromised running host process
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
