//! `openm2 bot`: a headless client that logs in, creates its character if
//! asked, enters the game, says its lines, uses and drops the items it is
//! told to, fights a monster if told to, picks up what lies at its feet if
//! told to, buys and sells in an NPC's shop if told to, learns a teacher's
//! skill group if told to, levels and uses skills if told to, whispers and
//! makes or ends friendships if told to, walks where it is told, stays a
//! while and logs out.
//! Used as an end-to-end test of protocol, core and net together, against
//! openm2's own server.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Args, ValueEnum};
use m2_core::{
    CharacterChoice, CharacterClass, ChatKind, Command, Empire, EntityKind, Event, FriendNotice,
    GOLD_POINT, Millis, NewCharacter, Point, SHOP_PLACES, Session, SessionConfig, SessionError,
    ShopRefusal, Stage, WhisperKind, WhisperRefusal, Window, valid_character_name,
};
use m2_net::{Capture, Controller, NetConfig, Trust};
use m2_protocol::tls::KeyPin;

use crate::config::ClientConfig;

/// Centimetres a second a warrior's run animation covers (300 cm in 0.667 s):
/// how fast `play` draws an actor without a run motion running.
pub const RUN_SPEED: u32 = 450;

/// Walks start when our character appears in the world, or this long after
/// entering if the server never shows it.
const WALK_WAIT_MS: u64 = 2_000;

/// `--fight` gives up waiting for a monster this long after entering.
const FOE_WAIT_MS: u64 = 10_000;

/// `--fight` gives up a fight after this long.
const FIGHT_LIMIT_MS: u64 = 60_000;

/// The bot attacks a monster no farther than this, in centimetres, and walks
/// up to [`STAND_OFF`] from it when it is farther: about a warrior's reach
/// with a sword (`combat.md`).
const REACH: f64 = 180.0;
const STAND_OFF: f64 = 100.0;

/// The bot's rhythm without motion data: a swing a second, hitting 200 ms
/// in, as a warrior's first one-handed combo motion does (1 s long, its hit
/// starting at 0.19 s in the human's client).
const SWING_MS: u64 = 1_000;
const HIT_MS: u64 = 200;

/// Combo steps the bot cycles through.
const COMBO_STEPS: u8 = 4;

/// `--restart` comes back to life this long after dying.
const RESTART_DELAY_MS: u64 = 3_000;

/// `--drop` and `--drop-gold` wait this long after entering, for the
/// server's items.
const DROP_WAIT_MS: u64 = 2_000;

/// `--pick-up` takes what lies this close to our character, in centimetres.
const PICK_UP_REACH: f64 = 300.0;

/// `--shop` gives up waiting for an NPC, or for its shop, this long after
/// entering.
const SHOP_WAIT_MS: u64 = 10_000;

/// `--skill` uses its first skill this long after entering (for the
/// server's skill levels, and a monster to appear), and each next one this
/// long after the last.
const SKILL_WAIT_MS: u64 = 2_000;

/// A skill's motion hits a monster no farther than this, in centimetres,
/// this long after it starts: about Three-Way Cut's reach and first hit
/// (its first attack sphere, 2 m ahead and 1.2 m wide, at 0.16 s).
const SKILL_REACH: f64 = 300.0;
const SKILL_HIT_MS: u64 = 160;

/// `--whisper` and `--befriend` try again this long after the server said
/// nobody of that name is in the game, until this long after entering:
/// another bot started at the same time may still be on its way in.
const SOCIAL_RETRY_MS: u64 = 1_000;
const SOCIAL_WAIT_MS: u64 = 10_000;

/// The auth server's port when neither a flag nor the settings file says.
pub(crate) const AUTH_PORT: u16 = 11002;

/// The game server's port when neither a flag nor the settings file says.
pub(crate) const GAME_PORT: u16 = 13001;

/// Where the server is and how the client trusts it (`bot`, `play` and
/// `status`), from the flags over the settings file (`config.rs`).
/// [`ServerArgs::resolve`] fills in `host`, `auth_port` and `game_port`.
#[derive(Debug, Args)]
pub struct ServerArgs {
    /// The server's name or IP address (default: the settings file's). The
    /// client only ever connects to this host.
    #[arg(long = "host", value_name = "HOST")]
    host_arg: Option<String>,
    /// Auth server port (default: the settings file's, else 11002)
    #[arg(long = "auth-port", value_name = "PORT")]
    auth_port_arg: Option<u16>,
    /// Game server port (default: the settings file's, else 13001)
    #[arg(long = "game-port", value_name = "PORT")]
    game_port_arg: Option<u16>,
    /// The server's key, for a server with a self-signed certificate: its
    /// log prints it, and `openm2-server tls key` (default: the settings
    /// file's). Without it the server needs a certificate for --host from
    /// an authority the system trusts
    #[arg(long, env = "OPENM2_SERVER_KEY", value_name = "sha256:…")]
    pub server_key: Option<KeyPin>,
    /// The settings file (default: openm2.toml beside the program, when
    /// there is one)
    #[arg(long, value_name = "FILE")]
    config: Option<PathBuf>,
    /// The server's host, once resolved.
    #[arg(skip)]
    pub host: String,
    /// The auth server's port, once resolved.
    #[arg(skip)]
    pub auth_port: u16,
    /// The game server's port, once resolved.
    #[arg(skip)]
    pub game_port: u16,
    /// What the provider shows at every sign-in, once resolved (the
    /// settings file's `sign_in_prompt`).
    #[arg(skip)]
    pub sign_in_prompt: Option<m2_net::signin::SignInPrompt>,
}

impl ServerArgs {
    /// Reads the settings file and fills in the server's host, ports and
    /// key from it where the flags (and `OPENM2_SERVER_KEY`) say nothing,
    /// and what the provider shows at a sign-in.
    /// Returns the file's settings, for the rest of the command.
    pub fn resolve(&mut self) -> Result<ClientConfig> {
        let config = self.for_flags(ClientConfig::load(self.config.as_deref())?);
        self.apply(&config)?;
        Ok(config)
    }

    /// The settings file as it goes with the flags: a `--host` naming
    /// another server than the file's takes nothing of the file's server,
    /// whose name, ports and key aren't its own.
    fn for_flags(&self, config: ClientConfig) -> ClientConfig {
        match (&self.host_arg, &config.host) {
            (Some(flag), Some(file)) if !flag.eq_ignore_ascii_case(file) => config.without_server(),
            _ => config,
        }
    }

    /// As [`ServerArgs::resolve`], for `play`, whose login screen lists the
    /// player's own servers too: without a host anywhere there is no game's
    /// server, `host` stays empty and the second value is false.
    pub fn resolve_for_list(&mut self) -> Result<(ClientConfig, bool)> {
        let config = self.for_flags(ClientConfig::load(self.config.as_deref())?);
        let named = self.host_arg.is_some() || config.host.is_some();
        if named {
            self.apply(&config)?;
        } else {
            self.fill(&config);
        }
        Ok((config, named))
    }

    /// Fills in the server from `config` under the flags.
    fn apply(&mut self, config: &ClientConfig) -> Result<()> {
        self.host = self
            .host_arg
            .clone()
            .or_else(|| config.host.clone())
            .context("which server? Give --host, or `host` in the settings file (openm2.toml)")?;
        self.fill(config);
        Ok(())
    }

    /// Fills in all but the host from `config` under the flags.
    fn fill(&mut self, config: &ClientConfig) {
        self.auth_port = self.auth_port_arg.or(config.auth_port).unwrap_or(AUTH_PORT);
        self.game_port = self.game_port_arg.or(config.game_port).unwrap_or(GAME_PORT);
        self.server_key = self.server_key.or(config.server_key);
        self.sign_in_prompt = config.sign_in_prompt;
    }

    /// Where the server is, with the key it holds, once resolved.
    pub fn address(&self) -> crate::servers::ServerAddress {
        crate::servers::ServerAddress {
            host: self.host.clone(),
            auth_port: self.auth_port,
            game_port: self.game_port,
            key: self.server_key,
        }
    }

    /// The network driver's settings.
    pub fn net_config(&self) -> NetConfig {
        let trust = self.server_key.map_or(Trust::System, Trust::Pinned);
        let mut config = NetConfig::new(self.host.clone(), self.auth_port, self.game_port, trust);
        config.sign_in_prompt = self.sign_in_prompt;
        config
    }
}

/// Arguments of `openm2 bot`.
#[derive(Debug, Args)]
pub struct BotArgs {
    #[command(flatten)]
    server: ServerArgs,
    /// Sign in with the server's provider of this id (`openm2-server`'s
    /// providers file): the bot prints the address to open in a browser,
    /// and waits for it; `key` signs in with the device's own key
    #[arg(long, value_name = "ID")]
    provider: String,
    /// The device's key file, for `--provider key` (default:
    /// device-key.pem beside OPENM2_SETTINGS); made when there is none
    #[arg(long = "device-key", value_name = "FILE")]
    device_key: Option<PathBuf>,
    /// Character slot to play (0-3); default: the first character
    #[arg(long, conflicts_with = "create")]
    slot: Option<u8>,
    /// Play the character with this name, creating it first if the account
    /// has none by that name (1-24 ASCII letters and digits)
    #[arg(long, value_name = "NAME")]
    create: Option<String>,
    /// Class and gender of a character made by --create
    #[arg(long, value_enum, default_value_t = ClassArg::WarriorMale, requires = "create")]
    class: ClassArg,
    /// Empire for --create when the account has no characters yet; an account
    /// with characters keeps its empire
    #[arg(long, value_enum, default_value_t = EmpireArg::Shinsoo, requires = "create")]
    empire: EmpireArg,
    /// Say this in normal chat after entering the game (repeatable); slash
    /// commands work too, e.g. `/skillgroup 1` chooses a skill group
    #[arg(long)]
    say: Vec<String>,
    /// After entering and saying its lines, use the item in this slot of
    /// the inventory (repeatable): put it on, or take it off from slot 90 up
    #[arg(long = "use", value_name = "SLOT")]
    uses: Vec<u16>,
    /// 2 s after entering, drop the item in this slot of the bag, the whole
    /// stack (repeatable)
    #[arg(long = "drop", value_name = "SLOT")]
    drops: Vec<u16>,
    /// 2 s after entering, drop this much gold
    #[arg(long, value_name = "AMOUNT")]
    drop_gold: Option<u32>,
    /// Pick up every item that appears on the ground within 3 m of our
    /// character: what it dropped, what a monster it killed left
    #[arg(long)]
    pick_up: bool,
    /// Fight the first monster that appears after entering: walk up to it
    /// and attack until it dies, for at most 60 s. Walks wait until the
    /// fight is over
    #[arg(long)]
    fight: bool,
    /// When our character dies, come back to life where it fell 3 s later
    #[arg(long)]
    restart: bool,
    /// After the stay, leave the game for the character list once and
    /// enter again (the character the login picked), then stay again and
    /// log out
    #[arg(long)]
    change_character: bool,
    /// Talk to the first NPC that appears after entering: its shop opens if
    /// it has one. Then buy and sell what --buy and --sell say, and close
    /// the shop
    #[arg(long)]
    shop: bool,
    /// When the shop opens, buy what lies in this place of it, 0-39
    /// (repeatable)
    #[arg(long = "buy", value_name = "PLACE", requires = "shop")]
    buys: Vec<u8>,
    /// When the shop opens, sell the item in this slot of the bag, the whole
    /// stack (repeatable)
    #[arg(long = "sell", value_name = "SLOT", requires = "shop")]
    sells: Vec<u16>,
    /// Talk to the first NPC that appears after entering, and learn its
    /// skill group when it shows what it teaches (a teacher of
    /// `teachers.toml`)
    #[arg(long, conflicts_with = "shop")]
    learn: bool,
    /// Talk to the first NPC that appears after entering, and train our
    /// mount once when it shows what it trains (a stable of `mounts.toml`)
    #[arg(long, conflicts_with_all = ["shop", "learn"])]
    train: bool,
    /// Get on our mount as soon as we have one (with `--say "/mount 1"` for
    /// a game master); the walks are ridden
    #[arg(long)]
    ride: bool,
    /// After entering (with --learn, once the group is learned), spend a
    /// skill point on this skill (repeatable; the skill's number, e.g. 1
    /// for the warrior's Three-Way Cut)
    #[arg(long = "skill-up", value_name = "SKILL")]
    skill_ups: Vec<u8>,
    /// After entering, spend a status point on this stat (repeatable; its
    /// point number: 12 strength, 13 vitality, 14 dexterity, 15
    /// intelligence)
    #[arg(long = "raise-status", value_name = "POINT", value_parser = clap::value_parser!(u8).range(12..=15))]
    raise_statuses: Vec<u8>,
    /// 2 s after entering, use this skill at the nearest monster (walking up
    /// to it first when it is farther than 3 m) and hit it, or where it
    /// faces without one (repeatable, 2 s apart)
    #[arg(long = "skill", value_name = "SKILL")]
    skills: Vec<u8>,
    /// After entering, whisper TEXT to the character NAME: NAME=TEXT
    /// (repeatable). The bot prints that it went out, never the text; while
    /// nobody of that name is in the game it tries again every second, for
    /// 10 s after entering
    #[arg(long = "whisper", value_name = "NAME=TEXT", value_parser = parse_whisper)]
    whispers: Vec<(String, String)>,
    /// After entering, ask the character NAME to be our friend (repeatable),
    /// trying again as --whisper does while it is not in the game
    #[arg(long = "befriend", value_name = "NAME", value_parser = parse_name)]
    befriends: Vec<String>,
    /// Accept every friend request
    #[arg(long)]
    accept_friends: bool,
    /// After entering, end the friendship with the character NAME
    /// (repeatable)
    #[arg(long = "unfriend", value_name = "NAME", value_parser = parse_name)]
    unfriends: Vec<String>,
    /// After entering, walk this many metres east and south: DX,DY
    /// (repeatable; each walk starts where the last one ended)
    #[arg(long, value_name = "DX,DY", value_parser = parse_walk, allow_hyphen_values = true)]
    walk: Vec<(i32, i32)>,
    /// Seconds to stay in the game before logging out, counted after the
    /// last walk
    #[arg(long, default_value_t = 5.0)]
    stay: f64,
    /// Write every message to this .m2cap file (tickets and providers' codes
    /// blanked)
    #[arg(long)]
    capture: Option<PathBuf>,
    /// Print the final world state
    #[arg(long)]
    dump_state: bool,
}

/// `--class` values.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum ClassArg {
    WarriorMale,
    WarriorFemale,
    NinjaMale,
    NinjaFemale,
    SuraMale,
    SuraFemale,
    ShamanMale,
    ShamanFemale,
}

impl From<ClassArg> for CharacterClass {
    fn from(class: ClassArg) -> Self {
        match class {
            ClassArg::WarriorMale => CharacterClass::WarriorMale,
            ClassArg::WarriorFemale => CharacterClass::WarriorFemale,
            ClassArg::NinjaMale => CharacterClass::NinjaMale,
            ClassArg::NinjaFemale => CharacterClass::NinjaFemale,
            ClassArg::SuraMale => CharacterClass::SuraMale,
            ClassArg::SuraFemale => CharacterClass::SuraFemale,
            ClassArg::ShamanMale => CharacterClass::ShamanMale,
            ClassArg::ShamanFemale => CharacterClass::ShamanFemale,
        }
    }
}

/// `--empire` values.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum EmpireArg {
    /// Red
    Shinsoo,
    /// Yellow
    Chunjo,
    /// Blue
    Jinno,
}

impl From<EmpireArg> for Empire {
    fn from(empire: EmpireArg) -> Self {
        match empire {
            EmpireArg::Shinsoo => Empire::Shinsoo,
            EmpireArg::Chunjo => Empire::Chunjo,
            EmpireArg::Jinno => Empire::Jinno,
        }
    }
}

/// `--whisper` values: a character's name, `=`, and a chat line.
fn parse_whisper(text: &str) -> Result<(String, String), String> {
    let (name, line) = text
        .split_once('=')
        .ok_or_else(|| "a whisper is NAME=TEXT".to_owned())?;
    let name = parse_name(name)?;
    if line.is_empty() || line.len() > 512 || line.chars().any(char::is_control) {
        return Err("a whisper's text is 1 to 512 bytes, without control characters".to_owned());
    }
    Ok((name, line.to_owned()))
}

/// `--befriend` and `--unfriend` values: a character's name.
fn parse_name(name: &str) -> Result<String, String> {
    if valid_character_name(name) {
        Ok(name.to_owned())
    } else {
        Err(format!(
            "`{name}` is no character's name: 1 to 24 ASCII letters and digits"
        ))
    }
}

/// Something `--whisper` or `--befriend` asks of another player, and when
/// to try it again: `None` while the server has not answered.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Social {
    Whisper { name: String, text: String },
    Befriend { name: String },
}

impl Social {
    fn name(&self) -> &str {
        match self {
            Social::Whisper { name, .. } | Social::Befriend { name } => name,
        }
    }

    fn command(&self) -> Command {
        match self.clone() {
            Social::Whisper { name, text } => Command::Whisper { name, text },
            Social::Befriend { name } => Command::AskFriend { name },
        }
    }
}

/// `--walk` values: whole metres east and south.
pub(crate) fn parse_walk(text: &str) -> Result<(i32, i32), String> {
    let (x, y) = text
        .split_once(',')
        .ok_or_else(|| format!("`{text}` is not DX,DY"))?;
    let metres = |s: &str| {
        s.trim()
            .parse::<i32>()
            .ok()
            .filter(|m| m.abs() <= 10_000)
            .ok_or_else(|| format!("`{s}` is not a whole number of metres from -10000 to 10000"))
    };
    Ok((metres(x)?, metres(y)?))
}

/// A monster the bot fights.
#[derive(Debug, Clone, Copy)]
struct Foe {
    vid: u32,
    /// When the fight began.
    since: Millis,
    /// Where it was at the last update.
    at: Point,
    /// True while walking up to it.
    approaching: bool,
    /// The combo step of the last swing, 1 to [`COMBO_STEPS`].
    combo: u8,
    /// When the next swing may start.
    next_swing: Millis,
    /// When the swing in progress hits.
    hit_at: Option<Millis>,
}

/// Where the bot is with `--fight`.
#[derive(Debug, Clone, Copy)]
enum Fight {
    /// Not asked, or over.
    Off,
    /// Waiting for a monster to appear.
    Waiting,
    /// Fighting.
    On(Foe),
}

/// Where the bot is with `--shop` or `--learn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shopping {
    /// Not asked, or over.
    Off,
    /// Waiting for an NPC to appear.
    Waiting,
    /// Talked to this NPC; waiting for its shop.
    Talking(u32),
    /// The shop is open: its goods are printed, then the next tick buys,
    /// sells and closes it.
    Open {
        /// True once its goods are printed.
        printed: bool,
    },
}

/// Prints events and drives the scenario: say the lines, fight, shop, walk,
/// wait, log out.
struct Bot {
    say: Vec<String>,
    /// Inventory slots whose items to use.
    uses: Vec<u16>,
    /// Bag slots whose items to drop, and gold to drop, once the server
    /// sent them.
    drops: Vec<u16>,
    drop_gold: Option<u32>,
    /// The counts of the bag's stacks, and our gold, as the world last had
    /// them.
    stacks: BTreeMap<u16, u8>,
    gold: u32,
    /// `--pick-up`.
    pick_up: bool,
    /// Walks still to do, in metres, first one last.
    walks: Vec<(i32, i32)>,
    /// Where the next walk starts.
    position: Option<Point>,
    /// Our vid and when we entered.
    entered: Option<(u32, Millis)>,
    /// True once the walks began.
    walking: bool,
    stay_ms: u64,
    /// When the stay began: after entering and the last walk.
    staying_since: Option<Millis>,
    leaving: bool,
    fight: Fight,
    /// Where our character was at the last update.
    here: Option<Point>,
    /// `--restart`.
    restart: bool,
    /// `--change-character`, until it is done.
    change_character: bool,
    /// When our character died, while it lies dead.
    dead_since: Option<Millis>,
    shopping: Shopping,
    /// `--learn`: the NPC talked to is a teacher.
    learn: bool,
    /// `--train`: the NPC talked to is a stable.
    train: bool,
    /// `--ride`, until we asked to get on.
    ride: bool,
    /// Shop places to buy and bag slots to sell, once the shop is open.
    buys: Vec<u8>,
    sells: Vec<u16>,
    /// Skills to level once in the game.
    skill_ups: Vec<u8>,
    /// Stats to raise once in the game.
    raise_statuses: Vec<u8>,
    /// Skills to use, first one last, and when the next may be.
    skills: Vec<u8>,
    next_skill: Option<Millis>,
    /// The monster nearest to us at the last update, and where it was.
    nearest: Option<(u32, Point)>,
    /// A skill's hit to tell: the skill, the monster and when.
    skill_hit: Option<(u8, u32, Millis)>,
    /// True while walking up to a monster to use a skill on it.
    skill_walking: bool,
    /// `--whisper` and `--befriend` not done yet, and when to try each
    /// again.
    socials: Vec<(Social, Option<Millis>)>,
    /// `--accept-friends`.
    accept_friends: bool,
    /// `--unfriend`, until entering.
    unfriends: Vec<String>,
}

impl Bot {
    /// The server answered a whisper or a friend request to `name`: done,
    /// or (`again`) to be tried again a second later while the wait lasts.
    fn social_answered(&mut self, now: Millis, whisper: bool, name: &str, again: bool) {
        let waited = self
            .entered
            .is_some_and(|(_, at)| now.since(at) >= SOCIAL_WAIT_MS);
        let Some(index) = self.socials.iter().position(|(social, retry)| {
            retry.is_none()
                && matches!(social, Social::Whisper { .. }) == whisper
                && social.name().eq_ignore_ascii_case(name)
        }) else {
            return;
        };
        if again && !waited {
            if let Some((_, retry)) = self.socials.get_mut(index) {
                *retry = Some(now.plus(SOCIAL_RETRY_MS));
            }
        } else {
            self.socials.remove(index);
            if again {
                println!(
                    "{} {} is not in the game: giving up",
                    stamp(now),
                    shown(name)
                );
            }
        }
    }

    /// The whispers and requests whose time to try again has come.
    fn social_retries(&mut self, now: Millis) -> Vec<Command> {
        let mut commands = Vec::new();
        for (social, retry) in &mut self.socials {
            if retry.is_some_and(|at| now >= at) {
                *retry = None;
                commands.push(social.command());
            }
        }
        commands
    }

    /// The next walk, or the start of the stay when there is none. Both
    /// wait for a fight to end.
    fn next_walk(&mut self, now: Millis) -> Vec<Command> {
        if !matches!(self.fight, Fight::Off) {
            return Vec::new();
        }
        let (Some((dx, dy)), Some(from)) = (self.walks.pop(), self.position) else {
            self.staying_since = Some(now);
            return Vec::new();
        };
        let to = Point::new(
            from.x.saturating_add(dx.saturating_mul(100)),
            from.y.saturating_add(dy.saturating_mul(100)),
        );
        println!(
            "{} walking from ({}, {}) to ({}, {})",
            stamp(now),
            from.x,
            from.y,
            to.x,
            to.y
        );
        self.position = Some(to);
        vec![Command::MoveTo { path: vec![to] }]
    }

    /// Starts fighting `vid`, standing at `at`.
    fn engage(&mut self, now: Millis, vid: u32, at: Point) -> Vec<Command> {
        println!("{} fighting vid {vid}", stamp(now));
        self.fight = Fight::On(Foe {
            vid,
            since: now,
            at,
            approaching: false,
            combo: 0,
            next_swing: now,
            hit_at: None,
        });
        let mut commands = vec![Command::Target { vid }];
        commands.extend(self.approach());
        commands
    }

    /// Walks up to the foe if it is out of reach.
    fn approach(&mut self) -> Option<Command> {
        let (Fight::On(foe), Some(here)) = (&mut self.fight, self.here) else {
            return None;
        };
        let distance = here.distance(foe.at);
        if distance <= REACH {
            return None;
        }
        let t = (distance - STAND_OFF) / distance;
        let to = Point::new(
            (f64::from(here.x) + (f64::from(foe.at.x) - f64::from(here.x)) * t).round() as i32,
            (f64::from(here.y) + (f64::from(foe.at.y) - f64::from(here.y)) * t).round() as i32,
        );
        foe.approaching = true;
        Some(Command::MoveTo { path: vec![to] })
    }

    /// A walk is over: a skill walked up for is used, a fight's approach
    /// ends, or the next walk starts.
    fn walk_ended(&mut self, now: Millis) -> Vec<Command> {
        if self.skill_walking {
            self.skill_walking = false;
            self.next_skill = Some(now);
            return Vec::new();
        }
        match &mut self.fight {
            Fight::On(foe) => {
                foe.approaching = false;
                Vec::new()
            }
            _ => self.next_walk(now),
        }
    }

    /// Ends the fight; the walks and the stay go on from where we are.
    fn end_fight(&mut self, now: Millis, why: &str) -> Vec<Command> {
        println!("{} fight over: {why}", stamp(now));
        self.fight = Fight::Off;
        if self.here.is_some() {
            self.position = self.here;
        }
        self.next_walk(now)
    }

    /// Swings, hits and closes in, as the time comes.
    fn fight_tick(&mut self, now: Millis) -> Vec<Command> {
        let Fight::On(foe) = &mut self.fight else {
            return Vec::new();
        };
        if now.since(foe.since) >= FIGHT_LIMIT_MS {
            return self.end_fight(now, "it did not die in time");
        }
        if foe.approaching {
            return Vec::new();
        }
        if let Some(hit) = foe.hit_at {
            if now >= hit {
                foe.hit_at = None;
                return vec![Command::Hit {
                    vid: foe.vid,
                    skill: 0,
                }];
            }
            return Vec::new();
        }
        let (foe_at, combo) = (foe.at, foe.combo % COMBO_STEPS + 1);
        if let Some(walk) = self.approach() {
            return vec![walk];
        }
        let (Fight::On(foe), Some(here)) = (&mut self.fight, self.here) else {
            return Vec::new();
        };
        if now < foe.next_swing {
            return Vec::new();
        }
        foe.combo = combo;
        foe.hit_at = Some(now.plus(HIT_MS));
        foe.next_swing = now.plus(SWING_MS);
        vec![Command::Attack {
            angle: here.angle_to(foe_at),
            combo,
        }]
    }

    /// `--drop` and `--drop-gold`, once.
    fn drop_things(&mut self, now: Millis) -> Vec<Command> {
        let mut commands = Vec::new();
        for slot in self.drops.drain(..) {
            match self.stacks.get(&slot) {
                Some(&count) => {
                    println!("{} dropping {count} from slot {slot}", stamp(now));
                    commands.push(Command::Drop { slot, count });
                }
                None => println!("{} nothing to drop in slot {slot}", stamp(now)),
            }
        }
        if let Some(amount) = self.drop_gold.take() {
            println!("{} dropping {amount} gold of {}", stamp(now), self.gold);
            commands.push(Command::DropGold { amount });
        }
        commands
    }

    /// `--skill`: uses the next skill, facing the nearest monster (its
    /// hit follows if it is in reach).
    fn use_skill(&mut self, now: Millis) -> Vec<Command> {
        let Some(skill) = self.skills.pop() else {
            return Vec::new();
        };
        self.next_skill = Some(now.plus(SKILL_WAIT_MS));
        let here = self.here.unwrap_or_default();
        let (angle, target) = match self.nearest {
            Some((vid, at)) if here.distance(at) > SKILL_REACH => {
                // Too far: walk up to it, and use the skill on arriving.
                println!("{} walking up to vid {vid} for skill {skill}", stamp(now));
                self.skills.push(skill);
                self.skill_walking = true;
                let distance = here.distance(at);
                let t = (distance - STAND_OFF) / distance;
                let to = Point::new(
                    (f64::from(here.x) + (f64::from(at.x) - f64::from(here.x)) * t).round() as i32,
                    (f64::from(here.y) + (f64::from(at.y) - f64::from(here.y)) * t).round() as i32,
                );
                return vec![Command::MoveTo { path: vec![to] }];
            }
            Some((vid, at)) => {
                println!("{} using skill {skill} at vid {vid}", stamp(now));
                self.skill_hit = Some((skill, vid, now.plus(SKILL_HIT_MS)));
                (here.angle_to(at), Some(vid))
            }
            None => {
                println!("{} using skill {skill}", stamp(now));
                (0, None)
            }
        };
        vec![Command::UseSkill {
            skill,
            angle,
            target,
            motion: None,
            to: None,
        }]
    }

    /// `--buy` and `--sell` in the open shop, then closes it.
    fn shop_things(&mut self, now: Millis) -> Vec<Command> {
        let mut commands = Vec::new();
        for position in self.buys.drain(..) {
            println!("{} buying place {position}", stamp(now));
            commands.push(Command::Buy { position });
        }
        for slot in self.sells.drain(..) {
            println!("{} selling slot {slot}", stamp(now));
            commands.push(Command::Sell { slot });
        }
        println!("{} closing the shop", stamp(now));
        commands.push(Command::CloseShop);
        commands
    }
}

/// A skill's level as words (`skills.md`): `not learned`, `level 5`,
/// `master M3`, `grand master G1`, `perfect master`.
pub(crate) fn skill_level_text(level: u8) -> String {
    match level {
        0 => "not learned".to_owned(),
        1..=19 => format!("level {level}"),
        20..=29 => format!("master M{}", level - 19),
        30..=39 => format!("grand master G{}", level - 29),
        _ => "perfect master".to_owned(),
    }
}

/// Server text, made safe to print: control characters are escaped, so a
/// server cannot send terminal escape sequences.
pub(crate) fn shown(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

pub(crate) fn stamp(now: Millis) -> String {
    format!("[{:>4}.{:03}s]", now.0 / 1000, now.0 % 1000)
}

/// Prints an event as a log line, the way `bot` and `play` show the session.
pub(crate) fn print_event(now: Millis, event: &Event) {
    let at = stamp(now);
    match event {
        Event::Welcomed {
            server,
            protocol,
            name,
        } => println!("{at} {server}: {} speaks protocol {protocol}", shown(name)),
        Event::Phase { server, phase } => println!("{at} {server}: phase {phase:?}"),
        Event::SignInOptions(offer) => println!(
            "{at} auth: {} provider(s){}{}",
            offer.providers.len(),
            offer
                .providers
                .iter()
                .map(|p| format!(" {} ({})", shown(&p.id), shown(&p.name)))
                .collect::<String>(),
            if offer.key { ", and devices' keys" } else { "" }
        ),
        Event::LoggedIn => println!("{at} auth: logged in"),
        Event::CharacterList(list) => {
            println!("{at} game: {} character(s)", list.len());
            for c in list {
                println!(
                    "{at}   slot {}: {} (level {})",
                    c.slot,
                    shown(&c.name),
                    c.level
                );
            }
        }
        Event::Creating {
            name,
            class,
            empire,
        } => {
            let empire = match empire {
                Some(empire) => format!(" in {empire:?}"),
                None => String::new(),
            };
            println!("{at} game: creating {} ({class:?}){empire}", shown(name));
        }
        Event::Created(c) => println!("{at} game: created {} in slot {}", shown(&c.name), c.slot),
        Event::CreateRefused { error } => {
            println!(
                "{at} game: {}",
                SessionError::CreateRefused { error: *error }
            );
        }
        Event::Selected { slot, name } => {
            println!("{at} game: playing slot {slot}: {}", shown(name));
        }
        Event::EnteredGame(me) => match me {
            Some(me) => println!("{at} game: entered as {} (vid {})", shown(&me.name), me.vid),
            None => println!("{at} game: entered"),
        },
        Event::LeftGame => println!("{at} game: left the game for the character list"),
        Event::EntityAppeared {
            vid,
            kind,
            race,
            at: point,
        } => println!(
            "{at} {kind:?} {race} (vid {vid}) appeared at ({}, {})",
            point.x, point.y
        ),
        Event::EntityLeft { vid } => println!("{at} vid {vid} left"),
        Event::GroundItemAppeared {
            vid,
            vnum,
            at: point,
        } => println!(
            "{at} item {vnum} (vid {vid}) lies at ({}, {})",
            point.x, point.y
        ),
        Event::GroundItemLeft { vid } => println!("{at} item vid {vid} is gone from the ground"),
        Event::ShopOpened { vid } => println!("{at} the shop of vid {vid} opened"),
        Event::ShopClosed => println!("{at} the shop closed"),
        Event::ShopRefused(ShopRefusal::NotEnoughGold) => {
            println!("{at} shop: not enough gold");
        }
        Event::ShopRefused(ShopRefusal::BagFull) => println!("{at} shop: no room in the bag"),
        Event::ShopRefused(ShopRefusal::Other(reason)) => {
            println!("{at} shop: refused (reason {reason})");
        }
        Event::TeacherOpened { vid } => println!("{at} the teacher vid {vid} opened"),
        Event::TeacherClosed => println!("{at} the teacher closed"),
        Event::SkillChoiceOpened { slot } => {
            println!("{at} the item in slot {slot} offers skills to train");
        }
        Event::SkillChoiceClosed => println!("{at} the skill choice closed"),
        Event::StableOpened { vid } => println!("{at} the stable of vid {vid} opened"),
        Event::StableClosed => println!("{at} the stable closed"),
        Event::UpgradeOffered { slot } => println!("{at} an upgrade of slot {slot} offered"),
        Event::UpgradeClosed => println!("{at} the upgrade offer closed"),
        Event::MountChanged { vid, look: 0 } => println!("{at} vid {vid} is on foot"),
        Event::MountChanged { vid, look } => println!("{at} vid {vid} rides a {look}"),
        Event::MountStateChanged => println!("{at} our mount changed"),
        Event::RunSpeedChanged { run_speed } => println!("{at} run speed {run_speed} cm/s"),
        Event::AffectsChanged => println!("{at} what our skills raise changed"),
        Event::SkillGroupChanged { group } => println!("{at} skill group {group}"),
        Event::SkillChanged { skill, level, .. } => {
            println!("{at} skill {skill}: {}", skill_level_text(*level));
        }
        Event::Arrived(point) => println!("{at} arrived at ({}, {})", point.x, point.y),
        Event::MoveRefused(point) => {
            println!("{at} move refused: back at ({}, {})", point.x, point.y);
        }
        Event::Damaged { vid, amount, flags } => {
            println!("{at} vid {vid} took {amount} damage (flags {flags:#04x})");
        }
        Event::Died { vid } => println!("{at} vid {vid} died"),
        Event::Shot {
            shooter,
            target,
            at: aim,
        } => println!(
            "{at} vid {shooter} shot at vid {target} at ({}, {})",
            aim.x, aim.y
        ),
        Event::Flight { kind, from, to } => {
            println!("{at} {kind:?} flies from vid {from} to vid {to}");
        }
        Event::EntityEffect { kind, vid } => println!("{at} vid {vid} shows {kind:?}"),
        Event::Fishing { vid, step, float } => {
            println!(
                "{at} vid {vid} fishing {step:?}, float at ({}, {})",
                float.x, float.y
            );
        }
        Event::Chat(line) => println!(
            "{at} chat {:?}{}: {}",
            line.kind,
            if line.mine { " (mine)" } else { "" },
            shown(&line.text)
        ),
        // Never what was whispered: `play`'s lines go to the launcher's
        // game.log.
        Event::Whisper { name, line } => match line.kind {
            WhisperKind::Sent => println!("{at} whispered to {}", shown(name)),
            WhisperKind::Received => println!("{at} a whisper from {}", shown(name)),
            WhisperKind::Refused(_) => {}
        },
        Event::WhisperRefused { name, reason } => {
            let why = match reason {
                WhisperRefusal::NotOnline => "nobody of that name is in the game".to_owned(),
                WhisperRefusal::TooMany => "too many whispers".to_owned(),
                WhisperRefusal::Other(reason) => format!("reason {reason}"),
            };
            println!("{at} the whisper to {} did not go out: {why}", shown(name));
        }
        Event::Friends => println!("{at} friends listed"),
        Event::FriendChanged { name, change } => {
            println!("{at} friend {}: {change:?}", shown(name));
        }
        Event::FriendAsked { name } => println!("{at} {} asks to be our friend", shown(name)),
        Event::FriendNotice { name, notice } => {
            println!("{at} friend request, {}: {notice:?}", shown(name));
        }
        Event::LoggedOut => println!("{at} logged out"),
        // Printed once at the end from `Session::error`.
        Event::Failed(_) => {}
        Event::Ignored { what, why } => tracing::debug!(what, why, "ignored"),
    }
}

impl Controller for Bot {
    fn on_event(&mut self, now: Millis, event: &Event) -> Vec<Command> {
        print_event(now, event);
        match event {
            Event::EnteredGame(me) => {
                self.position = me
                    .as_ref()
                    .map(|me| Point::new(me.position.0, me.position.1));
                self.entered = me.as_ref().map(|me| (me.vid, now));
                let mut commands: Vec<Command> = self
                    .say
                    .drain(..)
                    .map(|text| Command::Chat {
                        kind: ChatKind::Talk,
                        text,
                    })
                    .collect();
                commands.extend(self.uses.drain(..).map(|slot| Command::UseItem {
                    window: Window::Inventory,
                    slot,
                }));
                // With --learn the points wait for the group.
                if !self.learn {
                    commands.extend(
                        self.skill_ups
                            .drain(..)
                            .map(|skill| Command::SkillUp { skill }),
                    );
                }
                commands.extend(
                    self.raise_statuses
                        .drain(..)
                        .map(|point| Command::RaiseStatus { point }),
                );
                commands.extend(self.socials.iter().map(|(social, _)| social.command()));
                commands.extend(
                    self.unfriends
                        .drain(..)
                        .map(|name| Command::RemoveFriend { name }),
                );
                if self.walks.is_empty() || me.is_none() {
                    commands.extend(self.next_walk(now));
                }
                commands
            }
            Event::Whisper { name, line } if line.kind == WhisperKind::Sent => {
                self.social_answered(now, true, name, false);
                Vec::new()
            }
            Event::WhisperRefused { name, reason } => {
                let again = *reason == WhisperRefusal::NotOnline;
                self.social_answered(now, true, name, again);
                Vec::new()
            }
            Event::FriendNotice { name, notice } => {
                let again = *notice == FriendNotice::NotOnline;
                self.social_answered(now, false, name, again);
                Vec::new()
            }
            Event::FriendAsked { name } if self.accept_friends => {
                println!("{} accepting {}", stamp(now), shown(name));
                vec![Command::AnswerFriend {
                    name: name.clone(),
                    accept: true,
                }]
            }
            // Our own character: the server has placed us, walks can start.
            Event::EntityAppeared { vid, at, .. }
                if self.entered.is_some_and(|(me, _)| me == *vid) && !self.walking =>
            {
                self.walking = true;
                self.position = Some(*at);
                self.here = Some(*at);
                self.next_walk(now)
            }
            Event::EntityAppeared {
                vid,
                kind: EntityKind::Monster,
                at,
                ..
            } if matches!(self.fight, Fight::Waiting) && self.entered.is_some() => {
                self.engage(now, *vid, *at)
            }
            Event::Arrived(_) => self.walk_ended(now),
            // The server put us back: the walk is over where it has us.
            Event::MoveRefused(at) => {
                self.position = Some(*at);
                self.here = Some(*at);
                self.walk_ended(now)
            }
            Event::Died { vid } if self.entered.is_some_and(|(me, _)| me == *vid) => {
                self.dead_since = Some(now);
                match self.fight {
                    Fight::On(_) => self.end_fight(now, "we died"),
                    _ => Vec::new(),
                }
            }
            Event::Died { vid } | Event::EntityLeft { vid } if matches!(self.fight, Fight::On(foe) if foe.vid == *vid) => {
                self.end_fight(now, "the monster is gone")
            }
            Event::EntityAppeared {
                vid,
                kind: EntityKind::Npc,
                ..
            } if self.shopping == Shopping::Waiting && self.entered.is_some() => {
                println!("{} talking to vid {vid}", stamp(now));
                self.shopping = Shopping::Talking(*vid);
                vec![Command::Talk { vid: *vid }]
            }
            Event::ShopOpened { vid } if self.shopping == Shopping::Talking(*vid) => {
                self.shopping = Shopping::Open { printed: false };
                Vec::new()
            }
            Event::SkillGroupChanged { group } if self.learn && *group != 0 => self
                .skill_ups
                .drain(..)
                .map(|skill| Command::SkillUp { skill })
                .collect(),
            Event::StableOpened { vid }
                if self.train && self.shopping == Shopping::Talking(*vid) =>
            {
                println!("{} training our mount at vid {vid}", stamp(now));
                self.shopping = Shopping::Off;
                vec![Command::TrainMount { vid: *vid }, Command::CloseStable]
            }
            Event::MountStateChanged if self.ride => {
                println!("{} getting on our mount", stamp(now));
                self.ride = false;
                vec![Command::Ride { ride: true }]
            }
            Event::TeacherOpened { vid }
                if self.learn && self.shopping == Shopping::Talking(*vid) =>
            {
                println!("{} learning the skill group of vid {vid}", stamp(now));
                self.shopping = Shopping::Off;
                vec![
                    Command::LearnSkillGroup { vid: *vid },
                    Command::CloseTeacher,
                ]
            }
            Event::GroundItemAppeared { vid, at, .. } if self.pick_up => match self.here {
                Some(here) if here.distance(*at) <= PICK_UP_REACH => {
                    println!("{} picking up vid {vid}", stamp(now));
                    vec![Command::PickUp { vid: *vid }]
                }
                _ => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn on_update(&mut self, now: Millis, session: &Session) {
        let world = session.world();
        if let Some(me) = world.my_entity() {
            self.here = Some(me.track.point(now));
            if !me.dead {
                self.dead_since = None;
            }
        }
        if let Fight::On(foe) = &mut self.fight
            && let Some(entity) = world.entities.get(&foe.vid)
        {
            foe.at = entity.track.point(now);
        }
        let here = self.here.unwrap_or_default();
        self.nearest = world
            .entities
            .values()
            .filter(|e| e.kind == EntityKind::Monster && !e.dead)
            .map(|e| (e.vid, e.track.point(now)))
            .min_by(|a, b| here.distance(a.1).total_cmp(&here.distance(b.1)));
        self.stacks = world
            .items
            .iter()
            .filter(|((window, _), _)| *window == Window::Inventory)
            .map(|((_, slot), item)| (*slot, item.count))
            .collect();
        self.gold = world.points.get(&GOLD_POINT).copied().unwrap_or(0);
        if self.shopping == (Shopping::Open { printed: false })
            && let Some(shop) = &world.shop
        {
            self.shopping = Shopping::Open { printed: true };
            for (place, good) in &shop.goods {
                println!(
                    "{}   place {place}: item {} x{} for {} gold",
                    stamp(now),
                    good.vnum,
                    good.count,
                    good.price
                );
            }
        }
    }

    fn on_tick(&mut self, now: Millis) -> Vec<Command> {
        let retries = self.social_retries(now);
        if !retries.is_empty() {
            return retries;
        }
        if self.restart
            && let Some(died) = self.dead_since
            && now.since(died) >= RESTART_DELAY_MS
        {
            self.dead_since = None;
            println!("{} restarting where we fell", stamp(now));
            return vec![Command::Restart { town: false }];
        }
        if let Some((_, entered)) = self.entered
            && (!self.drops.is_empty() || self.drop_gold.is_some())
            && now.since(entered) >= DROP_WAIT_MS
        {
            return self.drop_things(now);
        }
        if let Some((_, entered)) = self.entered
            && matches!(self.fight, Fight::Waiting)
            && now.since(entered) >= FOE_WAIT_MS
        {
            return self.end_fight(now, "no monster appeared");
        }
        if self.shopping == (Shopping::Open { printed: true }) {
            self.shopping = Shopping::Off;
            return self.shop_things(now);
        }
        if let Some((skill, vid, at)) = self.skill_hit
            && now >= at
        {
            self.skill_hit = None;
            println!("{} skill {skill} hit vid {vid}", stamp(now));
            return vec![Command::Hit { vid, skill }];
        }
        if let Some((_, entered)) = self.entered
            && !self.skills.is_empty()
            && !self.skill_walking
            && now.since(entered) >= SKILL_WAIT_MS
            && self.next_skill.is_none_or(|at| now >= at)
        {
            return self.use_skill(now);
        }
        if let Some((_, entered)) = self.entered
            && matches!(self.shopping, Shopping::Waiting | Shopping::Talking(_))
            && now.since(entered) >= SHOP_WAIT_MS
        {
            let why = match self.shopping {
                Shopping::Talking(_) if self.learn => "no teacher opened",
                Shopping::Talking(_) if self.train => "no stable opened",
                Shopping::Talking(_) => "no shop opened",
                _ => "no NPC appeared",
            };
            println!("{} shopping over: {why}", stamp(now));
            self.shopping = Shopping::Off;
        }
        if matches!(self.fight, Fight::On(_)) {
            return self.fight_tick(now);
        }
        if let Some((_, entered)) = self.entered
            && !self.walking
            && !self.walks.is_empty()
            && now.since(entered) >= WALK_WAIT_MS
        {
            self.walking = true;
            return self.next_walk(now);
        }
        match self.staying_since {
            Some(at) if !self.leaving && now.since(at) >= self.stay_ms => {
                if std::mem::take(&mut self.change_character) {
                    // Entering again starts the stay over.
                    println!("{} changing character", stamp(now));
                    self.staying_since = None;
                    self.entered = None;
                    self.walking = false;
                    return vec![Command::ChangeCharacter];
                }
                self.leaving = true;
                vec![Command::Logout]
            }
            _ => Vec::new(),
        }
    }

    /// Prints where to sign in: a human opens it, the tests play the
    /// browser.
    fn sign_in(&mut self, provider: &str, url: &str) -> bool {
        use std::io::Write as _;
        println!("to sign in ({}), open: {url}", shown(provider));
        let _ = std::io::stdout().flush();
        true
    }
}

/// Creates a capture file (and its folder).
pub(crate) fn open_capture(path: &std::path::Path) -> Result<Capture> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    Capture::create(path).with_context(|| format!("creating {}", path.display()))
}

/// Runs the bot. Exit code 0 after a clean logout, 1 if the session failed.
pub fn run(mut args: BotArgs) -> Result<ExitCode> {
    args.server.resolve()?;
    if let Some(line) = args.say.iter().find(|line| line.len() > 512) {
        anyhow::bail!(
            "--say: a chat line is at most 512 bytes, not {}",
            line.len()
        );
    }
    let say = args.say.clone();
    let stay_ms = if args.stay.is_finite() && args.stay >= 0.0 {
        // Whole milliseconds are plenty; the cast saturates for huge values.
        (args.stay * 1000.0) as u64
    } else {
        anyhow::bail!("--stay must be a number of seconds, 0 or more");
    };
    let character = match (&args.create, args.slot) {
        (Some(name), _) => CharacterChoice::Create(
            NewCharacter::new(name, args.class.into(), args.empire.into()).context("--create")?,
        ),
        (None, Some(slot)) => CharacterChoice::Slot(slot),
        (None, None) => CharacterChoice::First,
    };

    let mut config = SessionConfig::provider(&args.provider).context("--provider")?;
    config.character = character;
    let with_key = config.signs_with_key();
    let mut session = Session::new(config);

    let mut capture = args.capture.as_deref().map(open_capture).transpose()?;
    let mut net = args.server.net_config();
    if with_key {
        let settings = std::env::var_os(crate::settings::ENV).map(PathBuf::from);
        net.device_key = Some(crate::key::load(
            args.device_key.as_deref(),
            settings.as_deref(),
        )?);
    }
    if let Some(place) = args.buys.iter().find(|place| **place >= SHOP_PLACES) {
        anyhow::bail!(
            "--buy {place}: a shop's places are 0 to {}",
            SHOP_PLACES - 1
        );
    }
    let mut bot = Bot {
        say,
        uses: args.uses.clone(),
        drops: args.drops.clone(),
        drop_gold: args.drop_gold,
        stacks: BTreeMap::new(),
        gold: 0,
        pick_up: args.pick_up,
        walks: args.walk.iter().rev().copied().collect(),
        position: None,
        entered: None,
        walking: false,
        stay_ms,
        staying_since: None,
        leaving: false,
        fight: if args.fight {
            Fight::Waiting
        } else {
            Fight::Off
        },
        here: None,
        restart: args.restart,
        change_character: args.change_character,
        dead_since: None,
        shopping: if args.shop || args.learn || args.train {
            Shopping::Waiting
        } else {
            Shopping::Off
        },
        learn: args.learn,
        train: args.train,
        ride: args.ride,
        buys: args.buys.clone(),
        sells: args.sells.clone(),
        skill_ups: args.skill_ups.clone(),
        raise_statuses: args.raise_statuses.clone(),
        skills: args.skills.iter().rev().copied().collect(),
        next_skill: None,
        nearest: None,
        skill_hit: None,
        skill_walking: false,
        socials: args
            .whispers
            .iter()
            .map(|(name, text)| Social::Whisper {
                name: name.clone(),
                text: text.clone(),
            })
            .chain(
                args.befriends
                    .iter()
                    .map(|name| Social::Befriend { name: name.clone() }),
            )
            .map(|social| (social, None))
            .collect(),
        accept_friends: args.accept_friends,
        unfriends: args.unfriends.clone(),
    };
    let traffic = m2_net::run(&mut session, &net, &mut bot, capture.as_mut())?;
    println!(
        "traffic: sent {} bytes, received {} over {} connection(s), {} resumed (TLS included)",
        traffic.sent, traffic.received, traffic.connections, traffic.resumed
    );

    if args.dump_state {
        println!("{:#?}", session.world());
    }
    match session.stage() {
        Stage::LoggedOut => Ok(ExitCode::SUCCESS),
        stage => {
            match session.error() {
                Some(error) => eprintln!("error: {error}"),
                None => eprintln!("error: the session stopped while {stage}"),
            }
            Ok(ExitCode::FAILURE)
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        server: ServerArgs,
    }

    /// The server's settings from `args` over `config`.
    fn server(args: &[&str], config: &ClientConfig) -> Result<ServerArgs> {
        let mut cli = Cli::try_parse_from(std::iter::once("openm2").chain(args.iter().copied()))?;
        let config = cli.server.for_flags(config.clone());
        cli.server.apply(&config)?;
        Ok(cli.server)
    }

    #[test]
    fn flags_win_over_the_settings_file_which_wins_over_the_defaults() {
        let pin: KeyPin = "sha256:5e1f0a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7"
            .parse()
            .unwrap();
        let config = ClientConfig {
            host: Some("play.example.net".to_owned()),
            auth_port: Some(21002),
            server_key: Some(pin),
            sign_in_prompt: Some(m2_net::signin::SignInPrompt::Login),
            ..ClientConfig::default()
        };
        let from_file = server(&[], &config).unwrap();
        assert_eq!(
            (
                from_file.host.as_str(),
                from_file.auth_port,
                from_file.game_port
            ),
            ("play.example.net", 21002, GAME_PORT)
        );
        assert_eq!(from_file.server_key, Some(pin));
        assert_eq!(
            from_file.net_config().sign_in_prompt,
            Some(m2_net::signin::SignInPrompt::Login)
        );

        let flags = server(
            &[
                "--host",
                "127.0.0.1",
                "--auth-port",
                "1",
                "--game-port",
                "2",
            ],
            &config,
        )
        .unwrap();
        assert_eq!(
            (flags.host.as_str(), flags.auth_port, flags.game_port),
            ("127.0.0.1", 1, 2)
        );

        // Another server than the file's takes nothing of the file's server:
        // not its ports, not its key (a local save the launcher runs).
        let other = server(&["--host", "127.0.0.1"], &config).unwrap();
        assert_eq!(
            (other.host.as_str(), other.auth_port, other.game_port),
            ("127.0.0.1", AUTH_PORT, GAME_PORT)
        );
        assert_eq!(other.server_key, None);
        assert_eq!(
            other.net_config().sign_in_prompt,
            Some(m2_net::signin::SignInPrompt::Login),
            "what the provider shows is the player's, for every server"
        );
        // The file's own server, by its host in any case, takes it all.
        let same = server(&["--host", "PLAY.example.net"], &config).unwrap();
        assert_eq!((same.auth_port, same.server_key), (21002, Some(pin)));

        let defaults = server(&["--host", "127.0.0.1"], &ClientConfig::default()).unwrap();
        assert_eq!(
            (defaults.auth_port, defaults.game_port),
            (AUTH_PORT, GAME_PORT)
        );
        assert_eq!(defaults.net_config().sign_in_prompt, None);
    }

    #[test]
    fn without_a_host_anywhere_the_server_is_asked_for() {
        let error = server(&[], &ClientConfig::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("--host"), "{error}");
        // `play`'s list needs none: the rest is filled in all the same.
        let mut cli = Cli::try_parse_from(["openm2", "--auth-port", "5"]).unwrap();
        cli.server.fill(&ClientConfig::default());
        assert_eq!(
            (
                cli.server.host.as_str(),
                cli.server.auth_port,
                cli.server.game_port
            ),
            ("", 5, GAME_PORT)
        );
        let address = server(&["--host", "a.example"], &ClientConfig::default())
            .unwrap()
            .address();
        assert_eq!(address.place(), "a.example");
    }
}
