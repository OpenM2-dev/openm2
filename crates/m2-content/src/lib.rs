//! openm2's own game content format: what the server's world
//! holds, as TOML files in a folder. No I/O: the edges read the files into
//! [`Sources`] and write what [`Content::to_files`] gives.
//!
//! - `content.toml`: the format and the content's name.
//! - `maps.toml`, `monsters.toml`, `items.toml`, `races.toml`,
//!   `skills.toml`: the maps, the monsters and NPCs, the items, the player
//!   races' walking speeds and the skills.
//!   `openm2 content import` writes them from the operator's own client; an
//!   original game writes its own.
//! - `levels.toml`: the experience each player level takes, written by the
//!   operator (optional: the server has a table of its own).
//! - `drops.toml`, `start.toml`, `shops.toml`, `teachers.toml` and
//!   `training.toml`: what monsters drop, what new characters start with,
//!   what NPCs sell, which NPCs teach skill groups and how skills train
//!   past a master's, written by the operator (optional).
//! - `mounts.toml`: players' mounts, their grades, levels, food and the
//!   NPCs that train them, written by the operator (optional).
//! - `upgrades.toml`: the NPCs and scrolls that upgrade items and what each
//!   upgrade set takes, written by the operator (optional).
//! - `fishing.toml`: how casts go, the fishers, what bites on which maps,
//!   what opens, the fire and what it grills, written by the operator
//!   (optional).
//! - `packs.toml`: monsters that live together, a leader and the others,
//!   written by the operator (optional).
//! - `spawns/*.toml`: where monsters and packs live and NPCs stand, written
//!   by the operator; `openm2 content import` writes `spawns/imported.toml`
//!   from the spawn lists a client carries.
//!
//! Files are in metres, seconds and degrees. [`parse`] checks every rule
//! and names each problem with its file and line; the checked [`Content`]
//! also gives the server its spawns in world units ([`Content::groups`],
//! [`Content::npcs`]). The format is described in
//! `docs/specs/server/content.md`; this crate's `AGENTS.md` has the rules.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod error;
mod fishing;
mod formula;
mod load;
mod model;
mod sources;
mod validate;
mod walk;
mod write;

pub use error::{ContentError, Errors, MAX_ERRORS};
pub use fishing::{
    Catch, Fire, Fish, Fisher, Fishing, FishingParts, FishingRules, Gift, Grill, Open, Timing,
    Water,
};
pub use formula::{Formula, Function, MAX_FORMULA, Name, ONE, SKILL_POWER, Values, skill_power};
pub use load::{parse, parse_packs};
pub use model::{
    ATTACK_SPEED_APPLY, Area, BLOCK_APPLY, BOOST_APPLIES, Books, Content, DODGE_APPLY, Drop,
    GOLD_VNUM, GoldDrop, Good, Group, GroupSpawn, HIDING_SECONDS, Help, Item, ItemBonus, ItemBoost,
    ItemUse, Level, MAX_STACK, MAX_UPGRADE_LEVEL, MOVE_SPEED_APPLY, Map, Monster, MonsterKind,
    MountCost, MountFeed, MountGrade, MountLevel, MountParts, MountRules, Mounts, Npc, NpcSpawn,
    OnFail, Pack, PackMember, Parts, Race, Regen, Rod, STACKABLE, ShopGood, Skill, SkillJob,
    SkillKind, Smith, SpawnFile, Spawned, Stable, Start, StartItem, Stats, Stones, Teacher,
    Training, UpgradeCost, UpgradeParts, UpgradeScroll, UpgradeSet, Upgrades, WeaponDamage, shop,
    wear,
};
pub use sources::{
    IMPORTED_FILES, IMPORTED_SPAWNS, MAX_FILE_BYTES, MAX_SPAWN_FILES, MAX_TOTAL_BYTES,
    MAX_WALK_FILES, SPAWNS_DIR, Sources, TOP_FILES, WALK_DIR, check_size, known_file,
    valid_spawn_name, walk_file,
};
pub use validate::{
    MAX_BITE_SECONDS, MAX_BONUSES, MAX_CALLS, MAX_CATCHES, MAX_DROPS, MAX_FEEDS, MAX_FIRE_SECONDS,
    MAX_GIFTS, MAX_GOLD_DROP, MAX_GOODS, MAX_GROUP_COUNT, MAX_GROUPS, MAX_ITEM_SIZE, MAX_ITEMS,
    MAX_LEVELS, MAX_MAPS, MAX_MONSTER_VNUM, MAX_MONSTERS, MAX_MOUNT_COSTS, MAX_MOUNT_GRADES,
    MAX_MOUNT_LEVELS, MAX_MOUNT_SKILL_POINTS, MAX_NAME, MAX_NPCS, MAX_PACK_SIZE, MAX_PACKS,
    MAX_PRICE, MAX_RADIUS, MAX_REACH, MAX_REACH_METRES, MAX_REACT_SECONDS, MAX_RESPAWN,
    MAX_RUN_SPEED, MAX_SIZE, MAX_SKILL_FLAGS, MAX_SKILL_VNUM, MAX_SKILLS, MAX_SPAWNED,
    MAX_START_GOLD, MAX_START_ITEMS, MAX_TIMING_POINTS, MAX_TRAINING_ITEMS, MAX_TRAINING_WAIT,
    MAX_UPGRADE_COSTS, MAX_UPGRADE_SCROLLS, MAX_UPGRADE_SETS, MAX_WORD,
};
pub use walk::{
    MAX_PATH_SIDE, MAX_WALK_SIDE, PATH_CLEARANCE, SQUARE, WalkError, WalkMap, square_of,
};
pub use write::{Header, IMPORT_MARKER};

/// The format these files are in. Every file starts with `format = 1`.
pub const FORMAT: u32 = 1;
