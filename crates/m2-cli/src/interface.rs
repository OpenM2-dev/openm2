//! The interface of openm2's windows (`play`, `view ui`): the built-in skin
//! and layouts (`crates/m2-cli/ui/`), an asset folder's skin and art, the
//! game's values written into the status, chat, target, restart and drop windows,
//! and drawing over a target (`docs/specs/ui.md`). `play`'s screens are in
//! `screens.rs`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result, bail};
use clap::{Args, ValueEnum};
use m2_assets::items::Monsters;
use m2_assets::skin_file::{ImageRef, SkinFile};
use m2_core::{ChatKind, ChatLine, Entity, Millis, World};
use m2_formats::RgbaImage;
use m2_render::png::decode_any;
use m2_render::{Gpu, UiFilter, UiRenderer};
use m2_ui::{Colour, Fonts, ImageId, Layout, Rect, Skin, Skins, Sprite, Ui};

use crate::inventory::grouped;
use crate::view::Files;

/// The skin file.
pub(crate) const SKIN: &str = include_str!("../ui/skin.ron");
/// The in-game layout: status, chat, the target, the minimap and the atlas,
/// the restart window, the drop question and the question before
/// forgetting.
pub(crate) const GAME: &str = include_str!("../ui/game.ron");
/// The background of the login and character screens, and loading.
pub(crate) const INTRO: &str = include_str!("../ui/intro.ron");
/// The login screen's Sign in board.
pub(crate) const LOGIN: &str = include_str!("../ui/login.ron");
/// The login screen's Servers board, and its dialog and question.
pub(crate) const SERVERS: &str = include_str!("../ui/servers.ron");
/// The character select and create screens.
pub(crate) const SELECT: &str = include_str!("../ui/select.ron");
/// The inventory and the quickslot bar.
pub(crate) const INVENTORY: &str = include_str!("../ui/inventory.ron");
/// An NPC's shop and the question before trading.
pub(crate) const SHOP: &str = include_str!("../ui/shop.ron");
/// A teacher NPC's window: the skill group it teaches.
pub(crate) const TEACHER: &str = include_str!("../ui/teacher.ron");
/// The window of an item that trains a skill we choose (a soul stone).
pub(crate) const CHOOSE_SKILL: &str = include_str!("../ui/choose_skill.ron");
/// A stable NPC's window: our mount and the level it trains it to next.
pub(crate) const STABLE: &str = include_str!("../ui/stable.ron");
/// An upgrade offered at a smith or with a scroll.
pub(crate) const UPGRADE: &str = include_str!("../ui/upgrade.ron");
/// The character window: its stats on one tab, its skills on the other.
pub(crate) const CHARACTER: &str = include_str!("../ui/character.ron");
/// The game menu and the Settings window.
pub(crate) const MENU: &str = include_str!("../ui/menu.ron");
/// Whispers: the envelopes and the conversation window.
pub(crate) const WHISPER: &str = include_str!("../ui/whisper.ron");
/// The friends list and the question about a friend.
pub(crate) const MESSENGER: &str = include_str!("../ui/messenger.ron");

/// Node ids of the built-in layouts that `play` fills in, shows and reads.
pub(crate) mod ids {
    // game.ron
    pub const CHAT: &str = "chat";
    pub const CHAT_LOG: &str = "chat.log";
    pub const CHAT_BAR: &str = "chat.bar";
    pub const CHAT_INPUT: &str = "chat.input";
    pub const AFFECTS: &str = "affects";
    pub const AFFECTS_ICONS: &str = "affects.icons";
    pub const TARGET: &str = "target";
    pub const TARGET_NAME: &str = "target.name";
    pub const TARGET_HEALTH: &str = "target.health";
    pub const RESTART: &str = "restart";
    pub const RESTART_HERE: &str = "restart.here";
    pub const RESTART_TOWN: &str = "restart.town";
    pub const DROP: &str = "drop";
    pub const DROP_TEXT: &str = "drop.text";
    pub const DROP_YES: &str = "drop.yes";
    pub const DROP_NO: &str = "drop.no";
    pub const FORGET: &str = "forget";
    pub const FORGET_YES: &str = "forget.yes";
    pub const FORGET_NO: &str = "forget.no";
    pub const MINIMAP: &str = "minimap";
    pub const MINIMAP_PICTURE: &str = "minimap.picture";
    pub const MINIMAP_ATLAS: &str = "minimap.atlas";
    pub const MINIMAP_CLOSE: &str = "minimap.close";
    pub const MINIMAP_ZOOM_OUT: &str = "minimap.zoom_out";
    pub const MINIMAP_ZOOM_IN: &str = "minimap.zoom_in";
    pub const MINIMAP_CLOSED: &str = "minimap.closed";
    pub const MINIMAP_OPEN: &str = "minimap.open";
    pub const ATLAS: &str = "atlas";
    pub const ATLAS_NAME: &str = "atlas.name";
    pub const ATLAS_CLOSE: &str = "atlas.close";
    pub const ATLAS_PICTURE: &str = "atlas.picture";
    // inventory.ron
    pub const TASKBAR: &str = "taskbar";
    pub const TASKBAR_QUICK: &str = "taskbar.quick";
    /// The quickslot bar's page buttons, and its number between them.
    pub const TASKBAR_PAGE_UP: &str = "taskbar.page_up";
    pub const TASKBAR_PAGE_DOWN: &str = "taskbar.page_down";
    pub const TASKBAR_PAGE: &str = "taskbar.page";
    pub const TASKBAR_CHARACTER: &str = "taskbar.character";
    pub const TASKBAR_INVENTORY: &str = "taskbar.inventory";
    pub const TASKBAR_SYSTEM: &str = "taskbar.system";
    pub const TASKBAR_MESSENGER: &str = "taskbar.messenger";
    pub const GLOBE: &str = "taskbar.globe";
    pub const HEALTH: &str = "taskbar.health";
    pub const MANA: &str = "taskbar.mana";
    /// Our mount's stamina, in the base's thin trough; shown with a mount.
    pub const STAMINA: &str = "taskbar.stamina";
    pub const EXPERIENCE: &str = "taskbar.experience";
    /// The experience's orbs, a quarter of the level each.
    pub const EXPERIENCE_ORBS: [&str; 4] = [
        "taskbar.experience0",
        "taskbar.experience1",
        "taskbar.experience2",
        "taskbar.experience3",
    ];
    pub const INVENTORY: &str = "inventory";
    pub const INVENTORY_CLOSE: &str = "inventory.close";
    pub const INVENTORY_PAGE1: &str = "inventory.page1";
    pub const INVENTORY_PAGE2: &str = "inventory.page2";
    pub const INVENTORY_BAG: &str = "inventory.bag";
    pub const INVENTORY_GOLD: &str = "inventory.gold";
    pub const WORN_BODY: &str = "inventory.body";
    pub const WORN_HEAD: &str = "inventory.head";
    pub const WORN_SHOES: &str = "inventory.shoes";
    pub const WORN_BRACELET: &str = "inventory.bracelet";
    pub const WORN_WEAPON: &str = "inventory.weapon";
    pub const WORN_NECKLACE: &str = "inventory.necklace";
    pub const WORN_EARRINGS: &str = "inventory.earrings";
    pub const WORN_UNIQUE1: &str = "inventory.unique1";
    pub const WORN_UNIQUE2: &str = "inventory.unique2";
    pub const WORN_ARROW: &str = "inventory.arrow";
    pub const WORN_SHIELD: &str = "inventory.shield";
    // shop.ron
    pub const SHOP: &str = "shop";
    pub const SHOP_TITLE: &str = "shop.title";
    pub const SHOP_CLOSE: &str = "shop.close";
    pub const SHOP_GOODS: &str = "shop.goods";
    pub const TRADE: &str = "trade";
    pub const TRADE_TEXT: &str = "trade.text";
    pub const TRADE_YES: &str = "trade.yes";
    pub const TRADE_NO: &str = "trade.no";
    // teacher.ron
    pub const TEACHER: &str = "teacher";
    pub const TEACHER_TITLE: &str = "teacher.title";
    pub const TEACHER_CLOSE: &str = "teacher.close";
    pub const TEACHER_TEXT: &str = "teacher.text";
    pub const TEACHER_LEARN: &str = "teacher.learn";
    /// The rows' icons and names, top to bottom.
    pub const TEACHER_ICON: [&str; 6] = [
        "teacher.icon0",
        "teacher.icon1",
        "teacher.icon2",
        "teacher.icon3",
        "teacher.icon4",
        "teacher.icon5",
    ];
    pub const TEACHER_NAME: [&str; 6] = [
        "teacher.name0",
        "teacher.name1",
        "teacher.name2",
        "teacher.name3",
        "teacher.name4",
        "teacher.name5",
    ];
    // choose_skill.ron
    pub const CHOOSE_SKILL: &str = "choose_skill";
    pub const CHOOSE_SKILL_TITLE: &str = "choose_skill.title";
    pub const CHOOSE_SKILL_CLOSE: &str = "choose_skill.close";
    /// The rows' icons, names, levels and Train buttons, top to bottom.
    pub const CHOOSE_SKILL_ICON: [&str; 6] = [
        "choose_skill.icon0",
        "choose_skill.icon1",
        "choose_skill.icon2",
        "choose_skill.icon3",
        "choose_skill.icon4",
        "choose_skill.icon5",
    ];
    pub const CHOOSE_SKILL_NAME: [&str; 6] = [
        "choose_skill.name0",
        "choose_skill.name1",
        "choose_skill.name2",
        "choose_skill.name3",
        "choose_skill.name4",
        "choose_skill.name5",
    ];
    pub const CHOOSE_SKILL_LEVEL: [&str; 6] = [
        "choose_skill.level0",
        "choose_skill.level1",
        "choose_skill.level2",
        "choose_skill.level3",
        "choose_skill.level4",
        "choose_skill.level5",
    ];
    pub const CHOOSE_SKILL_TRAIN: [&str; 6] = [
        "choose_skill.train0",
        "choose_skill.train1",
        "choose_skill.train2",
        "choose_skill.train3",
        "choose_skill.train4",
        "choose_skill.train5",
    ];
    // stable.ron
    pub const STABLE: &str = "stable";
    pub const STABLE_TITLE: &str = "stable.title";
    pub const STABLE_CLOSE: &str = "stable.close";
    pub const STABLE_MOUNT: &str = "stable.mount";
    pub const STABLE_NEXT: &str = "stable.next";
    pub const STABLE_NEEDS: &str = "stable.needs";
    pub const STABLE_TRAIN: &str = "stable.train";
    /// The lines of the items the next level costs, top to bottom.
    pub const STABLE_COST: [&str; 4] = [
        "stable.cost0",
        "stable.cost1",
        "stable.cost2",
        "stable.cost3",
    ];
    // upgrade.ron
    pub const UPGRADE: &str = "upgrade";
    pub const UPGRADE_TITLE: &str = "upgrade.title";
    pub const UPGRADE_CLOSE: &str = "upgrade.close";
    pub const UPGRADE_ICON: &str = "upgrade.icon";
    pub const UPGRADE_NOW: &str = "upgrade.now";
    pub const UPGRADE_NEXT: &str = "upgrade.next";
    pub const UPGRADE_CHANCE: &str = "upgrade.chance";
    pub const UPGRADE_GOLD: &str = "upgrade.gold";
    /// The lines of the items an upgrade costs, top to bottom.
    pub const UPGRADE_COST: [&str; 4] = [
        "upgrade.cost0",
        "upgrade.cost1",
        "upgrade.cost2",
        "upgrade.cost3",
    ];
    pub const UPGRADE_FAIL: &str = "upgrade.fail";
    pub const UPGRADE_OK: &str = "upgrade.ok";
    pub const UPGRADE_CANCEL: &str = "upgrade.cancel";
    // character.ron
    pub const CHARACTER: &str = "character";
    pub const CHARACTER_TITLE: &str = "character.title";
    pub const CHARACTER_CLOSE: &str = "character.close";
    pub const CHARACTER_TAB_STATUS: &str = "character.tab_status";
    pub const CHARACTER_TAB_SKILLS: &str = "character.tab_skills";
    /// The Character tab's page.
    pub const CHARACTER_STATUS: &str = "character.status";
    pub const CHARACTER_FACE: &str = "character.face";
    pub const CHARACTER_NAME: &str = "character.name";
    pub const CHARACTER_CLASS: &str = "character.class";
    pub const CHARACTER_EXPERIENCE: &str = "character.experience";
    pub const CHARACTER_ST: &str = "character.st";
    pub const CHARACTER_HT: &str = "character.ht";
    pub const CHARACTER_DX: &str = "character.dx";
    pub const CHARACTER_IQ: &str = "character.iq";
    pub const CHARACTER_ST_UP: &str = "character.st_up";
    pub const CHARACTER_HT_UP: &str = "character.ht_up";
    pub const CHARACTER_DX_UP: &str = "character.dx_up";
    pub const CHARACTER_IQ_UP: &str = "character.iq_up";
    pub const CHARACTER_POINTS: &str = "character.points";
    pub const CHARACTER_HEALTH: &str = "character.health";
    pub const CHARACTER_MANA: &str = "character.mana";
    pub const CHARACTER_ATTACK: &str = "character.attack";
    pub const CHARACTER_DEFENCE: &str = "character.defence";
    pub const CHARACTER_ATTACK_SPEED: &str = "character.attack_speed";
    pub const CHARACTER_MOVE_SPEED: &str = "character.move_speed";
    /// The four stats' values and + buttons: strength, vitality, dexterity,
    /// intelligence.
    pub const CHARACTER_STATS: [&str; 4] = [CHARACTER_ST, CHARACTER_HT, CHARACTER_DX, CHARACTER_IQ];
    pub const CHARACTER_STATS_UP: [&str; 4] = [
        CHARACTER_ST_UP,
        CHARACTER_HT_UP,
        CHARACTER_DX_UP,
        CHARACTER_IQ_UP,
    ];
    /// The Skills tab's page.
    pub const SKILLS: &str = "skills";
    pub const SKILLS_POINTS: &str = "skills.points";
    /// The group's name, and the buttons that look at each group
    /// before one is chosen.
    pub const SKILLS_GROUP: &str = "skills.group";
    pub const SKILLS_GROUPS: [&str; 2] = ["skills.group1", "skills.group2"];
    /// Each skill's icons, covers and level marks, a grade each; its +.
    pub const SKILLS_ICON: [[&str; 3]; 6] = [
        ["skills.icon0_0", "skills.icon0_1", "skills.icon0_2"],
        ["skills.icon1_0", "skills.icon1_1", "skills.icon1_2"],
        ["skills.icon2_0", "skills.icon2_1", "skills.icon2_2"],
        ["skills.icon3_0", "skills.icon3_1", "skills.icon3_2"],
        ["skills.icon4_0", "skills.icon4_1", "skills.icon4_2"],
        ["skills.icon5_0", "skills.icon5_1", "skills.icon5_2"],
    ];
    pub const SKILLS_COVER: [[&str; 3]; 6] = [
        ["skills.cover0_0", "skills.cover0_1", "skills.cover0_2"],
        ["skills.cover1_0", "skills.cover1_1", "skills.cover1_2"],
        ["skills.cover2_0", "skills.cover2_1", "skills.cover2_2"],
        ["skills.cover3_0", "skills.cover3_1", "skills.cover3_2"],
        ["skills.cover4_0", "skills.cover4_1", "skills.cover4_2"],
        ["skills.cover5_0", "skills.cover5_1", "skills.cover5_2"],
    ];
    pub const SKILLS_MARK: [[&str; 3]; 6] = [
        ["skills.mark0_0", "skills.mark0_1", "skills.mark0_2"],
        ["skills.mark1_0", "skills.mark1_1", "skills.mark1_2"],
        ["skills.mark2_0", "skills.mark2_1", "skills.mark2_2"],
        ["skills.mark3_0", "skills.mark3_1", "skills.mark3_2"],
        ["skills.mark4_0", "skills.mark4_1", "skills.mark4_2"],
        ["skills.mark5_0", "skills.mark5_1", "skills.mark5_2"],
    ];
    pub const SKILLS_UP: [&str; 6] = [
        "skills.up0",
        "skills.up1",
        "skills.up2",
        "skills.up3",
        "skills.up4",
        "skills.up5",
    ];
    // menu.ron
    pub const MENU: &str = "menu";
    pub const MENU_CLOSE: &str = "menu.close";
    pub const MENU_SETTINGS: &str = "menu.settings";
    pub const MENU_CHARACTERS: &str = "menu.characters";
    pub const MENU_LOGOUT: &str = "menu.logout";
    pub const MENU_QUIT: &str = "menu.quit";
    pub const MENU_BACK: &str = "menu.back";
    pub const SETTINGS: &str = "settings";
    pub const SETTINGS_CLOSE: &str = "settings.close";
    pub const SETTINGS_MUSIC: &str = "settings.music";
    pub const SETTINGS_MUSIC_VALUE: &str = "settings.music.value";
    pub const SETTINGS_SOUND: &str = "settings.sound";
    pub const SETTINGS_SOUND_VALUE: &str = "settings.sound.value";
    pub const SETTINGS_MUTE: &str = "settings.mute";
    pub const SETTINGS_SIZE_DOWN: &str = "settings.size.down";
    pub const SETTINGS_SIZE_UP: &str = "settings.size.up";
    pub const SETTINGS_SIZE_VALUE: &str = "settings.size.value";
    pub const SETTINGS_NAMES: &str = "settings.names";
    pub const SETTINGS_BACK: &str = "settings.back";
    // whisper.ron
    pub const WHISPERS: &str = "whispers";
    /// The conversations' envelopes, in their order, and their marks.
    pub const WHISPER_ENVELOPES: [&str; 8] = [
        "whisper.envelope0",
        "whisper.envelope1",
        "whisper.envelope2",
        "whisper.envelope3",
        "whisper.envelope4",
        "whisper.envelope5",
        "whisper.envelope6",
        "whisper.envelope7",
    ];
    pub const WHISPER_UNREAD: [&str; 8] = [
        "whisper.unread0",
        "whisper.unread1",
        "whisper.unread2",
        "whisper.unread3",
        "whisper.unread4",
        "whisper.unread5",
        "whisper.unread6",
        "whisper.unread7",
    ];
    pub const WHISPER: &str = "whisper";
    pub const WHISPER_TITLE: &str = "whisper.title";
    pub const WHISPER_CLOSE: &str = "whisper.close";
    pub const WHISPER_LOG: &str = "whisper.log";
    pub const WHISPER_INPUT: &str = "whisper.input";
    pub const WHISPER_END: &str = "whisper.end";
    // messenger.ron
    pub const MESSENGER: &str = "messenger";
    pub const MESSENGER_CLOSE: &str = "messenger.close";
    /// A friend a row, and whether it is in the game.
    pub const MESSENGER_ROWS: [&str; 10] = [
        "messenger.row0",
        "messenger.row1",
        "messenger.row2",
        "messenger.row3",
        "messenger.row4",
        "messenger.row5",
        "messenger.row6",
        "messenger.row7",
        "messenger.row8",
        "messenger.row9",
    ];
    pub const MESSENGER_STATUS: [&str; 10] = [
        "messenger.status0",
        "messenger.status1",
        "messenger.status2",
        "messenger.status3",
        "messenger.status4",
        "messenger.status5",
        "messenger.status6",
        "messenger.status7",
        "messenger.status8",
        "messenger.status9",
    ];
    pub const MESSENGER_EMPTY: &str = "messenger.empty";
    pub const MESSENGER_PREV: &str = "messenger.prev";
    pub const MESSENGER_PAGE: &str = "messenger.page";
    pub const MESSENGER_NEXT: &str = "messenger.next";
    pub const MESSENGER_WHISPER: &str = "messenger.whisper";
    pub const MESSENGER_REMOVE: &str = "messenger.remove";
    pub const MESSENGER_ADD: &str = "messenger.add";
    pub const MESSENGER_NAME_BAR: &str = "messenger.name_bar";
    pub const MESSENGER_NAME: &str = "messenger.name";
    pub const FRIEND_QUESTION: &str = "friend_question";
    pub const FRIEND_QUESTION_TEXT: &str = "friend_question.text";
    pub const FRIEND_QUESTION_YES: &str = "friend_question.yes";
    pub const FRIEND_QUESTION_NO: &str = "friend_question.no";
    // intro.ron
    pub const INTRO: &str = "intro";
    pub const LOADING: &str = "loading";
    pub const LOADING_TEXT: &str = "loading.text";
    pub const LOADING_GAUGE: &str = "loading.gauge";
    // login.ron
    pub const LOGO: &str = "login.logo";
    pub const LOGIN: &str = "login";
    pub const SERVER: &str = "login.server";
    pub const LOGIN_STATUS: &str = "login.status";
    pub const LOGIN_QUIT: &str = "login.quit";
    pub const LOGIN_BACK: &str = "login.back";
    pub const LOGIN_CLOSE: &str = "login.close";
    /// A button per provider, in the server's order.
    pub const PROVIDER_BUTTONS: [&str; 4] = [
        "login.provider.0",
        "login.provider.1",
        "login.provider.2",
        "login.provider.3",
    ];
    // servers.ron
    pub const SERVERS: &str = "servers";
    pub const SERVERS_CLOSE: &str = "servers.close";
    /// A row per server shown, eight a page; its name, address and state.
    pub const SERVER_ROWS: [&str; 8] = [
        "servers.row.0",
        "servers.row.1",
        "servers.row.2",
        "servers.row.3",
        "servers.row.4",
        "servers.row.5",
        "servers.row.6",
        "servers.row.7",
    ];
    pub const SERVER_NAMES: [&str; 8] = [
        "servers.row.0.name",
        "servers.row.1.name",
        "servers.row.2.name",
        "servers.row.3.name",
        "servers.row.4.name",
        "servers.row.5.name",
        "servers.row.6.name",
        "servers.row.7.name",
    ];
    pub const SERVER_PLACES: [&str; 8] = [
        "servers.row.0.place",
        "servers.row.1.place",
        "servers.row.2.place",
        "servers.row.3.place",
        "servers.row.4.place",
        "servers.row.5.place",
        "servers.row.6.place",
        "servers.row.7.place",
    ];
    pub const SERVER_STATES: [&str; 8] = [
        "servers.row.0.state",
        "servers.row.1.state",
        "servers.row.2.state",
        "servers.row.3.state",
        "servers.row.4.state",
        "servers.row.5.state",
        "servers.row.6.state",
        "servers.row.7.state",
    ];
    pub const SERVERS_EMPTY: &str = "servers.empty";
    pub const SERVERS_PREV: &str = "servers.prev";
    pub const SERVERS_PAGE: &str = "servers.page";
    pub const SERVERS_NEXT: &str = "servers.next";
    pub const SERVERS_STATUS: &str = "servers.status";
    pub const SERVERS_JOIN: &str = "servers.join";
    pub const SERVERS_ADD: &str = "servers.add";
    pub const SERVERS_EDIT: &str = "servers.edit";
    pub const SERVERS_REMOVE: &str = "servers.remove";
    pub const SERVERS_REFRESH: &str = "servers.refresh";
    pub const SERVERS_QUIT: &str = "servers.quit";
    pub const SERVER_EDIT: &str = "server_edit";
    pub const SERVER_EDIT_TITLE: &str = "server_edit.title";
    pub const SERVER_EDIT_CLOSE: &str = "server_edit.close";
    pub const SERVER_EDIT_NAME: &str = "server_edit.name";
    pub const SERVER_EDIT_ADDRESS: &str = "server_edit.address";
    pub const SERVER_EDIT_STATUS: &str = "server_edit.status";
    pub const SERVER_EDIT_SAVE: &str = "server_edit.save";
    pub const SERVER_EDIT_CANCEL: &str = "server_edit.cancel";
    pub const SERVER_QUESTION: &str = "server_question";
    pub const SERVER_QUESTION_LINES: [&str; 3] = [
        "server_question.line.0",
        "server_question.line.1",
        "server_question.line.2",
    ];
    pub const SERVER_QUESTION_YES: &str = "server_question.yes";
    pub const SERVER_QUESTION_NO: &str = "server_question.no";
    // select.ron: picking
    pub const MODEL: &str = "select.model";
    pub const SELECT: &str = "select";
    pub const SELECT_LEFT: &str = "select.left";
    pub const SELECT_RIGHT: &str = "select.right";
    pub const SELECT_STATUS: &str = "select.status";
    pub const SELECT_CREATE: &str = "select.create";
    pub const SELECT_PLAY: &str = "select.play";
    pub const SELECT_BACK: &str = "select.back";
    pub const INFO: &str = "select.info";
    pub const SELECT_TITLE: &str = "select.title";
    pub const SELECT_NAME: &str = "select.name";
    pub const SELECT_CLASS: &str = "select.class";
    pub const SELECT_LEVEL: &str = "select.level";
    pub const SELECT_EMPIRE: &str = "select.empire";
    pub const SELECT_GUILD: &str = "select.guild";
    pub const SELECT_HT: &str = "select.ht";
    pub const SELECT_IQ: &str = "select.iq";
    pub const SELECT_ST: &str = "select.st";
    pub const SELECT_DX: &str = "select.dx";
    // select.ron: making
    pub const CREATE: &str = "create";
    pub const CREATE_NAME: &str = "create.name";
    pub const CLASS_PREV: &str = "create.class_prev";
    pub const CREATE_CLASS: &str = "create.class";
    pub const CLASS_NEXT: &str = "create.class_next";
    pub const GENDER_PREV: &str = "create.gender_prev";
    pub const CREATE_GENDER: &str = "create.gender";
    pub const GENDER_NEXT: &str = "create.gender_next";
    pub const CREATE_EMPIRE: &str = "create.empire";
    pub const EMPIRE_PREV: &str = "create.empire_prev";
    pub const CREATE_EMPIRE_NAME: &str = "create.empire_name";
    pub const EMPIRE_NEXT: &str = "create.empire_next";
    pub const FLAG_SHINSOO: &str = "create.flag_shinsoo";
    pub const FLAG_CHUNJO: &str = "create.flag_chunjo";
    pub const FLAG_JINNO: &str = "create.flag_jinno";
    pub const CREATE_STATUS: &str = "create.status";
    pub const CREATE_OK: &str = "create.ok";
    pub const CREATE_CANCEL: &str = "create.cancel";
    pub const CREATE_CLOSE: &str = "create.close";

    /// Every id above but the skills window's rows (`skills.rs` checks
    /// those), for the test that checks the layouts have them.
    #[cfg(test)]
    pub const ALL: [&str; 351] = [
        CHAT,
        AFFECTS,
        AFFECTS_ICONS,
        CHAT_LOG,
        CHAT_BAR,
        CHAT_INPUT,
        TARGET,
        TARGET_NAME,
        TARGET_HEALTH,
        RESTART,
        RESTART_HERE,
        RESTART_TOWN,
        DROP,
        DROP_TEXT,
        DROP_YES,
        DROP_NO,
        FORGET,
        FORGET_YES,
        FORGET_NO,
        TASKBAR,
        TASKBAR_QUICK,
        TASKBAR_PAGE_UP,
        TASKBAR_PAGE_DOWN,
        TASKBAR_PAGE,
        TASKBAR_CHARACTER,
        TASKBAR_INVENTORY,
        TASKBAR_SYSTEM,
        GLOBE,
        HEALTH,
        MANA,
        STAMINA,
        EXPERIENCE,
        EXPERIENCE_ORBS[0],
        EXPERIENCE_ORBS[1],
        EXPERIENCE_ORBS[2],
        EXPERIENCE_ORBS[3],
        INVENTORY,
        INVENTORY_CLOSE,
        INVENTORY_PAGE1,
        INVENTORY_PAGE2,
        INVENTORY_BAG,
        INVENTORY_GOLD,
        WORN_BODY,
        WORN_HEAD,
        WORN_SHOES,
        WORN_BRACELET,
        WORN_WEAPON,
        WORN_NECKLACE,
        WORN_EARRINGS,
        WORN_UNIQUE1,
        WORN_UNIQUE2,
        WORN_ARROW,
        WORN_SHIELD,
        SHOP,
        SHOP_TITLE,
        SHOP_CLOSE,
        SHOP_GOODS,
        TRADE,
        TRADE_TEXT,
        TRADE_YES,
        TRADE_NO,
        TEACHER,
        TEACHER_TITLE,
        TEACHER_CLOSE,
        TEACHER_TEXT,
        TEACHER_LEARN,
        TEACHER_ICON[0],
        TEACHER_ICON[1],
        TEACHER_ICON[2],
        TEACHER_ICON[3],
        TEACHER_ICON[4],
        TEACHER_ICON[5],
        TEACHER_NAME[0],
        TEACHER_NAME[1],
        TEACHER_NAME[2],
        TEACHER_NAME[3],
        TEACHER_NAME[4],
        TEACHER_NAME[5],
        CHOOSE_SKILL,
        CHOOSE_SKILL_TITLE,
        CHOOSE_SKILL_CLOSE,
        CHOOSE_SKILL_ICON[0],
        CHOOSE_SKILL_ICON[1],
        CHOOSE_SKILL_ICON[2],
        CHOOSE_SKILL_ICON[3],
        CHOOSE_SKILL_ICON[4],
        CHOOSE_SKILL_ICON[5],
        CHOOSE_SKILL_NAME[0],
        CHOOSE_SKILL_NAME[1],
        CHOOSE_SKILL_NAME[2],
        CHOOSE_SKILL_NAME[3],
        CHOOSE_SKILL_NAME[4],
        CHOOSE_SKILL_NAME[5],
        CHOOSE_SKILL_LEVEL[0],
        CHOOSE_SKILL_LEVEL[1],
        CHOOSE_SKILL_LEVEL[2],
        CHOOSE_SKILL_LEVEL[3],
        CHOOSE_SKILL_LEVEL[4],
        CHOOSE_SKILL_LEVEL[5],
        CHOOSE_SKILL_TRAIN[0],
        CHOOSE_SKILL_TRAIN[1],
        CHOOSE_SKILL_TRAIN[2],
        CHOOSE_SKILL_TRAIN[3],
        CHOOSE_SKILL_TRAIN[4],
        CHOOSE_SKILL_TRAIN[5],
        STABLE,
        STABLE_TITLE,
        STABLE_CLOSE,
        STABLE_MOUNT,
        STABLE_NEXT,
        STABLE_NEEDS,
        STABLE_TRAIN,
        STABLE_COST[0],
        STABLE_COST[1],
        STABLE_COST[2],
        STABLE_COST[3],
        UPGRADE,
        UPGRADE_TITLE,
        UPGRADE_CLOSE,
        UPGRADE_ICON,
        UPGRADE_NOW,
        UPGRADE_NEXT,
        UPGRADE_CHANCE,
        UPGRADE_GOLD,
        UPGRADE_COST[0],
        UPGRADE_COST[1],
        UPGRADE_COST[2],
        UPGRADE_COST[3],
        UPGRADE_FAIL,
        UPGRADE_OK,
        UPGRADE_CANCEL,
        CHARACTER,
        CHARACTER_TITLE,
        CHARACTER_CLOSE,
        CHARACTER_TAB_STATUS,
        CHARACTER_TAB_SKILLS,
        CHARACTER_STATUS,
        CHARACTER_FACE,
        CHARACTER_NAME,
        CHARACTER_CLASS,
        CHARACTER_EXPERIENCE,
        CHARACTER_ST,
        CHARACTER_HT,
        CHARACTER_DX,
        CHARACTER_IQ,
        CHARACTER_ST_UP,
        CHARACTER_HT_UP,
        CHARACTER_DX_UP,
        CHARACTER_IQ_UP,
        CHARACTER_POINTS,
        CHARACTER_HEALTH,
        CHARACTER_MANA,
        CHARACTER_ATTACK,
        CHARACTER_DEFENCE,
        CHARACTER_ATTACK_SPEED,
        CHARACTER_MOVE_SPEED,
        SKILLS,
        SKILLS_POINTS,
        SKILLS_GROUP,
        SKILLS_GROUPS[0],
        SKILLS_GROUPS[1],
        MENU,
        MENU_CLOSE,
        MENU_SETTINGS,
        MENU_CHARACTERS,
        MENU_LOGOUT,
        MENU_QUIT,
        MENU_BACK,
        SETTINGS,
        SETTINGS_CLOSE,
        SETTINGS_MUSIC,
        SETTINGS_MUSIC_VALUE,
        SETTINGS_SOUND,
        SETTINGS_SOUND_VALUE,
        SETTINGS_MUTE,
        SETTINGS_SIZE_DOWN,
        SETTINGS_SIZE_UP,
        SETTINGS_SIZE_VALUE,
        SETTINGS_NAMES,
        SETTINGS_BACK,
        INTRO,
        LOADING,
        LOADING_TEXT,
        LOADING_GAUGE,
        LOGO,
        LOGIN,
        SERVER,
        LOGIN_STATUS,
        LOGIN_QUIT,
        LOGIN_CLOSE,
        PROVIDER_BUTTONS[0],
        PROVIDER_BUTTONS[1],
        PROVIDER_BUTTONS[2],
        PROVIDER_BUTTONS[3],
        MODEL,
        SELECT,
        SELECT_LEFT,
        SELECT_RIGHT,
        SELECT_STATUS,
        SELECT_CREATE,
        SELECT_PLAY,
        SELECT_BACK,
        INFO,
        SELECT_TITLE,
        SELECT_NAME,
        SELECT_CLASS,
        SELECT_LEVEL,
        SELECT_EMPIRE,
        SELECT_GUILD,
        SELECT_HT,
        SELECT_IQ,
        SELECT_ST,
        SELECT_DX,
        CREATE,
        CREATE_NAME,
        CLASS_PREV,
        CREATE_CLASS,
        CLASS_NEXT,
        GENDER_PREV,
        CREATE_GENDER,
        GENDER_NEXT,
        CREATE_EMPIRE,
        EMPIRE_PREV,
        CREATE_EMPIRE_NAME,
        EMPIRE_NEXT,
        FLAG_SHINSOO,
        FLAG_CHUNJO,
        FLAG_JINNO,
        CREATE_STATUS,
        CREATE_OK,
        CREATE_CANCEL,
        CREATE_CLOSE,
        TASKBAR_MESSENGER,
        WHISPERS,
        WHISPER_ENVELOPES[0],
        WHISPER_ENVELOPES[1],
        WHISPER_ENVELOPES[2],
        WHISPER_ENVELOPES[3],
        WHISPER_ENVELOPES[4],
        WHISPER_ENVELOPES[5],
        WHISPER_ENVELOPES[6],
        WHISPER_ENVELOPES[7],
        WHISPER_UNREAD[0],
        WHISPER_UNREAD[1],
        WHISPER_UNREAD[2],
        WHISPER_UNREAD[3],
        WHISPER_UNREAD[4],
        WHISPER_UNREAD[5],
        WHISPER_UNREAD[6],
        WHISPER_UNREAD[7],
        WHISPER,
        WHISPER_TITLE,
        WHISPER_CLOSE,
        WHISPER_LOG,
        WHISPER_INPUT,
        WHISPER_END,
        MESSENGER,
        MESSENGER_CLOSE,
        MESSENGER_ROWS[0],
        MESSENGER_ROWS[1],
        MESSENGER_ROWS[2],
        MESSENGER_ROWS[3],
        MESSENGER_ROWS[4],
        MESSENGER_ROWS[5],
        MESSENGER_ROWS[6],
        MESSENGER_ROWS[7],
        MESSENGER_ROWS[8],
        MESSENGER_ROWS[9],
        MESSENGER_STATUS[0],
        MESSENGER_STATUS[1],
        MESSENGER_STATUS[2],
        MESSENGER_STATUS[3],
        MESSENGER_STATUS[4],
        MESSENGER_STATUS[5],
        MESSENGER_STATUS[6],
        MESSENGER_STATUS[7],
        MESSENGER_STATUS[8],
        MESSENGER_STATUS[9],
        MESSENGER_EMPTY,
        MESSENGER_PREV,
        MESSENGER_PAGE,
        MESSENGER_NEXT,
        MESSENGER_WHISPER,
        MESSENGER_REMOVE,
        MESSENGER_ADD,
        MESSENGER_NAME_BAR,
        MESSENGER_NAME,
        FRIEND_QUESTION,
        FRIEND_QUESTION_TEXT,
        FRIEND_QUESTION_YES,
        FRIEND_QUESTION_NO,
        SERVERS,
        SERVERS_CLOSE,
        SERVER_ROWS[0],
        SERVER_ROWS[1],
        SERVER_ROWS[2],
        SERVER_ROWS[3],
        SERVER_ROWS[4],
        SERVER_ROWS[5],
        SERVER_ROWS[6],
        SERVER_ROWS[7],
        SERVER_NAMES[0],
        SERVER_NAMES[1],
        SERVER_NAMES[2],
        SERVER_NAMES[3],
        SERVER_NAMES[4],
        SERVER_NAMES[5],
        SERVER_NAMES[6],
        SERVER_NAMES[7],
        SERVER_PLACES[0],
        SERVER_PLACES[1],
        SERVER_PLACES[2],
        SERVER_PLACES[3],
        SERVER_PLACES[4],
        SERVER_PLACES[5],
        SERVER_PLACES[6],
        SERVER_PLACES[7],
        SERVER_STATES[0],
        SERVER_STATES[1],
        SERVER_STATES[2],
        SERVER_STATES[3],
        SERVER_STATES[4],
        SERVER_STATES[5],
        SERVER_STATES[6],
        SERVER_STATES[7],
        SERVERS_EMPTY,
        SERVERS_PREV,
        SERVERS_PAGE,
        SERVERS_NEXT,
        SERVERS_STATUS,
        SERVERS_JOIN,
        SERVERS_ADD,
        SERVERS_EDIT,
        SERVERS_REMOVE,
        SERVERS_REFRESH,
        SERVERS_QUIT,
        SERVER_EDIT,
        SERVER_EDIT_TITLE,
        SERVER_EDIT_CLOSE,
        SERVER_EDIT_NAME,
        SERVER_EDIT_ADDRESS,
        SERVER_EDIT_STATUS,
        SERVER_EDIT_SAVE,
        SERVER_EDIT_CANCEL,
        SERVER_QUESTION,
        SERVER_QUESTION_LINES[0],
        SERVER_QUESTION_LINES[1],
        SERVER_QUESTION_LINES[2],
        SERVER_QUESTION_YES,
        SERVER_QUESTION_NO,
        LOGIN_BACK,
    ];
}

/// Point numbers (`docs/specs/protocol/openm2/entities.md`, "Points").
const LEVEL: u8 = 1;
/// Our experience in the level, and what the next level needs (0 at the
/// highest level).
const EXPERIENCE: (u8, u8) = (3, 4);
const HEALTH: (u8, u8) = (5, 6);
const MANA: (u8, u8) = (7, 8);

/// A tooltip's first line, and the others.
const TOOLTIP_TITLE: Colour = [255, 225, 150, 255];
const TOOLTIP_DETAIL: Colour = [220, 220, 220, 255];

/// The taskbar's animations: the globe's frames (the looks `globe_1` and
/// on) and the health and mana fills' (`health_flow_1`, `mana_flow_1` and
/// on), each shown for `FRAME_MS`.
const GLOBE_FRAMES: u64 = 17;
const FLOW_FRAMES: u64 = 7;
const FRAME_MS: u64 = 100;

/// The built-in skin: openm2's looks, naming the client's art (game paths,
/// `.sub` files), which `assets import` converts into an asset folder's
/// skin.
pub(crate) fn built_in_skin() -> Result<Skins> {
    Skins::parse(SKIN).context("the built-in skin")
}

/// The built-in skin without its art: fallback colours only.
pub(crate) fn bare_skin() -> Result<Skins> {
    let mut skins = built_in_skin()?;
    for skin in skins.skins.values_mut() {
        for image in skin.images_mut() {
            image.clear();
        }
    }
    Ok(skins)
}

/// `skins` with the built-in skin's looks it lacks, in their fallback
/// colours: an asset folder imported before a look was added still draws
/// every window (the look's art comes with the next import). How many of
/// those have art; a look without any (a flat colour) is the same in every
/// import and is added without counting.
pub(crate) fn with_missing_looks(skins: &mut Skins) -> Result<usize> {
    let mut added = 0;
    for (name, mut look) in built_in_skin()?.skins {
        if let std::collections::btree_map::Entry::Vacant(entry) = skins.skins.entry(name) {
            let art = look.images().iter().any(|image| !image.is_empty());
            for image in look.images_mut() {
                image.clear();
            }
            entry.insert(look);
            added += usize::from(art);
        }
    }
    Ok(added)
}

/// An asset folder's skin file at `path` as the interface's skin, its
/// images resolved to paths of the folder (`file.png`, or
/// `file.png#xywh=X,Y,W,H` for a region); an image left out is an empty
/// name, which draws the fallback.
pub(crate) fn skin_of(file: &SkinFile, path: &str) -> Result<Skins> {
    let image = |reference: Option<&String>| -> Result<String> {
        match reference {
            Some(reference) => Ok(ImageRef::resolve(path, reference)
                .map_err(anyhow::Error::msg)?
                .to_text()),
            None => Ok(String::new()),
        }
    };
    let colour = |c: Option<[u8; 4]>| c.unwrap_or([0, 0, 0, 0]);
    let mut skins = BTreeMap::new();
    for (name, look) in &file.looks {
        let fallback = colour(look.fallback);
        let skin = match look.kind.as_str() {
            "colour" => Skin::Colour(colour(look.colour)),
            "frame" => {
                let c = look.corners.clone().unwrap_or_default();
                let e = look.edges.clone().unwrap_or_default();
                Skin::Frame {
                    corners: [
                        image(c.top_left.as_ref())?,
                        image(c.top_right.as_ref())?,
                        image(c.bottom_left.as_ref())?,
                        image(c.bottom_right.as_ref())?,
                    ],
                    edges: [
                        image(e.top.as_ref())?,
                        image(e.bottom.as_ref())?,
                        image(e.left.as_ref())?,
                        image(e.right.as_ref())?,
                    ],
                    fill: look.fill.as_ref().map(|f| image(Some(f))).transpose()?,
                    fallback,
                }
            }
            "bar" => Skin::Bar {
                left: image(look.left.as_ref())?,
                middle: image(look.middle.as_ref())?,
                right: image(look.right.as_ref())?,
                fallback,
            },
            "image" => Skin::Image {
                image: image(look.image.as_ref())?,
                fallback,
            },
            "tile" => Skin::Tile {
                image: image(look.image.as_ref())?,
                fallback,
            },
            "button" => Skin::Button {
                normal: image(look.normal.as_ref())?,
                hover: image(look.hover.as_ref())?,
                down: image(look.down.as_ref())?,
                text_colour: colour(look.text_colour),
                fallback,
            },
            "gauge" => {
                let [left, top, right, bottom] = look.inset.unwrap_or_default().map(|v| v as f32);
                Skin::Gauge {
                    slot: look.slot.clone(),
                    fill: image(look.fill.as_ref())?,
                    inset: (left, top, right, bottom),
                    fallback,
                }
            }
            other => bail!("look `{name}`: kind `{other}` is not one openm2 draws"),
        };
        skins.insert(name.clone(), skin);
    }
    Ok(Skins {
        font: m2_ui::Font {
            families: file.font.families.clone(),
            size: file.font.size as f32,
        },
        text_colour: file.text_colour,
        tooltip: file.tooltip.clone(),
        skins,
    })
}

/// An interface with the skin `skins`, the system's fonts and these
/// layouts.
pub(crate) fn new_ui(skins: Skins, layouts: &[&str]) -> Result<Ui> {
    let fonts = Fonts::system(&skins.font.families);
    match fonts.family() {
        Some(family) => eprintln!("interface font: {family}"),
        None if fonts.faces() == 0 => eprintln!("note: no fonts found; the interface has no text"),
        None => {
            eprintln!("note: none of the skin's fonts is installed; using the system's sans-serif")
        }
    }
    let mut ui = Ui::new(skins, fonts);
    for layout in layouts {
        ui.add(&Layout::parse(layout)?)?;
    }
    Ok(ui)
}

/// How the interface is scaled and its art drawn (`play`, `view ui`).
#[derive(Debug, Clone, Copy, Args)]
pub(crate) struct ScaleArgs {
    /// Interface scale: screen pixels per pixel of the layouts, 0.5 to 4.
    /// In a window the display's scale factor by default (125%, 250%, ...
    /// in Windows' display settings); 1 for pictures
    #[arg(long, value_name = "FACTOR", value_parser = parse_scale)]
    ui_scale: Option<f32>,
    /// How the interface's art looks when scaled: smooth (blended) or sharp
    /// (whole pixels, even only at whole scales); text is sharp either way
    #[arg(long, value_enum, default_value_t = ArtFilter::Smooth)]
    ui_filter: ArtFilter,
}

/// `--ui-filter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ArtFilter {
    Smooth,
    Sharp,
}

impl ScaleArgs {
    /// The scale for a window on a display with this scale factor, or for a
    /// picture (`None`): `--ui-scale` if given.
    pub fn scale(&self, display: Option<f64>) -> f32 {
        self.ui_scale
            .unwrap_or_else(|| display.map_or(1.0, |factor| factor as f32))
    }

    /// True if the scale follows the display's (no `--ui-scale`).
    pub fn follows_display(&self) -> bool {
        self.ui_scale.is_none()
    }

    /// How images are sampled.
    pub fn filter(&self) -> UiFilter {
        match self.ui_filter {
            ArtFilter::Smooth => UiFilter::Smooth,
            ArtFilter::Sharp => UiFilter::Sharp,
        }
    }
}

/// Reads `--ui-scale`.
fn parse_scale(text: &str) -> Result<f32, String> {
    let scale: f32 = text
        .parse()
        .map_err(|_| format!("`{text}` is not a number"))?;
    if (0.5..=4.0).contains(&scale) {
        Ok(scale)
    } else {
        Err("the interface scale goes from 0.5 to 4".to_owned())
    }
}

/// Lines a wheel's pixel delta stands for, at the interface's scale.
pub(crate) fn wheel_lines(pixels: f64, scale: f32) -> f32 {
    pixels as f32 / (40.0 * scale)
}

/// An image file on the GPU: its number and size.
type Uploaded = Option<(ImageId, u32, u32)>;

/// The interface's images on the GPU and the pipeline that draws it.
pub(crate) struct GpuInterface {
    renderer: UiRenderer,
    /// Image files by path, uploaded once (`None`: unreadable).
    images: HashMap<Vec<u8>, Uploaded>,
    /// Image names that could not be loaded; not tried again.
    failed: BTreeSet<String>,
}

impl GpuInterface {
    /// The pipeline for targets of `format`, with the art the interface names
    /// read from `files` (none without: fallback colours), sampled with
    /// `filter`.
    pub fn new(
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        ui: &mut Ui,
        files: Option<&Files<'_>>,
        filter: UiFilter,
    ) -> Result<Self> {
        let mut interface = Self {
            renderer: UiRenderer::new(gpu, format, filter)?,
            images: HashMap::new(),
            failed: BTreeSet::new(),
        };
        if let Some(files) = files {
            interface.load(gpu, ui, files, ui.image_paths())?;
        }
        Ok(interface)
    }

    /// Loads the images the interface names that it has no sprite for yet
    /// (new item icons), each once.
    pub fn load_missing(&mut self, gpu: &Gpu, ui: &mut Ui, files: &Files<'_>) -> Result<()> {
        let mut missing = ui.missing_images();
        missing.retain(|path| !self.failed.contains(path));
        if missing.is_empty() {
            return Ok(());
        }
        self.load(gpu, ui, files, missing)
    }

    fn load(
        &mut self,
        gpu: &Gpu,
        ui: &mut Ui,
        files: &Files<'_>,
        paths: BTreeSet<String>,
    ) -> Result<()> {
        let notes = load_art(gpu, &mut self.renderer, &mut self.images, ui, files, &paths)?;
        for note in &notes {
            eprintln!("note: interface: {note}");
        }
        self.failed.extend(paths);
        // Those that got a sprite are not failures.
        let still_missing = ui.missing_images();
        self.failed.retain(|path| still_missing.contains(path));
        Ok(())
    }

    /// Shows `view`, a picture the renderer draws, as image `id`.
    pub fn set_picture(&mut self, gpu: &Gpu, id: ImageId, view: &wgpu::TextureView) {
        self.renderer.set_picture(gpu, id, view);
    }

    /// Uploads `image`, a picture drawn on the CPU, as image `id` (again
    /// over the same texture when it has the same size).
    pub fn set_image(&mut self, gpu: &Gpu, id: ImageId, image: &RgbaImage) -> Result<()> {
        self.renderer.set_image(gpu, id, image)?;
        Ok(())
    }

    /// Draws the interface over `view`, a `width` × `height` target, after
    /// loading new images it names from `files`.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        ui: &mut Ui,
        files: Option<&Files<'_>>,
        view: &wgpu::TextureView,
        (width, height): (u32, u32),
    ) -> Result<()> {
        if let Some(files) = files {
            self.load_missing(gpu, ui, files)?;
        }
        let (list, pages) = ui.draw(width as f32, height as f32);
        self.renderer
            .draw(gpu, view, (width, height), list, pages)?;
        Ok(())
    }
}

/// Where an image the interface names is: its image file and, for a part
/// of it, the part's left, top, right and bottom edges. An asset folder's
/// images are paths of PNG files, with `#xywh=X,Y,W,H` for a region.
fn locate(name: &str) -> Result<(Vec<u8>, Option<[u32; 4]>)> {
    let image = ImageRef::parse(name).map_err(anyhow::Error::msg)?;
    let part = image
        .region
        .map(|[x, y, w, h]| [x, y, x.saturating_add(w), y.saturating_add(h)]);
    Ok((image.file.into_bytes(), part))
}

/// Loads images the interface names ([`locate`]); each image file is
/// uploaded once (`images` remembers them). Returns notes about what is
/// missing; an empty name (an item without an icon, a look without that
/// part) is skipped quietly.
fn load_art(
    gpu: &Gpu,
    renderer: &mut UiRenderer,
    images: &mut HashMap<Vec<u8>, Uploaded>,
    ui: &mut Ui,
    files: &Files<'_>,
    paths: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    for path in paths {
        if path.is_empty() {
            continue;
        }
        let (image_path, part) = match locate(path) {
            Ok(located) => located,
            Err(error) => {
                notes.push(format!("{path}: {error:#}"));
                continue;
            }
        };
        let next = ImageId(images.len() as u32);
        let uploaded = match images.get(&image_path) {
            Some(known) => *known,
            None => {
                let loaded = match read_image(files, &image_path) {
                    Ok(image) => {
                        renderer.set_image(gpu, next, &image)?;
                        Some((next, image.width(), image.height()))
                    }
                    Err(error) => {
                        notes.push(format!("{}: {error:#}", image_path.escape_ascii()));
                        None
                    }
                };
                images.insert(image_path, loaded);
                loaded
            }
        };
        let Some((image, w, h)) = uploaded else {
            continue;
        };
        // A rectangle past the image's edge is cut to it.
        let (left, top, right, bottom) = match part {
            Some([left, top, right, bottom]) => {
                (left.min(w), top.min(h), right.min(w), bottom.min(h))
            }
            None => (0, 0, w, h),
        };
        if right <= left || bottom <= top {
            notes.push(format!(
                "{path}: the rectangle is outside its {w}×{h} image"
            ));
            continue;
        }
        let (fw, fh) = (w as f32, h as f32);
        ui.set_sprite(
            path,
            Sprite {
                image,
                uv: Rect::new(
                    left as f32 / fw,
                    top as f32 / fh,
                    (right - left) as f32 / fw,
                    (bottom - top) as f32 / fh,
                ),
                size: ((right - left) as f32, (bottom - top) as f32),
            },
        );
    }
    Ok(notes)
}

/// An image the interface names as pixels ([`locate`]): the part cut out
/// of its image, or the image whole. For pictures drawn on the CPU (the
/// minimap's marks).
pub(crate) fn read_art(files: &Files<'_>, name: &str) -> Result<RgbaImage> {
    let (path, part) = locate(name)?;
    let image = read_image(files, &path)?;
    let Some([left, top, right, bottom]) = part else {
        return Ok(image);
    };
    image
        .crop(
            left,
            top,
            right.saturating_sub(left),
            bottom.saturating_sub(top),
        )
        .with_context(|| {
            format!(
                "the rectangle is outside its {}×{} image",
                image.width(),
                image.height()
            )
        })
}

/// An image file of an asset folder: a PNG file.
pub(crate) fn read_image(files: &Files<'_>, path: &[u8]) -> Result<RgbaImage> {
    let bytes = files.read_bytes(path)?;
    Ok(decode_any(&bytes)
        .with_context(|| format!("reading {}", path.escape_ascii()))?
        .0)
}

/// The interface key for a window key, if it is one the interface uses.
pub(crate) fn ui_key(key: &winit::keyboard::Key) -> Option<m2_ui::Key> {
    use winit::keyboard::{Key, NamedKey};
    let Key::Named(named) = key else {
        return None;
    };
    Some(match named {
        NamedKey::Enter => m2_ui::Key::Enter,
        NamedKey::Escape => m2_ui::Key::Escape,
        NamedKey::Backspace => m2_ui::Key::Backspace,
        NamedKey::Delete => m2_ui::Key::Delete,
        NamedKey::ArrowLeft => m2_ui::Key::Left,
        NamedKey::ArrowRight => m2_ui::Key::Right,
        NamedKey::Home => m2_ui::Key::Home,
        NamedKey::End => m2_ui::Key::End,
        NamedKey::Tab => m2_ui::Key::Tab,
        _ => return None,
    })
}

/// Text from the server for the screen: as it is, a tab as a space, other
/// control characters as `?`.
pub(crate) fn printable(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '\t' => ' ',
            c if c.is_control() => '?',
            c => c,
        })
        .collect()
}

/// Bytes for the screen that may not be text (a map's path): printable
/// ASCII as it is, anything else `?`.
pub(crate) fn readable(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| match b {
            b' '..=b'~' => char::from(*b),
            b'\t' => ' ',
            _ => '?',
        })
        .collect()
}

/// The characters of bytes 0x80 to 0x9F in Windows codepage 1252; the
/// others above 0x9F are Latin-1's. `?` where the codepage has none.
const CP1252_HIGH: [char; 32] = [
    '€', '?', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '?', 'Ž', '?', '?', '‘', '’',
    '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '?', 'ž', 'Ÿ',
];

/// A name of an asset folder's tables (items, monsters, skills) for the
/// screen: UTF-8, control characters as `?`.
pub(crate) fn text(bytes: &[u8]) -> String {
    printable(&String::from_utf8_lossy(bytes))
}

/// Text from the client's files (item names) in their locale's codepage,
/// for the import: codepage 1252 is decoded; others are shown like bytes
/// that may not be text ([`readable`]).
pub(crate) fn decode(bytes: &[u8], codepage: u32) -> String {
    if codepage != 1252 {
        return readable(bytes);
    }
    bytes
        .iter()
        .map(|b| match b {
            b' '..=b'~' => char::from(*b),
            0x80..=0x9F => CP1252_HIGH
                .get(usize::from(b - 0x80))
                .copied()
                .unwrap_or('?'),
            0xA0..=0xFF => char::from(*b),
            _ => '?',
        })
        .collect()
}

/// The colour of a chat line by its kind.
pub(crate) fn chat_colour(kind: ChatKind) -> Colour {
    match kind {
        ChatKind::Talk | ChatKind::Other(_) => [232, 228, 214, 255],
        ChatKind::Info => [255, 230, 150, 255],
        ChatKind::Notice | ChatKind::BigNotice => [255, 200, 80, 255],
        ChatKind::Party => [130, 220, 255, 255],
        ChatKind::Guild => [180, 255, 150, 255],
        ChatKind::Command => [200, 200, 200, 255],
        ChatKind::Shout => [255, 160, 120, 255],
    }
}

/// Writes the game's values into the in-game layout.
#[derive(Default)]
pub(crate) struct Hud {
    /// The newest chat line already in the log.
    last_chat: Option<ChatLine>,
    /// The asset folder's monster names.
    pub monsters: Option<Monsters>,
    /// The time, for the taskbar's animations.
    pub now: Millis,
}

impl Hud {
    /// The chat lines that arrived since the last call.
    pub fn update(&mut self, ui: &mut Ui, world: &World) {
        let new_from = self
            .last_chat
            .as_ref()
            .and_then(|last| world.chat.iter().rposition(|line| line == last))
            .map_or(0, |at| at + 1);
        for line in world.chat.iter().skip(new_from) {
            ui.push_line(
                ids::CHAT_LOG,
                &printable(&line.text),
                chat_colour(line.kind),
            );
        }
        if let Some(last) = world.chat.back() {
            self.last_chat = Some(last.clone());
        }
    }

    /// The taskbar: the globe glowing, our health and mana as flowing
    /// gauges, our experience in the level as four orbs, a quarter of the
    /// level each (empty until the server says, full at the highest level),
    /// and over each a tooltip with the numbers. After the windows that set
    /// the tooltip for their slots.
    pub fn update_taskbar(&self, ui: &mut Ui, world: &World) {
        let frame = self.now.0 / FRAME_MS;
        ui.set_look(ids::GLOBE, &format!("globe_{}", frame % GLOBE_FRAMES + 1));
        let flow = frame % FLOW_FRAMES + 1;
        let point = |number| world.points.get(&number).copied().unwrap_or(0);
        let mut tooltip = None;
        for ((now, max), gauge, look, name) in [
            (HEALTH, ids::HEALTH, "health_flow", "Health"),
            (MANA, ids::MANA, "mana_flow", "Mana"),
        ] {
            let (now, max) = (point(now), point(max));
            ui.set_value(
                gauge,
                if max > 0 {
                    now as f32 / max as f32
                } else {
                    0.0
                },
            );
            ui.set_look(gauge, &format!("{look}_{flow}"));
            if ui.is_hovered(gauge) {
                let line = format!("{name}: {} / {}", grouped(now), grouped(max));
                tooltip = Some(vec![(line, TOOLTIP_TITLE)]);
            }
        }
        let (level, now, next) = (point(LEVEL), point(EXPERIENCE.0), point(EXPERIENCE.1));
        let filled = match (level, next) {
            (0, _) => 0.0,
            (_, 0) => 1.0,
            _ => (now as f32 / next as f32).min(1.0),
        };
        for (at, orb) in ids::EXPERIENCE_ORBS.into_iter().enumerate() {
            ui.set_value(orb, filled * 4.0 - at as f32);
        }
        if level > 0 && ui.is_hovered(ids::EXPERIENCE) {
            tooltip = Some(experience_lines(level, now, next));
        }
        ui.set_visible(ids::STAMINA, world.mount.is_some());
        if let Some(mount) = world.mount {
            let filled = if mount.most > 0 {
                f32::from(mount.stamina) / f32::from(mount.most)
            } else {
                0.0
            };
            ui.set_value(ids::STAMINA, filled);
            if ui.is_hovered(ids::STAMINA) {
                let line = format!("Mount's stamina: {} / {}", mount.stamina, mount.most);
                tooltip = Some(vec![(line, TOOLTIP_TITLE)]);
            }
        }
        if let Some(lines) = tooltip {
            ui.set_tooltip(lines);
        }
    }

    /// The target window (the chosen entity's name and health; shown while
    /// there is one) and the restart window (shown while our character is
    /// dead).
    pub fn update_fight(&self, ui: &mut Ui, world: &World) {
        let target = world.target.and_then(|vid| world.entities.get(&vid));
        ui.set_visible(ids::TARGET, target.is_some());
        if let Some(entity) = target {
            ui.set_text(ids::TARGET_NAME, &self.name_of(entity));
            // Full until the server says otherwise.
            let percent = if entity.dead {
                0
            } else {
                entity.health.unwrap_or(100).min(100)
            };
            ui.set_value(ids::TARGET_HEALTH, f32::from(percent) / 100.0);
        }
        let dead = world.my_entity().is_some_and(|me| me.dead);
        ui.set_visible(ids::RESTART, dead);
    }

    /// An entity's name: the server's (players, NPCs), else the asset
    /// folder's monster names', else its kind and number.
    pub fn name_of(&self, entity: &Entity) -> String {
        if let Some(info) = &entity.info {
            return printable(&info.name);
        }
        self.monsters
            .as_ref()
            .and_then(|monsters| {
                let info = monsters.get(u32::from(entity.race))?;
                Some(text(info.name.as_bytes()))
            })
            .unwrap_or_else(|| format!("{:?} {}", entity.kind, entity.race))
    }
}

/// The experience's tooltip: our level, how much of what it needs we have,
/// and what is left for the next level; or that it is the highest.
fn experience_lines(level: u32, now: u32, next: u32) -> Vec<(String, Colour)> {
    let title = (format!("Level {level}"), TOOLTIP_TITLE);
    if next == 0 {
        return vec![title, ("Highest level".to_owned(), TOOLTIP_DETAIL)];
    }
    let percent = u64::from(now) * 100 / u64::from(next);
    vec![
        title,
        (
            format!(
                "Experience: {} / {} ({percent}%)",
                grouped(now),
                grouped(next)
            ),
            TOOLTIP_DETAIL,
        ),
        (
            format!(
                "{} more for level {}",
                grouped(next.saturating_sub(now)),
                level.saturating_add(1)
            ),
            TOOLTIP_DETAIL,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_core::Empire;

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        ui.add(&Layout::parse(GAME).unwrap()).unwrap();
        ui
    }

    #[test]
    fn the_in_game_layout_starts_with_the_chat_line_closed() {
        let ui = ui();
        assert!(ui.has(ids::CHAT_INPUT));
        assert!(
            !ui.is_visible(ids::CHAT_BAR),
            "the chat line opens with Enter"
        );
    }

    fn line(text: &str, kind: ChatKind) -> ChatLine {
        ChatLine {
            at: Millis(0),
            kind,
            vid: 0,
            empire: Empire::Shinsoo,
            text: text.to_owned(),
            mine: false,
        }
    }

    #[test]
    fn the_hud_shows_new_chat_lines_once() {
        let mut ui = ui();
        let mut world = World::default();
        world.chat.push_back(line("hello", ChatKind::Talk));
        let mut hud = Hud::default();
        hud.update(&mut ui, &world);
        hud.update(&mut ui, &world);
        world.chat.push_back(line("café \x1b[31m", ChatKind::Info));
        hud.update(&mut ui, &world);
        assert_eq!(ui.lines(ids::CHAT_LOG), ["hello", "café ?[31m"]);
    }

    #[test]
    fn the_taskbar_shows_health_mana_and_experience_and_their_numbers_over_them() {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        ui.add(&Layout::parse(INVENTORY).unwrap()).unwrap();
        let orbs = |ui: &Ui| ids::EXPERIENCE_ORBS.map(|orb| ui.value(orb).unwrap());
        let lines = |ui: &Ui| -> Vec<String> {
            ui.tooltip().iter().map(|(text, _)| text.clone()).collect()
        };
        let mut world = World::default();
        let mut hud = Hud::default();
        hud.update_taskbar(&mut ui, &world);
        assert_eq!(orbs(&ui), [0.0; 4], "empty until the server says");
        assert_eq!(ui.value(ids::HEALTH), Some(0.0));
        // Level 12 with 1,875 of 3,000: two orbs and a half.
        world.points.extend([
            (1, 12),
            (3, 1875),
            (4, 3000),
            (5, 4062),
            (6, 8124),
            (7, 0),
            (8, 0),
        ]);
        hud.update_taskbar(&mut ui, &world);
        assert_eq!(orbs(&ui), [1.0, 1.0, 0.5, 0.0]);
        assert_eq!(ui.value(ids::HEALTH), Some(0.5));
        assert_eq!(ui.value(ids::MANA), Some(0.0), "no mana at all");
        assert!(ui.tooltip().is_empty(), "nothing under the pointer");
        // The frames go round with the time.
        assert_eq!(ui.look(ids::GLOBE), Some("globe_1"));
        hud.now = Millis(1_650);
        hud.update_taskbar(&mut ui, &world);
        assert_eq!(ui.look(ids::GLOBE), Some("globe_17"));
        assert_eq!(ui.look(ids::HEALTH), Some("health_flow_3"));
        assert_eq!(ui.look(ids::MANA), Some("mana_flow_3"));
        hud.now = Millis(1_700);
        hud.update_taskbar(&mut ui, &world);
        assert_eq!(ui.look(ids::GLOBE), Some("globe_1"));
        // Over the gauges and the orbs, their numbers.
        ui.layout(1280.0, 800.0);
        let over = |ui: &mut Ui, id| {
            let rect = ui.rect(id).unwrap();
            ui.pointer_moved(rect.x + 10.0, rect.y + 5.0);
        };
        over(&mut ui, ids::HEALTH);
        hud.update_taskbar(&mut ui, &world);
        assert_eq!(lines(&ui), ["Health: 4,062 / 8,124"]);
        over(&mut ui, ids::MANA);
        hud.update_taskbar(&mut ui, &world);
        assert_eq!(lines(&ui), ["Mana: 0 / 0"]);
        over(&mut ui, ids::EXPERIENCE);
        hud.update_taskbar(&mut ui, &world);
        assert_eq!(
            lines(&ui),
            [
                "Level 12",
                "Experience: 1,875 / 3,000 (62%)",
                "1,125 more for level 13"
            ]
        );
        // The highest level: the server says 0 and 0.
        world.points.extend([(1, 99), (3, 0), (4, 0)]);
        hud.update_taskbar(&mut ui, &world);
        assert_eq!(orbs(&ui), [1.0; 4]);
        assert_eq!(lines(&ui), ["Level 99", "Highest level"]);
    }

    #[test]
    fn the_target_window_names_the_target_and_shows_its_health() {
        use m2_core::{EntityKind, Me, MoveKind, Point, Track};
        use m2_formats::{MobRecord, RawStr};
        let mut ui = ui();
        let entity = |vid: u32, kind: EntityKind, race: u16| Entity {
            vid,
            kind,
            race,
            track: Track::still(Point::new(0, 0), Millis(0)),
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
        let mut world = World {
            me: Some(Me {
                vid: 1,
                name: "Alice".to_owned(),
                class: 3,
                empire: Empire::Shinsoo,
                position: (0, 0, 0),
            }),
            ..World::default()
        };
        world.entities.insert(1, entity(1, EntityKind::Player, 3));
        world
            .entities
            .insert(9, entity(9, EntityKind::Monster, 101));
        let hud = Hud::default();
        hud.update_fight(&mut ui, &world);
        assert!(!ui.is_visible(ids::TARGET) && !ui.is_visible(ids::RESTART));
        world.target = Some(9);
        hud.update_fight(&mut ui, &world);
        assert!(ui.is_visible(ids::TARGET));
        assert_eq!(ui.text(ids::TARGET_NAME), Some("Monster 101"));
        // The asset folder's name for it; the server's health.
        let dog = MobRecord {
            vnum: 101,
            locale_name: RawStr::from("Wilder Hund ü".as_bytes()),
            ..MobRecord::default()
        };
        let hud = Hud {
            monsters: Some(Monsters::new(vec![dog])),
            ..Hud::default()
        };
        world.entities.get_mut(&9).unwrap().health = Some(40);
        hud.update_fight(&mut ui, &world);
        assert_eq!(ui.text(ids::TARGET_NAME), Some("Wilder Hund ü"));
        assert_eq!(ui.value(ids::TARGET_HEALTH), Some(0.4));
        world.entities.get_mut(&9).unwrap().dead = true;
        world.entities.get_mut(&1).unwrap().dead = true;
        hud.update_fight(&mut ui, &world);
        assert_eq!(ui.value(ids::TARGET_HEALTH), Some(0.0));
        assert!(ui.is_visible(ids::RESTART));
        world.target = None;
        hud.update_fight(&mut ui, &world);
        assert!(!ui.is_visible(ids::TARGET));
    }

    #[test]
    fn server_text_keeps_its_letters_but_no_control_characters() {
        assert_eq!(printable("héllo\tworld\u{1b}[2J"), "héllo world?[2J");
        assert_eq!(readable(b"a\tb\x00\xe9"), "a b??");
    }

    #[test]
    fn the_clients_names_are_decoded_in_codepage_1252() {
        assert_eq!(decode(b"Schwert\xfc \x80 \x81\x00", 1252), "Schwertü € ??");
        assert_eq!(decode(b"Schwert\xfc", 949), "Schwert?");
    }

    #[test]
    fn an_asset_folders_names_are_utf8() {
        assert_eq!(text("Schwert€\t".as_bytes()), "Schwert€ ");
        assert_eq!(text(b"Bad\xff\x01"), "Bad\u{fffd}?");
    }
}
