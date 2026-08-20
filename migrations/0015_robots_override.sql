-- An explicit, per-source exemption from the robots.txt gate.
--
-- Google News publishes RSS endpoints that carry nothing but headlines, links
-- and timestamps, and its robots.txt is `Disallow: /` for every agent. Our gate
-- read that correctly and refused to poll nine feeds, which is why the site had
-- no aggregator view of what was actually hot while Bitcoin moved 7.9% in a day.
--
-- The operator's call, recorded per source rather than taken globally. A single
-- switch would have turned the gate off for all 57 sources, including the
-- publishers whose text we extract — and *that* gate is the one holding up the
-- copyright posture. This one names its exceptions, shows them in `bg doctor`
-- and on the sources page, and is reversible one row at a time.
--
-- What it does NOT relax: the ≤25-word quote ceiling, mandatory attribution,
-- the canonical link-out, conditional GET, or the per-source rate limit. Those
-- are enforced in bg-core::policy at publish time and apply to every source
-- however it was fetched.
alter table sources
    add column if not exists robots_override boolean not null default false;

comment on column sources.robots_override is
    'Operator has authorised polling this source despite its robots.txt. Set deliberately, per source; never a global default.';
