-- Room for the World, Science and Culture desks and their categories.
--
-- The third time a CHECK whitelist has silently blocked a new enum variant:
-- `agents_role_check` did it when the Skein was added, `sources_kind_check`
-- when crawling arrived, and now `beat` and `category` on three tables at once.
-- An exhaustive match in Rust says nothing about a constraint in Postgres, and
-- the failure surfaces at insert time on a machine that is already running.
--
-- Rewritten as a single set per column so the next variant is one edit, and so
-- the four lists cannot drift apart — `sources`, `raw_items` and `stories` all
-- constrain `beat` and had to agree even before this.
ALTER TABLE sources   DROP CONSTRAINT IF EXISTS sources_beat_check;
ALTER TABLE raw_items DROP CONSTRAINT IF EXISTS raw_items_beat_check;
ALTER TABLE stories   DROP CONSTRAINT IF EXISTS stories_beat_check;
ALTER TABLE stories   DROP CONSTRAINT IF EXISTS stories_category_check;

ALTER TABLE sources ADD CONSTRAINT sources_beat_check
  CHECK (beat IS NULL OR beat = ANY (ARRAY[
    'ai','crypto','markets','tech','world','science','culture']));

ALTER TABLE raw_items ADD CONSTRAINT raw_items_beat_check
  CHECK (beat IS NULL OR beat = ANY (ARRAY[
    'ai','crypto','markets','tech','world','science','culture']));

ALTER TABLE stories ADD CONSTRAINT stories_beat_check
  CHECK (beat = ANY (ARRAY[
    'ai','crypto','markets','tech','world','science','culture']));

ALTER TABLE stories ADD CONSTRAINT stories_category_check
  CHECK (category = ANY (ARRAY[
    'markets','policy','tech','defi','business','security','ai','nft','gaming',
    'culture','research','models','compute','safety',
    'world','politics','health','climate','space','science','sports',
    'entertainment','energy']));
