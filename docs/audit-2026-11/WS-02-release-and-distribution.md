# WS-02 — Release, Distribution & Deploy Hardening

| | |
|---|---|
| **Priority** | P0 (2.1, 2.2, 2.6) · P1 (2.3–2.5) · P2 (2.7–2.12) |
| **Effort** | M |
| **Status** | ⏳ Not started |
| **Depends on** | WS-01 (green `main`) · decision **D8** |
| **Findings** | OPS-1 … OPS-11, H7 |
| **Canonical docs** | `docs/RELEASING.md`, `docs/RELEASE_OPERATIONS.md`, `docs/BUILD_PROFILES.md`, `docs/DEPLOYMENT.md`, `docs/deploy/*`, `README.md`, `CHANGELOG.md` |

## Goal

Every documented install path works and installs current code; release jobs run with least privilege; binaries are verifiable.

Baseline: the [v0.16.0](https://github.com/RNT56/ThinClaw/releases/tag/v0.16.0) and v0.15.0 releases have **0 assets** (v0.14.0 has 64). `releases/latest/download/thinclaw-installer.sh` → 404. GHCR `:latest` digest equals `:v0.14.0` (2026-05-14); `:v0.15.0`/`:v0.16.0` → 404. 45 commits since 2026-08-02 are unreleased.

## Tasks

- [ ] **2.1 — Restore an installable release (OPS-1).** Per D8: backfill v0.16.0 via `docs/RELEASE_OPERATIONS.md:42-59` or cut v0.16.1 from green `main`. Find and fix why the v0.15/v0.16 runs published no assets (release run logs; `release-credential-gate`).
  *Accept:* release has the full asset set from `release/non-apple-assets.json`; installer URL returns 200; `ghcr.io/rnt56/thinclaw:<new>` exists and `:latest` points to it.

- [ ] **2.2 — Ship the project installer again (OPS-2).** bfdc25e removed `cp scripts/thinclaw-installer.sh artifacts/thinclaw-installer.sh`; `scripts/ci/release_control.py:~169` now picks dist's generated installer. Restore the custom installer (or drop dist's `shell` installer in `Cargo.toml` `[workspace.metadata.dist]` `installers`), and remove darwin from dist `targets` (`Cargo.toml:~460`) while `release/non-apple-assets.json` forbids `*apple-darwin*`. Add a contract test asserting the published installer equals `scripts/thinclaw-installer.sh` and supports `--profile edge`, `--system`, `--static`. Reconcile `install-updater=true` with the asset contract.
  *Accept:* contract test in `scripts/ci/`; `mac-deploy.sh` / `docs/deploy/macos.md:~91` default path works or is documented as unsupported.

- [ ] **2.3 — Least-privilege release workflow (OPS-3 / H7).** `.github/workflows/release.yml:17-19` grants `contents: write` + `packages: write` to all jobs; `GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}` is exported into compile jobs (`build-local-artifacts`, `build-wasm-extensions`, `build-edge-artifacts`, ≈`:200`, `:345`, `:825`). Set top-level `contents: read`; grant write only to plan/host/publish/GHCR-promote jobs; remove `GH_TOKEN` from any job that compiles third-party code.
  *Accept:* `python3 scripts/ci/check-release-automation.py` extended to assert the permission split.

- [ ] **2.4 — No unverified `curl | sh` (OPS-4).** `release.yml:~103` (cargo-dist installer), `:~222` (rustup), `:~229`. Pin by SHA-256 in-repo or use `cargo install cargo-dist --locked --version 0.31.0`; use the pinned toolchain action for rustup.

- [ ] **2.5 — Signatures and provenance (OPS-5 / H7).** Add `actions/attest-build-provenance` and/or cosign/minisign for CLI tarballs and MSI; have `scripts/thinclaw-installer.sh` and `deploy/setup.sh:~850` verify a signature, not just a checksum fetched from the same release; enable immutable releases (`docs/RELEASE_OPERATIONS.md:~21`).

- [ ] **2.6 — Interim install-path banner.** Until 2.1 lands, mark the one-liners in `README.md:~40, 338, 345`, `docs/BUILD_PROFILES.md:13-27`, `docs/deploy/linux.md:~76`, `deploy/docker-compose.yml:~15`, `deploy/setup.sh:59-60` as temporarily broken and give a build-from-source path. Remove when 2.1 is done.

- [ ] **2.7 — Deploy hardening (OPS-7 … OPS-10).**
  - Root `docker-compose.yml:5-10`: publish Postgres as `127.0.0.1:5432:5432`; digest-pin `pgvector/pgvector`.
  - `Dockerfile:~14`: digest-pin `debian:bookworm-slim`, snapshot-pin apt, add `HEALTHCHECK`, drop curl/bash from runtime if unused (match `Dockerfile.worker`).
  - Native systemd unit written by `deploy/setup.sh:~977-996`: add `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ProtectHome=read-only` (+ explicit `ReadWritePaths`), `PrivateTmp=yes`, `RestrictSUIDSGID=yes`, `LockPersonality=yes`. Retire or pin legacy `deploy/thinclaw.service` (`--network=host`, mutable `:latest`).
  - Pin `cargo install cargo-deny` (`ci.yml:~107` + desktop job) and `cargo-fuzz` versions.
  - `ios.yml:~302-347`: put the TestFlight lane behind a protected `environment:`.

- [ ] **2.8 — Integration-test evidence check (OPS-11).** `scripts/ci/integration-tests.json` maps `desktop_autonomy_live_smoke` to `nightly.yml`, satisfied by the comment at `nightly.yml:~18` that says it is *not* run. Make the evidence check structural (job/step reference), and either schedule the 6 ignored cases or mark them explicitly unscheduled.

- [ ] **2.9 — Release-credential gate clarity.** The gate is a configuration-readiness check, not a security gate; it only blocks if branch protection requires "Release Credentials". Confirm branch protection, and fix `docs/RELEASING.md:20-31` (says GITHUB_TOKEN + Apple cert validation; code uses an App token and checks App credentials only; Apple signing is disabled at `release.yml:~589-593`).

- [ ] **2.10 — CHANGELOG hygiene.** Remove the orphaned `## [Unreleased]` block at `CHANGELOG.md:~167` (between 0.15.0 and 0.14.0); make sure release-please owns the next entry.

- [ ] **2.11 — Security advisory for the release that carries WS-03…WS-06.** Per D10, publish a GitHub Security Advisory covering C1–C4 with fixed version and upgrade guidance.

- [ ] **2.12 — Post-release smoke.** Script the checks from 2.1 (asset count vs contract, installer URL 200, GHCR tag + `latest` digest) as a post-release job so a zero-asset release can't happen silently again.

## Verification (workstream exit)

- Fresh VM: documented curl installer, `deploy/setup.sh` (native + docker), and `docker compose` all install the new version.
- `release.yml` permission split asserted by `check-release-automation.py`.
- Signature verification exercised by the installer in CI.
