-- Video items.
--
-- None of the nine text feeds carry video: a survey of their entries found no
-- video MIME types, no YouTube links and no iframes — every enclosure is an
-- image. Getting video from them would mean crawling article pages, which the
-- published /bot page promises we do not do.
--
-- So video comes from sources that syndicate it directly: YouTube channel
-- feeds, which are public RSS and carry a video id, a title, a description and
-- a thumbnail per entry. We embed through YouTube's own player, which is the
-- sanctioned route and leaves the creator in control — they can disable
-- embedding, and playback, ads and analytics stay theirs.
--
-- `video_id` is the provider's opaque id (an 11-character YouTube id today),
-- not a URL, so the embed host is our choice at render time rather than
-- something a feed can dictate.

ALTER TABLE raw_items ADD COLUMN IF NOT EXISTS video_id TEXT;
ALTER TABLE stories   ADD COLUMN IF NOT EXISTS video_id TEXT;

-- Partial: only a small minority of items are video, and every query that
-- wants them wants exactly those.
CREATE INDEX IF NOT EXISTS raw_items_video_idx ON raw_items (published_at DESC)
    WHERE video_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS stories_video_idx ON stories (published_at DESC)
    WHERE video_id IS NOT NULL;

-- An id, never a URL or markup. Cheap insurance against a feed smuggling
-- something into an iframe src.
-- Dropped first because Postgres has no ADD CONSTRAINT IF NOT EXISTS, and a
-- migration that cannot be re-run is a migration you cannot recover with.
ALTER TABLE raw_items DROP CONSTRAINT IF EXISTS raw_items_video_id_shape;
ALTER TABLE raw_items ADD CONSTRAINT raw_items_video_id_shape
    CHECK (video_id IS NULL OR video_id ~ '^[A-Za-z0-9_-]{6,24}$');
ALTER TABLE stories DROP CONSTRAINT IF EXISTS stories_video_id_shape;
ALTER TABLE stories ADD CONSTRAINT stories_video_id_shape
    CHECK (video_id IS NULL OR video_id ~ '^[A-Za-z0-9_-]{6,24}$');

-- Two new source kinds.
--
-- `video` is the channels above. `finance` is the mainstream financial press —
-- Bloomberg, the FT, CNBC, Yahoo Finance, MarketWatch — whose feeds are mostly
-- equities and rates, so their items are gated on crypto relevance at ingest
-- (see `bg_ingest::relevance`) instead of being taken wholesale.
--
-- The original CHECK was a fixed list, so adding a kind means replacing it.
ALTER TABLE sources DROP CONSTRAINT IF EXISTS sources_kind_check;
ALTER TABLE sources ADD CONSTRAINT sources_kind_check
    CHECK (kind IN ('rss','json_api','filing','onchain','social','wire','video','finance'));
