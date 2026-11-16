//! The session: from connecting to the auth server to chatting in game.
//!
//! [`Session::step`] takes the time and one [`Input`] and returns the
//! [`Effect`]s to carry out. The flow it implements is described in
//! `docs/specs/protocol/openm2/` (session, chat, entities, movement,
//! inventory, combat, ground-items, shops, skills).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use m2_protocol::profile::openm2::{self as msg, ClientMessage, PROTOCOL, ServerMessage};
use secrecy::{ExposeSecret, SecretSlice, SecretString};
use thiserror::Error;

use crate::time::{ConnId, Millis};
use crate::world::{
    Affect, BAG_SLOTS, CharacterClass, CharacterSummary, ChatKind, ChatLine, Empire, Entity,
    EntityEffectKind, EntityInfo, EntityKind, FRIEND_REQUESTS, FishingLine, FishingStep,
    FlightKind, Friend, FriendNotice, GOLD_POINT, GroundItem, Item, ItemBoost, Me, Mount,
    MountCost, MountOffer, MoveKind, Path, Point, QUICK_SLOTS, QuickSlot, QuickSlotKind,
    SHOP_PLACES, ServerClock, Shop, ShopGood, Skill, SkillChoice, Stable, Teacher, Track,
    UpgradeCost, UpgradeFailure, UpgradeOffer, WhisperKind, WhisperLine, WhisperRefusal, Window,
    World, skill_motion,
};

/// A connection that sends nothing for this long is given up.
pub const SILENCE_LIMIT_MS: u64 = 30_000;

/// The build [`SessionConfig::build`] starts with: the client's name and
/// version, for the server's logs.
pub const BUILD: &str = concat!("openm2 ", env!("CARGO_PKG_VERSION"));

/// Longest provider id SignInOptions carries, in bytes.
const PROVIDER_MAX: usize = 16;

/// Longest character name we create, in bytes.
const CHARACTER_NAME_MAX: usize = 24;

/// Longest chat line the chat message carries, in bytes.
const CHAT_MAX: usize = 512;

/// Slots of the character list.
const SLOTS: u8 = 4;

/// Percent of the run animation's time a walk takes at `move_speed`
/// (`movement.md`, "Speed"): 100 at speed 100, more below it, less above.
pub fn walk_time_percent(move_speed: u8) -> u64 {
    let speed = u64::from(move_speed);
    match speed.cmp(&100) {
        Ordering::Less => 200 - speed,
        Ordering::Equal => 100,
        Ordering::Greater => 10_000 / speed,
    }
}

/// Milliseconds a walk of `distance` centimetres takes for a character whose
/// run animation covers `speed` centimetres a second, at `move_speed`
/// (`movement.md`, "Speed"). `speed` must not be 0.
pub fn walk_duration(distance: f64, speed: u32, move_speed: u8) -> u64 {
    // Saturates for absurd distances; whole milliseconds, like the server.
    let base = (distance / f64::from(speed.max(1)) * 1000.0) as u64;
    base.saturating_mul(walk_time_percent(move_speed)) / 100
}

/// Which server a connection goes to. The network layer maps this to an
/// address from its own configuration; core never handles addresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ServerKind {
    /// The auth server (signing in).
    Auth,
    /// The game server (character select and the game).
    Game,
}

impl fmt::Display for ServerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ServerKind::Auth => "auth",
            ServerKind::Game => "game",
        })
    }
}

/// A phase of the game connection, as set by the server's `Phase` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// 1: the character list.
    Select,
    /// 2: loading the map.
    Loading,
    /// 3: in the game.
    Game,
    /// Any other value.
    Other(u8),
}

impl Phase {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            1 => Phase::Select,
            2 => Phase::Loading,
            3 => Phase::Game,
            other => Phase::Other(other),
        }
    }
}

/// Where the session is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Not started.
    Idle,
    /// On the auth server: asking for the providers, or relaying the
    /// provider's code.
    Auth,
    /// On the auth server, waiting for the provider's code: the player is
    /// signing in in a browser ([`Effect::SignIn`]).
    SigningIn,
    /// On the game server: login with the ticket, character select.
    Joining,
    /// The character list is here; waiting for [`Command::Select`] or
    /// [`Command::Create`] ([`CharacterChoice::Ask`]).
    Selecting,
    /// Waiting for the game server to create our character.
    Creating,
    /// A character is selected and the server is sending its data.
    Loading,
    /// In the game.
    InGame,
    /// Left the game for the character list ([`Command::ChangeCharacter`]),
    /// which the server sends again.
    Leaving,
    /// Logged out on request.
    LoggedOut,
    /// Stopped after an error; see [`Session::error`].
    Failed,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Stage::Idle => "not started",
            Stage::Auth => "logging in",
            Stage::SigningIn => "signing in with the provider",
            Stage::Joining => "joining the game server",
            Stage::Selecting => "choosing a character",
            Stage::Creating => "creating a character",
            Stage::Loading => "loading",
            Stage::InGame => "in game",
            Stage::Leaving => "leaving the game",
            Stage::LoggedOut => "logged out",
            Stage::Failed => "failed",
        })
    }
}

/// A login setting the protocol cannot carry.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// The value is empty.
    #[error("the {field} must not be empty")]
    Empty {
        /// `character name`.
        field: &'static str,
    },
    /// The value does not fit its message field.
    #[error("the {field} is {len} bytes; at most {max} are allowed")]
    TooLong {
        /// `character name`.
        field: &'static str,
        /// Its length in bytes.
        len: usize,
        /// The most that fits.
        max: usize,
    },
    /// The value has characters other than ASCII letters and digits.
    #[error("the {field} must be ASCII letters and digits only")]
    NotAlphanumeric {
        /// `character name`.
        field: &'static str,
    },
    /// Not a provider id.
    #[error("a provider id is 1 to 16 lowercase letters, digits and hyphens")]
    Provider,
}

/// Whether `name` could be a character's: 1 to 24 ASCII letters and
/// digits, as openm2's server accepts (and so what a whisper or a friend
/// request may name).
pub fn valid_character_name(name: &str) -> bool {
    (1..=CHARACTER_NAME_MAX).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A character for the session to create (`session.md`, "Characters").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCharacter {
    name: String,
    /// Class and gender.
    pub class: CharacterClass,
    /// The empire to pick while the account may pick one: when the server
    /// said none (Empire 0), before the account's first character and on a
    /// game master's account. Otherwise the account's is kept.
    pub empire: Empire,
}

impl NewCharacter {
    /// Checks the name: 1 to 24 ASCII letters and digits, as openm2's server
    /// accepts; names stay safe in chat lines and commands.
    pub fn new(name: &str, class: CharacterClass, empire: Empire) -> Result<Self, ConfigError> {
        let field = "character name";
        if name.is_empty() {
            return Err(ConfigError::Empty { field });
        }
        if !name.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(ConfigError::NotAlphanumeric { field });
        }
        if name.len() > CHARACTER_NAME_MAX {
            return Err(ConfigError::TooLong {
                field,
                len: name.len(),
                max: CHARACTER_NAME_MAX,
            });
        }
        Ok(Self {
            name: name.to_owned(),
            class,
            empire,
        })
    }

    /// The name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Which character a session plays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CharacterChoice {
    /// The first character on the account.
    #[default]
    First,
    /// The character in this slot (0 to 3).
    Slot(u8),
    /// The character with this name, created first if the account has none by
    /// that name (`session.md`, "Characters").
    Create(NewCharacter),
    /// Whichever the user picks: the session waits on the character list
    /// ([`Stage::Selecting`]) for [`Command::Select`] or [`Command::Create`],
    /// as a character select screen does.
    Ask,
}

/// How a session logs in: only ever through a provider.
#[derive(Debug)]
enum Credentials {
    /// A provider of the server's, by id: the player signs in in a browser.
    Provider(String),
    /// None: the session asks for the providers, reports them and logs out.
    Options,
}

/// An OpenID Connect provider a server offers (`session.md`, "Signing in
/// with a provider").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInProvider {
    /// Its id on the server.
    pub id: String,
    /// Its name, for a button.
    pub name: String,
    /// Its authorization endpoint, as the server says: the caller checks
    /// it before opening it.
    pub authorize: String,
    /// The server's client id at the provider.
    pub client_id: String,
}

impl From<msg::Provider> for SignInProvider {
    fn from(provider: msg::Provider) -> Self {
        Self {
            id: provider.id,
            name: provider.name,
            authorize: provider.authorize,
            client_id: provider.client_id,
        }
    }
}

/// What the provider's redirect brought back, for [`Input::SignInCode`].
#[derive(Clone)]
pub struct SignInCode {
    /// The authorization code.
    pub code: SecretString,
    /// The PKCE code verifier the authorization request's challenge came
    /// from.
    pub verifier: SecretString,
    /// The redirect URI the authorization request carried.
    pub redirect: String,
}

impl fmt::Debug for SignInCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignInCode")
            .field("redirect", &self.redirect)
            .finish_non_exhaustive()
    }
}

impl PartialEq for SignInCode {
    fn eq(&self, other: &Self) -> bool {
        self.code.expose_secret() == other.code.expose_secret()
            && self.verifier.expose_secret() == other.verifier.expose_secret()
            && self.redirect == other.redirect
    }
}

/// Login settings for one session.
#[derive(Debug)]
pub struct SessionConfig {
    credentials: Credentials,
    /// The character to play.
    pub character: CharacterChoice,
    /// The client's name and version, sent in `Hello` for the server's logs;
    /// [`BUILD`] unless changed.
    pub build: String,
}

impl SessionConfig {
    /// Signs in with the server's provider `id`: the session asks the auth
    /// server for its providers, then hands [`Effect::SignIn`] to the
    /// caller, which opens the browser and comes back with
    /// [`Input::SignInCode`].
    pub fn provider(id: &str) -> Result<Self, ConfigError> {
        let valid = (1..=PROVIDER_MAX).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid {
            return Err(ConfigError::Provider);
        }
        Ok(Self::with(Credentials::Provider(id.to_owned())))
    }

    /// Only asks the auth server for its providers: the session reports
    /// [`Event::SignInOptions`] and logs out.
    pub fn sign_in_options() -> Self {
        Self::with(Credentials::Options)
    }

    fn with(credentials: Credentials) -> Self {
        Self {
            credentials,
            character: CharacterChoice::First,
            build: BUILD.to_owned(),
        }
    }

    /// The provider of a sign-in.
    pub fn provider_id(&self) -> Option<&str> {
        match &self.credentials {
            Credentials::Provider(id) => Some(id),
            Credentials::Options => None,
        }
    }
}

/// Why a server refused a connection (`session.md`, "Versions").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// 1: it speaks other versions of the protocol, `oldest` to `newest`.
    Version {
        /// The oldest minor version it speaks.
        oldest: u16,
        /// The newest.
        newest: u16,
    },
    /// 2: it is full.
    Full,
    /// 3: too many connections come from our address.
    TooManyConnections,
    /// Any other reason.
    Other(u8),
}

impl Refusal {
    /// Reads a `Refused` message.
    pub fn of(refused: &msg::Refused) -> Self {
        match refused.reason {
            1 => Refusal::Version {
                oldest: refused.oldest,
                newest: refused.newest,
            },
            2 => Refusal::Full,
            3 => Refusal::TooManyConnections,
            other => Refusal::Other(other),
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Version { oldest, newest } => write!(
                f,
                "it speaks protocol versions {oldest} to {newest}, this client {PROTOCOL}"
            ),
            Refusal::Full => f.write_str("it is full"),
            Refusal::TooManyConnections => f.write_str("too many connections from this address"),
            Refusal::Other(reason) => write!(f, "reason {reason}"),
        }
    }
}

/// Why a server refused a login (`session.md`, "Logins").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginRefusal {
    /// 2: the account is in the game.
    InGame,
    /// 3: the server is full.
    Full,
    /// 4: too many failed logins; try later.
    TooManyTries,
    /// 5: the ticket from the auth server is not valid.
    BadTicket,
    /// 6: the provider did not confirm the sign-in.
    ProviderRefused,
    /// 7: the server could not reach the provider.
    ProviderUnavailable,
    /// Any other value, the reserved 1 among them.
    Other(u8),
}

impl LoginRefusal {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            2 => LoginRefusal::InGame,
            3 => LoginRefusal::Full,
            4 => LoginRefusal::TooManyTries,
            5 => LoginRefusal::BadTicket,
            6 => LoginRefusal::ProviderRefused,
            7 => LoginRefusal::ProviderUnavailable,
            other => LoginRefusal::Other(other),
        }
    }
}

impl fmt::Display for LoginRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoginRefusal::InGame => f.write_str("the account is already in the game"),
            LoginRefusal::Full => f.write_str("the server is full"),
            LoginRefusal::TooManyTries => f.write_str("too many failed logins; try again later"),
            LoginRefusal::BadTicket => f.write_str("the login ticket is not valid; log in again"),
            LoginRefusal::ProviderRefused => {
                f.write_str("the provider did not confirm the sign-in")
            }
            LoginRefusal::ProviderUnavailable => {
                f.write_str("the server could not reach the provider; try again later")
            }
            LoginRefusal::Other(reason) => write!(f, "reason {reason}"),
        }
    }
}

/// Why a session stopped.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionError {
    /// The server will not talk to us.
    #[error("the {server} server refused the connection: {refusal}{}", if text.is_empty() {
        String::new()
    } else {
        format!(" ({text})")
    })]
    Refused {
        /// Which server.
        server: ServerKind,
        /// Why.
        refusal: Refusal,
        /// The operator's line for the player.
        text: String,
    },
    /// A server refused the login.
    #[error("the {server} server refused the login: {reason}")]
    LoginRejected {
        /// Which server.
        server: ServerKind,
        /// Why.
        reason: LoginRefusal,
    },
    /// The server does not offer the provider asked for.
    #[error("the server offers no provider {id:?}{}", if offered.is_empty() {
        String::new()
    } else {
        format!(" (it offers {})", offered.join(", "))
    })]
    NoProvider {
        /// The provider asked for.
        id: String,
        /// The ids the server offers.
        offered: Vec<String>,
    },
    /// Signing in at the provider failed on this side: the browser, the
    /// redirect, the provider's error.
    #[error("the sign-in failed: {reason}")]
    SignIn {
        /// What happened.
        reason: String,
    },
    /// The account has no character to play.
    #[error("{}", match slot {
        Some(slot) => format!("the account has no character in slot {slot}"),
        None => "the account has no characters; create one first".to_owned(),
    })]
    NoCharacter {
        /// The slot asked for, if any.
        slot: Option<u8>,
    },
    /// The character to create has no free slot.
    #[error("the account's four slots are full and none holds {name}")]
    SlotsFull {
        /// The character's name.
        name: String,
    },
    /// The game server refused to create the character.
    #[error("the game server refused to create the character: {}", match error {
        0 => "the name is already taken".to_owned(),
        other => format!("error {other}"),
    })]
    CreateRefused {
        /// Reason code from the server (`session.md`, "Characters").
        error: u8,
    },
    /// The connection closed unexpectedly.
    #[error("the {server} connection closed while {stage}: {reason}")]
    Disconnected {
        /// Which server.
        server: ServerKind,
        /// What we were doing.
        stage: Stage,
        /// What the network layer reported.
        reason: String,
    },
    /// The server sent nothing for [`SILENCE_LIMIT_MS`].
    #[error("the {server} server sent nothing for {} s while {stage}", SILENCE_LIMIT_MS / 1000)]
    Silent {
        /// Which server.
        server: ServerKind,
        /// What we were doing.
        stage: Stage,
    },
    /// A bug in the session itself.
    #[error("internal error: {what}")]
    Internal {
        /// What went wrong.
        what: &'static str,
    },
}

/// Something the session reports to its user (the bot, later the UI).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A server speaks our protocol.
    Welcomed {
        /// Which server.
        server: ServerKind,
        /// The minor version it speaks.
        protocol: u16,
        /// Its name and version.
        name: String,
    },
    /// The game server moved the connection to another phase.
    Phase {
        /// Which server.
        server: ServerKind,
        /// The new phase.
        phase: Phase,
    },
    /// The auth server sent its providers (asked for by a provider's
    /// sign-in, or by [`SessionConfig::sign_in_options`]).
    SignInOptions(Vec<SignInProvider>),
    /// The auth server accepted the provider's sign-in.
    LoggedIn,
    /// The game server sent the character list (non-empty slots only).
    CharacterList(Vec<CharacterSummary>),
    /// We asked the game server to create a character.
    Creating {
        /// Its name.
        name: String,
        /// Its class and gender.
        class: CharacterClass,
        /// The empire we picked, if the account had no characters yet.
        empire: Option<Empire>,
    },
    /// The game server created our character.
    Created(CharacterSummary),
    /// The game server refused the character asked for with
    /// [`Command::Create`]; the session is back at [`Stage::Selecting`].
    CreateRefused {
        /// Reason code from the server; 0: the name is taken
        /// (`session.md`, "Characters").
        error: u8,
    },
    /// We picked a character.
    Selected {
        /// Its slot.
        slot: u8,
        /// Its name.
        name: String,
    },
    /// We are in the game.
    EnteredGame(Option<Me>),
    /// We left the game for the character list ([`Command::ChangeCharacter`]):
    /// the world is forgotten, and the list comes next.
    LeftGame,
    /// An entity appeared near us (or we did, after a jump).
    EntityAppeared {
        /// Its id.
        vid: u32,
        /// What it is.
        kind: EntityKind,
        /// Its class or monster number.
        race: u16,
        /// Where.
        at: Point,
    },
    /// An entity disappeared.
    EntityLeft {
        /// Its id.
        vid: u32,
    },
    /// Our character finished a walk asked for with [`Command::MoveTo`]:
    /// it stands at the path's last point.
    Arrived(Point),
    /// The server refused our last move (through a wall, off the map, out
    /// of reach): our character stands at this point instead, and the walk
    /// is over without arriving.
    MoveRefused(Point),
    /// An entity took damage: one we hit, or our own character
    /// (`combat.md`).
    Damaged {
        /// The entity hit.
        vid: u32,
        /// Health taken.
        amount: i32,
        /// 1 normal, 2 poison, 4 dodged, 8 blocked, 16 piercing, 32
        /// critical; may be combined.
        flags: u8,
    },
    /// An entity died; our own character too.
    Died {
        /// Its id.
        vid: u32,
    },
    /// A monster shot at a target (`combat.md`, "Monsters that shoot"): its
    /// attack motion's flying effect flies there.
    Shot {
        /// The monster that shot.
        shooter: u32,
        /// What it shot at.
        target: u32,
        /// Where the target was.
        at: Point,
    },
    /// Something flies from one entity we see to another (`combat.md`,
    /// "Flights"): the experience a kill gave, from the
    /// monster killed.
    Flight {
        /// What flies.
        kind: FlightKind,
        /// The entity it leaves.
        from: u32,
        /// The entity it flies to.
        to: u32,
    },
    /// A player we see, or we, took a step of fishing; its
    /// line is in [`Entity::line`] from the cast to its end.
    Fishing {
        /// The fisher.
        vid: u32,
        /// The step.
        step: FishingStep,
        /// Where its float is.
        float: Point,
    },
    /// An entity we see, or we, show what happened to it (`entities.md`,
    /// "Effects"): a potion's health or mana, a level.
    EntityEffect {
        /// What happened.
        kind: EntityEffectKind,
        /// The entity.
        vid: u32,
    },
    /// An item lies on the ground near us: dropped, or come into view
    /// (`ground-items.md`).
    GroundItemAppeared {
        /// Its id.
        vid: u32,
        /// Its item number.
        vnum: u32,
        /// Where.
        at: Point,
    },
    /// An item on the ground is gone: picked up, or out of view.
    GroundItemLeft {
        /// Its id.
        vid: u32,
    },
    /// An NPC's shop opened (`shops.md`); [`World::shop`] holds what it
    /// sells.
    ShopOpened {
        /// The NPC.
        vid: u32,
    },
    /// The shop closed: we closed it, or its NPC left.
    ShopClosed,
    /// The server refused a purchase.
    ShopRefused(ShopRefusal),
    /// A teacher NPC shows what it teaches (`skills.md`); [`World::teacher`]
    /// holds it.
    TeacherOpened {
        /// The NPC.
        vid: u32,
    },
    /// The teacher closed: we closed it, its NPC left, or a shop opened.
    TeacherClosed,
    /// An item we used offers skills to train (`skills.md`, "Training");
    /// [`World::skill_choice`] holds them.
    SkillChoiceOpened {
        /// The item's slot in the bag.
        slot: u16,
    },
    /// The skill choice closed: we picked or closed it, or its item's slot
    /// changed.
    SkillChoiceClosed,
    /// A stable NPC shows what it trains (`mounts.md`); [`World::stable`]
    /// holds it.
    StableOpened {
        /// The NPC.
        vid: u32,
    },
    /// The stable closed: we closed it, its NPC left, or a shop or teacher
    /// opened.
    StableClosed,
    /// The server offered to upgrade an item of our bag (`upgrades.md`);
    /// [`World::upgrade`] holds the offer.
    UpgradeOffered {
        /// The item's slot in the bag.
        slot: u16,
    },
    /// The upgrade offer closed: we upgraded or closed it, the item's or
    /// the scroll's slot changed, the smith left, or a shop, teacher or
    /// stable opened.
    UpgradeClosed,
    /// A player got on or off its mount (`mounts.md`): [`Entity::mount`].
    MountChanged {
        /// The player.
        vid: u32,
        /// The monster its mount looks like; 0 on foot.
        look: u16,
    },
    /// Our mount changed: [`World::mount`].
    MountStateChanged,
    /// Our character's run speed changed (getting on or off a mount):
    /// [`World::run_speed`].
    RunSpeedChanged {
        /// Centimetres a second at movement speed 100.
        run_speed: u32,
    },
    /// What skills raise on our character changed (`skills.md`, "What
    /// skills raise"); [`World::affects`] holds it.
    AffectsChanged,
    /// Our character's skill group changed (`skills.md`).
    SkillGroupChanged {
        /// 1 or 2; 0 for none.
        group: u8,
    },
    /// One of our character's skills changed, or was there when we
    /// entered; [`World::skills`] holds it.
    SkillChanged {
        /// The skill's number.
        skill: u8,
        /// Its level now, 0 for not learned.
        level: u8,
        /// Its grade now: 0 normal, 1 master, 2 grand master, 3 perfect
        /// master.
        grade: u8,
    },
    /// A chat line arrived.
    Chat(ChatLine),
    /// A whisper line: a player's to us, or ours gone out (`chat.md`,
    /// "Whispers"); [`World::whispers`] holds it in its conversation.
    Whisper {
        /// The other player, as the server wrote its name.
        name: String,
        /// The line.
        line: WhisperLine,
    },
    /// Our whisper to `name` did not go out.
    WhisperRefused {
        /// The name we whispered to.
        name: String,
        /// Why.
        reason: WhisperRefusal,
    },
    /// Our friends arrived, after entering the game (`friends.md`);
    /// [`World::friends`] holds them.
    Friends,
    /// A friend became one, stopped being one, came or went.
    FriendChanged {
        /// Its name.
        name: String,
        /// What changed.
        change: FriendChange,
    },
    /// A player asks to be our friend: answer with
    /// [`Command::AnswerFriend`]. [`World::friend_requests`] holds it.
    FriendAsked {
        /// Its name.
        name: String,
    },
    /// What became of a friend request, ours or one to us.
    FriendNotice {
        /// The other player's name.
        name: String,
        /// What became of it.
        notice: FriendNotice,
    },
    /// We logged out on request.
    LoggedOut,
    /// The session stopped; see [`Session::error`].
    Failed(SessionError),
    /// An input that does not fit the current stage was ignored.
    Ignored {
        /// The message or command.
        what: &'static str,
        /// Why it was ignored.
        why: &'static str,
    },
}

/// What changed of a friend ([`Event::FriendChanged`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendChange {
    /// It became our friend.
    Added {
        /// Whether it is in the game.
        online: bool,
    },
    /// It came into the game.
    Online,
    /// It left the game.
    Offline,
    /// It is no longer our friend.
    Removed,
}

/// Why the server refused a purchase (`shops.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopRefusal {
    /// 1: we have less gold than it costs.
    NotEnoughGold,
    /// 2: no room in our bag.
    BagFull,
    /// Any other value.
    Other(u8),
}

impl ShopRefusal {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            1 => ShopRefusal::NotEnoughGold,
            2 => ShopRefusal::BagFull,
            other => ShopRefusal::Other(other),
        }
    }
}

/// What the caller must do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Open a connection. Report the result with [`Input::Connected`] or [`Input::Closed`].
    Connect {
        /// Id for the new connection.
        conn: ConnId,
        /// Which server to connect to.
        server: ServerKind,
    },
    /// Encode, frame and send a message.
    Send {
        /// The connection.
        conn: ConnId,
        /// The message.
        message: ClientMessage,
    },
    /// Close a connection. No [`Input::Closed`] is expected for it.
    Disconnect {
        /// The connection.
        conn: ConnId,
    },
    /// Sign in at the provider: open its authorization endpoint in a
    /// browser with a PKCE challenge, a state and this nonce, catch the
    /// redirect on the loopback address, and report the code with
    /// [`Input::SignInCode`] (or [`Input::SignInFailed`]).
    SignIn {
        /// The provider, as the server offers it.
        provider: SignInProvider,
        /// The nonce for the authorization request.
        nonce: String,
    },
    /// Report an event.
    Event(Event),
}

/// A command from the user or a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Send a chat line (in game only).
    Chat {
        /// Kind of line, usually [`ChatKind::Talk`].
        kind: ChatKind,
        /// The message, or a slash command; at most 512 bytes.
        text: String,
    },
    /// Walk our character along a path: a straight leg from where it
    /// stands to each point in turn (in game only; `movement.md`, "Paths"),
    /// timed with the run speed the server gave ([`World::run_speed`]) and
    /// our movement speed. A walk asked for while one is on replaces it.
    MoveTo {
        /// The points to walk to, the end last.
        path: Vec<Point>,
    },
    /// Stop walking where we are.
    Stop,
    /// Play the character in this slot (while [`Stage::Selecting`]).
    Select {
        /// Slot 0 to 3 of the character list.
        slot: u8,
    },
    /// Create a character in the first free slot (while
    /// [`Stage::Selecting`]). Its empire counts only on an account without
    /// characters (`session.md`, "Characters").
    Create(NewCharacter),
    /// Use the item in a slot: wear it, take it off, or use it up (in game
    /// only; `inventory.md`). What changes comes back from the server.
    UseItem {
        /// Its window.
        window: Window,
        /// Its slot.
        slot: u16,
    },
    /// Move an item to another slot (in game only).
    MoveItem {
        /// Window and slot it is in.
        from: (Window, u16),
        /// Window and slot it goes to.
        to: (Window, u16),
        /// How many.
        count: u8,
    },
    /// Put an item or a skill on quickslot `index`, 0 to 31 (in game only).
    SetQuickSlot {
        /// The quickslot.
        index: u8,
        /// What it holds.
        slot: QuickSlot,
    },
    /// Empty quickslot `index` (in game only).
    ClearQuickSlot {
        /// The quickslot.
        index: u8,
    },
    /// Swap what two quickslots hold (in game only).
    SwapQuickSlots {
        /// One quickslot.
        first: u8,
        /// The other.
        second: u8,
    },
    /// Choose the entity we look at (in game only; `combat.md`). The
    /// server answers with its health.
    Target {
        /// The entity.
        vid: u32,
    },
    /// Start an attack motion, standing where we are and facing `angle`
    /// (in game only; `combat.md`). A walk in progress stops first. The
    /// hits are told with [`Command::Hit`].
    Attack {
        /// Direction faced, in degrees ([`Point::angle_to`]).
        angle: u16,
        /// The step of a combo, from 1; 0 for a single attack.
        combo: u8,
    },
    /// Tell the server one of our attacks hit an entity (in game only).
    Hit {
        /// The entity hit.
        vid: u32,
        /// The skill whose motion hit it; 0 for an attack.
        skill: u8,
    },
    /// Come back to life after dying, where we fell or in town (in game
    /// only; `combat.md`).
    Restart {
        /// True for town.
        town: bool,
    },
    /// Pick up an item from the ground (in game only; `ground-items.md`).
    /// Walking up to it first is the caller's business; what we got comes
    /// back from the server.
    PickUp {
        /// The ground item.
        vid: u32,
    },
    /// Drop `count` of the item in a slot of the bag where our character
    /// stands (in game only).
    Drop {
        /// Its slot, below [`crate::BAG_SLOTS`]: worn items are taken off
        /// first.
        slot: u16,
        /// How many, from 1 to the stack's count.
        count: u8,
    },
    /// Drop some of our gold where our character stands (in game only).
    DropGold {
        /// How much, from 1 to what we have.
        amount: u32,
    },
    /// Talk to an NPC near us (in game only; `shops.md`): its shop opens if
    /// it has one. Walking up to it first is the caller's business.
    Talk {
        /// The NPC.
        vid: u32,
    },
    /// Buy what lies in a place of the open shop. What we got comes back
    /// from the server; it may refuse ([`Event::ShopRefused`]).
    Buy {
        /// The place, below [`SHOP_PLACES`].
        position: u8,
    },
    /// Sell the item in a slot of the bag to the open shop: the whole stack.
    Sell {
        /// Its slot, below [`crate::BAG_SLOTS`].
        slot: u16,
    },
    /// Close the open shop.
    CloseShop,
    /// Learn the skill group of the teacher open (in game only;
    /// `skills.md`). The server checks the job, the level and the group;
    /// the group comes back in [`World::skill_group`].
    LearnSkillGroup {
        /// The teacher.
        vid: u32,
    },
    /// Close the open teacher.
    CloseTeacher,
    /// Train one of the skills the open skill choice offers with its item
    /// (in game only; `skills.md`, "Training"). The server checks the item
    /// and the skill again; the choice closes.
    TrainSkill {
        /// The skill.
        skill: u8,
    },
    /// Close the open skill choice; the server keeps nothing of it.
    CloseSkillChoice,
    /// Get on our mount (`ride` true; the server calls it first when it is
    /// away and the bag holds its call) or off it (in game only;
    /// `mounts.md`).
    Ride {
        /// True to get on.
        ride: bool,
    },
    /// Train our mount at the stable open (in game only; `mounts.md`). The
    /// server checks the costs; the level comes back in [`World::mount`].
    TrainMount {
        /// The stable.
        vid: u32,
    },
    /// Close the open stable.
    CloseStable,
    /// Ask NPC `vid` what upgrading the item at bag slot `slot` takes: the
    /// item dropped onto it (in game only; `upgrades.md`). A smith answers
    /// with an offer, in [`World::upgrade`].
    GiveItem {
        /// The NPC.
        vid: u32,
        /// The item's slot in the bag.
        slot: u16,
    },
    /// Cast our fishing pole's line the way `angle` says (degrees, as
    /// moves give them), in game only: we stop first and
    /// turn that way; the server answers with Fishing, or a line why not.
    Cast {
        /// The way, in degrees.
        angle: u16,
    },
    /// Reel our fishing line in (in game only): after a bite to catch what
    /// bit, before one to stop.
    ReelIn,
    /// Put the item at bag slot `slot` onto the one at `onto` (in game
    /// only): an upgrade scroll gets an offer, in [`World::upgrade`].
    UseItemOnItem {
        /// The item put on the other.
        slot: u16,
        /// The item it is put on.
        onto: u16,
    },
    /// Upgrade the item of the offer open, at its costs (in game only;
    /// `upgrades.md`); the offer closes. The server checks it all again;
    /// the result comes back as the item's ItemSet and a line.
    Upgrade,
    /// Close the upgrade offer open.
    CloseUpgrade,
    /// Spend a skill point on one of our skills (in game only; `skills.md`).
    /// The server checks the points and the skill; the new level comes back
    /// in [`World::skills`].
    SkillUp {
        /// The skill's number.
        skill: u8,
    },
    /// Spend a status point on strength (12), vitality (13), dexterity (14)
    /// or intelligence (15) (in game only; `entities.md`, "Points"). The
    /// server checks the points and the stat's highest; the new values come
    /// back in [`World::points`].
    RaiseStatus {
        /// The stat's point number.
        point: u8,
    },
    /// Use one of our skills (in game only; `skills.md`): its motion starts
    /// where we stand, facing `angle`; a walk in progress stops first. Its
    /// hits are told with [`Command::Hit`] and the skill.
    UseSkill {
        /// The skill's number: one of a job's ([`crate::skill_motion`]),
        /// or a horseback skill riding, learned.
        skill: u8,
        /// Direction faced, in degrees ([`Point::angle_to`]).
        angle: u16,
        /// The entity it is used on, if any.
        target: Option<u32>,
        /// The motion it plays, as the client's skill table says: needed
        /// for a horseback skill; a job's skill's is worked out without it.
        motion: Option<u8>,
        /// A charge (`mounts.md`): the point the skill takes us to, its
        /// motion played there.
        to: Option<Point>,
    },
    /// Leave the game for the character list, on the same game connection
    /// (in game only; `session.md`, "Changing character"): with
    /// [`CharacterChoice::Ask`] the session waits at [`Stage::Selecting`]
    /// again, else it picks as at the login.
    ChangeCharacter,
    /// Whisper to a player in the game, by its character's name (in game
    /// only; `chat.md`, "Whispers"): it comes back as ours
    /// ([`Event::Whisper`]), or refused. A name that is no character's, our
    /// own, or a line that is empty, over 512 bytes or with control
    /// characters is ignored.
    Whisper {
        /// The character's name, in any case.
        name: String,
        /// What we say.
        text: String,
    },
    /// Ask a player in the game to be our friend (in game only;
    /// `friends.md`).
    AskFriend {
        /// Its character's name, in any case.
        name: String,
    },
    /// Answer a player that asked to be our friend (in game only).
    AnswerFriend {
        /// Its name.
        name: String,
        /// True to become friends.
        accept: bool,
    },
    /// End a friendship (in game only); the friend need not be in the game.
    RemoveFriend {
        /// The friend's name.
        name: String,
    },
    /// Forget our whisper conversation with a player; nothing is sent.
    ForgetWhispers {
        /// Its name, in any case.
        name: String,
    },
    /// Close the game connection and stop.
    Logout,
}

/// One input to [`Session::step`].
#[derive(Debug, Clone, PartialEq)]
// Inputs are handled right away; boxing the message would only add noise.
#[allow(clippy::large_enum_variant)]
pub enum Input {
    /// Start: connect to the auth server.
    Start,
    /// A connection asked for with [`Effect::Connect`] is open.
    Connected(ConnId),
    /// A connection closed by the other side, failed to open, or was dropped by
    /// the network layer (for example after bytes it could not read).
    Closed {
        /// The connection.
        conn: ConnId,
        /// What happened, for the error message.
        reason: String,
    },
    /// A message arrived.
    Message(ConnId, ServerMessage),
    /// A command.
    Command(Command),
    /// The provider's redirect came back with a code ([`Effect::SignIn`]).
    SignInCode(SignInCode),
    /// Signing in at the provider failed ([`Effect::SignIn`]).
    SignInFailed {
        /// What happened, for the error message.
        reason: String,
    },
    /// Time passed; checks timeouts.
    Tick,
}

/// The current connection.
#[derive(Debug, Clone)]
struct Link {
    id: ConnId,
    server: ServerKind,
    /// The server answered our `Hello` with `Welcome`.
    welcomed: bool,
    phase: Option<Phase>,
    /// Last time it was opened or sent something.
    last_heard: Millis,
    enter_sent: bool,
}

/// The client session state machine. See the module docs.
#[derive(Debug)]
pub struct Session {
    config: SessionConfig,
    stage: Stage,
    next_conn: ConnId,
    link: Option<Link>,
    /// The auth server's ticket for the game server.
    ticket: Option<SecretSlice<u8>>,
    /// The account a provider's sign-in opened, for TicketLogin.
    account: Option<String>,
    /// Our walk in progress: where the leg we are on ends and its
    /// direction. The rest of the path is in [`World::path`].
    walk: Option<(Point, u16)>,
    /// The character we asked the server to create.
    creating: Option<NewCharacter>,
    error: Option<SessionError>,
    world: World,
}

impl Session {
    /// A session that has not started yet. Send [`Input::Start`] to begin.
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            stage: Stage::Idle,
            next_conn: ConnId(1),
            link: None,
            ticket: None,
            account: None,
            walk: None,
            creating: None,
            error: None,
            world: World::default(),
        }
    }

    /// Where the session is.
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// What the session has learned.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Why the session failed, if it did.
    pub fn error(&self) -> Option<&SessionError> {
        self.error.as_ref()
    }

    /// True once the session logged out or failed; no more effects will come.
    pub fn is_finished(&self) -> bool {
        matches!(self.stage, Stage::LoggedOut | Stage::Failed)
    }

    /// The connection in use, if any.
    pub fn current_conn(&self) -> Option<ConnId> {
        self.link.as_ref().map(|link| link.id)
    }

    /// Handles one input at time `now` and returns what to do.
    pub fn step(&mut self, now: Millis, input: Input) -> Vec<Effect> {
        let mut out = Vec::new();
        match input {
            Input::Start => self.start(now, &mut out),
            Input::Connected(conn) => self.on_connected(now, conn, &mut out),
            Input::Closed { conn, reason } => self.on_closed(conn, reason, &mut out),
            Input::Message(conn, message) => self.on_message(now, conn, message, &mut out),
            Input::Command(command) => self.on_command(now, command, &mut out),
            Input::SignInCode(code) => self.on_sign_in_code(code, &mut out),
            Input::SignInFailed { reason } => {
                if self.stage == Stage::SigningIn {
                    self.fail(SessionError::SignIn { reason }, &mut out);
                }
            }
            Input::Tick => self.on_tick(now, &mut out),
        }
        out
    }

    fn link_mut(&mut self, conn: ConnId) -> Option<&mut Link> {
        self.link.as_mut().filter(|link| link.id == conn)
    }

    fn start(&mut self, now: Millis, out: &mut Vec<Effect>) {
        if self.stage != Stage::Idle {
            out.push(ignored("Start", "the session already started"));
            return;
        }
        self.stage = Stage::Auth;
        self.connect(now, ServerKind::Auth, out);
    }

    fn connect(&mut self, now: Millis, server: ServerKind, out: &mut Vec<Effect>) {
        let conn = self.next_conn;
        self.next_conn = conn.next();
        self.link = Some(Link {
            id: conn,
            server,
            welcomed: false,
            phase: None,
            last_heard: now,
            enter_sent: false,
        });
        out.push(Effect::Connect { conn, server });
    }

    /// The connection is open: we say which protocol we speak
    /// (`session.md`, "Versions").
    fn on_connected(&mut self, now: Millis, conn: ConnId, out: &mut Vec<Effect>) {
        let Some(link) = self.link_mut(conn) else {
            return;
        };
        link.last_heard = now;
        let hello = msg::Hello {
            protocol: PROTOCOL,
            build: self.config.build.clone(),
        };
        self.send(ClientMessage::Hello(hello), out);
    }

    fn disconnect_current(&mut self, out: &mut Vec<Effect>) {
        if let Some(link) = self.link.take() {
            out.push(Effect::Disconnect { conn: link.id });
        }
    }

    fn send(&self, message: ClientMessage, out: &mut Vec<Effect>) {
        if let Some(link) = &self.link {
            out.push(Effect::Send {
                conn: link.id,
                message,
            });
        }
    }

    fn fail(&mut self, error: SessionError, out: &mut Vec<Effect>) {
        self.disconnect_current(out);
        self.stage = Stage::Failed;
        self.error = Some(error.clone());
        out.push(Effect::Event(Event::Failed(error)));
    }

    fn on_closed(&mut self, conn: ConnId, reason: String, out: &mut Vec<Effect>) {
        // A connection we already replaced or dropped: nothing to do.
        let Some(link) = self.link.as_ref().filter(|link| link.id == conn) else {
            return;
        };
        let server = link.server;
        self.link = None;
        let stage = self.stage;
        self.fail(
            SessionError::Disconnected {
                server,
                stage,
                reason,
            },
            out,
        );
    }

    fn on_tick(&mut self, now: Millis, out: &mut Vec<Effect>) {
        let Some(link) = &self.link else { return };
        if now.since(link.last_heard) >= SILENCE_LIMIT_MS {
            let server = link.server;
            let stage = self.stage;
            self.fail(SessionError::Silent { server, stage }, out);
            return;
        }
        self.check_arrival(now, out);
    }

    /// Ends our walk once the track says we are at the path's end: we tell
    /// the server we stand there (`movement.md`).
    fn check_arrival(&mut self, now: Millis, out: &mut Vec<Effect>) {
        self.follow_path(now, out);
        let Some((to, angle)) = self.walk else { return };
        let arrived = self.world.path.is_none()
            && self
                .world
                .my_entity()
                .is_none_or(|me| !me.track.is_moving(now));
        if arrived {
            self.walk = None;
            self.send_move(now, MoveKind::Wait, to, angle, out);
            out.push(Effect::Event(Event::Arrived(to)));
        }
    }

    /// Starts the legs of our path that have begun by `now`, each told to
    /// the server as a walk at the time it began (`movement.md`, "Paths").
    fn follow_path(&mut self, now: Millis, out: &mut Vec<Effect>) {
        if self.walk.is_none() {
            return;
        }
        for leg in self.world.follow_path(now) {
            self.walk = Some((leg.to, leg.angle));
            self.send_move(leg.start, MoveKind::Move, leg.to, leg.angle, out);
        }
    }

    /// Our walk ends, its path too.
    fn end_walk(&mut self) {
        self.walk = None;
        self.world.path = None;
    }

    fn send_move(&self, now: Millis, kind: MoveKind, at: Point, angle: u16, out: &mut Vec<Effect>) {
        self.send_move_with(now, kind, 0, at, angle, out);
    }

    fn send_move_with(
        &self,
        now: Millis,
        kind: MoveKind,
        argument: u8,
        at: Point,
        angle: u16,
        out: &mut Vec<Effect>,
    ) {
        let message = msg::CharacterMove {
            kind: kind.to_wire(),
            argument,
            angle: angle % 360,
            x: at.x,
            y: at.y,
            time: self.world.clock.map_or(0, |clock| clock.at(now)),
        };
        self.send(ClientMessage::CharacterMove(message), out);
    }

    /// Starts walking along `path` (`movement.md`, "Paths"): its first leg
    /// now, each of the others when the one before it ends.
    fn move_to(&mut self, now: Millis, path: Vec<Point>, out: &mut Vec<Effect>) {
        if self.stage != Stage::InGame {
            out.push(ignored("MoveTo", "walking needs to be in game"));
            return;
        }
        let speed = self.world.run_speed;
        if speed == 0 {
            out.push(ignored("MoveTo", "the server gave no run speed"));
            return;
        }
        if path.is_empty() {
            out.push(ignored("MoveTo", "the path is empty"));
            return;
        }
        let vid = self.world.me.as_ref().map(|me| me.vid);
        let Some(me) = vid.and_then(|vid| self.world.entities.get_mut(&vid)) else {
            out.push(ignored("MoveTo", "our character is not in the world yet"));
            return;
        };
        if me.dead {
            out.push(ignored("MoveTo", "our character is dead"));
            return;
        }
        let from = me.track.point(now);
        let mut ahead: VecDeque<Point> = path.into_iter().skip_while(|&p| p == from).collect();
        let Some(to) = ahead.pop_front() else {
            return;
        };
        let angle = from.angle_to(to);
        me.track = Track {
            from,
            to,
            start: now,
            duration: walk_duration(from.distance(to), speed, me.move_speed),
        };
        me.angle = angle;
        me.action = MoveKind::Move;
        me.argument = 0;
        self.walk = Some((to, angle));
        self.world.path = (!ahead.is_empty()).then_some(Path { ahead, speed });
        self.send_move(now, MoveKind::Move, to, angle, out);
    }

    /// Stops a walk where we are now; the rest of its path is dropped.
    fn stop(&mut self, now: Millis, out: &mut Vec<Effect>) {
        self.world.path = None;
        let Some((_, angle)) = self.walk.take() else {
            return;
        };
        let vid = self.world.me.as_ref().map(|me| me.vid);
        let Some(me) = vid.and_then(|vid| self.world.entities.get_mut(&vid)) else {
            return;
        };
        let at = me.track.point(now);
        me.track = Track::still(at, now);
        me.action = MoveKind::Wait;
        me.argument = 0;
        self.send_move(now, MoveKind::Wait, at, angle, out);
    }

    /// A charge (`mounts.md`): our character goes to `to` at once, its
    /// skill motion `motion` played there; one skill move tells the server.
    fn charge(&mut self, now: Millis, to: Point, motion: u8, angle: u16, out: &mut Vec<Effect>) {
        self.end_walk();
        let vid = self.world.me.as_ref().map(|me| me.vid);
        let Some(me) = vid.and_then(|vid| self.world.entities.get_mut(&vid)) else {
            return;
        };
        let angle = angle % 360;
        me.track = Track::still(to, now);
        me.angle = angle;
        me.action = MoveKind::Skill(motion);
        me.argument = 0;
        self.send_move_with(now, MoveKind::Skill(motion), 0, to, angle, out);
    }

    /// Starts an attack motion where we stand (`combat.md`).
    fn attack(&mut self, now: Millis, angle: u16, combo: u8, out: &mut Vec<Effect>) {
        if self.stage != Stage::InGame {
            out.push(ignored("Attack", "attacking needs to be in game"));
            return;
        }
        if self.world.riding() && !self.world.mount.is_some_and(|mount| mount.fights) {
            out.push(ignored("Attack", "our mount does not fight"));
            return;
        }
        match self.world.my_entity() {
            None => {
                out.push(ignored("Attack", "our character is not in the world yet"));
                return;
            }
            Some(me) if me.dead => {
                out.push(ignored("Attack", "our character is dead"));
                return;
            }
            Some(_) => {}
        }
        let kind = if combo == 0 {
            MoveKind::Attack
        } else {
            MoveKind::Combo
        };
        self.start_motion(now, kind, combo, angle, out);
    }

    /// Our character starts a motion where it stands, facing `angle`: an
    /// attack or a skill. The server hears of it in a move message.
    fn start_motion(
        &mut self,
        now: Millis,
        kind: MoveKind,
        argument: u8,
        angle: u16,
        out: &mut Vec<Effect>,
    ) {
        // The server keeps walking us to the end of a walk until it hears
        // where we stopped.
        self.stop(now, out);
        let vid = self.world.me.as_ref().map(|me| me.vid);
        let Some(me) = vid.and_then(|vid| self.world.entities.get_mut(&vid)) else {
            return;
        };
        let at = me.track.point(now);
        let angle = angle % 360;
        // Standing since now: the motion starts with the track.
        me.track = Track::still(at, now);
        me.angle = angle;
        me.action = kind;
        me.argument = argument;
        self.send_move_with(now, kind, argument, at, angle, out);
    }

    /// Uses one of our skills (`skills.md`): its motion, then the skill
    /// itself.
    fn use_skill(
        &mut self,
        now: Millis,
        (skill, told): (u8, Option<u8>),
        angle: u16,
        target: Option<u32>,
        to: Option<Point>,
        out: &mut Vec<Effect>,
    ) {
        if let Some(why) = self.cannot_act() {
            out.push(ignored("UseSkill", why));
            return;
        }
        // A job's skill on foot; a horseback skill (told its motion) riding
        // a mount that has them (`mounts.md`).
        let riding = self.world.riding();
        let motion = match (skill_motion(skill), told) {
            (Some(_), _) if riding => {
                out.push(ignored("UseSkill", "a job's skills are used on foot"));
                return;
            }
            (Some(motion), _) => motion,
            (None, Some(_)) if !riding => {
                out.push(ignored("UseSkill", "horseback skills are used riding"));
                return;
            }
            (None, Some(_)) if !self.world.mount.is_some_and(|mount| mount.skills) => {
                out.push(ignored("UseSkill", "our mount has no horseback skills"));
                return;
            }
            (None, Some(motion)) => motion,
            (None, None) => {
                out.push(ignored("UseSkill", "the skill's motion is not known"));
                return;
            }
        };
        if self.world.skills.get(&skill).is_none_or(|s| s.level == 0) {
            out.push(ignored("UseSkill", "the skill is not learned"));
            return;
        }
        if let Some(to) = to.filter(|_| riding) {
            self.charge(now, to, motion, angle, out);
        } else {
            self.start_motion(now, MoveKind::Skill(motion), 0, angle, out);
        }
        let message = msg::UseSkill { skill, target };
        self.send(ClientMessage::UseSkill(message), out);
    }

    /// All the skills we have learned (`skills.md`): each that changed (or,
    /// after entering, each learned) is told.
    fn on_skills(&mut self, learned: &[msg::SkillState], out: &mut Vec<Effect>) {
        let skills: BTreeMap<u8, Skill> = learned
            .iter()
            .map(|entry| {
                let skill = Skill {
                    level: entry.level,
                    grade: entry.grade,
                    next_read: entry.next_read,
                };
                (entry.skill, skill)
            })
            .filter(|(_, skill)| *skill != Skill::default())
            .collect();
        let numbers: BTreeSet<u8> = skills
            .keys()
            .chain(self.world.skills.keys())
            .copied()
            .collect();
        for skill in numbers {
            let now = skills.get(&skill).copied().unwrap_or_default();
            if self.world.skills.get(&skill).copied().unwrap_or_default() != now {
                out.push(Effect::Event(Event::SkillChanged {
                    skill,
                    level: now.level,
                    grade: now.grade,
                }));
            }
        }
        self.world.skills = skills;
    }

    fn on_command(&mut self, now: Millis, command: Command, out: &mut Vec<Effect>) {
        // Commands act from where our character is now, on whichever leg.
        self.follow_path(now, out);
        match command {
            Command::Chat { kind, text } => {
                if self.stage != Stage::InGame {
                    out.push(ignored("Chat", "chat needs to be in game"));
                } else if text.len() > CHAT_MAX {
                    out.push(ignored("Chat", "the line is longer than 512 bytes"));
                } else {
                    let kind = kind.to_wire();
                    self.send(ClientMessage::SendChat(msg::SendChat { kind, text }), out);
                }
            }
            Command::Whisper { name, text } => self.whisper(name, text, out),
            Command::AskFriend { name } => {
                if self.names_someone_else("AskFriend", &name, out) {
                    let ask = msg::AskFriend { name };
                    self.send_in_game("AskFriend", ClientMessage::AskFriend(ask), out);
                }
            }
            Command::AnswerFriend { name, accept } => {
                if self.names_someone_else("AnswerFriend", &name, out) {
                    let requests = &mut self.world.friend_requests;
                    requests.retain(|asker| !asker.eq_ignore_ascii_case(&name));
                    let answer = msg::AnswerFriend { name, accept };
                    self.send_in_game("AnswerFriend", ClientMessage::AnswerFriend(answer), out);
                }
            }
            Command::RemoveFriend { name } => {
                if self.names_someone_else("RemoveFriend", &name, out) {
                    let remove = msg::RemoveFriend { name };
                    self.send_in_game("RemoveFriend", ClientMessage::RemoveFriend(remove), out);
                }
            }
            Command::ForgetWhispers { name } => self.world.forget_whispers(&name),
            Command::MoveTo { path } => self.move_to(now, path, out),
            Command::Stop => self.stop(now, out),
            Command::Select { slot } => self.select_asked(slot, out),
            Command::Create(new) => self.create_asked(new, out),
            Command::UseItem { window, slot } => {
                let window = window.to_wire();
                self.send_in_game(
                    "UseItem",
                    ClientMessage::UseItem(msg::UseItem { window, slot }),
                    out,
                );
            }
            Command::MoveItem { from, to, count } => {
                let message = msg::MoveItem {
                    from_window: from.0.to_wire(),
                    from_slot: from.1,
                    to_window: to.0.to_wire(),
                    to_slot: to.1,
                    count,
                };
                self.send_in_game("MoveItem", ClientMessage::MoveItem(message), out);
            }
            Command::SetQuickSlot { index, slot } => {
                if index >= QUICK_SLOTS {
                    out.push(ignored("SetQuickSlot", "there are 32 quickslots"));
                    return;
                }
                let message = msg::SetQuickSlot {
                    index,
                    kind: slot.kind.to_wire(),
                    position: slot.position,
                };
                self.send_in_game("SetQuickSlot", ClientMessage::SetQuickSlot(message), out);
            }
            Command::ClearQuickSlot { index } => {
                if index >= QUICK_SLOTS {
                    out.push(ignored("ClearQuickSlot", "there are 32 quickslots"));
                    return;
                }
                let message = ClientMessage::ClearQuickSlot(msg::ClearQuickSlot { index });
                self.send_in_game("ClearQuickSlot", message, out);
            }
            Command::SwapQuickSlots { first, second } => {
                if first >= QUICK_SLOTS || second >= QUICK_SLOTS {
                    out.push(ignored("SwapQuickSlots", "there are 32 quickslots"));
                    return;
                }
                let message = ClientMessage::SwapQuickSlots(msg::SwapQuickSlots { first, second });
                self.send_in_game("SwapQuickSlots", message, out);
            }
            Command::Target { vid } => {
                if self.stage == Stage::InGame {
                    self.world.target = Some(vid);
                    self.send(ClientMessage::SetTarget(msg::SetTarget { vid }), out);
                } else {
                    out.push(ignored("Target", "fighting needs to be in game"));
                }
            }
            Command::Attack { angle, combo } => self.attack(now, angle, combo, out),
            Command::Hit { vid, skill } => {
                if self.world.my_entity().is_some_and(|me| me.dead) {
                    out.push(ignored("Hit", "our character is dead"));
                    return;
                }
                let fights = self.world.mount.is_some_and(|mount| mount.fights);
                if skill == 0 && self.world.riding() && !fights {
                    out.push(ignored("Hit", "our mount does not fight"));
                    return;
                }
                self.send_in_game("Hit", ClientMessage::Hit(msg::Hit { vid, skill }), out);
            }
            Command::Restart { town } => {
                let message = ClientMessage::Restart(msg::Restart { town });
                self.send_in_game("Restart", message, out);
            }
            Command::PickUp { vid } => self.pick_up(vid, out),
            Command::Drop { slot, count } => self.drop_item(slot, count, out),
            Command::DropGold { amount } => self.drop_gold(amount, out),
            Command::Talk { vid } => self.talk(vid, out),
            Command::Buy { position } => self.buy(position, out),
            Command::Sell { slot } => self.sell(slot, out),
            Command::CloseShop => {
                if self.world.shop.is_none() {
                    out.push(ignored("CloseShop", "no shop is open"));
                } else {
                    self.close_shop(out);
                }
            }
            Command::LearnSkillGroup { vid } => {
                if self.world.teacher.is_some_and(|teacher| teacher.vid == vid) {
                    let message = ClientMessage::LearnSkillGroup(msg::LearnSkillGroup { vid });
                    self.send_in_game("LearnSkillGroup", message, out);
                } else {
                    out.push(ignored("LearnSkillGroup", "that teacher is not open"));
                }
            }
            Command::CloseTeacher => {
                if self.world.teacher.is_none() {
                    out.push(ignored("CloseTeacher", "no teacher is open"));
                } else {
                    self.close_teacher(out);
                }
            }
            Command::TrainSkill { skill } => match self.world.skill_choice.as_ref() {
                Some(choice) if choice.skills.contains(&skill) => {
                    let message = ClientMessage::UseItemOnSkill(msg::UseItemOnSkill {
                        slot: choice.slot,
                        skill,
                    });
                    self.send_in_game("UseItemOnSkill", message, out);
                    self.close_skill_choice(out);
                }
                Some(_) => out.push(ignored("TrainSkill", "the item does not offer that skill")),
                None => out.push(ignored("TrainSkill", "no skill choice is open")),
            },
            Command::CloseSkillChoice => {
                if self.world.skill_choice.is_none() {
                    out.push(ignored("CloseSkillChoice", "no skill choice is open"));
                } else {
                    self.close_skill_choice(out);
                }
            }
            Command::Ride { ride } => {
                if ride && self.world.mount.is_none() {
                    out.push(ignored("Ride", "we have no mount"));
                } else if ride == self.world.riding() {
                    out.push(ignored("Ride", "riding already, or on foot already"));
                } else {
                    let message = ClientMessage::RideMount(msg::RideMount { ride });
                    self.send_in_game("Ride", message, out);
                }
            }
            Command::TrainMount { vid } => {
                if self
                    .world
                    .stable
                    .as_ref()
                    .is_some_and(|stable| stable.vid == vid)
                {
                    let message = ClientMessage::TrainMount(msg::TrainMount { vid });
                    self.send_in_game("TrainMount", message, out);
                } else {
                    out.push(ignored("TrainMount", "that stable is not open"));
                }
            }
            Command::CloseStable => {
                if self.world.stable.is_none() {
                    out.push(ignored("CloseStable", "no stable is open"));
                } else {
                    self.close_stable(out);
                }
            }
            Command::GiveItem { vid, slot } => {
                if slot < BAG_SLOTS {
                    let message = ClientMessage::GiveItem(msg::GiveItem { vid, slot });
                    self.send_in_game("GiveItem", message, out);
                } else {
                    out.push(ignored("GiveItem", "not a slot of the bag"));
                }
            }
            Command::Cast { angle } => self.cast(now, angle, out),
            Command::ReelIn => {
                self.send_in_game("ReelIn", ClientMessage::ReelIn(msg::ReelIn), out);
            }
            Command::UseItemOnItem { slot, onto } => {
                if slot < BAG_SLOTS && onto < BAG_SLOTS && slot != onto {
                    let message = ClientMessage::UseItemOnItem(msg::UseItemOnItem { slot, onto });
                    self.send_in_game("UseItemOnItem", message, out);
                } else {
                    out.push(ignored("UseItemOnItem", "not two slots of the bag"));
                }
            }
            Command::Upgrade => match self.world.upgrade.as_ref() {
                Some(offer) => {
                    let message = ClientMessage::Upgrade(msg::Upgrade {
                        smith: offer.smith,
                        scroll: offer.scroll,
                        slot: offer.slot,
                        vnum: offer.vnum,
                    });
                    self.send_in_game("Upgrade", message, out);
                    self.close_upgrade(out);
                }
                None => out.push(ignored("Upgrade", "no upgrade is offered")),
            },
            Command::CloseUpgrade => {
                if self.world.upgrade.is_none() {
                    out.push(ignored("CloseUpgrade", "no upgrade is offered"));
                } else {
                    self.close_upgrade(out);
                }
            }
            Command::SkillUp { skill } => {
                let message = ClientMessage::RaiseSkill(msg::RaiseSkill { skill });
                self.send_in_game("SkillUp", message, out);
            }
            Command::RaiseStatus { point } => {
                if (12..=15).contains(&point) {
                    let message = ClientMessage::RaiseStatus(msg::RaiseStatus { point });
                    self.send_in_game("RaiseStatus", message, out);
                } else {
                    out.push(ignored("RaiseStatus", "only a stat is raised"));
                }
            }
            Command::UseSkill {
                skill,
                angle,
                target,
                motion,
                to,
            } => self.use_skill(now, (skill, motion), angle, target, to, out),
            Command::ChangeCharacter => {
                if self.stage != Stage::InGame {
                    out.push(ignored("ChangeCharacter", "this needs to be in game"));
                    return;
                }
                self.stage = Stage::Leaving;
                self.send(ClientMessage::LeaveGame(msg::LeaveGame), out);
            }
            Command::Logout => {
                if self.is_finished() {
                    return;
                }
                self.disconnect_current(out);
                self.stage = Stage::LoggedOut;
                out.push(Effect::Event(Event::LoggedOut));
            }
        }
    }

    /// Why our character cannot act on the world now: not in the game, not
    /// in the world yet, or dead.
    fn cannot_act(&self) -> Option<&'static str> {
        if self.stage != Stage::InGame {
            return Some("this needs to be in game");
        }
        match self.world.my_entity() {
            None => Some("our character is not in the world yet"),
            Some(me) if me.dead => Some("our character is dead"),
            Some(_) => None,
        }
    }

    /// Asks for an item on the ground (`ground-items.md`).
    fn pick_up(&mut self, vid: u32, out: &mut Vec<Effect>) {
        if let Some(why) = self.cannot_act() {
            out.push(ignored("PickUp", why));
        } else if !self.world.ground.contains_key(&vid) {
            out.push(ignored("PickUp", "no such item on the ground"));
        } else {
            self.send(ClientMessage::PickUp(msg::PickUp { vid }), out);
        }
    }

    /// Drops some of the item in a slot of the bag (`ground-items.md`).
    fn drop_item(&mut self, slot: u16, count: u8, out: &mut Vec<Effect>) {
        if let Some(why) = self.cannot_act() {
            out.push(ignored("Drop", why));
            return;
        }
        if slot >= BAG_SLOTS {
            out.push(ignored("Drop", "only items in the bag are dropped"));
            return;
        }
        let window = Window::Inventory;
        match self.world.items.get(&(window, slot)) {
            None => out.push(ignored("Drop", "no item in that slot")),
            Some(item) if count == 0 || count > item.count => {
                out.push(ignored("Drop", "more than the stack, or none"));
            }
            Some(_) => {
                let message = msg::DropItem {
                    window: window.to_wire(),
                    slot,
                    count,
                };
                self.send(ClientMessage::DropItem(message), out);
            }
        }
    }

    /// Drops some of our gold (`ground-items.md`).
    fn drop_gold(&mut self, amount: u32, out: &mut Vec<Effect>) {
        if let Some(why) = self.cannot_act() {
            out.push(ignored("DropGold", why));
            return;
        }
        let gold = self.world.points.get(&GOLD_POINT).copied().unwrap_or(0);
        if amount == 0 || amount > gold {
            out.push(ignored("DropGold", "more than our gold, or none"));
            return;
        }
        self.send(ClientMessage::DropGold(msg::DropGold { amount }), out);
    }

    /// Talks to an NPC (`shops.md`). Only NPCs we know are asked for.
    fn talk(&mut self, vid: u32, out: &mut Vec<Effect>) {
        if let Some(why) = self.cannot_act() {
            out.push(ignored("Talk", why));
            return;
        }
        match self.world.entities.get(&vid) {
            None => out.push(ignored("Talk", "no such entity")),
            Some(entity) if entity.kind != EntityKind::Npc => {
                out.push(ignored("Talk", "only NPCs are talked to"));
            }
            Some(_) => self.send(ClientMessage::Talk(msg::Talk { vid }), out),
        }
    }

    /// Buys from the open shop (`shops.md`): only what lies there and we
    /// can pay for.
    fn buy(&mut self, position: u8, out: &mut Vec<Effect>) {
        if let Some(why) = self.cannot_act() {
            out.push(ignored("Buy", why));
            return;
        }
        let Some(shop) = &self.world.shop else {
            out.push(ignored("Buy", "no shop is open"));
            return;
        };
        let gold = self.world.points.get(&GOLD_POINT).copied().unwrap_or(0);
        match shop.goods.get(&position) {
            None => out.push(ignored("Buy", "nothing in that place")),
            Some(good) if good.price > gold => {
                out.push(ignored("Buy", "it costs more than our gold"));
            }
            Some(_) => self.send(ClientMessage::ShopBuy(msg::ShopBuy { position }), out),
        }
    }

    /// Sells an item of the bag to the open shop (`shops.md`).
    fn sell(&mut self, slot: u16, out: &mut Vec<Effect>) {
        if let Some(why) = self.cannot_act() {
            out.push(ignored("Sell", why));
            return;
        }
        if self.world.shop.is_none() {
            out.push(ignored("Sell", "no shop is open"));
            return;
        }
        if slot >= BAG_SLOTS {
            out.push(ignored("Sell", "only items in the bag are sold"));
            return;
        }
        match self.world.items.get(&(Window::Inventory, slot)) {
            None => out.push(ignored("Sell", "no item in that slot")),
            Some(item) => {
                let message = msg::ShopSell {
                    slot,
                    count: item.count,
                };
                self.send(ClientMessage::ShopSell(message), out);
            }
        }
    }

    /// Tells the server we closed the shop, and forgets it.
    fn close_shop(&mut self, out: &mut Vec<Effect>) {
        self.world.shop = None;
        if self.stage == Stage::InGame {
            self.send(ClientMessage::ShopClose(msg::ShopClose), out);
        }
        out.push(Effect::Event(Event::ShopClosed));
    }

    /// Forgets the teacher open; the server keeps nothing of it.
    fn close_teacher(&mut self, out: &mut Vec<Effect>) {
        self.world.teacher = None;
        out.push(Effect::Event(Event::TeacherClosed));
    }

    /// Forgets the skill choice open; the server keeps nothing of it.
    fn close_skill_choice(&mut self, out: &mut Vec<Effect>) {
        self.world.skill_choice = None;
        out.push(Effect::Event(Event::SkillChoiceClosed));
    }

    /// What skills raise on our character now (`skills.md`, "What skills
    /// raise"): the whole list, its times from `now`.
    /// Casts our line: in game, alive and on foot, we stop
    /// where we are, face `angle`, and ask the server.
    fn cast(&mut self, now: Millis, angle: u16, out: &mut Vec<Effect>) {
        if self.stage != Stage::InGame {
            out.push(ignored("Cast", "fishing needs to be in game"));
            return;
        }
        if self.world.riding() {
            out.push(ignored("Cast", "not on a mount"));
            return;
        }
        match self.world.my_entity() {
            None => {
                out.push(ignored("Cast", "our character is not in the world yet"));
                return;
            }
            Some(me) if me.dead => {
                out.push(ignored("Cast", "our character is dead"));
                return;
            }
            Some(me) if me.line.is_some() => {
                out.push(ignored("Cast", "our line is out already"));
                return;
            }
            Some(_) => {}
        }
        self.stop(now, out);
        let angle = angle % 360;
        let vid = self.world.me.as_ref().map(|me| me.vid);
        if let Some(me) = vid.and_then(|vid| self.world.entities.get_mut(&vid)) {
            me.angle = angle;
        }
        self.send_in_game("Cast", ClientMessage::Cast(msg::Cast { angle }), out);
    }

    /// A step of a player's fishing: its line from the cast to its end,
    /// and the event.
    fn on_fishing(&mut self, fishing: &msg::Fishing, out: &mut Vec<Effect>) {
        let Some(step) = FishingStep::from_wire(fishing.step) else {
            out.push(ignored("Fishing", "an unknown step"));
            return;
        };
        let Some(entity) = self.world.entities.get_mut(&fishing.vid) else {
            out.push(ignored("Fishing", "no such entity"));
            return;
        };
        let float = Point::new(fishing.x, fishing.y);
        entity.line = match step {
            FishingStep::Cast => Some(FishingLine {
                float,
                bitten: false,
            }),
            FishingStep::Bite => Some(FishingLine {
                float,
                bitten: true,
            }),
            FishingStep::ReeledIn | FishingStep::Caught | FishingStep::Lost => None,
        };
        if step == FishingStep::Cast {
            entity.angle = entity.track.to.angle_to(float);
        }
        out.push(Effect::Event(Event::Fishing {
            vid: fishing.vid,
            step,
            float,
        }));
    }

    /// What items raise on our character now.
    fn on_item_boosts(&mut self, now: Millis, boosts: &msg::ItemBoosts, out: &mut Vec<Effect>) {
        self.world.item_boosts = boosts
            .boosts
            .iter()
            .map(|boost| ItemBoost {
                item: boost.item,
                point: boost.point,
                amount: boost.amount,
                until: now.plus(u64::from(boost.seconds).saturating_mul(1_000)),
            })
            .collect();
        out.push(Effect::Event(Event::AffectsChanged));
    }

    fn on_affects(&mut self, now: Millis, affects: &msg::Affects, out: &mut Vec<Effect>) {
        self.world.affects = affects
            .affects
            .iter()
            .map(|affect| Affect {
                skill: affect.skill,
                point: affect.point,
                amount: affect.amount,
                until: (affect.seconds > 0)
                    .then(|| now.plus(u64::from(affect.seconds).saturating_mul(1_000))),
            })
            .collect();
        out.push(Effect::Event(Event::AffectsChanged));
    }

    /// An item we used offers skills to train (`skills.md`, "Training"),
    /// in place of any choice open.
    fn on_choose_skill(&mut self, choose: &msg::ChooseSkill, out: &mut Vec<Effect>) {
        let slot = choose.slot;
        let mut skills: Vec<u8> = Vec::new();
        for skill in choose.skills.iter().copied().filter(|skill| *skill != 0) {
            if !skills.contains(&skill) {
                skills.push(skill);
            }
        }
        self.world.skill_choice = Some(SkillChoice { slot, skills });
        out.push(Effect::Event(Event::SkillChoiceOpened { slot }));
    }

    /// Forgets the stable open; the server keeps nothing of it.
    fn close_stable(&mut self, out: &mut Vec<Effect>) {
        self.world.stable = None;
        out.push(Effect::Event(Event::StableClosed));
    }

    /// Forgets the upgrade offer open; the server keeps nothing of it.
    fn close_upgrade(&mut self, out: &mut Vec<Effect>) {
        self.world.upgrade = None;
        out.push(Effect::Event(Event::UpgradeClosed));
    }

    /// Closes the upgrade offer, if one is open (a shop, teacher or stable
    /// opened).
    fn close_any_upgrade(&mut self, out: &mut Vec<Effect>) {
        if self.world.upgrade.is_some() {
            self.close_upgrade(out);
        }
    }

    /// The server offered to upgrade an item of our bag (`upgrades.md`),
    /// in place of any offer open.
    fn on_upgrade_offer(&mut self, offer: &msg::UpgradeOffer, out: &mut Vec<Effect>) {
        if offer.slot >= BAG_SLOTS || (offer.smith.is_some() == offer.scroll.is_some()) {
            out.push(ignored(
                "UpgradeOffer",
                "not a bag slot, or not by one smith or scroll",
            ));
            return;
        }
        let slot = offer.slot;
        self.world.upgrade = Some(UpgradeOffer {
            smith: offer.smith,
            scroll: offer.scroll,
            slot,
            vnum: offer.vnum,
            next: offer.next,
            gold: offer.gold,
            items: offer
                .items
                .iter()
                .filter(|cost| cost.vnum != 0 && cost.count > 0)
                .map(|cost| UpgradeCost {
                    vnum: cost.vnum,
                    count: cost.count,
                })
                .collect(),
            chance: offer.chance.min(100),
            on_fail: UpgradeFailure::from_wire(offer.on_fail),
        });
        out.push(Effect::Event(Event::UpgradeOffered { slot }));
    }

    /// A stable showed what it trains (`mounts.md`), in place of any shop
    /// (which the server forgot) or teacher open; again after a training.
    fn on_stable_opened(&mut self, opened: msg::StableOpened, out: &mut Vec<Effect>) {
        if self.world.shop.take().is_some() {
            out.push(Effect::Event(Event::ShopClosed));
        }
        self.close_any_upgrade(out);
        if self.world.teacher.is_some() {
            self.close_teacher(out);
        }
        let next = opened.next.map(|offer| MountOffer {
            level: offer.level,
            grade: offer.grade,
            look: offer.look,
            move_speed: offer.move_speed,
            stamina: offer.stamina,
            rider_level: offer.rider_level,
            gold: offer.gold,
            items: offer
                .items
                .iter()
                .map(|cost| MountCost {
                    vnum: cost.vnum,
                    count: cost.count,
                })
                .collect(),
            skill_points: offer.skill_points,
        });
        let vid = opened.vid;
        self.world.stable = Some(Stable { vid, next });
        out.push(Effect::Event(Event::StableOpened { vid }));
    }

    /// A player got on or off its mount (`mounts.md`).
    fn on_entity_mount(&mut self, mount: msg::EntityMount, out: &mut Vec<Effect>) {
        let Some(entity) = self.world.entities.get_mut(&mount.vid) else {
            out.push(ignored("EntityMount", "no such entity"));
            return;
        };
        entity.mount = mount.look;
        out.push(Effect::Event(Event::MountChanged {
            vid: mount.vid,
            look: mount.look,
        }));
    }

    /// Our mount changed (`mounts.md`): level 0 is none.
    fn on_mount_state(&mut self, state: msg::MountState, out: &mut Vec<Effect>) {
        self.world.mount = (state.level > 0).then_some(Mount {
            level: state.level,
            grade: state.grade,
            look: state.look,
            stamina: state.stamina,
            most: state.most,
            ride_from: state.ride_from,
            fights: state.fights,
            skills: state.skills,
            skill_points: state.skill_points,
            summoned: state.summoned,
        });
        out.push(Effect::Event(Event::MountStateChanged));
    }

    /// Our character's run speed changed (`mounts.md`): the walk under way
    /// keeps its timing, the rest of a path takes the new speed.
    fn on_run_speed(&mut self, speed: msg::RunSpeed, _now: Millis, out: &mut Vec<Effect>) {
        let run_speed = speed.run_speed;
        self.world.run_speed = run_speed;
        if let Some(path) = &mut self.world.path {
            path.speed = run_speed;
        }
        out.push(Effect::Event(Event::RunSpeedChanged { run_speed }));
    }

    /// A teacher showed what it teaches (`skills.md`), in place of any shop
    /// open, which the server forgot, or stable.
    fn on_teacher_opened(&mut self, opened: &msg::TeacherOpened, out: &mut Vec<Effect>) {
        if self.world.shop.take().is_some() {
            out.push(Effect::Event(Event::ShopClosed));
        }
        self.close_any_upgrade(out);
        if self.world.stable.is_some() {
            self.close_stable(out);
        }
        let vid = opened.vid;
        self.world.teacher = Some(Teacher {
            vid,
            job: opened.job,
            group: opened.group,
            level: opened.level,
        });
        out.push(Effect::Event(Event::TeacherOpened { vid }));
    }

    /// A shop opened: what it sells, by place (`shops.md`). It takes the
    /// place of any teacher open.
    fn on_shop_opened(&mut self, opened: &msg::ShopOpened, out: &mut Vec<Effect>) {
        if self.world.teacher.is_some() {
            self.close_teacher(out);
        }
        self.close_any_upgrade(out);
        if self.world.stable.is_some() {
            self.close_stable(out);
        }
        let goods = opened
            .goods
            .iter()
            .filter(|good| good.position < SHOP_PLACES && good.vnum != 0)
            .map(|good| {
                let item = ShopGood {
                    vnum: good.vnum,
                    price: good.price,
                    count: good.count,
                };
                (good.position, item)
            })
            .collect();
        let vid = opened.vid;
        self.world.shop = Some(Shop { vid, goods });
        out.push(Effect::Event(Event::ShopOpened { vid }));
    }

    /// A whisper, checked as the server checks it (`chat.md`, "Whispers").
    fn whisper(&mut self, name: String, text: String, out: &mut Vec<Effect>) {
        if !self.names_someone_else("Whisper", &name, out) {
            return;
        }
        let why = if text.is_empty() {
            Some("the line is empty")
        } else if text.len() > CHAT_MAX {
            Some("the line is longer than 512 bytes")
        } else if text.chars().any(char::is_control) {
            Some("the line holds control characters")
        } else {
            None
        };
        if let Some(why) = why {
            out.push(ignored("Whisper", why));
            return;
        }
        let whisper = msg::SendWhisper { name, text };
        self.send_in_game("Whisper", ClientMessage::SendWhisper(whisper), out);
    }

    /// Whether `name` could be another player's character: else the
    /// command is ignored.
    fn names_someone_else(&self, what: &'static str, name: &str, out: &mut Vec<Effect>) -> bool {
        let own = self
            .world
            .me
            .as_ref()
            .is_some_and(|me| me.name.eq_ignore_ascii_case(name));
        if !valid_character_name(name) {
            out.push(ignored(what, "no character has that name"));
            false
        } else if own {
            out.push(ignored(what, "that is our own character"));
            false
        } else {
            true
        }
    }

    /// A whisper line, ours or to us, goes into its conversation.
    fn on_whisper(&mut self, now: Millis, whisper: msg::Whisper, out: &mut Vec<Effect>) {
        let line = WhisperLine {
            at: now,
            kind: if whisper.mine {
                WhisperKind::Sent
            } else {
                WhisperKind::Received
            },
            text: whisper.text,
        };
        self.world.push_whisper(&whisper.name, line.clone());
        out.push(Effect::Event(Event::Whisper {
            name: whisper.name,
            line,
        }));
    }

    /// Our whisper did not go out: a line in its conversation, if we have
    /// one with that name.
    fn on_whisper_refused(
        &mut self,
        now: Millis,
        refused: msg::WhisperRefused,
        out: &mut Vec<Effect>,
    ) {
        let reason = WhisperRefusal::from_wire(refused.reason);
        // The refusal names whom we wrote to as we wrote it: the
        // conversation keeps the server's spelling.
        if let Some(name) = self
            .world
            .conversation(&refused.name)
            .map(|c| c.name.clone())
        {
            let line = WhisperLine {
                at: now,
                kind: WhisperKind::Refused(reason),
                text: String::new(),
            };
            self.world.push_whisper(&name, line);
        }
        out.push(Effect::Event(Event::WhisperRefused {
            name: refused.name,
            reason,
        }));
    }

    /// Our friends, after entering: all of them.
    fn on_friends(&mut self, friends: msg::Friends, out: &mut Vec<Effect>) {
        self.world.friends = friends
            .friends
            .into_iter()
            .map(|friend| {
                let key = friend.name.to_ascii_lowercase();
                let friend = Friend {
                    name: friend.name,
                    online: friend.online,
                };
                (key, friend)
            })
            .collect();
        out.push(Effect::Event(Event::Friends));
    }

    /// A friend changed: added, gone, in or out of the game.
    fn on_friend_changed(&mut self, changed: msg::FriendChanged, out: &mut Vec<Effect>) {
        let key = changed.name.to_ascii_lowercase();
        let change = match changed.status {
            0 => {
                if self.world.friends.remove(&key).is_none() {
                    out.push(ignored("FriendChanged", "no such friend"));
                    return;
                }
                FriendChange::Removed
            }
            1 | 2 => {
                let online = changed.status == 2;
                let friend = Friend {
                    name: changed.name.clone(),
                    online,
                };
                match self.world.friends.insert(key, friend) {
                    None => FriendChange::Added { online },
                    Some(_) if online => FriendChange::Online,
                    Some(_) => FriendChange::Offline,
                }
            }
            _ => {
                out.push(ignored("FriendChanged", "an unknown status"));
                return;
            }
        };
        out.push(Effect::Event(Event::FriendChanged {
            name: changed.name,
            change,
        }));
    }

    /// A player asks to be our friend: it waits for our answer, the oldest
    /// going past [`FRIEND_REQUESTS`].
    fn on_friend_asked(&mut self, asked: msg::FriendAsked, out: &mut Vec<Effect>) {
        let requests = &mut self.world.friend_requests;
        requests.retain(|name| !name.eq_ignore_ascii_case(&asked.name));
        if requests.len() >= FRIEND_REQUESTS {
            requests.remove(0);
        }
        requests.push(asked.name.clone());
        out.push(Effect::Event(Event::FriendAsked { name: asked.name }));
    }

    /// What became of a friend request; one to us that ended is forgotten.
    fn on_friend_notice(&mut self, notice: msg::FriendNotice, out: &mut Vec<Effect>) {
        let what = FriendNotice::from_wire(notice.notice);
        if what == FriendNotice::Ended {
            let requests = &mut self.world.friend_requests;
            requests.retain(|name| !name.eq_ignore_ascii_case(&notice.name));
        }
        out.push(Effect::Event(Event::FriendNotice {
            name: notice.name,
            notice: what,
        }));
    }

    /// Sends a message for a command that needs to be in game.
    fn send_in_game(&mut self, what: &'static str, message: ClientMessage, out: &mut Vec<Effect>) {
        if self.stage == Stage::InGame {
            self.send(message, out);
        } else {
            out.push(ignored(what, "this needs to be in game"));
        }
    }

    fn on_message(
        &mut self,
        now: Millis,
        conn: ConnId,
        message: ServerMessage,
        out: &mut Vec<Effect>,
    ) {
        // Messages from a connection that is no longer current are dropped.
        let Some(link) = self.link_mut(conn) else {
            return;
        };
        link.last_heard = now;
        let server = link.server;
        match message {
            ServerMessage::Welcome(welcome) => self.on_welcome(server, welcome, out),
            ServerMessage::Refused(refused) => {
                let refusal = Refusal::of(&refused);
                let text = refused.text;
                self.fail(
                    SessionError::Refused {
                        server,
                        refusal,
                        text,
                    },
                    out,
                );
            }
            ServerMessage::Ping(_) => self.send(ClientMessage::Pong(msg::Pong), out),
            ServerMessage::Phase(p) => self.on_phase(server, Phase::from_wire(p.phase), out),
            ServerMessage::SignInOptions(options) => self.on_sign_in_options(server, options, out),
            ServerMessage::SignedIn(signed_in) => {
                if server == ServerKind::Auth && self.stage == Stage::Auth {
                    self.account = Some(signed_in.account);
                }
                self.on_signed_in(now, server, signed_in.ticket, out);
            }
            ServerMessage::LoginRefused(refused) => {
                let reason = LoginRefusal::from_wire(refused.reason);
                self.fail(SessionError::LoginRejected { server, reason }, out);
            }
            ServerMessage::Empire(e) => {
                self.world.empire = (e.empire != 0).then(|| Empire::from_wire(e.empire));
            }
            ServerMessage::CharacterList(list) => {
                self.on_character_list(server, &list.characters, out);
            }
            ServerMessage::CharacterCreated(created) => {
                self.on_character_created(server, &created.character, out);
            }
            ServerMessage::CharacterCreateFailed(failed) => {
                self.on_create_failed(server, failed.reason, out);
            }
            ServerMessage::MainCharacter(main) => self.on_main_character(&main, out),
            ServerMessage::GameTime(t) => {
                self.world.clock = Some(ServerClock {
                    time: t.time,
                    received: now,
                });
            }
            ServerMessage::Channel(c) => self.world.channel = Some(c.channel),
            ServerMessage::Chat(chat) => self.on_chat(now, chat, out),
            ServerMessage::EntitySpawn(spawn) => self.on_spawn(now, &spawn, out),
            ServerMessage::EntityRemove(remove) => self.on_remove(remove.vid, out),
            ServerMessage::EntityMove(movement) => self.on_entity_move(now, &movement, out),
            ServerMessage::MoveRefused(refused) => {
                self.on_move_refused(now, Point::new(refused.x, refused.y), out);
            }
            ServerMessage::EntityDied(died) => self.on_died(died.vid, out),
            ServerMessage::Projectile(shot) => {
                if self.world.entities.contains_key(&shot.shooter) {
                    out.push(Effect::Event(Event::Shot {
                        shooter: shot.shooter,
                        target: shot.target,
                        at: Point::new(shot.x, shot.y),
                    }));
                } else {
                    out.push(ignored("Projectile", "no such entity"));
                }
            }
            ServerMessage::Flight(flight) => {
                let seen = |vid| self.world.entities.contains_key(&vid);
                match FlightKind::from_wire(flight.kind) {
                    Some(kind) if seen(flight.from) && seen(flight.to) => {
                        out.push(Effect::Event(Event::Flight {
                            kind,
                            from: flight.from,
                            to: flight.to,
                        }));
                    }
                    Some(_) => out.push(ignored("Flight", "no such entity")),
                    None => out.push(ignored("Flight", "an unknown kind")),
                }
            }
            ServerMessage::EntityEffect(effect) => {
                let seen = self.world.entities.contains_key(&effect.vid);
                match EntityEffectKind::from_wire(effect.kind) {
                    Some(kind) if seen => out.push(Effect::Event(Event::EntityEffect {
                        kind,
                        vid: effect.vid,
                    })),
                    Some(_) => out.push(ignored("EntityEffect", "no such entity")),
                    None => out.push(ignored("EntityEffect", "an unknown kind")),
                }
            }
            ServerMessage::EntitySkills(skills) => match self.world.entities.get_mut(&skills.vid) {
                Some(entity) => entity.skills = skills.skills,
                None => out.push(ignored("EntitySkills", "no such entity")),
            },
            ServerMessage::TargetHealth(health) => match self.world.entities.get_mut(&health.vid) {
                Some(entity) => entity.health = Some(health.percent.min(100)),
                None => out.push(ignored("TargetHealth", "no such entity")),
            },
            ServerMessage::Damage(damage) => out.push(Effect::Event(Event::Damaged {
                vid: damage.vid,
                amount: damage.amount,
                flags: damage.flags,
            })),
            ServerMessage::EntityUpdate(update) => self.on_entity_update(&update, out),
            ServerMessage::EntityInfo(info) => self.on_entity_info(&info, out),
            ServerMessage::Points(points) => {
                self.world.points = points
                    .points
                    .iter()
                    .filter(|p| p.value != 0)
                    .map(|p| (p.point, p.value))
                    .collect();
            }
            ServerMessage::PointsChanged(changed) => {
                for p in changed.points {
                    if p.value == 0 {
                        self.world.points.remove(&p.point);
                    } else {
                        self.world.points.insert(p.point, p.value);
                    }
                }
            }
            ServerMessage::ItemSet(item) => {
                // The item that offered skills moved or went.
                let at_choice = self.world.skill_choice.as_ref().is_some_and(|choice| {
                    Window::from_wire(item.window) == Window::Inventory && choice.slot == item.slot
                });
                if at_choice {
                    self.close_skill_choice(out);
                }
                // The item or the scroll of the upgrade offer moved, changed
                // or went.
                let at_upgrade = self.world.upgrade.as_ref().is_some_and(|offer| {
                    Window::from_wire(item.window) == Window::Inventory
                        && (offer.slot == item.slot || offer.scroll == Some(item.slot))
                });
                if at_upgrade {
                    self.close_upgrade(out);
                }
                self.on_item_set(&item);
            }
            ServerMessage::GroundItemAdd(item) => {
                let at = Point::new(item.x, item.y);
                let ground = GroundItem {
                    vnum: item.vnum,
                    at,
                    owner: None,
                };
                self.world.ground.insert(item.vid, ground);
                out.push(Effect::Event(Event::GroundItemAppeared {
                    vid: item.vid,
                    vnum: item.vnum,
                    at,
                }));
            }
            ServerMessage::ItemOwnership(ownership) => {
                match self.world.ground.get_mut(&ownership.vid) {
                    Some(item) => {
                        let owner = ownership.owner;
                        item.owner = (!owner.is_empty()).then_some(owner);
                    }
                    None => out.push(ignored("ItemOwnership", "no such item on the ground")),
                }
            }
            ServerMessage::GroundItemRemove(removed) => {
                if self.world.ground.remove(&removed.vid).is_some() {
                    out.push(Effect::Event(Event::GroundItemLeft { vid: removed.vid }));
                } else {
                    out.push(ignored("GroundItemRemove", "no such item on the ground"));
                }
            }
            ServerMessage::ShopOpened(opened) => self.on_shop_opened(&opened, out),
            ServerMessage::ShopRefused(refused) => {
                let refusal = ShopRefusal::from_wire(refused.reason);
                out.push(Effect::Event(Event::ShopRefused(refusal)));
            }
            ServerMessage::QuickSlotSet(set) => {
                if set.index < QUICK_SLOTS {
                    let slot = QuickSlot {
                        kind: QuickSlotKind::from_wire(set.kind),
                        position: set.position,
                    };
                    self.world.quickslots.insert(set.index, slot);
                } else {
                    out.push(ignored("QuickSlotSet", "there are 32 quickslots"));
                }
            }
            ServerMessage::QuickSlotCleared(cleared) => {
                self.world.quickslots.remove(&cleared.index);
            }
            ServerMessage::QuickSlotsSwapped(swapped) => {
                let quickslots = &mut self.world.quickslots;
                let first = quickslots.remove(&swapped.first);
                let second = quickslots.remove(&swapped.second);
                quickslots.extend(first.map(|slot| (swapped.second, slot)));
                quickslots.extend(second.map(|slot| (swapped.first, slot)));
            }
            ServerMessage::Skills(skills) => self.on_skills(&skills.skills, out),
            ServerMessage::SkillGroup(group) => {
                self.world.skill_group = group.group;
                out.push(Effect::Event(Event::SkillGroupChanged {
                    group: group.group,
                }));
            }
            ServerMessage::TeacherOpened(opened) => self.on_teacher_opened(&opened, out),
            ServerMessage::ChooseSkill(choose) => self.on_choose_skill(&choose, out),
            ServerMessage::UpgradeOffer(offer) => self.on_upgrade_offer(&offer, out),
            ServerMessage::Whisper(whisper) => self.on_whisper(now, whisper, out),
            ServerMessage::WhisperRefused(refused) => self.on_whisper_refused(now, refused, out),
            ServerMessage::Friends(friends) => self.on_friends(friends, out),
            ServerMessage::FriendChanged(changed) => self.on_friend_changed(changed, out),
            ServerMessage::FriendAsked(asked) => self.on_friend_asked(asked, out),
            ServerMessage::FriendNotice(notice) => self.on_friend_notice(notice, out),
            ServerMessage::EntityMount(mount) => self.on_entity_mount(mount, out),
            ServerMessage::MountState(state) => self.on_mount_state(state, out),
            ServerMessage::RunSpeed(speed) => self.on_run_speed(speed, now, out),
            ServerMessage::StableOpened(opened) => self.on_stable_opened(opened, out),
            ServerMessage::Affects(affects) => self.on_affects(now, &affects, out),
            ServerMessage::Fishing(fishing) => self.on_fishing(&fishing, out),
            ServerMessage::ItemBoosts(boosts) => self.on_item_boosts(now, &boosts, out),
        }
    }

    /// The server speaks our protocol: we ask the auth server for its
    /// providers, and log in on the game server with the ticket
    /// (`session.md`).
    fn on_welcome(&mut self, server: ServerKind, welcome: msg::Welcome, out: &mut Vec<Effect>) {
        let Some(link) = &mut self.link else { return };
        if link.welcomed {
            out.push(ignored("Welcome", "the server welcomed us already"));
            return;
        }
        link.welcomed = true;
        out.push(Effect::Event(Event::Welcomed {
            server,
            protocol: welcome.protocol,
            name: welcome.server,
        }));
        match server {
            ServerKind::Auth => self.send_login(out),
            ServerKind::Game => self.send_ticket_login(out),
        }
    }

    /// The auth server's providers: a sign-in goes on with its provider; a
    /// session that only asked logs out.
    fn on_sign_in_options(
        &mut self,
        server: ServerKind,
        options: msg::SignInOptions,
        out: &mut Vec<Effect>,
    ) {
        if server != ServerKind::Auth || self.stage != Stage::Auth {
            out.push(ignored(
                "SignInOptions",
                "not logging in on the auth server",
            ));
            return;
        }
        let providers: Vec<SignInProvider> = options
            .providers
            .into_iter()
            .map(SignInProvider::from)
            .collect();
        out.push(Effect::Event(Event::SignInOptions(providers.clone())));
        match &self.config.credentials {
            Credentials::Options => {
                self.disconnect_current(out);
                self.stage = Stage::LoggedOut;
                out.push(Effect::Event(Event::LoggedOut));
            }
            Credentials::Provider(id) => {
                let Some(provider) = providers.iter().find(|p| &p.id == id).cloned() else {
                    let id = id.clone();
                    let offered = providers.into_iter().map(|p| p.id).collect();
                    self.fail(SessionError::NoProvider { id, offered }, out);
                    return;
                };
                self.stage = Stage::SigningIn;
                out.push(Effect::SignIn {
                    provider,
                    nonce: options.nonce,
                });
            }
        }
    }

    /// The provider's code: relayed to the auth server, which exchanges it.
    fn on_sign_in_code(&mut self, code: SignInCode, out: &mut Vec<Effect>) {
        let Credentials::Provider(provider) = &self.config.credentials else {
            out.push(ignored("SignInCode", "not signing in with a provider"));
            return;
        };
        if self.stage != Stage::SigningIn {
            out.push(ignored("SignInCode", "not waiting for a provider's code"));
            return;
        }
        let login = msg::ProviderLogin {
            provider: provider.clone(),
            code: code.code.expose_secret().to_owned(),
            verifier: code.verifier.expose_secret().to_owned(),
            redirect: code.redirect,
        };
        self.stage = Stage::Auth;
        self.send(ClientMessage::ProviderLogin(login), out);
    }

    fn on_phase(&mut self, server: ServerKind, phase: Phase, out: &mut Vec<Effect>) {
        if let Some(link) = &mut self.link {
            link.phase = Some(phase);
        }
        out.push(Effect::Event(Event::Phase { server, phase }));
        if matches!(phase, Phase::Select | Phase::Loading) {
            // A new map or the character list: the old world is gone. The
            // server sends the items and quickslots again (`inventory.md`).
            self.world.entities.clear();
            self.world.ground.clear();
            self.world.shop = None;
            self.world.teacher = None;
            self.world.skill_choice = None;
            self.world.stable = None;
            self.world.upgrade = None;
            self.world.mount = None;
            self.world.affects.clear();
            self.world.items.clear();
            self.world.quickslots.clear();
            self.world.skills.clear();
            self.world.target = None;
            // Whispers and friends are the character's: another may come.
            self.world.whispers.clear();
            self.world.friends.clear();
            self.world.friend_requests.clear();
            self.end_walk();
        }
        match (server, phase) {
            (ServerKind::Game, Phase::Select) if self.stage == Stage::Leaving => {
                // Back at the character list (`session.md`, "Changing
                // character"): our character is gone with the world, and the
                // next one is entered as the first was. Leaving lasts until
                // the list comes.
                self.world.me = None;
                self.world.selected = None;
                self.world.points.clear();
                if let Some(link) = &mut self.link {
                    link.enter_sent = false;
                }
                out.push(Effect::Event(Event::LeftGame));
            }
            (ServerKind::Game, Phase::Loading) => self.stage = Stage::Loading,
            (ServerKind::Game, Phase::Game) if self.stage != Stage::InGame => {
                self.stage = Stage::InGame;
                out.push(Effect::Event(Event::EnteredGame(self.world.me.clone())));
            }
            _ => {}
        }
    }

    fn send_login(&mut self, out: &mut Vec<Effect>) {
        if self.stage != Stage::Auth {
            let what = "the auth server welcomed us after the login";
            self.fail(SessionError::Internal { what }, out);
            return;
        }
        self.send(ClientMessage::StartSignIn(msg::StartSignIn), out);
    }

    fn on_signed_in(
        &mut self,
        now: Millis,
        server: ServerKind,
        ticket: Vec<u8>,
        out: &mut Vec<Effect>,
    ) {
        if server != ServerKind::Auth || self.stage != Stage::Auth {
            out.push(ignored("SignedIn", "not logging in on the auth server"));
            return;
        }
        self.ticket = Some(SecretSlice::from(ticket));
        out.push(Effect::Event(Event::LoggedIn));
        self.disconnect_current(out);
        self.stage = Stage::Joining;
        self.connect(now, ServerKind::Game, out);
    }

    fn send_ticket_login(&mut self, out: &mut Vec<Effect>) {
        let Some(ticket) = self.ticket.as_ref().map(|t| t.expose_secret().to_vec()) else {
            let what = "the game server welcomed us before the auth server gave a ticket";
            self.fail(SessionError::Internal { what }, out);
            return;
        };
        let Some(username) = self.account.clone() else {
            let what = "the game server welcomed us before the auth server named the account";
            self.fail(SessionError::Internal { what }, out);
            return;
        };
        let login = msg::TicketLogin { username, ticket };
        self.send(ClientMessage::TicketLogin(login), out);
    }

    fn on_character_list(
        &mut self,
        server: ServerKind,
        characters: &[msg::Character],
        out: &mut Vec<Effect>,
    ) {
        // After the login, or after leaving the game (`LeftGame` came).
        let leaving = self.stage == Stage::Leaving && self.world.me.is_none();
        if server != ServerKind::Game || !(self.stage == Stage::Joining || leaving) {
            out.push(ignored("CharacterList", "not selecting a character"));
            return;
        }
        self.stage = Stage::Joining;
        let mut list: Vec<CharacterSummary> = characters
            .iter()
            .filter(|c| c.slot < SLOTS)
            .map(summary)
            .collect();
        list.sort_by_key(|c| c.slot);
        list.dedup_by_key(|c| c.slot);
        self.world.characters = list;
        out.push(Effect::Event(Event::CharacterList(
            self.world.characters.clone(),
        )));
        let characters = &self.world.characters;
        let choice = match &self.config.character {
            CharacterChoice::First => characters.first(),
            CharacterChoice::Slot(slot) => characters.iter().find(|c| c.slot == *slot),
            CharacterChoice::Create(new) => characters.iter().find(|c| c.name == new.name),
            CharacterChoice::Ask => {
                self.stage = Stage::Selecting;
                return;
            }
        };
        if let Some(c) = choice {
            let (slot, name) = (c.slot, c.name.clone());
            self.select(slot, name, out);
            return;
        }
        match self.config.character.clone() {
            // Never here: asking returned above.
            CharacterChoice::Ask => {}
            CharacterChoice::First => self.fail(SessionError::NoCharacter { slot: None }, out),
            CharacterChoice::Slot(slot) => {
                self.fail(SessionError::NoCharacter { slot: Some(slot) }, out);
            }
            CharacterChoice::Create(new) => match self.free_slot() {
                Some(slot) => self.create(slot, new, out),
                None => {
                    let name = new.name;
                    self.fail(SessionError::SlotsFull { name }, out);
                }
            },
        }
    }

    /// The first slot of the character list without a character.
    fn free_slot(&self) -> Option<u8> {
        let characters = &self.world.characters;
        (0..SLOTS).find(|slot| characters.iter().all(|c| c.slot != *slot))
    }

    fn select(&mut self, slot: u8, name: String, out: &mut Vec<Effect>) {
        self.world.selected = Some(slot);
        out.push(Effect::Event(Event::Selected { slot, name }));
        self.send(
            ClientMessage::SelectCharacter(msg::SelectCharacter { slot }),
            out,
        );
    }

    /// Plays the character the user picked ([`CharacterChoice::Ask`]).
    fn select_asked(&mut self, slot: u8, out: &mut Vec<Effect>) {
        if self.stage != Stage::Selecting {
            out.push(ignored("Select", "not choosing a character"));
            return;
        }
        let Some(name) = self
            .world
            .characters
            .iter()
            .find(|c| c.slot == slot)
            .map(|c| c.name.clone())
        else {
            out.push(ignored("Select", "no character in that slot"));
            return;
        };
        self.stage = Stage::Joining;
        self.select(slot, name, out);
    }

    /// Creates the character the user asked for ([`CharacterChoice::Ask`]).
    fn create_asked(&mut self, new: NewCharacter, out: &mut Vec<Effect>) {
        if self.stage != Stage::Selecting {
            out.push(ignored("Create", "not choosing a character"));
            return;
        }
        match self.free_slot() {
            Some(slot) => self.create(slot, new, out),
            None => out.push(ignored("Create", "the account's four slots are full")),
        }
    }

    /// Asks for a new character in `slot` (`session.md`, "Characters").
    fn create(&mut self, slot: u8, new: NewCharacter, out: &mut Vec<Effect>) {
        // Only an account without an empire picks one: one without
        // characters, or a game master's (the server said 0).
        let empire = self.world.empire.is_none().then_some(new.empire);
        self.stage = Stage::Creating;
        out.push(Effect::Event(Event::Creating {
            name: new.name.clone(),
            class: new.class,
            empire,
        }));
        let message = msg::CreateCharacter {
            slot,
            name: new.name.clone(),
            class: new.class.to_wire(),
            empire: empire.map(Empire::to_wire),
        };
        self.send(ClientMessage::CreateCharacter(message), out);
        self.creating = Some(new);
    }

    /// Our character exists: we play it, or with [`CharacterChoice::Ask`]
    /// add it to the list and wait again.
    fn on_character_created(
        &mut self,
        server: ServerKind,
        created: &msg::Character,
        out: &mut Vec<Effect>,
    ) {
        if server != ServerKind::Game || self.stage != Stage::Creating {
            out.push(ignored("CharacterCreated", "not creating a character"));
            return;
        }
        if let Some(new) = self.creating.take()
            && self.world.characters.is_empty()
        {
            self.world.empire = Some(new.empire);
        }
        let character = summary(created);
        self.world.characters.retain(|c| c.slot != character.slot);
        self.world.characters.push(character.clone());
        self.world.characters.sort_by_key(|c| c.slot);
        out.push(Effect::Event(Event::Created(character.clone())));
        if self.config.character == CharacterChoice::Ask {
            self.stage = Stage::Selecting;
        } else {
            self.stage = Stage::Joining;
            self.select(character.slot, character.name, out);
        }
    }

    /// The server refused our character: the end of the session, or with
    /// [`CharacterChoice::Ask`] back to choosing.
    fn on_create_failed(&mut self, server: ServerKind, error: u8, out: &mut Vec<Effect>) {
        if server != ServerKind::Game || self.stage != Stage::Creating {
            out.push(ignored("CharacterCreateFailed", "not creating a character"));
            return;
        }
        self.creating = None;
        if self.config.character == CharacterChoice::Ask {
            self.stage = Stage::Selecting;
            out.push(Effect::Event(Event::CreateRefused { error }));
        } else {
            self.fail(SessionError::CreateRefused { error }, out);
        }
    }

    /// Our character arrives while loading; we then tell the server we are ready.
    fn on_main_character(&mut self, main: &msg::MainCharacter, out: &mut Vec<Effect>) {
        let class = u16::from(main.class);
        self.world.me = Some(Me {
            vid: main.vid,
            name: main.name.clone(),
            class,
            empire: Empire::from_wire(main.empire),
            position: (main.x, main.y, main.z),
        });
        self.world.skill_group = main.skill_group;
        self.world.run_speed = main.run_speed;
        // Until its spawn arrives, our character stands where the server
        // says. The time does not matter for a track that stands still.
        let at = Point::new(main.x, main.y);
        self.world.entities.insert(
            main.vid,
            Entity {
                vid: main.vid,
                kind: EntityKind::Player,
                race: class,
                track: Track::still(at, Millis(0)),
                angle: 0,
                z: main.z,
                action: MoveKind::Wait,
                argument: 0,
                move_speed: 100,
                attack_speed: 100,
                state: 0,
                affects: 0,
                parts: [0; 4],
                dead: false,
                health: None,
                info: None,
                mount: 0,
                skills: Vec::new(),
                line: None,
            },
        );
        let Some(link) = &mut self.link else { return };
        if link.server == ServerKind::Game && link.phase == Some(Phase::Loading) && !link.enter_sent
        {
            link.enter_sent = true;
            self.send(ClientMessage::EnterGame(msg::EnterGame), out);
        }
    }

    fn is_me(&self, vid: u32) -> bool {
        self.world.me.as_ref().is_some_and(|me| me.vid == vid)
    }

    /// An entity appears; one we already know is replaced (`entities.md`).
    fn on_spawn(&mut self, now: Millis, spawn: &msg::EntitySpawn, out: &mut Vec<Effect>) {
        let at = Point::new(spawn.x, spawn.y);
        if self.is_me(spawn.vid) {
            // The server put us here (entering, or a jump): a walk ends.
            self.end_walk();
        }
        let old = self.world.entities.remove(&spawn.vid);
        let entity = Entity {
            vid: spawn.vid,
            kind: EntityKind::from_wire(spawn.kind),
            race: spawn.race,
            track: Track::still(at, now),
            angle: spawn.angle % 360,
            z: spawn.z,
            action: MoveKind::Wait,
            argument: 0,
            move_speed: spawn.move_speed,
            attack_speed: spawn.attack_speed,
            state: spawn.state,
            affects: spawn.affects,
            dead: spawn.state & 1 != 0,
            health: None,
            // A spawn of an entity we know keeps what its info said, and
            // the skills on it.
            parts: old.as_ref().map_or([0; 4], |old| old.parts),
            skills: old
                .as_ref()
                .map(|old| old.skills.clone())
                .unwrap_or_default(),
            info: old.and_then(|old| old.info),
            // EntityMount follows the spawn of a player that rides.
            mount: 0,
            // Fishing follows the spawn of a player with a line out.
            line: None,
        };
        out.push(Effect::Event(Event::EntityAppeared {
            vid: entity.vid,
            kind: entity.kind,
            race: entity.race,
            at,
        }));
        self.world.entities.insert(spawn.vid, entity);
    }

    fn on_remove(&mut self, vid: u32, out: &mut Vec<Effect>) {
        if self.is_me(vid) {
            out.push(ignored("EntityRemove", "our own character stays"));
            return;
        }
        if self.world.entities.remove(&vid).is_some() {
            if self.world.target == Some(vid) {
                self.world.target = None;
            }
            out.push(Effect::Event(Event::EntityLeft { vid }));
            // Its shop or teaching goes with it.
            if self.world.shop.as_ref().is_some_and(|shop| shop.vid == vid) {
                self.close_shop(out);
            }
            if self.world.teacher.is_some_and(|teacher| teacher.vid == vid) {
                self.close_teacher(out);
            }
            if self
                .world
                .stable
                .as_ref()
                .is_some_and(|stable| stable.vid == vid)
            {
                self.close_stable(out);
            }
            if self
                .world
                .upgrade
                .as_ref()
                .is_some_and(|offer| offer.smith == Some(vid))
            {
                self.close_upgrade(out);
            }
        } else {
            out.push(ignored("EntityRemove", "no such entity"));
        }
    }

    /// An entity died (`combat.md`). Our own walk ends where we fell.
    fn on_died(&mut self, vid: u32, out: &mut Vec<Effect>) {
        let me = self.is_me(vid);
        let Some(entity) = self.world.entities.get_mut(&vid) else {
            out.push(ignored("EntityDied", "no such entity"));
            return;
        };
        entity.dead = true;
        if me {
            self.end_walk();
        }
        out.push(Effect::Event(Event::Died { vid }));
    }

    /// Another entity walks, stops or attacks (`movement.md`). The walk
    /// starts from where we last drew it, so it never jumps.
    fn on_entity_move(&mut self, now: Millis, movement: &msg::EntityMove, out: &mut Vec<Effect>) {
        if self.is_me(movement.vid) {
            out.push(ignored("EntityMove", "we lead our own character's moves"));
            return;
        }
        let Some(entity) = self.world.entities.get_mut(&movement.vid) else {
            out.push(ignored("EntityMove", "no such entity"));
            return;
        };
        let from = entity.track.point(now);
        entity.track = Track {
            from,
            to: Point::new(movement.x, movement.y),
            start: now,
            duration: u64::from(movement.duration),
        };
        entity.angle = movement.angle % 360;
        entity.action = MoveKind::from_wire(movement.kind);
        entity.argument = movement.argument;
    }

    /// The server refused our last move (`movement.md`, "Refused moves"):
    /// the walk and its path end, and our character stands where the
    /// server has it. Nothing goes back.
    fn on_move_refused(&mut self, now: Millis, at: Point, out: &mut Vec<Effect>) {
        let vid = self.world.me.as_ref().map(|me| me.vid);
        let Some(me) = vid.and_then(|vid| self.world.entities.get_mut(&vid)) else {
            out.push(ignored("MoveRefused", "our character is not in the world"));
            return;
        };
        me.track = Track::still(at, now);
        me.action = MoveKind::Wait;
        me.argument = 0;
        self.end_walk();
        out.push(Effect::Event(Event::MoveRefused(at)));
    }

    fn on_entity_update(&mut self, update: &msg::EntityUpdate, out: &mut Vec<Effect>) {
        let Some(entity) = self.world.entities.get_mut(&update.vid) else {
            out.push(ignored("EntityUpdate", "no such entity"));
            return;
        };
        entity.parts = update.parts;
        entity.move_speed = update.move_speed;
        entity.attack_speed = update.attack_speed;
        entity.state = update.state;
        entity.affects = update.affects;
        if let Some(info) = &mut entity.info {
            info.guild = update.guild;
        }
    }

    fn on_entity_info(&mut self, info: &msg::EntityInfo, out: &mut Vec<Effect>) {
        let Some(entity) = self.world.entities.get_mut(&info.vid) else {
            out.push(ignored("EntityInfo", "no such entity"));
            return;
        };
        entity.parts = info.parts;
        entity.info = Some(EntityInfo {
            name: info.name.clone(),
            empire: Empire::from_wire(info.empire),
            level: u32::from(info.level),
            guild: info.guild,
        });
    }

    /// An item goes into a slot; vnum or count 0 clears it (`inventory.md`).
    fn on_item_set(&mut self, item: &msg::ItemSet) {
        let key = (Window::from_wire(item.window), item.slot);
        if item.vnum == 0 || item.count == 0 {
            self.world.items.remove(&key);
            return;
        }
        let bonuses = item
            .bonuses
            .iter()
            .filter(|b| b.kind != 0)
            .map(|b| (b.kind, b.value))
            .collect();
        self.world.items.insert(
            key,
            Item {
                vnum: item.vnum,
                count: item.count,
                flags: item.flags,
                anti_flags: item.anti_flags,
                sockets: item.sockets,
                bonuses,
            },
        );
    }

    fn on_chat(&mut self, now: Millis, chat: msg::Chat, out: &mut Vec<Effect>) {
        let mine = chat.vid != 0 && self.world.me.as_ref().is_some_and(|me| me.vid == chat.vid);
        let line = ChatLine {
            at: now,
            kind: ChatKind::from_wire(chat.kind),
            vid: chat.vid,
            empire: Empire::from_wire(chat.empire),
            text: chat.text,
            mine,
        };
        self.world.push_chat(line.clone());
        out.push(Effect::Event(Event::Chat(line)));
    }
}

fn ignored(what: &'static str, why: &'static str) -> Effect {
    Effect::Event(Event::Ignored { what, why })
}

fn summary(character: &msg::Character) -> CharacterSummary {
    CharacterSummary {
        slot: character.slot,
        id: character.id,
        name: character.name.clone(),
        class: character.class,
        level: character.level,
        playtime: character.playtime,
        st: character.st,
        ht: character.ht,
        dx: character.dx,
        iq: character.iq,
        body_part: character.body_part,
        hair_part: character.hair_part,
        guild: character.guild.clone(),
    }
}
