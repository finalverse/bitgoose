-- Admit the Skein to the roster.
--
-- Its own migration rather than an edit to 0005: that file was already applied
-- and sqlx records a checksum per migration, so changing it after the fact
-- turns every existing database into a failed startup.

ALTER TABLE agents DROP CONSTRAINT IF EXISTS agents_role_check;
ALTER TABLE agents ADD CONSTRAINT agents_role_check CHECK (role IN (
    'scout', 'gosling', 'curator', 'scribe', 'sentinel', 'quant',
    'copydesk', 'gander', 'herald', 'ombuds', 'skein'
));
