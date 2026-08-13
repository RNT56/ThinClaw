# Core release operations

ThinClaw's core release is a staged, non-Apple delivery path. It publishes Linux CLI,
Windows CLI/MSI, edge binaries, WASM extensions, and the multi-architecture Linux
container. Signed Desktop and iOS artifacts remain in the separate Apple release lane.

## One-time repository configuration

1. Install a repository-scoped GitHub App with these repository permissions:
   - Metadata: read
   - Contents: read and write
   - Pull requests: read and write
   - Issues: read and write
   - Actions: read and write
2. Store its application ID as `RELEASE_APP_ID` and PEM private key as
   `RELEASE_APP_PRIVATE_KEY` in Actions secrets.
3. Create a `release-production` GitHub Environment. Require a maintainer reviewer,
   restrict deployment branches to `main`, and prevent self-review where supported.
4. Keep the repository-wide "Allow GitHub Actions to create and approve pull
   requests" option disabled. The narrowly scoped App owns release PRs and dispatch.
5. Enable immutable releases after the v0.16.0 backfill. The workflow already refuses
   to clobber an existing asset or versioned container tag.

## Safe dry run

Dispatch **Release** on `main` with the exact tag and `mode=dry-run`. Leave
`promote_latest=false`. This resolves the tag, checks that its package version agrees,
and asks cargo-dist to construct the platform plan. It does not build, upload, publish,
or change GHCR.

Local release-control tests are also non-publishing:

```bash
python3 scripts/ci/test_release_control.py
python3 scripts/ci/check-release-automation.py
python3 scripts/ci/release_control.py plan \
  --tag v0.16.0 --mode dry-run --root-version 0.16.0
python3 scripts/ci/extension_registry.py check-policy
python3 -m unittest scripts.ci.test_extension_registry
```

## Exact v0.16.0 backfill

Never move or recreate `v0.16.0`.

1. Record the current `v0.14.0`, `v0.15.0`, and `v0.16.0` release state plus GHCR
   digests. Confirm `v0.16.0^{commit}` is the intended 0.16.0 source commit.
2. Restore `v0.14.0` as GitHub/GHCR latest during the maintenance window if it is the
   last complete release. Keep incomplete v0.15.0 out of the latest channel.
3. First run a `v0.16.0` dry run.
4. Dispatch **Release** with `tag=v0.16.0`, `mode=backfill`, and
   `promote_latest=false`.
5. Review the `release-production` approval. Backfill mode may return the existing
   empty public release to draft, but it never moves the tag or overwrites an asset.
   If a versioned GHCR image already exists, its digest is preserved.
6. Verify the explicit-version downloads and both container architectures.
7. Run the same backfill with `promote_latest=true` only after verification.

If staging fails, leave v0.16.0 draft and restore the recorded v0.14.0 latest aliases.
Do not delete or retag evidence from the failed attempt.

## v0.16.1 and later

Release Please creates a protected release PR and, after merge, a draft release. Its
App-authenticated recovery step dispatches the artifact workflow. All outputs build in
Actions storage before a reviewer can stage the draft. The workflow then:

1. builds every extension twice from committed standalone lockfiles in clean target
   directories and rejects byte drift;
2. commits future-version URLs, bundle hashes, sizes, and the source digest to the
   release PR through the scoped Release App;
3. requires the protected PR CI to audit every standalone graph with cargo-deny;
4. rebuilds the tagged extension bundles and compares them with the committed hashes;
5. validates the non-Apple asset contract and uploads only missing, byte-identical
   assets;
6. publishes the immutable versioned container and versioned GitHub release without
   changing latest; and
7. promotes GitHub/GHCR latest only when explicitly requested and not a prerelease.

`v0.16.0` is the sole legacy exception because its immutable tag predates committed
tool lockfiles and prepared registry metadata. Its compatibility layer is hard-coded
to that exact tag. `v0.16.1` is the first strict lifecycle release; missing source,
lockfile, Dependabot entry, URL, hash, size, or preparation metadata fails before
publication. Never hand-edit prepared hashes or run a post-release registry update.

To reproduce the release-PR preparation locally without publishing, install the exact
component builder reported by `extension_registry.py tool-version`, build two isolated
sets, compare them, and prepare the release version on a clean release branch:

```bash
version="$(python3 scripts/ci/extension_registry.py tool-version)"
cargo install cargo-component --version "$version" --locked
THINCLAW_EXTENSION_BUILD_ONLY=1 CARGO_TARGET_DIR=/tmp/thinclaw-ext-target-1 \
  scripts/ci/build_extension_bundles.sh /tmp/thinclaw-ext-bundles-1
THINCLAW_EXTENSION_BUILD_ONLY=1 CARGO_TARGET_DIR=/tmp/thinclaw-ext-target-2 \
  scripts/ci/build_extension_bundles.sh /tmp/thinclaw-ext-bundles-2
python3 scripts/ci/extension_registry.py compare-bundles \
  --first /tmp/thinclaw-ext-bundles-1 --second /tmp/thinclaw-ext-bundles-2
python3 scripts/ci/extension_registry.py prepare \
  --tag "v$(awk -F'"' '/^version = "/ {print $2; exit}' Cargo.toml)" \
  --bundles /tmp/thinclaw-ext-bundles-1
```

For a failed published version, restore both latest aliases to the recorded prior tag
and ship a new patch version. Never replace bytes attached to an existing version.
