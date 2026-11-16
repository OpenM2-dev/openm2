-- openm2's server: what characters hold (docs/specs/server/storage.md).
-- A character's rows are replaced together with its row, in one
-- transaction, and only by a newer save.

CREATE TABLE items (
    character_id integer NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    -- Its place: 0 to 89 in the bag, 90 and up worn. The server checks
    -- where items may be; storage only that the numbers fit.
    slot integer NOT NULL CHECK (slot BETWEEN 0 AND 65535),
    -- The item's number in the content's items.
    vnum bigint NOT NULL CHECK (vnum BETWEEN 1 AND 4294967295),
    count smallint NOT NULL CHECK (count BETWEEN 1 AND 255),
    PRIMARY KEY (character_id, slot)
);

CREATE TABLE quickslots (
    character_id integer NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    -- Which quickslot, 0 to 7.
    place smallint NOT NULL CHECK (place BETWEEN 0 AND 7),
    -- What it holds: 1 an item of the bag, 2 a skill.
    kind smallint NOT NULL CHECK (kind BETWEEN 0 AND 255),
    -- The item's place in the bag, or the skill.
    position smallint NOT NULL CHECK (position BETWEEN 0 AND 255),
    PRIMARY KEY (character_id, place)
);
