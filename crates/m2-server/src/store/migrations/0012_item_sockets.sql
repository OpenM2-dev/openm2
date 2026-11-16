-- openm2's server: items keep three socket numbers (docs/specs/server/storage.md):
-- a fishing pole's points and its bait. Items saved before
-- have none.
ALTER TABLE items
    ADD COLUMN socket0 bigint NOT NULL DEFAULT 0 CHECK (socket0 BETWEEN 0 AND 4294967295),
    ADD COLUMN socket1 bigint NOT NULL DEFAULT 0 CHECK (socket1 BETWEEN 0 AND 4294967295),
    ADD COLUMN socket2 bigint NOT NULL DEFAULT 0 CHECK (socket2 BETWEEN 0 AND 4294967295);
