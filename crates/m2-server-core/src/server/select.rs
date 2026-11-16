//! The character list: the empire, creating and selecting characters
//! (`docs/specs/server/session.md`, "Characters").

use std::collections::BTreeMap;

use m2_protocol::profile::openm2::{
    self as msg, CharacterCreateFailed, CharacterCreated, CharacterList, CreateCharacter, Empire,
    MainCharacter, ServerMessage,
};

use super::items::{Bag, quickslots_of};
use super::points::{SKILL_POINTS_PER_LEVEL, STAT_POINTS_PER_LEVEL, maxima};
use super::{Event, Holder, Mount, Player, Server, Stage, phase};
use crate::store::{Character, CreateRefusal, StoreError, StoreRequest};
use crate::time::{ConnId, Millis};

/// Characters an account may have.
pub const MAX_CHARACTERS: usize = 4;
/// The longest character name the server takes.
const MAX_CHARACTER_NAME: usize = 24;
/// The empire of an account without one in the Empire message, and of a
/// game master's account: its next character picks one.
const NO_EMPIRE: u8 = 0;

/// CharacterCreateFailed's error for a name that is taken; the client shows
/// "That name is taken."
const NAME_TAKEN: u8 = 0;
/// CharacterCreateFailed's error for any other refusal.
const NOT_CREATED: u8 = 1;

/// True for a character name the server accepts: 1 to 24 ASCII letters and
/// digits.
pub fn valid_character_name(name: &[u8]) -> bool {
    (1..=MAX_CHARACTER_NAME).contains(&name.len()) && name.iter().all(u8::is_ascii_alphanumeric)
}

impl Server {
    /// The account's characters arrived: Empire, then the list. A game
    /// master's account says no empire, so its client lets each new
    /// character pick one.
    pub(super) fn characters_loaded(
        &mut self,
        now: Millis,
        conn: ConnId,
        result: Result<Vec<Character>, StoreError>,
    ) {
        let mut characters = match result {
            Ok(characters) => characters,
            Err(error) => {
                self.event(Event::StoreFailed {
                    what: "load characters",
                    error: error.0,
                });
                self.close(now, conn, "storage could not load the characters");
                return;
            }
        };
        characters.retain(|character| usize::from(character.slot) < MAX_CHARACTERS);
        characters.sort_by_key(|character| character.slot);
        let master = self.is_game_master(conn);
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        let empire = state
            .account
            .as_ref()
            .and_then(|account| account.empire)
            .or_else(|| characters.first().map(|character| character.empire))
            .filter(|_| !master)
            .unwrap_or(NO_EMPIRE);
        state.characters = characters;
        state.stage = Stage::Select;
        self.send(conn, ServerMessage::Empire(Empire { empire }));
        self.send_phase(conn, phase::SELECT);
        let characters = self
            .conns
            .get(&conn)
            .map(|state| state.characters.iter().map(|c| self.listed(c)).collect())
            .unwrap_or_default();
        self.send(
            conn,
            ServerMessage::CharacterList(CharacterList { characters }),
        );
    }

    /// A character as the character list shows it.
    fn listed(&self, character: &Character) -> msg::Character {
        msg::Character {
            slot: character.slot,
            id: character.id,
            name: character.name.clone(),
            class: character.class,
            level: character.level,
            playtime: u32::try_from(character.playtime / 60).unwrap_or(u32::MAX),
            st: character.st,
            ht: character.ht,
            dx: character.dx,
            iq: character.iq,
            body_part: self.stored_armour(&character.items),
            hair_part: 0,
            guild: String::new(),
        }
    }

    /// CreateCharacter: checked here, then stored (storage checks that the
    /// name is free). The first character of an account without an empire
    /// chooses it; afterwards the empire in the message is not read, but
    /// for a game master's account, whose every character picks its own
    /// (openm2).
    pub(super) fn create(&mut self, now: Millis, conn: ConnId, create: CreateCharacter) {
        let master = self.is_game_master(conn);
        let Some(state) = self.conns.get(&conn) else {
            return;
        };
        if state.stage != Stage::Select || state.waiting.is_some() {
            self.close(now, conn, "a new character out of turn");
            return;
        }
        let Some(account) = state.account.clone() else {
            return;
        };
        let name = create.name.as_bytes();
        let chosen = create
            .empire
            .filter(|empire| (master || state.characters.is_empty()) && (1..=3).contains(empire));
        let empire = if master {
            chosen.or(account.empire)
        } else {
            account.empire.or(chosen)
        };
        let free_slot = |slot: u8| state.characters.iter().all(|c| c.slot != slot);
        let slot = if usize::from(create.slot) < MAX_CHARACTERS && free_slot(create.slot) {
            Some(create.slot)
        } else {
            (0..MAX_CHARACTERS as u8).find(|slot| free_slot(*slot))
        };
        let (Some(empire), Some(slot), true, true) =
            (empire, slot, valid_character_name(name), create.class <= 7)
        else {
            self.send(
                conn,
                ServerMessage::CharacterCreateFailed(CharacterCreateFailed {
                    reason: NOT_CREATED,
                }),
            );
            return;
        };
        let base = self.config.class(create.class);
        let start = self.config.start(empire);
        let level = self.content.start().level.max(1);
        let skipped = u16::from(level - 1);
        let mut character = Character {
            id: 0,
            slot,
            name: String::from_utf8_lossy(name).into_owned(),
            class: create.class,
            level,
            st: base.st,
            ht: base.ht,
            dx: base.dx,
            iq: base.iq,
            x: start.x,
            y: start.y,
            empire,
            playtime: 0,
            hp: 0,
            sp: 0,
            exp: 0,
            gold: self.content.start().gold.min(self.config.max_gold),
            // A character starting past level 1 has the points of the
            // level-ups it skipped.
            stat_points: skipped * STAT_POINTS_PER_LEVEL,
            skill_points: skipped * SKILL_POINTS_PER_LEVEL,
            saves: 0,
            items: self.starting_items(create.class),
            quickslots: Vec::new(),
            skill_group: 0,
            skills: Vec::new(),
            read_bonus: 0,
            friends: Vec::new(),
            mount_level: 0,
            mount_stamina: 0,
            mount_skill_points: 0,
            incognito: false,
        };
        // New characters start with all their health and mana.
        (character.hp, character.sp) = maxima(&self.config, &character);
        self.request(
            conn,
            StoreRequest::CreateCharacter {
                account: account.id,
                character,
            },
        );
    }

    /// Storage's answer to a new character.
    pub(super) fn created(
        &mut self,
        _now: Millis,
        conn: ConnId,
        result: Result<Character, CreateRefusal>,
    ) {
        let character = match result {
            Ok(character) => character,
            Err(refusal) => {
                let reason = match refusal {
                    CreateRefusal::NameTaken => NAME_TAKEN,
                    CreateRefusal::NoRoom | CreateRefusal::Unavailable => NOT_CREATED,
                };
                self.send(
                    conn,
                    ServerMessage::CharacterCreateFailed(CharacterCreateFailed { reason }),
                );
                return;
            }
        };
        let listed = self.listed(&character);
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        if let Some(account) = state.account.as_mut() {
            account.empire.get_or_insert(character.empire);
        }
        let id = character.id;
        let place = character.slot;
        state.characters.retain(|c| c.slot != place);
        state.characters.push(character);
        state.characters.sort_by_key(|c| c.slot);
        self.event(Event::CharacterCreated {
            conn,
            character: id,
        });
        self.send(
            conn,
            ServerMessage::CharacterCreated(CharacterCreated { character: listed }),
        );
        // The client takes the first character's empire for the account's:
        // a game master's next one picks again.
        if self.is_game_master(conn) {
            self.send(conn, ServerMessage::Empire(Empire { empire: NO_EMPIRE }));
        }
    }

    /// SelectCharacter: the loading phase first (the client waits for it
    /// before MainCharacter), then the character, all its points and its
    /// quickslots (its items come once it enters). Health and mana above
    /// the maximum come down to it; stored items with no place in the bag
    /// are left out.
    pub(super) fn select(&mut self, now: Millis, conn: ConnId, slot: u8) {
        let Some(state) = self.conns.get(&conn) else {
            return;
        };
        if state.stage != Stage::Select || state.waiting.is_some() {
            self.close(now, conn, "a selection out of turn");
            return;
        }
        let Some(mut character) = state.characters.iter().find(|c| c.slot == slot).cloned() else {
            self.close(now, conn, "no character in that slot");
            return;
        };
        let vid = self.take_vid(Holder::Player(conn));
        let (max_hp, max_sp) = maxima(&self.config, &character);
        character.hp = character.hp.min(max_hp);
        character.sp = character.sp.min(max_sp);
        let (bag, lost) = Bag::from_stored(&self.content, &character.items);
        let quickslots = quickslots_of(&character.quickslots);
        character.items.clear();
        character.quickslots.clear();
        // Levels from 1 to a perfect master's; the rest is not a skill.
        character
            .skills
            .retain(|learned| learned.skill != 0 && (1..=40).contains(&learned.level));
        if character.skill_group > 2 {
            character.skill_group = 0;
        }
        super::mounts::clamp_mount(&self.content, &mut character);
        for item in lost {
            self.event(Event::ItemLeftOut {
                character: character.id,
                vnum: item.vnum,
                count: item.count,
            });
        }
        let main = MainCharacter {
            vid,
            class: character.class,
            name: character.name.clone(),
            x: character.x,
            y: character.y,
            z: 0,
            empire: character.empire,
            skill_group: character.skill_group,
            run_speed: self.run_speed_of(character.class),
        };
        if let Some(state) = self.conns.get_mut(&conn) {
            state.stage = Stage::Loading;
            state.player = Some(Player {
                vid,
                seen_from: (character.x, character.y),
                character,
                walk: None,
                angle: 0,
                entered: now,
                moved_at: now,
                seen: Default::default(),
                told: BTreeMap::new(),
                max_hp,
                max_sp,
                target: None,
                hunted_by: Default::default(),
                next_regen: now,
                bag,
                quickslots,
                drops: Vec::new(),
                shop: None,
                cooldowns: BTreeMap::new(),
                cast: None,
                buffs: Vec::new(),
                mount: Mount::default(),
                toggles: BTreeMap::new(),
                skills_shown: Vec::new(),
                line: None,
                boosts_told: false,
                speed: None,
            });
        }
        self.send_phase(conn, phase::LOADING);
        self.send(conn, ServerMessage::MainCharacter(main));
        self.send_all_points(conn);
        self.send_quickslots(conn);
    }
}
