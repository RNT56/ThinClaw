# Desktop Security And Dependency Baseline

> Verified 2026-07-13. This is the executable security baseline for the
> Tauri renderer boundary and Desktop dependency graph.

## Renderer capability boundary

Tauri capabilities are additive across every window they match, so Desktop
uses separate, window-specific files:

| Window | Renderer permissions |
|---|---|
| `main` | Open default web URLs; core event/window APIs; the filesystem plugin's app-owned default directories; write/existence checks for direct children of Downloads; updater and process APIs |
| `spotlight` | Core event and window APIs only |

The renderer does **not** receive shell execution, arbitrary file opening,
home-wide filesystem access, or machine-specific filesystem paths. Opening a
model directory, revealing the active workspace, and revealing chat files all
cross typed Rust commands. File reveal canonicalizes the requested path and
permits only existing targets inside app data or the active workspace, which
also rejects symlink escapes.

`scripts/validate_packaging_readiness.sh` fails if these boundaries drift.
This follows Tauri's guidance to grant only required permissions and to scope
filesystem commands narrowly:

- [Tauri capabilities](https://v2.tauri.app/security/capabilities/)
- [Tauri permissions](https://v2.tauri.app/security/permissions/)
- [Tauri filesystem plugin](https://v2.tauri.app/plugin/file-system/)
- [Tauri opener plugin](https://v2.tauri.app/plugin/opener/)

## Dependency baseline

The refresh intentionally updates direct contracts and then resolves the full
compatible graph:

- npm: `react-dropzone` 17; `npm audit` reports zero vulnerabilities.
- Tauri: 2.11.5 and current compatible plugin patches.
- ONNX Runtime: `ort` 2.0.0-rc.12, using its current tensor API.
- OpenDAL: 0.58, which moves XML request signing onto patched `quick-xml` 0.41.
- QUIC: `quinn-proto` 0.11.16.
- SQLx: defaults disabled; only Tokio, native TLS, SQLite, derive, and migration
  support are enabled. Migrations are embedded without the broad query macro.

`cargo deny check advisories` reports no advisory in the enabled Desktop graph
and the prior `quick-xml` advisory exceptions have been deleted. No RustSec
advisory is ignored. A raw lockfile-only `cargo audit` still reports
RUSTSEC-2023-0071 through SQLx's optional MySQL package; `cargo tree --target
all -i rsa@0.9.10` is empty because Desktop enables SQLite only. The upstream
RSA advisory has no patched release, and none of that package is compiled or
reachable by ThinClaw.

RustSec also reports informational, upstream maintenance warnings in the
Tauri Linux GTK3/WebKit dependency family and a small set of transitive
libraries. They are not suppressed; the enabled-graph CI audit remains the
release gate so a newly reachable vulnerability fails closed.

## Reproduce

```bash
cd apps/desktop
npm audit
cargo deny --manifest-path backend/Cargo.toml check advisories
cargo tree --manifest-path backend/Cargo.toml --target all -i rsa@0.9.10
bash scripts/validate_packaging_readiness.sh
```
