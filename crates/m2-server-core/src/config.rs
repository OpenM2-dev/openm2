//! The server's rules that an operator may change, with openm2's defaults
//! (`docs/specs/server/session.md` says where each default comes from).

use std::collections::BTreeSet;

/// Where new characters of an empire start, in world units (centimetres).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Start {
    /// East.
    pub x: i32,
    /// South.
    pub y: i32,
}

/// A job's values for a new character: its stats and the health and mana it
/// starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClassBase {
    /// Strength.
    pub st: u8,
    /// Vitality.
    pub ht: u8,
    /// Dexterity.
    pub dx: u8,
    /// Intelligence.
    pub iq: u8,
    /// Health before vitality and level are added.
    pub start_hp: u32,
    /// Mana before intelligence and level are added.
    pub start_sp: u32,
}

/// A state an attack skill may put on a monster: how likely,
/// how long, how much.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateRule {
    /// The percent chance a hit gives it, at a skill power of 1 (a perfect
    /// master's); a weaker skill's chance is its power times this.
    pub chance: u32,
    /// Milliseconds it lasts; a hit that gives it again starts it again.
    pub lasts: u64,
    /// How much: percent of the monster's maximum health a second for
    /// poison and fire, percent less movement speed for slowness; nothing
    /// for a stun.
    pub amount: u32,
}

/// An OpenID Connect provider players may sign in with, as the edge found
/// it at start (`docs/specs/server/session.md`, "Signing in with a
/// provider").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderOffer {
    /// Its id on this server ([`crate::rules::valid_provider_id`]).
    pub id: String,
    /// Its name, for the client's button.
    pub name: String,
    /// Its authorization endpoint.
    pub authorize: String,
    /// The server's client id at the provider.
    pub client_id: String,
}

/// The rules of [`crate::Server`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Where each empire's new characters start: Shinsoo (1), Chunjo (2),
    /// Jinno (3).
    pub starts: [Start; 3],
    /// Each job's values for new characters: warrior, ninja, sura, shaman
    /// (the class number modulo 4).
    pub classes: [ClassBase; 4],
    /// How far players see each other, in world units.
    pub view_radius: i32,
    /// How much farther than the view radius something must go before a
    /// player stops seeing it, so a player standing or fighting at the
    /// edge does not see it come and go with every step.
    pub view_margin: i32,
    /// How far a character runs in a second at movement speed 100, in world
    /// units; walks sent to others take the time this gives.
    pub run_speed: u32,
    /// Every player's movement speed.
    pub move_speed: u8,
    /// Every player's attack speed.
    pub attack_speed: u8,
    /// Milliseconds between pings on a game connection.
    pub ping_every: u64,
    /// Milliseconds a connection has to say Hello.
    pub hello_timeout: u64,
    /// Milliseconds a connection may stay silent.
    pub idle_timeout: u64,
    /// Milliseconds a ticket stays valid.
    pub key_lifetime: u64,
    /// The providers players may sign in with; none by default.
    pub providers: Vec<ProviderOffer>,
    /// Milliseconds an auth connection that asked for the providers has to
    /// sign in; meanwhile it is pinged, as its player is in a browser.
    pub sign_in_time: u64,
    /// Failed logins allowed per account name and per address in
    /// [`Config::failure_window`]; past that, logins are refused unchecked.
    pub max_failures: usize,
    /// The window of [`Config::max_failures`], in milliseconds.
    pub failure_window: u64,
    /// Connections at once, from everywhere.
    pub max_connections: usize,
    /// Connections at once from one address.
    pub max_per_address: usize,
    /// Milliseconds between saves of the characters in game.
    pub save_every: u64,
    /// Milliseconds between a monster's strolls, least and most.
    pub wander_every: [u64; 2],
    /// How far a monster strolls, least and most, in world units.
    pub wander_step: [i32; 2],
    /// How far a player's hit reaches, in world units, between where the
    /// player and its target are when the hit arrives; farther hits are
    /// ignored.
    pub attack_reach: i32,
    /// The least time between two hits of one player on one monster, in
    /// milliseconds; quicker hits are ignored.
    pub hit_gap: u64,
    /// Milliseconds between a monster's bites at attack speed 100.
    pub bite_every: u64,
    /// Milliseconds between the steps of a monster's fight: chasing,
    /// biting, giving up.
    pub chase_every: u64,
    /// How far an aggressive monster sees players when the content gives
    /// it no sight, in world units.
    pub aggressive_sight: i32,
    /// A monster gives up a target this far from it, in world units.
    pub give_up_distance: i32,
    /// A monster not hit for this many milliseconds gives up a target out
    /// of its reach, or when it is [`Config::leash`] from where it was last
    /// hit.
    pub give_up_after: u64,
    /// See [`Config::give_up_after`], in world units.
    pub leash: i32,
    /// Milliseconds a dead monster lies before it is removed.
    pub corpse_time: u64,
    /// Milliseconds between two regenerations of players' health and mana.
    pub regen_every: u64,
    /// Health and mana after a restart, at most the maximum.
    pub restart_points: u32,
    /// The highest level players reach when the content has no
    /// `levels.toml`.
    pub max_level: u8,
    /// How far a player picks up an item from, in world units, between
    /// where the player is and the item lies when the request arrives.
    pub pick_up_reach: i32,
    /// Milliseconds a monster's drop belongs to its killer alone.
    pub owner_time: u64,
    /// Milliseconds an item lies on the ground before it vanishes.
    pub ground_time: u64,
    /// Items on the ground at most, in the whole world; past it nothing is
    /// dropped.
    pub max_ground_items: usize,
    /// Drops one player may make in [`Config::drop_window`]; more are
    /// ignored (each drop saves the player).
    pub drop_burst: usize,
    /// See [`Config::drop_burst`], in milliseconds.
    pub drop_window: u64,
    /// The most gold a character holds.
    pub max_gold: u32,
    /// How far a player talks to an NPC from, in world units.
    pub talk_reach: i32,
    /// How far a player may be from the NPC whose shop it has open, in
    /// world units; farther, the shop is forgotten.
    pub shop_reach: i32,
    /// The lowest level that may choose a skill group.
    pub skill_group_level: u8,
    /// A skill's power per level, in thousandths (25: 1 at level 40).
    pub skill_power: u32,
    /// Milliseconds an attack skill's hits count after its use.
    pub skill_window: u64,
    /// Hits one use of a skill lands on one monster at most.
    pub skill_hits: u8,
    /// The highest a status point raises strength, vitality, dexterity or
    /// intelligence to.
    pub max_stat: u8,
    /// Whispers one player may send in [`Config::whisper_window`], to
    /// anyone and whether or not they arrive; more are refused
    /// (`session.md`, "Whispers").
    pub whisper_burst: usize,
    /// See [`Config::whisper_burst`], in milliseconds.
    pub whisper_window: u64,
    /// The most friends a character has; at most 64, what Friends carries
    /// (`session.md`, "Friends").
    pub max_friends: usize,
    /// Friend requests one player may make in
    /// [`Config::friend_ask_window`]; more are refused.
    pub friend_ask_burst: usize,
    /// See [`Config::friend_ask_burst`], in milliseconds.
    pub friend_ask_window: u64,
    /// How long a friend request waits for its answer, in milliseconds.
    pub friend_ask_time: u64,
    /// The most friend requests waiting for one player's answer at once.
    pub max_friend_asks: usize,
    /// How far from its rider a called mount may stand to be got on, in
    /// world units (`world.md`, "Mounts").
    pub mount_reach: i32,
    /// How far a following mount may fall behind before it appears beside
    /// its rider again, in world units.
    pub mount_leash: i32,
    /// How near its rider a following mount stops, in world units.
    pub mount_follow_gap: i32,
    /// Milliseconds between the steps of a following mount.
    pub mount_follow_every: u64,
    /// Milliseconds a rider waits between getting on and getting off.
    pub ride_gap: u64,
    /// The percent of its most stamina a tired-out mount needs to be ridden
    /// again.
    pub mount_rested: u8,
    /// `attack_poison`'s state: 1 % of the maximum health a second.
    pub poison: StateRule,
    /// `attack_fire_cont`'s state: 2 % a second.
    pub burning: StateRule,
    /// `attack_slow`'s state: 30 % slower.
    pub slowness: StateRule,
    /// `attack_stun`'s state: no moves, no bites.
    pub stun: StateRule,
    /// Milliseconds between the health poison and fire take.
    pub state_tick: u64,
    /// How far a skill with `crush` pushes a monster, in world units.
    pub crush_reach: i32,
    /// Milliseconds the push takes.
    pub crush_time: u32,
    /// Milliseconds between the mana a skill that stays on costs.
    pub toggle_every: u64,
    /// Milliseconds between the hits of an attack skill that stays on
    /// (Flame Spirit).
    pub spirit_every: u64,
    /// The percent of a bite a mana shield takes from mana.
    pub manashield_share: u32,
    /// The accounts whose players may use the game masters' commands, by
    /// name in lowercase (`docs/specs/server/session.md`, "Game masters");
    /// nobody by default.
    pub game_masters: BTreeSet<String>,
}

impl Config {
    /// The values for new characters of `class` (its job: the class number
    /// modulo 4).
    pub fn class(&self, class: u8) -> ClassBase {
        let [warrior, ninja, sura, shaman] = self.classes;
        match class % 4 {
            0 => warrior,
            1 => ninja,
            2 => sura,
            _ => shaman,
        }
    }

    /// Where new characters of `empire` start; Shinsoo's start for a number
    /// that is no empire.
    pub fn start(&self, empire: u8) -> Start {
        let [shinsoo, chunjo, jinno] = self.starts;
        match empire {
            2 => chunjo,
            3 => jinno,
            _ => shinsoo,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            starts: [
                Start {
                    x: 475_000,
                    y: 966_100,
                },
                Start {
                    x: 60_000,
                    y: 156_000,
                },
                Start {
                    x: 963_400,
                    y: 278_200,
                },
            ],
            classes: [
                ClassBase {
                    st: 6,
                    ht: 4,
                    dx: 3,
                    iq: 3,
                    start_hp: 600,
                    start_sp: 200,
                },
                ClassBase {
                    st: 4,
                    ht: 3,
                    dx: 6,
                    iq: 3,
                    start_hp: 650,
                    start_sp: 200,
                },
                ClassBase {
                    st: 5,
                    ht: 3,
                    dx: 3,
                    iq: 5,
                    start_hp: 650,
                    start_sp: 200,
                },
                ClassBase {
                    st: 3,
                    ht: 4,
                    dx: 3,
                    iq: 6,
                    start_hp: 700,
                    start_sp: 200,
                },
            ],
            view_radius: 10_000,
            view_margin: 2_000,
            run_speed: 450,
            move_speed: 150,
            attack_speed: 100,
            ping_every: 5_000,
            hello_timeout: 10_000,
            idle_timeout: 60_000,
            key_lifetime: 30_000,
            providers: Vec::new(),
            sign_in_time: 600_000,
            max_failures: 5,
            failure_window: 60_000,
            max_connections: 1_000,
            max_per_address: 16,
            save_every: 300_000,
            wander_every: [10_000, 20_000],
            wander_step: [300, 700],
            attack_reach: 500,
            hit_gap: 150,
            bite_every: 2_000,
            chase_every: 300,
            aggressive_sight: 1_000,
            give_up_distance: 4_000,
            give_up_after: 15_000,
            leash: 5_000,
            corpse_time: 5_000,
            regen_every: 3_000,
            restart_points: 50,
            max_level: 99,
            pick_up_reach: 500,
            owner_time: 30_000,
            ground_time: 300_000,
            max_ground_items: 20_000,
            drop_burst: 10,
            drop_window: 10_000,
            max_gold: 2_000_000_000,
            talk_reach: 500,
            shop_reach: 1_200,
            skill_group_level: 5,
            skill_power: m2_content::SKILL_POWER,
            skill_window: 3_000,
            skill_hits: 5,
            max_stat: 90,
            whisper_burst: 10,
            whisper_window: 10_000,
            max_friends: 50,
            friend_ask_burst: 5,
            friend_ask_window: 60_000,
            friend_ask_time: 60_000,
            max_friend_asks: 5,
            mount_reach: 500,
            mount_leash: 3_000,
            mount_follow_gap: 300,
            mount_follow_every: 300,
            ride_gap: 1_000,
            mount_rested: 20,
            poison: StateRule {
                chance: 60,
                lasts: 10_000,
                amount: 1,
            },
            burning: StateRule {
                chance: 60,
                lasts: 6_000,
                amount: 2,
            },
            slowness: StateRule {
                chance: 60,
                lasts: 6_000,
                amount: 30,
            },
            stun: StateRule {
                chance: 40,
                lasts: 2_000,
                amount: 0,
            },
            state_tick: 1_000,
            crush_reach: 300,
            crush_time: 200,
            toggle_every: 5_000,
            spirit_every: 2_000,
            manashield_share: 50,
            game_masters: BTreeSet::new(),
        }
    }
}
