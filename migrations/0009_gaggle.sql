-- Gaggles: a special topic, opened when coverage converges hard on one subject.
--
-- A skein is geese in flight and a gaggle is geese on the ground — which is
-- exactly the relation between the two features. The Skein reads where one
-- story is going; a Gaggle is what happens when many stories turn out to be
-- about the same thing.
--
-- Opened from a signal that costs nothing: the number of *independent outlets*
-- covering a subject. Measured on 1,235 live headlines, the Clarity Act had
-- seven sources across fifteen stories while the loudest single-outlet topic
-- had one source across thirty-two. Convergence is the signal; volume is not.

CREATE TABLE IF NOT EXISTS gaggles (
    id          UUID PRIMARY KEY,
    -- The detected term, lowercased. Stable across re-detection, so a gaggle
    -- that stays hot is updated rather than duplicated.
    topic       TEXT NOT NULL UNIQUE,
    slug        TEXT NOT NULL UNIQUE,
    -- Written by the Gander when the gaggle opens.
    title       TEXT NOT NULL,
    standfirst  TEXT NOT NULL,

    -- The evidence, kept so the page can show why this is a topic at all.
    source_count  INTEGER NOT NULL DEFAULT 0,
    story_count   INTEGER NOT NULL DEFAULT 0,

    model       TEXT,
    run_id      UUID REFERENCES agent_runs(id) ON DELETE SET NULL,
    opened_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Refreshed every time the topic is still hot; a gaggle that stops being
    -- covered goes quiet on its own rather than needing to be closed by hand.
    last_hot_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS gaggles_hot ON gaggles (last_hot_at DESC);

-- Membership. Rebuilt on each refresh rather than appended, so a story that
-- stops matching drops out.
CREATE TABLE IF NOT EXISTS gaggle_stories (
    gaggle_id UUID NOT NULL REFERENCES gaggles(id) ON DELETE CASCADE,
    story_id  UUID NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    PRIMARY KEY (gaggle_id, story_id)
);

CREATE INDEX IF NOT EXISTS gaggle_stories_story ON gaggle_stories (story_id);

ALTER TABLE gaggles DROP CONSTRAINT IF EXISTS gaggles_nonempty;
ALTER TABLE gaggles ADD CONSTRAINT gaggles_nonempty
    CHECK (length(trim(title)) > 0 AND length(trim(standfirst)) > 0);
