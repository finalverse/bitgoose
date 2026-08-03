-- Beats: BitGoose becomes a frontier-technology newsroom.
--
-- It launched as a crypto property. The claim graph — raw items clustering into
-- events, events decomposing into claims, claims carrying provenance — was
-- never crypto-specific, so widening the subject matter is an extension rather
-- than a rewrite. What it did lack was a way to say *which desk* a story
-- belongs to.
--
-- Beat is deliberately separate from `category`. They are orthogonal: "policy"
-- means the EU AI Act on one desk and a stablecoin bill on the other, "security"
-- means model jailbreaks or bridge exploits. Folding them into one flat list
-- would force a choice between losing the section or duplicating every one of
-- them per desk.
--
-- Existing rows are crypto by definition — everything published before this
-- migration was from the crypto roster.

ALTER TABLE stories   ADD COLUMN IF NOT EXISTS beat TEXT NOT NULL DEFAULT 'crypto';
ALTER TABLE raw_items ADD COLUMN IF NOT EXISTS beat TEXT;

-- A source can pin the beat of everything it publishes: arXiv cs.AI is never
-- crypto news. Left NULL for general-interest sources, whose items are routed
-- per item by `bg_ingest::relevance::classify`.
ALTER TABLE sources   ADD COLUMN IF NOT EXISTS beat TEXT;

ALTER TABLE stories DROP CONSTRAINT IF EXISTS stories_beat_check;
ALTER TABLE stories ADD CONSTRAINT stories_beat_check
    CHECK (beat IN ('ai','crypto'));
ALTER TABLE raw_items DROP CONSTRAINT IF EXISTS raw_items_beat_check;
ALTER TABLE raw_items ADD CONSTRAINT raw_items_beat_check
    CHECK (beat IS NULL OR beat IN ('ai','crypto'));
ALTER TABLE sources DROP CONSTRAINT IF EXISTS sources_beat_check;
ALTER TABLE sources ADD CONSTRAINT sources_beat_check
    CHECK (beat IS NULL OR beat IN ('ai','crypto'));

-- The front page and every desk page filter on this, so it goes in the index
-- alongside the ordering column rather than being a separate lookup.
CREATE INDEX IF NOT EXISTS stories_beat_published_idx
    ON stories (beat, published_at DESC) WHERE status = 'published';

-- Categories for the AI desk. The original list was written for a crypto site;
-- these four are the sections that beat actually has, and without them every AI
-- story would land in the catch-all "tech".
ALTER TABLE stories DROP CONSTRAINT IF EXISTS stories_category_check;
ALTER TABLE stories ADD CONSTRAINT stories_category_check
    CHECK (category IN ('markets','policy','tech','defi','business','security',
                        'ai','nft','gaming','culture',
                        'research','models','compute','safety'));

-- Two more source kinds.
--
-- `research` is preprint servers — arXiv and the like. A paper is not a news
-- item: it has authors, an abstract and no editor, and treating it as one would
-- put "we propose a novel attention variant" on the front page as if it were
-- reporting.
--
-- `forum` is Hacker News and Reddit. These are discussion, not reporting: the
-- signal is that practitioners are arguing about something, and the item is a
-- pointer to that argument. Their trust scores are low for exactly that reason
-- and they must never count as corroboration for a claim.
ALTER TABLE sources DROP CONSTRAINT IF EXISTS sources_kind_check;
ALTER TABLE sources ADD CONSTRAINT sources_kind_check
    CHECK (kind IN ('rss','json_api','filing','onchain','social','wire',
                    'video','finance','research','forum'));
