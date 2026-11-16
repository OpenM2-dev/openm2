-- openm2's server: characters' skills (docs/specs/server/storage.md).
-- Written with the character's row, in one transaction, and only by a
-- newer save, as items are.

-- The skill group: 0 before one is chosen, then 1 or 2.
ALTER TABLE characters
    ADD COLUMN skill_group smallint NOT NULL DEFAULT 0 CHECK (skill_group BETWEEN 0 AND 2);

CREATE TABLE skills (
    character_id integer NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    -- The skill's number in the content's skills.
    skill smallint NOT NULL CHECK (skill BETWEEN 1 AND 255),
    -- 1 to 19 normal, 20 to 29 master, 30 to 39 grand master, 40 perfect
    -- master; skills not learned have no row.
    level smallint NOT NULL CHECK (level BETWEEN 1 AND 40),
    PRIMARY KEY (character_id, skill)
);
