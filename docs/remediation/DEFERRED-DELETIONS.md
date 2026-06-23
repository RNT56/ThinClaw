# Deferred Deletions Ledger

> Per the execution directive **"wire now, batch deletions for review,"** no product code is deleted during the build waves. Every removal a fix would normally make is recorded here instead, and applied together in a single reviewable **Wave 4 deletion PR** after the operator signs off.
>
> Each entry: what, where, why it's safe to remove, what must be true first, and the owning workstream.

## How to use this ledger
- Implementation waves **add** the replacement/guard and leave the old code in place, recording it here.
- Before Wave 4, the operator reviews this list and approves/rejects each deletion.
- Wave 4 applies the approved deletions in one PR, re-running the full gate.

---

## Wave 0 (WS-01 / WS-02)

### D-01 — Dead HTTPS credential default-mappings (sandbox proxy)
- **What:** the three HTTPS entries in `default_credential_mappings()` — `OPENAI_API_KEY`→`api.openai.com` (bearer), `ANTHROPIC_API_KEY`→`api.anthropic.com` (x-api-key header), `NEARAI_API_KEY`→`api.near.ai` (bearer).
- **Where:** `src/sandbox/config.rs:11-15` (and the mirror that delegates to it, `src/sandbox/mod.rs:123-124`).
- **Why safe:** the proxy's in-band credential injection only fires on the plaintext-HTTP forward path; HTTPS is tunneled via `CONNECT`/`handle_connect`, which never injects. So these HTTPS mappings are unreachable dead defaults (audit Finding #7). HTTPS credentials are delivered out-of-band via the orchestrator `/worker/{id}/credentials` endpoint.
- **Precondition:** confirm no deployment relies on these defaults via the (now store-backed) HTTP injection path; confirm the OOB endpoint covers the three providers.
- **Owner:** WS-01 (recorded) → Wave 4 deletion. Decision register Decision-1 Option A.

---

## Notes on non-deletions (recorded so they are not mistaken for deferred deletions)
- **WS-01 `file.rs` containment:** the planned cwd-containment-when-`None` was **not** shipped (it broke the deliberate trusted-operator "unrestricted when no base_dir" contract — `register_filesystem_tools` has an explicit no-base branch). Instead containment stays fail-closed only when a base is configured, and registration now `warn!`s when no base is set. This is a grounded override of the decision register's cwd-containment choice, not a deferred deletion. See the Wave 0 report.
- **WS-02 `schema_divergence` strict mode:** new type/nullability/index comparisons are implemented but gated behind `SCHEMA_DIVERGENCE_STRICT=1` until a live-DB seeding pass records the genuinely-intended Postgres-vs-libSQL divergences (7 Postgres partial indexes libSQL lacks, etc.). Not a deletion; a seeding follow-up owned by WS-13.

---

*Add new entries under the owning wave as they arise.*
