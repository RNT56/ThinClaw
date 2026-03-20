---
description: How to audit and update project documentation against the codebase
---

# Documentation Audit Workflow

## When to Run
- After any major architectural change (e.g., agent migration, backend refactor)
- Before releases or milestones
- When `all-docs.md` feels stale

## Steps

### 1. Verify `all-docs.md` completeness
// turbo
```bash
find . -name "*.md" -not -path "*/node_modules/*" -not -path "*/target/*" -not -path "*/.gemini/*" -not -path "*/binaries/*" | sort
```
Compare output against `all-docs.md`. Add any missing files with appropriate status annotations.

### 2. Search for stale architecture patterns
Run these searches against `documentation/latest/` to find outdated references:
// turbo
```bash
grep -rn --include="*.md" -E "ipc\.rs|auth-profiles\.json|Node\.js.*sidecar|sidecar.*Node" documentation/latest/
```
Any hits in **Current** docs (not Historical/Archived) need fixing.

### 3. Validate "Current" doc claims
For each file marked **Current** in `all-docs.md`:
- Spot-check 3-5 specific code paths, file names, or struct references against the actual codebase
- Verify directory layouts match reality
- Check that any line number references are still approximately correct

### 4. Update status annotations
Use the status key from `all-docs.md`:
- **Current** = reflects live codebase
- ✅ Completed = finished plan/roadmap (kept as reference)
- 📦 Archived = pre-IronClaw historical
- ⚠️ Historical = superseded

### 5. Update timestamp
After all changes, update the "Last updated" line at the top of `all-docs.md` with the current date and "(validated against codebase)".

### 6. Stale reference patterns to check
These patterns often indicate outdated docs:
- `ipc.rs` → should be `ironclaw_channel.rs`
- `auth-profiles.json` → removed during IronClaw migration
- `Node.js` in context of the agent/sidecar → IronClaw is in-process Rust
- `WebSocket RPC` for agent communication → direct Rust function calls
- `OpenClaw engine process` → IronClaw agent engine (in-process)
- `sidecar` when referring to the agent → IronClaw is a library, not a sidecar

## Notes
- Don't modify **📦 Archived** or **⚠️ Historical** docs — they have reference value
- Do add IronClaw migration notes to historical docs if they're commonly accessed
- The `ironclaw/rewrite-docs/` are IronClaw's own internal docs — skip unless specifically asked
