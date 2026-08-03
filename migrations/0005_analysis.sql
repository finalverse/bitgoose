-- The Skein: AI analysis of a story.
--
-- Kept in its own table rather than as columns on `stories` for one reason
-- that matters editorially: analysis is *inference*, reporting is not, and the
-- two must never be confusable at the storage layer. A separate table means a
-- query cannot accidentally render a model's opinion as though it were a
-- sourced fact, and dropping every analysis on the site is one statement.

CREATE TABLE IF NOT EXISTS analyses (
    id              UUID PRIMARY KEY,
    story_id        UUID NOT NULL REFERENCES stories(id) ON DELETE CASCADE,

    -- What the event actually means. Grounded in the sources.
    significance    TEXT NOT NULL,
    -- Where it is heading. Explicitly a forecast, never stated as fact.
    direction       TEXT NOT NULL,
    -- Over what period `direction` is claimed. Renders next to it, because an
    -- unbounded prediction cannot be wrong and so is not worth printing.
    horizon         TEXT NOT NULL,
    -- 0..100. The model's own confidence in `direction`, shown to the reader.
    confidence      SMALLINT NOT NULL CHECK (confidence BETWEEN 0 AND 100),
    -- Concrete, checkable signals that would confirm or refute the direction.
    watch           TEXT[] NOT NULL DEFAULT '{}',

    -- Provenance of the inference itself. Which model said this, at what cost,
    -- is part of the story on a site whose pitch is a glass newsroom.
    model           TEXT,
    run_id          UUID REFERENCES agent_runs(id) ON DELETE SET NULL,
    -- How much real source text backed it. The grounding gate records its input
    -- so a thin analysis is auditable after the fact rather than merely absent.
    grounded_chars  INTEGER NOT NULL DEFAULT 0,

    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One live analysis per story. Re-running the Skein replaces it (ON CONFLICT),
-- which keeps the story page from accumulating stale contradictory takes.
CREATE UNIQUE INDEX IF NOT EXISTS analyses_story_uniq ON analyses(story_id);

-- Empty strings would pass NOT NULL and render as a blank section.
ALTER TABLE analyses DROP CONSTRAINT IF EXISTS analyses_nonempty;
ALTER TABLE analyses ADD CONSTRAINT analyses_nonempty
    CHECK (length(trim(significance)) > 0 AND length(trim(direction)) > 0);

-- Track extraction so `bg enrich` can resume and so we never re-fetch a page
-- that already gave us nothing. NULL = never attempted.
ALTER TABLE raw_items ADD COLUMN IF NOT EXISTS extracted_at TIMESTAMPTZ;
-- Which selector won, or 'none'. A publisher changing their layout shows up
-- here as a shift in the distribution rather than as quietly thinner stories.
ALTER TABLE raw_items ADD COLUMN IF NOT EXISTS extract_via TEXT;

CREATE INDEX IF NOT EXISTS raw_items_needs_extract
    ON raw_items (published_at DESC)
    WHERE extracted_at IS NULL;
