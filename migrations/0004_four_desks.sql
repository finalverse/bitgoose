-- Two more desks: capital markets, and high technology.
--
-- The general-interest sources were already being polled — Bloomberg, the FT,
-- CNBC, MarketWatch, Yahoo Finance, Ars Technica, The Verge, TechCrunch — and
-- almost everything they publish was being discarded, because the only two
-- desks that existed were AI and crypto. A Fed decision, a TSMC capex raise and
-- an SEC equities rule were all simply dropped.
--
-- Opening the two desks turns that discarded majority into coverage, without
-- adding a single new source or a single new request. The relevance gate stops
-- being a two-way filter that drops most of its input and becomes a four-way
-- router, with the same discipline: whole-word matching, specific over
-- sensitive, most-specific desk wins.

ALTER TABLE stories DROP CONSTRAINT IF EXISTS stories_beat_check;
ALTER TABLE stories ADD CONSTRAINT stories_beat_check
    CHECK (beat IN ('ai','crypto','markets','tech'));

ALTER TABLE raw_items DROP CONSTRAINT IF EXISTS raw_items_beat_check;
ALTER TABLE raw_items ADD CONSTRAINT raw_items_beat_check
    CHECK (beat IS NULL OR beat IN ('ai','crypto','markets','tech'));

ALTER TABLE sources DROP CONSTRAINT IF EXISTS sources_beat_check;
ALTER TABLE sources ADD CONSTRAINT sources_beat_check
    CHECK (beat IS NULL OR beat IN ('ai','crypto','markets','tech'));
