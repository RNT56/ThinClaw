# ThinClaw security backport

This local `0.13.1` package is the crates.io `libsql-sqlite3-parser` 0.13.0
source with the upstream invalid-UTF-8 fix backported from
`gwenn/lemon-rs@14f422a0dc7da2a2a23dcb76235559a6cd9d1647`.

The one-line change replaces `str::from_utf8_unchecked` with
`String::from_utf8_lossy`, closing CVE-2025-47736 / GHSA-8m95-fffc-h4c5.
Remove this patch once libsql consumes an upstream parser release containing
that commit.
