# Coverage Critic Report — Remediation Plan Gate

> Reviewed AUDIT-FINDINGS.md against all 13 WS docs on 2026-06-23. Verified anchors against code where contested.

## DAG_OK: yes

No cycles. Dependency edges form a strict partial order:
- Behavior WSs (WS-01..WS-09) have `depends_on: none`.
- WS-10 (overhaul) depends_on WS-01..WS-09 (land behavior first); blocks none.
- WS-11 (dead-code) depends_on WS-05 + WS-10; blocks none.
- WS-13 (test/CI) depends_on WS-01 + WS-02; blocks none.
- WS-02 blocks WS-13. WS-05 blocks none but is depended-on by WS-10 + WS-11. WS-07 blocks WS-10 + WS-13. WS-04 blocks WS-10. WS-09 blocks WS-10.
- WS-12 (docs) trails all, depends_on none.

Every depended-on WS sits earlier in the order than its dependent; no back-edge. Acyclic.

## UNCOVERED findings

1. **`image_gen.rs:700` divide-by-zero progress label** (AUDIT §3 lower-severity, `apps/desktop/backend/src/image_gen.rs:700-712`, confirmed: `let progress = current / total` with no zero-guard) — **ORPHANED.** WS-04 explicitly disclaims it ("not WS-04; note it but do not touch") and routes it to "whichever WS owns the desktop media/image-gen surface" — but no WS owns that surface. WS-12 only references `image_gen.rs` for Scrappy env-var must-not-touch lines. No task fixes it. **Assign to WS-04** (it is in `apps/desktop/backend`, which WS-04 exclusively owns and is the only WS that can build that excluded package).

2. **`build-all.sh` never builds `tools-src` + `channel-crates` CI matrix omits the 13 shims** (AUDIT §4 P2 build-ergonomics; §9). WS-03 T6 *hands* this requirement to WS-13, but **WS-13's Owns block and tasks T1–T6 cover none of it** — WS-13 owns only the nightly E2E job, db-contract/schema-divergence gating, and clippy-flag verification. `scripts/build-all.sh:80-83` and the `ci.yml:725-751 channel-crates` matrix are owned by no WS. **The hand-off lands nowhere.** Add a WS-13 task (or assign `scripts/build-all.sh` + `channel-crates` matrix expansion to WS-13's Owns) so the 13 shims + `tools-src` get a `wasm32-wasip2` compile gate.

3. **`JobToolHostPort` half stubbed `Unavailable`** (AUDIT §2 subsystem table: "Tools core & registry … structured `JobToolHostPort` half is stubbed `Unavailable`"). No WS doc mentions `JobToolHostPort`. Either a deliberate omission (minor, structured-tool-host path) or an oversight — **flag for operator: decide wire-or-document; currently uncovered.**

4. **`wasm-runtime` absent from the desktop profile** (AUDIT §4 P2: "wasm-runtime in desktop profile or document"; §2 WASM-tools row). WS-01 notes `wasm-runtime` is in light/full/all-features/bundled but NOT edge — it does not address the *desktop* profile gap. WS-03/WS-04 don't touch it. **Uncovered** — assign to WS-04 (desktop profile/feature owner) or WS-12 (document the deliberate omission).

5. **DNS-rebinding "claimed but not enforced" framing vs. OAuth `state` at `oauth_defaults.rs:347`** — both ARE covered (WS-01 T8 pins the IP in both url_guard.rs + wrapper.rs; WS-01 T9 validates OAuth state in `wait_for_callback`). No gap; noted only because the AUDIT §8 anchor `oauth_defaults.rs:347` differs from WS-01's `:303/:142` anchors — same function region, confirmed covered.

All P0/P1 confirmed bugs (#1–#8), the four named lower-severity bugs (finish_reason→WS-08, image_gen→**orphan above**, routine break-on-error→WS-09 T2, child-session leak→WS-04 T8), every overhaul candidate, every dead-code item, every drift item, and the remaining §8 security items (execute_code→WS-01 T10, DNS-rebind→WS-01 T8, OAuth state→WS-01 T9, filesystem containment→WS-01 T11, shell fail-open→WS-01 T12) are covered. The refuted DATABASE_URL finding is correctly not actioned anywhere.

## CONFLICTS

1. **`channels-src/discord/README.md:105-108`** — WS-03 (lists it in Owns + T2 fixes it) **vs** WS-12 (T9 also edits it). Both docs explicitly coordinate ("README edit coordinated with WS-12" / "WS-12 corrects the README in the same PR"). **Resolved-by-coordination, but two tasks edit the same lines.** Recommended owner: **WS-03 lands the README edit in the same PR as the code** (satisfies CLAUDE.md same-PR rule); WS-12 T9 becomes a verify-only fallback if WS-03 ships without it. Make WS-12 T9 conditional, not an independent edit.

2. **`src/cli/session_export.rs` + `src/voice_wake.rs`** — WS-11 (T2 deletes `session_export.rs`; T7 wires-or-erases `voice_wake.rs`) **vs** WS-12 (T8 edits a "Scrappy" doc-comment at `session_export.rs:188` and `voice_wake.rs:16`). If WS-11 deletes/keeps these files, WS-12's comment edits are wasted or conflict. **Owner: WS-11** for the file fate; **WS-12 must sequence T8 after WS-11's T2/T7 land** and skip any file WS-11 deleted (`session_export.rs` is recommended-ERASE; `voice_wake.rs` is recommended-WIRE so its comment edit survives). WS-12 T8's file list should be pruned of `session_export.rs` once WS-11 erases it.

3. **`src/extensions/manager.rs`** — three-way: WS-05 (adds native-plugin dispatch arms + a new submodule), WS-11 (T5 deletes dead `install_bundled_channel_from_artifacts`), WS-10 (T7 decomposes the whole 3343L file). All three acknowledge each other. **Owner sequencing: WS-11 T5 deletion first (or WS-05's additive arms), then WS-10 T7 decomposition last** (WS-10 already depends_on WS-05 and notes the WS-11 coordination). No unresolved conflict, but it is the highest-contention file — confirm WS-11 T5 + WS-05 land before WS-10 T7.

4. **`src/api/experiments.rs`** — WS-07 (additive reaper + error-taxonomy point-fixes + WS-13 race annotation comment) **vs** WS-10 (T4 decomposes the 5434L file, gated on WS-07's error mapping) **vs** WS-13 (opens the race issue; WS-07 adds the `// WS-13:` annotation). Cleanly sequenced: WS-07 additive → WS-10 structural (depends_on WS-07). **No conflict.** Note: WS-07 owns the `prepare_campaign_worktree` *annotation*, WS-07 also owns the *fix* per WS-13's issue (WS-13 §Owns says "fix … owned by WS-07"). Consistent.

5. **`src/llm/reasoning.rs` `Reasoning.safety` field** — WS-08 (T7 erases the field, ripples ~17 ctor sites) **vs** WS-10 (T6 decomposes `runtime_manager.rs`, and the `reasoning.rs` decomposition is a WS-10 target). WS-08 T7 explicitly says "coordinate with WS-10; fold into their split if concurrent." **Owner: WS-08** for the semantic field removal; WS-10 absorbs it. Acknowledged on both sides — no unresolved conflict, but it is a real shared-file touch requiring sequencing.

6. **`crates/thinclaw-agent/src/self_repair.rs:325 RepairTask`** — WS-05 (consumes `with_builder`, explicitly does NOT touch `RepairTask`) **vs** WS-11 (T10 is doc-only, hands `RepairTask` fate to WS-05). **No conflict** — both correctly defer the erase/wire decision to WS-05. Owner: **WS-05** (must add an explicit RepairTask decision; today WS-05 only "notes the dependency" — ensure WS-05 actually resolves wire-vs-erase, else `RepairTask` becomes a fifth orphan).

7. **`src/safety/*.rs`** — WS-10 (explicitly disclaims: "Does NOT own `src/safety/*.rs` orphan deletion (WS-11)") **vs** WS-11 (owns + T1 erases the 14 orphans). **No conflict** — clean hand-off, correct owner WS-11.

8. **`extra_public_routes` security layering** — WS-01 (T7 owns it) **vs** WS-03 (lists it out-of-scope: "owned by the gateway/security workstream"). **No conflict** — correctly routed to WS-01.

## NOTES (operator must reconcile)

- **WS-numbering drift in out-of-scope pointers.** WS-02's out-of-scope list misroutes findings to *wrong WS numbers*: it sends sandbox confinement to "WS-03 (sandbox/secrets)" (actually **WS-01**), desktop cloud-sync to "WS-12 (desktop)" (actually **WS-04**; WS-12 is docs), and history/store dedup to "WS-09 (crate migration)" (actually **WS-10**). WS-08 T7 references a nonexistent "**WS-14**" for the `src/safety` cleanup (it is **WS-11**). These are cross-reference label errors, not coverage gaps — the work is covered by the correct WS — but they will mislead executors. **Fix the WS-number labels in WS-02 (3 refs) and WS-08 (1 ref).**

- **`RepairTask` risks becoming an orphan.** WS-11 hands it to WS-05; WS-05 says it "only consumes `with_builder`, does not touch RepairTask" and "notes the dependency." Neither WS actually *resolves* wire-vs-erase. Ensure WS-05 adds a concrete RepairTask decision task, or it falls through both.

- **Two `#[allow(dead_code)]` copies of `conversation_metadata_with_handoff`** (`src/history/store/conversation_queries.rs:45` AND `crates/thinclaw-db/src/postgres_store/conversation_queries.rs:45`) — WS-10 T1 resolves both by deleting the root copy and deciding the crate copy's fate ("wire or erase; check WS that owns handoff"). No WS owns "conversation handoff" — confirm WS-10 erases it absent an owner.

- **`self_message` anti-loop module** — WS-11 T6 recommends ERASE with "low confidence this is vision vs cruft." AUDIT §2 (channels row) calls `self_message` "dead." This is a genuine judgment call the operator should sign off, since it deletes a *claimed* (unenforced) safety guarantee.

- **DECIDE items needing operator sign-off before execution:** WS-04 DP-2 (erase InferenceRouter chat modality), WS-09 DP-3 (`dedup_window` wire-vs-erase), WS-11 DPs 3–6 (`self_message`, `voice_wake`, `tailscale` discovery, `qr_pairing`). These are flagged in their docs but gate real deletions.

- **Cargo.lock contention.** WS-01 owns the wasmtime-wasi bump line; WS-10's consolidations and any dep additions (WS-03 adds `ed25519-dalek` to thinclaw-channels) also touch lockfiles. WS-01 notes "coordinate any other lockfile change." Low risk (different crates' lockfiles for standalone WASM workspaces) but worth a merge-order note.
