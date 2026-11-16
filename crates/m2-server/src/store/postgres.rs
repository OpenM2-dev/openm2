//! Accounts and characters in Postgres (`docs/specs/server/storage.md`).
//!
//! Every query is SQL with bound parameters, written here and checked by the
//! tests against a real database (`OPENM2_TEST_DATABASE_URL`). The schema
//! comes from the migrations in `migrations/`, applied in order by
//! [`PgStore::migrate`] (`openm2-server serve` and `migrate` run it).

use std::str::FromStr as _;
use std::time::Duration;

use m2_server_core::rules::MAX_CHARACTERS;
use m2_server_core::{
    AccountId, Character, CreateRefusal, QuickSlot, StoreAnswer, StoreError, StoreRequest,
    StoredFriend, StoredItem, StoredSkill,
};
use secrecy::{ExposeSecret as _, SecretString};
use sqlx::Row as _;
use sqlx::postgres::{PgConnectOptions, PgConnection, PgPool, PgPoolOptions, PgRow, PgSslMode};

use super::identity_account_name;

/// The migrations, in order: a version, a name, the SQL.
const MIGRATIONS: &[(i32, &str, &str)] = &[
    (
        1,
        "accounts and characters",
        include_str!("migrations/0001_accounts_and_characters.sql"),
    ),
    (
        2,
        "points and saves",
        include_str!("migrations/0002_points_and_saves.sql"),
    ),
    (
        3,
        "items and quickslots",
        include_str!("migrations/0003_items_and_quickslots.sql"),
    ),
    (4, "skills", include_str!("migrations/0004_skills.sql")),
    (
        5,
        "identities",
        include_str!("migrations/0005_identities.sql"),
    ),
    (
        6,
        "providers only",
        include_str!("migrations/0006_providers_only.sql"),
    ),
    (
        7,
        "skill training",
        include_str!("migrations/0007_skill_training.sql"),
    ),
    (8, "friends", include_str!("migrations/0008_friends.sql")),
    (9, "mounts", include_str!("migrations/0009_mounts.sql")),
    (
        10,
        "incognito",
        include_str!("migrations/0010_incognito.sql"),
    ),
    (
        11,
        "quickslot pages",
        include_str!("migrations/0011_quickslot_pages.sql"),
    ),
    (
        12,
        "item sockets",
        include_str!("migrations/0012_item_sockets.sql"),
    ),
];

/// The advisory lock that keeps two servers from migrating at once.
const MIGRATION_LOCK: i64 = 0x6f70_656e_6d32;

/// The advisory lock a running server holds, so that one server at a time
/// runs a database's world (openm2).
const WORLD_LOCK: i64 = 0x6f70_656e_6d32_7700;

/// Connections to the database at most: one server needs few.
const MAX_CONNECTIONS: u32 = 8;
/// How long a request waits for a free connection.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// The constraint of a character name that is taken.
const CHARACTER_NAME_KEY: &str = "characters_name_key";

/// The URL's keys that name the TLS mode (sqlx reads both).
const SSL_MODE_KEYS: [&str; 2] = ["sslmode", "ssl-mode"];
/// The environment variable sqlx (like libpq) reads the TLS mode from.
const SSL_MODE_ENV: &str = "PGSSLMODE";

/// Why the store could not open.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The database URL does not parse. (Its text is not repeated: it holds
    /// the password.)
    #[error("the database URL is not a valid Postgres URL")]
    BadUrl,
    /// The database did not answer.
    #[error("connecting to the database failed: {0}")]
    Connect(sqlx::Error),
    /// TLS with the database failed: it offers none, or its certificate is
    /// not trusted or not for its name. Trying again does not mend it.
    #[error(
        "TLS with the database failed: {0} (the URL's sslmode is verify-full unless it names another, docs/server.md)"
    )]
    Tls(sqlx::Error),
    /// A migration failed.
    #[error("migrating the database failed: {0}")]
    Migrate(sqlx::Error),
    /// The database was migrated by a newer server.
    #[error(
        "the database has migration {0}, which this server does not know: run a newer openm2-server"
    )]
    TooNew(i32),
    /// Taking the world's lock failed.
    #[error("taking the world's lock failed: {0}")]
    Lock(sqlx::Error),
}

/// The world's lock, held on a connection of its own until
/// [`WorldLock::release`] or until it is dropped (which closes the
/// connection, and Postgres lets go of the lock).
#[derive(Debug)]
pub struct WorldLock {
    connection: PgConnection,
}

impl WorldLock {
    /// Lets go of the lock and closes its connection.
    pub async fn release(mut self) {
        let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(WORLD_LOCK)
            .execute(&mut self.connection)
            .await;
        let _ = sqlx::Connection::close(self.connection).await;
    }
}

/// Accounts and characters in Postgres.
#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    /// The options of the database at `url`. TLS checks the database's
    /// certificate and name (`sslmode=verify-full`) unless the URL or
    /// `PGSSLMODE` names another mode (openm2; sqlx and libpq would `prefer`
    /// TLS without checking it).
    pub fn options(url: &SecretString) -> Result<PgConnectOptions, OpenError> {
        options_from(
            url.expose_secret(),
            std::env::var_os(SSL_MODE_ENV).is_some(),
        )
    }

    /// Connects to the database at `url` ([`PgStore::options`]). Call
    /// [`PgStore::migrate`] before using it.
    pub async fn connect(url: &SecretString) -> Result<Self, OpenError> {
        Self::connect_with(Self::options(url)?).await
    }

    /// Connects with these options (tests use them to pick a schema).
    pub async fn connect_with(options: PgConnectOptions) -> Result<Self, OpenError> {
        let options = options.application_name("openm2-server");
        let pool = PgPoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .acquire_timeout(ACQUIRE_TIMEOUT)
            .connect_with(options)
            .await
            .map_err(connect_error)?;
        Ok(Self { pool })
    }

    /// Takes the world's lock if no other server holds it.
    pub async fn try_lock_world(&self) -> Result<Option<WorldLock>, OpenError> {
        let mut connection = self.own_connection().await?;
        let taken: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(WORLD_LOCK)
            .fetch_one(&mut connection)
            .await
            .map_err(OpenError::Lock)?;
        Ok(taken.then_some(WorldLock { connection }))
    }

    /// Takes the world's lock, waiting as long as another server holds it.
    pub async fn lock_world(&self) -> Result<WorldLock, OpenError> {
        let mut connection = self.own_connection().await?;
        sqlx::query("SELECT pg_advisory_lock($1)")
            .bind(WORLD_LOCK)
            .execute(&mut connection)
            .await
            .map_err(OpenError::Lock)?;
        Ok(WorldLock { connection })
    }

    /// A connection out of the pool, for a lock that lives as long as it.
    async fn own_connection(&self) -> Result<PgConnection, OpenError> {
        Ok(self.pool.acquire().await.map_err(OpenError::Lock)?.detach())
    }

    /// Applies the migrations the database does not have yet, in one
    /// transaction under an advisory lock; returns the versions applied.
    pub async fn migrate(&self) -> Result<Vec<i32>, OpenError> {
        let mut tx = self.pool.begin().await.map_err(OpenError::Migrate)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(MIGRATION_LOCK)
            .execute(&mut *tx)
            .await
            .map_err(OpenError::Migrate)?;
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS openm2_migrations (
                version integer PRIMARY KEY,
                name text NOT NULL,
                applied_at timestamptz NOT NULL DEFAULT now()
            )",
        )
        .execute(&mut *tx)
        .await
        .map_err(OpenError::Migrate)?;
        let applied: Vec<i32> = sqlx::query_scalar("SELECT version FROM openm2_migrations")
            .fetch_all(&mut *tx)
            .await
            .map_err(OpenError::Migrate)?;
        let known = MIGRATIONS
            .iter()
            .map(|(version, _, _)| *version)
            .max()
            .unwrap_or(0);
        if let Some(newer) = applied.iter().copied().filter(|v| *v > known).max() {
            return Err(OpenError::TooNew(newer));
        }
        let mut done = Vec::new();
        for (version, name, sql) in MIGRATIONS {
            if applied.contains(version) {
                continue;
            }
            sqlx::raw_sql(*sql)
                .execute(&mut *tx)
                .await
                .map_err(OpenError::Migrate)?;
            sqlx::query("INSERT INTO openm2_migrations (version, name) VALUES ($1, $2)")
                .bind(version)
                .bind(name)
                .execute(&mut *tx)
                .await
                .map_err(OpenError::Migrate)?;
            done.push(*version);
        }
        tx.commit().await.map_err(OpenError::Migrate)?;
        Ok(done)
    }

    /// Closes every connection.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub(super) async fn handle(&self, request: StoreRequest) -> StoreAnswer {
        match request {
            StoreRequest::IdentityLogin {
                provider,
                issuer,
                subject,
            } => StoreAnswer::LoginChecked(
                self.identity_login(&provider, &issuer, &subject)
                    .await
                    .map_err(failed),
            ),
            StoreRequest::LoadCharacters { account } => {
                StoreAnswer::Characters(self.load(account).await.map_err(failed))
            }
            StoreRequest::CreateCharacter { account, character } => {
                StoreAnswer::CharacterCreated(self.create(account, character).await)
            }
            StoreRequest::SaveCharacter { account, character } => {
                StoreAnswer::Saved(self.save(account, &character).await)
            }
            StoreRequest::AddFriendship { characters } => {
                StoreAnswer::FriendshipWritten(self.add_friendship(characters).await)
            }
            StoreRequest::RemoveFriendship { characters } => {
                StoreAnswer::FriendshipWritten(self.remove_friendship(characters).await)
            }
        }
    }

    /// Two characters become friends: one row, the lower number first,
    /// kept as it is when it is there. A character that does not exist
    /// fails on the foreign key.
    async fn add_friendship(&self, characters: [u32; 2]) -> Result<(), StoreError> {
        let (first, second) = friendship(characters)?;
        sqlx::query(
            "INSERT INTO friendships (first_id, second_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
        )
        .bind(first)
        .bind(second)
        .execute(&self.pool)
        .await
        .map_err(failed)?;
        Ok(())
    }

    /// Two characters stop being friends; no row is no error.
    async fn remove_friendship(&self, characters: [u32; 2]) -> Result<(), StoreError> {
        let (first, second) = friendship(characters)?;
        sqlx::query("DELETE FROM friendships WHERE first_id = $1 AND second_id = $2")
            .bind(first)
            .bind(second)
            .execute(&self.pool)
            .await
            .map_err(failed)?;
        Ok(())
    }

    /// The account of a provider's player: found by the issuer and subject,
    /// or made with them at the first sign-in. Two first sign-ins at once
    /// make one account: the second finds the first's.
    async fn identity_login(
        &self,
        provider: &str,
        issuer: &str,
        subject: &str,
    ) -> Result<m2_server_core::Account, sqlx::Error> {
        if let Some(account) = self.find_identity(issuer, subject).await? {
            return Ok(account);
        }
        let name = identity_account_name(provider, issuer, subject);
        match self.make_identity(&name, issuer, subject).await {
            Ok(Some(id)) => Ok(m2_server_core::Account {
                id,
                name,
                empire: None,
            }),
            Ok(None) | Err(_) => self
                .find_identity(issuer, subject)
                .await?
                .ok_or(sqlx::Error::RowNotFound),
        }
    }

    async fn find_identity(
        &self,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<m2_server_core::Account>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT a.id, a.name, a.empire FROM identities i
             JOIN accounts a ON a.id = i.account_id
             WHERE i.issuer = $1 AND i.subject = $2",
        )
        .bind(issuer)
        .bind(subject)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| -> Result<m2_server_core::Account, sqlx::Error> {
            Ok(m2_server_core::Account {
                id: AccountId(convert(row.try_get::<i64, _>("id")?, "account id")?),
                name: row.try_get("name")?,
                empire: row
                    .try_get::<Option<i16>, _>("empire")?
                    .map(|e| convert(e, "empire"))
                    .transpose()?,
            })
        })
        .transpose()
    }

    /// Makes the account and its identity in one transaction; `None` when
    /// another sign-in made the identity first.
    async fn make_identity(
        &self,
        name: &str,
        issuer: &str,
        subject: &str,
    ) -> Result<Option<AccountId>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let id: i64 = sqlx::query_scalar("INSERT INTO accounts (name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(&mut *tx)
            .await?;
        let made = sqlx::query(
            "INSERT INTO identities (issuer, subject, account_id) VALUES ($1, $2, $3)
             ON CONFLICT DO NOTHING",
        )
        .bind(issuer)
        .bind(subject)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if made.rows_affected() == 0 {
            tx.rollback().await?;
            return Ok(None);
        }
        tx.commit().await?;
        Ok(Some(AccountId(convert(id, "account id")?)))
    }

    /// The account's characters with their items, quickslots, skills and
    /// friends, read in one transaction so they match.
    async fn load(&self, account: AccountId) -> Result<Vec<Character>, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT id, slot, name, class, level, st, ht, dx, iq, x, y, empire, playtime,
                 hp, sp, exp, gold, stat_points, skill_points, saves, skill_group, read_bonus,
                 mount_level, mount_stamina, mount_skill_points, incognito
             FROM characters WHERE account_id = $1 ORDER BY slot",
        )
        .bind(account_id(account)?)
        .fetch_all(&mut *tx)
        .await?;
        let mut characters = rows
            .iter()
            .map(character_of)
            .collect::<Result<Vec<_>, _>>()?;
        let ids = characters
            .iter()
            .map(|c| convert(c.id, "character id"))
            .collect::<Result<Vec<i32>, _>>()?;
        let items = sqlx::query(
            "SELECT character_id, slot, vnum, count, socket0, socket1, socket2 FROM items
             WHERE character_id = ANY($1) ORDER BY character_id, slot",
        )
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await?;
        for row in &items {
            let owner: u32 = convert(row.try_get::<i32, _>("character_id")?, "character id")?;
            let item = StoredItem {
                slot: convert(row.try_get::<i32, _>("slot")?, "slot")?,
                vnum: convert(row.try_get::<i64, _>("vnum")?, "vnum")?,
                count: convert(row.try_get::<i16, _>("count")?, "count")?,
                sockets: [
                    convert(row.try_get::<i64, _>("socket0")?, "socket")?,
                    convert(row.try_get::<i64, _>("socket1")?, "socket")?,
                    convert(row.try_get::<i64, _>("socket2")?, "socket")?,
                ],
            };
            if let Some(character) = characters.iter_mut().find(|c| c.id == owner) {
                character.items.push(item);
            }
        }
        let quickslots = sqlx::query(
            "SELECT character_id, place, kind, position FROM quickslots
             WHERE character_id = ANY($1) ORDER BY character_id, place",
        )
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await?;
        for row in &quickslots {
            let owner: u32 = convert(row.try_get::<i32, _>("character_id")?, "character id")?;
            let quick = QuickSlot {
                index: convert(row.try_get::<i16, _>("place")?, "place")?,
                kind: convert(row.try_get::<i16, _>("kind")?, "kind")?,
                position: convert(row.try_get::<i16, _>("position")?, "position")?,
            };
            if let Some(character) = characters.iter_mut().find(|c| c.id == owner) {
                character.quickslots.push(quick);
            }
        }
        let skills = sqlx::query(
            "SELECT character_id, skill, level, next_read FROM skills
             WHERE character_id = ANY($1) ORDER BY character_id, skill",
        )
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await?;
        for row in &skills {
            let owner: u32 = convert(row.try_get::<i32, _>("character_id")?, "character id")?;
            let skill = StoredSkill {
                skill: convert(row.try_get::<i16, _>("skill")?, "skill")?,
                level: convert(row.try_get::<i16, _>("level")?, "skill level")?,
                next_read: convert(row.try_get::<i64, _>("next_read")?, "next read")?,
            };
            if let Some(character) = characters.iter_mut().find(|c| c.id == owner) {
                character.skills.push(skill);
            }
        }
        // A friendship is one row; each side finds the other in it.
        let friends = sqlx::query(
            "SELECT f.character_id, c.id, c.name, c.account_id
             FROM (SELECT first_id AS character_id, second_id AS friend_id
                   FROM friendships WHERE first_id = ANY($1)
                   UNION ALL
                   SELECT second_id, first_id
                   FROM friendships WHERE second_id = ANY($1)) f
             JOIN characters c ON c.id = f.friend_id
             ORDER BY f.character_id, lower(c.name)",
        )
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await?;
        for row in &friends {
            let owner: u32 = convert(row.try_get::<i32, _>("character_id")?, "character id")?;
            let friend = StoredFriend {
                id: convert(row.try_get::<i32, _>("id")?, "character id")?,
                name: row.try_get("name")?,
                account: AccountId(convert(row.try_get::<i64, _>("account_id")?, "account id")?),
            };
            if let Some(character) = characters.iter_mut().find(|c| c.id == owner) {
                character.friends.push(friend);
            }
        }
        tx.commit().await?;
        Ok(characters)
    }

    async fn create(
        &self,
        account: AccountId,
        character: Character,
    ) -> Result<Character, CreateRefusal> {
        let unavailable = |e: sqlx::Error| {
            tracing::warn!(target: "m2_server", "postgres: creating a character failed: {e}");
            CreateRefusal::Unavailable
        };
        let id = account_id(account).map_err(unavailable)?;
        let mut tx = self.pool.begin().await.map_err(unavailable)?;
        // The account's row is locked so that two creations of one account
        // count its characters one after the other.
        let found: Option<i64> =
            sqlx::query_scalar("SELECT id FROM accounts WHERE id = $1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(unavailable)?;
        if found.is_none() {
            return Err(CreateRefusal::Unavailable);
        }
        let owned: i64 =
            sqlx::query_scalar("SELECT count(*) FROM characters WHERE account_id = $1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(unavailable)?;
        if owned >= MAX_CHARACTERS as i64 {
            return Err(CreateRefusal::NoRoom);
        }
        let points = Points::of(&character).map_err(unavailable)?;
        let inserted = sqlx::query_scalar::<_, i32>(
            "INSERT INTO characters (account_id, slot, name, class, level, st, ht, dx, iq, x, y,
                 empire, playtime, hp, sp, exp, gold, stat_points, skill_points, saves,
                 skill_group, mount_level, mount_stamina, mount_skill_points)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16,
                 $17, $18, $19, $20, $21, $22, $23, $24)
             RETURNING id",
        )
        .bind(id)
        .bind(i16::from(character.slot))
        .bind(&character.name)
        .bind(i16::from(character.class))
        .bind(i16::from(character.level))
        .bind(i16::from(character.st))
        .bind(i16::from(character.ht))
        .bind(i16::from(character.dx))
        .bind(i16::from(character.iq))
        .bind(character.x)
        .bind(character.y)
        .bind(i16::from(character.empire))
        .bind(i64::try_from(character.playtime).unwrap_or(i64::MAX))
        .bind(points.hp)
        .bind(points.sp)
        .bind(points.exp)
        .bind(points.gold)
        .bind(points.stat_points)
        .bind(points.skill_points)
        .bind(points.saves)
        .bind(i16::from(character.skill_group))
        .bind(i16::from(character.mount_level))
        .bind(i32::from(character.mount_stamina))
        .bind(i16::from(character.mount_skill_points))
        .fetch_one(&mut *tx)
        .await;
        let new_id = match inserted {
            Ok(new_id) => new_id,
            Err(e) => {
                return Err(match constraint(&e) {
                    Some(CHARACTER_NAME_KEY) => CreateRefusal::NameTaken,
                    Some(_) => CreateRefusal::NoRoom,
                    None => unavailable(e),
                });
            }
        };
        sqlx::query("UPDATE accounts SET empire = COALESCE(empire, $2) WHERE id = $1")
            .bind(id)
            .bind(i16::from(character.empire))
            .execute(&mut *tx)
            .await
            .map_err(unavailable)?;
        write_belongings(&mut tx, new_id, &character)
            .await
            .map_err(unavailable)?;
        tx.commit().await.map_err(unavailable)?;
        // A new character has no friends: friendships are written on their
        // own (`storage.md`, "Friends").
        Ok(Character {
            id: convert(new_id, "character id").map_err(unavailable)?,
            friends: Vec::new(),
            ..character
        })
    }

    /// Stores the character as it is now, with its items, quickslots and
    /// skills in place of the stored ones, in one transaction, if its save
    /// count is higher than the one stored; an older save is skipped.
    async fn save(&self, account: AccountId, character: &Character) -> Result<(), StoreError> {
        let id = i32::try_from(character.id)
            .map_err(|_| StoreError("a character id past i32".to_owned()))?;
        let owner = account_id(account).map_err(failed)?;
        let points = Points::of(character).map_err(failed)?;
        let mut tx = self.pool.begin().await.map_err(failed)?;
        let result = sqlx::query(
            "UPDATE characters SET x = $3, y = $4, playtime = $5, level = $6, st = $7, ht = $8,
                 dx = $9, iq = $10, hp = $11, sp = $12, exp = $13, gold = $14, stat_points = $15,
                 skill_points = $16, saves = $17, skill_group = $18, read_bonus = $19,
                 mount_level = $20, mount_stamina = $21, mount_skill_points = $22,
                 incognito = $23,
                 saved_at = now()
             WHERE id = $1 AND account_id = $2 AND saves < $17",
        )
        .bind(id)
        .bind(owner)
        .bind(character.x)
        .bind(character.y)
        .bind(i64::try_from(character.playtime).unwrap_or(i64::MAX))
        .bind(i16::from(character.level))
        .bind(i16::from(character.st))
        .bind(i16::from(character.ht))
        .bind(i16::from(character.dx))
        .bind(i16::from(character.iq))
        .bind(points.hp)
        .bind(points.sp)
        .bind(points.exp)
        .bind(points.gold)
        .bind(points.stat_points)
        .bind(points.skill_points)
        .bind(points.saves)
        .bind(i16::from(character.skill_group))
        .bind(i16::from(character.read_bonus))
        .bind(i16::from(character.mount_level))
        .bind(i32::from(character.mount_stamina))
        .bind(i16::from(character.mount_skill_points))
        .bind(character.incognito)
        .execute(&mut *tx)
        .await
        .map_err(failed)?;
        if result.rows_affected() > 0 {
            write_belongings(&mut tx, id, character)
                .await
                .map_err(failed)?;
            tx.commit().await.map_err(failed)?;
            return Ok(());
        }
        // Nothing changed: an older save, or no such character.
        let exists: Option<i32> =
            sqlx::query_scalar("SELECT id FROM characters WHERE id = $1 AND account_id = $2")
                .bind(id)
                .bind(owner)
                .fetch_optional(&mut *tx)
                .await
                .map_err(failed)?;
        tx.rollback().await.map_err(failed)?;
        match exists {
            Some(_) => Ok(()),
            None => Err(StoreError(format!(
                "no character {} of account {}",
                character.id, account.0
            ))),
        }
    }
}

/// Replaces character `id`'s items, quickslots and skills with
/// `character`'s, inside the caller's transaction.
async fn write_belongings(
    tx: &mut PgConnection,
    id: i32,
    character: &Character,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM items WHERE character_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM quickslots WHERE character_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM skills WHERE character_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if !character.skills.is_empty() {
        let skills: Vec<i16> = character
            .skills
            .iter()
            .map(|s| i16::from(s.skill))
            .collect();
        let levels: Vec<i16> = character
            .skills
            .iter()
            .map(|s| i16::from(s.level))
            .collect();
        let next_reads = character
            .skills
            .iter()
            .map(|s| convert(s.next_read, "next read"))
            .collect::<Result<Vec<i64>, _>>()?;
        sqlx::query(
            "INSERT INTO skills (character_id, skill, level, next_read)
             SELECT $1, * FROM UNNEST($2::smallint[], $3::smallint[], $4::bigint[])",
        )
        .bind(id)
        .bind(skills)
        .bind(levels)
        .bind(next_reads)
        .execute(&mut *tx)
        .await?;
    }
    if !character.items.is_empty() {
        let slots: Vec<i32> = character.items.iter().map(|i| i32::from(i.slot)).collect();
        let vnums: Vec<i64> = character.items.iter().map(|i| i64::from(i.vnum)).collect();
        let counts: Vec<i16> = character.items.iter().map(|i| i16::from(i.count)).collect();
        let socket = |n: usize| -> Vec<i64> {
            character
                .items
                .iter()
                .map(|i| i64::from(i.sockets[n]))
                .collect()
        };
        sqlx::query(
            "INSERT INTO items (character_id, slot, vnum, count, socket0, socket1, socket2)
             SELECT $1, * FROM UNNEST($2::integer[], $3::bigint[], $4::smallint[],
                                      $5::bigint[], $6::bigint[], $7::bigint[])",
        )
        .bind(id)
        .bind(slots)
        .bind(vnums)
        .bind(counts)
        .bind(socket(0))
        .bind(socket(1))
        .bind(socket(2))
        .execute(&mut *tx)
        .await?;
    }
    if !character.quickslots.is_empty() {
        let places: Vec<i16> = character
            .quickslots
            .iter()
            .map(|q| i16::from(q.index))
            .collect();
        let kinds: Vec<i16> = character
            .quickslots
            .iter()
            .map(|q| i16::from(q.kind))
            .collect();
        let positions: Vec<i16> = character
            .quickslots
            .iter()
            .map(|q| i16::from(q.position))
            .collect();
        sqlx::query(
            "INSERT INTO quickslots (character_id, place, kind, position)
             SELECT $1, * FROM UNNEST($2::smallint[], $3::smallint[], $4::smallint[])",
        )
        .bind(id)
        .bind(places)
        .bind(kinds)
        .bind(positions)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// A character's points in the columns' types.
struct Points {
    hp: i32,
    sp: i32,
    exp: i64,
    gold: i64,
    stat_points: i32,
    skill_points: i32,
    saves: i64,
}

impl Points {
    fn of(character: &Character) -> Result<Self, sqlx::Error> {
        Ok(Self {
            hp: convert(character.hp, "health")?,
            sp: convert(character.sp, "mana")?,
            exp: i64::from(character.exp),
            gold: i64::from(character.gold),
            stat_points: i32::from(character.stat_points),
            skill_points: i32::from(character.skill_points),
            saves: convert(character.saves, "save count")?,
        })
    }
}

/// The name of the constraint a statement broke, if it broke one.
fn constraint(error: &sqlx::Error) -> Option<&str> {
    match error {
        sqlx::Error::Database(db) if db.is_unique_violation() => db.constraint(),
        _ => None,
    }
}

fn failed(error: sqlx::Error) -> StoreError {
    StoreError(error.to_string())
}

/// A friendship's row: two different characters, the lower number first.
fn friendship([a, b]: [u32; 2]) -> Result<(i32, i32), StoreError> {
    if a == b {
        return Err(StoreError(format!(
            "character {a} cannot be its own friend"
        )));
    }
    let number = |c: u32| convert(c, "character id").map_err(failed);
    Ok((number(a.min(b))?, number(a.max(b))?))
}

fn account_id(account: AccountId) -> Result<i64, sqlx::Error> {
    convert(account.0, "account id")
}

/// A number from the database in the type the core uses, or a decoding error
/// if it does not fit (the schema's checks keep that from happening).
fn convert<T, U>(value: T, what: &str) -> Result<U, sqlx::Error>
where
    T: Copy + std::fmt::Display,
    U: TryFrom<T>,
{
    U::try_from(value)
        .map_err(|_| sqlx::Error::Decode(format!("{what} {value} out of range").into()))
}

fn character_of(row: &PgRow) -> Result<Character, sqlx::Error> {
    let small = |column: &str| -> Result<u8, sqlx::Error> {
        convert(row.try_get::<i16, _>(column)?, column)
    };
    Ok(Character {
        id: convert(row.try_get::<i32, _>("id")?, "character id")?,
        slot: small("slot")?,
        name: row.try_get("name")?,
        class: small("class")?,
        level: small("level")?,
        st: small("st")?,
        ht: small("ht")?,
        dx: small("dx")?,
        iq: small("iq")?,
        x: row.try_get("x")?,
        y: row.try_get("y")?,
        empire: small("empire")?,
        playtime: convert(row.try_get::<i64, _>("playtime")?, "playtime")?,
        hp: convert(row.try_get::<i32, _>("hp")?, "hp")?,
        sp: convert(row.try_get::<i32, _>("sp")?, "sp")?,
        exp: convert(row.try_get::<i64, _>("exp")?, "exp")?,
        gold: convert(row.try_get::<i64, _>("gold")?, "gold")?,
        stat_points: convert(row.try_get::<i32, _>("stat_points")?, "stat_points")?,
        skill_points: convert(row.try_get::<i32, _>("skill_points")?, "skill_points")?,
        saves: convert(row.try_get::<i64, _>("saves")?, "saves")?,
        items: Vec::new(),
        quickslots: Vec::new(),
        skill_group: small("skill_group")?,
        skills: Vec::new(),
        read_bonus: small("read_bonus")?,
        friends: Vec::new(),
        mount_level: small("mount_level")?,
        mount_stamina: convert(row.try_get::<i32, _>("mount_stamina")?, "mount_stamina")?,
        mount_skill_points: small("mount_skill_points")?,
        incognito: row.try_get("incognito")?,
    })
}

/// Parses `url`. Without a TLS mode in it, nor in the environment
/// (`mode_in_env`), the mode is verify-full.
fn options_from(url: &str, mode_in_env: bool) -> Result<PgConnectOptions, OpenError> {
    let options = PgConnectOptions::from_str(url).map_err(|_| OpenError::BadUrl)?;
    let parsed = url::Url::parse(url).map_err(|_| OpenError::BadUrl)?;
    let mode_in_url = parsed
        .query_pairs()
        .any(|(key, _)| SSL_MODE_KEYS.contains(&key.as_ref()));
    Ok(if mode_in_url || mode_in_env {
        options
    } else {
        options.ssl_mode(PgSslMode::VerifyFull)
    })
}

/// A connection that failed, TLS's failures apart: sqlx reports the
/// handshake's as I/O errors that carry rustls's.
fn connect_error(error: sqlx::Error) -> OpenError {
    let tls = match &error {
        sqlx::Error::Tls(_) => true,
        sqlx::Error::Io(io) => io
            .get_ref()
            .is_some_and(|inner| inner.is::<rustls::Error>()),
        _ => false,
    };
    if tls {
        OpenError::Tls(error)
    } else {
        OpenError::Connect(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(url: &str, mode_in_env: bool) -> PgSslMode {
        options_from(url, mode_in_env).unwrap().get_ssl_mode()
    }

    #[test]
    fn tls_is_checked_unless_the_url_or_the_environment_says() {
        let url = "postgres://openm2:secret@db.example:5432/openm2";
        assert!(matches!(mode(url, false), PgSslMode::VerifyFull));
        for (query, expected) in [
            ("?sslmode=disable", PgSslMode::Disable),
            ("?sslmode=require", PgSslMode::Require),
            ("?ssl-mode=verify-ca", PgSslMode::VerifyCa),
            (
                "?application_name=x&sslmode=verify-full&sslrootcert=/ca.pem",
                PgSslMode::VerifyFull,
            ),
        ] {
            let got = mode(&format!("{url}{query}"), false);
            assert_eq!(format!("{got:?}"), format!("{expected:?}"), "{query}");
        }
        // PGSSLMODE names one: sqlx's own reading stands.
        let sqlx = PgConnectOptions::from_str(url).unwrap().get_ssl_mode();
        assert_eq!(format!("{:?}", mode(url, true)), format!("{sqlx:?}"));
    }

    #[test]
    fn a_bad_url_is_not_repeated() {
        let error = options_from("postgres://openm2:secret@db.example:port/x", false).unwrap_err();
        assert!(matches!(error, OpenError::BadUrl));
        assert!(!error.to_string().contains("secret"));
    }
}
