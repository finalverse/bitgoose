-- Human editors can steer discovery without bypassing the autonomous newsroom.
-- A direction is a search assignment, never an instruction to publish.

CREATE TABLE IF NOT EXISTS editorial_directions (
    id UUID PRIMARY KEY,
    title TEXT NOT NULL CHECK (char_length(title) BETWEEN 4 AND 200),
    briefing TEXT NOT NULL DEFAULT '',
    anchor_terms TEXT[] NOT NULL CHECK (cardinality(anchor_terms) BETWEEN 1 AND 12),
    keywords TEXT[] NOT NULL CHECK (cardinality(keywords) BETWEEN 1 AND 20),
    editorial_language TEXT NOT NULL CHECK (editorial_language IN ('en','zh','zh-hant','fr','es','ja','ko')),
    beat TEXT NOT NULL CHECK (beat IN ('ai','crypto','markets','tech','world','science','culture')),
    priority SMALLINT NOT NULL DEFAULT 50 CHECK (priority BETWEEN 1 AND 100),
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','paused','completed')),
    created_by TEXT NOT NULL,
    last_searched_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS editorial_directions_due_idx
    ON editorial_directions (status, last_searched_at, priority DESC);

CREATE TABLE IF NOT EXISTS editorial_audit_log (
    id UUID PRIMARY KEY,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    direction_id UUID REFERENCES editorial_directions(id) ON DELETE SET NULL,
    detail JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS editorial_audit_recent_idx
    ON editorial_audit_log (created_at DESC);

COMMENT ON TABLE editorial_directions IS
    'Human editor discovery assignments. They influence intake, never bypass autonomous verification or publication policy.';
