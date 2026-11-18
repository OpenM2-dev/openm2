//! Accounts and characters in one file of a save's folder
//! (`docs/specs/server/storage.md`, "In a save folder"): the store in
//! memory's data and rules, written whole to `world.json` after every
//! change, so a save needs no database. For a server on the player's own
//! machine; one server holds a save at a time (`world.lock`).
//!
//! A write goes to `world.json.tmp`, is synced to the disk, then renamed
//! over `world.json`, so the file is always one whole save or the one
//! before. A write that fails changes nothing: the request answers an
//! error, and what is in memory stays what is on disk.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use m2_server_core::rules::{MAX_CHARACTERS, valid_character_name};
use m2_server_core::{
    Account, AccountId, Character, CreateRefusal, QuickSlot, StoreAnswer, StoreError, StoreRequest,
    StoredItem, StoredSkill,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::memory::MemoryData;

/// The save's file, in its folder.
pub const WORLD_FILE: &str = "world.json";
/// Where a write goes before it is renamed into place.
const TEMP_FILE: &str = "world.json.tmp";
/// What a server holds while it runs on the save.
pub const LOCK_FILE: &str = "world.lock";
/// The format this server reads and writes.
const FORMAT: u32 = 1;
/// The largest file read, in bytes.
const MAX_SIZE: u64 = 64 * 1024 * 1024;
/// The most quickslots' places (four pages of eight).
const QUICKSLOTS: u8 = 32;

/// Why a save's folder could not be opened.
#[derive(Debug, Error)]
pub enum SaveError {
    /// The folder or a file in it could not be read or made.
    #[error("the save {}: {source}", path.display())]
    Io {
        /// What.
        path: PathBuf,
        /// What happened.
        source: io::Error,
    },
    /// Another server runs on it.
    #[error("another openm2-server holds the save {}", path.display())]
    Held {
        /// Its folder.
        path: PathBuf,
    },
    /// Its file is larger than any save.
    #[error("the save {} is larger than 64 MiB", path.display())]
    TooLarge {
        /// The file.
        path: PathBuf,
    },
    /// Its file was written by a newer server.
    #[error("the save {} has format {format}, newer than this openm2-server's {FORMAT}", path.display())]
    Newer {
        /// The file.
        path: PathBuf,
        /// Its format.
        format: u32,
    },
    /// Its file is not a save, or breaks a rule.
    #[error("the save {} is damaged: {why}", path.display())]
    Damaged {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        why: String,
    },
}

/// Accounts and characters in a save's folder.
#[derive(Debug, Clone)]
pub struct FileStore {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    folder: PathBuf,
    /// What is on disk, and so what is answered from.
    data: tokio::sync::Mutex<MemoryData>,
    /// Held while the store lives: another server can't take the save.
    _lock: File,
}

impl FileStore {
    /// Opens the save in `folder`, making the folder if there is none:
    /// takes its lock, then reads `world.json` (an empty world without
    /// one).
    pub fn open(folder: &Path) -> Result<Self, SaveError> {
        let io = |path: &Path| {
            let path = path.to_owned();
            move |source| SaveError::Io { path, source }
        };
        fs::create_dir_all(folder).map_err(io(folder))?;
        let lock_path = folder.join(LOCK_FILE);
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(io(&lock_path))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(SaveError::Held {
                    path: folder.to_owned(),
                });
            }
            Err(fs::TryLockError::Error(source)) => {
                return Err(SaveError::Io {
                    path: lock_path,
                    source,
                });
            }
        }
        // A write that never finished: the file before it still stands.
        let temp = folder.join(TEMP_FILE);
        match fs::remove_file(&temp) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(SaveError::Io { path: temp, source }),
        }
        let data = read(&folder.join(WORLD_FILE))?;
        Ok(Self {
            inner: Arc::new(Inner {
                folder: folder.to_owned(),
                data: tokio::sync::Mutex::new(data),
                _lock: lock,
            }),
        })
    }

    /// The folder it keeps.
    pub fn folder(&self) -> &Path {
        &self.inner.folder
    }

    /// The characters stored, with their accounts' numbers (for tests).
    pub async fn characters(&self) -> Vec<(AccountId, Character)> {
        self.inner
            .data
            .lock()
            .await
            .characters
            .values()
            .map(|(account, character)| (AccountId(*account), character.clone()))
            .collect()
    }

    /// Carries out one request, in turn with the others: a change is
    /// written to disk before it is answered, and kept only when the write
    /// worked.
    pub(super) async fn handle(&self, request: StoreRequest) -> StoreAnswer {
        let mut data = self.inner.data.lock().await;
        let kind = Kind::of(&request);
        let mut next = data.clone();
        let answer = next.apply(request);
        if next == *data {
            return answer;
        }
        let failed = |error: &dyn std::fmt::Display| {
            tracing::error!(target: "m2_server", "the save {} could not be written: {error}", self.inner.folder.display());
            kind.failed(format!("the save could not be written: {error}"))
        };
        let bytes = match serde_json::to_vec_pretty(&WorldFile::of(&next)) {
            Ok(bytes) => bytes,
            Err(error) => return failed(&error),
        };
        let folder = self.inner.folder.clone();
        match tokio::task::spawn_blocking(move || write(&folder, &bytes)).await {
            Ok(Ok(())) => {
                *data = next;
                answer
            }
            Ok(Err(error)) => failed(&error),
            Err(error) => failed(&error),
        }
    }
}

/// What a request was, for the answer when its change could not be kept.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Login,
    Load,
    Create,
    Save,
    Friendship,
}

impl Kind {
    fn of(request: &StoreRequest) -> Self {
        match request {
            StoreRequest::IdentityLogin { .. } => Kind::Login,
            StoreRequest::LoadCharacters { .. } => Kind::Load,
            StoreRequest::CreateCharacter { .. } => Kind::Create,
            StoreRequest::SaveCharacter { .. } => Kind::Save,
            StoreRequest::AddFriendship { .. } | StoreRequest::RemoveFriendship { .. } => {
                Kind::Friendship
            }
        }
    }

    /// The answer when the change could not be written.
    fn failed(self, why: String) -> StoreAnswer {
        match self {
            Kind::Login => StoreAnswer::LoginChecked(Err(StoreError(why))),
            Kind::Load => StoreAnswer::Characters(Err(StoreError(why))),
            Kind::Create => StoreAnswer::CharacterCreated(Err(CreateRefusal::Unavailable)),
            Kind::Save => StoreAnswer::Saved(Err(StoreError(why))),
            Kind::Friendship => StoreAnswer::FriendshipWritten(Err(StoreError(why))),
        }
    }
}

/// Writes `bytes` as the save in `folder`: the temporary file, synced,
/// renamed over the save.
fn write(folder: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = folder.join(TEMP_FILE);
    let path = folder.join(WORLD_FILE);
    let mut file = File::create(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    rename(&temp, &path)?;
    #[cfg(unix)]
    File::open(folder)?.sync_all()?;
    Ok(())
}

/// Renames `from` over `to`. On Windows a program that has `to` open for a
/// moment (a virus scanner, an indexer) makes it fail: it is tried a few
/// times.
fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let mut tries = 0;
    loop {
        match fs::rename(from, to) {
            Err(error)
                if cfg!(windows)
                    && tries < 10
                    && error.kind() == io::ErrorKind::PermissionDenied =>
            {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

/// The save in `path`; an empty world when there is no file.
fn read(path: &Path) -> Result<MemoryData, SaveError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(MemoryData::default()),
        Err(source) => {
            return Err(SaveError::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    let mut bytes = Vec::new();
    file.take(MAX_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| SaveError::Io {
            path: path.to_owned(),
            source,
        })?;
    if bytes.len() as u64 > MAX_SIZE {
        return Err(SaveError::TooLarge {
            path: path.to_owned(),
        });
    }
    parse(&bytes).map_err(|error| match error {
        ParseError::Newer(format) => SaveError::Newer {
            path: path.to_owned(),
            format,
        },
        ParseError::Damaged(why) => SaveError::Damaged {
            path: path.to_owned(),
            why,
        },
    })
}

#[derive(Debug, PartialEq, Eq)]
enum ParseError {
    Newer(u32),
    Damaged(String),
}

/// Only the format, read first, so a newer file is told from a damaged one.
#[derive(Deserialize)]
struct Format {
    format: u32,
}

fn parse(bytes: &[u8]) -> Result<MemoryData, ParseError> {
    let damaged = |why: String| ParseError::Damaged(why);
    let Format { format } =
        serde_json::from_slice(bytes).map_err(|error| damaged(error.to_string()))?;
    if format > FORMAT {
        return Err(ParseError::Newer(format));
    }
    if format != FORMAT {
        return Err(damaged(format!("format {format}")));
    }
    let file: WorldFile =
        serde_json::from_slice(bytes).map_err(|error| damaged(error.to_string()))?;
    file.into_data().map_err(damaged)
}

/// The save as written: `docs/specs/server/storage.md`, "In a save folder".
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldFile {
    format: u32,
    next_account: u64,
    next_character: u32,
    accounts: Vec<AccountRow>,
    identities: Vec<IdentityRow>,
    characters: Vec<CharacterRow>,
    /// Each friendship once, the lower character number first.
    friendships: Vec<[u32; 2]>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountRow {
    id: u64,
    name: String,
    empire: Option<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityRow {
    issuer: String,
    subject: String,
    account: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterRow {
    account: u64,
    id: u32,
    slot: u8,
    name: String,
    class: u8,
    level: u8,
    st: u8,
    ht: u8,
    dx: u8,
    iq: u8,
    x: i32,
    y: i32,
    empire: u8,
    playtime: u64,
    hp: u32,
    sp: u32,
    exp: u32,
    gold: u32,
    stat_points: u16,
    skill_points: u16,
    saves: u64,
    items: Vec<ItemRow>,
    quickslots: Vec<QuickSlotRow>,
    skill_group: u8,
    skills: Vec<SkillRow>,
    read_bonus: u8,
    mount_level: u8,
    mount_stamina: u16,
    mount_skill_points: u8,
    incognito: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemRow {
    slot: u16,
    vnum: u32,
    count: u8,
    sockets: [u32; 3],
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuickSlotRow {
    index: u8,
    kind: u8,
    position: u8,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillRow {
    skill: u8,
    level: u8,
    next_read: u64,
}

impl WorldFile {
    fn of(data: &MemoryData) -> Self {
        Self {
            format: FORMAT,
            next_account: data.next_account,
            next_character: data.next_character,
            accounts: data
                .accounts
                .values()
                .map(|account| AccountRow {
                    id: account.id.0,
                    name: account.name.clone(),
                    empire: account.empire,
                })
                .collect(),
            identities: data
                .identities
                .iter()
                .map(|((issuer, subject), account)| IdentityRow {
                    issuer: issuer.clone(),
                    subject: subject.clone(),
                    account: *account,
                })
                .collect(),
            characters: data
                .characters
                .values()
                .map(|(account, c)| CharacterRow {
                    account: *account,
                    id: c.id,
                    slot: c.slot,
                    name: c.name.clone(),
                    class: c.class,
                    level: c.level,
                    st: c.st,
                    ht: c.ht,
                    dx: c.dx,
                    iq: c.iq,
                    x: c.x,
                    y: c.y,
                    empire: c.empire,
                    playtime: c.playtime,
                    hp: c.hp,
                    sp: c.sp,
                    exp: c.exp,
                    gold: c.gold,
                    stat_points: c.stat_points,
                    skill_points: c.skill_points,
                    saves: c.saves,
                    items: c
                        .items
                        .iter()
                        .map(|item| ItemRow {
                            slot: item.slot,
                            vnum: item.vnum,
                            count: item.count,
                            sockets: item.sockets,
                        })
                        .collect(),
                    quickslots: c
                        .quickslots
                        .iter()
                        .map(|q| QuickSlotRow {
                            index: q.index,
                            kind: q.kind,
                            position: q.position,
                        })
                        .collect(),
                    skill_group: c.skill_group,
                    skills: c
                        .skills
                        .iter()
                        .map(|s| SkillRow {
                            skill: s.skill,
                            level: s.level,
                            next_read: s.next_read,
                        })
                        .collect(),
                    read_bonus: c.read_bonus,
                    mount_level: c.mount_level,
                    mount_stamina: c.mount_stamina,
                    mount_skill_points: c.mount_skill_points,
                    incognito: c.incognito,
                })
                .collect(),
            friendships: data.friendships.iter().map(|&(a, b)| [a, b]).collect(),
        }
    }

    /// The data, once every rule a database's constraints would keep
    /// holds; else what breaks first.
    fn into_data(self) -> Result<MemoryData, String> {
        let mut data = MemoryData {
            next_account: self.next_account,
            next_character: self.next_character,
            ..MemoryData::default()
        };
        let mut names = BTreeSet::new();
        for row in self.accounts {
            let id = row.id;
            if id == 0 || id > self.next_account {
                return Err(format!(
                    "account {id} is not between 1 and next_account ({})",
                    self.next_account
                ));
            }
            if !(1..=30).contains(&row.name.len())
                || !row.name.bytes().all(|b| b.is_ascii_graphic())
            {
                return Err(format!(
                    "account {id}'s name is not 1 to 30 printable characters"
                ));
            }
            if !names.insert(row.name.to_ascii_lowercase()) {
                return Err(format!("two accounts are named {}", row.name));
            }
            if row.empire.is_some_and(|empire| !(1..=3).contains(&empire)) {
                return Err(format!("account {id}'s empire is not 1 to 3"));
            }
            let account = Account {
                id: AccountId(id),
                name: row.name,
                empire: row.empire,
            };
            if data.accounts.insert(id, account).is_some() {
                return Err(format!("two accounts are number {id}"));
            }
        }
        for row in self.identities {
            if !(1..=512).contains(&row.issuer.len()) || !(1..=255).contains(&row.subject.len()) {
                return Err(format!(
                    "an identity of account {} is not an issuer of 1 to 512 bytes and a subject of 1 to 255",
                    row.account
                ));
            }
            if !data.accounts.contains_key(&row.account) {
                return Err(format!("an identity names no account {}", row.account));
            }
            let key = (row.issuer, row.subject);
            if data.identities.insert(key, row.account).is_some() {
                return Err("two identities are the same".to_owned());
            }
        }
        let mut character_names = BTreeSet::new();
        let mut slots: BTreeMap<u64, BTreeSet<u8>> = BTreeMap::new();
        for row in self.characters {
            let id = row.id;
            let character = row.into_character(self.next_character)?;
            if !data.accounts.contains_key(&character.0) {
                return Err(format!("character {id} names no account {}", character.0));
            }
            if !character_names.insert(character.1.name.to_ascii_lowercase()) {
                return Err(format!("two characters are named {}", character.1.name));
            }
            let taken = slots.entry(character.0).or_default();
            if !taken.insert(character.1.slot) || taken.len() > MAX_CHARACTERS {
                return Err(format!(
                    "account {} has two characters in slot {}, or more than {MAX_CHARACTERS}",
                    character.0, character.1.slot
                ));
            }
            if data.characters.insert(id, character).is_some() {
                return Err(format!("two characters are number {id}"));
            }
        }
        for [a, b] in self.friendships {
            if a >= b {
                return Err(format!("the friendship {a}, {b} is not lower first"));
            }
            if !data.characters.contains_key(&a) || !data.characters.contains_key(&b) {
                return Err(format!("the friendship {a}, {b} names no such character"));
            }
            if !data.friendships.insert((a, b)) {
                return Err(format!("the friendship {a}, {b} is there twice"));
            }
        }
        Ok(data)
    }
}

impl CharacterRow {
    fn into_character(self, next: u32) -> Result<(u64, Character), String> {
        let id = self.id;
        let bad = |what: &str| Err(format!("character {id}'s {what}"));
        if id == 0 || id > next {
            return Err(format!(
                "character {id} is not between 1 and next_character ({next})"
            ));
        }
        if !valid_character_name(self.name.as_bytes()) {
            return bad("name is not 1 to 24 letters and digits");
        }
        if usize::from(self.slot) >= MAX_CHARACTERS {
            return bad("slot is not 0 to 3");
        }
        if self.class > 7 {
            return bad("class is not 0 to 7");
        }
        if self.level == 0 {
            return bad("level is 0");
        }
        if !(1..=3).contains(&self.empire) {
            return bad("empire is not 1 to 3");
        }
        if self.skill_group > 2 {
            return bad("skill group is not 0 to 2");
        }
        if self.read_bonus > 100 {
            return bad("read bonus is over 100");
        }
        let mut places = BTreeSet::new();
        for item in &self.items {
            if item.vnum == 0 || item.count == 0 || !places.insert(item.slot) {
                return bad("items are not each a number and a count in a place of their own");
            }
        }
        let mut indices = BTreeSet::new();
        for quickslot in &self.quickslots {
            if quickslot.index >= QUICKSLOTS || !indices.insert(quickslot.index) {
                return bad("quickslots are not each in a place of their own, 0 to 31");
            }
        }
        let mut learned = BTreeSet::new();
        for skill in &self.skills {
            if skill.skill == 0 || !(1..=40).contains(&skill.level) || !learned.insert(skill.skill)
            {
                return bad("skills are not each learned once at a level of 1 to 40");
            }
        }
        let character = Character {
            id,
            slot: self.slot,
            name: self.name,
            class: self.class,
            level: self.level,
            st: self.st,
            ht: self.ht,
            dx: self.dx,
            iq: self.iq,
            x: self.x,
            y: self.y,
            empire: self.empire,
            playtime: self.playtime,
            hp: self.hp,
            sp: self.sp,
            exp: self.exp,
            gold: self.gold,
            stat_points: self.stat_points,
            skill_points: self.skill_points,
            saves: self.saves,
            items: self
                .items
                .into_iter()
                .map(|item| StoredItem {
                    slot: item.slot,
                    vnum: item.vnum,
                    count: item.count,
                    sockets: item.sockets,
                })
                .collect(),
            quickslots: self
                .quickslots
                .into_iter()
                .map(|q| QuickSlot {
                    index: q.index,
                    kind: q.kind,
                    position: q.position,
                })
                .collect(),
            skill_group: self.skill_group,
            skills: self
                .skills
                .into_iter()
                .map(|s| StoredSkill {
                    skill: s.skill,
                    level: s.level,
                    next_read: s.next_read,
                })
                .collect(),
            read_bonus: self.read_bonus,
            friends: Vec::new(),
            mount_level: self.mount_level,
            mount_stamina: self.mount_stamina,
            mount_skill_points: self.mount_skill_points,
            incognito: self.incognito,
        };
        Ok((self.account, character))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(characters: &str) -> String {
        format!(
            r#"{{"format": 1, "next_account": 1, "next_character": 1,
            "accounts": [{{"id": 1, "name": "key:abcdefghijklm", "empire": 3}}],
            "identities": [{{"issuer": "openm2:key", "subject": "ed25519:00", "account": 1}}],
            "characters": [{characters}],
            "friendships": []}}"#
        )
    }

    const HERO: &str = r#"{"account": 1, "id": 1, "slot": 0, "name": "Hero", "class": 0,
        "level": 1, "st": 6, "ht": 4, "dx": 3, "iq": 3, "x": 963400, "y": 278200,
        "empire": 3, "playtime": 0, "hp": 100, "sp": 50, "exp": 0, "gold": 0,
        "stat_points": 0, "skill_points": 0, "saves": 0,
        "items": [{"slot": 0, "vnum": 27001, "count": 5, "sockets": [0, 0, 0]}],
        "quickslots": [{"index": 0, "kind": 1, "position": 0}],
        "skill_group": 0, "skills": [{"skill": 1, "level": 1, "next_read": 0}],
        "read_bonus": 0, "mount_level": 0, "mount_stamina": 0, "mount_skill_points": 0,
        "incognito": false}"#;

    #[test]
    fn a_save_is_read_and_written_back_the_same() {
        let data = parse(world(HERO).as_bytes()).unwrap();
        assert_eq!(data.accounts.len(), 1);
        assert_eq!(data.characters.len(), 1);
        let written = serde_json::to_vec(&WorldFile::of(&data)).unwrap();
        assert_eq!(parse(&written).unwrap(), data);
    }

    #[test]
    fn a_newer_format_is_told_apart_from_damage() {
        assert_eq!(
            parse(br#"{"format": 2, "whatever": true}"#),
            Err(ParseError::Newer(2))
        );
        assert!(matches!(parse(b"not json"), Err(ParseError::Damaged(_))));
        assert!(matches!(
            parse(br#"{"format": 0}"#),
            Err(ParseError::Damaged(_))
        ));
        let unknown = world(HERO).replacen("\"format\": 1,", "\"format\": 1, \"extra\": 1,", 1);
        assert!(matches!(
            parse(unknown.as_bytes()),
            Err(ParseError::Damaged(_))
        ));
    }

    #[test]
    fn what_a_database_would_refuse_is_damage() {
        for (what, broken) in [
            (
                "a slot past 3",
                HERO.replace("\"slot\": 0, \"name\"", "\"slot\": 4, \"name\""),
            ),
            ("an empty name", HERO.replace("\"Hero\"", "\"\"")),
            (
                "a class past 7",
                HERO.replace("\"class\": 0", "\"class\": 8"),
            ),
            (
                "empire 0",
                HERO.replace("\"empire\": 3, \"playtime\"", "\"empire\": 0, \"playtime\""),
            ),
            (
                "no such account",
                HERO.replace("\"account\": 1, \"id\"", "\"account\": 9, \"id\""),
            ),
            (
                "an item without a count",
                HERO.replace("\"count\": 5", "\"count\": 0"),
            ),
            (
                "a skill at level 41",
                HERO.replace(
                    "\"level\": 1, \"next_read\"",
                    "\"level\": 41, \"next_read\"",
                ),
            ),
            (
                "a quickslot past 31",
                HERO.replace("\"index\": 0", "\"index\": 32"),
            ),
            (
                "a number past next_character",
                HERO.replace("\"id\": 1, \"slot\"", "\"id\": 2, \"slot\""),
            ),
        ] {
            assert!(
                matches!(
                    parse(world(&broken).as_bytes()),
                    Err(ParseError::Damaged(_))
                ),
                "{what}"
            );
        }
        let twice = format!("{HERO}, {HERO}");
        assert!(
            matches!(parse(world(&twice).as_bytes()), Err(ParseError::Damaged(_))),
            "the same character twice"
        );
        let friends = world(HERO).replace("\"friendships\": []", "\"friendships\": [[1, 1]]");
        assert!(matches!(
            parse(friends.as_bytes()),
            Err(ParseError::Damaged(_))
        ));
    }
}
