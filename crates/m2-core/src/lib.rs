//! Deterministic client state machine: messages and commands in, effects out.
//!
//! Core never touches sockets, threads or the clock. The caller passes the time
//! in (`now: Millis`) and carries out the returned effects (send a message, open
//! or close a connection, show an event). The same inputs always give the same
//! outputs, which is what makes the scenario tests reliable. This crate's
//! `AGENTS.md` has the rules.
//!
//! - [`session`]: the login → character select (or create) → game flow
//!   ([`Session::step`]).
//! - [`world`]: what the session has learned (characters, our character,
//!   chat, the entities around us and how they move, points, items,
//!   quickslots, the items on the ground, the open shop, skills).

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod session;
pub mod time;
pub mod world;

pub use session::{
    BUILD, CharacterChoice, Command, ConfigError, Effect, Event, FriendChange, Input, LoginRefusal,
    NewCharacter, Phase, Refusal, ServerKind, Session, SessionConfig, SessionError, ShopRefusal,
    SignInCode, SignInProvider, Stage, valid_character_name, walk_duration, walk_time_percent,
};
pub use time::{ConnId, Millis};
pub use world::{
    Affect, BAG_SLOTS, CharacterClass, CharacterSummary, ChatKind, ChatLine, Conversation, Empire,
    Entity, EntityEffectKind, EntityInfo, EntityKind, EntityState, FRIEND_REQUESTS, FishingLine,
    FishingStep, FlightKind, Friend, FriendNotice, GOLD_POINT, GOLD_VNUM, GROUP_SKILLS, GroundItem,
    Item, ItemBoost, Leg, MAX_SKILL_LEVEL, Me, Mount, MountCost, MountOffer, MoveKind, Path, Point,
    QUICK_PAGE_SLOTS, QUICK_PAGES, QUICK_SLOTS, QuickSlot, QuickSlotKind, SHOP_PLACES, SHOP_WIDTH,
    SKILL_POINT, ServerClock, Shop, ShopGood, Skill, SkillChoice, Stable, Teacher, Track,
    UpgradeCost, UpgradeFailure, UpgradeOffer, WHISPER_CONVERSATIONS, WHISPER_LINES, WORN_FROM,
    WhisperKind, WhisperLine, WhisperRefusal, Window, World, degrees, group_skills, skill_motion,
    skill_of_motion,
};
