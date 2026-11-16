-- openm2's server: accounts of players who sign in with an OpenID Connect
-- provider (docs/specs/server/storage.md).

-- Such an account has no password: a password login never opens it.
ALTER TABLE accounts ALTER COLUMN password_hash DROP NOT NULL;

-- Who an account belongs to at a provider: its issuer and the player's id
-- there (`iss` and `sub` of the ID token). An account is never found by its
-- name or an e-mail address.
CREATE TABLE identities (
    issuer text NOT NULL CHECK (length(issuer) BETWEEN 1 AND 512),
    subject text NOT NULL CHECK (length(subject) BETWEEN 1 AND 255),
    account_id bigint NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (issuer, subject)
);
CREATE INDEX identities_account_id ON identities (account_id);
