//! What the core asks of storage and what comes back
//! (`docs/specs/server/storage.md`).
//!
//! The core never waits: it returns a [`StoreRequest`] in an effect, the edge
//! carries it out (a database, or memory in tests) and hands the
//! [`StoreAnswer`] back as an input with the same [`RequestId`].

/// Numbers one store request and its answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// An account's number in storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AccountId(pub u64);

/// An account that signed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Its number.
    pub id: AccountId,
    /// Its name as stored: the provider's id, `:` and 13 letters and digits
    /// (printable ASCII, at most 30 bytes).
    pub name: String,
    /// The empire of its characters, once it has one.
    pub empire: Option<u8>,
}

/// A character as stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    /// Its number in storage, never 0 (the character list uses 0 for an
    /// empty slot).
    pub id: u32,
    /// Its place in the account's list, 0 to 3.
    pub slot: u8,
    /// Its name: 1 to 24 ASCII letters and digits.
    pub name: String,
    /// Class and gender, 0 to 7.
    pub class: u8,
    /// Level.
    pub level: u8,
    /// Strength.
    pub st: u8,
    /// Vitality.
    pub ht: u8,
    /// Dexterity.
    pub dx: u8,
    /// Intelligence.
    pub iq: u8,
    /// Where it stands, east.
    pub x: i32,
    /// Where it stands, south.
    pub y: i32,
    /// Its empire, 1 to 3.
    pub empire: u8,
    /// Seconds played.
    pub playtime: u64,
    /// Health; 0 while dead. Above the maximum it enters with the
    /// maximum (storage gives characters from before health was kept the
    /// most it holds).
    pub hp: u32,
    /// Mana, likewise.
    pub sp: u32,
    /// Experience gathered towards the next level.
    pub exp: u32,
    /// Gold.
    pub gold: u32,
    /// Status points not spent yet.
    pub stat_points: u16,
    /// Skill points not spent yet.
    pub skill_points: u16,
    /// How many times it was saved. Storage keeps a save only when this is
    /// higher than what it holds, so an older save that arrives late never
    /// overwrites a newer one.
    pub saves: u64,
    /// Its items, by place.
    pub items: Vec<StoredItem>,
    /// Its quickslots that hold something.
    pub quickslots: Vec<QuickSlot>,
    /// Its skill group: 0 before one is chosen, then 1 or 2.
    pub skill_group: u8,
    /// The skills it has learned.
    pub skills: Vec<StoredSkill>,
    /// The percent added to its next skill book's or soul stone's chance,
    /// 0 to 100.
    pub read_bonus: u8,
    /// Its friends, by name in lowercase. Loaded with it; saves and new
    /// characters never write them: friendships are written on their own
    /// ([`StoreRequest::AddFriendship`], [`StoreRequest::RemoveFriendship`]).
    pub friends: Vec<StoredFriend>,
    /// Its mount's level: 0 for none.
    pub mount_level: u8,
    /// Its mount's stamina.
    pub mount_stamina: u16,
    /// Horseback skill points not spent yet.
    pub mount_skill_points: u8,
    /// A game master's character whose mark doesn't show (`/gm off`).
    pub incognito: bool,
}

/// A friend of a character, as loaded with it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoredFriend {
    /// The friend's number in storage.
    pub id: u32,
    /// Its name, as it was created.
    pub name: String,
    /// Its account: the core finds the friend's connection by it. It never
    /// goes on the wire.
    pub account: AccountId,
}

/// A skill a character has learned, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoredSkill {
    /// The skill's number in the content's skills; never 0.
    pub skill: u8,
    /// Its level, 1 to 40.
    pub level: u8,
    /// When it may train again with a book or a soul stone, in Unix
    /// seconds; 0 for any time.
    pub next_read: u64,
}

/// An item a character holds, as stored. Storage checks only that the
/// numbers fit; the core checks where items may be when it loads them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct StoredItem {
    /// Its place: 0 to 89 in the bag, 90 and up worn (90 + its position).
    pub slot: u16,
    /// Its number in the content's items; never 0.
    pub vnum: u32,
    /// How many; never 0.
    pub count: u8,
    /// Its socket numbers: a fishing pole's points and bait.
    pub sockets: [u32; 3],
}

/// A quickslot that holds something, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct QuickSlot {
    /// Which, 0 to 7.
    pub index: u8,
    /// What it holds: 1 an item of the bag, 2 a skill.
    pub kind: u8,
    /// The item's place in the bag, or the skill.
    pub position: u8,
}

/// What the core asks of storage.
#[derive(Debug)]
pub enum StoreRequest {
    /// The account a provider's player signed in to: found by the issuer
    /// and subject, or made for them at their first sign-in. Never found by
    /// name or e-mail.
    IdentityLogin {
        /// The provider's id on this server, for a new account's name.
        provider: String,
        /// The provider's issuer.
        issuer: String,
        /// The player's id at the issuer.
        subject: String,
    },
    /// The account's characters.
    LoadCharacters {
        /// Whose.
        account: AccountId,
    },
    /// Store a new character (its `id` is 0 and is given by storage) with
    /// its items. The name must be unique, compared without case, and the
    /// slot free. The account takes the character's empire if it has none
    /// yet.
    CreateCharacter {
        /// Whose.
        account: AccountId,
        /// The character, with `id` 0.
        character: Character,
    },
    /// Store a character as it is now: everything but its name, class,
    /// slot and empire, its items, quickslots and skills in place of the
    /// stored ones, all at once and only if its save count is higher than
    /// the one stored.
    SaveCharacter {
        /// Whose.
        account: AccountId,
        /// The character as it is now.
        character: Character,
    },
    /// Two characters become friends, both ways at once. The order does
    /// not matter, and a friendship that is stored already stays as it is.
    /// The same character twice, or one that does not exist, is an error.
    AddFriendship {
        /// The two characters' numbers.
        characters: [u32; 2],
    },
    /// Two characters stop being friends, both ways at once. The order
    /// does not matter; a friendship that is not stored is no error.
    RemoveFriendship {
        /// The two characters' numbers.
        characters: [u32; 2],
    },
}

/// Why a character was not created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateRefusal {
    /// Another character has that name.
    NameTaken,
    /// The slot is taken or the account has four characters.
    NoRoom,
    /// Storage could not answer.
    Unavailable,
}

/// Storage could not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError(pub String);

/// What storage answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreAnswer {
    /// The answer to [`StoreRequest::IdentityLogin`].
    LoginChecked(Result<Account, StoreError>),
    /// The answer to [`StoreRequest::LoadCharacters`].
    Characters(Result<Vec<Character>, StoreError>),
    /// The answer to [`StoreRequest::CreateCharacter`]: the character with
    /// its new `id`.
    CharacterCreated(Result<Character, CreateRefusal>),
    /// The answer to [`StoreRequest::SaveCharacter`]; a save older than the
    /// one stored is skipped and answers `Ok`.
    Saved(Result<(), StoreError>),
    /// The answer to [`StoreRequest::AddFriendship`] and
    /// [`StoreRequest::RemoveFriendship`].
    FriendshipWritten(Result<(), StoreError>),
}
