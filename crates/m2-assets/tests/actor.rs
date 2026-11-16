//! Where the models of monsters and player characters are, in a synthetic
//! client.

mod support;

use m2_assets::Vfs;
use m2_assets::actor::{
    GENERAL_MOTIONS, Grip, Hands, MAX_COMBO, MONSTER_FOLDERS, body_shape, combo_motion, hand_bones,
    monster_script, player_motion, player_script, read_npc_list, weapon_grip,
};
use m2_formats::model::{Bone, Skeleton, Transform};
use m2_formats::script::npc_list::{self, NpcLine};
use m2_formats::{EntryKind, ItemRecord, RawStr};
use support::{TempDir, test_keys, write_archive};

fn line(vnum: u32, name: &str, folder: Option<&str>) -> NpcLine {
    NpcLine {
        vnum,
        name: RawStr::from(name),
        folder: folder.map(RawStr::from),
    }
}

#[test]
fn monsters_lead_to_their_model_scripts() {
    let tmp = TempDir::new("actor");
    let list = npc_list::write(&[
        line(101, "stray_dog", None),
        line(104, "wolf_blue", None),
        line(0, "wolf_blue", Some("wolf")),
        line(9001, "arms", None),
        line(693, "#season1/monster/orc_lord_e/", None),
        line(2292, "red_dragon", None),
    ])
    .unwrap();
    let script = b"ScriptType RaceDataScript\r\n".as_slice();
    let files: Vec<(&str, &[u8], EntryKind)> = vec![
        ("npclist.txt", &list, EntryKind::Lzo),
        (
            "d:/ymir work/monster/stray_dog/stray_dog.msm",
            script,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster2/wolf/wolf_blue.msm",
            script,
            EntryKind::Lzo,
        ),
        ("D:\\Ymir Work\\npc\\arms\\arms.msm", script, EntryKind::Lzo),
        (
            "season1/monster/orc_lord_e/shape.msm",
            script,
            EntryKind::Lzo,
        ),
    ];
    write_archive(&tmp.path().join("pack"), "monsters", &files, true);
    let vfs = Vfs::open_client(tmp.path(), &test_keys()).unwrap();
    let lines = read_npc_list(&vfs).unwrap();
    let found = |vnum| monster_script(&vfs, &lines, vnum).map(|p| String::from_utf8(p).unwrap());

    assert_eq!(
        found(101).as_deref(),
        Some("d:/ymir work/monster/stray_dog/stray_dog.msm")
    );
    // The folder line, and the next monster folder.
    assert_eq!(
        found(104).as_deref(),
        Some("d:/ymir work/monster2/wolf/wolf_blue.msm")
    );
    assert_eq!(
        found(9001).as_deref(),
        Some("d:/ymir work/npc/arms/arms.msm")
    );
    assert_eq!(
        found(693).as_deref(),
        Some("season1/monster/orc_lord_e/shape.msm")
    );
    // Listed but not shipped; not listed at all.
    assert_eq!(found(2292), None);
    assert_eq!(found(5), None);
    assert_eq!(MONSTER_FOLDERS.len(), 4);
}

#[test]
fn a_client_without_a_monster_list_says_so() {
    let tmp = TempDir::new("actor-none");
    let files: Vec<(&str, &[u8], EntryKind)> = vec![("other.txt", b"x", EntryKind::Lzo)];
    write_archive(&tmp.path().join("pack"), "none", &files, true);
    let vfs = Vfs::open_client(tmp.path(), &test_keys()).unwrap();
    assert!(read_npc_list(&vfs).is_err());
}

#[test]
fn player_classes_have_a_script_and_motions_beside_their_body() {
    let names: Vec<&[u8]> = (0..8).map(|race| player_script(race).unwrap()).collect();
    assert_eq!(
        names,
        [
            b"warrior_m.msm".as_slice(),
            b"assassin_w.msm",
            b"sura_m.msm",
            b"shaman_w.msm",
            b"warrior_w.msm",
            b"assassin_m.msm",
            b"sura_w.msm",
            b"shaman_m.msm",
        ]
    );
    assert_eq!(player_script(8), None);
    assert_eq!(
        player_motion(
            b"d:/ymir work/pc2/warrior/warrior_novice.GR2",
            GENERAL_MOTIONS,
            b"run"
        ),
        b"d:/ymir work/pc2/warrior/general/run.msa"
    );
    assert_eq!(
        player_motion(
            b"D:\\Ymir Work\\pc\\sura\\sura_novice.gr2",
            b"onehand_sword",
            b"wait"
        ),
        b"d:/ymir work/pc/sura/onehand_sword/wait.msa"
    );
    // A weapon's combo, step by step.
    assert_eq!(
        player_motion(
            b"d:/ymir work/pc/warrior/warrior_novice.gr2",
            b"onehand_sword",
            &combo_motion(1)
        ),
        b"d:/ymir work/pc/warrior/onehand_sword/combo_01.msa"
    );
    assert_eq!(combo_motion(MAX_COMBO), b"combo_08");
}

fn item(kind: u8, subtype: u8, wear_flags: u32, values: [i32; 6]) -> ItemRecord {
    ItemRecord {
        kind,
        subtype,
        wear_flags,
        values,
        ..ItemRecord::default()
    }
}

#[test]
fn worn_items_give_a_shape_and_a_grip() {
    // An armour's fourth value is its shape; a helmet changes no shape.
    assert_eq!(body_shape(&item(2, 0, 1, [0, 90, 0, 12, 0, 27])), Some(12));
    assert_eq!(body_shape(&item(2, 1, 2, [0, 13, 0, 0, 0, 33])), None);
    assert_eq!(body_shape(&item(2, 0, 1, [0, 0, 0, -1, 0, 0])), None);
    let grip = |subtype| weapon_grip(&item(1, subtype, 16, [0; 6]));
    assert_eq!(
        grip(0),
        Some(Grip {
            motions: b"onehand_sword",
            riding: b"horse_onehand_sword",
            hands: Hands::Right
        })
    );
    assert_eq!(grip(2).unwrap().riding, b"horse_bow");
    assert_eq!(grip(1).unwrap().hands, Hands::Both);
    assert_eq!(grip(2).unwrap().hands, Hands::Left);
    assert_eq!(grip(3).unwrap().motions, b"twohand_sword");
    assert_eq!(grip(5).unwrap().motions, b"fan");
    assert_eq!(grip(6), None, "arrows are not held");
    assert_eq!(weapon_grip(&item(2, 0, 1, [0; 6])), None);
    // A fishing pole (kind 13): in the right hand with the fishing motions,
    // the plain riding ones on a mount.
    assert_eq!(
        weapon_grip(&item(13, 0, 16, [10, 5, 10, 100, 27400, 0])),
        Some(Grip {
            motions: b"fishing",
            riding: b"horse",
            hands: Hands::Right
        })
    );
}

#[test]
fn weapons_go_to_the_equip_bones() {
    let bone = |name: &str| Bone {
        name: RawStr::from(name),
        parent: None,
        local: Transform::default(),
        inverse_world: [0.0; 16],
    };
    let warrior = Skeleton {
        name: RawStr::from("Bip01"),
        bones: vec![
            bone("Bip01"),
            bone("Bip01 R Hand"),
            bone("equip_right_hand"),
        ],
    };
    let names =
        |bones: Vec<RawStr>| -> Vec<String> { bones.iter().map(ToString::to_string).collect() };
    assert_eq!(
        names(hand_bones(&warrior, Hands::Right)),
        ["equip_right_hand"]
    );
    assert!(hand_bones(&warrior, Hands::Left).is_empty());
    let assassin = Skeleton {
        name: RawStr::from("Bip01"),
        bones: vec![bone("Equip_Left"), bone("equip_right")],
    };
    assert_eq!(
        names(hand_bones(&assassin, Hands::Both)),
        ["equip_right", "Equip_Left"]
    );
}
