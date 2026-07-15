# Releasing ThinClaw

This is the maintainer runbook for publishing a ThinClaw release. A release is
complete only when the GitHub release contains the expected host artifacts and
the signed Desktop update metadata. A tag or an empty GitHub release is not an
acceptable finish line.

## Before Merging The Release PR

Release Please opens or updates the version PR after conventional commits land
on `main`. Review all of these before merging it:

1. The changelog and root version are correct.
2. Root Cargo, both Desktop Cargo files and lockfiles, Desktop package files,
   and `tauri.conf.json` all carry the same version.
3. The PR-associated CI run is green.
4. Every required release secret below is present.

Release Please authenticates with `GITHUB_TOKEN`. GitHub creates the normal
`pull_request` CI run for that automated PR in an `action_required` state. A
maintainer must inspect and approve that pending run in GitHub Actions. A
manually dispatched CI run can test the same commit, but GitHub does not attach
it to the pull request and it cannot satisfy branch protection.

Use these commands for a quick read-only check:

```bash
gh pr view <release-pr> --repo RNT56/ThinClaw \
  --json headRefOid,mergeable,mergeStateStatus,statusCheckRollup
gh run list --repo RNT56/ThinClaw \
  --branch release-please--branches--main--components--thinclaw
gh secret list --repo RNT56/ThinClaw
```

Do not print, copy into logs, or paste secret values into a pull request or chat.

## Required GitHub Actions Secrets

The signed macOS Desktop build fails closed before host artifacts are published
when any required value is missing or invalid.

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application `.p12` certificate |
| `APPLE_CERTIFICATE_PASSWORD` | Password used when exporting that `.p12` |
| `APPLE_ID` | Apple ID used for notarization |
| `APPLE_PASSWORD` | App-specific password for that Apple ID |
| `APPLE_TEAM_ID` | Apple Developer team identifier |
| `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater private key matching the public key embedded in `tauri.conf.json` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Private-key password when the updater key is encrypted; otherwise omit it |

Treat the Developer ID certificate and updater private key as recoverable
release infrastructure. Back them up in the project password manager before
adding repository secrets. Rotating the updater key also requires a deliberate
client migration; do not silently generate a replacement during a release.

## Publish And Verify

1. Merge the green Release Please PR.
2. Confirm Release Please creates the expected `vX.Y.Z` tag and GitHub release.
3. Confirm the explicitly dispatched `Release` workflow succeeds for that tag.
4. Inspect the release itself. Verify host archives/installers, checksums,
   packaged WASM extensions, the notarized/stapled macOS Desktop DMG, the Tauri
   updater archive and signature, and `latest.json` are present.
5. Download representative artifacts and run the documented install/update
   smoke before announcing the release.

If an artifact job fails, keep the release unannounced, correct the underlying
credential or pipeline failure, and rerun the existing tag through the
workflow's `tag` input. Do not create a second tag to hide a broken release.
