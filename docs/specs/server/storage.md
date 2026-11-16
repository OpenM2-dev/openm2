# Server: storage

What openm2's server stores and what it asks of storage. The core never waits on storage: it asks with a request (`m2_server_core::StoreRequest`) and goes on with the answer when it comes back (`StoreAnswer`), so a slow database never holds up other players.

Two stores answer the same requests by the same rules (`crates/m2-server/tests/storage.rs` checks both):
- **Postgres** (`m2_server::store::PgStore`): what `openm2-server serve` uses, at `OPENM2_DATABASE_URL`. [Below](#postgres): the schema and the migrations.
- **In memory** (`MemoryStore`, `serve --memory`): lost when the server stops. For the tests (which then need no database) and quick tries.

## Accounts
| Field | What |
|---|---|
| id | number, given by storage |
| name | the provider's id, a colon and 13 letters and digits (below); unique, compared without case |
| empire | 1 to 3, once the account has a character; none before |

- **No passwords**: every account belongs to a provider's player and is made at their first sign-in. There is no default account, no operator's command to add one, and no other way to register. An **identity** ties it to the provider: the issuer (1 to 512 bytes) and the player's subject there (1 to 255 bytes), unique together; it is found again by them only, never by name or e-mail. Its name is the provider's id, a colon and the first 13 characters of the base32 (RFC 4648, lowercase) of the SHA-256 of the issuer, a zero byte and the subject, such as `google:nixzoi2rkfeta`.

## Characters
| Field | What |
|---|---|
| id | number, given by storage, never 0 |
| account | whose |
| slot | 0 to 3, unique per account |
| name | 1 to 24 ASCII letters and digits; unique, compared without case |
| class | 0 to 7 (class and gender) |
| level | 1 to 255 |
| st, ht, dx, iq | strength, vitality, dexterity, intelligence |
| x, y | where it stands, in world units |
| empire | 1 to 3 |
| playtime | seconds played |
| hp, sp | health and mana; health 0 is dead ([`world.md`](world.md), "Dying") |
| exp | experience towards the next level |
| gold | gold |
| stat_points, skill_points | status and skill points not spent yet |
| saves | how many times the server saved it |
| items | what it holds: for each, its place (0 to 89 in the bag, 90 and up worn), the item's number, how many and its three sockets (a fishing pole's points and bait; [`world.md`](world.md), "Items"). Storage checks only that the numbers fit (a place up to 65,535, a number above 0, a count 1 to 255, sockets 0 to 4,294,967,295); the core checks where items may be when the character is selected |
| quickslots | its quickslots that hold something: index 0 to 31, kind, position |
| skill group | 0 before one is chosen, then 1 or 2 ([`world.md`](world.md), "Skills") |
| skills | the skills it has learned: for each, its number (1 to 255), level (1 to 40) and `next_read`, when it may train again (Unix seconds, 0 for any time; [`world.md`](world.md), "Training"). Storage checks only that the numbers fit |
| read bonus | the percent added to its next skill book's or soul stone's chance, 0 to 100 |
| friends | loaded with it, never written by a save or a new character: each friend's number, name and account (the core finds the friend's connection by it; it never goes on the wire), by name in lowercase ("Friends", below) |

## Requests
| Request | Answer |
|---|---|
| IdentityLogin {provider, issuer, subject} | the account of that identity, made (with its identity, at once) if there is none; or "unavailable". Two first sign-ins at once make one account |
| LoadCharacters {account} | the account's characters with their items, quickslots, skills and friends, or an error |
| CreateCharacter {account, character} | the character with its id, stored with its starting items; or "name taken", "no room" (the slot is taken or there are four), "unavailable". The account takes the character's empire if it had none |
| SaveCharacter {account, character} | done, or an error; everything but the name, class, slot and empire is written, its items, quickslots and skills in place of the stored ones, all at once, only for a character of that account, and only when the save's count is higher than the one stored (an older or repeated save is skipped, its items too, and answers done) |

| AddFriendship {characters} | done, or an error: two characters become friends, both ways, in either order; a friendship stored already stays as it is. The same character twice, or one that does not exist, is an error |
| RemoveFriendship {characters} | done, or an error: two characters stop being friends, in either order; one not stored is no error, the same character twice is |

- A failed save is logged; the player plays on.
- **Saves in order** (openm2): requests run side by side at the edge, so two saves of one character can land in either order (the 5-minute save and the one on leaving). Each save counts one more than the last (`Character::saves`, starting from what storage holds), and storage keeps only a higher count.
- **A login waits for its saves** (openm2): while a save of an account is under way, the core holds back loading its characters; they load once the last save has landed, so a player who leaves and logs straight back in finds what it just saved. A friendship's write counts as a save under way of both its characters' accounts.

## Friends
openm2's own. A friendship is between two characters, both ways: one record, the lower character number first, so it can never be half. It goes with either character. It is written on its own (AddFriendship, RemoveFriendship) when the players make or end it, never by a save; a character's friends come with it when its account's characters load.
- **No item in two places** (openm2): a character's items are written only with the character, so storage never holds half a save; and a player who drops something is saved at once ([`world.md`](world.md), "The ground"), so the item has left its storage before anyone can pick it up and be saved with it.

## Postgres
- **Tables** (`crates/m2-server/src/store/migrations/`): `accounts` and `characters` with the fields above, plus `created_at` on both and `saved_at` on characters. Migration 0002 added health, mana, experience, gold, the points and the save count; characters from before it got the largest health and mana a column holds, which the server brings down to their maxima when they enter, so they enter with both full. Migration 0003 added `items` (character, place, number, count; one row a place) and `quickslots` (character, place, kind, position), deleted with their character. Migration 0004 added `skill_group` to characters (0 for those from before it) and `skills` (character, skill, level; one row a skill learned), deleted with their character. Migration 0005 made `password_hash` optional and added `identities` (issuer, subject, account; the issuer and subject the primary key), deleted with their account. Migration 0006 deleted every account without an identity, its characters, items, quickslots and skills with it, and dropped `password_hash`. Migration 0007 added `next_read` to skills and `read_bonus` to characters, 0 for those from before it. Names are unique through indexes on `lower(name)` (`accounts_name_key`, `characters_name_key`); a character's slot through `(account_id, slot)` (`characters_slot_key`). Checks keep every number in the range the core uses (a slot 0 to 3, a class 0 to 7, stats 0 to 255, …). Deleting an account deletes its characters. Migration 0008 added `friendships` (`first_id`, `second_id`, `created_at`; the pair the primary key, `first_id < second_id`, each side deleted with its character; an index on `second_id`). Migration 0009 added `mount_level` (0 to 255), `mount_stamina` (0 to 65,535) and `mount_skill_points` (0 to 255) to characters, 0 for those from before it, written by a newer save like the skill group. Migration 0010 added `incognito` to characters (a game master's character whose mark doesn't show, `/gm off`), false for those from before it, saved with the character's row. Migration 0011 let quickslots be 0 to 31 (four pages of eight), the check named `quickslots_place_check` as before; those saved before stay where they were, on the first page. Migration 0012 added `socket0`, `socket1` and `socket2` to items (0 to 4,294,967,295), 0 for those from before it.
- **Queries** are SQL with bound parameters only, written in `store/postgres.rs`; no query is built from text.
- **Loading** reads the characters, then their items, quickslots and skills, in one transaction.
- **Saving** is one transaction: `UPDATE … WHERE saves < $count`; when it changes a row, the character's items, quickslots and skills are deleted and written again (`INSERT … SELECT … FROM UNNEST(…)`), then it commits; when it changes no row, a second query tells an older save (done) from a character that is not the account's (an error), and nothing is written.
- **Creating a character** runs in one transaction: the account's row is locked (`FOR UPDATE`), so two creations of one account count its characters one after the other; a broken name index answers "name taken", a broken slot key "no room"; then the account takes the character's empire if it had none, and the starting items are written.
- **Migrations:** numbered SQL files compiled into the binary. `openm2-server serve` and `openm2-server migrate` apply the ones the database lacks, in order, in one transaction, under an advisory lock (two servers starting together do not both migrate), and record them in `openm2_migrations`. A database with a migration this server does not know (a newer server migrated it) is refused. A migration is never changed once released; a change is a new file.
- **One world per database** (openm2):
  - `openm2-server serve` takes a session advisory lock (`pg_try_advisory_lock`) on a connection of its own, after the migrations and before it listens, and keeps it while it runs. It lets go after its last saves on stopping.
  - A second server started on the same database logs that it waits, and waits (`pg_advisory_lock`) until the first lets go, or until its own Ctrl+C or SIGTERM.
  - Postgres lets go of the lock when its connection closes, so a crashed server never keeps the world.
  - The lock is a guard against starting two servers, not a lease: a server whose lock connection breaks does not notice.
  - `serve --memory` takes none.
- **Connections:** at most 8, waiting 5 s for a free one, and one more for the world's lock. The URL holds the database's password: it comes only from `OPENM2_DATABASE_URL` and is never printed.
- **TLS** (`docs/server.md`, "The database's TLS"; checked by `crates/m2-server/tests/postgres_tls.rs`):
  - **The default is `verify-full` (openm2):** when neither the URL's `sslmode` (or `ssl-mode`) nor `PGSSLMODE` names a mode, the connection requires TLS and checks the database's certificate against webpki-roots, plus the URL's `sslrootcert`, and its name against the URL's host.
  - **A mode the URL names is kept.** `compose.yaml`'s database beside the server says `sslmode=disable`.
  - **A TLS failure is not tried again:** no TLS offered, an unknown authority, another name (`OpenError::Tls`). Only a database that doesn't answer is tried again, for 30 s at start.
  - **At start the server logs** the host, the port and the mode. It warns about `require`, `prefer` and `allow`, which don't check the certificate.
- **Errors** are logged without the URL. A login storage cannot check closes the connection; a character it cannot create is refused (CharacterCreateFailed 1).
