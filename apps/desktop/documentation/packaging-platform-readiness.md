# ThinClaw Desktop Packaging And Platform Readiness

Last updated: 2026-07-29

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
- macOS entitlements include microphone, network client, network server, and
  user-selected file access, while explicitly excluding the Mac App Store
  sandbox that would be applied to—and break—the bundled runtime helpers.
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
| `llamacpp` | `llama-server` | `whisper`, `whisper-server`, `sd`, `tts` | Default outside Apple Silicon macOS. |
| `mlx` | `uv` | `whisper`, `whisper-server`, `tts` | Default on Apple Silicon macOS. |
| `vllm` | `uv` | `whisper`, `whisper-server`, `tts` | Linux CUDA only. |

Chromium is included automatically when `backend/resources/chromium` exists. Set `INCLUDE_CHROMIUM=1` to require it in a release build, or `INCLUDE_CHROMIUM=0` to omit it deliberately. The macOS release pipeline requires Chromium and builds the Apple Silicon MLX profile with macOS 14 as its minimum; optional voice/image sidecars are not downloaded or declared unless an operator explicitly installs them.

For the default macOS Apple Silicon release candidate:

```bash
THINCLAW_DESKTOP_ENGINE=mlx npm run setup:all
INCLUDE_CHROMIUM=1 npm run tauri:build:mlx
```

For a local packaging smoke without updater signing secrets:

```bash
npm run tauri:build:cloud:unsigned
```

For a launchable local Apple Silicon MLX build:

```bash
APPLE_SIGNING_IDENTITY=- DISABLE_UPDATER_ARTIFACTS=1 INCLUDE_CHROMIUM=1 npm run tauri:build:mlx
bash scripts/verify_macos_mlx_bundle.sh backend/target/release/bundle/macos
```

The ad-hoc signature is for local validation only; tagged releases still
require the Developer ID and notarization credentials enforced by CI.

If `backend/bin` is empty, native sidecar builds fail in strict mode. That is intentional: run `npm run setup:ai` or an engine-specific setup script before packaging a native local build.

`npm run setup:all` resolves the platform default (MLX on Apple Silicon macOS, llama.cpp elsewhere), downloads pinned archives, verifies them before replacing local assets, validates the extracted executables, generates the matching strict override, and enforces the sidecar budgets. `npm run test:setup:all` executes deterministic setup fixtures and engine-resolution tests without mutating the checkout.

Current limits are 512 MiB per native artifact, 1 GiB for native sidecars and libraries, 768 MiB for Chromium, and 1.5 GiB total bundled runtime. A deliberate increase requires changing `sidecar-budgets.json` in review.

## Local Inference Setup

- llama.cpp uses a bundled `llama-server-{target-triple}` sidecar.
- MLX and vLLM use the bundled `uv-{target-triple}` binary and backend-owned,
  hash-locked first-launch provisioning. A system `uv` is accepted only under
  the explicit development override `THINCLAW_ALLOW_SYSTEM_UV=1`.
- Ollama uses an external daemon and should expose read/status UI when the daemon is absent.
- Cloud-only builds use no local inference sidecars.

The macOS release target is MLX on Apple Silicon. llama.cpp remains an explicit
builder option on its reviewed targets, and vLLM remains gated to Linux x64
CUDA hosts that pass its preflight.

## Updater And Notarization

Configured:

- `bundle.createUpdaterArtifacts = true`
- Updater endpoint points to the GitHub release `latest.json`
- Updater public key is present in `tauri.conf.json`
- macOS Developer ID entitlements are configured through
  `backend/Entitlements.plist`. App Sandbox is intentionally absent because
  this app provisions and launches signed external runtime helpers; hardened
  runtime, notarization, and Gatekeeper validation remain required.

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
