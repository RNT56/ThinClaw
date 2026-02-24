# S-Data #10: Timestamp Consistency — Comprehensive Plan

> **Status:** Ready for implementation  
> **Risk Level:** Medium — requires coordinated backend + frontend + migration changes  
> **Target:** Normalize all timestamps to **milliseconds (Unix epoch)** everywhere

---

## Current State Analysis

### Backend (Rust) — How timestamps are written

| Table | Column | Rust Code | Unit | File:Line |
|-------|--------|-----------|------|-----------|
| `conversations` | `created_at` | `as_secs() as i64` | **Seconds** | `history.rs:77` |
| `conversations` | `updated_at` | `as_secs() as i64` | **Seconds** | `history.rs:77,259,329` |
| `messages` | `created_at` | `as_secs() as i64` | **Seconds** | `history.rs:259` |
| `documents` | `created_at` | `as_millis() as i64` | **Milliseconds** | `rag.rs:345` |
| `documents` | `updated_at` | `as_millis() as i64` | **Milliseconds** | `rag.rs:345` |
| `projects` | `created_at` | `as_millis() as i64` | **Milliseconds** | `projects.rs:46` |
| `projects` | `updated_at` | `as_millis() as i64` | **Milliseconds** | `projects.rs:46,157` |
| `chat_summaries` | `created_at` | (unused) | Unknown | — |
| `chat_summaries` | `updated_at` | (unused) | Unknown | — |
| `generated_images` | `created_at` | `chrono::Utc::now().to_rfc3339()` | **RFC 3339 string** | `imagine.rs:328` |

### Frontend (TypeScript) — How timestamps are consumed

| Location | Code | Assumption | Correct? |
|----------|------|------------|----------|
| `use-chat.ts:75` | `created_at: Date.now()` (optimistic msg) | milliseconds | ✅ |
| `use-chat.ts:295` | `timeDiff < 300000` (reconciliation) | **assumes ms** | ❌ BUG — backend sends seconds, so this is a 3.5-day window instead of 5 minutes |
| `use-chat.ts:356` | `dbMessages[0].created_at` (pagination cursor) | passed to backend as `before_created_at` | ❌ — optimistic msgs use ms, DB msgs use seconds; cursor could be wrong |
| `use-chat.ts:417` | `created_at: Date.now()` (optimistic user msg) | milliseconds | ✅ |
| `ProjectSettingsDialog.tsx:203` | `new Date(doc.created_at).toLocaleDateString()` | milliseconds | ✅ (documents use ms) |
| `ImagineGallery.tsx:348` | `new Date(image.createdAt).toLocaleDateString()` | ISO string | ✅ (generated_images use RFC 3339) |

### Active Bug: Message Reconciliation Window

```typescript
// use-chat.ts:294-296
if (curr.role === m.role && m.role === 'assistant') {
    const timeDiff = Math.abs((curr.created_at || 0) - (m.created_at || 0));
    return timeDiff < 300000; // Intended: 5 minutes in ms. Actual: 3.5 DAYS in seconds
}
```

When an optimistic message has `created_at: Date.now()` (e.g. `1740422400000`) and a DB message has `created_at` in seconds (e.g. `1740422400`), the diff is `1740422400000 - 1740422400 = 1738682000000`, which is >> 300000. So the comparison actually **fails** (doesn't match), and it falls through to the content-based matching. The bug is latent — it doesn't cause visible issues because content matching catches it, but it means the time-based path is dead code.

---

## Recommended Approach: Normalize to Milliseconds

**Why milliseconds?**
1. JavaScript `Date.now()` returns milliseconds — no conversion needed on the frontend
2. `documents` and `projects` already use milliseconds — fewer changes
3. `specta::Type` exports `created_at: number` (f64) — milliseconds fit naturally
4. Industry standard for JSON APIs (Unix epoch ms)

---

## Implementation Steps

### Step 1: Data Migration (SQL)

Create migration `20260225000000_normalize_timestamps.sql`:

```sql
-- Step 1: Convert conversations timestamps from seconds to milliseconds
-- Only convert rows where created_at looks like seconds (< 10_000_000_000)
-- Values > 10B are already in milliseconds (year ~2286 in seconds, or ~2001 in ms)
UPDATE conversations
SET created_at = created_at * 1000,
    updated_at = updated_at * 1000
WHERE created_at < 10000000000;

-- Step 2: Convert messages timestamps from seconds to milliseconds
UPDATE messages
SET created_at = created_at * 1000
WHERE created_at < 10000000000;

-- Step 3: Convert chat_summaries timestamps from seconds to milliseconds
UPDATE chat_summaries
SET created_at = created_at * 1000,
    updated_at = updated_at * 1000
WHERE created_at < 10000000000;

-- Step 4: generated_images uses DATETIME default — leave as-is
-- (RFC 3339 strings work with JavaScript's new Date() natively)
```

**Safety:** The `WHERE created_at < 10000000000` guard ensures:
- A seconds timestamp (e.g. `1740422400`) → gets multiplied → `1740422400000` ✅
- A ms timestamp (e.g. `1740422400000`) → skipped (> 10B) ✅
- The threshold 10B corresponds to ~2286 in seconds or ~1970+115d in ms — no ambiguity

### Step 2: Backend Changes (Rust)

#### `history.rs` — 3 locations

```diff
- .as_secs() as i64;   // line 77
+ .as_millis() as i64;

- .as_secs() as i64;   // line 259
+ .as_millis() as i64;

- .as_secs() as i64;   // line 329
+ .as_millis() as i64;
```

#### `get_messages` — pagination threshold

The `before_created_at` parameter is received as `Option<f64>` from the frontend. Currently compared against seconds. After migration, it will compare against milliseconds. Since the frontend paginates using the `created_at` of the first loaded message (which will now be in ms), this is self-consistent.

**No change needed** — the comparison is relative (`created_at < ?`), so both sides will be in ms after migration.

### Step 3: Frontend Changes (TypeScript)

#### `use-chat.ts:295` — Fix reconciliation window

```diff
- const timeDiff = Math.abs((curr.created_at || 0) - (m.created_at || 0));
- return timeDiff < 300000;
+ const timeDiff = Math.abs((curr.created_at || 0) - (m.created_at || 0));
+ return timeDiff < 300000; // 5 minutes in milliseconds — now correct with ms timestamps
```

No code change needed! The constant `300000` (5 minutes in ms) was always the intent. Once the backend sends milliseconds, this works correctly.

#### `ProjectSettingsDialog.tsx:203` — Already correct

`new Date(doc.created_at)` already receives milliseconds from documents. No change.

#### Frontend-generated `created_at` values — Already correct

`Date.now()` returns milliseconds. After migration, backend also returns milliseconds. They're now consistent.

### Step 4: No changes needed for `generated_images`

`generated_images.created_at` is a `DATETIME` column with `DEFAULT CURRENT_TIMESTAMP`, stored as RFC 3339 string. The frontend uses `new Date(image.createdAt)` which handles ISO strings natively. This is a different type (`String` not `i64`) and doesn't need normalization — it's internally consistent.

---

## Verification Checklist

1. **Migration safety:**
   - [ ] Run migration on a copy of the production DB
   - [ ] Verify `conversations.created_at` values are now 13-digit (ms)
   - [ ] Verify `messages.created_at` values are now 13-digit (ms)
   - [ ] Verify no double-multiplication (values > 10^16 would indicate double-multiply)

2. **Backend:**
   - [ ] `cargo check --features llamacpp` passes
   - [ ] New conversation `created_at` is in milliseconds
   - [ ] New message `created_at` is in milliseconds

3. **Frontend:**
   - [ ] Chat messages load without duplicates or blank bubbles
   - [ ] Pagination ("load more messages") works correctly
   - [ ] Message editing (which deletes subsequent messages by `created_at >`) works
   - [ ] Project document dates display correctly

4. **Edge case:**
   - [ ] App starts fresh (no existing DB) — everything works
   - [ ] App starts with old DB — migration converts correctly
   - [ ] App starts with partially-migrated DB — `WHERE < 10B` guard prevents double-multiply

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Double-multiplication on re-run | Very Low | High | `WHERE < 10B` guard + sqlx migration tracking |
| Pagination breaks for existing chats | Low | Medium | Relative comparison (`<`) is unit-agnostic |
| Message edit deletes wrong messages | Low | Medium | `created_at >` comparison — both sides will be ms after migration |
| Frontend shows wrong dates | Very Low | Low | Only `documents` and `generated_images` display dates; both already correct |

---

## Files to Modify

| File | Change | Lines |
|------|--------|-------|
| `backend/migrations/20260225000000_normalize_timestamps.sql` | **NEW** — Data migration | — |
| `backend/src/history.rs` | `as_secs()` → `as_millis()` | 77, 259, 329 |
| No frontend changes required | — | — |
