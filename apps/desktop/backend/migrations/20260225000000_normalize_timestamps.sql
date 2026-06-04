-- Normalize all integer timestamps to milliseconds (Unix epoch).
--
-- History:
--   conversations & messages used as_secs() (seconds)
--   documents & projects used as_millis() (milliseconds)
--
-- After this migration, ALL integer timestamp columns use milliseconds.
--
-- Safety: The WHERE guard (< 10_000_000_000) ensures:
--   - Seconds values (e.g. 1740422400)    → multiplied → 1740422400000 ✅
--   - Millisecond values (e.g. 1740422400000) → skipped (already correct) ✅
--   - Threshold 10B = year ~2286 in seconds = harmless boundary

-- conversations: created_at and updated_at were stored in seconds
UPDATE conversations
SET created_at = created_at * 1000,
    updated_at = updated_at * 1000
WHERE created_at < 10000000000;

-- messages: created_at was stored in seconds
UPDATE messages
SET created_at = created_at * 1000
WHERE created_at < 10000000000;

-- chat_summaries: created_at and updated_at were stored in seconds
UPDATE chat_summaries
SET created_at = created_at * 1000,
    updated_at = updated_at * 1000
WHERE created_at < 10000000000;

-- documents and projects already use milliseconds — no change needed.
-- generated_images uses DATETIME/RFC3339 strings — no change needed.
