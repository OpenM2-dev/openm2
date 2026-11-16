//! The game menu and the Settings window of `openm2 play` (`docs/specs/ui.md`,
//! "The game menu"): Escape in the game opens the menu, which changes
//! character, logs out, quits or opens the Settings window. What the
//! Settings window shows applies at once, and `play` keeps it in the
//! player's settings file (`settings.rs`).
//!
//! - Escape or the taskbar's gears open it; Escape, the X, the gears or
//!   Back to the game close it. Escape, the X or the gears close the
//!   Settings window too, and its Back goes back to the menu.
//! - The volumes are sliders (whole percents); the interface's size goes a
//!   tenth down or up with − and +; Mute holds down while on.

use m2_core::Command;
use m2_ui::Ui;

use crate::interface::ids;
use crate::screens::Action;
use crate::settings::{SIZES, Settings, percent, step};

/// The menu and the settings it shows.
pub(crate) struct GameMenu {
    settings: Settings,
}

impl GameMenu {
    pub fn new() -> Self {
        Self {
            settings: Settings::default(),
        }
    }

    /// Shows `settings` (the player's file, at the start).
    pub fn set_settings(&mut self, ui: &mut Ui, settings: Settings) {
        self.settings = settings;
        self.fill(ui);
    }

    /// True while the menu or the Settings window is open.
    pub fn is_open(&self, ui: &Ui) -> bool {
        ui.is_visible(ids::MENU) || ui.is_visible(ids::SETTINGS)
    }

    /// Opens the menu, or closes it (and the Settings window) when open.
    pub fn toggle(&mut self, ui: &mut Ui) {
        if self.is_open(ui) {
            self.close(ui);
        } else {
            ui.set_visible(ids::MENU, true);
            ui.raise(ids::MENU);
        }
    }

    /// Escape: closes the menu and the Settings window. False if neither
    /// was open.
    pub fn cancel(&mut self, ui: &mut Ui) -> bool {
        let open = self.is_open(ui);
        self.close(ui);
        open
    }

    /// Leaving the game closes them.
    pub fn reset(&mut self, ui: &mut Ui) {
        self.close(ui);
    }

    fn close(&mut self, ui: &mut Ui) {
        ui.set_visible(ids::MENU, false);
        ui.set_visible(ids::SETTINGS, false);
    }

    /// A click on one of the windows' buttons: what the player asked for,
    /// if anything (`Some(None)` for a click taken without one); `None` if
    /// it is not theirs.
    pub fn clicked(&mut self, ui: &mut Ui, id: &str) -> Option<Option<Action>> {
        let action = match id {
            ids::MENU_CLOSE | ids::MENU_BACK | ids::SETTINGS_CLOSE => {
                self.close(ui);
                None
            }
            ids::TASKBAR_SYSTEM => {
                self.toggle(ui);
                None
            }
            ids::MENU_SETTINGS => {
                ui.set_visible(ids::MENU, false);
                ui.set_visible(ids::SETTINGS, true);
                ui.raise(ids::SETTINGS);
                self.fill(ui);
                None
            }
            ids::SETTINGS_BACK => {
                ui.set_visible(ids::SETTINGS, false);
                ui.set_visible(ids::MENU, true);
                ui.raise(ids::MENU);
                None
            }
            ids::MENU_CHARACTERS => {
                self.close(ui);
                Some(Action::Command(Command::ChangeCharacter))
            }
            ids::MENU_LOGOUT => {
                self.close(ui);
                Some(Action::Back)
            }
            ids::MENU_QUIT => Some(Action::Quit),
            ids::SETTINGS_MUTE => self.change(ui, |s| s.mute = !s.mute),
            ids::SETTINGS_NAMES => self.change(ui, |s| s.names = !s.names),
            ids::SETTINGS_SIZE_DOWN | ids::SETTINGS_SIZE_UP => {
                let up = id == ids::SETTINGS_SIZE_UP;
                self.change(ui, |s| s.interface = step(s.interface, up, SIZES))
            }
            _ => return None,
        };
        Some(action)
    }

    /// A slider moved: its volume, in whole percents. `None` if it is not
    /// one of theirs.
    pub fn changed(&mut self, ui: &mut Ui, id: &str) -> Option<Action> {
        let value = (ui.value(id)? * 100.0).round() / 100.0;
        match id {
            ids::SETTINGS_MUSIC => self.change(ui, |s| s.music = value),
            ids::SETTINGS_SOUND => self.change(ui, |s| s.sounds = value),
            _ => None,
        }
    }

    /// Changes the settings: shown, and handed to `play` if they differ.
    fn change(&mut self, ui: &mut Ui, how: impl FnOnce(&mut Settings)) -> Option<Action> {
        let before = self.settings;
        how(&mut self.settings);
        self.fill(ui);
        (self.settings != before).then_some(Action::Settings(self.settings))
    }

    /// Writes the settings into the Settings window.
    fn fill(&self, ui: &mut Ui) {
        let s = self.settings;
        ui.set_value(ids::SETTINGS_MUSIC, s.music);
        ui.set_text(ids::SETTINGS_MUSIC_VALUE, &percent(s.music));
        ui.set_value(ids::SETTINGS_SOUND, s.sounds);
        ui.set_text(ids::SETTINGS_SOUND_VALUE, &percent(s.sounds));
        ui.set_text(ids::SETTINGS_MUTE, if s.mute { "On" } else { "Off" });
        ui.set_latched(ids::SETTINGS_MUTE, s.mute);
        ui.set_text(ids::SETTINGS_SIZE_VALUE, &percent(s.interface));
        ui.set_text(ids::SETTINGS_NAMES, if s.names { "On" } else { "Off" });
        ui.set_latched(ids::SETTINGS_NAMES, s.names);
    }
}
