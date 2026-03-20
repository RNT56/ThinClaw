# ⛔ ARCHIVED — Do Not Use as Current Reference

> **Status:** ARCHIVED (since 2026-02-28)  
> **Purpose:** Historical Node.js → Rust migration comparison guides  
> **Replaced by:** [`../CLAUDE.md`](../CLAUDE.md), [`../Agent_flow.md`](../Agent_flow.md), [`../FEATURE_PARITY.md`](../FEATURE_PARITY.md)

---

## What This Directory Contains

These 34 documents were written during the OpenClaw → IronClaw rewrite (Jan–Feb 2026)
to guide the migration from the Node.js/TypeScript codebase to Rust. Each `*_RS.md`
file describes how a specific OpenClaw module should be implemented in Rust.

**The rewrite is complete.** These files are kept for historical context only.

## Why You Should NOT Reference These Files

1. **Outdated architecture** — The Rust codebase has evolved significantly since these
   were written. Module names, file paths, and API surfaces have changed.
2. **Missing features** — Many features added after the rewrite (Bedrock, Gemini,
   llama.cpp, native channels, media pipeline, etc.) are not covered here.
3. **Stale `src-tauri/` paths** — These docs reference the pre-rename directory
   structure (`src-tauri/`) which is now `backend/` in Scrappy.

## Where to Find Current Documentation

| Need | Go to |
|------|-------|
| IronClaw development guide | [`../CLAUDE.md`](../CLAUDE.md) |
| Agent boot/runtime flow | [`../Agent_flow.md`](../Agent_flow.md) |
| Feature parity matrix | [`../FEATURE_PARITY.md`](../FEATURE_PARITY.md) |
| Module specs | `../src/tools/README.md`, `../src/workspace/README.md`, `../src/setup/README.md` |
| Scrappy integration | `../../documentation/latest/` |
