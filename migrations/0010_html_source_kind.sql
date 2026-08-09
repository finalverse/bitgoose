-- Admit crawled sources.
--
-- `sources.kind` carries a whitelist, so a new SourceKind variant compiles
-- fine and then fails at insert time. This is the second such list to need
-- widening after an enum grew (see 0006 for the agent roster); both are the
-- same trap, which is that an exhaustive match in Rust says nothing about a
-- CHECK constraint in Postgres.
ALTER TABLE sources DROP CONSTRAINT IF EXISTS sources_kind_check;
ALTER TABLE sources ADD CONSTRAINT sources_kind_check CHECK (kind IN (
    'rss', 'json_api', 'filing', 'onchain', 'social', 'wire', 'video',
    'finance', 'research', 'forum', 'html'
));
