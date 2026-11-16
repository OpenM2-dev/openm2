-- openm2's server: training skills past a master's
-- (docs/specs/server/storage.md). Written with the
-- character's row and its skills, in one transaction, as they are.

-- When the skill may train again with a skill book or a soul stone, in
-- Unix seconds; 0 for any time.
ALTER TABLE skills
    ADD COLUMN next_read bigint NOT NULL DEFAULT 0 CHECK (next_read >= 0);

-- The percent added to the character's next book's or stone's chance.
ALTER TABLE characters
    ADD COLUMN read_bonus smallint NOT NULL DEFAULT 0 CHECK (read_bonus BETWEEN 0 AND 100);
