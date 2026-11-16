//! Where the client keeps the models of characters and monsters
//! (`docs/specs/formats/script.md`, "Monster list" and "Player characters").
//!
//! - Player characters: one model script per class and gender at the root
//!   (`warrior_m.msm`, `shaman_w.msm`, …); their motions lie in `general/`
//!   beside the base model the script names, or in a weapon's folder
//!   (`onehand_sword/`, `bow/`, …) while they hold one.
//! - What a character wears shows: the body armour picks a shape of the
//!   script, a weapon's model is held at a hand bone
//!   (`docs/specs/formats/script.md`, "Worn items").
//! - Monsters and NPCs: `npclist.txt` names a model per monster number; its
//!   script is `<name>.msm` in one of the monster folders, or `shape.msm` in
//!   a folder the list gives as a path.
//! - Fighting: players attack with their weapon folder's combo
//!   (`combo_01.msa`, …) or single attacks (`attack.msa`, …), flinch with
//!   `damage.msa` and fall with `general/dead.msa`; monsters with
//!   `NORMAL_ATTACK`, `FRONT_DAMAGE` and `FRONT_DEAD` from their motion list
//!   (`docs/specs/formats/script.md`, "Attack motions").

use m2_formats::gr2::Gr2;
use m2_formats::gr2::model::Gr2File;
use m2_formats::model::Skeleton;
use m2_formats::script::motion::{self, MotionData, MotionListEntry};
use m2_formats::script::npc_list::{self, NpcLine, NpcModel};
use m2_formats::script::race::RaceData;
use m2_formats::{ItemRecord, RawStr, normalize_name};

use crate::error::AssetError;
use crate::map::read_parsed;
use crate::vfs::Vfs;

/// The monster list.
pub const NPC_LIST: &[u8] = b"npclist.txt";

/// The folders a named monster model is looked for in, in this order.
pub const MONSTER_FOLDERS: [&[u8]; 4] = [
    b"d:/ymir work/monster/",
    b"d:/ymir work/monster2/",
    b"d:/ymir work/npc/",
    b"d:/ymir work/npc2/",
];

/// Reads the client's monster list.
pub fn read_npc_list(vfs: &Vfs) -> Result<Vec<NpcLine>, AssetError> {
    read_parsed(vfs, NPC_LIST, npc_list::parse)
}

/// The game path of monster `vnum`'s model script, if the list names the
/// monster and the script is in the client.
pub fn monster_script(vfs: &Vfs, lines: &[NpcLine], vnum: u32) -> Option<Vec<u8>> {
    let candidates: Vec<Vec<u8>> = match npc_list::find(lines, vnum)? {
        NpcModel::Path(path) => {
            let mut folder = path.to_vec();
            if !folder.ends_with(b"/") {
                folder.push(b'/');
            }
            vec![[folder.as_slice(), b"shape.msm"].concat()]
        }
        NpcModel::Named { name, folder } => MONSTER_FOLDERS
            .iter()
            .map(|root| [*root, folder, b"/", name, b".msm"].concat())
            .collect(),
    };
    candidates
        .into_iter()
        .map(|path| normalize_name(&path))
        .find(|path| vfs.source(path).is_some())
}

/// The model script of a player character of `race` (class and gender, as
/// the server sends it: 0 warrior, 1 ninja, 2 sura, 3 shaman, 4 to 7 the
/// same classes of the other gender).
pub fn player_script(race: u16) -> Option<&'static [u8]> {
    Some(match race {
        0 => b"warrior_m.msm",
        1 => b"assassin_w.msm",
        2 => b"sura_m.msm",
        3 => b"shaman_w.msm",
        4 => b"warrior_w.msm",
        5 => b"assassin_m.msm",
        6 => b"sura_w.msm",
        7 => b"shaman_m.msm",
        _ => return None,
    })
}

/// The motion folder of a character without a weapon in hand.
pub const GENERAL_MOTIONS: &[u8] = b"general";

/// The motion folder of a character riding a mount: it stands, runs,
/// flinches and falls with these, a weapon or not ([`Grip::riding`] has a
/// weapon's attacks).
pub const RIDING_MOTIONS: &[u8] = b"horse";

/// The bone of a mount's skeleton its rider sits at: where it is, not how
/// it turns (`docs/specs/render.md`, "Riding").
pub const SEAT_BONE: &str = "saddle";

/// The game path of a player motion script by name (`wait`, `run`, …): in
/// motion folder `motions` (`general`, or a weapon's: [`Grip::motions`])
/// beside the character's base model.
pub fn player_motion(base_model: &[u8], motions: &[u8], name: &[u8]) -> Vec<u8> {
    let folder_end = base_model
        .iter()
        .rposition(|b| matches!(b, b'/' | b'\\'))
        .map_or(0, |at| at + 1);
    let folder = base_model.get(..folder_end).unwrap_or_default();
    normalize_name(&[folder, motions, b"/", name, b".msa"].concat())
}

/// Most steps a weapon's combo has (`combo_01.msa` to `combo_08.msa`: the
/// ninja's daggers have 8, the other weapons 7).
pub const MAX_COMBO: u8 = 8;

/// The name of combo step `step` (from 1) in a weapon's motion folder:
/// `combo_01`, `combo_02`, …
pub fn combo_motion(step: u8) -> Vec<u8> {
    format!("combo_{step:02}").into_bytes()
}

/// Players' single attacks, in the weapon's motion folder or `general`.
pub const PLAYER_ATTACKS: [&[u8]; 3] = [b"attack", b"attack_1", b"attack_2"];

/// Players flinching when hit, in the weapon's motion folder or `general`.
pub const PLAYER_DAMAGES: [&[u8]; 4] = [b"damage", b"damage_1", b"damage_2", b"damage_3"];

/// Players falling dead, in `general`.
pub const PLAYER_DEAD: &[u8] = b"dead";

/// Monsters' attack in their motion list, with its numbered variants.
pub const MONSTER_ATTACK: &str = "NORMAL_ATTACK";

/// Monsters flinching when hit: the first name their motion list has (with
/// its numbered variants).
pub const MONSTER_DAMAGE: [&str; 2] = ["FRONT_DAMAGE", "DAMAGE"];

/// Monsters falling dead: the first name their motion list has.
pub const MONSTER_DEAD: [&str; 2] = ["FRONT_DEAD", "DEAD"];

/// Item type 1 in the item table: a weapon (Quantum Core X's item types).
pub const WEAPON: u8 = 1;

/// Wear flag 1: worn on the body, an armour (`inventory.md`).
pub const WEAR_BODY: u32 = 1;

/// The shape a body armour gives the character wearing it: the
/// `ShapeIndex` of the character's model script, which is the item's fourth
/// value. `None` for items not worn on the body.
pub fn body_shape(record: &ItemRecord) -> Option<u32> {
    if record.wear_flags & WEAR_BODY == 0 {
        return None;
    }
    u32::try_from(*record.values.get(3)?).ok()
}

/// The hands a weapon is held in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hands {
    /// The right hand.
    Right,
    /// The left hand (bows).
    Left,
    /// One in each hand (daggers).
    Both,
}

/// How a character holds a weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Grip {
    /// The motion folder it stands and runs with (`onehand_sword`).
    pub motions: &'static [u8],
    /// The motion folder it attacks with riding a mount
    /// (`horse_onehand_sword`).
    pub riding: &'static [u8],
    /// Where the weapon's model goes.
    pub hands: Hands,
}

/// The client's item kind of fishing poles.
pub const ROD: u8 = 13;

/// The motion folder of a character holding a fishing pole:
/// its standing, walking and running, and the fishing motions.
pub const FISHING_MOTIONS: &[u8] = b"fishing";

/// How a weapon is held, by its subtype (Quantum Core X's weapon types):
/// 0 sword, 1 dagger, 2 bow, 3 two-handed, 4 bell, 5 fan, each with the
/// client's motion folder of that name, and `horse_` and that name riding.
/// `None` for items that are not weapons, and for arrows and the rest,
/// which no one holds.
pub fn weapon_grip(record: &ItemRecord) -> Option<Grip> {
    // A fishing pole, worn as a weapon: in the right hand, with the
    // fishing motions on foot and the plain riding ones on a mount.
    if record.kind == ROD {
        return Some(Grip {
            motions: FISHING_MOTIONS,
            riding: RIDING_MOTIONS,
            hands: Hands::Right,
        });
    }
    if record.kind != WEAPON {
        return None;
    }
    let (motions, riding, hands): (&'static [u8], &'static [u8], Hands) = match record.subtype {
        0 => (b"onehand_sword", b"horse_onehand_sword", Hands::Right),
        1 => (b"dualhand_sword", b"horse_dualhand_sword", Hands::Both),
        2 => (b"bow", b"horse_bow", Hands::Left),
        3 => (b"twohand_sword", b"horse_twohand_sword", Hands::Right),
        4 => (b"bell", b"horse_bell", Hands::Right),
        5 => (b"fan", b"horse_fan", Hands::Right),
        _ => return None,
    };
    Some(Grip {
        motions,
        riding,
        hands,
    })
}

/// The bones of a character's skeleton that hold a weapon in `hands`, right
/// first: the first bone whose name starts with `equip_right` or
/// `equip_left` (warriors have `equip_right_hand`, the other classes
/// `equip_right` and `equip_left`). Hands the skeleton has no bone for are
/// left out.
pub fn hand_bones(skeleton: &Skeleton, hands: Hands) -> Vec<RawStr> {
    let find = |prefix: &[u8]| {
        skeleton
            .bones
            .iter()
            .find(|bone| {
                bone.name
                    .as_bytes()
                    .get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            })
            .map(|bone| bone.name.clone())
    };
    let sides: &[&[u8]] = match hands {
        Hands::Right => &[b"equip_right"],
        Hands::Left => &[b"equip_left"],
        Hands::Both => &[b"equip_right", b"equip_left"],
    };
    sides.iter().filter_map(|side| find(side)).collect()
}

/// A monster's motion list when its model script names none.
pub const MOTION_LIST: &[u8] = b"motlist.txt";

/// The motions a moving monster plays, by name in its motion list: the first
/// one the list has.
pub const MONSTER_RUN: [&str; 2] = ["RUN", "WALK"];

/// The game path of a monster's motion list: `list` (its model script's
/// `MotionListFileName`), else [`MOTION_LIST`], beside the script.
pub fn motion_list_path(script: &[u8], list: Option<&RawStr>) -> Vec<u8> {
    let folder_end = script
        .iter()
        .rposition(|b| matches!(b, b'/' | b'\\'))
        .map_or(0, |at| at + 1);
    let folder = script.get(..folder_end).unwrap_or_default();
    let name = list.map_or(MOTION_LIST, RawStr::as_bytes);
    [folder, name].concat()
}

/// The script of the first of `names` (case ignored) that the motion list
/// at `list_path` holds.
pub fn find_motion(list: &[MotionListEntry], list_path: &[u8], names: &[&str]) -> Option<Vec<u8>> {
    names.iter().find_map(|name| {
        list.iter()
            .find(|entry| entry.name.as_bytes().eq_ignore_ascii_case(name.as_bytes()))
            .map(|entry| entry.script_path(list_path))
    })
}

/// The motion script a moving monster `vnum` plays ([`MONSTER_RUN`]), if
/// the client has its model and motion list.
pub fn monster_run_motion(vfs: &Vfs, lines: &[NpcLine], vnum: u32) -> Option<Vec<u8>> {
    let script = monster_script(vfs, lines, vnum)?;
    let race = read_parsed(vfs, &script, RaceData::parse).ok()?;
    let list_path = motion_list_path(&script, race.motion_list.as_ref());
    let list = read_parsed(vfs, &list_path, motion::parse_list).ok()?;
    find_motion(&list, &list_path, &MONSTER_RUN)
}

/// The run motion script of player race `race` without a weapon in hand:
/// `general/run.msa` beside its base model.
pub fn player_run_motion(vfs: &Vfs, race: u16) -> Option<Vec<u8>> {
    let race = read_parsed(vfs, player_script(race)?, RaceData::parse).ok()?;
    let base = race.base_model?;
    Some(player_motion(base.as_bytes(), GENERAL_MOTIONS, b"run"))
}

/// How fast the motion script at `script` moves its actor forward, in
/// centimetres a second ([`MotionData::forward_speed`]); its GR2 animation
/// is read only when the script gives no `MotionDuration`. `Ok(None)` for a
/// motion that moves nothing.
pub fn motion_speed(vfs: &Vfs, script: &[u8]) -> Result<Option<f32>, AssetError> {
    let data = read_parsed(vfs, script, MotionData::parse)?;
    if data.accumulation.is_none() {
        return Ok(None);
    }
    let animation = match data.duration {
        Some(_) => None,
        None => {
            let gr2_path = data.motion_file.as_bytes();
            let file = read_parsed(vfs, gr2_path, |bytes| Gr2File::read(&Gr2::parse(bytes)?))?;
            file.animations.first().map(|animation| animation.duration)
        }
    };
    Ok(data.forward_speed(animation))
}
