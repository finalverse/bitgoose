-- ===========================================================================
-- BitGoose initial schema
--
-- Shape of the graph:
--
--   sources ─▶ raw_items ─▶ stories ─▶ claims ─▶ articles
--                              │         │          │
--                          story_items  claim_sources  article_citations
--
-- The invariant the whole product rests on: a published sentence can always be
-- walked back to the source items that justify it. Foreign keys and CHECK
-- constraints below make the alternative unrepresentable rather than merely
-- discouraged.
-- ===========================================================================

CREATE EXTENSION IF NOT EXISTS vector;
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ---------------------------------------------------------------------------
-- sources
-- ---------------------------------------------------------------------------
CREATE TABLE sources (
    id               UUID PRIMARY KEY,
    slug             TEXT NOT NULL UNIQUE,
    name             TEXT NOT NULL,
    kind             TEXT NOT NULL CHECK (kind IN ('rss','json_api','filing','onchain','social','wire')),
    url              TEXT NOT NULL,
    homepage         TEXT NOT NULL DEFAULT '',
    -- Weights corroboration. Three low-trust aggregators reprinting one another
    -- must not outweigh a single tier-1 outlet with a named reporter.
    trust            SMALLINT NOT NULL DEFAULT 50 CHECK (trust BETWEEN 0 AND 100),
    robots_ok        BOOLEAN NOT NULL DEFAULT TRUE,
    poll_interval_s  INTEGER NOT NULL DEFAULT 300 CHECK (poll_interval_s >= 30),
    etag             TEXT,
    last_modified    TEXT,
    last_polled_at   TIMESTAMPTZ,
    last_error       TEXT,
    enabled          BOOLEAN NOT NULL DEFAULT TRUE,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sources_due_idx ON sources (enabled, last_polled_at) WHERE enabled;

-- ---------------------------------------------------------------------------
-- raw_items — one report from one outlet, before any editorial judgement
-- ---------------------------------------------------------------------------
CREATE TABLE raw_items (
    id             UUID PRIMARY KEY,
    source_id      UUID NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    external_id    TEXT,
    canonical_url  TEXT NOT NULL,
    -- SHA-256 of canonical_url. The dedupe key: the same story syndicated to
    -- two feed URLs canonicalizes to one hash and is stored once.
    url_hash       TEXT NOT NULL UNIQUE,
    title          TEXT NOT NULL,
    dek            TEXT,
    authors        TEXT[] NOT NULL DEFAULT '{}',
    published_at   TIMESTAMPTZ NOT NULL,
    fetched_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    summary_raw    TEXT,
    -- PRIVATE WORKING COPY. Used for claim extraction and the verbatim-overlap
    -- check only. Never selected into an API response or a rendered page; the
    -- repository layer has no method that returns it to a public projection.
    body_raw       TEXT,
    body_hash      TEXT,
    -- 64-bit SimHash over normalized title+lede, stored signed because Postgres
    -- has no unsigned 64-bit integer. Cast back to u64 on read.
    simhash        BIGINT NOT NULL DEFAULT 0,
    lang           TEXT NOT NULL DEFAULT 'en',
    image_url      TEXT,
    story_id       UUID,
    triaged        BOOLEAN NOT NULL DEFAULT FALSE,
    category       TEXT,
    assets         TEXT[] NOT NULL DEFAULT '{}',
    -- Gosling's 0-100 first read, before clustering.
    triage_score   SMALLINT,
    embedding      vector(1536)
);
CREATE INDEX raw_items_untriaged_idx ON raw_items (triaged, published_at DESC) WHERE NOT triaged;
CREATE INDEX raw_items_story_idx     ON raw_items (story_id);
CREATE INDEX raw_items_published_idx ON raw_items (published_at DESC);
CREATE INDEX raw_items_simhash_idx   ON raw_items (simhash);
CREATE INDEX raw_items_title_trgm    ON raw_items USING gin (title gin_trgm_ops);

-- ---------------------------------------------------------------------------
-- stories — the EVENT, distinct from any single report of it
-- ---------------------------------------------------------------------------
CREATE TABLE stories (
    id              UUID PRIMARY KEY,
    slug            TEXT NOT NULL UNIQUE,
    kind            TEXT NOT NULL CHECK (kind IN ('wire','desk','golden_egg')),
    status          TEXT NOT NULL CHECK (status IN ('triage','clustering','drafting','review','published','held','killed')),
    title           TEXT NOT NULL,
    -- 2-3 sentences in our own words. What the Wire renders.
    summary         TEXT,
    category        TEXT NOT NULL,
    newsworthiness  SMALLINT NOT NULL DEFAULT 0 CHECK (newsworthiness BETWEEN 0 AND 100),
    -- Independent-source arrival rate. Four outlets in ten minutes is a
    -- different signal from four over two days.
    velocity        REAL NOT NULL DEFAULT 0,
    source_count    INTEGER NOT NULL DEFAULT 0,
    primary_asset   TEXT,
    assets          TEXT[] NOT NULL DEFAULT '{}',
    image_url       TEXT,
    first_seen_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    published_at    TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Gander's reasoning when it holds or kills, so the call is auditable.
    editor_note     TEXT,
    embedding       vector(1536),
    -- A published story must have a timestamp; nothing else may have one.
    CONSTRAINT stories_published_has_ts
        CHECK ((status = 'published') = (published_at IS NOT NULL))
);
CREATE INDEX stories_front_idx     ON stories (status, published_at DESC NULLS LAST);
CREATE INDEX stories_rank_idx      ON stories (status, newsworthiness DESC, published_at DESC);
CREATE INDEX stories_category_idx  ON stories (category, published_at DESC);
CREATE INDEX stories_kind_idx      ON stories (kind, status, published_at DESC);
CREATE INDEX stories_open_idx      ON stories (status, updated_at DESC)
    WHERE status IN ('triage','clustering','drafting','review');

ALTER TABLE raw_items
    ADD CONSTRAINT raw_items_story_fk
    FOREIGN KEY (story_id) REFERENCES stories(id) ON DELETE SET NULL;

-- ---------------------------------------------------------------------------
-- story_items — which reports back which event, and how
-- ---------------------------------------------------------------------------
CREATE TABLE story_items (
    story_id     UUID NOT NULL REFERENCES stories(id)   ON DELETE CASCADE,
    raw_item_id  UUID NOT NULL REFERENCES raw_items(id) ON DELETE CASCADE,
    -- 'contradicting' is kept deliberately. Disagreement between outlets is
    -- signal; discarding it is how aggregators quietly mislead.
    role         TEXT NOT NULL CHECK (role IN ('seed','corroborating','contradicting','context')),
    added_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (story_id, raw_item_id)
);
CREATE INDEX story_items_item_idx ON story_items (raw_item_id);

-- ---------------------------------------------------------------------------
-- claims — the unit of truth
-- ---------------------------------------------------------------------------
CREATE TABLE claims (
    id             UUID PRIMARY KEY,
    story_id       UUID NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    -- One self-contained sentence. No pronouns pointing outside itself, so a
    -- claim stays meaningful when surfaced alone via the API or MCP.
    text           TEXT NOT NULL CHECK (length(btrim(text)) > 0),
    kind           TEXT NOT NULL CHECK (kind IN ('fact','figure','quote','forecast')),
    confidence     REAL NOT NULL DEFAULT 0 CHECK (confidence BETWEEN 0 AND 1),
    verification   TEXT NOT NULL CHECK (verification IN
                       ('unverified','single_source','corroborated','disputed','refuted','primary_verified')),
    numeric_value  NUMERIC,
    unit           TEXT,
    -- Crypto figures go stale in hours. A claim without an as-of is undated,
    -- not timeless.
    as_of          TIMESTAMPTZ,
    created_by_run UUID,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX claims_story_idx ON claims (story_id);
CREATE INDEX claims_verif_idx ON claims (verification);

-- ---------------------------------------------------------------------------
-- claim_sources — the provenance edge
-- ---------------------------------------------------------------------------
CREATE TABLE claim_sources (
    claim_id     UUID NOT NULL REFERENCES claims(id)    ON DELETE CASCADE,
    raw_item_id  UUID NOT NULL REFERENCES raw_items(id) ON DELETE CASCADE,
    stance       TEXT NOT NULL CHECK (stance IN ('supports','contradicts','mentions')),
    excerpt      TEXT,
    PRIMARY KEY (claim_id, raw_item_id),
    -- The 25-word quotation cap, enforced in the database as well as in
    -- bg-core::policy. Belt and braces: a bug in the policy path, or any future
    -- code that writes here directly, still cannot store an over-long quote.
    CONSTRAINT claim_sources_excerpt_word_cap CHECK (
        excerpt IS NULL
        OR array_length(regexp_split_to_array(btrim(excerpt), '\s+'), 1) <= 25
    )
);
CREATE INDEX claim_sources_item_idx ON claim_sources (raw_item_id);

-- ---------------------------------------------------------------------------
-- articles — a RENDERING of a claim set, versioned
-- ---------------------------------------------------------------------------
CREATE TABLE articles (
    id              UUID PRIMARY KEY,
    story_id        UUID NOT NULL REFERENCES stories(id) ON DELETE CASCADE,
    version         INTEGER NOT NULL CHECK (version >= 1),
    headline        TEXT NOT NULL,
    dek             TEXT NOT NULL DEFAULT '',
    slug            TEXT NOT NULL,
    body_md         TEXT NOT NULL,
    seo_title       TEXT NOT NULL DEFAULT '',
    seo_desc        TEXT NOT NULL DEFAULT '',
    reading_time_s  INTEGER NOT NULL DEFAULT 60,
    status          TEXT NOT NULL,
    published_at    TIMESTAMPTZ,
    -- SHA-256 of body_md. Makes any post-hoc edit detectable by a reader who
    -- kept the old hash.
    content_hash    TEXT NOT NULL,
    editor_run_id   UUID,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (story_id, version)
);
CREATE INDEX articles_story_idx ON articles (story_id, version DESC);
CREATE INDEX articles_slug_idx  ON articles (slug);

CREATE TABLE article_citations (
    article_id  UUID NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
    marker      TEXT NOT NULL,
    claim_id    UUID NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
    PRIMARY KEY (article_id, marker)
);

-- ---------------------------------------------------------------------------
-- corrections — append-only. We never silently edit a published page.
-- ---------------------------------------------------------------------------
CREATE TABLE corrections (
    id            UUID PRIMARY KEY,
    article_id    UUID NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
    from_version  INTEGER NOT NULL,
    to_version    INTEGER NOT NULL,
    reason        TEXT NOT NULL,
    diff_md       TEXT NOT NULL DEFAULT '',
    issued_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    agent_id      UUID,
    CONSTRAINT corrections_move_forward CHECK (to_version > from_version)
);
CREATE INDEX corrections_article_idx ON corrections (article_id, issued_at DESC);

-- ---------------------------------------------------------------------------
-- entities — the knowledge graph behind topic and asset hubs
-- ---------------------------------------------------------------------------
CREATE TABLE entities (
    id          UUID PRIMARY KEY,
    kind        TEXT NOT NULL CHECK (kind IN
                    ('person','company','protocol','token','chain','regulator','fund','exchange')),
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL UNIQUE,
    ticker      TEXT,
    aliases     TEXT[] NOT NULL DEFAULT '{}',
    summary     TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX entities_ticker_idx ON entities (ticker) WHERE ticker IS NOT NULL;

CREATE TABLE entity_mentions (
    entity_id  UUID NOT NULL REFERENCES entities(id) ON DELETE CASCADE,
    story_id   UUID NOT NULL REFERENCES stories(id)  ON DELETE CASCADE,
    -- 0-1: how central the entity is to the story, so hub pages can lead with
    -- coverage that is actually about the entity rather than passing mentions.
    salience   REAL NOT NULL DEFAULT 0.5 CHECK (salience BETWEEN 0 AND 1),
    PRIMARY KEY (entity_id, story_id)
);
CREATE INDEX entity_mentions_story_idx ON entity_mentions (story_id);

-- ---------------------------------------------------------------------------
-- the Flock
-- ---------------------------------------------------------------------------
CREATE TABLE agents (
    id             UUID PRIMARY KEY,
    slug           TEXT NOT NULL UNIQUE,
    name           TEXT NOT NULL,
    role           TEXT NOT NULL UNIQUE CHECK (role IN
                       ('scout','gosling','curator','scribe','sentinel','quant','copydesk','gander','herald','ombuds')),
    tier           TEXT NOT NULL CHECK (tier IN ('none','fast','mid','top')),
    system_prompt  TEXT NOT NULL DEFAULT '',
    temperature    REAL NOT NULL DEFAULT 0.2,
    enabled        BOOLEAN NOT NULL DEFAULT TRUE
);

-- One row per agent invocation, LLM-backed or not. This table is public: it
-- powers /flock, where BitGoose publishes its own error rate and running cost.
-- An AI newsroom asking for trust has to open its books.
CREATE TABLE agent_runs (
    id                 UUID PRIMARY KEY,
    agent_id           UUID NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    role               TEXT NOT NULL,
    story_id           UUID REFERENCES stories(id) ON DELETE SET NULL,
    stage              TEXT NOT NULL,
    status             TEXT NOT NULL CHECK (status IN ('running','ok','failed','skipped','budgeted')),
    provider           TEXT NOT NULL DEFAULT '',
    model              TEXT NOT NULL DEFAULT '',
    prompt_tokens      INTEGER NOT NULL DEFAULT 0,
    completion_tokens  INTEGER NOT NULL DEFAULT 0,
    cost_usd           NUMERIC(12,6) NOT NULL DEFAULT 0,
    latency_ms         INTEGER NOT NULL DEFAULT 0,
    input_hash         TEXT,
    output_hash        TEXT,
    note               TEXT,
    error              TEXT,
    started_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at        TIMESTAMPTZ
);
CREATE INDEX agent_runs_recent_idx ON agent_runs (started_at DESC);
CREATE INDEX agent_runs_role_idx   ON agent_runs (role, started_at DESC);
CREATE INDEX agent_runs_story_idx  ON agent_runs (story_id, started_at);

-- ---------------------------------------------------------------------------
-- market data
-- ---------------------------------------------------------------------------
CREATE TABLE assets (
    id            UUID PRIMARY KEY,
    symbol        TEXT NOT NULL UNIQUE,
    name          TEXT NOT NULL,
    coingecko_id  TEXT UNIQUE,
    rank          INTEGER
);

CREATE TABLE price_ticks (
    symbol          TEXT NOT NULL,
    ts              TIMESTAMPTZ NOT NULL,
    price_usd       NUMERIC(24,8) NOT NULL,
    change_24h_pct  DOUBLE PRECISION,
    volume_24h      NUMERIC(28,2),
    market_cap      NUMERIC(28,2),
    PRIMARY KEY (symbol, ts)
);
CREATE INDEX price_ticks_recent_idx ON price_ticks (symbol, ts DESC);

-- ---------------------------------------------------------------------------
-- policy_violations — every refused publish, kept
-- ---------------------------------------------------------------------------
-- A block that is logged and forgotten is a block that will recur. These rows
-- are the evidence trail for the copyright posture and feed the /flock counters.
CREATE TABLE policy_violations (
    id          UUID PRIMARY KEY,
    story_id    UUID REFERENCES stories(id) ON DELETE CASCADE,
    article_id  UUID REFERENCES articles(id) ON DELETE SET NULL,
    run_id      UUID,
    code        TEXT NOT NULL,
    severity    TEXT NOT NULL CHECK (severity IN ('block','warn')),
    detail      TEXT NOT NULL,
    subject     TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX policy_violations_recent_idx ON policy_violations (created_at DESC);
CREATE INDEX policy_violations_story_idx  ON policy_violations (story_id);

-- ---------------------------------------------------------------------------
-- distribution
-- ---------------------------------------------------------------------------
CREATE TABLE newsletter_editions (
    id            UUID PRIMARY KEY,
    slug          TEXT NOT NULL UNIQUE,
    subject       TEXT NOT NULL,
    body_md       TEXT NOT NULL,
    story_ids     UUID[] NOT NULL DEFAULT '{}',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at       TIMESTAMPTZ
);
