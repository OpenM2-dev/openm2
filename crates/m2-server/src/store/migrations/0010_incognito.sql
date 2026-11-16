-- openm2's server: a game master's character whose mark doesn't show
-- (docs/specs/server/storage.md). Written with the
-- character's row.
ALTER TABLE characters
    ADD COLUMN incognito boolean NOT NULL DEFAULT false;
