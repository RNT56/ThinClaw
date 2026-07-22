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
