-- Bound extraction retries.
--
-- A failed fetch deliberately leaves `extracted_at` NULL so a transient network
-- error is tried again. That is right for a blip and wrong for a publisher who
-- refuses us every time: the queue is ordered newest-first, so the same wall of
-- permanently-failing URLs sits at the head of it forever and starves every
-- item behind. Observed live — fourteen consecutive failures, all one host,
-- with sixty more of its items queued behind them.
--
-- Counting attempts lets a blip retry and a refusal give up.

ALTER TABLE raw_items ADD COLUMN IF NOT EXISTS extract_attempts SMALLINT NOT NULL DEFAULT 0;

-- The partial index that drives the queue has to know about the counter, or
-- Postgres scans the whole table once most rows are exhausted.
DROP INDEX IF EXISTS raw_items_needs_extract;
CREATE INDEX IF NOT EXISTS raw_items_needs_extract
    ON raw_items (published_at DESC)
    WHERE extracted_at IS NULL AND extract_attempts < 3;
