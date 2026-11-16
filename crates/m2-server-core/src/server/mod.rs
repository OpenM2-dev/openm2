//! The server's state machine (`docs/specs/server/session.md`).
//!
//! [`Server::step`] takes one input (a connection accepted, a message, a
//! connection closed, a store answer, a tick) and returns the effects:
//! messages to send, connections to close, store requests, events to log.
//! Each part of the session has its own module: [`hello`] (versions),
//! [`auth`] (providers' sign-ins and tickets), [`select`] (the
//! character list,
//! creating and selecting),
//! [`world`] (players, moves, chat), [`mobs`] (monsters and NPCs),
//! [`combat`] (fights, death, restarts), [`points`] (health, experience,
//! levels), [`items`] (the bag, gear, quickslots) and [`ground`] (items on
//! the ground, drops).

mod auth;
mod combat;
mod effects;
mod fishing;
mod friends;
mod grid;
mod ground;
mod hello;
mod items;
mod masters;
mod mobs;
mod mounts;
mod points;
mod select;
mod shops;
mod skills;
mod training;
mod upgrades;
mod walk;
mod walls;
mod warps;
mod whispers;
mod world;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::IpAddr;
use std::sync::Arc;

use m2_content::Content;
use m2_protocol::profile::openm2::{ClientMessage, EntityRemove, Phase, Ping, ServerMessage};
use rand_chacha::ChaCha20Rng;
use rand_core::{Rng as _, SeedableRng as _};
use secrecy::SecretBox;

pub use auth::valid_provider_id;
pub use select::{MAX_CHARACTERS, valid_character_name};
pub use walk::{walk_duration, walk_time_percent};
pub use world::MAX_CHAT;

use self::effects::Toggle;
use self::friends::FriendAsk;
use self::grid::{Grid, GroupId};
use self::ground::GroundItem;
use self::items::{Bag, QUICKSLOTS, Quick};
use self::mobs::{Group, Mob};
use self::mounts::Mount;
use self::shops::OpenShop;
use self::skills::{Buff, Cast};
use self::walk::Walk;
use crate::config::Config;
use crate::sign_in::{Identity, SignIn, SignInRefusal};
use crate::store::{Account, AccountId, Character, RequestId, StoreAnswer, StoreRequest};
use crate::time::{ConnId, Millis};

/// The phase numbers of the Phase message (`docs/specs/protocol/openm2/`).
mod phase {
    pub const SELECT: u8 = 1;
    pub const LOADING: u8 = 2;
    pub const GAME: u8 = 3;
}

/// Which listener accepted a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Listener {
    /// The auth server: providers' sign-ins.
    Auth,
    /// The game server: the character list and the game.
    Game,
}

impl fmt::Display for Listener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Listener::Auth => "auth",
            Listener::Game => "game",
        })
    }
}

/// One input to [`Server::step`].
#[derive(Debug)]
pub enum Input {
    /// A listener accepted a connection from `peer`.
    Accepted {
        /// Its number.
        conn: ConnId,
        /// Which listener.
        listener: Listener,
        /// Where it comes from.
        peer: IpAddr,
    },
    /// A message arrived on a connection.
    Message(ConnId, ClientMessage),
    /// A connection closed (the client left, the network failed, or its bytes
    /// made no sense).
    Closed {
        /// Which.
        conn: ConnId,
        /// Why, for the log.
        reason: String,
    },
    /// Storage answered a request.
    Stored {
        /// The request's number.
        request: RequestId,
        /// The answer.
        answer: StoreAnswer,
    },
    /// The edge checked a provider's sign-in ([`Effect::CheckSignIn`]).
    SignInChecked {
        /// The request's number.
        request: RequestId,
        /// Who signed in, or why not.
        result: Result<Identity, SignInRefusal>,
    },
    /// Time passed: pings, timeouts, saves.
    Tick,
    /// The server is stopping: save every character and close every
    /// connection.
    Shutdown,
}

/// What the edge must do.
// Some messages are large (CharacterList); effects are short-lived, so they
// are not boxed.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Effect {
    /// Send a message on a connection.
    Send {
        /// Where.
        conn: ConnId,
        /// What.
        message: ServerMessage,
    },
    /// Close a connection. The core has forgotten it already; a later
    /// [`Input::Closed`] for it is ignored.
    Close {
        /// Which.
        conn: ConnId,
    },
    /// Ask storage; the answer comes back as [`Input::Stored`].
    Store {
        /// The request's number.
        request: RequestId,
        /// What to do.
        what: StoreRequest,
    },
    /// Exchange a provider's code and check its ID token; the answer comes
    /// back as [`Input::SignInChecked`].
    CheckSignIn {
        /// The request's number.
        request: RequestId,
        /// What to check.
        sign_in: SignIn,
    },
    /// Something to log or count. Events name accounts and characters by
    /// number only: never tickets, providers' codes or chat.
    Event(Event),
}

/// Something that happened, for logs and metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A connection was accepted.
    Accepted {
        /// Which.
        conn: ConnId,
        /// On which listener.
        listener: Listener,
    },
    /// A connection was refused: too many connections, or another
    /// version of the protocol.
    Refused {
        /// Which.
        conn: ConnId,
        /// Why.
        why: &'static str,
    },
    /// A client said which protocol it speaks.
    Hello {
        /// On which connection.
        conn: ConnId,
        /// The minor version it speaks.
        protocol: u16,
        /// Its build, control characters shown as `?`.
        build: String,
    },
    /// A provider's sign-in succeeded and a ticket was given out.
    SignedIn {
        /// On which connection.
        conn: ConnId,
        /// Whose.
        account: AccountId,
        /// With which provider.
        provider: String,
    },
    /// A sign-in or a ticket was refused.
    LoginRefused {
        /// On which connection.
        conn: ConnId,
        /// Why: the provider did not confirm it, too many failures, already
        /// in game.
        why: &'static str,
    },
    /// A ticket was accepted on the game server.
    Joined {
        /// On which connection.
        conn: ConnId,
        /// Whose.
        account: AccountId,
    },
    /// A character was created.
    CharacterCreated {
        /// On which connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
    },
    /// A character entered the game.
    Entered {
        /// On which connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// Its vid.
        vid: u32,
    },
    /// A character left the game for the character list (LeaveGame).
    Left {
        /// On which connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
    },
    /// A character died.
    Died {
        /// On which connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
    },
    /// A fish bit and the character reeled in, or let it go.
    Fished {
        /// On which connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// What it caught, if anything.
        item: Option<u32>,
    },
    /// A warp took a character to another place.
    Warped {
        /// On which connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// Where to, in world units.
        to: (i32, i32),
    },
    /// A character reached a new level.
    LevelledUp {
        /// On which connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// The level reached.
        level: u8,
    },
    /// A game master's command changed its character
    /// (`docs/specs/server/session.md`, "Game masters").
    GameMaster {
        /// On which connection.
        conn: ConnId,
        /// The game master's account.
        account: AccountId,
        /// The character's number.
        character: u32,
        /// The command, as the server understood it: `/level 30`.
        command: String,
    },
    /// Two characters became friends.
    FriendsMade {
        /// One character's number.
        a: u32,
        /// The other's.
        b: u32,
    },
    /// Two characters stopped being friends.
    FriendsParted {
        /// The character that ended it.
        a: u32,
        /// The other.
        b: u32,
    },
    /// A player spent a skill point.
    SkillRaised {
        /// Its connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// The skill.
        skill: u8,
        /// Its new level.
        level: u8,
    },
    /// A player spent a status point.
    StatRaised {
        /// Its connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// The stat's point number: 12 strength, 13 vitality, 14
        /// dexterity, 15 intelligence.
        point: u8,
        /// Its new value.
        value: u8,
    },
    /// A player trained its mount at a stable.
    MountTrained {
        /// Its connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// The mount's new level.
        level: u8,
    },
    /// A player trained a skill with a skill book or a soul stone.
    SkillTrained {
        /// Its connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// The skill.
        skill: u8,
        /// The book or stone used.
        item: u32,
        /// Whether it learned: the skill rose a level.
        learned: bool,
        /// The skill's level after.
        level: u8,
    },
    /// A player upgraded an item, at a smith or with a scroll.
    ItemUpgraded {
        /// Its connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// The item upgraded.
        item: u32,
        /// Whether it worked.
        worked: bool,
        /// The item in its place after: its next item, the one it went
        /// down to or itself; none when it broke.
        after: Option<u32>,
    },
    /// A player forgot its skills and its skill group.
    SkillsForgotten {
        /// Its connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
    },
    /// A player chose its skill group, at a teacher or with
    /// `/skillgroup`.
    SkillGroupChosen {
        /// Its connection.
        conn: ConnId,
        /// The character's number.
        character: u32,
        /// The group: 1 or 2.
        group: u8,
    },
    /// A stored item had no place in its character's bag and was left
    /// out (the content's item sizes changed, or storage held something
    /// wrong); the next save forgets it.
    ItemLeftOut {
        /// The character's number.
        character: u32,
        /// The item's number.
        vnum: u32,
        /// How many.
        count: u8,
    },
    /// A player's move was refused.
    MoveRefused {
        /// On which connection.
        conn: ConnId,
        /// Why: off its map, through a wall, out of reach.
        why: &'static str,
    },
    /// A message was ignored.
    Ignored {
        /// On which connection.
        conn: ConnId,
        /// The message's name.
        message: &'static str,
    },
    /// Storage failed.
    StoreFailed {
        /// What it was asked.
        what: &'static str,
        /// What it said.
        error: String,
    },
    /// A connection was closed.
    Closed {
        /// Which.
        conn: ConnId,
        /// Why.
        reason: String,
    },
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Accepted { conn, listener } => write!(f, "{conn}: accepted on {listener}"),
            Event::Refused { conn, why } => write!(f, "{conn}: refused: {why}"),
            Event::Hello {
                conn,
                protocol,
                build,
            } => write!(f, "{conn}: {build} speaks protocol {protocol}"),
            Event::SignedIn {
                conn,
                account,
                provider,
            } => write!(f, "{conn}: account {} signed in with {provider}", account.0),
            Event::LoginRefused { conn, why } => write!(f, "{conn}: login refused: {why}"),
            Event::Joined { conn, account } => {
                write!(f, "{conn}: account {} joined the game server", account.0)
            }
            Event::CharacterCreated { conn, character } => {
                write!(f, "{conn}: character {character} created")
            }
            Event::Entered {
                conn,
                character,
                vid,
            } => write!(f, "{conn}: character {character} entered as vid {vid}"),
            Event::Left { conn, character } => {
                write!(
                    f,
                    "{conn}: character {character} left for the character list"
                )
            }
            Event::Died { conn, character } => write!(f, "{conn}: character {character} died"),
            Event::Fished {
                conn,
                character,
                item: Some(item),
            } => write!(f, "{conn}: character {character} caught item {item}"),
            Event::Fished {
                conn,
                character,
                item: None,
            } => write!(f, "{conn}: character {character} caught nothing"),
            Event::Warped {
                conn,
                character,
                to: (x, y),
            } => write!(
                f,
                "{conn}: character {character} went through a warp to ({x}, {y})"
            ),
            Event::LevelledUp {
                conn,
                character,
                level,
            } => write!(f, "{conn}: character {character} reached level {level}"),
            Event::GameMaster {
                conn,
                account,
                character,
                command,
            } => write!(
                f,
                "{conn}: game master account {} on character {character}: {command}",
                account.0
            ),
            Event::FriendsMade { a, b } => write!(f, "characters {a} and {b} became friends"),
            Event::FriendsParted { a, b } => {
                write!(f, "character {a} is no longer friends with character {b}")
            }
            Event::SkillRaised {
                conn,
                character,
                skill,
                level,
            } => write!(
                f,
                "{conn}: character {character} raised skill {skill} to level {level}"
            ),
            Event::StatRaised {
                conn,
                character,
                point,
                value,
            } => write!(
                f,
                "{conn}: character {character} raised point {point} to {value}"
            ),
            Event::SkillGroupChosen {
                conn,
                character,
                group,
            } => write!(f, "{conn}: character {character} chose skill group {group}"),
            Event::MountTrained {
                conn,
                character,
                level,
            } => write!(
                f,
                "{conn}: character {character} trained its mount to level {level}"
            ),
            Event::SkillTrained {
                conn,
                character,
                skill,
                item,
                learned,
                level,
            } => write!(
                f,
                "{conn}: character {character} trained skill {skill} with item {item}: {}, level {level}",
                if *learned { "learned" } else { "not learned" }
            ),
            Event::ItemUpgraded {
                conn,
                character,
                item,
                worked,
                after,
            } => {
                let result = if *worked { "worked" } else { "failed" };
                match after {
                    Some(after) => write!(
                        f,
                        "{conn}: character {character} upgraded item {item}: {result}, item {after} now"
                    ),
                    None => write!(
                        f,
                        "{conn}: character {character} upgraded item {item}: {result}, it broke"
                    ),
                }
            }
            Event::SkillsForgotten { conn, character } => {
                write!(f, "{conn}: character {character} forgot its skills")
            }
            Event::ItemLeftOut {
                character,
                vnum,
                count,
            } => write!(
                f,
                "character {character}: {count} of item {vnum} had no place in its bag and were left out"
            ),
            Event::MoveRefused { conn, why } => write!(f, "{conn}: move refused: {why}"),
            Event::Ignored { conn, message } => write!(f, "{conn}: ignored {message}"),
            Event::StoreFailed { what, error } => write!(f, "storage failed to {what}: {error}"),
            Event::Closed { conn, reason } => write!(f, "{conn}: closed: {reason}"),
        }
    }
}

/// Where a connection is in the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Waiting for the client's Hello.
    Hello,
    /// Auth: waiting for StartSignIn and ProviderLogin.
    Login,
    /// Auth: the provider's sign-in is being checked.
    Checking,
    /// Auth: the ticket went out; the client closes next.
    LoggedIn,
    /// Game: waiting for TicketLogin.
    Ticket,
    /// Game: the ticket was accepted; the characters are being loaded.
    Joining,
    /// Game: the character list is shown.
    Select,
    /// Game: a character was picked; waiting for EnterGame.
    Loading,
    /// Game: in the world.
    Game,
}

/// A character picked on a connection, in the world once its stage is
/// [`Stage::Game`].
#[derive(Debug, Clone)]
struct Player {
    vid: u32,
    /// The character; its position is where it stands, or where its walk
    /// ends while it walks.
    character: Character,
    /// The walk under way, if any.
    walk: Option<Walk>,
    /// Where it was when it last looked around: what it sees is measured
    /// from here, and the grid keeps it here (`world.md`, "View").
    seen_from: (i32, i32),
    /// The direction it faces, in degrees.
    angle: u16,
    /// When it entered the game.
    entered: Millis,
    /// The time its last move said it was made at, or when the server last
    /// put it somewhere: a later move is never taken to be earlier.
    moved_at: Millis,
    /// The vids of what it sees (and, for players, what sees it).
    seen: BTreeSet<u32>,
    /// The points it was last told, by number (`Points`, then
    /// `PointsChanged` with what differs).
    told: BTreeMap<u8, u32>,
    /// Its maximum health and mana, from its class, level and stats.
    max_hp: u32,
    max_sp: u32,
    /// What it looks at (SetTarget).
    target: Option<u32>,
    /// The monsters fighting it.
    hunted_by: BTreeSet<u32>,
    /// When its health and mana come back next.
    next_regen: Millis,
    /// Its bag and what it wears; the character's stored items are moved
    /// here at selection and back at each save.
    bag: Bag,
    /// Its quickslots.
    quickslots: [Quick; QUICKSLOTS],
    /// When it dropped something, within the last
    /// [`Config::drop_window`].
    drops: Vec<Millis>,
    /// The shop it has open.
    shop: Option<OpenShop>,
    /// When each skill it used may be used again.
    cooldowns: BTreeMap<u8, Millis>,
    /// The attack skill it is using.
    cast: Option<Cast>,
    /// What its skills raise for a while.
    buffs: Vec<Buff>,
    /// Its mount while it is in the game (`mounts.rs`).
    mount: Mount,
    /// Its skills that stay on, by number.
    toggles: BTreeMap<u8, Toggle>,
    /// The skills on it the players that see it were last told of
    /// (EntitySkills).
    skills_shown: Vec<u8>,
    /// Its fishing line, while one is out.
    line: Option<fishing::Line>,
    /// Whether it was last told of points items raise (ItemBoosts).
    boosts_told: bool,
    /// A game master's own movement speed (`/speed`, 1 to
    /// [`masters::MAX_SPEED`]), over what its items, skills and mount give;
    /// not saved.
    speed: Option<u16>,
}

impl Player {
    /// Where the character is at `now`: along its walk, or where it stands.
    fn position(&self, now: Millis) -> (i32, i32) {
        walk::position_at((self.character.x, self.character.y), self.walk, now)
    }

    /// Where its walk ends, or where it stands.
    fn at(&self) -> (i32, i32) {
        (self.character.x, self.character.y)
    }
}

/// One connection.
#[derive(Debug)]
struct Conn {
    listener: Listener,
    peer: IpAddr,
    opened: Millis,
    /// When the client last sent something.
    heard: Millis,
    stage: Stage,
    next_ping: Option<Millis>,
    /// The nonce given out with SignInOptions, until a sign-in uses it.
    nonce: Option<String>,
    /// When an auth connection that asked for the providers must have
    /// signed in.
    sign_in_by: Option<Millis>,
    /// The provider of the sign-in being checked.
    provider: Option<String>,
    account: Option<Account>,
    characters: Vec<Character>,
    /// The store request this connection waits for.
    waiting: Option<RequestId>,
    player: Option<Player>,
    /// When it whispered, within the last [`Config::whisper_window`].
    whispered_at: Vec<Millis>,
    /// When it asked someone to be its friend, within the last
    /// [`Config::friend_ask_window`].
    asked_at: Vec<Millis>,
}

/// Who waits for a store request's answer.
#[derive(Debug, Clone, Copy)]
enum Waiter {
    /// A connection: its login, its characters.
    Conn(ConnId),
    /// Nobody: a save of a character of this account.
    Save(AccountId),
    /// Nobody: a friendship between characters of these two accounts
    /// written; a login of either waits for it as for a save.
    Friendship([AccountId; 2]),
}

/// A ticket given out and not used yet.
#[derive(Debug)]
struct IssuedTicket {
    account: Account,
    ticket: SecretBox<[u8; TICKET_LEN]>,
    expires: Millis,
}

/// The length of a ticket, in bytes.
const TICKET_LEN: usize = 32;

/// The server: every connection, the tickets, the players in game.
#[derive(Debug)]
pub struct Server {
    config: Config,
    rng: ChaCha20Rng,
    conns: BTreeMap<ConnId, Conn>,
    /// Who waits for each store request.
    requests: BTreeMap<RequestId, Waiter>,
    /// The saves under way, by account.
    saving: BTreeMap<AccountId, u32>,
    /// Connections waiting for their account's saves to land before its
    /// characters are loaded.
    deferred: BTreeMap<AccountId, Vec<ConnId>>,
    next_request: u64,
    next_vid: u32,
    /// The tickets given out, by account name in lowercase: one each.
    tickets: BTreeMap<String, IssuedTicket>,
    /// The game connection of each account that joined.
    online: BTreeMap<AccountId, ConnId>,
    /// The players in the game, by their character's name in lowercase.
    names: BTreeMap<String, ConnId>,
    /// The friend requests waiting for their answers, by the asked
    /// character's number and the asker's.
    friend_asks: BTreeMap<(u32, u32), FriendAsk>,
    /// Failed sign-ins and tickets, by where they came from.
    failures: BTreeMap<IpAddr, Vec<Millis>>,
    next_save: Millis,
    /// Every vid in use and what holds it.
    vids: BTreeMap<u32, Holder>,
    /// The players, monsters and groups in game, by where they stand.
    grid: Grid,
    /// What the world holds.
    content: Arc<Content>,
    /// The world's random numbers: a stream of its own, so monsters never
    /// change the tickets.
    world_rng: ChaCha20Rng,
    /// The monsters and NPCs, by vid.
    mobs: BTreeMap<u32, Mob>,
    /// The groups, by [`GroupId`].
    groups: Vec<Group>,
    /// Places of `groups` free again: the groups stones called that are
    /// gone.
    spare_groups: Vec<GroupId>,
    /// The groups a player is near.
    awake: BTreeSet<GroupId>,
    /// When each monster takes its next step: a stroll, a step of a fight,
    /// coming home, its removal.
    timers: BTreeSet<(Millis, u32)>,
    /// When each group whose monsters are all gone comes back.
    group_timers: BTreeSet<(Millis, GroupId)>,
    /// The monsters with states on them.
    afflicted: BTreeSet<u32>,
    /// The warps and gotos that take players somewhere, by vid.
    warps: BTreeMap<u32, warps::Warp>,
    /// When the warps next look around.
    next_warps: Millis,
    /// The players with a fishing line out.
    fishers: BTreeSet<ConnId>,
    /// The fires players lit, by player.
    fires: BTreeMap<ConnId, fishing::LitFire>,
    /// The items on the ground, by vid.
    ground: BTreeMap<u32, GroundItem>,
    /// When ground items stop being owned and when they vanish.
    ground_timers: BTreeSet<(Millis, u32)>,
    /// The Unix time, in seconds, at time 0: 0 unless the
    /// edge passes its clock with [`Server::started_at`].
    epoch: u64,
    out: Vec<Effect>,
}

/// What a vid belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holder {
    /// The character picked on a connection (from selection on).
    Player(ConnId),
    /// A monster or NPC.
    Mob,
    /// An item on the ground.
    Ground,
}

impl Server {
    /// A server with these rules and an empty world. `seed` seeds the random
    /// generator of the tickets: the edge takes it from the operating
    /// system, tests use a fixed one.
    pub fn new(config: Config, seed: [u8; 32]) -> Self {
        Self::with_content(config, Arc::new(Content::default()), seed)
    }

    /// A server with these rules and the content's monsters and NPCs in its
    /// world. The world's random numbers come from `seed` too, on a stream
    /// of their own.
    pub fn with_content(config: Config, content: Arc<Content>, seed: [u8; 32]) -> Self {
        let next_save = Millis(config.save_every);
        let grid = Grid::new(config.view_radius);
        let mut world_rng = ChaCha20Rng::from_seed(seed);
        world_rng.set_stream(1);
        let mut server = Self {
            config,
            rng: ChaCha20Rng::from_seed(seed),
            conns: BTreeMap::new(),
            requests: BTreeMap::new(),
            saving: BTreeMap::new(),
            deferred: BTreeMap::new(),
            next_request: 0,
            next_vid: 0,
            tickets: BTreeMap::new(),
            online: BTreeMap::new(),
            names: BTreeMap::new(),
            friend_asks: BTreeMap::new(),
            failures: BTreeMap::new(),
            next_save,
            vids: BTreeMap::new(),
            grid,
            content,
            world_rng,
            mobs: BTreeMap::new(),
            groups: Vec::new(),
            spare_groups: Vec::new(),
            awake: BTreeSet::new(),
            timers: BTreeSet::new(),
            group_timers: BTreeSet::new(),
            afflicted: BTreeSet::new(),
            warps: BTreeMap::new(),
            next_warps: Millis(0),
            fishers: BTreeSet::new(),
            fires: BTreeMap::new(),
            ground: BTreeMap::new(),
            ground_timers: BTreeSet::new(),
            epoch: 0,
            out: Vec::new(),
        };
        server.populate();
        server
    }

    /// The same server, with `epoch` as the Unix time, in seconds, at its
    /// time 0: the edge reads the system's clock when it starts the
    /// server, tests pass a fixed time. Waits that outlive the server
    /// (skill training) are kept in this time.
    #[must_use]
    pub fn started_at(mut self, epoch: u64) -> Self {
        self.epoch = epoch;
        self
    }

    /// The rules it runs with.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// How many connections are open.
    pub fn connections(&self) -> usize {
        self.conns.len()
    }

    /// How many players are in game.
    pub fn players(&self) -> usize {
        self.conns
            .values()
            .filter(|conn| conn.stage == Stage::Game)
            .count()
    }

    /// Handles one input at time `now` and returns what to do.
    pub fn step(&mut self, now: Millis, input: Input) -> Vec<Effect> {
        match input {
            Input::Accepted {
                conn,
                listener,
                peer,
            } => self.accept(now, conn, listener, peer),
            Input::Message(conn, message) => self.message(now, conn, message),
            Input::Closed { conn, reason } => self.forget(now, conn, reason),
            Input::Stored { request, answer } => self.stored(now, request, answer),
            Input::SignInChecked { request, result } => self.sign_in_checked(now, request, result),
            Input::Tick => self.tick(now),
            Input::Shutdown => {
                let conns: Vec<ConnId> = self.conns.keys().copied().collect();
                for conn in conns {
                    self.close(now, conn, "the server is stopping");
                }
            }
        }
        std::mem::take(&mut self.out)
    }

    fn accept(&mut self, now: Millis, conn: ConnId, listener: Listener, peer: IpAddr) {
        if self.conns.contains_key(&conn) {
            return;
        }
        let from_peer = self.conns.values().filter(|c| c.peer == peer).count();
        let refusal = if self.conns.len() >= self.config.max_connections {
            Some(hello::FULL)
        } else if from_peer >= self.config.max_per_address {
            Some(hello::PER_ADDRESS)
        } else {
            None
        };
        if let Some(reason) = refusal {
            self.event(Event::Refused {
                conn,
                why: "too many connections",
            });
            self.send(conn, hello::refused(reason));
            self.out.push(Effect::Close { conn });
            return;
        }
        self.conns.insert(
            conn,
            Conn {
                listener,
                peer,
                opened: now,
                heard: now,
                stage: Stage::Hello,
                next_ping: None,
                nonce: None,
                sign_in_by: None,
                provider: None,
                account: None,
                characters: Vec::new(),
                waiting: None,
                player: None,
                whispered_at: Vec::new(),
                asked_at: Vec::new(),
            },
        );
        self.event(Event::Accepted { conn, listener });
    }

    fn message(&mut self, now: Millis, conn: ConnId, message: ClientMessage) {
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        state.heard = now;
        if state.stage == Stage::Hello {
            match message {
                ClientMessage::Hello(hello) => self.hello(now, conn, hello),
                _ => self.close(now, conn, "a message before Hello"),
            }
            return;
        }
        match message {
            ClientMessage::Hello(_) => self.close(now, conn, "a Hello after the Hello"),
            ClientMessage::TicketLogin(login) => self.ticket_login(now, conn, login),
            ClientMessage::CreateCharacter(create) => self.create(now, conn, create),
            ClientMessage::SelectCharacter(select) => self.select(now, conn, select.slot),
            ClientMessage::EnterGame(_) => self.enter(now, conn),
            ClientMessage::LeaveGame(_) => self.leave_game(now, conn),
            ClientMessage::CharacterMove(m) => self.moved(now, conn, m),
            ClientMessage::SendChat(chat) => self.chat(now, conn, chat),
            ClientMessage::SendWhisper(whisper) => self.whisper(now, conn, whisper),
            ClientMessage::AskFriend(ask) => self.ask_friend(now, conn, ask),
            ClientMessage::AnswerFriend(answer) => self.answer_friend(now, conn, answer),
            ClientMessage::RemoveFriend(remove) => self.remove_friend(now, conn, remove),
            ClientMessage::Pong(_) => {}
            ClientMessage::SetTarget(target) => self.set_target(now, conn, target),
            ClientMessage::Hit(hit) => self.hit(now, conn, hit),
            ClientMessage::UseItem(message) => self.use_item(now, conn, message),
            ClientMessage::MoveItem(message) => self.move_item(now, conn, message),
            ClientMessage::PickUp(message) => self.pick_up(now, conn, message),
            ClientMessage::SetQuickSlot(message) => self.set_quickslot(now, conn, message),
            ClientMessage::ClearQuickSlot(message) => self.clear_quickslot(now, conn, message),
            ClientMessage::SwapQuickSlots(message) => self.swap_quickslots(now, conn, message),
            ClientMessage::DropItem(message) => self.drop_item(now, conn, message),
            ClientMessage::DropGold(message) => self.drop_gold(now, conn, message.amount),
            ClientMessage::Talk(talk) => self.talk(now, conn, talk),
            ClientMessage::ShopClose(_) => self.shop_close(now, conn),
            ClientMessage::ShopBuy(buy) => self.shop_buy(now, conn, buy),
            ClientMessage::ShopSell(sell) => self.shop_sell(now, conn, sell),
            ClientMessage::UseSkill(message) => self.use_skill(now, conn, message),
            ClientMessage::RaiseSkill(raise) => self.raise_skill(now, conn, raise.skill),
            ClientMessage::RaiseStatus(raise) => self.raise_status(now, conn, raise.point),
            ClientMessage::LearnSkillGroup(learn) => self.learn_skill_group(now, conn, learn.vid),
            ClientMessage::UseItemOnSkill(used) => self.use_item_on_skill(now, conn, used),
            ClientMessage::Restart(restart) => self.restart_asked(now, conn, restart.town),
            ClientMessage::RideMount(ride) => self.ride_mount(now, conn, ride),
            ClientMessage::TrainMount(train) => self.train_mount(now, conn, train),
            ClientMessage::GiveItem(ask) => self.give_item(now, conn, ask),
            ClientMessage::UseItemOnItem(used) => self.use_item_on_item(now, conn, used),
            ClientMessage::Upgrade(upgrade) => self.upgrade(now, conn, upgrade),
            ClientMessage::Cast(cast) => self.cast(now, conn, cast),
            ClientMessage::ReelIn(_) => self.reel_in(now, conn),
            ClientMessage::StartSignIn(_) => self.start_sign_in(now, conn),
            ClientMessage::ProviderLogin(login) => self.provider_login(now, conn, login),
        }
    }

    fn stored(&mut self, now: Millis, request: RequestId, answer: StoreAnswer) {
        let Some(waiter) = self.requests.remove(&request) else {
            return;
        };
        let conn = match waiter {
            Waiter::Conn(conn) => conn,
            Waiter::Save(account) => {
                if let StoreAnswer::Saved(Err(error)) = answer {
                    self.event(Event::StoreFailed {
                        what: "save a character",
                        error: error.0,
                    });
                }
                self.saved(account);
                return;
            }
            Waiter::Friendship([first, second]) => {
                if let StoreAnswer::FriendshipWritten(Err(error)) = answer {
                    self.event(Event::StoreFailed {
                        what: "write a friendship",
                        error: error.0,
                    });
                }
                self.saved(first);
                if second != first {
                    self.saved(second);
                }
                return;
            }
        };
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        if state.waiting != Some(request) {
            return;
        }
        state.waiting = None;
        match answer {
            StoreAnswer::LoginChecked(result) => self.login_checked(now, conn, result),
            StoreAnswer::Characters(result) => self.characters_loaded(now, conn, result),
            StoreAnswer::CharacterCreated(result) => self.created(now, conn, result),
            StoreAnswer::Saved(_) | StoreAnswer::FriendshipWritten(_) => {}
        }
    }

    fn tick(&mut self, now: Millis) {
        self.tickets.retain(|_, issued| issued.expires > now);
        let window = self.config.failure_window;
        for times in self.failures.values_mut() {
            times.retain(|at| now.since(*at) < window);
        }
        self.failures.retain(|_, times| !times.is_empty());

        let conns: Vec<ConnId> = self.conns.keys().copied().collect();
        for conn in conns {
            let Some(state) = self.conns.get_mut(&conn) else {
                continue;
            };
            if state.stage == Stage::Hello && now.since(state.opened) > self.config.hello_timeout {
                self.close(now, conn, "no Hello in time");
            } else if state.sign_in_by.is_some_and(|by| now >= by) {
                self.close(now, conn, "no sign-in in time");
            } else if now.since(state.heard) > self.config.idle_timeout {
                self.close(now, conn, "the client went silent");
            } else if state.next_ping.is_some_and(|at| now >= at) {
                state.next_ping = Some(now.plus(self.config.ping_every));
                self.send(conn, ServerMessage::Ping(Ping));
            }
        }

        if now >= self.next_save {
            self.next_save = now.plus(self.config.save_every);
            let playing: Vec<ConnId> = self
                .conns
                .iter()
                .filter(|(_, state)| state.stage == Stage::Game)
                .map(|(conn, _)| *conn)
                .collect();
            for conn in playing {
                self.save(now, conn);
            }
        }
        self.expire_friend_asks(now);
        self.regenerate(now);
        self.mount_tick(now);
        self.end_buffs(now);
        self.effects_tick(now);
        self.follow_walkers(now);
        self.warp_tick(now);
        self.fishing_tick(now);
        self.fires_tick(now);
        self.world_tick(now);
        self.ground_tick(now);
    }

    /// Closes a connection: the edge closes the socket, the core forgets it at
    /// once.
    fn close(&mut self, now: Millis, conn: ConnId, why: &str) {
        if self.conns.contains_key(&conn) {
            self.out.push(Effect::Close { conn });
            self.forget(now, conn, why.to_owned());
        }
    }

    /// Forgets a connection: its player leaves the world and is saved, its
    /// account is free to log in again.
    fn forget(&mut self, now: Millis, conn: ConnId, reason: String) {
        if self.stage(conn) == Some(Stage::Game) {
            self.save(now, conn);
            self.leave_world(conn, now);
        }
        let Some(state) = self.conns.remove(&conn) else {
            return;
        };
        if let Some(player) = &state.player {
            self.vids.remove(&player.vid);
        }
        if let Some(account) = &state.account
            && self.online.get(&account.id) == Some(&conn)
        {
            self.online.remove(&account.id);
        }
        self.event(Event::Closed { conn, reason });
    }

    /// Takes a player out of the world: the players that saw it see it go,
    /// the monsters fighting it give it up.
    fn leave_world(&mut self, conn: ConnId, now: Millis) {
        self.mount_left(conn);
        self.friends_left(conn);
        if let Some(name) = self
            .player(conn)
            .map(|p| p.character.name.to_ascii_lowercase())
            && self.names.get(&name) == Some(&conn)
        {
            self.names.remove(&name);
        }
        self.release_hunters(conn, now);
        let watchers = self.watchers_of(conn);
        let Some((vid, at, seen)) = self
            .conns
            .get_mut(&conn)
            .and_then(|state| state.player.as_mut())
            .map(|player| {
                (
                    player.vid,
                    player.seen_from,
                    std::mem::take(&mut player.seen),
                )
            })
        else {
            return;
        };
        self.grid.remove_player(conn, at);
        for seen in seen {
            self.forget_watcher(conn, seen);
            self.forget_ground_watcher(conn, seen);
        }
        for other in watchers {
            if let Some(player) = self
                .conns
                .get_mut(&other)
                .and_then(|state| state.player.as_mut())
            {
                player.seen.remove(&vid);
                if player.target == Some(vid) {
                    player.target = None;
                }
            }
            self.send(other, ServerMessage::EntityRemove(EntityRemove { vid }));
        }
    }

    /// The players that see the player on `conn`, in connection order.
    fn watchers_of(&self, conn: ConnId) -> BTreeSet<ConnId> {
        let Some(player) = self.conns.get(&conn).and_then(|s| s.player.as_ref()) else {
            return BTreeSet::new();
        };
        player
            .seen
            .iter()
            .filter_map(|vid| match self.vids.get(vid) {
                Some(Holder::Player(other)) => Some(*other),
                Some(Holder::Mob | Holder::Ground) | None => None,
            })
            .collect()
    }

    /// Saves a player in game: where it stands, how long it has played,
    /// its points, items and quickslots; each save counts one more.
    pub(super) fn save(&mut self, now: Millis, conn: ConnId) {
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        let (Some(account), Some(player)) = (&state.account, state.player.as_mut()) else {
            return;
        };
        player.character.saves = player.character.saves.saturating_add(1);
        let played = now.since(player.entered) / 1000;
        player.character.playtime = player.character.playtime.saturating_add(played);
        player.entered = player.entered.plus(played * 1000);
        // Mid-walk, the point it has reached, not where the walk ends.
        let (x, y) = player.position(now);
        let account = account.id;
        let what = StoreRequest::SaveCharacter {
            account,
            character: Character {
                x,
                y,
                items: player.bag.stored(),
                quickslots: items::stored_quickslots(&player.quickslots),
                ..player.character.clone()
            },
        };
        *self.saving.entry(account).or_default() += 1;
        self.next_request += 1;
        let request = RequestId(self.next_request);
        self.requests.insert(request, Waiter::Save(account));
        self.out.push(Effect::Store { request, what });
    }

    /// A save of `account` landed: once none is under way, the connections
    /// waiting for its characters get them.
    fn saved(&mut self, account: AccountId) {
        let Some(count) = self.saving.get_mut(&account) else {
            return;
        };
        *count = count.saturating_sub(1);
        if *count > 0 {
            return;
        }
        self.saving.remove(&account);
        for conn in self.deferred.remove(&account).unwrap_or_default() {
            let joining = self.conns.get(&conn).is_some_and(|state| {
                state.stage == Stage::Joining
                    && state.waiting.is_none()
                    && state.account.as_ref().is_some_and(|a| a.id == account)
            });
            if joining {
                self.request(conn, StoreRequest::LoadCharacters { account });
            }
        }
    }

    /// Loads `account`'s characters for `conn`, after any save of that
    /// account under way: the last session's save lands first.
    fn load_characters(&mut self, conn: ConnId, account: AccountId) {
        if self.saving.contains_key(&account) {
            self.deferred.entry(account).or_default().push(conn);
            return;
        }
        self.request(conn, StoreRequest::LoadCharacters { account });
    }

    /// Asks storage on behalf of `conn`, which waits for the answer.
    fn request(&mut self, conn: ConnId, what: StoreRequest) {
        self.next_request += 1;
        let request = RequestId(self.next_request);
        self.requests.insert(request, Waiter::Conn(conn));
        if let Some(state) = self.conns.get_mut(&conn) {
            state.waiting = Some(request);
        }
        self.out.push(Effect::Store { request, what });
    }

    /// Asks the edge to check a provider's sign-in for `conn`.
    fn check_sign_in(&mut self, conn: ConnId, sign_in: SignIn) {
        self.next_request += 1;
        let request = RequestId(self.next_request);
        self.requests.insert(request, Waiter::Conn(conn));
        if let Some(state) = self.conns.get_mut(&conn) {
            state.waiting = Some(request);
        }
        self.out.push(Effect::CheckSignIn { request, sign_in });
    }

    fn stage(&self, conn: ConnId) -> Option<Stage> {
        self.conns.get(&conn).map(|state| state.stage)
    }

    fn set_stage(&mut self, conn: ConnId, stage: Stage) {
        if let Some(state) = self.conns.get_mut(&conn) {
            state.stage = stage;
        }
    }

    fn send(&mut self, conn: ConnId, message: ServerMessage) {
        self.out.push(Effect::Send { conn, message });
    }

    fn send_phase(&mut self, conn: ConnId, phase: u8) {
        self.send(conn, ServerMessage::Phase(Phase { phase }));
    }

    fn event(&mut self, event: Event) {
        self.out.push(Effect::Event(event));
    }

    /// A sign-in's nonce: 16 random bytes from the tickets' generator, in
    /// hex.
    fn random_nonce(&mut self) -> String {
        let mut bytes = [0u8; 16];
        self.rng.fill_bytes(&mut bytes);
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// 32 random bytes from the tickets' generator.
    fn random_ticket(&mut self) -> [u8; TICKET_LEN] {
        let mut ticket = [0; TICKET_LEN];
        self.rng.fill_bytes(&mut ticket);
        ticket
    }

    /// The next vid not in use, now held by `holder`. Vids start at 1 and
    /// wrap past `u32::MAX`; players and monsters share them.
    fn take_vid(&mut self, holder: Holder) -> u32 {
        loop {
            self.next_vid = self.next_vid.wrapping_add(1);
            if self.next_vid != 0 && !self.vids.contains_key(&self.next_vid) {
                self.vids.insert(self.next_vid, holder);
                return self.next_vid;
            }
        }
    }
}
