-- Every permanent topic is an active reporting assignment in its own edition.
-- Discovery is a cheap RSS search cadence, separate from model briefing.
ALTER TABLE gaggles
    ADD COLUMN IF NOT EXISTS last_searched_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS gaggles_topic_search_due
    ON gaggles (last_searched_at ASC NULLS FIRST, last_hot_at DESC)
    WHERE pinned;
