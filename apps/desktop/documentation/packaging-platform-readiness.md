# ThinClaw Desktop Packaging And Platform Readiness

Last updated: 2026-07-14

This checklist is the P3-W3 release-readiness gate for the macOS alpha. It records what is enforced by config or tests and what still requires release operator secrets or host prerequisites.

## Automated Gate

Run from `apps/desktop`:

```bash
npm run validate:packaging
```

The gate verifies:

- `tauri info` metadata is readable on the host.
- The app identity is `ThinClaw Desktop` / `com.thinclaw.desktop`.
- The Keychain service remains `com.thinclaw.desktop`, matching the bundle identifier.
- Updater artifacts are enabled and updater endpoint/public key metadata exists.
- macOS entitlements include sandbox, microphone, network client, network server, and user-selected file access.
- Engine-specific Tauri override generation declares the expected sidecars for cloud, Ollama, llama.cpp, MLX, and vLLM builds.
- The real Chromium/llama setup scripts pass isolated clean-machine fixtures, including checksum rejection and required-sidecar layout.
- Declared native runtimes, libraries, Chromium, and their total stay below the committed `sidecar-budgets.json` limits.
- Static updater metadata and macOS release artifact collection pass deterministic contract fixtures.
- Focused Keychain, legacy Scrappy fallback, iCloud fallback, and migration path tests pass.

The script preserves any existing `backend/tauri.override.json` after validation.

## Current `tauri info` Findings

The current local host reports:

- Full Xcode is not installed. Xcode Command Line Tools are installed, which is enough for normal local checks. Full Xcode is required for signing, notarization, and final macOS release packaging.
- Tauri package updates may be reported by `tauri info`. Patch-level Tauri updates are allowed during this hardening phase only after `cargo check --locked`, frontend typecheck, and `npm run build` pass.
- The Rust `tauri-cli` line can refer to a globally installed `cargo-tauri` binary. The repo-controlled JavaScript CLI is `@tauri-apps/cli` in `package-lock.json`; use `npm run tauri -- ...` or `npx tauri ...` for reproducible Desktop commands.
- `@tauri-apps/plugin-shell` and `@tauri-apps/plugin-global-shortcut` JavaScript packages may be absent. That is acceptable because Desktop uses those plugins from Rust, not from frontend JavaScript.

## macOS Identity

- Product name: `ThinClaw Desktop`
- Bundle identifier: `com.thinclaw.desktop`
- Keychain service: `com.thinclaw.desktop`
- Cloud encryption Keychain service: `com.thinclaw.desktop.cloud-key`
- Legacy readable paths remain fallback-only:
  - App support: `~/Library/Application Support/com.schack.scrappy`
  - iCloud container: `iCloud~com~scrappy~app`

New writes must use ThinClaw identifiers and ThinClaw storage roots.

## Sidecars And Resources

`scripts/generate_tauri_overrides.sh` owns the build-specific `externalBin` and resource list.

| Build | Required sidecars | Optional sidecars | Notes |
|---|---|---|---|
| `none` / cloud | none | none | Used for CI build smoke and remote/cloud-only packaging. |
| `ollama` | none | `whisper`, `whisper-server`, `tts` | Ollama itself is external and must not be bundled. |
| `llamacpp` | `llama-server` | `whisper`, `whisper-server`, `sd`, `tts` | Native local alpha default. |
| `mlx` | `uv` | `whisper`, `whisper-server`, `tts` | macOS Apple Silicon only. |
| `vllm` | `uv` | `whisper`, `whisper-server`, `tts` | Linux CUDA only. |

Chromium is included automatically when `backend/resources/chromium` exists. Set `INCLUDE_CHROMIUM=1` to require it in a release build, or `INCLUDE_CHROMIUM=0` to omit it deliberately. The release pipeline requires Chromium and packages only the core llama.cpp runtime installed by `setup:all`; optional voice/image sidecars are not downloaded or declared unless an operator explicitly installs them.

For a macOS llama.cpp release candidate:

```bash
npm run setup:all
INCLUDE_CHROMIUM=1 npm run tauri:build:llamacpp
```

For a local packaging smoke without updater signing secrets:

```bash
npm run tauri:build:cloud:unsigned
```

If `backend/bin` is empty, native sidecar builds fail in strict mode. That is intentional: run `npm run setup:ai` or an engine-specific setup script before packaging a native local build.

`npm run setup:all` downloads pinned archives, verifies them before replacing local assets, validates the extracted executables, generates the strict llama.cpp override, and enforces the sidecar budgets. `npm run test:setup:all` executes those same scripts against tiny deterministic archives without mutating the checkout.

Current limits are 512 MiB per native artifact, 1 GiB for native sidecars and libraries, 768 MiB for Chromium, and 1.5 GiB total bundled runtime. A deliberate increase requires changing `sidecar-budgets.json` in review.

## Local Inference Setup

- llama.cpp uses a bundled `llama-server-{target-triple}` sidecar.
- MLX and vLLM use the bundled or discovered `uv-{target-triple}` binary and first-launch Python bootstrap.
- Ollama uses an external daemon and should expose read/status UI when the daemon is absent.
- Cloud-only builds use no local inference sidecars.

The alpha macOS release target is llama.cpp on Apple Silicon. MLX remains gated to macOS Apple Silicon, and vLLM remains gated to Linux CUDA.

## Updater And Notarization

Configured:

- `bundle.createUpdaterArtifacts = true`
- Updater endpoint points to the GitHub release `latest.json`
- Updater public key is present in `tauri.conf.json`
- macOS entitlements are configured through `backend/Entitlements.plist`

Automated tag-release behavior:

- Apple Silicon runs on GitHub's `macos-15` Arm64 image.
- The workflow imports an ephemeral Developer ID Application certificate keychain.
- Tauri signs, notarizes, and staples the app/DMG and signs the updater archive.
- Post-build checks require `codesign`, Gatekeeper (`spctl`), and `stapler validate` to pass.
- `latest.json` embeds the `.sig` contents under `darwin-aarch64` and is uploaded with the DMG/updater archive.
- The cargo-dist host job cannot publish unless the Desktop job succeeds.

Release operator prerequisites:

- Provision the exact GitHub Actions secrets listed in `external-release-prerequisites.md`.
- Trigger a product tag whose version matches the root Cargo package.
- Perform first-release clean-machine launch acceptance on the uploaded DMG.

Regular `tauri:build:*` scripts keep updater artifacts enabled and require `TAURI_SIGNING_PRIVATE_KEY`. Use only the `:unsigned` smoke script when validating packaging on a workstation without release signing secrets.

Do not commit release private keys, Apple credentials, generated `.app` bundles, or notarization artifacts.

## Platform Gates

- iCloud Drive uses local filesystem roots. Native entitlement container work requires release-operator entitlement validation; legacy Scrappy iCloud roots are read-only fallback paths.
- Autonomy execution remains disabled unless explicit reckless desktop config and host permission checks allow it.
- GPU cloud experiment launch/test actions must remain unavailable with concrete reasons unless the gateway/API and required secrets are configured.
- Remote mode must never expose raw provider secrets; only save, delete, and status capabilities are allowed.
