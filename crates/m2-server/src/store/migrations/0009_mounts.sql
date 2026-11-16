-- openm2's server: characters' mounts
-- (docs/specs/server/storage.md). Written with the character's row, only by a
-- newer save, as the skill group is.

ALTER TABLE characters
    -- The mount's level: 0 for none.
    ADD COLUMN mount_level smallint NOT NULL DEFAULT 0 CHECK (mount_level BETWEEN 0 AND 255),
    -- Its stamina.
    ADD COLUMN mount_stamina integer NOT NULL DEFAULT 0 CHECK (mount_stamina BETWEEN 0 AND 65535),
    -- Horseback skill points not spent yet.
    ADD COLUMN mount_skill_points smallint NOT NULL DEFAULT 0
        CHECK (mount_skill_points BETWEEN 0 AND 255);
