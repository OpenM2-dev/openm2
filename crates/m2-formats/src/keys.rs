//! The XTEA keys a client uses.
//!
//! They are part of the user's client, so the user supplies them
//! (`keys.toml`, loaded by `m2-assets`, format in
//! `docs/specs/formats/keys.md`). The repo only holds our own test keys.

use std::fmt;

use crate::error::FormatError;
use crate::xtea::XteaKey;

/// Which key a structure needs, named as in `keys.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyName {
    /// `eterpack.index`: encrypted archive indexes (`.eix`).
    EterpackIndex,
    /// `eterpack.entries`: archive entries of type 2 (LZO + XTEA).
    EterpackEntries,
    /// `proto.item`: `item_proto`.
    ItemProto,
    /// `proto.mob`: `mob_proto`.
    MobProto,
}

impl KeyName {
    /// Every key, in `keys.toml` order.
    pub const ALL: [KeyName; 4] = [
        KeyName::EterpackIndex,
        KeyName::EterpackEntries,
        KeyName::ItemProto,
        KeyName::MobProto,
    ];

    /// The dotted `keys.toml` name, e.g. `eterpack.index`.
    pub fn as_str(self) -> &'static str {
        match self {
            KeyName::EterpackIndex => "eterpack.index",
            KeyName::EterpackEntries => "eterpack.entries",
            KeyName::ItemProto => "proto.item",
            KeyName::MobProto => "proto.mob",
        }
    }
}

impl fmt::Display for KeyName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The keys the user supplied. Any of them may be missing; a structure that
/// needs a missing key fails with [`FormatError::MissingKey`].
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Keys {
    /// `eterpack.index`
    pub eterpack_index: Option<XteaKey>,
    /// `eterpack.entries`
    pub eterpack_entries: Option<XteaKey>,
    /// `proto.item`
    pub item_proto: Option<XteaKey>,
    /// `proto.mob`
    pub mob_proto: Option<XteaKey>,
}

impl Keys {
    /// No keys: only unencrypted data can be read.
    pub fn none() -> Self {
        Self::default()
    }

    /// The key slot for `name`.
    pub fn get(&self, name: KeyName) -> Option<&XteaKey> {
        match name {
            KeyName::EterpackIndex => self.eterpack_index.as_ref(),
            KeyName::EterpackEntries => self.eterpack_entries.as_ref(),
            KeyName::ItemProto => self.item_proto.as_ref(),
            KeyName::MobProto => self.mob_proto.as_ref(),
        }
    }

    /// Sets the key slot for `name`.
    pub fn set(&mut self, name: KeyName, key: XteaKey) {
        let slot = match name {
            KeyName::EterpackIndex => &mut self.eterpack_index,
            KeyName::EterpackEntries => &mut self.eterpack_entries,
            KeyName::ItemProto => &mut self.item_proto,
            KeyName::MobProto => &mut self.mob_proto,
        };
        *slot = Some(key);
    }

    /// The key for `name`, or the error that tells the user to add it.
    pub fn require(&self, name: KeyName, what: &'static str) -> Result<&XteaKey, FormatError> {
        self.get(name)
            .ok_or(FormatError::MissingKey { what, key: name })
    }
}

/// Shows which keys are set, never the keys themselves.
impl fmt::Debug for Keys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut set = f.debug_set();
        for name in KeyName::ALL {
            if self.get(name).is_some() {
                set.entry(&name.as_str());
            }
        }
        set.finish()
    }
}
