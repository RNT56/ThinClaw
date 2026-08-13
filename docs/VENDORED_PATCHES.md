# Vendored Cargo Patch Policy

ThinClaw carries local Cargo patches only when a released dependency graph
cannot safely or reproducibly provide the required code. The authoritative
inventory is [`patches/manifest.json`](../patches/manifest.json). It records,
for every `[patch.crates-io]` entry:

- the owning maintainer and next mandatory review date;
- the exact crates.io version, archive checksum, and upstream VCS revision;
- upstream issues, pull requests, commits, advisories, or releases;
- the reason the patch still exists, its security-update procedure, and the
  tests required before removal;
- a deterministic fingerprint and exact changed-path set relative to the
  checksum-pinned crates.io archive.

Run the local structural check before editing any patched dependency:

```bash
python3 scripts/ci/check-vendored-patches.py
```

CI also downloads each pinned crates.io archive and verifies its checksum and
fork fingerprint:

```bash
python3 scripts/ci/check-vendored-patches.py --verify-upstream
```

The structural check fails when an unrecorded path patch is added, a recorded
patch disappears, required ownership/upstream/removal data is missing, or a
review date expires. The upstream check additionally fails when source files
drift without an explicit inventory update. `THINCLAW-PATCH.md`, registry
metadata, generated locks, repository workflow files, and formatting-only
vendor metadata are excluded; runtime/package-source changes are not.

## Review and update procedure

1. Review every entry no later than its `review_on` date and immediately after
   a matching RustSec advisory or upstream release.
2. Test whether its `removal_condition` is now satisfied. Prefer deleting the
   patch and refreshing all affected lockfiles.
3. If it must remain, start from the pinned upstream archive, apply only the
   required hunks, update upstream links/status, move the review date by no more
   than 45 days, and regenerate the fingerprint.
4. Run `cargo deny` for every affected lock graph and the patch-specific test
   gates named in the inventory. Security advisories apply to both upstream and
   locally changed code; a path dependency is never an advisory exemption.
5. Do not use a local patch to hide an incompatible dependency upgrade. Track
   the upstream migration explicitly and make the removal gate executable.

The glib and parser fixes already have merged upstream fixes. The libSQL
double-close has an open upstream issue and pull request. The Telegram
`glass_pumpkin` mirror carries no ThinClaw source diff; it is a reproducibility
bridge until the `grammers-*` family can move atomically to the current line.
