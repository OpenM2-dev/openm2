-- openm2's server: players sign in only with an OpenID Connect provider
-- (docs/specs/server/storage.md), so no password is kept.

-- Accounts without a provider's identity had only a password: they go, and
-- their characters with them (characters, items, quickslots and skills
-- follow their account's rows).
DELETE FROM accounts a
WHERE NOT EXISTS (SELECT 1 FROM identities i WHERE i.account_id = a.id);

ALTER TABLE accounts DROP COLUMN password_hash;
