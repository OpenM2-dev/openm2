//! The glows of upgraded items in `openm2 play`: a body
//! armour or a weapon worn upgraded far enough glows on its wearer, round
//! and round, with the effects the asset folder's `[[effects.upgraded]]`
//! names (`docs/specs/render.md`, "Effects").

use m2_assets::actor::{WEAPON, WEAR_BODY};
use m2_assets::items::Items;
use m2_formats::RawStr;

use crate::world_effects::Loop;

/// The bone a body armour's glow plays on, turning with it: the pelvis,
/// as the client's own scripts attach it.
pub(crate) const ARMOUR_BONE: &str = "Bip01";

/// The weapon subtype of two-handed weapons (`items.toml`).
const TWO_HANDED: u8 = 3;

/// True if item `weapon` (0 for none) is a two-handed weapon: a skill's
/// look may differ with one, as the original client's
/// two-handed motions tell it.
pub(crate) fn two_handed(items: &Items, weapon: u32) -> bool {
    (weapon != 0)
        .then(|| items.get(weapon))
        .flatten()
        .is_some_and(|info| info.record.kind == WEAPON && info.record.subtype == TWO_HANDED)
}

/// A glow of upgraded items, its files as paths of the asset folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct UpgradeGlow {
    /// The lowest upgrade level it shows from: 7 for a +7.
    pub level: u8,
    /// On a body armour.
    pub armour: Option<Vec<u8>>,
    /// On a weapon, by its subtype: 0 sword, 1 dagger, 2 bow, 3
    /// two-handed, 4 bell, 5 fan.
    pub weapons: [Option<Vec<u8>>; 6],
}

/// The glows a player wearing `body` and `weapon` (item numbers, 0 for
/// none) shows of `glows` (lowest level first): a body armour's on its
/// pelvis, a weapon's on each bone of `held` (the hands that hold it,
/// both for daggers), each turning with its bone, as the client attaches
/// them. Each item shows the glow of the highest level not above its own
/// (`ItemInfo::glow`) that has one for it.
pub(crate) fn worn(
    glows: &[UpgradeGlow],
    items: &Items,
    body: u32,
    weapon: u32,
    held: &[RawStr],
) -> Vec<Loop> {
    let mut loops = Vec::new();
    if glows.is_empty() {
        return loops;
    }
    let pick = |level: u8, file: &dyn Fn(&UpgradeGlow) -> Option<&Vec<u8>>| {
        glows
            .iter()
            .rev()
            .filter(|glow| glow.level <= level)
            .find_map(file)
            .cloned()
    };
    let item = |vnum: u32| (vnum != 0).then(|| items.get(vnum)).flatten();
    if let Some(info) = item(body)
        && info.record.wear_flags & WEAR_BODY != 0
        && let Some(path) = pick(info.glow, &|glow| glow.armour.as_ref())
    {
        loops.push(Loop {
            path,
            bone: Some(RawStr::from(ARMOUR_BONE)),
            turns: true,
        });
    }
    if let Some(info) = item(weapon)
        && info.record.kind == WEAPON
    {
        let kind = usize::from(info.record.subtype);
        if let Some(path) = pick(info.glow, &|glow| glow.weapons.get(kind)?.as_ref()) {
            for bone in held {
                loops.push(Loop {
                    path: path.clone(),
                    bone: Some(bone.clone()),
                    turns: true,
                });
            }
        }
    }
    loops
}

#[cfg(test)]
mod tests {
    use m2_assets::items::{ItemInfo, client_glow};
    use m2_formats::ItemRecord;

    use super::*;

    /// Made-up items, by number, kind, subtype, wear flags and glow level:
    /// swords of levels 6 to 9 (106 to 109), daggers of 9 (209), body
    /// armours of 6, 7 and 9 (306, 307, 309).
    fn items() -> Items {
        let item = |vnum: u32, kind: u8, subtype: u8, wear_flags: u32, glow: u8| ItemInfo {
            record: ItemRecord {
                vnum,
                kind,
                subtype,
                wear_flags,
                ..ItemRecord::default()
            },
            icon: None,
            model: None,
            glow,
        };
        let mut infos: Vec<ItemInfo> = (6..=9u8)
            .map(|n| item(100 + u32::from(n), 1, 0, 16, n))
            .collect();
        infos.push(item(209, 1, 1, 16, 9));
        infos.push(item(306, 2, 0, 1, 6));
        infos.push(item(307, 2, 0, 1, 7));
        infos.push(item(309, 2, 0, 1, 9));
        Items::from_infos(infos, None)
    }

    fn file(name: &str) -> Option<Vec<u8>> {
        Some(name.as_bytes().to_vec())
    }

    /// Glows from 7 and 9 (none for 8), the 9's without a dagger's.
    fn glows() -> Vec<UpgradeGlow> {
        vec![
            UpgradeGlow {
                level: 7,
                armour: file("armour7"),
                weapons: [file("long7"), file("short7"), None, None, None, None],
            },
            UpgradeGlow {
                level: 9,
                armour: file("armour9"),
                weapons: [file("long9"), None, None, None, None, None],
            },
        ]
    }

    fn paths(loops: &[Loop]) -> Vec<(String, String, bool)> {
        loops
            .iter()
            .map(|l| {
                (
                    String::from_utf8_lossy(&l.path).into_owned(),
                    l.bone
                        .as_ref()
                        .map_or(String::new(), |b| b.as_bytes().escape_ascii().to_string()),
                    l.turns,
                )
            })
            .collect()
    }

    #[test]
    fn the_clients_glow_level_counts_the_items_sockets() {
        let record = |vnum: u32, kind: u8, subtype: u8, socket_percent: u8| ItemRecord {
            vnum,
            kind,
            subtype,
            socket_percent,
            ..ItemRecord::default()
        };
        // A basic sword (1 socket) glows only at +9, and as a +7.
        assert_eq!(client_glow(&record(17, 1, 0, 1)), 5);
        assert_eq!(client_glow(&record(19, 1, 0, 1)), 7);
        // One of 3 sockets as its upgrade; of none, never past 6.
        assert_eq!(client_glow(&record(297, 1, 0, 3)), 7);
        assert_eq!(client_glow(&record(299, 1, 0, 0)), 6);
        assert_eq!(client_glow(&record(10, 1, 0, 0)), 0);
        // A body armour; a helmet (armour subtype 1) and a potion never.
        assert_eq!(client_glow(&record(11209, 2, 0, 1)), 7);
        assert_eq!(client_glow(&record(12209, 2, 1, 3)), 0);
        assert_eq!(client_glow(&record(27009, 3, 0, 3)), 0);
    }

    #[test]
    fn an_item_shows_the_glow_of_the_highest_level_it_reaches() {
        let (items, glows) = (items(), glows());
        let hand = [RawStr::from("equip_right")];
        assert!(worn(&glows, &items, 306, 106, &hand).is_empty());
        assert_eq!(
            paths(&worn(&glows, &items, 307, 107, &hand)),
            [
                ("armour7".to_owned(), "Bip01".to_owned(), true),
                ("long7".to_owned(), "equip_right".to_owned(), true),
            ]
        );
        // 8 has no glow of its own: 7's shows.
        assert_eq!(
            paths(&worn(&glows, &items, 0, 108, &hand)),
            [("long7".to_owned(), "equip_right".to_owned(), true)]
        );
        assert_eq!(
            paths(&worn(&glows, &items, 309, 0, &hand)),
            [("armour9".to_owned(), "Bip01".to_owned(), true)]
        );
    }

    #[test]
    fn daggers_glow_in_both_hands_with_their_own_kind() {
        let (items, glows) = (items(), glows());
        let hands = [RawStr::from("equip_right"), RawStr::from("equip_left")];
        // 9 has no dagger's glow: 7's shows, on each.
        assert_eq!(
            paths(&worn(&glows, &items, 0, 209, &hands)),
            [
                ("short7".to_owned(), "equip_right".to_owned(), true),
                ("short7".to_owned(), "equip_left".to_owned(), true),
            ]
        );
    }

    #[test]
    fn nothing_glows_without_glows_or_out_of_place() {
        let items = items();
        let hand = [RawStr::from("equip_right")];
        assert!(worn(&[], &items, 309, 109, &hand).is_empty());
        // A weapon number as the body, an armour as the weapon.
        assert!(worn(&glows(), &items, 109, 309, &hand).is_empty());
    }
}
