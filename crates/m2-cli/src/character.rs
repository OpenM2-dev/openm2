//! The character window of `openm2 play` (`docs/specs/ui.md`, "The
//! character window"): our character, its four stats with the status points
//! to spend on them, and what they make, on the Character tab; the skills of
//! its group on the Skills tab (`skills.rs`). The server has the last word:
//! a point spent changes the stat when its answer comes.
//!
//! - C opens it on the Character tab and K on the Skills tab; the same key
//!   closes it again, the other switches tabs.
//! - The taskbar's character button opens it on the tab last shown, or
//!   closes it; so do its X and Escape.
//! - A + beside a stat spends a status point on it while there are points.

use m2_core::{Command, World};
use m2_ui::Ui;

use crate::interface::{ids, printable};
use crate::inventory::grouped;
use crate::screens::job_name;

/// The points the window shows (`entities.md`, "Points").
mod point {
    pub const LEVEL: u8 = 1;
    pub const EXP: u8 = 3;
    pub const NEXT_EXP: u8 = 4;
    pub const MAX_HP: u8 = 6;
    pub const MAX_SP: u8 = 8;
    /// Strength, vitality, dexterity, intelligence: the stats, in the
    /// window's order.
    pub const STATS: [u8; 4] = [12, 13, 14, 15];
    pub const DEFENCE: u8 = 16;
    pub const ATTACK_SPEED: u8 = 17;
    pub const ATTACK_GRADE: u8 = 18;
    pub const MOVE_SPEED: u8 = 19;
    pub const STAT_POINTS: u8 = 26;
}

/// The window's tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    /// The character and its stats.
    Status,
    /// The skills (`skills.rs`).
    Skills,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Status => "Character",
            Tab::Skills => "Skills",
        }
    }
}

/// The character window's state between frames.
pub(crate) struct CharacterWindow {
    /// The tab shown, or shown last.
    tab: Tab,
    /// Status points not spent yet, as last seen.
    points: u32,
}

impl CharacterWindow {
    pub fn new() -> Self {
        Self {
            tab: Tab::Status,
            points: 0,
        }
    }

    /// Shows `tab`'s page, its tab held down and its title.
    fn show_tab(&mut self, ui: &mut Ui, tab: Tab) {
        self.tab = tab;
        ui.set_visible(ids::CHARACTER_STATUS, tab == Tab::Status);
        ui.set_visible(ids::SKILLS, tab == Tab::Skills);
        ui.set_latched(ids::CHARACTER_TAB_STATUS, tab == Tab::Status);
        ui.set_latched(ids::CHARACTER_TAB_SKILLS, tab == Tab::Skills);
        ui.set_text(ids::CHARACTER_TITLE, tab.title());
    }

    /// Opens the window on `tab`, on top of the others.
    pub fn open(&mut self, ui: &mut Ui, tab: Tab) {
        self.show_tab(ui, tab);
        ui.set_visible(ids::CHARACTER, true);
        ui.raise(ids::CHARACTER);
    }

    /// C or K: opens the window on `tab`; closes it when it is open on
    /// that tab already.
    pub fn key(&mut self, ui: &mut Ui, tab: Tab) {
        if ui.is_visible(ids::CHARACTER) && self.tab == tab {
            ui.set_visible(ids::CHARACTER, false);
        } else {
            self.open(ui, tab);
        }
    }

    /// The taskbar's button: opens the window on the tab shown last, or
    /// closes it.
    pub fn toggle(&mut self, ui: &mut Ui) {
        if ui.is_visible(ids::CHARACTER) {
            ui.set_visible(ids::CHARACTER, false);
        } else {
            self.open(ui, self.tab);
        }
    }

    /// Escape: closes the window. False if it was closed.
    pub fn cancel(&mut self, ui: &mut Ui) -> bool {
        let open = ui.is_visible(ids::CHARACTER);
        ui.set_visible(ids::CHARACTER, false);
        open
    }

    /// Forgets the game (on leaving it): closed, on the Character tab.
    pub fn reset(&mut self, ui: &mut Ui) {
        self.points = 0;
        self.show_tab(ui, Tab::Status);
        ui.set_visible(ids::CHARACTER, false);
    }

    /// A button of the window or the taskbar's: the character button and
    /// the X open and close it, the tabs switch, a + spends a status point.
    /// True if it was one, with the command to send.
    pub fn clicked(&mut self, ui: &mut Ui, id: &str) -> Option<Option<Command>> {
        match id {
            ids::TASKBAR_CHARACTER => self.toggle(ui),
            ids::CHARACTER_CLOSE => {
                ui.set_visible(ids::CHARACTER, false);
            }
            ids::CHARACTER_TAB_STATUS => self.show_tab(ui, Tab::Status),
            ids::CHARACTER_TAB_SKILLS => self.show_tab(ui, Tab::Skills),
            _ => {
                let stat = ids::CHARACTER_STATS_UP.iter().position(|up| *up == id)?;
                let point = point::STATS.get(stat).copied()?;
                return Some((self.points > 0).then_some(Command::RaiseStatus { point }));
            }
        }
        Some(None)
    }

    /// Writes our character, its stats, the status points left and what
    /// the stats make into the Character tab.
    pub fn update(&mut self, ui: &mut Ui, world: &World) {
        let value = |point: u8| world.points.get(&point).copied().unwrap_or(0);
        self.points = value(point::STAT_POINTS);
        if let Some(me) = world.me.as_ref() {
            ui.set_text(ids::CHARACTER_NAME, &printable(&me.name));
            let level = value(point::LEVEL);
            ui.set_text(
                ids::CHARACTER_CLASS,
                &format!("{}, level {level}", job_name(me.class)),
            );
            let face = match me.class % 4 {
                0 => "face_warrior",
                1 => "face_ninja",
                2 => "face_sura",
                _ => "face_shaman",
            };
            ui.set_look(ids::CHARACTER_FACE, face);
        }
        ui.set_text(
            ids::CHARACTER_EXPERIENCE,
            &experience_text(value(point::EXP), value(point::NEXT_EXP)),
        );
        for ((id, up), stat) in ids::CHARACTER_STATS
            .iter()
            .zip(ids::CHARACTER_STATS_UP)
            .zip(point::STATS)
        {
            ui.set_text(id, &value(stat).to_string());
            ui.set_visible(up, self.points > 0);
        }
        ui.set_text(
            ids::CHARACTER_POINTS,
            &format!("Status points: {}", self.points),
        );
        let shown = [
            (ids::CHARACTER_HEALTH, point::MAX_HP),
            (ids::CHARACTER_MANA, point::MAX_SP),
            (ids::CHARACTER_ATTACK, point::ATTACK_GRADE),
            (ids::CHARACTER_DEFENCE, point::DEFENCE),
            (ids::CHARACTER_ATTACK_SPEED, point::ATTACK_SPEED),
            (ids::CHARACTER_MOVE_SPEED, point::MOVE_SPEED),
        ];
        for (id, point) in shown {
            ui.set_text(id, &grouped(value(point)));
        }
    }
}

/// The experience line: `Experience: 62%` of the next level (rounded down),
/// or `Highest level`, where the server sends 0 and 0 (`ui.md`,
/// "Experience").
fn experience_text(exp: u32, next: u32) -> String {
    if next == 0 {
        return "Highest level".to_owned();
    }
    let percent = (u64::from(exp) * 100 / u64::from(next)).min(100);
    format!("Experience: {percent}%")
}

#[cfg(test)]
mod tests {
    use m2_core::Me;
    use m2_ui::{Fonts, Layout, Skins};

    use super::*;
    use crate::interface::{CHARACTER, GAME, INVENTORY, SKIN};

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        for layout in [GAME, INVENTORY, CHARACTER] {
            ui.add(&Layout::parse(layout).unwrap()).unwrap();
        }
        ui.layout(1280.0, 800.0);
        ui
    }

    /// A ninja at level 12, 75% of the way, with `points` status points.
    fn world(points: u32) -> World {
        let mut world = World {
            me: Some(Me {
                vid: 1,
                name: "Admin".to_owned(),
                class: 5,
                empire: m2_core::Empire::Shinsoo,
                position: (0, 0, 0),
            }),
            ..World::default()
        };
        for (point, value) in [
            (1, 12),
            (3, 2_250),
            (4, 3_000),
            (6, 4_062),
            (8, 1_200),
            (12, 7),
            (13, 5),
            (14, 9),
            (15, 3),
            (16, 31),
            (17, 100),
            (18, 38),
            (19, 150),
            (26, points),
        ] {
            world.points.insert(point, value);
        }
        world
    }

    #[test]
    fn the_character_tab_shows_the_stats_and_what_they_make() {
        let mut ui = ui();
        let mut window = CharacterWindow::new();
        window.open(&mut ui, Tab::Status);
        window.update(&mut ui, &world(2));
        assert_eq!(ui.text(ids::CHARACTER_NAME), Some("Admin"));
        assert_eq!(ui.text(ids::CHARACTER_CLASS), Some("Ninja, level 12"));
        assert_eq!(ui.look(ids::CHARACTER_FACE), Some("face_ninja"));
        assert_eq!(ui.text(ids::CHARACTER_EXPERIENCE), Some("Experience: 75%"));
        let stats: Vec<_> = ids::CHARACTER_STATS.iter().map(|id| ui.text(id)).collect();
        assert_eq!(stats, [Some("7"), Some("5"), Some("9"), Some("3")]);
        assert_eq!(ui.text(ids::CHARACTER_POINTS), Some("Status points: 2"));
        assert_eq!(ui.text(ids::CHARACTER_HEALTH), Some("4,062"));
        assert_eq!(ui.text(ids::CHARACTER_ATTACK), Some("38"));
        assert_eq!(ui.text(ids::CHARACTER_MOVE_SPEED), Some("150"));
        assert!(ui.is_visible(ids::CHARACTER_IQ_UP));

        // A + spends a point on its stat; without points the +s go.
        assert_eq!(
            window.clicked(&mut ui, ids::CHARACTER_HT_UP),
            Some(Some(Command::RaiseStatus { point: 13 }))
        );
        window.update(&mut ui, &world(0));
        assert!(!ui.is_visible(ids::CHARACTER_HT_UP));
        assert_eq!(window.clicked(&mut ui, ids::CHARACTER_HT_UP), Some(None));
        assert_eq!(window.clicked(&mut ui, "inventory.close"), None);
    }

    #[test]
    fn keys_open_their_tab_and_the_taskbar_the_last_one() {
        let mut ui = ui();
        let mut window = CharacterWindow::new();
        let shown = |ui: &Ui| {
            (
                ui.is_visible(ids::CHARACTER),
                ui.is_visible(ids::CHARACTER_STATUS),
                ui.is_visible(ids::SKILLS),
                ui.text(ids::CHARACTER_TITLE).map(str::to_owned),
            )
        };
        let skills = (true, false, true, Some("Skills".to_owned()));
        let status = (true, true, false, Some("Character".to_owned()));
        // K: the Skills tab; C switches to the Character tab; C again closes.
        window.key(&mut ui, Tab::Skills);
        assert_eq!(shown(&ui), skills);
        window.key(&mut ui, Tab::Status);
        assert_eq!(shown(&ui), status);
        window.key(&mut ui, Tab::Status);
        assert!(!ui.is_visible(ids::CHARACTER));
        // The tabs switch pages; the taskbar's button opens the last tab.
        window.open(&mut ui, Tab::Status);
        assert_eq!(
            window.clicked(&mut ui, ids::CHARACTER_TAB_SKILLS),
            Some(None)
        );
        assert_eq!(shown(&ui), skills);
        assert_eq!(window.clicked(&mut ui, ids::CHARACTER_CLOSE), Some(None));
        assert!(!ui.is_visible(ids::CHARACTER));
        assert_eq!(window.clicked(&mut ui, ids::TASKBAR_CHARACTER), Some(None));
        assert_eq!(shown(&ui), skills);
        assert_eq!(window.clicked(&mut ui, ids::TASKBAR_CHARACTER), Some(None));
        assert!(!ui.is_visible(ids::CHARACTER));
        // Escape closes it once; leaving the game goes back to the first tab.
        window.open(&mut ui, Tab::Skills);
        assert!(window.cancel(&mut ui));
        assert!(!window.cancel(&mut ui));
        window.reset(&mut ui);
        assert!(!ui.is_visible(ids::CHARACTER));
        window.toggle(&mut ui);
        assert_eq!(shown(&ui), status);
    }

    #[test]
    fn experience_in_percent_of_the_level() {
        assert_eq!(experience_text(0, 300), "Experience: 0%");
        assert_eq!(experience_text(299, 300), "Experience: 99%");
        assert_eq!(experience_text(0, 0), "Highest level");
    }
}
