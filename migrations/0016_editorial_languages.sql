-- Independent English, Chinese, and French editorial streams.
ALTER TABLE stories
    ADD COLUMN IF NOT EXISTS editorial_language TEXT NOT NULL DEFAULT 'en';

ALTER TABLE stories DROP CONSTRAINT IF EXISTS stories_editorial_language_check;
ALTER TABLE stories ADD CONSTRAINT stories_editorial_language_check
    CHECK (editorial_language IN ('en', 'zh', 'fr'));

UPDATE stories s
   SET editorial_language = CASE
       WHEN lower(coalesce((
           SELECT r.lang
             FROM story_items si
             JOIN raw_items r ON r.id = si.raw_item_id
            WHERE si.story_id = s.id
            ORDER BY (si.role = 'seed') DESC, r.published_at ASC
            LIMIT 1
       ), 'en')) LIKE 'zh%' THEN 'zh'
       WHEN lower(coalesce((
           SELECT r.lang
             FROM story_items si
             JOIN raw_items r ON r.id = si.raw_item_id
            WHERE si.story_id = s.id
            ORDER BY (si.role = 'seed') DESC, r.published_at ASC
            LIMIT 1
       ), 'en')) LIKE 'fr%' THEN 'fr'
       ELSE 'en'
   END;

CREATE INDEX IF NOT EXISTS stories_language_front_idx
    ON stories (editorial_language, status, published_at DESC);

ALTER TABLE gaggles
    ADD COLUMN IF NOT EXISTS editorial_language TEXT NOT NULL DEFAULT 'en'
        CHECK (editorial_language IN ('en', 'zh', 'fr')),
    ADD COLUMN IF NOT EXISTS pinned BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS analysis_md TEXT NOT NULL DEFAULT '',
    ADD COLUMN IF NOT EXISTS watchpoints TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS anchor_terms TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS keywords TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS primary_source_names TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS primary_source_urls TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN IF NOT EXISTS last_briefed_at TIMESTAMPTZ;

ALTER TABLE gaggles DROP CONSTRAINT IF EXISTS gaggles_topic_key;
ALTER TABLE gaggles DROP CONSTRAINT IF EXISTS gaggles_slug_key;
CREATE UNIQUE INDEX IF NOT EXISTS gaggles_topic_language_unique
    ON gaggles (topic, editorial_language);
CREATE UNIQUE INDEX IF NOT EXISTS gaggles_slug_language_unique
    ON gaggles (slug, editorial_language);
CREATE INDEX IF NOT EXISTS gaggles_language_hot
    ON gaggles (editorial_language, last_hot_at DESC);
