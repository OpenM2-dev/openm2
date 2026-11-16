//! How fast monsters and player characters run, from their motions, in a
//! synthetic client: what `openm2 content import` writes as run speeds.

mod support;

use m2_assets::Vfs;
use m2_assets::actor::{monster_run_motion, motion_speed, player_run_motion, read_npc_list};
use m2_formats::gr2::Compression;
use m2_formats::gr2::model::{Animation, Curve, Gr2File, TrackGroup, TransformTrack};
use m2_formats::gr2::write::write_model;
use m2_formats::script::motion::{self, MotionData, MotionListEntry};
use m2_formats::script::npc_list::{self, NpcLine};
use m2_formats::script::race::RaceData;
use m2_formats::{EntryKind, RawStr};
use support::{TempDir, test_keys, write_archive};

fn motion_script(gr2: &str, duration: Option<f32>, forward: Option<f32>) -> Vec<u8> {
    MotionData {
        motion_file: RawStr::from(gr2),
        duration,
        accumulation: forward.map(|distance| [0.0, -distance, 0.0]),
        ..MotionData::default()
    }
    .write()
    .unwrap()
}

fn entry(name: &str, file: &str) -> MotionListEntry {
    MotionListEntry {
        mode: RawStr::from("GENERAL"),
        name: RawStr::from(name),
        file: RawStr::from(file),
        weight: 100,
    }
}

/// An animation of `duration` seconds that moves nothing.
fn animation(duration: f32) -> Vec<u8> {
    let empty = |dimension| Curve {
        degree: 0,
        dimension,
        knots: Vec::new(),
        controls: Vec::new(),
    };
    let file = Gr2File {
        animations: vec![Animation {
            name: RawStr::from("run"),
            duration,
            time_step: 1.0 / 30.0,
            track_groups: vec![TrackGroup {
                name: RawStr::from("root"),
                tracks: vec![TransformTrack {
                    name: RawStr::from("root"),
                    position: empty(3),
                    orientation: empty(4),
                    scale_shear: empty(9),
                }],
            }],
        }],
        ..Gr2File::default()
    };
    write_model(&file, Compression::None).unwrap()
}

#[test]
fn run_motions_follow_the_motion_lists_and_their_speeds() {
    let tmp = TempDir::new("run-speed");
    let lines = npc_list::write(&[
        NpcLine {
            vnum: 1001,
            name: RawStr::from("hopper"),
            folder: None,
        },
        NpcLine {
            vnum: 1002,
            name: RawStr::from("crawler"),
            folder: None,
        },
        NpcLine {
            vnum: 1003,
            name: RawStr::from("statue"),
            folder: None,
        },
    ])
    .unwrap();
    let hopper = RaceData {
        motion_list: Some(RawStr::from("moves.txt")),
        ..RaceData::default()
    }
    .write()
    .unwrap();
    let plain = RaceData::default().write().unwrap();
    let hopper_list = motion::write_list(&[
        entry("WAIT", "wait.msa"),
        entry("WALK", "walk.msa"),
        entry("RUN", "run.msa"),
    ])
    .unwrap();
    let crawler_list = motion::write_list(&[entry("WALK", "walk.msa")]).unwrap();
    let statue_list = motion::write_list(&[entry("WAIT", "wait.msa")]).unwrap();
    let warrior = RaceData {
        base_model: Some(RawStr::from("d:/ymir work/pc/warrior/warrior_novice.gr2")),
        ..RaceData::default()
    }
    .write()
    .unwrap();
    let hopper_run = motion_script(
        "d:/ymir work/monster/hopper/run.gr2",
        Some(1.0),
        Some(300.0),
    );
    let hopper_walk = motion_script(
        "d:/ymir work/monster/hopper/walk.gr2",
        Some(1.0),
        Some(100.0),
    );
    // No MotionDuration: the animation's length counts.
    let crawler_walk = motion_script("d:/ymir work/monster/crawler/walk.gr2", None, Some(120.0));
    let crawler_gr2 = animation(0.8);
    let warrior_run = motion_script(
        "d:/ymir work/pc/warrior/general/run.gr2",
        Some(0.8),
        Some(360.0),
    );
    let still = motion_script("d:/ymir work/pc/warrior/general/wait.gr2", Some(2.0), None);
    let files: Vec<(&str, &[u8], EntryKind)> = vec![
        ("npclist.txt", &lines, EntryKind::Lzo),
        (
            "d:/ymir work/monster/hopper/hopper.msm",
            &hopper,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/hopper/moves.txt",
            &hopper_list,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/hopper/run.msa",
            &hopper_run,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/hopper/walk.msa",
            &hopper_walk,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/crawler/crawler.msm",
            &plain,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/crawler/motlist.txt",
            &crawler_list,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/crawler/walk.msa",
            &crawler_walk,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/crawler/walk.gr2",
            &crawler_gr2,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/statue/statue.msm",
            &plain,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/statue/motlist.txt",
            &statue_list,
            EntryKind::Lzo,
        ),
        ("warrior_m.msm", &warrior, EntryKind::Lzo),
        (
            "d:/ymir work/pc/warrior/general/run.msa",
            &warrior_run,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/pc/warrior/general/wait.msa",
            &still,
            EntryKind::Lzo,
        ),
    ];
    write_archive(&tmp.path().join("pack"), "actors", &files, true);
    let vfs = Vfs::open_client(tmp.path(), &test_keys()).unwrap();
    let npcs = read_npc_list(&vfs).unwrap();
    let speed = |script: Option<Vec<u8>>| motion_speed(&vfs, &script.unwrap()).unwrap();

    // RUN before WALK, in the list the script names.
    let run = monster_run_motion(&vfs, &npcs, 1001).unwrap();
    assert_eq!(run, b"d:/ymir work/monster/hopper/run.msa");
    assert_eq!(speed(Some(run)), Some(300.0));
    // WALK when there is no RUN, timed by its animation.
    let walk = monster_run_motion(&vfs, &npcs, 1002);
    assert_eq!(speed(walk), Some(150.0));
    // Neither, or no model at all.
    assert_eq!(monster_run_motion(&vfs, &npcs, 1003), None);
    assert_eq!(monster_run_motion(&vfs, &npcs, 1004), None);

    let run = player_run_motion(&vfs, 0).unwrap();
    assert_eq!(run, b"d:/ymir work/pc/warrior/general/run.msa");
    assert_eq!(speed(Some(run)), Some(450.0));
    assert_eq!(player_run_motion(&vfs, 3), None);
    assert_eq!(player_run_motion(&vfs, 8), None);
    // A motion that moves nothing has no speed.
    assert_eq!(
        motion_speed(&vfs, b"d:/ymir work/pc/warrior/general/wait.msa").unwrap(),
        None
    );
}
