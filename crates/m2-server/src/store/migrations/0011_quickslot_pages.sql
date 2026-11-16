-- openm2's server: four pages of eight quickslots (docs/specs/server/storage.md).
-- Quickslots 0 to 31; those saved before are 0 to 7.
ALTER TABLE quickslots
    DROP CONSTRAINT quickslots_place_check,
    ADD CONSTRAINT quickslots_place_check CHECK (place BETWEEN 0 AND 31);
