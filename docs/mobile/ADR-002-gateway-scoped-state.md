# ADR-002: Gateway-Scoped Local State

Accepted: 2026-07-10

Every sensitive local artifact is owned by the authoritative gateway instance
returned by pairing. Filesystem namespaces use a SHA-256 directory name derived
from that instance identifier. Transcript/outbox databases, snapshots, widget
receipts, and Watch mirrors must reject or clear data whose gateway ownership
cannot be proven.

The legacy unscoped transcript database is copied into the active namespace,
opened, and integrity-checked before its source is removed. If ownership is
unknown, the app does not guess. Unpair clears the namespace and shared mirrors.
