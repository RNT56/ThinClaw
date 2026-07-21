# ADR-003: Durable Authoritative Approvals

Accepted: 2026-07-10

The gateway owns a durable pending-approval registry populated at the central
approval-created lifecycle boundary and reconciled at startup. `GET
/api/chat/approvals` is a complete, oldest-first authoritative snapshot.

Clients replace their pending set from that snapshot while retaining only a
locally in-flight decision. Resolution from desktop, Watch, notification action,
or another phone therefore removes the entry on refresh. Repeated decisions are
treated as idempotent terminal outcomes, not permanent client failures.
