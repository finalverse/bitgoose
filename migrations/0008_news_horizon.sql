-- Age out items that stopped being news before we got to them.
--
-- The newsroom takes in roughly three times what a free inference tier can
-- triage, so a backlog builds. Measured at 3,764 items: 137 from the last day,
-- 1,972 one to three days old, 1,655 three to seven. Ninety-six per cent of the
-- queue was stale.
--
-- That is not a queue to work through. Spending a scarce token budget on a
-- five-day-old item displaces today's stories, and publishing it would put
-- last week's news on a news front page. Better to let it lapse and say so.
--
-- Marked rather than deleted: the item stays readable, the URL stays
-- de-duplicated so a re-post is still recognised, and the decision is
-- reversible if the horizon turns out to be wrong.

ALTER TABLE raw_items ADD COLUMN IF NOT EXISTS aged_out_at TIMESTAMPTZ;

-- The triage queue's index has to know about it, or Postgres scans every
-- lapsed row on the way to the current ones.
DROP INDEX IF EXISTS raw_items_untriaged;
CREATE INDEX IF NOT EXISTS raw_items_untriaged
    ON raw_items (published_at DESC)
    WHERE NOT triaged AND aged_out_at IS NULL;
