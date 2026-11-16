//! `openm2 play`: the game in a window, from the login screen to playing.
//! The screens (`screens.rs`) show a button per provider of the server's
//! (signing in opens the system's browser), show the account's characters with the chosen one drawn in the middle, make new
//! ones, and show the loading screen; then `play` draws the map around our
//! character with the characters and monsters near it, follows our
//! character with the camera, and walks it to where the player clicks
//! (`docs/specs/render.md`, "Playing"; `docs/specs/ui.md`, "Play's
//! screens").
//!
//! In the game: left click on the ground walks there, on a monster runs up
//! to it and attacks it (`fight.rs`: the weapon's combo while the button
//! is held), on an item on the ground runs to it and picks it up
//! (`ground.rs`), on anyone else chooses them; Space attacks as long as it
//! is held; right drag turns the camera, the wheel zooms, F switches the
//! fog, Enter opens the chat line, I opens the inventory (`inventory.rs`:
//! click to pick up and put down, right click to use, let go over the world
//! to drop), 1 to 4 and F1 to F4 use the quickslots, M opens the atlas,
//! Escape (with nothing else to close) or the taskbar's gears open the game
//! menu (`menu.rs`: settings, changing character, logging out, quitting),
//! and closing the window logs out. The target window shows the
//! chosen entity, damage rises over heads, items on the ground show their
//! names, a restart window comes up when our character dies, and the
//! minimap shows the map around our character (`minimap.rs`).
//!
//! What it draws and plays comes from an asset folder (`--assets`, or
//! `assets` in `openm2.toml`; `source.rs`, `docs/specs/assets.md`): a
//! game's own world, or a player's client imported by `openm2 assets
//! import`, as the launcher does.
//!
//! With `--screenshot` no window opens: it signs in with `--provider` and draws
//! one picture of a screen (`--screen`), for the game after `--stay`
//! seconds in it (and after any `--walk`, with the inventory open for
//! `--inventory`, the atlas for `--atlas`, fighting the nearest monsters for `--fight`, picking up
//! what lies around for `--pick-up`), then logs out. That is how agents
//! and tests look at it.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, ValueEnum};
use glam::{Mat4, Vec3};
use m2_assets::actor::{ROD, body_shape, weapon_grip};
use m2_assets::items::{Items, Monsters};
use m2_assets::skills::{Skills, weapon_allows};
use m2_audio::Audio;
use m2_content::Formula;
use m2_core::{
    CharacterChoice, ChatKind, Command, Entity, EntityEffectKind, EntityKind, EntityState, Event,
    FishingStep, FlightKind, Millis, MoveKind, Point, Session, SessionConfig, SessionError,
    SignInProvider, Stage, WORN_FROM, Window, World, skill_motion, skill_of_motion,
};
use m2_formats::RawStr;
use m2_net::{Controller, Net, NetConfig, Remote, Trust};
use m2_render::renderer::{Attachments, OFFSCREEN_FORMAT};
use m2_render::terrain::TEXTURE_SPAN;
use m2_render::{
    Bounds, Camera, EffectGeometry, Gpu, GpuArea, GpuEffects, GpuOptions, GpuPart, GpuShineMap,
    GpuTexture, Offscreen, Renderer, Scene, World as WorldDraw,
};
use m2_ui::Cooldown;
use m2_ui::Label;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{UserAttentionType, WindowId};

use crate::actors::{self, ActorModel, FightMotions, FishingMotion, Look, Once, Shining};
use crate::bot::{RUN_SPEED, open_capture, parse_walk, print_event, shown};
use crate::character::Tab;
use crate::clipboard::{Clipboard, Shortcut};
use crate::effects::upload_pending;
use crate::fight::{self, Body, Fighter, HitSphere, Stance, attack_seed, can_fight};
use crate::glows::{self, UpgradeGlow};
use crate::ground::{self, Gatherer};
use crate::interface::{ScaleArgs, readable, wheel_lines};
use crate::map_folder::AssetMap;
use crate::mapview::{self, GpuMap, LoadedMap};
use crate::minimap::{MapPictures, MarkKind, MinimapView};
use crate::nametags::{self, NAME_HEIGHT, NAME_LINE, NAME_REACH};
use crate::screens::{Action, PlayInterface, Progress, Screen, failure_text};
use crate::server_list::{ServerAction, ServerRow};
use crate::servers::{AskError, Check, Reach, SavedServer, ServerAddress, ServersFile};
use crate::settings::Settings;
use crate::shop::{self, Talker};
use crate::skills;
use crate::source::{GameEffect, Music, NumberPaths, SkillLookPaths, Source, WorldMap};
use crate::steer::{Steering, Way};
use crate::view::Files;
use crate::walls::{self, Walls};
use crate::window::Surface;
use crate::world_effects::{Loop, MotionClock, WorldEffects};
use crate::world_sounds::WorldSounds;

/// Radians the camera turns per pixel dragged.
/// How far out `play` looks for water to cast at, in centimetres (the
/// original client's 6 m).
const FISHING_REACH: f64 = 600.0;

/// The shortest time between two reel-ins a move or a click sends.
const REEL_GAP: Duration = Duration::from_millis(500);

/// How many of the Servers board's servers are asked at once how they
/// answer.
const PING_THREADS: usize = 8;

/// The point `distance` centimetres from `from` the way `angle` degrees
/// points (0 south, 90 east, as moves give them).
fn ahead(from: Point, angle: u16, distance: f64) -> Point {
    let radians = f64::from(angle % 360).to_radians();
    Point::new(
        (f64::from(from.x) + distance * radians.sin()).round() as i32,
        (f64::from(from.y) + distance * radians.cos()).round() as i32,
    )
}

/// The way to cast from `from` facing `facing` (degrees) on a map of
/// `walls`: the first of straight ahead, then 10° to either side in turn
/// up to 180°, whose point [`FISHING_REACH`] out is on water.
fn water_way(walls: &Walls, from: Point, facing: u16) -> Option<u16> {
    (0..=36i32)
        .map(|n| {
            let turn = (n + 1) / 2 * 10;
            if n % 2 == 1 { turn } else { -turn }
        })
        .map(|turn| u16::try_from((i32::from(facing) + turn).rem_euclid(360)).unwrap_or(0))
        .find(|angle| walls.is_water(ahead(from, *angle, FISHING_REACH)))
}

/// Centimetres over its feet a fisher's fish shows when its model gives no
/// height.
const HEAD_HEIGHT: f32 = 180.0;

const TURN: f32 = 0.01;
/// Pixels the mouse may move between press and release for a click.
const CLICK_SLOP: f64 = 5.0;
/// How far a click looks for the ground, in centimetres.
const REACH: f32 = 100_000.0;
/// The camera looks at this point above the character's feet.
const EYE_HEIGHT: f32 = 120.0;
/// `--screenshot` gives up if its screen has not come after this long.
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(120);
/// Height angle of the camera on the character on the select screen.
const SHOWCASE_PITCH: f32 = 8.0;
/// How far apart a damage number's digits stand, in centimetres, and the
/// most it has, as the original client draws them.
const DIGIT_WIDTH: f32 = 30.0;
const MOST_DIGITS: usize = 8;
/// The size of an entity without a model, for picking and hitting: radius
/// and height in centimetres.
const NO_MODEL_SIZE: (f32, f32) = (40.0, 150.0);
/// A move without a duration farther than this (cm) is a jump, not a run.
const CHASE_LIMIT: f64 = 3_000.0;
/// The least size of an item on the ground for picking it: radius and
/// height in centimetres.
const ITEM_SIZE: (f32, f32) = (25.0, 30.0);
/// Items' names show this far above them, in centimetres; names of items
/// lying together stack this many pixels apart (a line of the skin's
/// text), times the interface's scale.
const ITEM_NAME_HEIGHT: f32 = 15.0;
const ITEM_NAME_LINE: f32 = 15.0;
/// Effects play and show within this many centimetres of the camera's
/// target.
const EFFECT_REACH: f32 = 8_000.0;
/// The weapon's wear position (`inventory.md`, "Worn items").
const WEAPON_POSITION: u16 = 4;
/// `--skill` waits this long after the walks for the server's skill levels.
const SKILL_WAIT: Duration = Duration::from_secs(2);
/// How far a skill's target may be without a range of its own: a hit's
/// reach, the server's (`world.md`, "Skills"), in centimetres.
const SKILL_REACH: f64 = 500.0;
/// A charging skill (Dash) is used within this of its foe, runs up to one
/// at most this far, and stops this short of it, in centimetres; it gives
/// up after `CHARGE_TIME`.
const CHARGE_NEAR: f64 = 250.0;
const CHARGE_FAR: f64 = 1_500.0;
const CHARGE_STOP: f64 = 150.0;
const CHARGE_TIME: u64 = 3_000;
/// Our mana (`entities.md`, "Points").
const SP_POINT: u8 = 7;
/// Damage's flags (`combat.md`, "Damage").
const POISON_HIT: u8 = 2;
const DODGED: u8 = 4;
const BLOCKED: u8 = 8;
const CRITICAL: u8 = 32;
const HEAL: u8 = 64;
const BURNING_HIT: u8 = 128;
/// The states drawn on entities with a game effect, and
/// the bone each plays on, turning with it, as the client's scripts attach
/// them: from the feet without one.
const STATE_EFFECTS: [(EntityState, GameEffect, Option<&str>); 4] = [
    (EntityState::Poisoned, GameEffect::Poisoned, Some("Bip01")),
    (EntityState::Burning, GameEffect::Burning, Some("Bip01")),
    (EntityState::Slowed, GameEffect::Slowed, None),
    (
        EntityState::Stunned,
        GameEffect::Stunned,
        Some("Bip01 Head"),
    ),
];

/// Which screen `--screenshot` draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Shot {
    /// The login screen (no server needed).
    Login,
    /// The account's characters (or making one, on an account without).
    Select,
    /// The game.
    Game,
}

/// `openm2 play`
#[derive(Debug, Args)]
pub struct PlayArgs {
    #[command(flatten)]
    server: crate::bot::ServerArgs,
    /// Sign in at once with the server's provider of this id, in the
    /// system's browser (the login screen has a button per provider);
    /// --screenshot signs in with it (default: the settings file's)
    #[arg(long, value_name = "ID")]
    provider: Option<String>,
    /// Only print where to sign in, without opening the system's browser
    /// (to open it elsewhere, or on a machine without one)
    #[arg(long)]
    no_browser: bool,
    /// Character slot to play (0-3), without the select screen
    #[arg(long)]
    slot: Option<u8>,
    /// The asset folder to play with (default: the settings file's); `openm2
    /// assets import` makes one of a client
    #[arg(long = "assets", value_name = "DIR")]
    assets_arg: Option<PathBuf>,
    /// The asset folder, once resolved.
    #[arg(skip)]
    assets: Option<PathBuf>,
    /// Load the map's areas within this many metres of the character
    #[arg(long, value_name = "METRES", default_value_t = 400.0)]
    radius: f32,
    /// Camera distance from the character, in metres
    #[arg(long, value_name = "METRES", default_value_t = 15.0)]
    distance: f32,
    /// Camera angle around the character in degrees; 0 looks north
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    yaw: f32,
    /// Camera height angle in degrees
    #[arg(long, default_value_t = 35.0, allow_negative_numbers = true)]
    pitch: f32,
    /// Draw the environment's fog
    #[arg(long)]
    fog: bool,
    /// Draw the world only in the game, without the status and chat windows
    #[arg(long)]
    no_interface: bool,
    /// Size of the window (before the display's scaling) or picture,
    /// WIDTHxHEIGHT
    #[arg(long, default_value = "1280x800", value_parser = crate::view::parse_size)]
    size: (u32, u32),
    #[command(flatten)]
    scale: ScaleArgs,
    /// Use a software adapter (no GPU needed; slower)
    #[arg(long)]
    software: bool,
    /// Write every message to this .m2cap file (tickets and providers' codes
    /// blanked); each later login in the window writes its own file, with
    /// -2, -3, ... after the name
    #[arg(long, value_name = "FILE")]
    capture: Option<PathBuf>,
    /// Play no effects: the maps', the motions', the hits' and the items'
    /// (to compare pictures without them)
    #[arg(long)]
    no_effects: bool,
    /// No sounds and no music, and no audio device opened
    #[arg(long)]
    mute: bool,
    /// Volume of the music for this session, 0 to 1 (default: the player's
    /// settings, else 0.5)
    #[arg(long, value_name = "VOLUME")]
    music_volume: Option<f32>,
    /// Volume of the other sounds for this session, 0 to 1 (default: the
    /// player's settings, else 1)
    #[arg(long, value_name = "VOLUME")]
    sound_volume: Option<f32>,
    /// The player's settings file (volumes, muting, the interface's size),
    /// read at the start and written when they change in the game's
    /// Settings window; without it they last for the session
    #[arg(long, value_name = "FILE", env = crate::settings::ENV)]
    settings: Option<PathBuf>,
    /// The player's servers file, the login screen's Servers board's list
    /// (default: servers.toml beside the settings file; without either,
    /// the list lasts for the session)
    #[arg(long, value_name = "FILE")]
    servers: Option<PathBuf>,
    /// The game's own server's name on the Servers board, once resolved
    /// (`server_name` in the settings file, else its host); none without a
    /// host anywhere.
    #[arg(skip)]
    official: Option<String>,
    /// No window: draw one picture of a screen into this PNG, then log out
    #[arg(long, value_name = "FILE")]
    screenshot: Option<PathBuf>,
    /// With --screenshot: which screen to draw
    #[arg(long, value_enum, default_value_t = Shot::Game, requires = "screenshot")]
    screen: Shot,
    /// With --screenshot: seconds to wait on the screen (in the game, after
    /// the walks) before the picture
    #[arg(long, default_value_t = 2.0, requires = "screenshot")]
    stay: f64,
    /// With --screenshot of the game: take the picture this many seconds
    /// after the first damage shown instead (to see hit effects)
    #[arg(long, value_name = "SECONDS", requires = "screenshot", hide = true)]
    after_hit: Option<f64>,
    /// With --screenshot of the game: open the inventory, to see the items
    /// the server sent
    #[arg(long, requires = "screenshot")]
    inventory: bool,
    /// With --screenshot of the game: open the atlas, the whole map
    #[arg(long, requires = "screenshot")]
    atlas: bool,
    /// With --screenshot of the game: open the character window on its
    /// Skills tab
    #[arg(long, requires = "screenshot", conflicts_with = "character")]
    skills: bool,
    /// With --screenshot of the game: open the character window on its
    /// Character tab, to see the stats the server sent
    #[arg(long, requires = "screenshot")]
    character: bool,
    /// After entering, walk this many metres east and south: DX,DY
    /// (repeatable; each walk starts where the last one ended)
    #[arg(long, value_name = "DX,DY", value_parser = parse_walk, allow_hyphen_values = true)]
    walk: Vec<(i32, i32)>,
    /// After entering, say this in the chat before walking (repeatable);
    /// slash commands work too, e.g. `/skillgroup 1`
    #[arg(long, value_name = "TEXT")]
    say: Vec<String>,
    /// After entering (and the walks), fight: run up to the nearest monster
    /// and attack it as if the button were held, then the next one
    #[arg(long)]
    fight: bool,
    /// After entering (and the walks and any fight), pick up the items
    /// lying around that are ours to take: run to the nearest and pick it
    /// up, then the next
    #[arg(long)]
    pick_up: bool,
    /// After entering (and the walks, any fight and pick-ups), walk up to
    /// the nearest NPC and talk to it: its shop opens if it has one
    #[arg(long)]
    shop: bool,
    /// After entering (and the walks, any fight, pick-ups and talk), use
    /// this skill once (its number, e.g. 1 for the warrior's Three-Way
    /// Cut), at the nearest monster if there is one
    #[arg(long, value_name = "SKILL")]
    skill: Option<u8>,
}

/// What the network thread last saw of the session.
struct Shared {
    world: World,
    stage: Stage,
    /// The session's clock at the last update, and when that was.
    clock: Option<(Millis, Instant)>,
    /// True once the session ended.
    finished: bool,
    error: Option<String>,
    failure: Option<SessionError>,
    /// Events the screens react to, not yet taken.
    events: Vec<Event>,
}

/// A copy of [`Shared`] with the session's clock brought forward to now.
struct Snapshot {
    world: World,
    stage: Stage,
    now: Millis,
    finished: bool,
    events: Vec<Event>,
}

/// Whether the network thread hands `event` on to the window, which reacts
/// to it; the others are only printed.
fn handed_on(event: &Event) -> bool {
    matches!(
        event,
        Event::Created(_)
            | Event::CreateRefused { .. }
            | Event::LeftGame
            | Event::Damaged { .. }
            | Event::ShopRefused(_)
            | Event::MoveRefused(_)
            | Event::Shot { .. }
            | Event::Flight { .. }
            | Event::EntityEffect { .. }
            | Event::Fishing { .. }
            | Event::WhisperRefused { .. }
            | Event::FriendNotice { .. }
            | Event::FriendChanged { .. }
    )
}

/// The network thread's controller: prints events, publishes the world,
/// opens the browser for a sign-in (unless `--no-browser`).
struct Publisher {
    shared: Arc<Mutex<Shared>>,
    browser: bool,
}

impl Controller for Publisher {
    fn on_event(&mut self, now: Millis, event: &Event) -> Vec<Command> {
        print_event(now, event);
        // The screens react to these (whispers refused and friends' news
        // become chat lines), the world's actors flinch at damage, walks end
        // where the server put us back, monsters' shots and the experience
        // of kills fly.
        if handed_on(event)
            && let Ok(mut shared) = self.shared.lock()
        {
            shared.events.push(event.clone());
        }
        Vec::new()
    }

    fn on_tick(&mut self, _now: Millis) -> Vec<Command> {
        Vec::new()
    }

    fn on_update(&mut self, now: Millis, session: &Session) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.world.clone_from(session.world());
            shared.stage = session.stage();
            shared.clock = Some((now, Instant::now()));
            shared.error = session.error().map(ToString::to_string);
            shared.failure = session.error().cloned();
        }
    }

    fn sign_in(&mut self, provider: &str, url: &str) -> bool {
        println!("to sign in ({}), open: {url}", crate::bot::shown(provider));
        if self.browser
            && let Err(error) = open_browser(url)
        {
            eprintln!("note: cannot open the browser ({error}); open the address above");
        }
        true
    }
}

/// Opens `url` (`https`, or `http` on this machine: `m2_net::signin`
/// checked it) in the system's browser.
fn open_browser(url: &str) -> std::io::Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(std::io::Error::other("not a web address"));
    }
    let mut command = if cfg!(windows) {
        let mut command = std::process::Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else {
        std::process::Command::new("xdg-open")
    };
    command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Asks the auth server at `net` for its providers on a thread of its own;
/// the login screen shows a button each when they come, or why there are
/// none.
fn ask_providers(net: NetConfig) -> mpsc::Receiver<Result<Vec<SignInProvider>, String>> {
    let (tx, rx) = mpsc::channel();
    let asked = thread::Builder::new()
        .name("openm2 providers".to_owned())
        .spawn(move || {
            let result = match crate::servers::ask(&net) {
                Ok(answer) => Ok(answer.providers),
                Err(AskError::Session(error)) => Err(crate::screens::failure_text(&error)),
                Err(AskError::Silent) => Err("The server did not say how to sign in.".to_owned()),
                Err(AskError::Net(words)) => Err(crate::screens::sentence(&words)),
            };
            if let Err(why) = &result {
                tracing::debug!("asking for the providers: {why}");
            }
            let _ = tx.send(result);
        });
    if let Err(error) = asked {
        eprintln!("note: cannot ask the server for its providers: {error}");
    }
    rx
}

/// The capture file of the window's login number `login`: `path` for the
/// first, then `name-2.m2cap`, `name-3.m2cap`, ... beside it, so a new
/// login does not overwrite what the last one recorded.
fn capture_path(path: &Path, login: u32) -> PathBuf {
    if login <= 1 {
        return path.to_owned();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let name = match path.extension() {
        Some(extension) => format!("{stem}-{login}.{}", extension.to_string_lossy()),
        None => format!("{stem}-{login}"),
    };
    path.with_file_name(name)
}

/// The session, running on its own thread.
struct Connection {
    remote: Remote,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

impl Connection {
    /// Logs in on a new thread, like `bot`: the window's login number
    /// `login`, from 1, which names its capture.
    fn start(
        args: &PlayArgs,
        net_config: NetConfig,
        config: SessionConfig,
        login: u32,
    ) -> Result<Self> {
        let path = args
            .capture
            .as_deref()
            .map(|path| capture_path(path, login));
        let mut capture = path.as_deref().map(open_capture).transpose()?;
        let mut session = Session::new(config);
        let net = Net::new();
        let remote = net.remote();
        let shared = Arc::new(Mutex::new(Shared {
            world: World::default(),
            stage: Stage::Idle,
            clock: None,
            finished: false,
            error: None,
            failure: None,
            events: Vec::new(),
        }));
        let mut publisher = Publisher {
            shared: Arc::clone(&shared),
            browser: !args.no_browser,
        };
        let thread = thread::Builder::new()
            .name("openm2 net".to_owned())
            .spawn(move || {
                let result = net.run(&mut session, &net_config, &mut publisher, capture.as_mut());
                if let Ok(mut shared) = publisher.shared.lock() {
                    shared.finished = true;
                    shared.stage = session.stage();
                    shared.failure = session.error().cloned();
                    shared.error = match result {
                        Err(error) => Some(format!("{error:#}")),
                        Ok(_) => session.error().map(ToString::to_string),
                    };
                }
            })
            .context("starting the network thread")?;
        Ok(Self {
            remote,
            shared,
            thread: Some(thread),
        })
    }

    /// The session's state; takes the events not taken yet.
    fn snapshot(&self) -> Result<Snapshot> {
        let mut shared = self
            .shared
            .lock()
            .map_err(|_| anyhow!("the network thread stopped"))?;
        let now = shared.clock.map_or(Millis(0), |(at, when)| {
            at.plus(u64::try_from(when.elapsed().as_millis()).unwrap_or(u64::MAX))
        });
        // Our path's corners turned on time, between the session's ticks.
        let mut world = shared.world.clone();
        world.follow_path(now);
        Ok(Snapshot {
            world,
            stage: shared.stage,
            now,
            finished: shared.finished,
            events: std::mem::take(&mut shared.events),
        })
    }

    /// Logs out and waits for the network thread. Why the session failed,
    /// if it did, in words for the player.
    fn finish(&mut self) -> Option<String> {
        self.remote.send(Command::Logout);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let shared = self.shared.lock().ok()?;
        match (shared.stage, &shared.failure, &shared.error) {
            (Stage::LoggedOut, _, _) => None,
            (_, Some(failure), _) => Some(failure_text(failure)),
            (_, None, Some(error)) => Some(error.clone()),
            (stage, None, None) => Some(format!("The session stopped while {stage}.")),
        }
    }
}

/// The map we are on, read around where we stood.
struct PlacedMap {
    entry: WorldMap,
    /// Where we stood when it was read (world centimetres).
    focus: Point,
    loaded: LoadedMap,
}

/// Which model an entity is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ActorKey {
    /// A player character: class and gender, hair, and the item numbers of
    /// the body armour and the weapon it wears (0 for none).
    Player {
        race: u16,
        hair: u16,
        body: u16,
        weapon: u16,
        /// Riding a mount: the riding motions.
        riding: bool,
    },
    Monster(u16),
}

impl ActorKey {
    fn of(entity: &Entity) -> Self {
        match entity.kind {
            EntityKind::Player if entity.race < 8 => ActorKey::Player {
                race: entity.race,
                hair: entity.parts[3],
                body: entity.parts[0],
                weapon: entity.parts[1],
                riding: entity.mount != 0,
            },
            _ => ActorKey::Monster(entity.race),
        }
    }
}

/// An entity's model on the GPU.
struct DrawnEntity {
    /// What it was drawn as; a new key (other clothes) means a new model.
    key: ActorKey,
    model: Option<usize>,
    parts: Vec<GpuPart>,
    /// Seconds into its standing or running motion.
    clock: f32,
    /// A motion it plays once (an attack, a flinch, its fall), and seconds
    /// into it.
    once: Option<(Once, f32)>,
    /// When its last attack move started (its track's start): each attack
    /// starts its motion once.
    attack_seen: Option<Millis>,
    /// Where it was drawn last (world centimetres), for running to where a
    /// move without a duration put it ([`chase`]).
    shown: Option<(f64, f64)>,
    /// The motion it played last frame and how far into it, for the
    /// effects of its events.
    clock_seen: Option<MotionClock>,
    /// A player hidden from us: not drawn.
    hidden: bool,
    /// A player's mount while it rides (`docs/specs/render.md`, "Riding").
    mount: Option<MountView>,
    /// A fisher's float, on the water: where its fishing
    /// effects play.
    float: Option<Vec3>,
    /// A motion to play once the one playing ends: a bite during the
    /// throw.
    then: Option<Once>,
}

/// The mount a player rides, drawn under it.
struct MountView {
    /// The monster it looks like.
    look: u16,
    model: Option<usize>,
    parts: Vec<GpuPart>,
    /// Seconds into its standing or running motion.
    clock: f32,
}

/// An item's model as it lies on the ground.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ItemShape {
    /// Which of the models.
    model: usize,
    /// From the model to the item's spot, before its turn
    /// ([`ground::lying`]).
    lay: Mat4,
    /// Its box lying there.
    bounds: Bounds,
}

/// An item on the ground on the GPU.
struct DrawnItem {
    vnum: u32,
    shape: Option<ItemShape>,
    parts: Vec<GpuPart>,
}

/// What is on the GPU: the map, one copy of each model, and an instance of
/// it per entity and per item on the ground.
#[derive(Default)]
struct Drawn {
    map: Option<GpuMap>,
    templates: Templates,
    entities: BTreeMap<u32, DrawnEntity>,
    ground: BTreeMap<u32, DrawnItem>,
    /// The effects' textures, numbered as the game's effect library
    /// numbers them.
    effect_textures: Vec<GpuTexture>,
    /// This frame's effect triangles.
    effects: Option<GpuEffects>,
}

/// One copy of each model on the GPU, and the shine maps their shining
/// parts reflect (armours', weapons'), read the first time one shines.
#[derive(Default)]
struct Templates {
    models: HashMap<usize, Vec<GpuPart>>,
    shine: Option<[Option<GpuShineMap>; 2]>,
}

/// Instances of model `index`'s parts, uploading the model the first time:
/// its shining parts (`ActorModel::shine`) shine with the asset folder's
/// shine map.
fn instances(
    gpu: &Gpu,
    renderer: &Renderer,
    templates: &mut Templates,
    source: &Source,
    models: &[ActorModel],
    index: usize,
) -> Result<Vec<GpuPart>> {
    let parts = match templates.models.entry(index) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            let model = models.get(index);
            let mut uploaded = model
                .map(|m| {
                    m.parts
                        .iter()
                        .map(|p| renderer.upload(gpu, &p.part, &p.textures))
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            let shine = model.map_or(&[][..], |m| &m.shine);
            if shine.iter().any(Option::is_some) {
                let maps = templates.shine.get_or_insert_with(|| {
                    [Shining::Armour, Shining::Weapon].map(|what| {
                        let data = source.shine_map(what)?;
                        renderer
                            .upload_shine_map(gpu, &data)
                            .inspect_err(|error| eprintln!("note: a shine map: {error}"))
                            .ok()
                    })
                });
                for (part, shine) in uploaded.iter_mut().zip(shine) {
                    let Some((what, shine)) = *shine else {
                        continue;
                    };
                    let map = match what {
                        Shining::Armour => &maps[0],
                        Shining::Weapon => &maps[1],
                    };
                    if let Some(map) = map {
                        renderer.set_shine(gpu, part, shine, map);
                    }
                }
            }
            entry.insert(uploaded)
        }
    };
    Ok(parts.iter().map(|t| renderer.instance(gpu, t)).collect())
}

/// The game as drawn: the map and the actors, the camera, the scripted
/// walks.
struct Game {
    /// Where the game is read from: an asset folder.
    source: Source,
    /// The maps of the world.
    atlas: Vec<WorldMap>,
    /// The items, for what characters wear and what lies on the ground.
    items: Option<Items>,
    /// The monsters' names and levels, for the names over their heads.
    monsters: Option<Monsters>,
    /// Names show over characters' heads (the player's settings).
    names: bool,
    radius: f32,
    map: Option<PlacedMap>,
    /// The map whose walls were read last (its map file), and its walls if
    /// it has attribute files: our walks go around them.
    walls: Option<(Vec<u8>, Option<Walls>)>,
    models: Vec<ActorModel>,
    by_key: HashMap<ActorKey, Option<usize>>,
    /// Ground models by path and whether they lie down, read the first
    /// time.
    item_models: HashMap<(String, bool), Option<ItemShape>>,
    camera: Camera,
    fog: bool,
    /// `--walk`, first one last.
    planned: Vec<(i32, i32)>,
    /// `--say`, said once in each game.
    lines: Vec<String>,
    /// Lines still to say.
    says: Vec<String>,
    /// Walks still to do, first one last.
    walks: Vec<(i32, i32)>,
    /// Where the last scripted walk goes.
    walking_to: Option<Point>,
    /// Since when the walks are done and our character stands in the world.
    ready_since: Option<Instant>,
    /// Entities that took damage since the last frame: they flinch.
    hit: Vec<u32>,
    /// Damage taken since the last frame (vid, amount, flags): its numbers
    /// show.
    numbers_due: Vec<(u32, i32, u8)>,
    /// How damage numbers show, as the source names their files.
    numbers: Option<NumberPaths>,
    /// Which of the effects of a hit on us the next one plays.
    ours_turn: usize,
    /// How the skills on characters look while they are on, by skill
    /// number, as the source names them.
    skill_looks: BTreeMap<u8, SkillLookPaths>,
    /// Our side of fights.
    fighter: Fighter,
    /// `--fight`.
    fight_script: bool,
    /// Our way to an item we were asked to pick up.
    gatherer: Gatherer,
    /// `--pick-up`, and the items it went for.
    pick_up_script: bool,
    tried: BTreeSet<u32>,
    /// Our way to an NPC we were asked to talk to.
    talker: Talker,
    /// `--shop`, and the NPCs it talked to.
    shop_script: bool,
    talked: BTreeSet<u32>,
    /// The NPC whose shop we closed by walking away, until the session
    /// forgets it; the same for a teacher and a smith's upgrade offer.
    left_shop: Option<u32>,
    left_teacher: Option<u32>,
    left_smith: Option<u32>,
    /// Our walks from the movement keys.
    steering: Steering,
    /// The skills: their names, flags and motions.
    skills: Option<Skills>,
    /// `--skill`, until it is used.
    skill_script: Option<u8>,
    /// Until when the motion of the skill we last used plays: no other
    /// skill before.
    skill_until: Option<Millis>,
    /// The cooldown and mana cost formula of each skill, and how far its
    /// target may be (in metres), from the skills file.
    skill_cooldowns: BTreeMap<u8, Formula>,
    skill_costs: BTreeMap<u8, Formula>,
    skill_ranges: BTreeMap<u8, f64>,
    /// A charging skill on its way to its foe.
    charging: Option<Charge>,
    /// When a move or a click last reeled our line in.
    reeled_at: Option<Instant>,
    /// The skills we used that are cooling down: when, and until when.
    cooldowns: BTreeMap<u8, (Millis, Millis)>,
    /// What can be picked and hit this frame: every entity in the scene.
    bodies: Vec<Body>,
    /// What can be picked of the items on the ground this frame.
    item_bodies: Vec<Body>,
    /// Which of the labels shown name items on the ground, to pick them
    /// up by their names.
    item_names: ItemNames,
    /// Where our character is drawn this frame.
    my_place: Option<Mat4>,
    /// False with `--no-effects`: no effect plays.
    play_effects: bool,
    /// What a hit, a critical hit, an item on the ground and the states
    /// show ([`GameEffect`]), as the source names them.
    game_effects: [Option<Vec<u8>>; GameEffect::ALL.len()],
    /// The glows of upgraded items the asset folder names, lowest level
    /// first.
    upgrade_glows: Vec<UpgradeGlow>,
    /// The flying effect of the experience a kill gave, as the source
    /// names it.
    experience_flight: Option<Vec<u8>>,
    /// The sound of our character drinking a potion, as the source names
    /// it.
    potion_sound: Option<Vec<u8>>,
    /// What happened to entities since the last frame, to show on them.
    shown: Vec<(u32, EntityEffectKind)>,
    /// The fishing steps of players since the last frame.
    fishing_steps: Vec<(u32, FishingStep, Point)>,
    /// Where the pointer is over the world (not the windows), as a spot of
    /// the view (`x`, `y` from -1 to 1, y up) and the view's aspect.
    pointer: Option<(f32, f32, f32)>,
    /// The entity under it: its ring shows.
    hovered: Option<u32>,
    /// The effects playing, and those read from the asset folder.
    effects: WorldEffects,
    /// This frame's effect triangles.
    effect_geometry: EffectGeometry,
    /// Damage taken since the last frame (vid and flags): a hit effect
    /// shows on each.
    hits_shown: Vec<(u32, u8)>,
    /// When the first hit effect showed (`--after-hit`).
    first_hit: Option<Instant>,
    /// What is heard (silent for `--screenshot`).
    sounds: WorldSounds,
    /// The minimap's and the atlas's pictures.
    maps: MapPictures,
}

/// What a Damage shows over the entity it names, as the original client
/// shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Number {
    /// A hit on us that missed.
    OurMiss,
    /// A hit on another that missed.
    TheirMiss,
    /// A hit on us, of this much.
    Ours(u32),
    /// A hit on our target, of this much.
    Target(u32),
}

/// What a Damage of `amount` with `flags` (`combat.md`) shows: a miss on
/// whoever it names, a hit's amount on us (`ours`) or our target
/// (`target`); nothing for a heal, nothing taken, or a hit on another.
/// Critical, piercing, poison and fire show as any hit.
fn number(amount: i32, flags: u8, ours: bool, target: bool) -> Option<Number> {
    if flags & (DODGED | BLOCKED) != 0 {
        return Some(if ours {
            Number::OurMiss
        } else {
            Number::TheirMiss
        });
    }
    let amount = u32::try_from(amount).ok().filter(|_| flags & HEAL == 0)?;
    match (amount > 0, ours, target) {
        (false, _, _) => None,
        (true, true, _) => Some(Number::Ours(amount)),
        (true, false, true) => Some(Number::Target(amount)),
        (true, false, false) => None,
    }
}

/// The digits of `amount`, the last first (536: 6, 3, 5), at most
/// [`MOST_DIGITS`].
fn digits(amount: u32) -> Vec<usize> {
    let mut left = amount;
    let mut digits = Vec::new();
    while left > 0 && digits.len() < MOST_DIGITS {
        digits.push((left % 10) as usize);
        left /= 10;
    }
    digits
}

/// Where a skill goes, once it may be used ([`aim_skill`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Aim {
    /// Used now on this entity (or no one).
    At(Option<u32>),
    /// A charging skill runs up to its foe first, to this point.
    Charge { foe: u32, to: Point },
}

/// Who skill of flags `flag` goes on, for our character `me` at `now`
/// (`docs/specs/ui.md`, "Skills"): `foe` (else our target)
/// if it can be fought; for a skill used on oneself (`CAN_USE_FOR_ME`), us;
/// for one for others (`ONLY_FOR_ALLIANCE`), the player we chose, else us.
/// Refused: riding (a job's skills are used on foot), no
/// foe for a skill that needs one, its target farther than `reach`
/// centimetres, a `cost` above our mana. A charging skill
/// (`CHARGE_ATTACK`) on a foe out of reach, within [`CHARGE_FAR`], runs up
/// to [`CHARGE_STOP`] short of it.
fn aim_skill(
    world: &World,
    now: Millis,
    me: &Entity,
    flag: &dyn Fn(&str) -> bool,
    foe: Option<u32>,
    reach: f64,
    cost: Option<i64>,
) -> Result<Aim, String> {
    if me.mount != 0 {
        return Err("Get off your mount to use this skill.".to_owned());
    }
    let foe = foe
        .or(world.target)
        .filter(|vid| *vid != me.vid && world.entities.get(vid).is_some_and(fight::can_fight));
    let on_us = flag("CAN_USE_FOR_ME");
    let for_others = flag("ONLY_FOR_ALLIANCE");
    if flag("NEED_TARGET") && !on_us && foe.is_none() {
        return Err("This skill needs a target.".to_owned());
    }
    // The player we chose, for a skill for others.
    let friend = world.target.filter(|vid| *vid != me.vid).filter(|vid| {
        world.entities.get(vid).is_some_and(|e| {
            matches!(e.kind, EntityKind::Player | EntityKind::TransformedPlayer) && !e.dead
        })
    });
    let aim = if for_others {
        friend
    } else if flag("NEED_TARGET") {
        foe
    } else {
        None
    };
    let here = me.track.point(now);
    if let Some((vid, there)) =
        aim.and_then(|vid| Some((vid, world.entities.get(&vid)?.track.point(now))))
    {
        let distance = here.distance(there);
        if flag("CHARGE_ATTACK") && distance > CHARGE_NEAR.max(reach) && distance <= CHARGE_FAR {
            let short = (distance - CHARGE_STOP) / distance;
            let to = Point::new(
                here.x + (f64::from(there.x - here.x) * short) as i32,
                here.y + (f64::from(there.y - here.y) * short) as i32,
            );
            return Ok(Aim::Charge { foe: vid, to });
        }
        if distance > reach {
            return Err("Too far away.".to_owned());
        }
    }
    // Its mana, as the server works it out with what we know.
    let mana = i64::from(world.points.get(&SP_POINT).copied().unwrap_or(0));
    if cost.is_some_and(|cost| cost > mana) {
        return Err("You do not have enough mana.".to_owned());
    }
    Ok(Aim::At(if for_others {
        Some(friend.unwrap_or(me.vid))
    } else if on_us {
        Some(me.vid)
    } else {
        foe
    }))
}

/// A charging skill on its way to its foe: used once our
/// character is near it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Charge {
    skill: u8,
    foe: u32,
    /// It gives up then.
    until: Millis,
}

/// Where an actor is drawn this frame: where the session puts it (`truth`)
/// while it follows its track (`follow`: a timed walk, our own character,
/// the dead), or the first time; else it runs from where it was drawn
/// towards that point, `step` centimetres at most. A jump farther than
/// [`CHASE_LIMIT`] is taken at once. Also says whether it is still running.
fn chase(
    shown: Option<(f64, f64)>,
    truth: (f64, f64),
    follow: bool,
    step: f64,
) -> ((f64, f64), bool) {
    let Some(from) = shown.filter(|_| !follow) else {
        return (truth, false);
    };
    let (dx, dy) = (truth.0 - from.0, truth.1 - from.1);
    let distance = dx.hypot(dy);
    if distance <= step.max(1.0) || distance > CHASE_LIMIT {
        return (truth, false);
    }
    let t = step / distance;
    ((from.0 + dx * t, from.1 + dy * t), true)
}

/// Where an actor at `position` facing `angle` (degrees) is drawn.
fn placement(position: Vec3, angle: u16) -> Mat4 {
    Mat4::from_translation(position) * Mat4::from_rotation_z(f32::from(angle).to_radians())
}

/// What a press on the world found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Picked {
    Entity(u32),
    Item(u32),
}

/// What a press on the world is on, as the original client chooses: an
/// item on the ground under the pointer (`item`), else the item whose name
/// is there (`named`), else the entity under it. So an item still lying in
/// a dead monster's body, or behind someone, is picked up.
fn picked(
    item: Option<u32>,
    named: Option<u32>,
    entity: impl FnOnce() -> Option<u32>,
) -> Option<Picked> {
    item.or(named)
        .map(Picked::Item)
        .or_else(|| entity().map(Picked::Entity))
}

/// Which of the labels over the scene name items on the ground: from
/// `first` on, one for each of `vids` in turn.
#[derive(Debug, Default)]
struct ItemNames {
    first: usize,
    vids: Vec<u32>,
}

impl ItemNames {
    /// The item the label at `index` names, if it names one.
    fn item(&self, index: usize) -> Option<u32> {
        self.vids.get(index.checked_sub(self.first)?).copied()
    }
}

/// An item on the ground at `spot` as something to pick: an upright
/// cylinder around it as it lies, wide enough for a weapon's length and
/// never too small to click.
fn item_body(vid: u32, spot: Vec3, shape: Option<ItemShape>) -> Body {
    let (radius, height) = shape.map_or(ITEM_SIZE, |s| {
        let size = s.bounds.max - s.bounds.min;
        (size.x.max(size.y) * 0.5, s.bounds.max.z)
    });
    Body {
        vid,
        feet: spot,
        radius: radius.max(ITEM_SIZE.0),
        height: height.max(ITEM_SIZE.1),
        spheres: Vec::new(),
        foe: false,
    }
}

/// Whether an entity of `kind` has a body to pick and hit. A warp or a
/// goto has none: it is a point that takes whoever comes near it, drawn as
/// nothing, so a click on it is a click on the ground there and our
/// character walks in (openm2: before, a click there chose it as a target
/// and kept us still).
fn has_body(kind: EntityKind) -> bool {
    !matches!(kind, EntityKind::Warp | EntityKind::Goto)
}

impl Game {
    fn new(args: &PlayArgs) -> Result<Self> {
        let source = Source::open(args.assets.as_deref().context("no asset folder")?)?;
        let atlas = source.world()?;
        let game_effects = GameEffect::ALL.map(|effect| source.game_effect(effect));
        let experience_flight = source.flight(FlightKind::Experience);
        let potion_sound = source.potion_sound();
        let upgrade_glows = source.upgrade_glows();
        let numbers = source.numbers();
        let skill_looks = match source.skill_looks() {
            Ok(looks) => looks
                .into_iter()
                .filter_map(|(vnum, look)| Some((u8::try_from(vnum).ok()?, look)))
                .collect(),
            Err(error) => {
                eprintln!("note: no skill looks: {error:#}");
                BTreeMap::new()
            }
        };
        let camera = Camera {
            target: Vec3::ZERO,
            yaw: args.yaw.to_radians(),
            pitch: args.pitch.clamp(-89.9, 89.9).to_radians(),
            distance: args.distance * 100.0,
            fov_y: Camera::FOV_Y,
        };
        let planned: Vec<(i32, i32)> = args.walk.iter().rev().copied().collect();
        if let Some(line) = args.say.iter().find(|line| line.len() > 512) {
            anyhow::bail!(
                "--say: a chat line is at most 512 bytes, not {}",
                line.len()
            );
        }
        let lines = args.say.clone();
        Ok(Self {
            source,
            atlas,
            items: None,
            radius: args.radius * 100.0,
            map: None,
            walls: None,
            models: Vec::new(),
            by_key: HashMap::new(),
            item_models: HashMap::new(),
            camera,
            fog: args.fog,
            walks: planned.clone(),
            planned,
            says: lines.clone(),
            lines,
            walking_to: None,
            ready_since: None,
            hit: Vec::new(),
            numbers_due: Vec::new(),
            numbers,
            ours_turn: 0,
            skill_looks,
            fighter: Fighter::default(),
            fight_script: args.fight,
            gatherer: Gatherer::default(),
            pick_up_script: args.pick_up,
            tried: BTreeSet::new(),
            talker: Talker::default(),
            shop_script: args.shop,
            talked: BTreeSet::new(),
            left_shop: None,
            left_teacher: None,
            left_smith: None,
            steering: Steering::default(),
            skills: None,
            skill_script: args.skill,
            skill_until: None,
            skill_cooldowns: BTreeMap::new(),
            skill_costs: BTreeMap::new(),
            skill_ranges: BTreeMap::new(),
            charging: None,
            reeled_at: None,
            cooldowns: BTreeMap::new(),
            bodies: Vec::new(),
            item_bodies: Vec::new(),
            item_names: ItemNames::default(),
            my_place: None,
            play_effects: !args.no_effects,
            game_effects,
            experience_flight,
            potion_sound,
            shown: Vec::new(),
            fishing_steps: Vec::new(),
            pointer: None,
            hovered: None,
            upgrade_glows,
            effects: WorldEffects::default(),
            effect_geometry: EffectGeometry::default(),
            hits_shown: Vec::new(),
            first_hit: None,
            sounds: WorldSounds::silent(),
            maps: MapPictures::default(),
            monsters: None,
            names: true,
        })
    }

    /// Notes what the actors react to: who took damage flinches and shows
    /// it. A refused move ends the walk under way: what it went for is not
    /// walked to again from where the server put us (`render.md`, "Walking
    /// around walls").
    fn on_event(&mut self, event: &Event) {
        if let Event::MoveRefused(at) = *event {
            self.walking_to = None;
            self.fighter.refused(at);
            self.gatherer.refused(at);
            self.talker.refused(at);
            self.steering.refused(at);
        }
        if let Event::Shot {
            shooter, target, ..
        } = *event
        {
            self.effects.aim(shooter, target);
        }
        // The experience a kill gave flies from the monster's feet to who
        // gained it.
        if let Event::Flight {
            kind: FlightKind::Experience,
            from,
            to,
        } = *event
            && let Some(path) = self.experience_flight.clone()
            && let Some(feet) = self.bodies.iter().find(|b| b.vid == from).map(|b| b.feet)
            && let Some(files) = effect_files(&self.source, self.play_effects)
        {
            self.effects.launch(&files, &path, feet, to);
        }
        // What happened to an entity shows on it in the next frame, where
        // it stands.
        if let Event::EntityEffect { kind, vid } = *event {
            self.shown.push((vid, kind));
        }
        // A fishing step plays its motion in the next frame.
        if let Event::Fishing { vid, step, float } = *event {
            self.fishing_steps.push((vid, step, float));
        }
        if let Event::Damaged { vid, amount, flags } = *event {
            // A heal, a miss and what poison and fire take neither make it
            // flinch nor show a hit.
            if flags & (DODGED | BLOCKED | HEAL | POISON_HIT | BURNING_HIT) == 0 {
                self.hit.push(vid);
                self.hits_shown.push((vid, flags));
            }
            self.numbers_due.push((vid, amount, flags));
        }
    }

    /// Forgets the world after a logout; the models stay read.
    fn reset(&mut self) {
        self.map = None;
        self.effects.clear();
        self.sounds.clear();
        self.maps.clear();
        self.hits_shown.clear();
        self.shown.clear();
        self.walks.clone_from(&self.planned);
        self.says.clone_from(&self.lines);
        self.walking_to = None;
        self.ready_since = None;
        self.hit.clear();
        self.numbers_due.clear();
        self.fighter = Fighter::default();
        self.gatherer = Gatherer::default();
        self.tried.clear();
        self.talker = Talker::default();
        self.talked.clear();
        self.left_shop = None;
        self.left_teacher = None;
        self.left_smith = None;
        self.steering = Steering::default();
        self.skill_until = None;
        self.cooldowns.clear();
        self.bodies.clear();
        self.item_bodies.clear();
        self.item_names = ItemNames::default();
        self.my_place = None;
    }

    /// Whether our character now stands off the map that was read: a warp
    /// took it to another. The loading screen shows before
    /// that map is read, as after entering.
    fn left_map(&self, world: &World, now: Millis) -> bool {
        let (Some(map), Some(me)) = (&self.map, world.my_entity()) else {
            return false;
        };
        let here = me.track.point(now);
        let inside = u32::try_from(here.x)
            .is_ok_and(|x| u32::try_from(here.y).is_ok_and(|y| map.entry.contains(x, y)));
        !inside
    }

    /// Reads the map under `here` if it is not the one loaded, or if we went
    /// far from where it was read. True if a new map was read.
    fn ensure_map(&mut self, here: Point) -> Result<bool> {
        if let Some(map) = &self.map {
            let inside = u32::try_from(here.x)
                .is_ok_and(|x| u32::try_from(here.y).is_ok_and(|y| map.entry.contains(x, y)));
            if inside && f64::from(self.radius) * 0.5 > map.focus.distance(here) {
                return Ok(false);
            }
        }
        let (Ok(x), Ok(y)) = (u32::try_from(here.x), u32::try_from(here.y)) else {
            bail!("no map of the world holds ({}, {})", here.x, here.y);
        };
        let entry = self
            .atlas
            .iter()
            .find(|e| e.contains(x, y))
            .cloned()
            .with_context(|| format!("no map of the world holds ({x}, {y})"))?;
        let folder = entry.map.as_bytes().to_vec();
        let local = ((x - entry.base[0]) as f32, (y - entry.base[1]) as f32);
        eprintln!(
            "loading {} around ({:.0} m, {:.0} m)",
            entry.name(),
            local.0 / 100.0,
            local.1 / 100.0
        );
        let parts = mapview::MapParts {
            focus: Some(local),
            radius: Some(self.radius),
            objects: true,
            effects: self.play_effects,
        };
        let loaded = AssetMap::open(&self.source.folder, &entry.map)
            .and_then(|map| mapview::load_map(&map, parts, &mut self.effects.library))
            .with_context(|| format!("reading {}", entry.name()))?;
        for note in &loaded.notes {
            eprintln!("note: {note}");
        }
        eprintln!(
            "floors: {} triangles of {} objects",
            loaded.floors.len(),
            loaded.floored
        );
        self.effects.set_map(&loaded.effects);
        self.sounds.set_map(loaded.sounds.clone());
        if self.walls.as_ref().is_none_or(|(read, _)| *read != folder) {
            let walls = self.read_walls(&entry);
            self.walls = Some((folder, walls));
        }
        self.map = Some(PlacedMap {
            entry,
            focus: here,
            loaded,
        });
        Ok(true)
    }

    /// The walls of the map `entry` names from its attributes (`walls.rs`),
    /// or none: then our walks go straight.
    fn read_walls(&self, entry: &WorldMap) -> Option<Walls> {
        let name = readable(entry.name().as_bytes());
        let base = Point::new(
            i32::try_from(entry.base[0]).ok()?,
            i32::try_from(entry.base[1]).ok()?,
        );
        let map = AssetMap::open(&self.source.folder, &entry.map);
        let walk = map
            .as_ref()
            .map_err(|error| anyhow::anyhow!("{error:#}"))
            .and_then(walls::asset_walk_map);
        match walk {
            Ok(Some(walk)) => {
                eprintln!(
                    "walls of {name}: {} of {} × {} squares blocked",
                    walk.blocked_squares(),
                    walk.width(),
                    walk.height()
                );
                // Its water, where fishing casts.
                let water = map
                    .as_ref()
                    .ok()
                    .and_then(|map| walls::asset_water_map(map).ok().flatten());
                Some(Walls::new(walk, base).with_water(water))
            }
            Ok(None) => {
                eprintln!("note: {name} has no attributes: walks go straight");
                None
            }
            Err(error) => {
                eprintln!("note: cannot read the walls of {name} ({error:#}): walks go straight");
                None
            }
        }
    }

    /// The walls of the map we are on, if it has any.
    fn walls(&self) -> Option<&Walls> {
        self.walls.as_ref().and_then(|(_, walls)| walls.as_ref())
    }

    /// Whether our character holds a fishing pole.
    fn holds_pole(&self, world: &World) -> bool {
        world
            .items
            .get(&(Window::Inventory, WORN_FROM + WEAPON_POSITION))
            .and_then(|item| self.items.as_ref()?.get(item.vnum))
            .is_some_and(|info| info.record.kind == ROD)
    }

    /// The way to cast from `from` facing `facing` (degrees): the first of
    /// straight ahead, then 10° to either side in turn up to 180°, whose
    /// point [`FISHING_REACH`] out is on water.
    fn cast_angle(&self, from: Point, facing: u16) -> Option<u16> {
        water_way(self.walls()?, from, facing)
    }

    /// The model for a key, read the first time it is needed.
    fn model_for(&mut self, key: ActorKey) -> Option<usize> {
        if let Some(index) = self.by_key.get(&key) {
            return *index;
        }
        let loaded = match key {
            ActorKey::Player {
                race,
                hair,
                body,
                weapon,
                riding,
            } => actors::load_player(
                &self.source,
                race,
                hair,
                &Look {
                    riding,
                    ..self.look(body, weapon)
                },
                self.skills.as_ref(),
            ),
            ActorKey::Monster(vnum) => actors::load_monster(&self.source, vnum),
        };
        let index = match loaded {
            Ok(model) => {
                for note in &model.notes {
                    eprintln!("note: {key:?}: {note}");
                }
                self.models.push(model);
                Some(self.models.len() - 1)
            }
            Err(error) => {
                eprintln!("note: {key:?} is not drawn: {error:#}");
                None
            }
        };
        self.by_key.insert(key, index);
        index
    }

    /// How item `vnum` lies on the ground: its model, read the first time.
    fn item_shape(&mut self, vnum: u32) -> Option<ItemShape> {
        let key = ground::ground_model(self.items.as_ref(), vnum);
        if let Some(shape) = self.item_models.get(&key) {
            return *shape;
        }
        if key.0.is_empty() {
            self.item_models.insert(key, None);
            return None;
        }
        let shape = match actors::load_item(&self.source.files(), &key.0) {
            Ok(model) => {
                for note in &model.notes {
                    eprintln!("note: {}: {note}", key.0);
                }
                let bounds = model.bounds;
                self.models.push(model);
                let model = self.models.len() - 1;
                bounds.map(|bounds| {
                    let (lay, bounds) = ground::lying(bounds, key.1);
                    ItemShape { model, lay, bounds }
                })
            }
            Err(error) => {
                eprintln!("note: item {vnum} is not drawn: {error:#}");
                None
            }
        };
        self.item_models.insert(key, shape);
        shape
    }

    /// How a body armour and a weapon (item numbers, 0 for none) show,
    /// by the item table and the item list.
    fn look(&self, body: u16, weapon: u16) -> Look {
        let item = |vnum: u16| {
            (vnum != 0)
                .then_some(self.items.as_ref())
                .flatten()
                .and_then(|items| items.get(u32::from(vnum)))
        };
        let shine = |vnum: u16| {
            item(vnum).map_or(0.0, |info| f32::from(info.record.specular.min(100)) / 100.0)
        };
        Look {
            shape: item(body).and_then(|info| body_shape(&info.record)),
            weapon: item(weapon).and_then(|info| {
                let grip = weapon_grip(&info.record)?;
                let model = info.model.as_ref()?;
                Some((model.escape_ascii().to_string(), grip))
            }),
            riding: false,
            shine: [shine(body), shine(weapon)],
        }
    }

    /// A fishing float at world `point` on the map we are on: on the
    /// water's surface there, else the ground.
    fn float_scene(&self, point: Point) -> Option<Vec3> {
        let map = self.map.as_ref()?;
        let sx = (f64::from(point.x) - f64::from(map.entry.base[0])) as f32;
        let sy = -(f64::from(point.y) - f64::from(map.entry.base[1])) as f32;
        let z = map
            .loaded
            .water_at(sx, sy)
            .or_else(|| map.loaded.ground_at(sx, sy))
            .unwrap_or(0.0);
        Some(Vec3::new(sx, sy, z))
    }

    /// A world position (centimetres) on the map we are on, with the
    /// ground's height: the terrain's, or a building's floor over it.
    fn to_scene(&self, x: f64, y: f64) -> Option<Vec3> {
        let map = self.map.as_ref()?;
        let sx = (x - f64::from(map.entry.base[0])) as f32;
        let sy = -(y - f64::from(map.entry.base[1])) as f32;
        let z = map.loaded.ground_at(sx, sy).unwrap_or(0.0);
        Some(Vec3::new(sx, sy, z))
    }

    /// The world position of a point of the scene.
    fn to_world(&self, p: Vec3) -> Option<Point> {
        let map = self.map.as_ref()?;
        let x = f64::from(map.entry.base[0]) + f64::from(p.x);
        let y = f64::from(map.entry.base[1]) - f64::from(p.y);
        Some(Point::new(x.round() as i32, y.round() as i32))
    }

    /// What the minimap shows now: our map position and facing, the
    /// camera, the others alive where they are drawn, and our target.
    fn minimap_view(&self, drawn: &Drawn, world: &World, now: Millis) -> Option<MinimapView> {
        let map = self.map.as_ref()?;
        let me = world.my_entity()?;
        let base = (f64::from(map.entry.base[0]), f64::from(map.entry.base[1]));
        let local = |(x, y): (f64, f64)| (x - base.0, y - base.1);
        let at = |entity: &Entity| {
            let shown = drawn.entities.get(&entity.vid).and_then(|v| v.shown);
            local(shown.unwrap_or_else(|| entity.track.at(now)))
        };
        let marks = world
            .entities
            .values()
            .filter(|e| e.vid != me.vid && !e.dead && !e.shows(EntityState::Hidden))
            .filter_map(|e| Some((at(e), MarkKind::of(e.kind)?)))
            .collect();
        let target = world
            .target
            .filter(|vid| *vid != me.vid)
            .and_then(|vid| world.entities.get(&vid))
            .map(at);
        Some(MinimapView {
            centre: local(me.track.at(now)),
            facing: me.angle,
            camera_yaw: self.camera.yaw,
            marks,
            target,
            blink: now.0 / 500 % 2 == 1,
        })
    }

    /// Brings the map and the actors up to date with the world at `now`,
    /// `seconds` after the last time. False while there is nothing to draw.
    fn update(
        &mut self,
        gpu: &Gpu,
        renderer: &Renderer,
        drawn: &mut Drawn,
        world: &World,
        now: Millis,
        seconds: f32,
    ) -> Result<bool> {
        let Some(me) = world.my_entity() else {
            return Ok(false);
        };
        self.hovered = self
            .pointer
            .and_then(|(x, y, aspect)| self.entity_at(me.vid, aspect, x, y));
        if self.ensure_map(me.track.point(now))? {
            let map = self.map.as_ref().map(|m| &m.loaded);
            if let Some(map) = map {
                drawn.map = Some(GpuMap::upload(gpu, renderer, map, TEXTURE_SPAN)?);
            }
        }
        upload_pending(
            gpu,
            renderer,
            &mut self.effects.library,
            &mut drawn.effect_textures,
        )?;
        drawn
            .entities
            .retain(|vid, _| world.entities.contains_key(vid));
        for (vid, entity) in &world.entities {
            let key = ActorKey::of(entity);
            let look = (entity.mount != 0).then_some(entity.mount);
            let (clock, once, attack_seen, shown, clock_seen) = match drawn.entities.get(vid) {
                Some(view) if view.key == key && view.mount.as_ref().map(|m| m.look) == look => {
                    continue;
                }
                // Other clothes: the motions go on.
                Some(view) => (
                    view.clock,
                    view.once,
                    view.attack_seen,
                    view.shown,
                    view.clock_seen,
                ),
                // Met dead: already lying.
                None => (
                    0.0,
                    entity.dead.then_some((Once::Dead, f32::INFINITY)),
                    None,
                    None,
                    None,
                ),
            };
            let model = self.model_for(key);
            let parts = match model {
                Some(index) => instances(
                    gpu,
                    renderer,
                    &mut drawn.templates,
                    &self.source,
                    &self.models,
                    index,
                )?,
                None => Vec::new(),
            };
            let mount = match look {
                Some(look) => {
                    let model = self.model_for(ActorKey::Monster(look));
                    let parts = match model {
                        Some(index) => instances(
                            gpu,
                            renderer,
                            &mut drawn.templates,
                            &self.source,
                            &self.models,
                            index,
                        )?,
                        None => Vec::new(),
                    };
                    Some(MountView {
                        look,
                        model,
                        parts,
                        clock,
                    })
                }
                None => None,
            };
            drawn.entities.insert(
                *vid,
                DrawnEntity {
                    key,
                    model,
                    parts,
                    clock,
                    once,
                    attack_seen,
                    shown,
                    clock_seen,
                    mount,
                    hidden: false,
                    float: None,
                    then: None,
                },
            );
        }
        let hit = std::mem::take(&mut self.hit);
        let shown = std::mem::take(&mut self.shown);
        let fishing_steps = std::mem::take(&mut self.fishing_steps);
        // How high our character sits on a mount, for the camera.
        let mut my_seat = 0.0;
        for (vid, view) in &mut drawn.entities {
            let (Some(entity), Some(model)) = (
                world.entities.get(vid),
                view.model.and_then(|i| self.models.get(i)),
            ) else {
                continue;
            };
            // A move the server timed is followed; one without a time is
            // run to at the actor's run speed.
            let timed = entity.track.is_moving(now);
            let mine = world.me.as_ref().is_some_and(|me| me.vid == *vid);
            // Another player hidden (Stealth) isn't drawn: neither it nor
            // its effects.
            let hidden = !mine && entity.shows(EntityState::Hidden);
            view.hidden = hidden;
            let pace = f32::from(entity.move_speed.max(1)) / 100.0;
            let run = model.run.as_ref().and_then(|m| m.speed);
            let speed = run.unwrap_or(RUN_SPEED as f32) * pace;
            let follow = timed || mine || entity.dead;
            let step = f64::from(speed * seconds);
            let ((x, y), chasing) = chase(view.shown, entity.track.at(now), follow, step);
            view.shown = Some((x, y));
            let Some(position) = self.to_scene(x, y) else {
                continue;
            };
            let moving = timed || chasing;
            let seed = attack_seed(*vid, entity.track.start);
            // A skill plays at the grade we have it; others' at normal
            // grade (the move does not say theirs).
            let skill = match entity.action {
                MoveKind::Skill(number) => {
                    let grade = world
                        .me
                        .as_ref()
                        .filter(|_| mine)
                        .and_then(|me| skill_of_motion(me.class, number))
                        .and_then(|skill| world.skills.get(&skill))
                        .map_or(0, |skill| skill.grade);
                    Some((number, grade))
                }
                _ => None,
            };
            let acting = matches!(
                entity.action,
                MoveKind::Attack | MoveKind::Combo | MoveKind::Skill(_)
            );
            view.once = next_once(
                view.once,
                &model.fight,
                OnceInput {
                    dead: entity.dead,
                    moving,
                    attack: acting && view.attack_seen != Some(entity.track.start),
                    step: (entity.action == MoveKind::Combo).then_some(entity.argument),
                    skill,
                    since: now.0.saturating_sub(entity.track.start.0) as f32 / 1000.0,
                    hit: hit.contains(vid),
                    attack_seed: seed,
                    flinch_seed: seed ^ now.0,
                },
            );
            if acting {
                view.attack_seen = Some(entity.track.start);
            }
            // Each fishing step plays its motion once, the float where the
            // server put it; a bite during the throw waits for its end.
            let mut bitten = false;
            for (_, step, float) in fishing_steps.iter().filter(|(on, _, _)| on == vid) {
                view.float = self.float_scene(*float);
                let motion = match step {
                    FishingStep::Cast => FishingMotion::Throw,
                    FishingStep::Bite => FishingMotion::React,
                    FishingStep::Caught => FishingMotion::Catch,
                    FishingStep::Lost => FishingMotion::Fail,
                    FishingStep::ReeledIn => FishingMotion::Cancel,
                };
                bitten |= *step == FishingStep::Bite;
                if model.fight.fishing.get(motion).is_none() {
                    continue;
                }
                let throwing = matches!(view.once, Some((Once::Fishing(FishingMotion::Throw), _)));
                if *step == FishingStep::Bite && throwing {
                    view.then = Some(Once::Fishing(motion));
                } else {
                    view.once = Some((Once::Fishing(motion), 0.0));
                    view.then = None;
                }
            }
            let once = view.once.and_then(|(once, time)| {
                let motion = model.fight.get(once)?;
                Some((once, motion, time))
            });
            let (motion, time, key) = match once {
                Some((once, motion, time)) => {
                    // Played out: back to standing (the dead stay down),
                    // or to the motion waiting for its end.
                    let time = time + seconds;
                    let length = motion.animation.duration;
                    view.once = (once == Once::Dead || time < length).then_some((once, time));
                    if view.once.is_none() {
                        view.once = view.then.take().map(|then| (then, 0.0));
                    }
                    // Just before the end: the pose would loop round at it.
                    (
                        Some(motion),
                        time.min((length - 0.001).max(0.0)),
                        once_key(once),
                    )
                }
                None => {
                    let running = moving && model.run.is_some();
                    // With a line out, the fishing wait.
                    let fishing = entity
                        .line
                        .and(model.fight.fishing.wait.as_ref())
                        .filter(|_| !moving);
                    let motion = if running {
                        model.run.as_ref()
                    } else {
                        fishing.or(model.wait.as_ref())
                    };
                    // Run as fast as the entity moves, so the feet keep up.
                    let rate = match (moving, motion.and_then(|m| m.speed)) {
                        (true, _) if chasing => pace.clamp(0.25, 4.0),
                        (true, Some(speed)) => {
                            let track = &entity.track;
                            let moved = track.from.distance(track.to) as f32;
                            let per_second = moved / (track.duration.max(1) as f32 / 1000.0);
                            (per_second / speed).clamp(0.25, 4.0)
                        }
                        _ => 1.0,
                    };
                    view.clock += seconds * rate;
                    (motion, view.clock, u32::from(running))
                }
            };
            let mut place = placement(position, entity.angle);
            let mut time = time;
            // Riding: the mount under it, the rider at its seat, the
            // rider's standing and running in step with the mount's.
            let mount = view.mount.as_mut().and_then(|mount| {
                let mount_model = mount.model.and_then(|i| self.models.get(i))?;
                Some((mount, mount_model))
            });
            if let Some((mount, mount_model)) = mount {
                let running = moving && mount_model.run.is_some();
                let mount_motion = if running {
                    mount_model.run.as_ref()
                } else {
                    mount_model.wait.as_ref()
                };
                let rate = match (moving, mount_motion.and_then(|m| m.speed)) {
                    (true, _) if chasing => pace.clamp(0.25, 4.0),
                    (true, Some(speed)) => {
                        let track = &entity.track;
                        let moved = track.from.distance(track.to) as f32;
                        let per_second = moved / (track.duration.max(1) as f32 / 1000.0);
                        (per_second / speed).clamp(0.25, 4.0)
                    }
                    _ => 1.0,
                };
                mount.clock += seconds * rate;
                if let Some(pose) = mount_model.pose(mount_motion, mount.clock) {
                    for (part, palette) in mount.parts.iter().zip(mount_model.palettes_of(&pose)) {
                        let placed: Vec<Mat4> = palette.iter().map(|m| place * *m).collect();
                        renderer.set_palette(gpu, part, &placed);
                    }
                    let seat = actors::seat(mount_model, &pose);
                    place *= Mat4::from_translation(seat);
                    if mine {
                        my_seat = seat.z;
                    }
                }
                let mount_length = mount_motion.map_or(0.0, |m| m.animation.duration);
                let rider_length = motion.map_or(0.0, |m| m.animation.duration);
                if once.is_none() && mount_length > 0.0 {
                    time = mount.clock.rem_euclid(mount_length) / mount_length * rider_length;
                }
            }
            let pose = model.pose(motion, time);
            if let Some(pose) = &pose {
                for (part, palette) in view.parts.iter().zip(model.palettes_of(pose)) {
                    let placed: Vec<Mat4> = palette.iter().map(|m| place * *m).collect();
                    renderer.set_palette(gpu, part, &placed);
                }
            }
            let now_clock = MotionClock { key, time };
            if hidden {
                self.effects.forget(*vid);
            } else if let Some(files) = effect_files(&self.source, self.play_effects) {
                self.effects.actor(
                    &files,
                    *vid,
                    place,
                    pose.as_ref(),
                    &model.attached,
                    model.smoke.as_ref().map(|smoke| (smoke, entity.health)),
                    motion,
                    view.clock_seen,
                    now_clock,
                    view.float,
                );
                // The loops of the states it shows, and the glows of the
                // upgraded items a player wears.
                let mut loops: Vec<Loop> = STATE_EFFECTS
                    .iter()
                    .filter(|(state, _, _)| entity.shows(*state))
                    .filter_map(|(_, effect, bone)| {
                        let path = self.game_effects.get(*effect as usize).cloned()??;
                        Some(Loop {
                            path,
                            bone: bone.map(RawStr::from),
                            turns: bone.is_some(),
                        })
                    })
                    .collect();
                // The skills on a player, while they are on, as the
                // original client attaches them: some only
                // while it moves, the weapon's another with a two-handed
                // one.
                let two_handed = match (view.key, self.items.as_ref()) {
                    (ActorKey::Player { weapon, .. }, Some(items)) => {
                        glows::two_handed(items, u32::from(weapon))
                    }
                    _ => false,
                };
                loops.extend(entity.skills.iter().filter_map(|skill| {
                    let look = self.skill_looks.get(skill)?;
                    if look.moving && !moving {
                        return None;
                    }
                    let path = look.two_handed.as_ref().filter(|_| two_handed);
                    Some(Loop {
                        path: path.unwrap_or(&look.effect).clone(),
                        bone: look.bone.clone(),
                        turns: look.bone.is_some(),
                    })
                }));
                // The rings at the feet of the living entity under the
                // pointer and of our target, as the original client
                // attaches them.
                let rings = [
                    (self.hovered, GameEffect::Hovered),
                    (world.target, GameEffect::Targeted),
                ];
                loops.extend(
                    rings
                        .iter()
                        .filter(|(on, _)| !entity.dead && *on == Some(*vid))
                        .filter_map(|(_, ring)| self.game_effects.get(*ring as usize).cloned())
                        .flatten()
                        .map(Loop::at_feet),
                );
                if let (ActorKey::Player { body, weapon, .. }, Some(items)) =
                    (view.key, self.items.as_ref())
                {
                    let held: Vec<_> = model.held.iter().flatten().cloned().collect();
                    loops.extend(glows::worn(
                        &self.upgrade_glows,
                        items,
                        u32::from(body),
                        u32::from(weapon),
                        &held,
                    ));
                }
                self.effects
                    .loops(&files, *vid, place, pose.as_ref(), &loops);
                // What happened to it plays on it once, moving with it, as
                // the original client attaches its effects.
                for (_, kind) in shown.iter().filter(|(on, _)| on == vid) {
                    let effect = match kind {
                        EntityEffectKind::HealthPotion => GameEffect::HealthPotion,
                        EntityEffectKind::ManaPotion => GameEffect::ManaPotion,
                        EntityEffectKind::LevelUp => GameEffect::LevelUp,
                    };
                    if let Some(path) = self.game_effects.get(effect as usize).cloned().flatten() {
                        self.effects.once(&files, *vid, place, &path);
                    }
                }
                // A fish over the head of a fisher a fish bit, as the
                // original client shows it.
                let fish = self
                    .game_effects
                    .get(GameEffect::FishBite as usize)
                    .cloned()
                    .flatten();
                if let (true, Some(path)) = (bitten, fish) {
                    let head = model.bounds.map_or(HEAD_HEIGHT, |b| b.max.z);
                    self.effects.once_over(&files, *vid, place, &path, head);
                }
            }
            if let Some(motion) = motion {
                self.sounds.motion(
                    &self.source.files(),
                    motion,
                    position,
                    view.clock_seen,
                    now_clock,
                );
            }
            // Our own potions are heard, once a drink, as the original
            // client plays its sound for its own character only.
            let drank = shown.iter().any(|(on, kind)| {
                on == vid
                    && matches!(
                        kind,
                        EntityEffectKind::HealthPotion | EntityEffectKind::ManaPotion
                    )
            });
            if mine
                && drank
                && let Some(sound) = &self.potion_sound
            {
                self.sounds.play_at(&self.source.files(), sound, position);
            }
            view.clock_seen = Some(now_clock);
        }
        let me_vid = me.vid;
        self.bodies = world
            .entities
            .values()
            .filter(|entity| entity.vid == me_vid || !entity.shows(EntityState::Hidden))
            .filter_map(|entity| {
                let view = drawn.entities.get(&entity.vid);
                let at = view.and_then(|v| v.shown);
                self.body(entity, at.unwrap_or_else(|| entity.track.at(now)))
            })
            .collect();
        self.update_ground(gpu, renderer, drawn, world)?;
        let (x, y) = me.track.at(now);
        let position = self.to_scene(x, y);
        self.my_place = position.map(|p| placement(p, me.angle));
        if let Some(position) = position {
            self.camera.target = position + Vec3::Z * (EYE_HEIGHT + my_seat);
            self.sounds.listen(position, self.camera.view().right);
        }
        self.show_hits();
        self.show_numbers(world);
        // What monsters shot flies to where hits show: 60% of the target's
        // height up (`show_hits`).
        if let Some(files) = effect_files(&self.source, self.play_effects) {
            let targets: Vec<(u32, Vec3)> = self
                .bodies
                .iter()
                .map(|b| (b.vid, b.feet + Vec3::Z * (b.height * 0.6)))
                .collect();
            self.effects.fly(&files, seconds, &targets);
        }
        let target = self.camera.target;
        let alive: Vec<u32> = drawn.entities.keys().copied().collect();
        self.effects.advance(seconds, target, EFFECT_REACH, &alive);
        if let Some(files) = effect_files(&self.source, self.play_effects) {
            for (effect, at) in self.effects.take_started() {
                self.sounds.effect(&files, &effect, at);
            }
        }
        self.sounds.advance(&self.source.files(), seconds);
        self.effect_geometry.clear();
        self.effects.draw(
            &self.camera.view(),
            target,
            EFFECT_REACH,
            &mut self.effect_geometry,
        );
        drawn.effects = Some(renderer.upload_effects(gpu, &self.effect_geometry)?);
        Ok(true)
    }

    /// A hit effect on each entity that took damage since the last frame
    /// (not on a miss): the effect's own centre at its body's surface on
    /// the side of the one who hit (us, or for hits on us the camera), 60%
    /// of its height up, turned so what flies off flies away from them
    /// (openm2's choice).
    fn show_hits(&mut self) {
        let mine = self.my_place.map(|m| m.w_axis.truncate());
        for (vid, flags) in std::mem::take(&mut self.hits_shown) {
            if flags & (DODGED | BLOCKED) != 0 {
                continue;
            }
            let Some(body) = self.bodies.iter().find(|b| b.vid == vid) else {
                continue;
            };
            let ours = mine.is_some_and(|m| m.distance(body.feet) < 1.0);
            let from = match mine.filter(|_| !ours) {
                Some(me) => me,
                None => self.camera.eye(),
            };
            let away = (body.feet - from) * Vec3::new(1.0, 1.0, 0.0);
            let away = away.try_normalize().unwrap_or(Vec3::Y);
            let centre = body.feet + Vec3::Z * (body.height * 0.6) - away * body.radius;
            // The effect's −Y along `away`.
            let turn = Mat4::from_rotation_z(away.y.atan2(away.x) + std::f32::consts::FRAC_PI_2);
            let effect = if flags & CRITICAL != 0 {
                GameEffect::Critical
            } else {
                GameEffect::Hit
            };
            let effect = self.game_effects.get(effect as usize).cloned().flatten();
            if let (Some(files), Some(effect)) =
                (effect_files(&self.source, self.play_effects), effect)
            {
                self.effects.play_centred(&files, &effect, centre, turn);
            }
            self.sounds.hit(&self.source.files(), centre);
            self.first_hit.get_or_insert_with(Instant::now);
        }
    }

    /// The numbers of the damage taken since the last frame, as the
    /// original client draws them: a hit's amount over the
    /// head of us or of our target, an effect a digit played once with that
    /// digit's picture, the digits side by side along the screen, the last
    /// over the head; a miss over whoever it names. Each is turned to the
    /// camera, as the original turns them.
    fn show_numbers(&mut self, world: &World) {
        let due = std::mem::take(&mut self.numbers_due);
        let (Some(numbers), Some(files)) = (
            self.numbers.as_ref(),
            effect_files(&self.source, self.play_effects),
        ) else {
            return;
        };
        let me = world.me.as_ref().map(|me| me.vid);
        let right = self.camera.view().right;
        // Facing the camera: x along the screen to the right, y away from
        // the camera, z up.
        let away = Vec3::Z.cross(right);
        let place = |at: Vec3| {
            Mat4::from_cols(
                right.extend(0.0),
                away.extend(0.0),
                Vec3::Z.extend(0.0),
                at.extend(1.0),
            )
        };
        for (vid, amount, flags) in due {
            let Some(body) = self.bodies.iter().find(|b| b.vid == vid) else {
                continue;
            };
            let head = body.feet + Vec3::Z * body.height;
            let ours = Some(vid) == me;
            let (effect, pictures, amount) =
                match number(amount, flags, ours, world.target == Some(vid)) {
                    None => continue,
                    Some(miss @ (Number::OurMiss | Number::TheirMiss)) => {
                        let effect = if miss == Number::OurMiss {
                            &numbers.miss
                        } else {
                            &numbers.target_miss
                        };
                        if let Some(effect) = effect {
                            self.effects.play_placed(&files, effect, place(head));
                        }
                        continue;
                    }
                    Some(Number::Ours(amount)) => {
                        let Some(effect) =
                            numbers.ours.get(self.ours_turn % numbers.ours.len().max(1))
                        else {
                            continue;
                        };
                        self.ours_turn = self.ours_turn.wrapping_add(1);
                        (effect, &numbers.ours_digits, amount)
                    }
                    Some(Number::Target(amount)) => {
                        let Some(effect) = &numbers.target else {
                            continue;
                        };
                        (effect, &numbers.target_digits, amount)
                    }
                };
            for (place_value, digit) in digits(amount).into_iter().enumerate() {
                let Some(picture) = pictures.get(digit) else {
                    continue;
                };
                let at = head - right * (DIGIT_WIDTH * place_value as f32);
                self.effects
                    .play_retextured(&files, effect, picture, place(at));
            }
        }
    }

    /// An entity as something to pick and hit: where it is drawn (`x`, `y`
    /// in world centimetres), and its model's size and collision spheres
    /// when standing, turned as it faces. None for what has no body
    /// ([`has_body`]).
    fn body(&self, entity: &Entity, (x, y): (f64, f64)) -> Option<Body> {
        if !has_body(entity.kind) {
            return None;
        }
        let feet = self.to_scene(x, y)?;
        let model = self
            .by_key
            .get(&ActorKey::of(entity))
            .copied()
            .flatten()
            .and_then(|index| self.models.get(index));
        let (radius, height) = model.and_then(|m| m.bounds).map_or(NO_MODEL_SIZE, |b| {
            let size = b.max - b.min;
            ((size.x + size.y) * 0.25, b.max.z)
        });
        let place = placement(feet, entity.angle);
        let spheres = model.map_or_else(Vec::new, |m| {
            m.spheres
                .iter()
                .map(|s| HitSphere {
                    centre: place.transform_point3(s.centre),
                    radius: s.radius,
                })
                .collect()
        });
        Some(Body {
            vid: entity.vid,
            feet,
            radius: radius.max(20.0),
            height: height.max(50.0),
            spheres,
            foe: can_fight(entity),
        })
    }

    /// Brings the items on the ground up to date: a model for each new one,
    /// lying on its spot and turned its way; and what can be picked of them.
    fn update_ground(
        &mut self,
        gpu: &Gpu,
        renderer: &Renderer,
        drawn: &mut Drawn,
        world: &World,
    ) -> Result<()> {
        drawn.ground.retain(|vid, view| {
            world
                .ground
                .get(vid)
                .is_some_and(|item| item.vnum == view.vnum)
        });
        for (vid, item) in &world.ground {
            if drawn.ground.contains_key(vid) {
                continue;
            }
            let shape = self.item_shape(item.vnum);
            let parts = match shape {
                Some(shape) => instances(
                    gpu,
                    renderer,
                    &mut drawn.templates,
                    &self.source,
                    &self.models,
                    shape.model,
                )?,
                None => Vec::new(),
            };
            let vnum = item.vnum;
            drawn.ground.insert(*vid, DrawnItem { vnum, shape, parts });
        }
        let mut bodies = Vec::new();
        let mut spots = Vec::new();
        for (vid, view) in &drawn.ground {
            let Some(item) = world.ground.get(vid) else {
                continue;
            };
            let Some(spot) = self.to_scene(f64::from(item.at.x), f64::from(item.at.y)) else {
                continue;
            };
            bodies.push(item_body(*vid, spot, view.shape));
            spots.push((*vid, spot));
            let (Some(shape), Some(model)) = (
                view.shape,
                view.shape.and_then(|s| self.models.get(s.model)),
            ) else {
                continue;
            };
            let place = Mat4::from_translation(spot)
                * Mat4::from_rotation_z(ground::turn(*vid))
                * shape.lay;
            for (part, palette) in view.parts.iter().zip(model.palettes(None, 0.0)) {
                let placed: Vec<Mat4> = palette.iter().map(|m| place * *m).collect();
                renderer.set_palette(gpu, part, &placed);
            }
        }
        self.item_bodies = bodies;
        if let Some(files) = effect_files(&self.source, self.play_effects) {
            let effect = self
                .game_effects
                .get(GameEffect::Item as usize)
                .and_then(Option::as_deref);
            self.effects.items(&files, effect, &spots);
        }
        Ok(())
    }

    /// A frame of picking up: `--pick-up` goes for the nearest item ours to
    /// take once the says, walks and fights are done; the walks and the
    /// pick-ups go to the session.
    fn gather(&mut self, world: &World, now: Millis, remote: &Remote) {
        let Some(me) = world.my_entity() else {
            return;
        };
        if self.pick_up_script
            && self.ready_since.is_some()
            && !me.dead
            && self.fighter.foe().is_none()
            && self.gatherer.item().is_none()
            && let Some(vid) = ground::nearest_item(world, me.track.point(now), &self.tried)
        {
            println!("picking up vid {vid}");
            self.tried.insert(vid);
            self.gatherer.go_for(vid);
        }
        let walls = self.walls.as_ref().and_then(|(_, walls)| walls.as_ref());
        for command in self.gatherer.frame(world, now, walls) {
            remote.send(command);
        }
    }

    /// A frame of talking: `--shop` goes to the nearest NPC once the says,
    /// walks, fights and pick-ups are done; the walk and the talk go to the
    /// session. Walking away from an open shop closes it.
    fn talk(&mut self, world: &World, now: Millis, remote: &Remote) {
        let Some(me) = world.my_entity() else {
            return;
        };
        if self.shop_script
            && self.ready_since.is_some()
            && !me.dead
            && self.fighter.foe().is_none()
            && self.gatherer.item().is_none()
            && self.talker.npc().is_none()
            && world.shop.is_none()
            && self.talked.is_empty()
            && let Some(vid) = shop::nearest_npc(world, me.track.point(now), now, &self.talked)
        {
            println!("talking to vid {vid}");
            self.talked.insert(vid);
            self.talker.go_to(vid);
        }
        let walls = self.walls.as_ref().and_then(|(_, walls)| walls.as_ref());
        for command in self.talker.frame(world, now, walls) {
            remote.send(command);
        }
        let open = world.shop.as_ref().map(|shop| shop.vid);
        if open.is_none() {
            self.left_shop = None;
        }
        if open.is_some() && open != self.left_shop && shop::walked_away(world, now) {
            self.left_shop = open;
            remote.send(Command::CloseShop);
        }
        let teacher = world.teacher.map(|teacher| teacher.vid);
        if teacher.is_none() {
            self.left_teacher = None;
        }
        if let Some(vid) = teacher
            && teacher != self.left_teacher
            && shop::walked_away_from(world, now, vid)
        {
            self.left_teacher = teacher;
            remote.send(Command::CloseTeacher);
        }
        let smith = world.upgrade.as_ref().and_then(|offer| offer.smith);
        if smith.is_none() {
            self.left_smith = None;
        }
        if let Some(vid) = smith
            && smith != self.left_smith
            && shop::walked_away_from(world, now, vid)
        {
            self.left_smith = smith;
            remote.send(Command::CloseUpgrade);
        }
    }

    /// A frame of walking with the movement keys (`steer.rs`): they lead
    /// our character when nothing else does (a fight, a pick-up, a talk, a
    /// skill's motion, the scripted walks).
    fn steer(&mut self, world: &World, now: Millis, remote: &Remote) {
        let none = FightMotions::default();
        let fight = world
            .my_entity()
            .and_then(|me| self.by_key.get(&ActorKey::of(me)).copied().flatten())
            .and_then(|index| self.models.get(index))
            .map_or(&none, |model| &model.fight);
        let others = self.fighter.leads(fight, now)
            || self.gatherer.item().is_some()
            || self.talker.npc().is_some()
            || self.skill_until.is_some_and(|until| now < until)
            || self.ready_since.is_none();
        let walls = self.walls.as_ref().and_then(|(_, walls)| walls.as_ref());
        for command in self
            .steering
            .frame(world, now, self.camera.yaw, walls, others)
        {
            remote.send(command);
        }
    }

    /// What using skill `skill` asks of the session (`docs/specs/ui.md`,
    /// "Skills"): its motion, facing `foe` (else our target) if it can be
    /// fought, or the player it goes on; nothing while our character is
    /// dead or the motion of the skill used last still plays. Else why it
    /// cannot be used: not learned, the wrong weapon or none, no foe for a
    /// skill that needs one, its target too far, not enough mana. A skill used on oneself (`CAN_USE_FOR_ME`) is used on our
    /// character, one for others (`ONLY_FOR_ALLIANCE`) on the player we
    /// chose, else on us. A charging skill (`CHARGE_ATTACK`) on a foe out
    /// of reach but within [`CHARGE_FAR`] runs up to it first: the walk is
    /// asked for, and [`Game::charge`] uses the skill there.
    fn use_skill(
        &mut self,
        world: &World,
        now: Millis,
        skill: u8,
        foe: Option<u32>,
    ) -> Result<Option<Command>, String> {
        let Some(me) = world.my_entity().filter(|me| !me.dead) else {
            return Ok(None);
        };
        let (grade, level) = match world.skills.get(&skill) {
            Some(known) if known.level > 0 => (known.grade, known.level),
            _ => return Err("You have not learned this skill yet.".to_owned()),
        };
        if let Some((_, until)) = self.cooldowns.get(&skill).filter(|(_, until)| now < *until) {
            let seconds = until.since(now).div_ceil(1_000);
            return Err(format!("The skill is ready again in {seconds} s."));
        }
        if self.skill_until.is_some_and(|until| now < until) {
            return Ok(None);
        }
        if self.holds_pole(world) {
            return Err("Not with a fishing pole in hand.".to_owned());
        }
        let line = self.skills.as_ref().and_then(|s| s.get(u32::from(skill)));
        let weapon = world
            .items
            .get(&(Window::Inventory, WORN_FROM + WEAPON_POSITION))
            .and_then(|item| self.items.as_ref()?.get(item.vnum))
            .and_then(|info| weapon_grip(&info.record).map(|_| info.record.subtype));
        if let Some(line) = line
            && !weapon_allows(line, weapon)
        {
            return Err(match weapon {
                None => "This skill needs a weapon.",
                Some(_) => "This skill cannot be used with this weapon.",
            }
            .to_owned());
        }
        let flag = |flag: &str| line.is_some_and(|line| line.has_flag(flag));
        let reach = self
            .skill_ranges
            .get(&skill)
            .map_or(0.0, |metres| metres * 100.0)
            .max(SKILL_REACH);
        let cost = self
            .skill_costs
            .get(&skill)
            .map(|cost| skills::work_out(cost, world, level));
        let here = me.track.point(now);
        let target = match aim_skill(world, now, me, &flag, foe, reach, cost)? {
            Aim::At(target) => target,
            Aim::Charge { foe, to } => {
                let path = walls::path(self.walls(), here, to);
                if path.is_empty() {
                    return Err("Too far away.".to_owned());
                }
                self.charging = Some(Charge {
                    skill,
                    foe,
                    until: now.plus(CHARGE_TIME),
                });
                return Ok(Some(Command::MoveTo { path }));
            }
        };
        let angle = target
            .filter(|vid| *vid != me.vid)
            .and_then(|vid| world.entities.get(&vid))
            .map_or(me.angle, |them| here.angle_to(them.track.point(now)));
        // Not again until its motion played out.
        let length = skill_motion(skill)
            .zip(self.my_model(me))
            .and_then(|(number, model)| {
                let once = model.fight.skill_for(number, grade)?;
                Some(model.fight.get(once)?.animation.duration)
            })
            .unwrap_or(0.0);
        self.skill_until = Some(now.plus((f64::from(length) * 1000.0) as u64));
        self.start_cooldown(world, now, skill, level);
        self.fighter.used_skill(now);
        self.charging = None;
        Ok(Some(Command::UseSkill {
            skill,
            angle,
            target,
            motion: None,
            to: None,
        }))
    }

    /// A frame of a charging skill on its way: once our
    /// character is within [`CHARGE_NEAR`] of its foe, the skill is used;
    /// a foe gone or dead, our death or [`CHARGE_TIME`] passing ends it.
    pub(crate) fn charge(&mut self, world: &World, now: Millis, remote: &Remote) {
        let Some(charge) = self.charging else {
            return;
        };
        let (Some(me), Some(foe)) = (world.my_entity(), world.entities.get(&charge.foe)) else {
            self.charging = None;
            return;
        };
        if me.dead || foe.dead || now > charge.until {
            self.charging = None;
            return;
        }
        if me.track.point(now).distance(foe.track.point(now)) > CHARGE_NEAR {
            return;
        }
        self.charging = None;
        match self.use_skill(world, now, charge.skill, Some(charge.foe)) {
            Ok(Some(command)) => {
                remote.send(command);
            }
            Ok(None) => {}
            Err(why) => println!("skill {} not used: {why}", charge.skill),
        }
    }

    /// Starts skill `skill`'s cooldown, at `level`: its formula in the
    /// client's skill table, worked out with openm2's skill power and our
    /// points, as the server does (`docs/specs/ui.md`, "Skills"). No
    /// formula, no cooldown.
    fn start_cooldown(&mut self, world: &World, now: Millis, skill: u8, level: u8) {
        let Some(formula) = self.skill_cooldowns.get(&skill) else {
            return;
        };
        let seconds = skills::work_out(formula, world, level);
        if let Ok(seconds) = u64::try_from(seconds)
            && seconds > 0
        {
            self.cooldowns
                .insert(skill, (now, now.plus(seconds.saturating_mul(1_000))));
        }
    }

    /// The cooldowns under way at `now`, as the icons show them; those
    /// over are forgotten.
    fn cooldowns_at(&mut self, now: Millis) -> BTreeMap<u8, Cooldown> {
        self.cooldowns.retain(|_, (_, until)| now < *until);
        self.cooldowns
            .iter()
            .map(|(skill, (from, until))| {
                let left = until.since(now);
                let whole = until.since(*from).max(1);
                let cooldown = Cooldown {
                    left: u16::try_from(left * 1_000 / whole).unwrap_or(1_000),
                    seconds: u32::try_from(left.div_ceil(1_000)).unwrap_or(u32::MAX),
                };
                (*skill, cooldown)
            })
            .collect()
    }

    /// Our character's model, if it was read.
    fn my_model(&self, me: &Entity) -> Option<&ActorModel> {
        self.by_key
            .get(&ActorKey::of(me))
            .copied()
            .flatten()
            .and_then(|index| self.models.get(index))
    }

    /// A frame of `--skill`: once the says, walks, fights, pick-ups and
    /// talks are done and the server told our skills, uses the skill at
    /// the nearest monster, once. Why it could not is printed.
    fn skill_script(&mut self, world: &World, now: Millis, remote: &Remote) {
        let Some(skill) = self.skill_script else {
            return;
        };
        let Some(me) = world.my_entity() else {
            return;
        };
        let Some(ready) = self.ready_since else {
            return;
        };
        if me.dead
            || self.fighter.foe().is_some()
            || self.gatherer.item().is_some()
            || self.talker.npc().is_some()
            || (world.skills.is_empty() && ready.elapsed() < SKILL_WAIT)
        {
            return;
        }
        self.skill_script = None;
        let foe = fight::nearest_foe(world, me.track.point(now), now, me.vid);
        if let Some(vid) = foe
            && world.target != Some(vid)
        {
            remote.send(Command::Target { vid });
        }
        match self.use_skill(world, now, skill, foe) {
            Ok(Some(command)) => {
                println!("using skill {skill}");
                remote.send(command);
            }
            Ok(None) => println!("skill {skill} not used"),
            Err(why) => println!("skill {skill} not used: {why}"),
        }
    }

    /// A frame of our fighting: `--fight` goes for the nearest monster once
    /// the says and walks are done; the swings and hits go to the session.
    fn fight(&mut self, world: &World, now: Millis, remote: &Remote) {
        let (Some(me), Some(place)) = (world.my_entity(), self.my_place) else {
            return;
        };
        let Some(model) = self
            .by_key
            .get(&ActorKey::of(me))
            .copied()
            .flatten()
            .and_then(|index| self.models.get(index))
        else {
            return;
        };
        if self.fight_script
            && self.ready_since.is_some()
            && !me.dead
            && self.fighter.foe().is_none()
            && self.gatherer.item().is_none()
            && let Some(vid) = fight::nearest_foe(world, me.track.point(now), now, me.vid)
        {
            println!("fighting vid {vid}");
            self.fighter.press_on(vid);
        }
        let stance = Stance {
            fight: &model.fight,
            place,
            bodies: &self.bodies,
            now,
            walls: self.walls.as_ref().and_then(|(_, walls)| walls.as_ref()),
        };
        for command in self.fighter.frame(world, &stance) {
            remote.send(command);
        }
    }

    /// What a press at a spot of the view (`x`, `y` from −1 to 1, y up)
    /// is on (`picked`): an item on the ground under it, the item whose
    /// name is there (`named`), or an entity under it but our own
    /// character.
    fn picked_at(
        &self,
        me: u32,
        aspect: f32,
        (x, y): (f32, f32),
        named: Option<u32>,
    ) -> Option<Picked> {
        let (origin, direction) = self.camera.ray(aspect, x, y);
        let item = fight::pick(&self.item_bodies, origin, direction, me).map(|(_, vid)| vid);
        picked(item, named, || self.entity_at(me, aspect, x, y))
    }

    /// The nearest entity under a spot of the view (`x`, `y` from −1 to 1,
    /// y up) but our own character.
    fn entity_at(&self, me: u32, aspect: f32, x: f32, y: f32) -> Option<u32> {
        let (origin, direction) = self.camera.ray(aspect, x, y);
        fight::pick(&self.bodies, origin, direction, me).map(|(_, vid)| vid)
    }

    /// The names over characters' heads (`nametags.rs`) with a game
    /// master's mark, the names over the items on the ground and the damage
    /// numbers over heads, for a view of `width` × `height` with the
    /// interface at `scale`, at `now` (the marks fade); and which of them
    /// name items.
    fn labels(
        &self,
        world: &World,
        (width, height): (u32, u32),
        scale: f32,
        now: Millis,
    ) -> (Vec<Label>, ItemNames) {
        let (w, h) = (width as f32, height.max(1) as f32);
        let view_proj = self.camera.view_proj(w / h);
        // Where a point of the scene is on the screen, if in front.
        let screen = |point: Vec3| {
            let clip = view_proj * point.extend(1.0);
            if clip.w <= 0.0 {
                return None;
            }
            let ndc = clip.truncate() / clip.w;
            Some(((ndc.x + 1.0) * 0.5 * w, (1.0 - ndc.y) * 0.5 * h))
        };
        // Items lying together: their names over the tallest, stacked.
        let spots: Vec<Vec3> = self.item_bodies.iter().map(|b| b.feet).collect();
        let stacks = ground::stacked(&spots);
        let mut tops = vec![0.0f32; spots.len()];
        for (body, (first, _)) in self.item_bodies.iter().zip(&stacks) {
            if let Some(top) = tops.get_mut(*first) {
                *top = top.max(body.height);
            }
        }
        let (vids, names): (Vec<u32>, Vec<Label>) = self
            .item_bodies
            .iter()
            .zip(&stacks)
            .filter_map(|(body, &(first, line))| {
                let item = world.ground.get(&body.vid)?;
                let feet = self.item_bodies.get(first)?.feet;
                let top = tops.get(first)?;
                let (x, y) = screen(feet + Vec3::Z * (top + ITEM_NAME_HEIGHT))?;
                let label = Label {
                    at: (x, y - line as f32 * ITEM_NAME_LINE * scale),
                    text: ground::item_name(self.items.as_ref(), item.vnum),
                    colour: ground::name_colour(item, world),
                    size: 0.0,
                    lead: None,
                    image: None,
                };
                Some((body.vid, label))
            })
            .unzip();
        let me = world.me.as_ref().map(|me| me.vid);
        // Names over the characters near ours, the farthest first so that
        // nearer ones are drawn over them.
        let here = me
            .and_then(|vid| self.bodies.iter().find(|b| b.vid == vid))
            .map_or(self.camera.target, |b| b.feet);
        let mut tagged: Vec<_> = self
            .bodies
            .iter()
            .filter(|_| self.names)
            .filter_map(|body| {
                let distance = here.distance(body.feet);
                if distance > NAME_REACH {
                    return None;
                }
                let entity = world.entities.get(&body.vid)?;
                let tag = nametags::name_tag(entity, self.monsters.as_ref())?;
                Some((distance, body, tag))
            })
            .collect();
        tagged.sort_by(|a, b| b.0.total_cmp(&a.0));
        let tags = tagged.into_iter().flat_map(|(_, body, tag)| {
            let Some(at) = screen(body.feet + Vec3::Z * (body.height + NAME_HEIGHT)) else {
                return Vec::new();
            };
            let mark = tag
                .master
                .then(|| nametags::mark((at.0, at.1 - NAME_LINE * scale), now.0));
            let name = Label {
                at,
                lead: tag.lead(),
                text: tag.name,
                colour: tag.colour,
                size: 0.0,
                image: None,
            };
            std::iter::once(name).chain(mark).collect()
        });
        let mut labels: Vec<Label> = tags.collect();
        let item_names = ItemNames {
            first: labels.len(),
            vids,
        };
        labels.extend(names);
        (labels, item_names)
    }

    /// Says the `--say` lines once in the world, then starts the next
    /// scripted walk once the last one ended; notes when the walks are done
    /// (from then on, fighting may move us).
    fn walk(&mut self, world: &World, now: Millis, remote: &Remote) {
        let Some(me) = world.my_entity() else { return };
        for text in self.says.drain(..) {
            remote.send(Command::Chat {
                kind: ChatKind::Talk,
                text,
            });
        }
        let idle = !me.track.is_moving(now) && self.walking_to.is_none_or(|to| me.track.to == to);
        if self.ready_since.is_some() || !idle || self.map.is_none() {
            return;
        }
        let Some((dx, dy)) = self.walks.pop() else {
            self.ready_since.get_or_insert_with(Instant::now);
            return;
        };
        let from = me.track.point(now);
        let to = Point::new(
            from.x.saturating_add(dx.saturating_mul(100)),
            from.y.saturating_add(dy.saturating_mul(100)),
        );
        let speed = world.run_speed;
        let path = walls::path(self.walls(), from, to);
        let Some(&end) = path.last() else {
            println!(
                "not walking from ({}, {}) to ({}, {}): walls all round",
                from.x, from.y, to.x, to.y
            );
            return;
        };
        let short = if end == to {
            String::new()
        } else {
            format!(", as near to ({}, {}) as the walls let us", to.x, to.y)
        };
        println!(
            "walking from ({}, {}) to ({}, {}) at {speed} cm/s, {} leg{}{short}",
            from.x,
            from.y,
            end.x,
            end.y,
            path.len(),
            if path.len() == 1 { "" } else { "s" }
        );
        self.walking_to = Some(end);
        self.ready_since = None;
        remote.send(Command::MoveTo { path });
    }

    /// The point on the ground under a spot of the view (`x`, `y` from −1
    /// to 1, y up).
    fn ground_at(&self, aspect: f32, x: f32, y: f32) -> Option<Point> {
        let map = self.map.as_ref()?;
        let (origin, direction) = self.camera.ray(aspect, x, y);
        let hit = map.loaded.pick(origin, direction, REACH)?;
        self.to_world(hit)
    }

    fn scene(&self) -> Option<Scene> {
        let map = self.map.as_ref()?;
        Some(mapview::scene(&map.loaded, self.camera, self.fog))
    }

    /// A line for the window title in the game.
    fn status(&self, world: &World, now: Millis) -> Option<String> {
        let (me, map) = (world.me.as_ref()?, self.map.as_ref()?);
        let here = world
            .my_entity()
            .map(|e| e.track.point(now))
            .unwrap_or_default();
        Some(format!(
            "{} on {} ({:.0} m, {:.0} m), {} around",
            shown(&me.name),
            map.entry.name(),
            (f64::from(here.x) - f64::from(map.entry.base[0])) / 100.0,
            (f64::from(here.y) - f64::from(map.entry.base[1])) / 100.0,
            world.entities.len().saturating_sub(1)
        ))
    }
}

/// What starts or ends an entity's one-shot motion this frame.
#[derive(Debug, Clone, Copy)]
struct OnceInput {
    /// It is dead.
    dead: bool,
    /// It walks.
    moving: bool,
    /// An attack move came that has not played yet.
    attack: bool,
    /// The attack's combo step, from 1, for a combo move.
    step: Option<u8>,
    /// For a skill move: its skill motion number and the grade to play.
    skill: Option<(u8, u8)>,
    /// Seconds since the attack move started: its motion starts that far
    /// in (an attack made long before we saw it is over at once).
    since: f32,
    /// It took damage.
    hit: bool,
    /// For picking an attack among its variants ([`attack_seed`]: the same
    /// for the picture and our hits).
    attack_seed: u64,
    /// For picking a flinch.
    flinch_seed: u64,
}

/// The one-shot motion an entity plays from now (`docs/specs/render.md`,
/// "Playing"):
/// - dead, it falls once and stays down; back alive, it stands again;
/// - a new attack or skill move starts the attack's or the skill's motion,
///   as far in as the move is old;
/// - damage makes it flinch, unless it is attacking;
/// - walking ends any of them.
fn next_once(
    current: Option<(Once, f32)>,
    fight: &FightMotions,
    input: OnceInput,
) -> Option<(Once, f32)> {
    let falling = matches!(current, Some((Once::Dead, _)));
    if input.dead {
        return if falling {
            current
        } else {
            fight.dead.as_ref().map(|_| (Once::Dead, 0.0))
        };
    }
    let current = current.filter(|(once, _)| *once != Once::Dead && !input.moving);
    if input.attack {
        let once = match input.skill {
            Some((number, grade)) => fight.skill_for(number, grade),
            None => fight.attack_for(input.step, input.attack_seed),
        };
        return once.map(|once| (once, input.since)).or(current);
    }
    let attacking = current.is_some_and(|(once, _)| once.is_attack());
    if input.hit && !attacking && !input.moving {
        return fight
            .damage_for(input.flinch_seed)
            .map(|once| (once, 0.0))
            .or(current);
    }
    current
}

/// Everything drawn this frame.
fn draw_lists(drawn: &Drawn) -> (Vec<&GpuArea>, Vec<&GpuPart>) {
    let areas = drawn
        .map
        .as_ref()
        .map(|m| m.areas.iter().collect())
        .unwrap_or_default();
    let mut parts: Vec<&GpuPart> = drawn
        .map
        .as_ref()
        .map(|m| m.parts.iter().collect())
        .unwrap_or_default();
    parts.extend(drawn.entities.values().filter(|e| !e.hidden).flat_map(|e| {
        e.parts
            .iter()
            .chain(e.mount.iter().flat_map(|mount| mount.parts.iter()))
    }));
    parts.extend(drawn.ground.values().flat_map(|i| i.parts.iter()));
    (areas, parts)
}

/// The character on the select and create screens, drawn into a picture
/// that the interface shows (`@character`).
#[derive(Default)]
struct Showcase {
    picture: Option<Offscreen>,
    /// The character shown: race, hair and body armour.
    shown: Option<(u16, u16, u16)>,
    model: Option<usize>,
    parts: Vec<GpuPart>,
    camera: Option<Camera>,
    /// Seconds into its wait motion.
    clock: f32,
    /// True once the character the screen asks for was drawn.
    ready: bool,
}

/// A camera on a model's front that keeps all of it in view while it plays
/// its wait motion, with some room around it.
fn frame_model(model: &ActorModel, aspect: f32) -> Option<Camera> {
    let mut camera = Camera::framing(model.bounds?, 0.0, SHOWCASE_PITCH.to_radians(), aspect);
    camera.distance *= 1.1;
    Some(camera)
}

/// What a frame came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameEnd {
    Continue,
    Quit,
}

/// What the window and `--screenshot` both run: the session, the screens,
/// the world and the character on the select screen, a frame at a time.
struct Client<'a> {
    args: &'a PlayArgs,
    game: Game,
    interface: PlayInterface,
    connection: Option<Connection>,
    drawn: Drawn,
    showcase: Showcase,
    /// The loading screen was drawn once the game began: the map loads next.
    loading_drawn: bool,
    /// Since when the select (or create) screen is up.
    choosing_since: Option<Instant>,
    /// Why the last session ended, if it failed.
    last_failure: Option<String>,
    /// Logins so far.
    logins: u32,
    /// A line for the window title.
    title: String,
    /// The server's providers, once it says ([`ask_providers`]).
    providers: Option<mpsc::Receiver<Result<Vec<SignInProvider>, String>>>,
    /// The last frame waited for the browser of a sign-in.
    signing_in: bool,
    /// The browser came back (or the sign-in ended): the window comes to
    /// the front at the next frame.
    come_back: bool,
    /// The player's settings, as the Settings window shows them.
    settings: Settings,
    /// The display's scale factor, once the window is open.
    display: Option<f64>,
    /// Since when the settings changed without being written.
    unsaved: Option<Instant>,
    /// The game's own server, the Servers board's first row, if there is
    /// one: its name and where it is.
    official: Option<(String, ServerAddress)>,
    /// The player's servers, the board's other rows, and their file.
    book: ServersFile,
    book_path: Option<PathBuf>,
    /// The server joined: the one a sign-in goes to.
    server: Option<NetConfig>,
    /// How the board's servers answer: (round, row, answer), the answers
    /// of earlier rounds dropped.
    pings: Option<mpsc::Receiver<(u32, usize, Reach)>>,
    ping_round: u32,
    /// The check of a server being joined: its row, what it found.
    checking: Option<mpsc::Receiver<(usize, Check)>>,
}

/// The skills' cooldown and mana cost formulas and their targets' ranges
/// (metres), by skill (`docs/specs/ui.md`, "Skills").
type SkillNumbers = (
    BTreeMap<u8, Formula>,
    BTreeMap<u8, Formula>,
    BTreeMap<u8, f64>,
);

/// [`SkillNumbers`] from the source's skills file.
fn load_skill_numbers(source: &Source) -> Result<SkillNumbers> {
    let (numbers, from) = source.skill_numbers()?;
    let formulas = |list: &[(u32, String)]| -> BTreeMap<u8, Formula> {
        list.iter()
            .filter_map(|(vnum, text)| {
                let skill = u8::try_from(*vnum).ok()?;
                let formula = Formula::parse(text).ok()?;
                Some((skill, formula))
            })
            .collect()
    };
    let cooldowns = formulas(&numbers.cooldowns);
    let costs = formulas(&numbers.costs);
    let ranges = numbers
        .ranges
        .iter()
        .filter_map(|(vnum, metres)| Some((u8::try_from(*vnum).ok()?, *metres)))
        .collect();
    eprintln!("skill cooldowns: {} from {from}", cooldowns.len());
    eprintln!("skill mana costs: {} from {from}", costs.len());
    Ok((cooldowns, costs, ranges))
}

/// The files effects are read from, the asset folder's, unless effects are
/// off (`--no-effects`).
fn effect_files(source: &Source, on: bool) -> Option<Files<'_>> {
    on.then(|| source.files())
}

impl<'a> Client<'a> {
    fn new(args: &'a PlayArgs) -> Result<Self> {
        let mut game = Game::new(args)?;
        let mut interface = PlayInterface::new(game.source.skin()?, !args.no_interface)?;
        let settings = player_settings(args);
        interface.set_settings(settings);
        interface
            .ui
            .set_scale(args.scale.scale(None) * settings.interface);
        match game.source.items() {
            Ok(items) => {
                game.items = Some(items.clone());
                if !args.no_interface {
                    interface.set_items(items);
                }
            }
            Err(error) => eprintln!(
                "note: items: {error:#}; worn items are not drawn, items on the ground show as bags, the inventory shows no names or icons"
            ),
        }
        match game.source.skills() {
            Ok(skills) => {
                if !args.no_interface {
                    interface.set_skills(skills.clone());
                }
                game.skills = Some(skills);
            }
            Err(error) => eprintln!(
                "note: skills: {error:#}; characters play no skill motions, the skills window shows numbers"
            ),
        }
        match load_skill_numbers(&game.source) {
            Ok((cooldowns, costs, ranges)) => {
                if !args.no_interface {
                    interface.set_skill_costs(costs.clone());
                }
                game.skill_cooldowns = cooldowns;
                game.skill_costs = costs;
                game.skill_ranges = ranges;
            }
            Err(error) => eprintln!(
                "note: skill table: {error:#}; skills show no cooldown and cost no mana here"
            ),
        }
        let monsters = game.source.monsters();
        game.monsters = Some(monsters.clone());
        if !args.no_interface {
            interface.set_monsters(monsters);
        }
        game.names = settings.names;
        let official = args
            .official
            .clone()
            .map(|name| (name, args.server.address()));
        let book_path =
            crate::servers::file_path(args.servers.as_deref(), args.settings.as_deref());
        let book = book_path
            .as_deref()
            .map(ServersFile::load)
            .unwrap_or_default();
        Ok(Self {
            args,
            game,
            interface,
            connection: None,
            drawn: Drawn::default(),
            showcase: Showcase::default(),
            loading_drawn: false,
            choosing_since: None,
            last_failure: None,
            logins: 0,
            title: String::new(),
            providers: None,
            signing_in: false,
            come_back: false,
            settings,
            display: None,
            unsaved: None,
            official,
            book,
            book_path,
            server: None,
            pings: None,
            ping_round: 0,
            checking: None,
        })
    }

    /// The interface's scale: the display's (or `--ui-scale`) times the
    /// player's interface size.
    fn apply_scale(&mut self) {
        let scale = self.args.scale.scale(self.display) * self.settings.interface;
        self.interface.ui.set_scale(scale);
    }

    /// The player changed the settings in the game: they apply at once and
    /// are written to the player's file soon ([`Client::save_settings`]).
    fn change_settings(&mut self, settings: Settings) {
        self.settings = settings;
        self.game.sounds.set_volumes(settings.volumes());
        self.game.names = settings.names;
        self.apply_scale();
        self.unsaved.get_or_insert_with(Instant::now);
    }

    /// Writes changed settings to the player's file, if there is one: half
    /// a second after they changed (a slider dragged writes it a few times,
    /// not every frame), or at once with `now`.
    fn save_settings(&mut self, now: bool) {
        let Some(since) = self.unsaved else {
            return;
        };
        if !now && since.elapsed() < SAVE_SETTINGS_AFTER {
            return;
        }
        self.unsaved = None;
        if let Some(path) = &self.args.settings
            && let Err(error) = self.settings.write(path)
        {
            eprintln!("note: the settings were not saved: {error:#}");
        }
    }

    /// Loads the interface's art and pipeline for targets of `format`.
    fn attach(&mut self, gpu: &Gpu, format: wgpu::TextureFormat) -> Result<()> {
        let filter = self.args.scale.filter();
        self.interface
            .attach(gpu, format, &self.game.source.files(), filter)
    }

    /// Starts a session, unless one runs.
    fn connect(&mut self, mut config: SessionConfig, character: CharacterChoice) {
        if self.connection.is_some() {
            return;
        }
        config.character = character;
        self.reset_world();
        self.last_failure = None;
        let Some(net) = self.server.clone() else {
            self.interface.fail("Join a server first.");
            return;
        };
        self.logins += 1;
        match Connection::start(self.args, net, config, self.logins) {
            Ok(connection) => self.connection = Some(connection),
            Err(error) => {
                let text = format!("{error:#}");
                eprintln!("error: {text}");
                self.interface.fail(&text);
                self.last_failure = Some(text);
            }
        }
    }

    /// The character choice of a login from the window: `--slot`, else the
    /// select screen.
    fn choice(&self) -> CharacterChoice {
        self.args
            .slot
            .map_or(CharacterChoice::Ask, CharacterChoice::Slot)
    }

    /// The Servers board's rows: the game's own server, then the player's.
    fn server_rows(&self) -> Vec<ServerRow> {
        let official = self.official.iter().map(|(name, address)| ServerRow {
            name: name.clone(),
            address: address.clone(),
            official: true,
        });
        let saved = self.book.servers.iter().map(|server| ServerRow {
            name: server.name.clone(),
            address: server.address.clone(),
            official: false,
        });
        official.chain(saved).collect()
    }

    /// The index in the player's file of the board's row `at`; none for
    /// the game's own server.
    fn saved_index(&self, at: usize) -> Option<usize> {
        match self.official {
            Some(_) => at.checked_sub(1),
            None => Some(at),
        }
    }

    /// Writes the board's rows, picking `picked` (else the one joined
    /// last, else the first), and asks every server how it answers.
    fn show_servers(&mut self, picked: Option<usize>) {
        let rows = self.server_rows();
        let last = self.book.last.as_deref();
        let picked = picked
            .or_else(|| {
                rows.iter()
                    .position(|row| Some(row.address.place().as_str()) == last)
            })
            .or_else(|| (!rows.is_empty()).then_some(0));
        self.interface.set_server_rows(rows, picked);
        self.ping_servers();
    }

    /// Asks every server of the board how it answers, at most 8 at once,
    /// each on its own, the answers coming as they come.
    fn ping_servers(&mut self) {
        self.ping_round = self.ping_round.wrapping_add(1);
        self.interface.servers_asking();
        let round = self.ping_round;
        let jobs: Vec<(usize, ServerAddress)> = self
            .server_rows()
            .into_iter()
            .map(|row| row.address)
            .enumerate()
            .collect();
        let jobs = Arc::new(Mutex::new(jobs.into_iter()));
        let (tx, rx) = mpsc::channel();
        for _ in 0..PING_THREADS {
            let (jobs, tx) = (Arc::clone(&jobs), tx.clone());
            let started = thread::Builder::new()
                .name("openm2 ping".to_owned())
                .spawn(move || {
                    loop {
                        let job = jobs.lock().ok().and_then(|mut jobs| jobs.next());
                        let Some((at, address)) = job else {
                            return;
                        };
                        let reach = crate::servers::ping(&address, address.trust());
                        if tx.send((round, at, reach)).is_err() {
                            return;
                        }
                    }
                });
            if let Err(error) = started {
                eprintln!("note: cannot ask the servers how they answer: {error}");
                break;
            }
        }
        self.pings = Some(rx);
    }

    /// The board's answers: how its servers answered, and what joining one
    /// found.
    fn follow_servers(&mut self) {
        if let Some(pings) = &self.pings {
            let answers: Vec<_> = pings.try_iter().collect();
            for (round, at, reach) in answers {
                if round == self.ping_round {
                    self.interface.set_server_state(at, &reach.text());
                }
            }
        }
        let Some(found) = self.checking.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.checking = None;
        self.interface.set_checking(false);
        let (at, check) = found;
        let name = self
            .interface
            .server_row(at)
            .map(|row| crate::interface::printable(&row.name))
            .unwrap_or_default();
        match check {
            Check::Trusted(trust) => self.enter_server(at, trust),
            Check::Ask(key) => {
                self.interface.server_status("");
                self.interface.ask_trust(at, key);
            }
            Check::KeyChanged(why) => {
                eprintln!("note: {name}: {why}");
                self.interface.server_status(&format!(
                    "The key of {name} changed. If its owner says so, remove it and add it again."
                ));
            }
            Check::Failed(why) => self
                .interface
                .server_status(&crate::screens::sentence(&why)),
        }
    }

    /// What the Servers board asked for.
    fn server_action(&mut self, action: ServerAction) {
        match action {
            ServerAction::Join(at) => self.join(at),
            ServerAction::Save { at, name, address } => {
                let server = SavedServer { name, address };
                let saved = match at {
                    // A change: only of one of the player's servers.
                    Some(at) => {
                        let Some(index) = self.saved_index(at) else {
                            return;
                        };
                        let Some(old) = self.book.servers.get_mut(index) else {
                            return;
                        };
                        *old = server;
                        index
                    }
                    None if self.book.servers.len() >= crate::servers::MAX_SERVERS => {
                        self.interface.server_status(&format!(
                            "The list holds at most {} servers.",
                            crate::servers::MAX_SERVERS
                        ));
                        return;
                    }
                    None => {
                        self.book.servers.push(server);
                        self.book.servers.len() - 1
                    }
                };
                self.save_servers();
                let row = saved + usize::from(self.official.is_some());
                self.show_servers(Some(row));
            }
            ServerAction::Remove(at) => {
                if let Some(index) = self.saved_index(at)
                    && index < self.book.servers.len()
                {
                    self.book.servers.remove(index);
                    self.save_servers();
                    self.show_servers(None);
                }
            }
            ServerAction::Refresh => self.ping_servers(),
            ServerAction::Trust { at, key, yes } => {
                if !yes {
                    return;
                }
                let Some(index) = self.saved_index(at) else {
                    return;
                };
                if let Some(server) = self.book.servers.get_mut(index) {
                    server.address.key = Some(key);
                    self.save_servers();
                    self.show_servers(Some(at));
                    self.enter_server(at, Trust::Pinned(key));
                }
            }
            ServerAction::Quit => {}
        }
    }

    /// Joins the board's row `at`: checks its server on a thread of its
    /// own first (`follow_servers` carries on).
    fn join(&mut self, at: usize) {
        if self.checking.is_some() {
            return;
        }
        let Some(row) = self.interface.server_row(at).cloned() else {
            return;
        };
        self.interface.server_status(&format!(
            "Reaching {}...",
            crate::interface::printable(&row.name)
        ));
        self.interface.set_checking(true);
        let (tx, rx) = mpsc::channel();
        // The game's own server is never asked about a new key: it is
        // trusted as its settings say, or not at all.
        let may_ask = !row.official;
        let started = thread::Builder::new()
            .name("openm2 join".to_owned())
            .spawn(move || {
                let _ = tx.send((at, crate::servers::check(&row.address, may_ask)));
            });
        match started {
            Ok(_) => self.checking = Some(rx),
            Err(error) => {
                self.interface.set_checking(false);
                self.interface
                    .server_status(&format!("Cannot reach the server: {error}."));
            }
        }
    }

    /// The board's row `at` is the server to sign in to, trusted by
    /// `trust`: its Sign in board, and its providers asked for.
    fn enter_server(&mut self, at: usize, trust: Trust) {
        let Some(row) = self.interface.server_row(at).cloned() else {
            return;
        };
        let net = self.net_of(&row.address, trust);
        self.server = Some(net.clone());
        let place = row.address.place();
        self.book.last = Some(place.clone());
        self.save_servers();
        self.interface.join_server(&row.name, &place);
        self.interface.ask_providers();
        self.providers = Some(ask_providers(net));
    }

    /// Where to connect for a server at `address` trusted by `trust`, with
    /// what the provider shows at a sign-in (the settings file's).
    fn net_of(&self, address: &ServerAddress, trust: Trust) -> NetConfig {
        let mut net = address.net_config(trust);
        net.sign_in_prompt = self.args.server.sign_in_prompt;
        net
    }

    /// Writes the player's servers file, if there is one.
    fn save_servers(&mut self) {
        if let Some(path) = &self.book_path
            && let Err(error) = self.book.write(path)
        {
            eprintln!("note: the servers were not saved: {error:#}");
        }
    }

    /// The game's own server is the one to sign in to, as its settings
    /// trust it (`--provider` and `--screenshot`, which skip the board).
    fn enter_official(&mut self) -> Result<()> {
        let Some((name, address)) = self.official.clone() else {
            bail!(
                "--provider and --screenshot sign in to the game's server: give --host, or `host` in the settings file (openm2.toml)"
            );
        };
        let net = self.net_of(&address, address.trust());
        self.server = Some(net);
        self.interface.join_server(&name, &address.place());
        Ok(())
    }

    /// Logs out (if still in) and shows why the session failed, if it did.
    fn end_session(&mut self) {
        if let Some(mut connection) = self.connection.take()
            && let Some(text) = connection.finish()
        {
            eprintln!("error: {text}");
            self.interface.fail(&text);
            self.last_failure = Some(text);
        }
        self.reset_world();
    }

    /// Draws the character the screen shows, if it shows one, into the
    /// picture the interface shows.
    fn draw_showcase(
        &mut self,
        gpu: &Gpu,
        renderer: &Renderer,
        (width, height): (u32, u32),
        seconds: f32,
    ) -> Result<()> {
        let Some(rect) = self.interface.picture_rect(width, height) else {
            return Ok(());
        };
        let showcase = &mut self.showcase;
        let size = (rect.w as u32, rect.h as u32);
        if showcase
            .picture
            .as_ref()
            .is_none_or(|p| p.attachments().size() != size)
        {
            let picture = Offscreen::new(gpu, renderer, size.0, size.1)?;
            self.interface.set_picture(gpu, picture.view(), size);
            showcase.picture = Some(picture);
            showcase.camera = None;
        }
        let wanted = self.interface.model();
        if wanted != showcase.shown {
            showcase.shown = wanted;
            showcase.model = wanted.and_then(|(race, hair, body)| {
                self.game.model_for(ActorKey::Player {
                    race,
                    hair,
                    body,
                    weapon: 0,
                    riding: false,
                })
            });
            showcase.parts = match showcase.model {
                Some(index) => instances(
                    gpu,
                    renderer,
                    &mut self.drawn.templates,
                    &self.game.source,
                    &self.game.models,
                    index,
                )?,
                None => Vec::new(),
            };
            showcase.camera = None;
            showcase.clock = 0.0;
        }
        let Some(picture) = &showcase.picture else {
            return Ok(());
        };
        let model = showcase.model.and_then(|index| self.game.models.get(index));
        if showcase.camera.is_none() {
            showcase.camera = model.and_then(|m| frame_model(m, size.0 as f32 / size.1 as f32));
        }
        showcase.clock += seconds;
        let mut parts: Vec<&GpuPart> = Vec::new();
        if let (Some(model), Some(_)) = (model, showcase.camera) {
            let motion = model.wait.as_ref();
            for (part, palette) in showcase
                .parts
                .iter()
                .zip(model.palettes(motion, showcase.clock))
            {
                renderer.set_palette(gpu, part, &palette);
            }
            parts = showcase.parts.iter().collect();
        }
        let mut scene = Scene::new(showcase.camera.unwrap_or(self.game.camera));
        // See-through around the character: the screen's background shows.
        scene.background = [0.0; 4];
        renderer.draw(gpu, picture.attachments(), picture.view(), &scene, &parts)?;
        showcase.ready = !parts.is_empty() || wanted.is_none();
        Ok(())
    }

    /// The session's world now, and the time, handing on the events the
    /// snapshot took.
    fn world_now(&mut self) -> Option<(World, Millis)> {
        let snapshot = self.connection.as_ref()?.snapshot().ok()?;
        for event in &snapshot.events {
            self.interface.on_event(event);
            self.game.on_event(event);
        }
        Some((snapshot.world, snapshot.now))
    }

    /// Draws the minimap and, while it is open, the atlas, for a `width` ×
    /// `height` screen, when what they show changed.
    fn update_maps(
        &mut self,
        gpu: &Gpu,
        world: &World,
        now: Millis,
        (width, height): (u32, u32),
    ) -> Result<()> {
        let Some(map) = &self.game.map else {
            return Ok(());
        };
        let folder = map.entry.map.as_bytes().to_vec();
        let name = readable(map.entry.name().as_bytes());
        let areas = map.entry.size;
        let base = (f64::from(map.entry.base[0]), f64::from(map.entry.base[1]));
        let scale = self.interface.ui.scale();
        if let Some(rect) = self.interface.minimap_rect(width, height)
            && let Some(view) = self.game.minimap_view(&self.drawn, world, now)
            && let Some(image) = self.game.maps.minimap(
                &self.game.source.files(),
                self.interface.ui.skins(),
                &folder,
                areas,
                &view,
                rect.w.round() as u32,
                self.interface.minimap_zoom(),
                scale,
            )
        {
            self.interface.set_minimap(gpu, &image)?;
        }
        if let Some(rect) = self.interface.atlas_rect(width, height) {
            let me = world.my_entity().map(|me| {
                let (x, y) = me.track.at(now);
                ((x - base.0, y - base.1), me.angle)
            });
            let side = f64::from(map.entry.area);
            let map_cm = (f64::from(areas[0]) * side, f64::from(areas[1]) * side);
            let size = (rect.w.round() as u32, rect.h.round() as u32);
            if let Some(image) = self.game.maps.atlas(
                &self.game.source,
                self.interface.ui.skins(),
                &folder,
                map_cm,
                me,
                size,
                scale,
            ) {
                self.interface.set_atlas(gpu, &image, &name)?;
            }
        }
        Ok(())
    }

    fn reset_world(&mut self) {
        self.game.reset();
        self.drawn.map = None;
        self.drawn.entities.clear();
        self.drawn.ground.clear();
        self.loading_drawn = false;
    }

    /// One frame: follows the session, carries out what the player asked
    /// for, and draws into `view`.
    fn frame(
        &mut self,
        gpu: &Gpu,
        renderer: &Renderer,
        attachments: &Attachments,
        view: &wgpu::TextureView,
        seconds: f32,
    ) -> Result<FrameEnd> {
        let size = attachments.size();
        let snapshot = self
            .connection
            .as_ref()
            .map(Connection::snapshot)
            .transpose()?;
        let (world, now, stage) = match snapshot {
            Some(snapshot) => {
                for event in &snapshot.events {
                    self.interface.on_event(event);
                    self.game.on_event(event);
                    if *event == Event::LeftGame {
                        // Changing character: the next one's map comes
                        // as after a login.
                        self.reset_world();
                    }
                }
                if snapshot.finished {
                    // The session ended on its own: an error, or the server
                    // let us go.
                    self.end_session();
                    (World::default(), Millis(0), None)
                } else {
                    (snapshot.world, snapshot.now, Some(snapshot.stage))
                }
            }
            None => (World::default(), Millis(0), None),
        };

        // Taken to another map (a warp): its map loads as after entering.
        if stage == Some(Stage::InGame) && self.game.left_map(&world, now) {
            self.game.map = None;
            self.drawn.map = None;
            self.loading_drawn = false;
        }
        // In the game, the map loads once the loading screen was drawn.
        let mut ready = false;
        if stage == Some(Stage::InGame) && (self.game.map.is_some() || self.loading_drawn) {
            ready = self
                .game
                .update(gpu, renderer, &mut self.drawn, &world, now, seconds)?;
            if ready && let Some(connection) = &self.connection {
                self.game.walk(&world, now, &connection.remote);
                self.game.fight(&world, now, &connection.remote);
                self.game.charge(&world, now, &connection.remote);
                self.game.gather(&world, now, &connection.remote);
                self.game.talk(&world, now, &connection.remote);
                self.game.skill_script(&world, now, &connection.remote);
                self.game.steer(&world, now, &connection.remote);
            }
        }
        let progress = match stage {
            None | Some(Stage::LoggedOut | Stage::Failed) => Progress::Offline,
            Some(Stage::Idle | Stage::Auth) => Progress::Connecting,
            Some(Stage::SigningIn) => Progress::SigningIn,
            Some(Stage::Joining) if world.selected.is_none() => Progress::Connecting,
            Some(Stage::Joining) => Progress::Loading(0.25),
            Some(Stage::Selecting) => Progress::Choosing,
            Some(Stage::Creating) => Progress::Creating,
            Some(Stage::Loading) => Progress::Loading(0.5),
            Some(Stage::InGame) if ready => Progress::Playing,
            Some(Stage::InGame) => Progress::Loading(0.8),
            Some(Stage::Leaving) => Progress::Leaving,
        };
        let signing_in = progress == Progress::SigningIn;
        self.come_back |= self.signing_in && !signing_in;
        self.signing_in = signing_in;
        self.interface.set_cooldowns(self.game.cooldowns_at(now));
        self.interface.set_time(now);
        self.interface.follow(progress, &world);
        self.follow_servers();
        if let Some(providers) = &self.providers
            && let Ok(result) = providers.try_recv()
        {
            match result {
                Ok(list) => self.interface.set_providers(list),
                // A sign-in under way says how it went itself.
                Err(why) if self.connection.is_none() => self.interface.fail(&why),
                Err(_) => {}
            }
            self.providers = None;
        }
        let (labels, item_names) = if ready {
            self.game
                .labels(&world, size, self.interface.ui.scale(), now)
        } else {
            Default::default()
        };
        self.game.item_names = item_names;
        self.interface.set_labels(labels);

        for action in self.interface.take_actions() {
            match action {
                Action::LogIn(config) => self.connect(config, self.choice()),
                Action::Command(command) => {
                    if let Some(connection) = &self.connection {
                        connection.remote.send(command);
                    }
                }
                Action::UseSkill(skill) => {
                    if let Some(connection) = &self.connection {
                        match self.game.use_skill(&world, now, skill, None) {
                            Ok(Some(command)) => {
                                connection.remote.send(command);
                            }
                            Ok(None) => {}
                            Err(why) => self.interface.notice(&why),
                        }
                    }
                }
                Action::ToggleRide => {
                    if let Some(connection) = &self.connection {
                        let ride = !world.riding();
                        connection.remote.send(Command::Ride { ride });
                    }
                }
                Action::Back => self.end_session(),
                Action::Servers(action) => self.server_action(action),
                Action::ToServers => {
                    self.providers = None;
                    self.server = None;
                    self.show_servers(None);
                }
                Action::Quit => return Ok(FrameEnd::Quit),
                Action::Settings(settings) => self.change_settings(settings),
            }
        }

        let screen = self.interface.screen();
        let music = match screen {
            Screen::Login => Some(Music::Login),
            Screen::Select | Screen::Create => Some(Music::Select),
            Screen::Game => Some(Music::Game),
            // Between the select screen and the game, what played goes on.
            Screen::Loading => None,
        };
        if let Some(music) = music {
            self.game.sounds.music(&self.game.source, Some(music));
        }
        if ready && screen == Screen::Game {
            self.update_maps(gpu, &world, now, size)?;
        }
        match (screen, self.game.scene(), &self.drawn.map) {
            (Screen::Game, Some(scene), Some(map)) if ready => {
                let (areas, parts) = draw_lists(&self.drawn);
                let world = WorldDraw {
                    effects: self.drawn.effects.as_ref(),
                    effect_textures: &self.drawn.effect_textures,
                    // The clouds drift on the session's clock.
                    seconds: (now.0 as f64 / 1000.0) as f32,
                    ..map.world(&areas, &parts)
                };
                renderer.draw_world(gpu, attachments, view, &scene, &world)?;
            }
            _ => {
                if matches!(screen, Screen::Select | Screen::Create) {
                    self.draw_showcase(gpu, renderer, size, seconds)?;
                }
                // Behind the screens' own background.
                renderer.draw(gpu, attachments, view, &Scene::new(self.game.camera), &[])?;
            }
        }
        self.interface
            .draw(gpu, &self.game.source.files(), view, size)?;

        if stage == Some(Stage::InGame) {
            self.loading_drawn = true;
        }
        if matches!(screen, Screen::Select | Screen::Create) {
            self.choosing_since.get_or_insert_with(Instant::now);
        } else {
            self.choosing_since = None;
        }
        self.save_settings(false);
        self.title = match screen {
            Screen::Game => self
                .game
                .status(&world, now)
                .unwrap_or_else(|| "in game".to_owned()),
            Screen::Login => "sign in".to_owned(),
            Screen::Select => "choosing a character".to_owned(),
            Screen::Create => "making a character".to_owned(),
            Screen::Loading => "loading".to_owned(),
        };
        Ok(FrameEnd::Continue)
    }
}

/// How long changed settings wait before they are written.
const SAVE_SETTINGS_AFTER: Duration = Duration::from_millis(500);

/// Runs `play`.
pub fn run(mut args: PlayArgs) -> Result<ExitCode> {
    let (config, named) = args.server.resolve_for_list()?;
    if named {
        args.official = Some(
            config
                .server_name
                .clone()
                .unwrap_or_else(|| args.server.host.clone()),
        );
    }
    args.assets = args.assets_arg.clone().or(config.assets);
    if args.assets.is_none() {
        bail!(
            "which asset folder? Give --assets, or `assets` in the settings file (openm2.toml); `openm2 assets import` makes one of your client"
        );
    }
    args.provider = args.provider.or(config.provider);
    if !(args.radius > 0.0 && args.radius.is_finite()) {
        bail!("--radius must be more than 0 metres");
    }
    if !(args.stay.is_finite() && args.stay >= 0.0) {
        bail!("--stay must be a number of seconds, 0 or more");
    }
    let mut client = Client::new(&args)?;
    let options = GpuOptions {
        software: args.software,
        ..GpuOptions::default()
    };
    let result = match &args.screenshot {
        Some(out) => screenshot(&mut client, options, out),
        None => window(&mut client, options),
    };
    client.save_settings(true);
    client.end_session();
    result?;
    match (&args.screenshot, client.last_failure) {
        (Some(_), Some(error)) => {
            eprintln!("error: {error}");
            Ok(ExitCode::FAILURE)
        }
        _ => Ok(ExitCode::SUCCESS),
    }
}

/// Signs in with `--provider` for `--screenshot`, in the system's browser.
fn screenshot_login(client: &mut Client<'_>, character: CharacterChoice) -> Result<()> {
    let Some(provider) = &client.args.provider else {
        bail!("--screenshot needs --provider to sign in");
    };
    let config = SessionConfig::provider(provider).context("--provider")?;
    client.enter_official()?;
    client.connect(config, character);
    if let Some(error) = &client.last_failure {
        bail!("{error}");
    }
    Ok(())
}

/// Plays without a window until the picture is taken. The GPU comes first:
/// without one, nothing signs in.
fn screenshot(client: &mut Client<'_>, options: GpuOptions, out: &Path) -> Result<()> {
    let args = client.args;
    let gpu = Gpu::headless(options)?;
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT))?;
    match args.screen {
        Shot::Login => client.show_servers(None),
        Shot::Select => screenshot_login(client, CharacterChoice::Ask)?,
        Shot::Game => {
            let character = args
                .slot
                .map_or(CharacterChoice::First, CharacterChoice::Slot);
            screenshot_login(client, character)?;
        }
    }
    client.attach(&gpu, OFFSCREEN_FORMAT)?;
    let (width, height) = args.size;
    let target = Offscreen::new(&gpu, &renderer, width, height)?;
    let stay = Duration::from_secs_f64(args.stay);
    let started = Instant::now();
    let mut last = Instant::now();
    loop {
        if args.inventory && client.interface.screen() == Screen::Game {
            client.interface.open_inventory();
        }
        if args.atlas && client.interface.screen() == Screen::Game {
            client.interface.open_atlas();
        }
        if args.skills && client.interface.screen() == Screen::Game {
            client.interface.open_character(Tab::Skills);
        }
        if args.character && client.interface.screen() == Screen::Game {
            client.interface.open_character(Tab::Status);
        }
        let seconds = last.elapsed().as_secs_f32();
        last = Instant::now();
        client.frame(
            &gpu,
            &renderer,
            target.attachments(),
            target.view(),
            seconds,
        )?;
        if args.screen != Shot::Login && client.connection.is_none() {
            bail!(
                "no picture: {}",
                client
                    .last_failure
                    .clone()
                    .unwrap_or_else(|| "the session ended".to_owned())
            );
        }
        let screen = client.interface.screen();
        let done = match args.screen {
            Shot::Login => true,
            Shot::Select => {
                client.showcase.ready
                    && client
                        .choosing_since
                        .is_some_and(|since| since.elapsed() >= stay)
            }
            Shot::Game => {
                let waited = match args.after_hit {
                    Some(after) => client
                        .game
                        .first_hit
                        .is_some_and(|hit| hit.elapsed().as_secs_f64() >= after),
                    None => client
                        .game
                        .ready_since
                        .is_some_and(|since| since.elapsed() >= stay),
                };
                screen == Screen::Game && waited
            }
        };
        if done {
            let image = target.read(&gpu)?;
            std::fs::write(out, m2_render::png::encode(&image)?)
                .with_context(|| format!("writing {}", out.display()))?;
            println!(
                "wrote {} ({width}×{height}: {}) with {}",
                out.display(),
                client.title,
                gpu.adapter_name()
            );
            return Ok(());
        }
        if started.elapsed() > SCREENSHOT_TIMEOUT {
            bail!(
                "no picture after {} s: the screen is {}",
                SCREENSHOT_TIMEOUT.as_secs(),
                client.title
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// The player's settings: their file's (`--settings`), with the volumes
/// the flags give for this session.
fn player_settings(args: &PlayArgs) -> Settings {
    let mut settings = args
        .settings
        .as_deref()
        .map(Settings::load)
        .unwrap_or_default();
    let volume = |flag: Option<f32>, or: f32| {
        flag.filter(|v| v.is_finite())
            .map_or(or, |v| v.clamp(0.0, 1.0))
    };
    settings.music = volume(args.music_volume, settings.music);
    settings.sounds = volume(args.sound_volume, settings.sounds);
    settings
}

/// The sounds of the window: on the system's audio device, at the player's
/// volumes (none while muted, the device open all the same so the Settings
/// window can unmute); silent with `--mute` or without a device.
fn open_sounds(args: &PlayArgs, source: &Source, settings: Settings) -> WorldSounds {
    if args.mute {
        return WorldSounds::silent();
    }
    let volumes = settings.volumes();
    match Audio::open(volumes) {
        Ok(audio) => {
            eprintln!(
                "sound: music at {:.2}, sounds at {:.2}",
                volumes.music, volumes.sounds
            );
            WorldSounds::new(Some(audio), source.hit_sounds())
        }
        Err(error) => {
            eprintln!("note: {error}; playing without sound");
            WorldSounds::silent()
        }
    }
}

/// Plays in a window until it is closed.
fn window(client: &mut Client<'_>, options: GpuOptions) -> Result<()> {
    client.game.sounds = open_sounds(client.args, &client.game.source, client.settings);
    // With a provider given, sign in at once. Either way the login screen
    // asks the server which providers it has: their buttons start a sign-in
    // over (its browser closed) or again (cancelled, or failed).
    if let Some(provider) = &client.args.provider {
        client.enter_official()?;
        match SessionConfig::provider(provider) {
            Ok(config) => client.connect(config, client.choice()),
            Err(error) => client.interface.fail(&error.to_string()),
        }
        if let Some(net) = client.server.clone() {
            client.providers = Some(ask_providers(net));
        }
    } else {
        client.show_servers(None);
    }
    let mut app = PlayWindow {
        size: client.args.size,
        options,
        client,
        surface: None,
        title: String::new(),
        cursor: None,
        pressed_at: None,
        turning: false,
        held: ModifiersState::empty(),
        clipboard: Clipboard::default(),
        last_frame: None,
        error: None,
    };
    let event_loop = EventLoop::new().context("opening a window")?;
    event_loop.run_app(&mut app).context("running the window")?;
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The digit of a digit key of the main rows, 1 to 9, by its place on the
/// keyboard.
fn digit_of(key: PhysicalKey) -> Option<u8> {
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    let digit = match code {
        KeyCode::Digit1 => 1,
        KeyCode::Digit2 => 2,
        KeyCode::Digit3 => 3,
        KeyCode::Digit4 => 4,
        KeyCode::Digit5 => 5,
        KeyCode::Digit6 => 6,
        KeyCode::Digit7 => 7,
        KeyCode::Digit8 => 8,
        KeyCode::Digit9 => 9,
        _ => return None,
    };
    Some(digit)
}

struct PlayWindow<'c, 'a> {
    size: (u32, u32),
    options: GpuOptions,
    client: &'c mut Client<'a>,
    surface: Option<Surface>,
    title: String,
    cursor: Option<PhysicalPosition<f64>>,
    /// Where the left button went down on the ground, for telling a click
    /// from a drag.
    pressed_at: Option<PhysicalPosition<f64>>,
    /// True while the right button turns the camera.
    turning: bool,
    /// The modifier keys held: with Shift, digits 1 to 4 choose the
    /// quickslot bar's page; with Ctrl, fields paste and copy.
    held: ModifiersState,
    clipboard: Clipboard,
    last_frame: Option<Instant>,
    error: Option<anyhow::Error>,
}

impl PlayWindow<'_, '_> {
    fn redraw(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let now = Instant::now();
        let seconds = self
            .last_frame
            .map_or(0.0, |last| now.duration_since(last).as_secs_f32().min(0.25));
        self.last_frame = Some(now);
        self.client.game.pointer = self.pointer();
        let Some(surface) = &mut self.surface else {
            return Ok(());
        };
        let Some((frame, view)) = surface.frame()? else {
            return Ok(());
        };
        let end = self.client.frame(
            &surface.gpu,
            &surface.renderer,
            &surface.attachments,
            &view,
            seconds,
        )?;
        surface.present(frame);
        if std::mem::take(&mut self.client.come_back) {
            // Back from the browser: to the front, or (when the system
            // keeps the browser in front) a flash in the taskbar.
            surface.window.set_minimized(false);
            surface.window.focus_window();
            surface
                .window
                .request_user_attention(Some(UserAttentionType::Informational));
        }
        let title = format!("openm2: {}", self.client.title);
        if title != self.title {
            surface.window.set_title(&title);
            self.title = title;
        }
        if end == FrameEnd::Quit {
            event_loop.exit();
        }
        Ok(())
    }

    fn in_game(&self) -> bool {
        self.client.interface.screen() == Screen::Game
    }

    /// A spot of the view for a pointer position: `x`, `y` from −1 to 1,
    /// y up, and the view's aspect.
    fn spot(&self, at: PhysicalPosition<f64>) -> Option<(f32, f32, f32)> {
        let surface = self.surface.as_ref()?;
        let (width, height) = surface.attachments.size();
        let x = (at.x / f64::from(width.max(1))) as f32 * 2.0 - 1.0;
        let y = 1.0 - (at.y / f64::from(height.max(1))) as f32 * 2.0;
        Some((x, y, surface.aspect()))
    }

    /// A click on the ground: walk there around the walls, leaving any
    /// fight.
    fn click(&mut self, at: PhysicalPosition<f64>) {
        if self.reel_in_instead() {
            return;
        }
        let Some((x, y, aspect)) = self.spot(at) else {
            return;
        };
        let Some(to) = self.client.game.ground_at(aspect, x, y) else {
            return;
        };
        let Some((world, now)) = self.client.world_now() else {
            return;
        };
        let Some(me) = world.my_entity() else {
            return;
        };
        let game = &mut self.client.game;
        game.fighter.stop();
        game.gatherer.stop();
        game.talker.stop();
        game.charging = None;
        let path = walls::path(game.walls(), me.track.point(now), to);
        if let Some(connection) = &self.client.connection
            && !path.is_empty()
        {
            connection.remote.send(Command::MoveTo { path });
        }
    }

    /// The main button went down on the world: on a monster, go for it
    /// (`fight.rs`); on an item on the ground, go and pick it up
    /// (`ground.rs`); on an NPC, go and talk to it (`shop.rs`); on anyone
    /// else, choose them. True if it was on an entity or an item.
    fn press_world(&mut self, at: PhysicalPosition<f64>) -> bool {
        let Some((x, y, aspect)) = self.spot(at) else {
            return false;
        };
        let Some((world, now)) = self.client.world_now() else {
            return false;
        };
        let Some(me) = world.me.as_ref().map(|me| me.vid) else {
            return false;
        };
        let named = self
            .client
            .interface
            .ui
            .label_at(at.x as f32, at.y as f32)
            .and_then(|label| self.client.game.item_names.item(label));
        let game = &mut self.client.game;
        let Some(picked) = game.picked_at(me, aspect, (x, y), named) else {
            return false;
        };
        game.charging = None;
        match picked {
            Picked::Item(vid) if world.ground.contains_key(&vid) => {
                game.fighter.stop();
                game.talker.stop();
                game.gatherer.go_for(vid);
            }
            Picked::Entity(vid) => match world.entities.get(&vid) {
                Some(entity) if can_fight(entity) => {
                    game.gatherer.stop();
                    game.talker.stop();
                    game.fighter.press_on(vid);
                }
                Some(entity) if entity.kind == EntityKind::Npc && !entity.dead => {
                    game.fighter.stop();
                    game.gatherer.stop();
                    game.talker.go_to(vid);
                }
                Some(_) => {
                    game.gatherer.stop();
                    game.talker.stop();
                    game.fighter.chose_target(now);
                    if let Some(connection) = &self.client.connection {
                        connection.remote.send(Command::Target { vid });
                    }
                }
                None => return false,
            },
            Picked::Item(_) => return false,
        }
        true
    }

    /// The main button went down on the world, or a drag from the bag ended
    /// there, while the pointer carries an item of the bag: over a living
    /// NPC, the item goes back to its place and our character walks up to
    /// the NPC to ask it about upgrading the item (`upgrade.rs`). True if it
    /// did.
    fn drop_on_npc(&mut self) -> bool {
        let Some(at) = self.cursor else {
            return false;
        };
        let Some(vid) = self.npc_at(at) else {
            return false;
        };
        let Some(slot) = self.client.interface.drop_on_npc() else {
            return false;
        };
        let game = &mut self.client.game;
        game.charging = None;
        game.fighter.stop();
        game.gatherer.stop();
        game.talker.go_to_upgrade(vid, slot);
        true
    }

    /// Where the pointer is over the world, in the game and off the
    /// windows, as a spot of the view (what is under it shows its ring).
    fn pointer(&self) -> Option<(f32, f32, f32)> {
        if !self.in_game() {
            return None;
        }
        let at = self.cursor?;
        if self.client.interface.ui.is_over(at.x as f32, at.y as f32) {
            return None;
        }
        self.spot(at)
    }

    /// The living NPC under the pointer at `at`, if there is one.
    fn npc_at(&mut self, at: PhysicalPosition<f64>) -> Option<u32> {
        let (x, y, aspect) = self.spot(at)?;
        let (world, _) = self.client.world_now()?;
        let me = world.me.as_ref()?.vid;
        let vid = self.client.game.entity_at(me, aspect, x, y)?;
        world
            .entities
            .get(&vid)
            .filter(|entity| entity.kind == EntityKind::Npc && !entity.dead)
            .map(|_| vid)
    }

    /// Space with a fishing pole in hand, on foot: reels the
    /// line in if it is out, else casts toward the water 6 m out, from
    /// where we face then 10° to either side in turn up to 180°; with none
    /// a notice says so. Returns whether Space was fishing's.
    fn fishing_key(&mut self) -> bool {
        let Some((world, now)) = self.client.world_now() else {
            return false;
        };
        let game = &mut self.client.game;
        if world.riding() || !game.holds_pole(&world) {
            return false;
        }
        let Some(me) = world.my_entity().filter(|me| !me.dead) else {
            return true;
        };
        game.fighter.stop();
        game.gatherer.stop();
        game.talker.stop();
        game.charging = None;
        let command = if me.line.is_some() {
            Some(Command::ReelIn)
        } else {
            game.cast_angle(me.track.point(now), me.angle)
                .map(|angle| Command::Cast { angle })
        };
        match (command, &self.client.connection) {
            (Some(command), Some(connection)) => {
                connection.remote.send(command);
            }
            (None, _) => self
                .client
                .interface
                .notice("There is no water close enough to fish."),
            _ => {}
        }
        true
    }

    /// A move key or a click on the ground while our line is out reels it
    /// in instead of walking, at most every [`REEL_GAP`].
    /// Returns whether it did.
    fn reel_in_instead(&mut self) -> bool {
        let Some((world, _)) = self.client.world_now() else {
            return false;
        };
        if world.my_entity().is_none_or(|me| me.line.is_none()) {
            return false;
        }
        let game = &mut self.client.game;
        if game.reeled_at.is_some_and(|at| at.elapsed() < REEL_GAP) {
            return true;
        }
        game.reeled_at = Some(Instant::now());
        if let Some(connection) = &self.client.connection {
            connection.remote.send(Command::ReelIn);
        }
        true
    }

    /// Space went down: attack the foe, else the target if it can be
    /// fought, else where we face; with a fishing pole, fishing.
    fn attack_key(&mut self) {
        if self.fishing_key() {
            return;
        }
        let target = self.client.world_now().and_then(|(world, _)| {
            world
                .target
                .filter(|vid| world.entities.get(vid).is_some_and(can_fight))
        });
        self.client.game.gatherer.stop();
        self.client.game.talker.stop();
        self.client.game.fighter.key_down(target);
    }

    /// A movement key went down: the keys lead our character, leaving any
    /// fight, pick-up or talk (`steer.rs`).
    fn move_key(&mut self, way: Way) {
        if self.reel_in_instead() {
            return;
        }
        let game = &mut self.client.game;
        if game.steering.press(way) {
            game.fighter.stop();
            game.gatherer.stop();
            game.talker.stop();
            game.charging = None;
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: anyhow::Error) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl ApplicationHandler for PlayWindow<'_, '_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        let opened =
            Surface::open(event_loop, "openm2", self.size, self.options).and_then(|surface| {
                self.client.display = Some(surface.window.scale_factor());
                self.client.apply_scale();
                self.client
                    .attach(&surface.gpu, surface.renderer.format())?;
                Ok(surface)
            });
        match opened {
            Ok(surface) => {
                surface.window.request_redraw();
                self.surface = Some(surface);
            }
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(surface) = &mut self.surface {
                    surface.resize(size);
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.client.display = Some(scale_factor);
                self.client.apply_scale();
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw(event_loop) {
                    self.fail(event_loop, error);
                    return;
                }
                if let Some(surface) = &self.surface {
                    surface.window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.held = modifiers.state();
                self.client.interface.set_alt(self.held.alt_key());
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        physical_key,
                        text,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                // A field's paste and copy.
                if self.client.interface.ui.focused().is_some()
                    && let Some(shortcut) = Shortcut::of(&logical_key, physical_key, self.held)
                {
                    if let Some(note) = self
                        .clipboard
                        .apply(shortcut, &mut self.client.interface.ui)
                    {
                        eprintln!("note: {note}");
                    }
                    return;
                }
                // Shift and a digit: the key's place, as Shift changes
                // what it types.
                if self.held.shift_key()
                    && let Some(digit) = digit_of(physical_key)
                    && self.client.interface.page_key(digit)
                {
                    return;
                }
                if self.client.interface.key(&logical_key, text.as_deref()) || !self.in_game() {
                    return;
                }
                match logical_key {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Named(NamedKey::Space) => self.attack_key(),
                    Key::Character(c) if c.eq_ignore_ascii_case("f") => {
                        self.client.game.fog = !self.client.game.fog;
                    }
                    key => {
                        if let Some(way) = Way::of(&key) {
                            self.move_key(way);
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Released,
                        ..
                    },
                ..
            } => {
                if logical_key == Key::Named(NamedKey::Space) {
                    self.client.game.fighter.key_up();
                } else if let Some(way) = Way::of(&logical_key) {
                    self.client.game.steering.release(way);
                }
            }
            // Keys and buttons let go of elsewhere.
            WindowEvent::Focused(false) => {
                self.held = ModifiersState::empty();
                self.client.interface.set_alt(false);
                self.client.game.fighter.key_up();
                self.client.game.fighter.button_up();
                self.client.game.steering.release_all();
            }
            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => {
                    // A click on the interface is not a walk, nor one that
                    // puts back what the pointer carries, nor one on an
                    // entity.
                    let taken = self.client.interface.ui.press();
                    // An item of the bag let go over an NPC asks it about
                    // upgrading the item.
                    let on_npc = !taken && self.in_game() && self.drop_on_npc();
                    let put_back =
                        !taken && !on_npc && self.in_game() && self.client.interface.world_press();
                    let mut on_entity = false;
                    if !taken
                        && !on_npc
                        && !put_back
                        && self.in_game()
                        && let Some(at) = self.cursor
                    {
                        on_entity = self.press_world(at);
                    }
                    self.pressed_at = if taken || on_npc || put_back || on_entity || !self.in_game()
                    {
                        None
                    } else {
                        self.cursor
                    };
                }
                (MouseButton::Left, ElementState::Released) => {
                    // An item dragged from the bag and let go off the
                    // windows, over an NPC, asks it about upgrading it; over
                    // anything else it stays on the pointer, for a click.
                    let off_windows = self.cursor.is_some_and(|at| {
                        !self.client.interface.ui.is_over(at.x as f32, at.y as f32)
                    });
                    self.client.interface.ui.release();
                    if off_windows && self.in_game() && self.drop_on_npc() {
                        self.pressed_at = None;
                    }
                    self.client.game.fighter.button_up();
                    if let (Some(down), Some(up)) = (self.pressed_at.take(), self.cursor)
                        && (down.x - up.x).hypot(down.y - up.y) <= CLICK_SLOP
                    {
                        self.click(up);
                    }
                }
                (MouseButton::Right, ElementState::Pressed) => {
                    // A right click on the interface uses an item; elsewhere
                    // dragging turns the camera.
                    let taken = self.client.interface.ui.press_secondary();
                    self.turning = !taken && self.in_game();
                }
                (MouseButton::Right, ElementState::Released) => self.turning = false,
                _ => {}
            },
            WindowEvent::CursorMoved { position, .. } => {
                if self.turning
                    && let Some(last) = self.cursor
                {
                    let camera = &mut self.client.game.camera;
                    camera.yaw -= (position.x - last.x) as f32 * TURN;
                    camera.pitch =
                        (camera.pitch + (position.y - last.y) as f32 * TURN).clamp(0.05, 1.5);
                }
                self.cursor = Some(position);
                self.client
                    .interface
                    .ui
                    .pointer_moved(position.x as f32, position.y as f32);
            }
            WindowEvent::CursorLeft { .. } => self.client.interface.ui.pointer_left(),
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => {
                        wheel_lines(p.y, self.client.interface.ui.scale())
                    }
                };
                if self.client.interface.ui.wheel(lines) || !self.in_game() {
                    return;
                }
                let camera = &mut self.client.game.camera;
                camera.distance = (camera.distance * 0.9f32.powf(lines)).clamp(300.0, 10_000.0);
            }
            _ => {}
        }
    }
}

/// What tells an actor's once-played motions apart, for the effects of
/// their events ([`MotionClock`]); standing is 0 and running 1.
fn once_key(once: Once) -> u32 {
    let index = |n: usize| u32::try_from(n).unwrap_or(0xff).min(0xff);
    match once {
        Once::Combo(n) => 0x100 + index(n),
        Once::Attack(n) => 0x200 + index(n),
        Once::Damage(n) => 0x300 + index(n),
        Once::Dead => 0x400,
        Once::Skill(n) => 0x500 + index(n),
        Once::Fishing(motion) => 0x600 + motion as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::tests::motion;

    #[test]
    fn each_login_in_the_window_has_its_own_capture() {
        let path = Path::new("local/try.m2cap");
        assert_eq!(capture_path(path, 1), Path::new("local/try.m2cap"));
        assert_eq!(capture_path(path, 2), Path::new("local/try-2.m2cap"));
        assert_eq!(capture_path(Path::new("try"), 3), Path::new("try-3"));
    }

    fn fight() -> FightMotions {
        FightMotions {
            combo: vec![motion(1.0, 1), motion(1.0, 1)],
            attack: vec![motion(1.0, 1)],
            damage: vec![motion(0.5, 1)],
            dead: Some(motion(2.0, 1)),
            ..FightMotions::default()
        }
    }

    fn input() -> OnceInput {
        OnceInput {
            dead: false,
            moving: false,
            attack: false,
            step: None,
            skill: None,
            since: 0.0,
            hit: false,
            attack_seed: 0,
            flinch_seed: 0,
        }
    }

    #[test]
    fn what_flies_is_handed_on_to_the_window() {
        // The window flies them, the experience's flight among them.
        let flight = Event::Flight {
            kind: FlightKind::Experience,
            from: 7,
            to: 1,
        };
        let shot = Event::Shot {
            shooter: 7,
            target: 1,
            at: Point::new(0, 0),
        };
        // So are what entities show.
        let levelled = Event::EntityEffect {
            kind: EntityEffectKind::LevelUp,
            vid: 1,
        };
        assert!(handed_on(&flight) && handed_on(&shot) && handed_on(&levelled));
        assert!(!handed_on(&Event::Died { vid: 7 }));
    }

    #[test]
    fn a_press_is_on_an_item_before_anyone_and_on_an_items_name() {
        let nobody = || None;
        let monster = || Some(7);
        // The item under the pointer, even in a body (a dead monster's,
        // whose drops lie at its feet), or behind someone.
        assert_eq!(picked(Some(500), None, monster), Some(Picked::Item(500)));
        assert_eq!(
            picked(Some(500), Some(501), nobody),
            Some(Picked::Item(500))
        );
        // Else the item whose name is there.
        assert_eq!(picked(None, Some(501), monster), Some(Picked::Item(501)));
        assert_eq!(picked(None, None, monster), Some(Picked::Entity(7)));
        assert_eq!(picked(None, None, nobody), None);
        // The labels: two names over heads, then three items' names, then
        // damage.
        let names = ItemNames {
            first: 2,
            vids: vec![500, 501, 502],
        };
        assert_eq!(names.item(0), None);
        assert_eq!(names.item(2), Some(500));
        assert_eq!(names.item(4), Some(502));
        assert_eq!(names.item(5), None);
        assert_eq!(ItemNames::default().item(0), None);
    }

    #[test]
    fn jumps_without_a_time_are_run_and_timed_walks_followed() {
        // A monster the server put 3 m east at once: drawn running there,
        // 4.5 m a second, so 45 cm in a tenth of a second.
        let (at, running) = chase(Some((0.0, 0.0)), (300.0, 0.0), false, 45.0);
        assert_eq!((at, running), ((45.0, 0.0), true));
        // Arriving, it stands on the point.
        assert_eq!(
            chase(Some((290.0, 0.0)), (300.0, 0.0), false, 45.0),
            ((300.0, 0.0), false)
        );
        // Met for the first time, on a timed walk, or very far: where the
        // session says.
        for (shown, follow, truth) in [
            (None, false, (300.0, 0.0)),
            (Some((0.0, 0.0)), true, (300.0, 0.0)),
            (Some((0.0, 0.0)), false, (5_000.0, 0.0)),
        ] {
            assert_eq!(chase(shown, truth, follow, 45.0), (truth, false));
        }
    }

    #[test]
    fn a_click_on_a_warp_walks_to_it() {
        assert!(!has_body(EntityKind::Warp));
        assert!(!has_body(EntityKind::Goto));
        for kind in [
            EntityKind::Monster,
            EntityKind::Npc,
            EntityKind::MetinStone,
            EntityKind::Door,
            EntityKind::Player,
            EntityKind::Horse,
        ] {
            assert!(has_body(kind), "{kind:?}");
        }
    }

    /// Us at the origin, a wolf 4 m east, Bob 9 m north, a wolf 12 m
    /// west; Bob chosen, with 100 mana.
    fn around() -> World {
        let at = |vid: u32, kind: EntityKind, x: i32, y: i32| Entity {
            vid,
            kind,
            race: 0,
            track: m2_core::Track::still(Point::new(x, y), Millis(0)),
            angle: 0,
            z: 0,
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
        };
        let mut world = World::default();
        for entity in [
            at(1, EntityKind::Player, 0, 0),
            at(2, EntityKind::Monster, 400, 0),
            at(3, EntityKind::Player, 0, 900),
            at(4, EntityKind::Monster, -1_200, 0),
        ] {
            world.entities.insert(entity.vid, entity);
        }
        world.target = Some(3);
        world.points.insert(SP_POINT, 100);
        world
    }

    #[test]
    fn a_skill_aims_at_a_foe_a_friend_or_us_within_reach() {
        let world = around();
        let me = world.entities[&1].clone();
        let now = Millis(0);
        let flags = |words: &'static [&'static str]| move |flag: &str| words.contains(&flag);
        let attack = flags(&["ATTACK_SKILL", "NEED_TARGET"]);
        let support = flags(&["CAN_USE_FOR_ME", "NEED_TARGET", "ONLY_FOR_ALLIANCE"]);
        // An attack on the wolf within 5 m; on one 12 m off, too far; with
        // no foe chosen (Bob is no foe), it needs one.
        let aim = |flag: &dyn Fn(&str) -> bool, foe, reach, cost| {
            aim_skill(&world, now, &me, flag, foe, reach, cost)
        };
        assert_eq!(aim(&attack, Some(2), 500.0, None), Ok(Aim::At(Some(2))));
        assert_eq!(
            aim(&attack, Some(4), 500.0, None),
            Err("Too far away.".to_owned())
        );
        assert_eq!(
            aim(&attack, None, 500.0, None),
            Err("This skill needs a target.".to_owned())
        );
        // A support skill goes to Bob, chosen, within its 10 m; out of a
        // 5 m reach, too far.
        assert_eq!(aim(&support, None, 1_000.0, None), Ok(Aim::At(Some(3))));
        assert_eq!(
            aim(&support, None, 500.0, None),
            Err("Too far away.".to_owned())
        );
        // Mana: 100 pays 100, not 101.
        assert_eq!(
            aim(&support, None, 1_000.0, Some(100)),
            Ok(Aim::At(Some(3)))
        );
        assert_eq!(
            aim(&support, None, 1_000.0, Some(101)),
            Err("You do not have enough mana.".to_owned())
        );
        // A charge at the far wolf runs up to 1.5 m short of it.
        let charge = flags(&["ATTACK_SKILL", "NEED_TARGET", "CHARGE_ATTACK"]);
        assert_eq!(
            aim(&charge, Some(4), 500.0, None),
            Ok(Aim::Charge {
                foe: 4,
                to: Point::new(-1_050, 0)
            })
        );
        assert_eq!(aim(&charge, Some(2), 500.0, None), Ok(Aim::At(Some(2))));
        // Without Bob chosen, a support skill is ours.
        let mut alone = world.clone();
        alone.target = None;
        assert_eq!(
            aim_skill(&alone, now, &me, &support, None, 1_000.0, None),
            Ok(Aim::At(Some(1)))
        );
        // Riding, none: not even a charge's run.
        let mut riding = me.clone();
        riding.mount = 1201;
        for flag in [&attack as &dyn Fn(&str) -> bool, &support, &charge] {
            assert_eq!(
                aim_skill(&world, now, &riding, flag, Some(4), 1_000.0, None),
                Err("Get off your mount to use this skill.".to_owned())
            );
        }
    }

    #[test]
    fn damage_shows_on_us_and_our_target_as_the_original() {
        // Hits on us and on our target show their amount, whatever they
        // were: critical, poison, fire.
        assert_eq!(number(536, 1, false, true), Some(Number::Target(536)));
        assert_eq!(number(60, 1, true, false), Some(Number::Ours(60)));
        assert_eq!(number(900, 1 | 32, false, true), Some(Number::Target(900)));
        assert_eq!(number(12, 2, false, true), Some(Number::Target(12)));
        assert_eq!(number(40, 128, true, false), Some(Number::Ours(40)));
        // A hit on another, a heal and nothing taken show nothing.
        assert_eq!(number(536, 1, false, false), None);
        assert_eq!(number(300, 64, true, false), None);
        assert_eq!(number(0, 1, true, false), None);
        // A dodge or a block is a miss, on us or on whoever it names.
        assert_eq!(number(0, 4, true, false), Some(Number::OurMiss));
        assert_eq!(number(0, 8, true, false), Some(Number::OurMiss));
        assert_eq!(number(0, 4, false, false), Some(Number::TheirMiss));
        // Its digits, the last first, eight at most.
        assert_eq!(digits(536), [6, 3, 5]);
        assert_eq!(digits(7), [7]);
        assert_eq!(digits(1_000), [0, 0, 0, 1]);
        assert_eq!(digits(1_234_567_890).len(), MOST_DIGITS);
    }

    #[test]
    fn a_cast_looks_for_water_ahead_then_to_either_side() {
        // A pond 6 m east of the map's corner, a metre wide, 2 to 4 m
        // south of it: water 6 m out east of a point 3 m south.
        let mut walk = m2_content::WalkMap::open(20, 20).unwrap();
        walk.set_blocked(19, 19, true);
        let mut water = m2_content::WalkMap::open(20, 20).unwrap();
        for y in 2..=4 {
            water.set_blocked(6, y, true);
        }
        let base = Point::new(1_000, 2_000);
        let walls = Walls::new(walk, base).with_water(Some(water));
        let here = Point::new(1_050, 2_350);
        // Facing east (90): straight ahead.
        assert_eq!(water_way(&walls, here, 90), Some(90));
        // Facing south-east (45): 10° to the left after none to the right.
        assert_eq!(water_way(&walls, here, 45), Some(85));
        // Facing west: the long way round, 170° to one side reaching it
        // first.
        assert_eq!(water_way(&walls, here, 270), Some(80));
        // No water anywhere 6 m out.
        assert_eq!(water_way(&walls, Point::new(1_050, 3_850), 90), None);
        assert_eq!(ahead(Point::new(0, 0), 0, 600.0), Point::new(0, 600));
        assert_eq!(ahead(Point::new(0, 0), 90, 600.0), Point::new(600, 0));
    }

    #[test]
    fn attacks_flinches_and_falls_start_and_end() {
        let fight = fight();
        let combo = OnceInput {
            attack: true,
            step: Some(2),
            ..input()
        };
        let swinging = Some((Once::Combo(1), 0.0));
        assert_eq!(next_once(None, &fight, combo), swinging);
        // Seen late: as far in as the move is old.
        let late = OnceInput {
            since: 0.25,
            ..combo
        };
        assert_eq!(next_once(None, &fight, late), Some((Once::Combo(1), 0.25)));
        // A swing goes on; a hit does not stop it.
        let later = Some((Once::Combo(1), 0.4));
        assert_eq!(next_once(later, &fight, input()), later);
        let hit = OnceInput {
            hit: true,
            ..input()
        };
        assert_eq!(next_once(later, &fight, hit), later);
        // Standing, a hit makes it flinch; a new attack cuts the flinch.
        let flinch = Some((Once::Damage(0), 0.0));
        assert_eq!(next_once(None, &fight, hit), flinch);
        let attack = OnceInput {
            attack: true,
            ..input()
        };
        assert_eq!(
            next_once(Some((Once::Damage(0), 0.2)), &fight, attack),
            Some((Once::Attack(0), 0.0))
        );
        // Walking ends it.
        let moving = OnceInput {
            moving: true,
            ..input()
        };
        assert_eq!(next_once(later, &fight, moving), None);
        // Dead: it falls once and stays down, until it is alive again.
        let dead = OnceInput {
            dead: true,
            ..input()
        };
        assert_eq!(next_once(later, &fight, dead), Some((Once::Dead, 0.0)));
        let lying = Some((Once::Dead, 5.0));
        assert_eq!(next_once(lying, &fight, dead), lying);
        assert_eq!(next_once(lying, &fight, input()), None);
        // Without the motions, nothing plays.
        let none = FightMotions::default();
        assert_eq!(next_once(None, &none, dead), None);
        assert_eq!(next_once(None, &none, combo), None);
    }
}
