//! Storage's rules (`docs/specs/server/storage.md`), the same for both
//! stores: in memory always, in Postgres when `OPENM2_TEST_DATABASE_URL` is
//! set. And a character found where it was left after a restart, with a real
//! client on a server backed by Postgres.

mod support;

use std::thread;

use m2_core::{
    CharacterChoice, CharacterClass, Command, Empire, Event, Millis, NewCharacter, Point, Session,
    Stage,
};
use m2_net::Controller;
use m2_server::store::{OpenError, PgStore, Store};
use m2_server_core::{
    Account, AccountId, Character, CreateRefusal, QuickSlot, StoreAnswer, StoreRequest,
    StoredFriend, StoredItem, StoredSkill,
};
use sqlx::Row as _;
use support::{As, Harness, Schema};

/// The issuer of the providers' players in [`check_rules`].
const ISSUER: &str = "https://id.example";

fn character(slot: u8, name: &str) -> Character {
    Character {
        id: 0,
        slot,
        name: name.to_owned(),
        class: 7,
        level: 1,
        st: 3,
        ht: 4,
        dx: 3,
        iq: 6,
        x: 963_400,
        y: 278_200,
        empire: 3,
        playtime: 0,
        hp: 1_000,
        sp: 500,
        exp: 0,
        gold: 0,
        stat_points: 0,
        skill_points: 0,
        saves: 0,
        items: vec![
            StoredItem {
                slot: 0,
                vnum: 27_001,
                count: 5,
                sockets: [0; 3],
            },
            StoredItem {
                slot: 94,
                vnum: 7_000_000,
                count: 1,
                // A fishing pole's points and bait.
                sockets: [12, 0, 27_801],
            },
        ],
        quickslots: Vec::new(),
        skill_group: 0,
        skills: Vec::new(),
        read_bonus: 0,
        friends: Vec::new(),
        mount_level: 0,
        mount_stamina: 0,
        mount_skill_points: 0,
        incognito: false,
    }
}

/// Every rule of storage, on one store.
fn check_rules(harness: &Harness, store: &Store) {
    let run = |request| harness.runtime.block_on(store.handle(request));

    // Accounts: one of their own for each provider's player at the first
    // sign-in, found again by issuer and subject, never by name.
    let identity = |issuer: &str, subject: &str| match run(StoreRequest::IdentityLogin {
        provider: "example".to_owned(),
        issuer: issuer.to_owned(),
        subject: subject.to_owned(),
    }) {
        StoreAnswer::LoginChecked(result) => result.unwrap(),
        other => panic!("{other:?}"),
    };
    let player = identity(ISSUER, "player-1");
    assert_eq!(player.name, "example:nixzoi2rkfeta");
    assert_eq!(player.empire, None);
    assert_eq!(identity(ISSUER, "player-1"), player);
    let other = identity(ISSUER, "player-2");
    assert_ne!(other.id, player.id);
    let elsewhere = identity("https://other.example", "player-1");
    assert_ne!(elsewhere.id, player.id);
    let account = |subject: &str| -> Account { identity(ISSUER, subject) };
    let alice = account("alice").id;
    let bob = account("bob").id;

    // Characters: unique names without case, four at most, slots unique.
    let create =
        |account: AccountId, character: Character| match run(StoreRequest::CreateCharacter {
            account,
            character,
        }) {
            StoreAnswer::CharacterCreated(result) => result,
            other => panic!("{other:?}"),
        };
    let mage = create(alice, character(2, "Mage")).unwrap();
    assert_ne!(mage.id, 0);
    assert_eq!(mage.items, character(2, "Mage").items);
    assert_eq!(
        create(bob, character(0, "MAGE")),
        Err(CreateRefusal::NameTaken)
    );
    assert_eq!(
        create(alice, character(2, "Other")),
        Err(CreateRefusal::NoRoom)
    );
    for (slot, name) in [(0, "First"), (1, "Second"), (3, "Third")] {
        create(alice, character(slot, name)).unwrap();
    }
    assert_eq!(
        create(alice, character(0, "Fifth")),
        Err(CreateRefusal::NoRoom)
    );
    // The account took its first character's empire.
    assert_eq!(account("alice").empire, Some(3));
    assert_eq!(account("bob").empire, None);

    // Loading: an account's own characters, by slot.
    let load = |account| match run(StoreRequest::LoadCharacters { account }) {
        StoreAnswer::Characters(result) => result.unwrap(),
        other => panic!("{other:?}"),
    };
    let names: Vec<(u8, String)> = load(alice).into_iter().map(|c| (c.slot, c.name)).collect();
    assert_eq!(
        names,
        [
            (0, "First".to_owned()),
            (1, "Second".to_owned()),
            (2, "Mage".to_owned()),
            (3, "Third".to_owned())
        ]
    );
    assert!(load(bob).is_empty());
    // A new character's items are stored with it.
    let loaded = load(alice).into_iter().find(|c| c.id == mage.id).unwrap();
    assert_eq!(loaded, mage);

    // Saving: everything but the name, class, slot and empire, the items,
    // quickslots and skills in place of the stored ones, only for the
    // owner, and only a save counted higher than the one stored.
    let moved = Character {
        x: 964_400,
        y: 278_300,
        playtime: 66,
        level: 12,
        st: 9,
        hp: 0,
        sp: 7,
        exp: 4_000_000_000,
        gold: 123,
        stat_points: 33,
        skill_points: 11,
        saves: 2,
        name: "Renamed".to_owned(),
        class: 1,
        slot: 0,
        empire: 1,
        items: vec![
            StoredItem {
                slot: 3,
                vnum: 27_001,
                count: 200,
                sockets: [0; 3],
            },
            StoredItem {
                slot: 90,
                vnum: 11_200,
                count: 1,
                sockets: [0; 3],
            },
            StoredItem {
                slot: 65_535,
                vnum: u32::MAX,
                count: 255,
                sockets: [u32::MAX, 1, u32::MAX],
            },
        ],
        quickslots: vec![
            QuickSlot {
                index: 0,
                kind: 1,
                position: 3,
            },
            QuickSlot {
                index: 31,
                kind: 2,
                position: 255,
            },
        ],
        skill_group: 2,
        skills: vec![
            StoredSkill {
                skill: 1,
                level: 40,
                next_read: 1_800_003_600,
            },
            StoredSkill {
                skill: 16,
                level: 20,
                next_read: 0,
            },
            // A wait past the protocol's 32 bits is kept.
            StoredSkill {
                skill: 254,
                level: 1,
                next_read: 5_000_000_000,
            },
        ],
        read_bonus: 25,
        // The widest a mount's numbers go.
        mount_level: 255,
        mount_stamina: 65_535,
        mount_skill_points: 255,
        incognito: true,
        ..mage.clone()
    };
    let save = |account, character| match run(StoreRequest::SaveCharacter { account, character }) {
        StoreAnswer::Saved(result) => result,
        other => panic!("{other:?}"),
    };
    save(alice, moved.clone()).unwrap();
    assert!(
        save(
            bob,
            Character {
                saves: 3,
                ..mage.clone()
            }
        )
        .is_err()
    );
    let stored = || load(alice).into_iter().find(|c| c.id == mage.id).unwrap();
    assert_eq!(
        stored(),
        Character {
            name: "Mage".to_owned(),
            class: 7,
            slot: 2,
            empire: 3,
            ..moved.clone()
        }
    );
    // An older save, or the same one again, changes nothing (its items
    // neither) and is no error.
    for saves in [1, 2] {
        save(
            alice,
            Character {
                saves,
                x: 1,
                items: Vec::new(),
                ..moved.clone()
            },
        )
        .unwrap();
        assert_eq!(stored().x, 964_400);
        assert_eq!(stored().items, moved.items);
        assert_eq!(stored().skills, moved.skills);
        assert_eq!(stored().mount_stamina, 65_535);
    }
    save(
        alice,
        Character {
            saves: 3,
            x: 1,
            items: Vec::new(),
            quickslots: Vec::new(),
            skill_group: 0,
            skills: Vec::new(),
            mount_level: 1,
            mount_stamina: 0,
            mount_skill_points: 0,
            ..moved.clone()
        },
    )
    .unwrap();
    assert_eq!((stored().x, stored().saves), (1, 3));
    let mount = |c: Character| (c.mount_level, c.mount_stamina, c.mount_skill_points);
    assert_eq!(mount(stored()), (1, 0, 0));
    assert!(stored().items.is_empty() && stored().quickslots.is_empty());
    assert!(stored().skills.is_empty() && stored().skill_group == 0);
    // Other characters keep their items.
    let first = load(alice).into_iter().find(|c| c.name == "First").unwrap();
    assert_eq!(first.items, character(0, "First").items);

    // Friends: one friendship both ways, loaded with each side by name in
    // lowercase; adding it again, or the other way round, changes nothing;
    // saves and new characters never write friends; either side ends it,
    // and ending one that is not there is no error; never with oneself,
    // nor made with a character that does not exist.
    let friendship = |what| match run(what) {
        StoreAnswer::FriendshipWritten(result) => result,
        other => panic!("{other:?}"),
    };
    let add = |characters| friendship(StoreRequest::AddFriendship { characters });
    let end = |characters| friendship(StoreRequest::RemoveFriendship { characters });
    let bobby = create(
        bob,
        Character {
            friends: vec![StoredFriend {
                id: mage.id,
                name: "Mage".to_owned(),
                account: alice,
            }],
            ..character(0, "bobby")
        },
    )
    .unwrap();
    assert!(bobby.friends.is_empty());
    let friends_of = |account, id| -> Vec<StoredFriend> {
        let loaded = load(account).into_iter().find(|c| c.id == id).unwrap();
        loaded.friends
    };
    assert!(friends_of(bob, bobby.id).is_empty());
    add([mage.id, bobby.id]).unwrap();
    add([bobby.id, mage.id]).unwrap();
    add([bobby.id, first.id]).unwrap();
    let as_friend = |id, name: &str, account| StoredFriend {
        id,
        name: name.to_owned(),
        account,
    };
    assert_eq!(
        friends_of(alice, mage.id),
        [as_friend(bobby.id, "bobby", bob)]
    );
    assert_eq!(
        friends_of(bob, bobby.id),
        [
            as_friend(first.id, "First", alice),
            as_friend(mage.id, "Mage", alice)
        ]
    );
    save(
        alice,
        Character {
            saves: 4,
            friends: Vec::new(),
            ..stored()
        },
    )
    .unwrap();
    assert_eq!(friends_of(alice, mage.id).len(), 1);
    end([bobby.id, mage.id]).unwrap();
    assert!(friends_of(alice, mage.id).is_empty());
    assert_eq!(
        friends_of(bob, bobby.id),
        [as_friend(first.id, "First", alice)]
    );
    end([mage.id, bobby.id]).unwrap();
    end([mage.id, 999_999]).unwrap();
    assert!(add([mage.id, mage.id]).is_err());
    assert!(end([mage.id, mage.id]).is_err());
    assert!(add([mage.id, 999_999]).is_err());
    assert!(friends_of(alice, mage.id).is_empty());
}

#[test]
fn storage_in_memory_follows_the_rules() {
    let harness = Harness::new();
    let store = harness.memory();
    check_rules(&harness, &store);
}

#[test]
fn storage_in_postgres_follows_the_rules() {
    let harness = Harness::new();
    let Some(schema) = Schema::create(&harness) else {
        return;
    };
    let store = harness.postgres(&schema);
    check_rules(&harness, &store);
    harness.runtime.block_on(store.close());
    schema.drop(&harness);
}

#[test]
fn migrations_run_once_and_refuse_a_newer_database() {
    let harness = Harness::new();
    let Some(schema) = Schema::create(&harness) else {
        return;
    };
    harness.runtime.block_on(async {
        let store = PgStore::connect_with(schema.options()).await.unwrap();
        assert_eq!(
            store.migrate().await.unwrap(),
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
        );
        assert!(store.migrate().await.unwrap().is_empty());
        // A server that knows fewer migrations than the database has.
        let mut conn =
            <sqlx::postgres::PgConnection as sqlx::Connection>::connect_with(&schema.options())
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO openm2_migrations (version, name) VALUES (999, 'from the future')",
        )
        .execute(&mut conn)
        .await
        .unwrap();
        assert!(matches!(store.migrate().await, Err(OpenError::TooNew(999))));
        store.close().await;
    });
    schema.drop(&harness);
}

/// One server at a time runs a database's world: a second waits for the
/// first to let go, and a lock whose connection closes is let go.
#[test]
fn one_server_at_a_time_holds_the_world() {
    let harness = Harness::new();
    let Some(schema) = Schema::create(&harness) else {
        return;
    };
    harness.runtime.block_on(async {
        let first = PgStore::connect_with(schema.options()).await.unwrap();
        let second = PgStore::connect_with(schema.options()).await.unwrap();
        let world = first.try_lock_world().await.unwrap().expect("a free world");
        assert!(second.try_lock_world().await.unwrap().is_none());

        let waiter = second.clone();
        let waiting = tokio::spawn(async move { waiter.lock_world().await });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(!waiting.is_finished(), "the second server waits");
        world.release().await;
        let world = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
            .await
            .expect("the second server gets the world")
            .unwrap()
            .unwrap();
        assert!(first.try_lock_world().await.unwrap().is_none());

        drop(world);
        let mut taken = None;
        for _ in 0..50 {
            taken = first.try_lock_world().await.unwrap();
            if taken.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        taken.expect("a dropped lock is let go").release().await;
        first.close().await;
        second.close().await;
    });
    schema.drop(&harness);
}

/// The migrations before players signed in only with providers.
const BEFORE_PROVIDERS_ONLY: [(i32, &str); 5] = [
    (
        1,
        include_str!("../src/store/migrations/0001_accounts_and_characters.sql"),
    ),
    (
        2,
        include_str!("../src/store/migrations/0002_points_and_saves.sql"),
    ),
    (
        3,
        include_str!("../src/store/migrations/0003_items_and_quickslots.sql"),
    ),
    (4, include_str!("../src/store/migrations/0004_skills.sql")),
    (
        5,
        include_str!("../src/store/migrations/0005_identities.sql"),
    ),
];

/// A friendship goes with either character: an account deleted (as
/// migration 6 deleted some) takes its characters' friendships with it.
#[test]
fn deleting_an_account_ends_its_characters_friendships() {
    let harness = Harness::new();
    let Some(schema) = Schema::create(&harness) else {
        return;
    };
    let store = harness.postgres(&schema);
    let run = |request| harness.runtime.block_on(store.handle(request));
    let account = |subject: &str| match run(StoreRequest::IdentityLogin {
        provider: "example".to_owned(),
        issuer: ISSUER.to_owned(),
        subject: subject.to_owned(),
    }) {
        StoreAnswer::LoginChecked(result) => result.unwrap().id,
        other => panic!("{other:?}"),
    };
    let create = |account, name| match run(StoreRequest::CreateCharacter {
        account,
        character: character(0, name),
    }) {
        StoreAnswer::CharacterCreated(result) => result.unwrap(),
        other => panic!("{other:?}"),
    };
    let (alice, bob) = (account("alice"), account("bob"));
    let (ann, ben) = (create(alice, "Ann"), create(bob, "Ben"));
    let added = run(StoreRequest::AddFriendship {
        characters: [ann.id, ben.id],
    });
    assert_eq!(added, StoreAnswer::FriendshipWritten(Ok(())));
    harness.runtime.block_on(async {
        let mut conn =
            <sqlx::postgres::PgConnection as sqlx::Connection>::connect_with(&schema.options())
                .await
                .unwrap();
        sqlx::query("DELETE FROM accounts WHERE id = $1")
            .bind(i64::try_from(bob.0).unwrap())
            .execute(&mut conn)
            .await
            .unwrap();
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM friendships")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(left, 0);
    });
    match run(StoreRequest::LoadCharacters { account: alice }) {
        StoreAnswer::Characters(Ok(characters)) => {
            assert!(characters.iter().all(|c| c.friends.is_empty()));
        }
        other => panic!("{other:?}"),
    }
    harness.runtime.block_on(store.close());
    schema.drop(&harness);
}

#[test]
fn migration_6_deletes_the_password_accounts_and_their_characters() {
    let harness = Harness::new();
    let Some(schema) = Schema::create(&harness) else {
        return;
    };
    harness.runtime.block_on(async {
        // A database as a server before migration 6 left it: a password
        // account and a provider's, each with a character and an item.
        let mut conn =
            <sqlx::postgres::PgConnection as sqlx::Connection>::connect_with(&schema.options())
                .await
                .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE openm2_migrations (
                version integer PRIMARY KEY,
                name text NOT NULL,
                applied_at timestamptz NOT NULL DEFAULT now()
            )",
        )
        .execute(&mut conn)
        .await
        .unwrap();
        for (version, sql) in BEFORE_PROVIDERS_ONLY {
            sqlx::raw_sql(sql).execute(&mut conn).await.unwrap();
            sqlx::query("INSERT INTO openm2_migrations (version, name) VALUES ($1, 'before')")
                .bind(version)
                .execute(&mut conn)
                .await
                .unwrap();
        }
        let insert = |name: &'static str, hash: Option<&'static str>| {
            sqlx::query_scalar::<_, i64>(
                "INSERT INTO accounts (name, password_hash) VALUES ($1, $2) RETURNING id",
            )
            .bind(name)
            .bind(hash)
        };
        let local = insert(
            "alice",
            Some("$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA"),
        )
        .fetch_one(&mut conn)
        .await
        .unwrap();
        let provider = insert("example:nixzoi2rkfeta", None)
            .fetch_one(&mut conn)
            .await
            .unwrap();
        sqlx::query("INSERT INTO identities (issuer, subject, account_id) VALUES ($1, $2, $3)")
            .bind(ISSUER)
            .bind("player-1")
            .bind(provider)
            .execute(&mut conn)
            .await
            .unwrap();
        for (account, name) in [(local, "Alice"), (provider, "Hero")] {
            let character: i32 = sqlx::query_scalar(
                "INSERT INTO characters
                 (account_id, slot, name, class, level, st, ht, dx, iq, x, y, empire)
                 VALUES ($1, 0, $2, 0, 1, 6, 4, 3, 3, 0, 0, 1) RETURNING id",
            )
            .bind(account)
            .bind(name)
            .fetch_one(&mut conn)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO items (character_id, slot, vnum, count) VALUES ($1, 0, 19, 1)",
            )
            .bind(character)
            .execute(&mut conn)
            .await
            .unwrap();
        }

        let store = PgStore::connect_with(schema.options()).await.unwrap();
        assert_eq!(store.migrate().await.unwrap(), [6, 7, 8, 9, 10, 11, 12]);
        let accounts: Vec<String> = sqlx::query_scalar("SELECT name FROM accounts")
            .fetch_all(&mut conn)
            .await
            .unwrap();
        assert_eq!(accounts, ["example:nixzoi2rkfeta"]);
        let characters: Vec<String> = sqlx::query_scalar("SELECT name FROM characters")
            .fetch_all(&mut conn)
            .await
            .unwrap();
        assert_eq!(characters, ["Hero"]);
        let items: i64 = sqlx::query_scalar("SELECT count(*) FROM items")
            .fetch_one(&mut conn)
            .await
            .unwrap();
        assert_eq!(items, 1);
        let columns = sqlx::query(
            "SELECT column_name FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name = 'accounts'",
        )
        .fetch_all(&mut conn)
        .await
        .unwrap();
        let columns: Vec<String> = columns
            .iter()
            .map(|row| row.get::<String, _>("column_name"))
            .collect();
        assert!(!columns.iter().any(|c| c == "password_hash"), "{columns:?}");
        store.close().await;
    });
    schema.drop(&harness);
}

/// Walks to a point once its character appears and logs out when it
/// arrives; with no point, logs out at once. Remembers where it entered, and
/// gives up after 20 s so a broken server fails the test instead of hanging
/// it.
struct Walker {
    to: Option<Point>,
    me: Option<u32>,
    entered: Option<(i32, i32)>,
    arrived: bool,
    started: Option<Millis>,
}

impl Controller for Walker {
    fn on_event(&mut self, _now: Millis, event: &Event) -> Vec<Command> {
        match event {
            Event::EnteredGame(me) => {
                self.me = me.as_ref().map(|me| me.vid);
                self.entered = me.as_ref().map(|me| (me.position.0, me.position.1));
                Vec::new()
            }
            // As the bot does: walk once our own character is in the world.
            Event::EntityAppeared { vid, .. } if Some(*vid) == self.me => match self.to {
                Some(to) => vec![Command::MoveTo { path: vec![to] }],
                None => vec![Command::Logout],
            },
            Event::Arrived(_) => {
                self.arrived = true;
                vec![Command::Logout]
            }
            _ => Vec::new(),
        }
    }

    fn on_tick(&mut self, now: Millis) -> Vec<Command> {
        let started = *self.started.get_or_insert(now);
        if now.since(started) > 20_000 {
            vec![Command::Logout]
        } else {
            Vec::new()
        }
    }
}

fn walk(harness: &Harness, choice: CharacterChoice, to: Option<Point>) -> Walker {
    let net = harness.net();
    thread::spawn(move || {
        let mut config = support::config();
        config.character = choice;
        let mut session = Session::new(config);
        let walker = Walker {
            to,
            me: None,
            entered: None,
            arrived: false,
            started: None,
        };
        let mut walker = As::new("alice", walker);
        m2_net::run(&mut session, &net, &mut walker, None).unwrap();
        assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
        walker.inner
    })
    .join()
    .unwrap()
}

#[test]
fn a_character_is_where_it_was_left_after_a_restart() {
    let mut harness = Harness::new();
    let Some(schema) = Schema::create(&harness) else {
        return;
    };
    let store = harness.postgres(&schema);
    harness.start(store);
    let first = walk(
        &harness,
        CharacterChoice::Create(
            NewCharacter::new("Alice", CharacterClass::ShamanFemale, Empire::Jinno).unwrap(),
        ),
        Some(Point {
            x: 963_900,
            y: 278_500,
        }),
    );
    assert_eq!(first.entered, Some((963_400, 278_200)));
    assert!(first.arrived, "the walk did not arrive");
    harness.stop();

    // A new server on the same database.
    let store = harness.postgres(&schema);
    harness.start(store.clone());
    let second = walk(&harness, CharacterChoice::Slot(0), None);
    assert_eq!(second.entered, Some((963_900, 278_500)));
    harness.stop();
    harness.runtime.block_on(store.close());
    schema.drop(&harness);
}
