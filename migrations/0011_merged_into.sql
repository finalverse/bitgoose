-- Where a folded story's reporting went.
--
-- `bg recluster` moves a duplicate story's items onto the story it was always
-- part of and kills the husk. That left the husk's URL — which may be the one
-- somebody actually shared — serving an empty shell with a 200, which is worse
-- than a 404: a reader gets a blank page and a crawler files it as thin
-- content. Recording the destination lets the URL redirect to the reporting.
--
-- ON DELETE SET NULL rather than CASCADE: losing the pointer should degrade the
-- redirect to a not-found, never delete a published story.
ALTER TABLE stories
  ADD COLUMN merged_into UUID REFERENCES stories(id) ON DELETE SET NULL;

-- Only killed stories are folded, and nothing may be folded into itself.
ALTER TABLE stories
  ADD CONSTRAINT stories_merge_is_a_kill
  CHECK (merged_into IS NULL OR (status = 'killed' AND merged_into <> id));

CREATE INDEX IF NOT EXISTS stories_merged_into_idx
  ON stories (merged_into) WHERE merged_into IS NOT NULL;
