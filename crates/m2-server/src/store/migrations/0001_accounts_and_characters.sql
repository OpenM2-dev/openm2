-- openm2's server: accounts and their characters (docs/specs/server/storage.md).

CREATE TABLE accounts (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 30),
    -- The PHC string of the password's Argon2id hash; never the password.
    password_hash text NOT NULL,
    -- The empire of its characters, once it has one.
    empire smallint CHECK (empire BETWEEN 1 AND 3),
    created_at timestamptz NOT NULL DEFAULT now()
);
-- Names are unique without case.
CREATE UNIQUE INDEX accounts_name_key ON accounts (lower(name));

CREATE TABLE characters (
    id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id bigint NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    slot smallint NOT NULL CHECK (slot BETWEEN 0 AND 3),
    name text NOT NULL CHECK (length(name) BETWEEN 1 AND 24),
    class smallint NOT NULL CHECK (class BETWEEN 0 AND 7),
    level smallint NOT NULL CHECK (level BETWEEN 1 AND 255),
    st smallint NOT NULL CHECK (st BETWEEN 0 AND 255),
    ht smallint NOT NULL CHECK (ht BETWEEN 0 AND 255),
    dx smallint NOT NULL CHECK (dx BETWEEN 0 AND 255),
    iq smallint NOT NULL CHECK (iq BETWEEN 0 AND 255),
    x integer NOT NULL,
    y integer NOT NULL,
    empire smallint NOT NULL CHECK (empire BETWEEN 1 AND 3),
    -- Seconds played.
    playtime bigint NOT NULL DEFAULT 0 CHECK (playtime >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    saved_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT characters_slot_key UNIQUE (account_id, slot)
);
-- Names are unique without case, across all accounts.
CREATE UNIQUE INDEX characters_name_key ON characters (lower(name));
