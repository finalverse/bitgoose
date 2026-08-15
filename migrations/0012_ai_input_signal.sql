-- Whether a publisher permits its text being put into a model.
--
-- Distinct from `robots_ok`, which answers "may we fetch this?". A great many
-- sites now answer those two questions differently: they welcome crawlers and
-- link traffic while blocking GPTBot, ClaudeBot and CCBot by name, or publish
-- `Content-Signal: ai-train=no`. theaiinsider.tech does both.
--
-- Reading a welcome for crawlers as consent to feed the text to a model is
-- choosing the interpretation that suits us. A source marked false here is
-- still polled, still ranked and still linked to — it simply never reaches the
-- Skein, and never has its body text stored.
--
-- Defaults true: the great majority of sites say nothing, and inventing a
-- refusal would silently drop sources nobody asked us to drop.
ALTER TABLE sources
  ADD COLUMN ai_input_ok BOOLEAN NOT NULL DEFAULT true;

-- What the site actually said, kept so a change of posture is visible rather
-- than inferred fresh each time.
ALTER TABLE sources
  ADD COLUMN ai_signal TEXT;
