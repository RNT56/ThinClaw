# Extension Process Isolation

This document separates **admission** (whether code may be used) from
**containment** (what happens after that code starts). A signature, allowlist,
OAuth token, MCP tool approval, or capability declaration can establish
identity and policy; none of those controls stops a crash or host syscall.

## Local stdio MCP

New local stdio servers use strict isolation by default:

| Boundary | macOS | Linux | Windows |
|---|---|---|---|
| Filesystem | Seatbelt denies writes globally and denies operator-home, removable-volume, network-volume, and shared-temp reads except the private launcher root, validated executable/runtime directories, and explicit file roots | Bubblewrap mount namespace; system and validated executable/runtime paths read-only, private home/temp, explicit file roots read/write | Strict mode unavailable (fails closed) |
| Network | Seatbelt deny by default | Separate network namespace by default | Compatibility mode has host network |
| Descendants | Inherit Seatbelt; owned process group is killed/reaped together | Inherit mount/network/PID namespaces; owned process group is killed/reaped together | Kill-on-close Job Object in compatibility mode |
| Environment | Ambient environment cleared; only validated public values, authorized secret slots, private HOME/TMP, and a canonical PATH are supplied | Same | Same in compatibility mode |

Strict network access can be enabled per server. Filesystem grants come from
canonical local paths in `roots_grants`; non-file MCP roots are never treated
as host mounts. File roots are read/write because MCP roots do not carry an
access-mode field. Restart the server after changing roots so the OS boundary
and protocol view agree.

Linux strict mode requires `bwrap`; macOS requires `/usr/bin/sandbox-exec`.
Missing enforcement is an authorization error, never a silent host fallback.
`stdio_isolation.mode = "compatibility"` is the explicit escape hatch. It
retains private environment construction, bounded protocol records, request
deadlines, and owned-tree cleanup, but grants direct host filesystem/network
access and must be treated as privileged execution.

Pre-v3 configs are not silently reinterpreted. They migrate into a blocked,
observable `migration_required` state representing their old compatibility
boundary. The operator must run one of:

```text
thinclaw extensions mcp server isolation NAME --strict
thinclaw extensions mcp server isolation NAME --compatibility
```

Isolation, network, timeout, and filesystem-mount policy are fixed at process
launch. Restart or reactivate an already-running server after changing them.

A child exit or crash releases pending callers. A request deadline marks the
transport unusable and terminates the entire tree, preventing late JSON-RPC
responses from corrupting later request state. Existing MCP authentication,
secret-source resolution, capability policy, roots policy, tool approval, and
tool authorization remain enforced above the process boundary.

## Native dynamic-library plugins

Native plugins currently have admission but no process containment. Signature,
ABI, artifact hash, and path-allowlist checks all occur before `dlopen`, but the
library then shares ThinClaw memory and host privileges. A caught Rust unwind
can be reported; an abort, native fault, or compromise can terminate or control
the host.

Consequently both settings are required:

```toml
[extensions]
allow_native_plugins = true
allow_unsafe_in_process_native_plugins = true
```

The first setting admits native contributions. The second separately accepts
the legacy in-process compatibility boundary. Both default to false and the
loader rechecks both immediately before `dlopen`. Prefer WASM or strict stdio
MCP whenever the integration can use either boundary; a future out-of-process
native host can remove the compatibility requirement without weakening
admission checks.
