//! `openm2 view ui`: draws interface layouts (or one of `play`'s screens)
//! over a plain background, with an asset folder's skin and art with
//! `--assets` (else fallback colours), in a window or into a PNG.
//! `--item` and `--quick` fill the inventory and the quickslots with the
//! items of the folder, as the game would, `--good` an open
//! shop, `--teacher` a teacher's window, `--choose-skill` a soul stone's
//! window, and `--skill` the skills window. In the window the interface
//! takes the mouse and keyboard, and prints what the user does (clicks,
//! submitted fields, slots). Escape outside a field closes it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use glam::Vec3;
use m2_core::{
    Affect, Empire, Friend, Item, Me, Millis, QuickSlot, QuickSlotKind, SHOP_PLACES, SKILL_POINT,
    Shop, ShopGood, Skill, SkillChoice, Teacher, WhisperKind, WhisperLine, Window, World,
};
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::{Camera, Gpu, GpuOptions, Offscreen, Renderer, Scene};
use m2_ui::{Cooldown, Ui, UiEvent};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::WindowId;

use crate::affects::AffectsBar;
use crate::character::{CharacterWindow, Tab};
use crate::choose_skill::SkillChoiceWindow;
use crate::interface::{self, GpuInterface, ScaleArgs};
use crate::inventory::Inventory;
use crate::messenger::FriendsWindow;
use crate::screens::{self, Screen};
use crate::shop::ShopWindow;
use crate::skills::SkillWindow;
use crate::source::Source;
use crate::stable::StableWindow;
use crate::teacher::TeacherWindow;
use crate::upgrade::UpgradeWindow;
use crate::whisper::WhisperWindows;
use crate::window::Surface;

/// The background behind the windows.
const BACKGROUND: [f64; 4] = [0.18, 0.21, 0.24, 1.0];

/// `openm2 view ui`
#[derive(Debug, Args)]
pub struct UiArgs {
    /// Layout files (RON); without any, the built-in in-game layout
    layouts: Vec<PathBuf>,
    /// Draw one of `play`'s screens instead: its built-in layouts, with its
    /// windows shown
    #[arg(long, value_enum, conflicts_with = "layouts")]
    screen: Option<Screen>,
    /// Draw with this asset folder's skin, items and skills (else fallback
    /// colours)
    #[arg(long, value_name = "DIR")]
    assets: Option<PathBuf>,
    /// Set a text, label or field: ID=TEXT (repeatable)
    #[arg(long, value_name = "ID=TEXT")]
    set: Vec<String>,
    /// Set a gauge: ID=VALUE from 0 to 1 (repeatable)
    #[arg(long, value_name = "ID=VALUE")]
    value: Vec<String>,
    /// Add a line to a log: ID=TEXT (repeatable)
    #[arg(long, value_name = "ID=TEXT")]
    line: Vec<String>,
    /// Show a hidden node (repeatable)
    #[arg(long, value_name = "ID")]
    show: Vec<String>,
    /// Give a field the keyboard
    #[arg(long, value_name = "ID")]
    focus: Option<String>,
    /// With --screen game: an item in the inventory, SLOT=VNUM or
    /// SLOT=VNUMxCOUNT; slots 0-89 are the bag, 90 and up what is worn
    /// (repeatable; names and icons need --assets)
    #[arg(long, value_name = "SLOT=VNUM[xCOUNT]")]
    item: Vec<String>,
    /// With --screen game: a quickslot holding the bag's item, INDEX=SLOT
    /// (repeatable)
    #[arg(long, value_name = "INDEX=SLOT")]
    quick: Vec<String>,
    /// With --screen game: what an open shop sells in a place (0-39),
    /// PLACE=VNUM or PLACE=VNUMxCOUNT, at the item table's buy price
    /// (repeatable)
    #[arg(long, value_name = "PLACE=VNUM[xCOUNT]")]
    good: Vec<String>,
    /// With --screen game: a teacher showing what it teaches, JOB:GROUP
    /// (job 0 warrior, 1 ninja, 2 sura, 3 shaman; group 1 or 2), from
    /// level 5; `--class`, `--skill-group` and `--point 1=LEVEL` decide
    /// what it says. Names and icons need --assets
    #[arg(long, value_name = "JOB:GROUP")]
    teacher: Option<String>,
    /// With --screen game: a soul stone's window, offering these skills,
    /// SKILL,SKILL,… (at most 6; their levels from --skill). Names and
    /// icons need --assets
    #[arg(long, value_name = "SKILL,…")]
    choose_skill: Option<String>,
    /// With --screen game: our mount at level 1 with this stamina,
    /// STAMINA/MOST: the taskbar's stamina gauge shows it
    #[arg(long, value_name = "STAMINA/MOST")]
    mount: Option<String>,
    /// With --screen game: a stable offering to train our mount to LEVEL
    /// (from level 5, for 500 gold and 2 of each `--item`'s item; 1 is
    /// the first mount)
    #[arg(long, value_name = "LEVEL")]
    stable: Option<u8>,
    /// With --screen game: a smith offering to upgrade the bag's first
    /// `--item` to item NEXT, at 50 %, for 1,200 gold and 2 of each other
    /// `--item`'s item; a failure destroys it
    #[arg(long, value_name = "NEXT")]
    upgrade: Option<u32>,
    /// With --screen game: one of our skills at a level (1-19 normal, 20-29
    /// master, 30-39 grand master, 40 perfect master), SKILL=LEVEL
    /// (repeatable): the character window's Skills tab (`--show skills`)
    /// and the quickslots show it; names and icons need --assets
    #[arg(long, value_name = "SKILL=LEVEL")]
    skill: Vec<String>,
    /// With --screen game: a quickslot holding a skill, INDEX=SKILL
    /// (repeatable)
    #[arg(long, value_name = "INDEX=SKILL")]
    quick_skill: Vec<String>,
    /// With --screen game: a skill cooling down, SKILL=SECONDS left of a
    /// cooldown twice as long (repeatable): its icons are darkened and show
    /// the seconds
    #[arg(long, value_name = "SKILL=SECONDS")]
    cooldown: Vec<String>,
    /// With --screen game: a point a skill raises on us for now,
    /// SKILL:POINT=AMOUNT/SECONDS (repeatable; 0 seconds while a skill that
    /// stays on is on; points as Affects numbers them: 1 attack speed, 2
    /// movement speed, ... 12 hidden): the affects bar shows it
    #[arg(long, value_name = "SKILL:POINT=AMOUNT/SECONDS")]
    affect: Vec<String>,
    /// With --screen game: our skill group (1 or 2; 0 for none), for the
    /// skills window
    #[arg(long, default_value_t = 1)]
    skill_group: u8,
    /// With --screen game: our class and gender (0 a male warrior, 1 a
    /// female ninja, 2 a male sura, 3 a female shaman, 4-7 the others),
    /// for the skills window
    #[arg(long, default_value_t = 0)]
    class: u16,
    /// With --screen game: skill points left
    #[arg(long, default_value_t = 0)]
    skill_points: u32,
    /// With --screen game: one of our points, POINT=VALUE (repeatable; 1
    /// level, 3 and 4 experience and what the level needs, 6 and 8 most
    /// health and mana, 12 to 15 strength, vitality, dexterity and
    /// intelligence, 16 defence, 17 and 19 attack and movement speed, 18
    /// attack grade, 26 status points), for the character window's
    /// Character tab (`--show character`)
    #[arg(long, value_name = "POINT=VALUE")]
    point: Vec<String>,
    /// With --screen game: a whisper to us, NAME=TEXT (repeatable): its
    /// conversation's envelope shows, marked
    #[arg(long, value_name = "NAME=TEXT")]
    whisper: Vec<String>,
    /// With --screen game: our whisper to a player, NAME=TEXT (repeatable)
    #[arg(long, value_name = "NAME=TEXT")]
    whisper_to: Vec<String>,
    /// With --screen game: the whisper window open on the conversation
    /// with NAME
    #[arg(long, value_name = "NAME")]
    open_whisper: Option<String>,
    /// With --screen game: a friend, NAME or NAME:online (repeatable); the
    /// list shows with `--show messenger`
    #[arg(long, value_name = "NAME[:online]")]
    friend: Vec<String>,
    /// With --screen game: a player asking to be our friend: the question
    /// shows
    #[arg(long, value_name = "NAME")]
    friend_request: Option<String>,
    /// Size of the window (before the display's scaling) or picture,
    /// WIDTHxHEIGHT
    #[arg(long, default_value = "1280x800", value_parser = crate::view::parse_size)]
    size: (u32, u32),
    #[command(flatten)]
    scale: ScaleArgs,
    /// Use a software adapter (no GPU needed; slower)
    #[arg(long)]
    software: bool,
    /// No window: draw one picture into this PNG
    #[arg(long, value_name = "FILE")]
    screenshot: Option<PathBuf>,
}

/// Splits `ID=VALUE`.
fn pair(text: &str) -> Result<(&str, &str)> {
    text.split_once('=')
        .ok_or_else(|| anyhow!("`{text}` is not ID=VALUE"))
}

/// `SLOT=VNUM[xCOUNT]`: the slot, the item number and the count.
fn slot_item(text: &str) -> Result<(u16, u32, u8)> {
    let (slot, item) = pair(text)?;
    let (vnum, count) = item.split_once('x').unwrap_or((item, "1"));
    let number = |what: &str, value: &str| {
        value
            .parse::<u32>()
            .with_context(|| format!("`{value}` is not {what} (in `{text}`)"))
    };
    let slot = u16::try_from(number("a slot", slot)?).context("the slot is too large")?;
    let count = u8::try_from(number("a count", count)?).context("the count is over 255")?;
    Ok((slot, number("an item number", vnum)?, count))
}

/// The world `--item`, `--quick`, `--good` and the skill arguments
/// describe; the goods' prices from `items`.
fn items_world(args: &UiArgs, items: Option<&m2_assets::items::Items>) -> Result<World> {
    let mut world = World {
        me: Some(Me {
            vid: 1,
            name: "Admin".to_owned(),
            class: args.class,
            empire: Empire::Shinsoo,
            position: (0, 0, 0),
        }),
        skill_group: args.skill_group,
        ..World::default()
    };
    world.points.insert(SKILL_POINT, args.skill_points);
    for text in &args.point {
        let (point, value) = pair(text)?;
        let point: u8 = point
            .parse()
            .with_context(|| format!("`{point}` is not a point number (in `{text}`)"))?;
        let value: u32 = value
            .parse()
            .with_context(|| format!("`{value}` is not a number (in `{text}`)"))?;
        world.points.insert(point, value);
    }
    let byte = |what: &str, value: &str, text: &str| {
        value
            .parse::<u8>()
            .with_context(|| format!("`{value}` is not {what} (in `{text}`)"))
    };
    for text in &args.skill {
        let (skill, level) = pair(text)?;
        let (skill, level) = (byte("a skill", skill, text)?, byte("a level", level, text)?);
        let grade = match level {
            0..20 => 0,
            20..30 => 1,
            30..40 => 2,
            _ => 3,
        };
        let known = Skill {
            level,
            grade,
            next_read: 0,
        };
        world.skills.insert(skill, known);
    }
    for text in &args.quick_skill {
        let (index, skill) = pair(text)?;
        let (index, position) = (
            byte("a quickslot", index, text)?,
            byte("a skill", skill, text)?,
        );
        let kind = QuickSlotKind::Skill;
        world.quickslots.insert(index, QuickSlot { kind, position });
    }
    for text in &args.item {
        let (slot, vnum, count) = slot_item(text)?;
        let item = Item {
            vnum,
            count,
            flags: 0,
            anti_flags: 0,
            sockets: [0; 3],
            bonuses: Vec::new(),
        };
        world.items.insert((Window::Inventory, slot), item);
    }
    for text in &args.quick {
        let (index, slot) = pair(text)?;
        let index: u8 = index
            .parse()
            .with_context(|| format!("`{index}` is not a quickslot"))?;
        let position: u8 = slot
            .parse()
            .with_context(|| format!("`{slot}` is not a bag slot"))?;
        let kind = QuickSlotKind::Item;
        world.quickslots.insert(index, QuickSlot { kind, position });
    }
    if !args.good.is_empty() {
        let mut shop = Shop {
            vid: 0,
            goods: Default::default(),
        };
        for text in &args.good {
            let (place, vnum, count) = slot_item(text)?;
            let place = u8::try_from(place)
                .ok()
                .filter(|place| *place < SHOP_PLACES)
                .with_context(|| {
                    format!("a shop's places are 0 to {} (in `{text}`)", SHOP_PLACES - 1)
                })?;
            let price = items
                .and_then(|items| items.get(vnum))
                .map_or(0, |info| info.record.buy_price);
            shop.goods.insert(place, ShopGood { vnum, price, count });
        }
        world.shop = Some(shop);
    }
    if let Some(text) = &args.teacher {
        let (job, group) = text
            .split_once(':')
            .with_context(|| format!("`{text}` is not JOB:GROUP"))?;
        let job = byte("a job", job, text)?;
        let group = byte("a skill group", group, text)?;
        if job > 3 || !(1..=2).contains(&group) {
            bail!("a teacher's job is 0 to 3 and its group 1 or 2 (in `{text}`)");
        }
        world.teacher = Some(Teacher {
            vid: 0,
            job,
            group,
            level: 5,
        });
    }
    if let Some(text) = &args.choose_skill {
        let skills = text
            .split(',')
            .map(|skill| byte("a skill", skill.trim(), text))
            .collect::<Result<Vec<u8>>>()?;
        if skills.is_empty() || skills.len() > 6 {
            bail!("a soul stone offers 1 to 6 skills (in `{text}`)");
        }
        world.skill_choice = Some(SkillChoice { slot: 0, skills });
    }
    if let Some(text) = &args.mount {
        let (stamina, most) = text
            .split_once('/')
            .with_context(|| format!("`{text}` is not STAMINA/MOST"))?;
        let number = |n: &str| {
            n.parse::<u16>()
                .with_context(|| format!("`{n}` is not a stamina (in `{text}`)"))
        };
        world.mount = Some(m2_core::Mount {
            level: 1,
            grade: 1,
            look: 0,
            stamina: number(stamina)?,
            most: number(most)?,
            ride_from: 0,
            fights: false,
            skills: false,
            skill_points: 0,
            summoned: None,
        });
    }
    if let Some(level) = args.stable {
        let costs = world
            .items
            .values()
            .map(|item| m2_core::MountCost {
                vnum: item.vnum,
                count: 2,
            })
            .collect();
        world.stable = Some(m2_core::Stable {
            vid: 0,
            next: Some(m2_core::MountOffer {
                level,
                grade: 1,
                look: 0,
                move_speed: 200,
                stamina: 60,
                rider_level: if level > 1 { 5 } else { 0 },
                gold: if level > 1 { 500 } else { 0 },
                items: if level > 1 { costs } else { Vec::new() },
                skill_points: 0,
            }),
        });
    }
    if let Some(next) = args.upgrade {
        let mut bag = world
            .items
            .iter()
            .filter(|((window, slot), _)| {
                *window == m2_core::Window::Inventory && *slot < m2_core::BAG_SLOTS
            })
            .map(|((_, slot), item)| (*slot, item.vnum));
        let (slot, vnum) = bag
            .next()
            .context("--upgrade needs an --item in the bag to upgrade")?;
        let items = bag
            .map(|(_, vnum)| m2_core::UpgradeCost { vnum, count: 2 })
            .take(4)
            .collect();
        world.upgrade = Some(m2_core::UpgradeOffer {
            smith: Some(0),
            scroll: None,
            slot,
            vnum,
            next,
            gold: 1_200,
            items,
            chance: 50,
            on_fail: m2_core::UpgradeFailure::Destroyed,
        });
    }
    Ok(world)
}

/// Fills the inventory, quickslots, shop and character window from
/// `--item`, `--quick`, `--good`, `--point` and the skill arguments, with
/// the items and skills of `source` if there is one. `--show skills` opens
/// the character window on its Skills tab, `--show character` on its
/// Character tab.
fn fill_inventory(args: &UiArgs, ui: &mut Ui, source: Option<&Source>) -> Result<()> {
    let shown = |id: &str| args.show.iter().any(|shown| shown == id);
    let skills_shown = shown(interface::ids::SKILLS);
    let character_shown = shown(interface::ids::CHARACTER);
    if args.item.is_empty()
        && args.quick.is_empty()
        && args.good.is_empty()
        && args.skill.is_empty()
        && args.quick_skill.is_empty()
        && args.cooldown.is_empty()
        && args.affect.is_empty()
        && args.point.is_empty()
        && args.teacher.is_none()
        && args.choose_skill.is_none()
        && args.mount.is_none()
        && args.stable.is_none()
        && args.upgrade.is_none()
        && !skills_shown
        && !character_shown
    {
        return Ok(());
    }
    if !ui.has(interface::ids::INVENTORY_BAG) {
        bail!(
            "--item, --quick, --good, --teacher, --choose-skill, --point, --skill and --quick-skill need --screen game"
        );
    }
    let mut inventory = Inventory::new();
    let mut shop = ShopWindow::new();
    let mut teacher = TeacherWindow::new();
    let mut choice = SkillChoiceWindow::new();
    let mut stable = StableWindow::new();
    let mut upgrade = UpgradeWindow::new();
    let mut skills = SkillWindow::new();
    let mut character = CharacterWindow::new();
    let mut affects = AffectsBar::new();
    let mut items = None;
    if let Some(source) = source {
        match source.items() {
            Ok(read) => {
                inventory.set_items(read.clone());
                shop.set_items(read.clone());
                stable.set_items(read.clone());
                upgrade.set_items(read.clone());
                items = Some(read);
            }
            Err(error) => eprintln!("note: items: {error:#}; the items show no names or icons"),
        }
        match source.skills() {
            Ok(read) => {
                teacher.set_skills(read.clone());
                choice.set_skills(read.clone());
                affects.set_skills(read.clone());
                skills.set_skills(read);
            }
            Err(error) => eprintln!("note: skills: {error:#}; the skills window shows numbers"),
        }
    }
    let mut world = items_world(args, items.as_ref())?;
    for text in &args.affect {
        world.affects.push(parse_affect(text)?);
    }
    let mut cooldowns = BTreeMap::new();
    for text in &args.cooldown {
        let (skill, seconds) = pair(text)?;
        let skill: u8 = skill
            .parse()
            .with_context(|| format!("`{skill}` is not a skill (in `{text}`)"))?;
        let seconds: u32 = seconds
            .parse()
            .with_context(|| format!("`{seconds}` is not a number of seconds (in `{text}`)"))?;
        cooldowns.insert(skill, Cooldown { left: 500, seconds });
    }
    inventory.set_cooldowns(cooldowns.clone());
    skills.set_cooldowns(cooldowns);
    inventory.set_skills(skills.shown(&world));
    inventory.update(ui, &world);
    shop.update(ui, &world, None);
    teacher.update(ui, &world, None);
    choice.update(ui, &world, None);
    stable.update(ui, &world, None);
    upgrade.update(ui, &world, None);
    if let Some(mount) = world.mount {
        ui.set_visible(interface::ids::STAMINA, true);
        let most = f32::from(mount.most.max(1));
        ui.set_value(interface::ids::STAMINA, f32::from(mount.stamina) / most);
    }
    if skills_shown {
        character.open(ui, Tab::Skills);
    } else if character_shown {
        character.open(ui, Tab::Status);
    }
    character.update(ui, &world);
    skills.update(ui, &world);
    affects.update(ui, &world, Millis(0));
    Ok(())
}

/// Fills the whisper windows and the friends list from `--whisper`,
/// `--whisper-to`, `--open-whisper`, `--friend` and `--friend-request`; we
/// are Hero.
fn fill_social(args: &UiArgs, ui: &mut Ui) -> Result<()> {
    if args.whisper.is_empty()
        && args.whisper_to.is_empty()
        && args.open_whisper.is_none()
        && args.friend.is_empty()
        && args.friend_request.is_none()
    {
        return Ok(());
    }
    if !ui.has(interface::ids::WHISPERS) {
        bail!(
            "--whisper, --whisper-to, --open-whisper, --friend and --friend-request need --screen game"
        );
    }
    let mut world = World::default();
    for (n, (text, kind)) in args
        .whisper
        .iter()
        .map(|text| (text, WhisperKind::Received))
        .chain(args.whisper_to.iter().map(|text| (text, WhisperKind::Sent)))
        .enumerate()
    {
        let (name, line) = pair(text)?;
        let line = WhisperLine {
            at: m2_core::Millis(n as u64),
            kind,
            text: line.to_owned(),
        };
        world.push_whisper(name, line);
    }
    for text in &args.friend {
        let (name, online) = match text.split_once(':') {
            Some((name, "online")) => (name, true),
            Some(_) => bail!("a friend is NAME or NAME:online, not `{text}`"),
            None => (text.as_str(), false),
        };
        let friend = Friend {
            name: name.to_owned(),
            online,
        };
        world.friends.insert(name.to_ascii_lowercase(), friend);
    }
    world.friend_requests.extend(args.friend_request.clone());
    let mut whispers = WhisperWindows::new();
    if let Some(name) = &args.open_whisper {
        whispers.open(ui, name);
        ui.blur();
    }
    whispers.update(ui, &world, "Hero", m2_core::Millis(0));
    FriendsWindow::new().update(ui, &world);
    Ok(())
}

/// `--affect`'s SKILL:POINT=AMOUNT/SECONDS, its time from 0.
fn parse_affect(text: &str) -> Result<Affect> {
    let (head, tail) = pair(text)?;
    let (skill, point) = head
        .split_once(':')
        .with_context(|| format!("`{text}` is not SKILL:POINT=AMOUNT/SECONDS"))?;
    let (amount, seconds) = tail
        .split_once('/')
        .with_context(|| format!("`{text}` is not SKILL:POINT=AMOUNT/SECONDS"))?;
    let number = |what: &str, value: &str| -> Result<i64> {
        value
            .parse()
            .with_context(|| format!("`{value}` is not {what} (in `{text}`)"))
    };
    let seconds = u64::try_from(number("a number of seconds", seconds)?)
        .with_context(|| format!("`{seconds}` is not a number of seconds (in `{text}`)"))?;
    Ok(Affect {
        skill: u8::try_from(number("a skill", skill)?).context("a skill is 0 to 255")?,
        point: u8::try_from(number("a point", point)?).context("a point is 0 to 255")?,
        amount: i32::try_from(number("an amount", amount)?).context("an amount is too large")?,
        until: (seconds > 0).then(|| Millis(seconds.saturating_mul(1_000))),
    })
}

/// The interface with the layouts and the values from the arguments; the
/// skin, the items' icons and names from `source`.
fn build(args: &UiArgs, source: Option<&Source>) -> Result<Ui> {
    let skins = match source {
        Some(source) => source.skin()?,
        None => interface::built_in_skin()?,
    };
    let mut ui = match args.screen {
        Some(screen) => {
            let mut ui = screens::play_ui(skins)?;
            screens::show_screen(&mut ui, screen, true);
            ui
        }
        None => {
            let texts: Vec<String> = if args.layouts.is_empty() {
                vec![interface::GAME.to_owned()]
            } else {
                args.layouts
                    .iter()
                    .map(|path| {
                        std::fs::read_to_string(path)
                            .with_context(|| format!("reading {}", path.display()))
                    })
                    .collect::<Result<_>>()?
            };
            let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
            interface::new_ui(skins, &refs)?
        }
    };
    fill_inventory(args, &mut ui, source)?;
    fill_social(args, &mut ui)?;
    let unknown = |id: &str, what: &str| anyhow!("no {what} has the id `{id}`");
    for item in &args.set {
        let (id, text) = pair(item)?;
        if !ui.set_text(id, text) {
            bail!(unknown(id, "text, button or field"));
        }
    }
    for item in &args.value {
        let (id, value) = pair(item)?;
        let value: f32 = value
            .parse()
            .with_context(|| format!("`{value}` is not a number"))?;
        if !ui.set_value(id, value) {
            bail!(unknown(id, "gauge"));
        }
    }
    for item in &args.line {
        let (id, text) = pair(item)?;
        if !ui.push_line(id, text, [232, 228, 214, 255]) {
            bail!(unknown(id, "log"));
        }
    }
    for id in &args.show {
        if !ui.set_visible(id, true) {
            bail!(unknown(id, "node"));
        }
    }
    if let Some(id) = &args.focus
        && !ui.focus(id)
    {
        bail!(unknown(id, "field"));
    }
    Ok(ui)
}

fn background() -> Scene {
    let mut scene = Scene::new(Camera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance: 100.0,
        fov_y: Camera::FOV_Y,
    });
    scene.background = BACKGROUND;
    scene
}

/// Runs `view ui`.
pub fn run(args: UiArgs) -> Result<ExitCode> {
    let source = args.assets.as_deref().map(Source::open).transpose()?;
    let mut ui = build(&args, source.as_ref())?;
    let options = GpuOptions {
        software: args.software,
        ..GpuOptions::default()
    };
    match &args.screenshot {
        Some(out) => screenshot(&args, &mut ui, source.as_ref(), options, out)?,
        None => window(&args, ui, source, options)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn screenshot(
    args: &UiArgs,
    ui: &mut Ui,
    source: Option<&Source>,
    options: GpuOptions,
    out: &Path,
) -> Result<()> {
    let files = source.map(Source::files);
    let gpu = Gpu::headless(options)?;
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT, 1)?;
    ui.set_scale(args.scale.scale(None));
    let filter = args.scale.filter();
    let mut gpu_ui = GpuInterface::new(&gpu, OFFSCREEN_FORMAT, ui, files.as_ref(), filter)?;
    let (width, height) = args.size;
    let target = Offscreen::new(&gpu, &renderer, width, height)?;
    renderer.draw(
        &gpu,
        target.attachments(),
        target.view(),
        &background(),
        &[],
    )?;
    gpu_ui.draw(&gpu, ui, files.as_ref(), target.view(), args.size)?;
    let image = target.read(&gpu)?;
    std::fs::write(out, m2_render::png::encode(&image)?)
        .with_context(|| format!("writing {}", out.display()))?;
    println!(
        "wrote {} ({width}×{height}) with {}",
        out.display(),
        gpu.adapter_name()
    );
    Ok(())
}

fn window(args: &UiArgs, ui: Ui, source: Option<Source>, options: GpuOptions) -> Result<()> {
    let mut app = UiWindow {
        size: args.size,
        scale: args.scale,
        options,
        ui,
        source,
        state: None,
        error: None,
    };
    let event_loop = EventLoop::new().context("opening a window")?;
    event_loop.run_app(&mut app).context("running the window")?;
    match app.error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

struct UiWindow {
    size: (u32, u32),
    scale: ScaleArgs,
    options: GpuOptions,
    ui: Ui,
    source: Option<Source>,
    state: Option<(Surface, GpuInterface)>,
    error: Option<anyhow::Error>,
}

impl UiWindow {
    fn redraw(&mut self) -> Result<()> {
        let Some((surface, gpu_ui)) = &mut self.state else {
            return Ok(());
        };
        let Some((frame, view)) = surface.frame()? else {
            return Ok(());
        };
        surface.renderer.draw(
            &surface.gpu,
            &surface.attachments,
            &view,
            &background(),
            &[],
        )?;
        gpu_ui.draw(
            &surface.gpu,
            &mut self.ui,
            self.source.as_ref().map(Source::files).as_ref(),
            &view,
            surface.attachments.size(),
        )?;
        surface.present(frame);
        for event in self.ui.take_events() {
            match event {
                UiEvent::Clicked(id) => println!("clicked {id}"),
                UiEvent::Submitted { id, text } => println!("submitted {id}: {text}"),
                UiEvent::Cancelled(id) => println!("left {id}"),
                UiEvent::SlotPressed { id, cell, button } => {
                    println!("pressed {id} cell {cell} ({button:?})");
                }
                UiEvent::SlotReleased { id, cell } => println!("released over {id} cell {cell}"),
                UiEvent::Changed(id) => {
                    println!("changed {id}: {:.2}", self.ui.value(&id).unwrap_or(0.0));
                }
            }
        }
        Ok(())
    }
}

impl ApplicationHandler for UiWindow {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let opened = Surface::open(event_loop, "openm2 view ui", self.size, self.options).and_then(
            |surface| {
                let display = surface.window.scale_factor();
                self.ui.set_scale(self.scale.scale(Some(display)));
                let gpu_ui = GpuInterface::new(
                    &surface.gpu,
                    surface.renderer.format(),
                    &mut self.ui,
                    self.source.as_ref().map(Source::files).as_ref(),
                    self.scale.filter(),
                )?;
                Ok((surface, gpu_ui))
            },
        );
        match opened {
            Ok(state) => {
                state.0.window.request_redraw();
                self.state = Some(state);
            }
            Err(error) => {
                self.error = Some(error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some((surface, _)) = &mut self.state {
                    surface.resize(size);
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if self.scale.follows_display() {
                    self.ui.set_scale(scale_factor as f32);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.redraw() {
                    self.error = Some(error);
                    event_loop.exit();
                    return;
                }
                if let Some((surface, _)) = &self.state {
                    surface.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        text,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                let taken = interface::ui_key(&logical_key).is_some_and(|key| self.ui.key(key))
                    || text.as_deref().is_some_and(|text| self.ui.text_input(text));
                if !taken && logical_key == Key::Named(NamedKey::Escape) {
                    event_loop.exit();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.ui.pointer_moved(position.x as f32, position.y as f32);
            }
            WindowEvent::CursorLeft { .. } => self.ui.pointer_left(),
            WindowEvent::MouseInput { state, button, .. } => match (button, state) {
                (MouseButton::Left, ElementState::Pressed) => {
                    self.ui.press();
                }
                (MouseButton::Left, ElementState::Released) => {
                    self.ui.release();
                }
                (MouseButton::Right, ElementState::Pressed) => {
                    self.ui.press_secondary();
                }
                _ => {}
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => interface::wheel_lines(p.y, self.ui.scale()),
                };
                self.ui.wheel(lines);
            }
            _ => {}
        }
    }
}
