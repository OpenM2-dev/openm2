-- openm2's server: what fights change, and the save count
-- (docs/specs/server/storage.md).

ALTER TABLE characters
    -- Health and mana; 0 health is dead. Characters from before these
    -- columns get the most an integer holds: the server brings it down to
    -- their maximum, so they enter with all their health and mana.
    ADD COLUMN hp integer NOT NULL DEFAULT 2147483647 CHECK (hp >= 0),
    ADD COLUMN sp integer NOT NULL DEFAULT 2147483647 CHECK (sp >= 0),
    -- Experience towards the next level, and gold.
    ADD COLUMN exp bigint NOT NULL DEFAULT 0 CHECK (exp BETWEEN 0 AND 4294967295),
    ADD COLUMN gold bigint NOT NULL DEFAULT 0 CHECK (gold BETWEEN 0 AND 4294967295),
    -- Status and skill points not spent yet.
    ADD COLUMN stat_points integer NOT NULL DEFAULT 0 CHECK (stat_points BETWEEN 0 AND 65535),
    ADD COLUMN skill_points integer NOT NULL DEFAULT 0 CHECK (skill_points BETWEEN 0 AND 65535),
    -- How many times the server saved it: a save is kept only when its
    -- count is higher, so an older save that lands late changes nothing.
    ADD COLUMN saves bigint NOT NULL DEFAULT 0 CHECK (saves >= 0);
