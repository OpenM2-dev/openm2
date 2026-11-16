-- openm2's server: friendships between characters
-- (docs/specs/server/storage.md, "Friends").
-- One row a friendship, both ways at once: the lower character number
-- first. Written on its own, never by a character's save; it goes with
-- either character.

CREATE TABLE friendships (
    first_id integer NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    second_id integer NOT NULL REFERENCES characters (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (first_id, second_id),
    CHECK (first_id < second_id)
);

-- A character's friends are looked up from either side.
CREATE INDEX friendships_second_id ON friendships (second_id);
