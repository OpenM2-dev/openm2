//! Flying effect files (`docs/specs/assets.md`, "Flying effects"): the
//! client's flying effect scripts through TOML and back, what openm2
//! doesn't read left out, fly types that fly the same as one word, and
//! what is refused.

use m2_assets::actor_file::References;
use m2_assets::fly_file::{FlyFile, FlyPath};
use m2_assets::folder::to_toml;
use m2_formats::RawStr;
use m2_formats::script::fly::{FlyAttach, FlyingData, Tail};

/// Game paths as the import's references: the file's name with
/// `.effect.toml`.
struct Paths;

impl References for Paths {
    fn refer(&mut self, game_path: &[u8]) -> String {
        let text = String::from_utf8_lossy(game_path).to_ascii_lowercase();
        let name = text.rsplit('/').next().unwrap_or(&text).to_owned();
        match name.rsplit_once('.') {
            Some((stem, "mse")) => format!("{stem}.effect.toml"),
            _ => name,
        }
    }

    fn texture_name(&mut self, game_path: &[u8]) -> String {
        self.refer(game_path)
    }
}

fn attach(fly_type: u32, file: &str) -> FlyAttach {
    FlyAttach {
        kind: 1,
        fly_type,
        file: RawStr::from(file),
        tail: None,
        roll: 90.0,
        distance: 10.0,
        period: 0.4,
        amplitude: 150.0,
    }
}

/// A made-up homing bolt that speeds up, with a tail and four attached
/// effects, one of each fly type, and one with no file.
fn bolt() -> FlyingData {
    FlyingData {
        spreading: true,
        maintain_parallel: true,
        initial_velocity: -100.0,
        cone_angle: 90.0,
        roll_angle: 5.0,
        angular_velocity: [0.0, 450.0, 0.0],
        gravity: 3.0,
        hit_on_background: true,
        hit_on_another_monster: true,
        pierce_count: 10,
        collision_radius: 12.0,
        bomb_range: 40.0,
        bomb_effect: Some(RawStr::from("d:/test/blow.mse")),
        homing: true,
        homing_start: 1.0,
        homing_max_angle: 5.0,
        range: 10_000.0,
        acceleration: [0.0, -9000.0, 0.0],
        ignore_z: true,
        attached: vec![
            FlyAttach {
                tail: Some(Tail {
                    colour: 1_726_079_457,
                    length: 0.18,
                    size: 3.0,
                    rect: true,
                }),
                ..attach(1, "d:/test/bolt.mse")
            },
            attach(2, "d:/test/sway.mse"),
            attach(3, "d:/test/ring.mse"),
            attach(4, "d:/test/four.mse"),
            attach(1, ""),
        ],
    }
}

#[test]
fn a_flying_effect_is_written_in_metres_and_words() {
    let file = FlyFile::from_flying(&bolt(), &mut Paths);
    assert_eq!(
        to_toml(&file, false).unwrap(),
        r#"format = 1
speed = -1.0
acceleration = [0.0, -90.0, 0.0]
gravity = 3.0
range = 100.0
bomb_range = 0.4
bomb = "blow.effect.toml"
spread = 90.0
roll = 5.0
spin = [0.0, 450.0, 0.0]

[homing]
from = 1.0
turn = 600.0

[[attach]]
effect = "bolt.effect.toml"
amplitude = 1.5
period = 0.4
roll = 90.0
distance = 0.1

[attach.tail]
colour = [0.8823529411764706, 0.8823529411764706, 0.8823529411764706, 0.4]
length = 0.18
size = 3.0
rect = true

[[attach]]
effect = "sway.effect.toml"
path = "beside"
amplitude = 1.5
period = 0.4
roll = 90.0
distance = 0.1

[[attach]]
effect = "ring.effect.toml"
path = "sway"
amplitude = 1.5
period = 0.4
roll = 90.0
distance = 0.1

[[attach]]
effect = "four.effect.toml"
path = "swell"
amplitude = 1.5
period = 0.4
roll = 90.0
distance = 0.1
"#
    );
    assert_eq!(
        file.attached.iter().map(|a| a.path).collect::<Vec<_>>(),
        [
            FlyPath::Straight,
            FlyPath::Beside,
            FlyPath::Sway,
            FlyPath::Swell
        ]
    );
    assert_eq!(
        file.references(),
        [
            ("effect", "bolt.effect.toml"),
            ("effect", "sway.effect.toml"),
            ("effect", "ring.effect.toml"),
            ("effect", "four.effect.toml"),
            ("effect", "blow.effect.toml"),
        ]
    );
}

#[test]
fn a_flying_effect_reads_back_as_the_client_says_what_openm2_reads() {
    let file = FlyFile::from_flying(&bolt(), &mut Paths);
    let read = FlyFile::parse(&to_toml(&file, false).unwrap()).unwrap();
    assert_eq!(read, file);
    let back = read.to_flying(&|r| Ok(format!("/x/{r}"))).unwrap();
    let mut expected = bolt();
    // What the file leaves out is 0; the attached effect without a file
    // is gone; names are the folder's.
    expected.maintain_parallel = false;
    expected.hit_on_background = false;
    expected.hit_on_another_monster = false;
    expected.pierce_count = 0;
    expected.collision_radius = 0.0;
    expected.ignore_z = false;
    expected.bomb_effect = Some(RawStr::from("/x/blow.effect.toml"));
    expected.attached.pop();
    for (attach, name) in expected
        .attached
        .iter_mut()
        .zip(["bolt", "sway", "ring", "four"])
    {
        attach.file = RawStr::from(format!("/x/{name}.effect.toml").as_str());
    }
    assert_eq!(back, expected);
}

#[test]
fn a_plain_arrow_writes_only_what_it_has() {
    let arrow = FlyingData {
        initial_velocity: 4000.0,
        range: 5000.0,
        bomb_range: 20.0,
        attached: vec![FlyAttach {
            period: 1.0,
            ..attach(1, "arrow.mse")
        }],
        ..FlyingData::default()
    };
    let file = FlyFile::from_flying(&arrow, &mut Paths);
    assert_eq!(file.homing, None);
    assert_eq!(file.bomb, None);
    assert_eq!((file.spread, file.cone, file.roll), (None, None, 0.0));
    let text = to_toml(&file, false).unwrap();
    assert!(
        !text.contains("homing") && !text.contains("acceleration"),
        "{text}"
    );
    let back = FlyFile::parse(&text)
        .unwrap()
        .to_flying(&|r| Ok(r.to_owned()))
        .unwrap();
    assert!(!back.homing && back.bomb_effect.is_none());
    assert_eq!(back.initial_velocity, 4000.0);
}

#[test]
fn broken_flying_effect_files_are_refused() {
    assert!(
        FlyFile::parse("format = 1\nspeed = 1.0\nrange = 1.0\nbomb_range = 0.1\nwobble = 3\n")
            .unwrap_err()
            .contains("wobble")
    );
    assert!(FlyFile::parse("format = 1\nspeed = 1.0\n").is_err());
    assert!(FlyFile::parse("format = 2\nspeed = 1.0\nrange = 1.0\nbomb_range = 0.1\n").is_err());
    let bad_path = "format = 1\nspeed = 1.0\nrange = 1.0\nbomb_range = 0.1\n[[attach]]\neffect = \"a.effect.toml\"\npath = \"zigzag\"\n";
    assert!(FlyFile::parse(bad_path).is_err());
    let file =
        FlyFile::parse("format = 1\nspeed = 1.0\nrange = 1.0\nbomb_range = 0.1\nbomb = \"../x\"\n")
            .unwrap();
    assert!(
        file.to_flying(&|_| Err("outside the folder".to_owned()))
            .is_err()
    );
}

#[test]
fn a_cone_without_spreading_is_kept_as_a_cone() {
    // As the shaman's dragon of the human's client: turned −90° and
    // rolled 90°, not spread.
    let dragon = FlyingData {
        initial_velocity: 1500.0,
        cone_angle: -90.0,
        roll_angle: 90.0,
        range: 3000.0,
        homing: true,
        homing_max_angle: 1.0,
        ..FlyingData::default()
    };
    let file = FlyFile::from_flying(&dragon, &mut Paths);
    assert_eq!(
        (file.spread, file.cone, file.roll),
        (None, Some(-90.0), 90.0)
    );
    let text = to_toml(&file, false).unwrap();
    assert!(text.contains("cone = -90.0\nroll = 90.0\n"), "{text}");
    assert!(text.contains("turn = 120.0"), "{text}");
    let back = FlyFile::parse(&text)
        .unwrap()
        .to_flying(&|r| Ok(r.to_owned()))
        .unwrap();
    assert_eq!(back, dragon);
}
