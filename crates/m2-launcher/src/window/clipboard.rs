//! The system's clipboard in the window's fields: Ctrl+V or
//! Shift+Insert pastes at the caret, Ctrl+C or Ctrl+Insert copies the
//! field's whole text (⌘ in place of Ctrl on macOS). The same as `openm2
//! play`'s (`m2-cli`'s `clipboard.rs`), which the launcher can't depend on.

use m2_ui::Ui;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};

/// What a key does with the clipboard while a field has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shortcut {
    /// The clipboard's text into the field.
    Paste,
    /// The field's text into the clipboard.
    Copy,
}

impl Shortcut {
    /// The shortcut a key pressed with these modifiers is, if it is one.
    /// Not with Alt: AltGr, which some layouts type letters with, is
    /// Ctrl+Alt on Windows.
    pub(crate) fn of(logical: &Key, physical: PhysicalKey, held: ModifiersState) -> Option<Self> {
        let command = if cfg!(target_os = "macos") {
            held.super_key()
        } else {
            held.control_key()
        };
        if held.alt_key() {
            return None;
        }
        if *logical == Key::Named(NamedKey::Insert) {
            return match (command, held.shift_key()) {
                (false, true) => Some(Self::Paste),
                (true, false) => Some(Self::Copy),
                _ => None,
            };
        }
        if !command {
            return None;
        }
        match letter(logical, physical)? {
            'v' => Some(Self::Paste),
            'c' => Some(Self::Copy),
            _ => None,
        }
    }
}

/// The letter a key stands for: what it types on a Latin layout, else the
/// letter at its place on a US keyboard, so Ctrl+V works where the layout
/// types another script.
fn letter(logical: &Key, physical: PhysicalKey) -> Option<char> {
    if let Key::Character(text) = logical {
        let mut chars = text.chars();
        if let (Some(c), None) = (chars.next(), chars.next())
            && c.is_ascii_alphabetic()
        {
            return Some(c.to_ascii_lowercase());
        }
    }
    match physical {
        PhysicalKey::Code(KeyCode::KeyV) => Some('v'),
        PhysicalKey::Code(KeyCode::KeyC) => Some('c'),
        _ => None,
    }
}

/// The system's clipboard, opened at its first use and kept open: on X11,
/// what was copied lasts only while it is.
#[derive(Default)]
pub(crate) struct Clipboard(Option<arboard::Clipboard>);

impl Clipboard {
    /// Does a shortcut on the field with the keyboard. A note if the
    /// clipboard failed; its text is never in it.
    pub(crate) fn apply(&mut self, shortcut: Shortcut, ui: &mut Ui) -> Option<String> {
        match shortcut {
            Shortcut::Paste => match self.open().and_then(|c| c.get_text()) {
                Ok(text) => {
                    ui.paste(&text);
                    None
                }
                // Empty, or not text.
                Err(arboard::Error::ContentNotAvailable) => None,
                Err(error) => Some(format!("pasting from the clipboard: {error}")),
            },
            Shortcut::Copy => {
                let text = ui.copy().filter(|text| !text.is_empty())?;
                self.open()
                    .and_then(|c| c.set_text(text))
                    .err()
                    .map(|error| format!("copying to the clipboard: {error}"))
            }
        }
    }

    fn open(&mut self) -> Result<&mut arboard::Clipboard, arboard::Error> {
        let clipboard = match self.0.take() {
            Some(clipboard) => clipboard,
            None => arboard::Clipboard::new()?,
        };
        Ok(self.0.insert(clipboard))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn character(c: &str) -> Key {
        Key::Character(c.into())
    }

    fn with(ctrl: bool, shift: bool, alt: bool) -> ModifiersState {
        let mut held = ModifiersState::empty();
        if ctrl {
            held |= if cfg!(target_os = "macos") {
                ModifiersState::SUPER
            } else {
                ModifiersState::CONTROL
            };
        }
        if shift {
            held |= ModifiersState::SHIFT;
        }
        if alt {
            held |= ModifiersState::ALT;
        }
        held
    }

    #[test]
    fn ctrl_v_and_shift_insert_paste_ctrl_c_and_ctrl_insert_copy() {
        let v = PhysicalKey::Code(KeyCode::KeyV);
        let c = PhysicalKey::Code(KeyCode::KeyC);
        let insert = PhysicalKey::Code(KeyCode::Insert);
        let of = Shortcut::of;
        assert_eq!(
            of(&character("v"), v, with(true, false, false)),
            Some(Shortcut::Paste)
        );
        assert_eq!(
            of(&character("V"), v, with(true, true, false)),
            Some(Shortcut::Paste)
        );
        assert_eq!(
            of(&character("c"), c, with(true, false, false)),
            Some(Shortcut::Copy)
        );
        let named = Key::Named(NamedKey::Insert);
        assert_eq!(
            of(&named, insert, with(false, true, false)),
            Some(Shortcut::Paste)
        );
        assert_eq!(
            of(&named, insert, with(true, false, false)),
            Some(Shortcut::Copy)
        );
        assert_eq!(of(&named, insert, ModifiersState::empty()), None);
        assert_eq!(
            of(&character("v"), v, ModifiersState::empty()),
            None,
            "typing v"
        );
        assert_eq!(
            of(&character("v"), v, with(true, false, true)),
            None,
            "AltGr"
        );
        assert_eq!(
            of(
                &character("x"),
                PhysicalKey::Code(KeyCode::KeyX),
                with(true, false, false)
            ),
            None
        );
    }

    #[test]
    fn the_letter_typed_counts_and_the_place_where_it_isnt_latin() {
        let ctrl = with(true, false, false);
        // Dvorak: the key at V's place types K.
        assert_eq!(
            Shortcut::of(&character("k"), PhysicalKey::Code(KeyCode::KeyV), ctrl),
            None
        );
        assert_eq!(
            Shortcut::of(&character("v"), PhysicalKey::Code(KeyCode::Period), ctrl),
            Some(Shortcut::Paste)
        );
        // Russian: the key at V's place types м.
        assert_eq!(
            Shortcut::of(&character("м"), PhysicalKey::Code(KeyCode::KeyV), ctrl),
            Some(Shortcut::Paste)
        );
        assert_eq!(
            Shortcut::of(&character("с"), PhysicalKey::Code(KeyCode::KeyC), ctrl),
            Some(Shortcut::Copy)
        );
    }
}
