-- Remember when a model has refused a piece of work, so we stop asking.
--
-- The Gander declined 279 of 290 topic framings, writing "No viable story"
-- because the topic genuinely did not have one yet. Nothing recorded that, so
-- the same handful of topics were re-offered on every pass, all day: a refusal
-- costs the same tokens as an acceptance, and on a 200,000-token daily
-- allowance those retries were displacing work that would have published.
--
-- Keyed by (kind, subject) rather than a foreign key, because the things worth
-- backing off from are not all rows — a trend topic exists only as a string
-- until it earns a gaggle.
create table if not exists model_declines (
    kind        text        not null,
    subject     text        not null,
    attempts    integer     not null default 1,
    -- Not before this do we ask again. Grows with each refusal.
    retry_after timestamptz not null,
    reason      text,
    first_seen  timestamptz not null default now(),
    last_seen   timestamptz not null default now(),
    primary key (kind, subject)
);

-- The read is always "what may I attempt now", so index the answer.
create index if not exists model_declines_ready
    on model_declines (kind, retry_after);
