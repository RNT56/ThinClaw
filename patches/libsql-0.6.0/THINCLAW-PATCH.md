# ThinClaw libSQL patch

This vendored `libsql` 0.6.0 package carries compatibility and correctness
fixes required by the ThinClaw workspace.

## Connection lifecycle

The upstream local wrapper closed a raw SQLite handle from
`LibsqlConnection::drop` and then closed it a second time when its wrapped
`Connection` field was dropped. Under parallel database workloads, the double
close could surface later as nondeterministic `SQLITE_MISUSE` query failures.

ThinClaw removes the redundant wrapper close, makes explicit disconnects
idempotent, and tests both repeated disconnects and cloned-handle lifetime.

The upstream defect is tracked by
[`tursodatabase/libsql#2251`](https://github.com/tursodatabase/libsql/issues/2251)
and proposed fix
[`#2261`](https://github.com/tursodatabase/libsql/pull/2261). The local patch
also tolerates another linked SQLite client having initialized a thread-safe
runtime first; that compatibility hunk must be reviewed independently of the
double-close fix.

The canonical owner, crates.io base checksum/revision, reproducible fork
fingerprint, review date, security procedure, and full removal gate live in
`../manifest.json` and are enforced by
`scripts/ci/check-vendored-patches.py`.
