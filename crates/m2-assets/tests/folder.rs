//! Asset folders: paths and references, writing inside a folder, the
//! import's markers, `assets.toml`, and actor and motion files through TOML
//! and back to the client's types.

mod support;

use m2_assets::actor_file::{
    ActorFile, EventFile, MotionEntry, MotionFile, References, centimetres, metres,
};
use m2_assets::folder::{
    AssetFolder, FolderFile, IMPORT_MARKER, IMPORT_TEXT, MonsterEntry, MusicFile, PlayerActor,
    check_path, folder_of, from_toml, import_mark, reference, relative, resolve, to_toml,
    write_file,
};
use m2_formats::RawStr;
use m2_formats::gltf::{Glb, write_glb};
use m2_formats::script::motion::{
    AttackEvent, Attacking, ComboInput, Effect, EventKind, Fly, Hit, HitPosition, MotionData,
    MotionEvent, Shake, Sphere,
};
use m2_formats::script::race::{
    AttachedEffect, Collision, PartEntry, PartList, RaceData, Skin, Smoke,
};
use m2_formats::script::sound::{SoundScript, TimedSound};
use proptest::prelude::*;
use support::TempDir;

fn s(text: &str) -> RawStr {
    RawStr::from(text)
}

#[test]
fn paths_are_names_between_slashes_that_are_safe_on_every_system() {
    for ok in [
        "a.glb",
        "ymir work/pc/warrior/a.glb",
        "a%C7.png",
        "épée/lame.png",
        "com10",
    ] {
        assert_eq!(check_path(ok), Ok(()), "{ok}");
    }
    for bad in [
        "",
        "/a",
        "a/",
        "a//b",
        "./a",
        "a/../b",
        "a\\b",
        "c:/a",
        "a/b:c",
        "a/x.",
        "a/x ",
        "con",
        "a/NUL.txt",
        "lpt1.log",
        "a\u{1}b",
        "a/b?",
    ] {
        assert!(
            check_path(bad).is_err(),
            "`{}` should be refused",
            bad.escape_debug()
        );
    }
    assert!(check_path(&"a".repeat(256)).is_err());
}

#[test]
fn references_are_relative_to_their_file_or_from_the_root() {
    let from = "ymir work/pc/warrior/general/run.motion.toml";
    assert_eq!(folder_of(from), "ymir work/pc/warrior/general/");
    assert_eq!(
        resolve(from, "run.glb").unwrap(),
        "ymir work/pc/warrior/general/run.glb"
    );
    assert_eq!(
        resolve(from, "../wait.glb").unwrap(),
        "ymir work/pc/warrior/wait.glb"
    );
    assert_eq!(
        resolve(from, "./a/./b.glb").unwrap(),
        "ymir work/pc/warrior/general/a/b.glb"
    );
    assert_eq!(resolve(from, "/sound/walk.wav").unwrap(), "sound/walk.wav");
    assert_eq!(
        resolve("warrior_m.actor.toml", "ymir work/x.glb").unwrap(),
        "ymir work/x.glb"
    );
    for bad in [
        "../../../../../x",
        "/../x",
        "",
        "/",
        "a//b",
        "a\\b",
        "c:/x",
        "a/con",
    ] {
        assert!(resolve(from, bad).is_err(), "`{bad}` should be refused");
    }

    assert_eq!(
        reference(from, "ymir work/pc/warrior/general/run.glb"),
        "run.glb"
    );
    assert_eq!(reference(from, "sound/walk.wav"), "/sound/walk.wav");
    assert_eq!(
        reference("a.actor.toml", "ymir work/x.glb"),
        "ymir work/x.glb"
    );
    assert_eq!(relative("a/b/m.glb", "a/c/t.png"), "../c/t.png");
    assert_eq!(relative("a/b/m.glb", "a/b/t.png"), "t.png");
    assert_eq!(relative("m.glb", "a/t.png"), "a/t.png");
    assert_eq!(relative("a/m.glb", "t.png"), "../t.png");
    for (from, to) in [
        (from, "ymir work/pc/common/x.png"),
        (from, "sound/walk.wav"),
        ("a/b/m.glb", "a/b/c/d.png"),
        ("m.glb", "n.png"),
    ] {
        assert_eq!(resolve(from, &relative(from, to)).unwrap(), to);
        assert_eq!(resolve(from, &reference(from, to)).unwrap(), to);
    }
}

#[test]
fn files_are_written_inside_the_folder_and_replaced_only_when_asked() {
    let tmp = TempDir::new("folder-write");
    let root = tmp.path();
    write_file(root, "a/b/c.txt", b"one", false).unwrap();
    assert_eq!(std::fs::read(root.join("a/b/c.txt")).unwrap(), b"one");
    assert!(write_file(root, "a/b/c.txt", b"two", false).is_err());
    write_file(root, "a/b/c.txt", b"two", true).unwrap();
    assert_eq!(std::fs::read(root.join("a/b/c.txt")).unwrap(), b"two");
    // A file where a folder should be.
    assert!(write_file(root, "a/b/c.txt/d.txt", b"x", true).is_err());
    // A folder where the file should be.
    assert!(write_file(root, "a/b", b"x", true).is_err());
    for bad in ["../x.txt", "/x.txt", "a/../../x.txt", "a\\..\\x.txt"] {
        assert!(write_file(root, bad, b"x", true).is_err(), "{bad}");
    }
    let folder = AssetFolder::new(root);
    assert_eq!(folder.read("a/b/c.txt").unwrap(), b"two");
    assert!(folder.read("a/b/missing.txt").is_err());
    assert!(folder.read("../x").is_err());
    let (files, refused) = folder.files().unwrap();
    assert_eq!(files, ["a/b/c.txt"]);
    assert!(refused.is_empty());
}

/// Writers on many threads at once make the same new folders: each one's
/// file is written, as the import writes on its threads.
#[test]
fn files_are_written_from_many_threads_into_the_same_new_folders() {
    let tmp = TempDir::new("folder-threads");
    let root = tmp.path();
    std::thread::scope(|scope| {
        for thread in 0..8 {
            scope.spawn(move || {
                for file in 0..16 {
                    let path = format!("a/b/c/{thread}-{file}.txt");
                    write_file(root, &path, path.as_bytes(), false).unwrap();
                }
            });
        }
    });
    let (files, refused) = AssetFolder::new(root).files().unwrap();
    assert_eq!(files.len(), 8 * 16);
    assert!(refused.is_empty());
    assert_eq!(
        std::fs::read(root.join("a/b/c/7-15.txt")).unwrap(),
        b"a/b/c/7-15.txt"
    );
}

/// A PNG's chunks, their CRCs left zero (the marker search doesn't check
/// them).
fn png(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in chunks {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(*kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0; 4]);
    }
    out
}

#[test]
fn imported_files_are_known_by_their_marker() {
    let toml = format!("{IMPORT_MARKER}\nformat = 1\n");
    assert_eq!(import_mark(toml.as_bytes()), Some("openm2 assets import"));
    let content = b"# openm2 content import: from the operator's own client\nformat = 1\n";
    assert_eq!(import_mark(content), Some("openm2 content import"));
    assert_eq!(import_mark(b"# my own notes\nformat = 1\n"), None);
    assert_eq!(import_mark(b"format = 1\n"), None);

    let text = [b"Comment\0".as_slice(), IMPORT_TEXT.as_bytes()].concat();
    let marked = png(&[
        (b"IHDR", &[0; 13]),
        (b"tEXt", &text),
        (b"IDAT", &[1, 2]),
        (b"IEND", &[]),
    ]);
    assert_eq!(import_mark(&marked), Some("openm2 assets import"));
    let international = [b"Comment\0\0\0en\0\0".as_slice(), IMPORT_TEXT.as_bytes()].concat();
    let marked = png(&[
        (b"IHDR", &[0; 13]),
        (b"iTXt", &international),
        (b"IEND", &[]),
    ]);
    assert_eq!(import_mark(&marked), Some("openm2 assets import"));
    let plain = png(&[
        (b"IHDR", &[0; 13]),
        (b"tEXt", b"Comment\0drawn by hand"),
        (b"IEND", &[]),
    ]);
    assert_eq!(import_mark(&plain), None);
    // A damaged PNG: no marker, no panic.
    assert_eq!(import_mark(&marked[..20]), None);

    let glb = |generator: &str| {
        write_glb(&Glb {
            generator,
            ..Glb::default()
        })
        .unwrap()
    };
    assert_eq!(import_mark(&glb(IMPORT_TEXT)), Some("openm2 assets import"));
    assert_eq!(import_mark(&glb("Khronos glTF Blender I/O")), None);
}

#[test]
fn the_folder_file_names_its_actors() {
    let file = FolderFile {
        format: 1,
        name: "imported".to_owned(),
        import: Some(1),
        players: vec![PlayerActor {
            race: 0,
            actor: "warrior_m.actor.toml".to_owned(),
        }],
        monsters: vec![
            MonsterEntry {
                vnum: 101,
                name: Some("Wild Dog".to_owned()),
                level: Some(1),
                actor: Some("ymir work/monster/stray_dog/stray_dog.actor.toml".to_owned()),
            },
            // Named, not drawn.
            MonsterEntry {
                vnum: 102,
                name: Some("Wolf".to_owned()),
                ..MonsterEntry::default()
            },
        ],
        skin: Some("skin.toml".to_owned()),
        items: Some("items.toml".to_owned()),
        skills: Some("skills.toml".to_owned()),
        hit_sounds: vec!["sound/hit_1.wav".to_owned()],
        music: Some(MusicFile {
            login: Some("bgm/login.mp3".to_owned()),
            ..MusicFile::default()
        }),
        ..FolderFile::default()
    };
    let text = to_toml(&file, true).unwrap();
    assert!(text.starts_with(IMPORT_MARKER));
    assert!(text.contains("name = \"imported\"\nimport = 1\n"), "{text}");
    assert!(text.contains("[[player]]\nrace = 0\n"), "{text}");
    assert!(
        text.contains("skin = \"skin.toml\"\nitems = \"items.toml\"\nskills = \"skills.toml\"\n"),
        "{text}"
    );
    assert!(
        text.contains("[music]\nlogin = \"bgm/login.mp3\"\n"),
        "{text}"
    );
    assert!(
        text.ends_with("[[monster]]\nvnum = 102\nname = \"Wolf\"\n"),
        "{text}"
    );
    assert_eq!(FolderFile::parse(&text).unwrap(), file);
    let error = FolderFile::parse("format = 1\nname = \"x\"\ncolour = 3\n").unwrap_err();
    assert!(
        error.contains("line 3") && error.contains("colour"),
        "{error}"
    );
    let error = FolderFile::parse("format = 2\nname = \"x\"\n").unwrap_err();
    assert!(error.contains("format 2"), "{error}");
}

/// Game paths as the folder names them: their last two names, with the
/// extension a converted file gets.
struct Paths;

impl References for Paths {
    fn refer(&mut self, game_path: &[u8]) -> String {
        let text = String::from_utf8_lossy(game_path)
            .replace('\\', "/")
            .to_ascii_lowercase();
        let name = text.rsplit('/').next().unwrap_or(&text).to_owned();
        let (stem, extension) = name.rsplit_once('.').unwrap_or((&name, ""));
        let extension = match extension {
            "gr2" => "glb",
            "dds" | "tga" => "png",
            "msa" => "motion.toml",
            other => other,
        };
        format!("/{stem}.{extension}")
    }

    fn texture_name(&mut self, game_path: &[u8]) -> String {
        self.refer(game_path).trim_start_matches('/').to_owned()
    }
}

fn race() -> RaceData {
    RaceData {
        base_model: Some(s("d:/ymir work/pc/warrior/warrior_novice.gr2")),
        attribute_file: None,
        motion_list: None,
        shapes: PartList {
            path: s("d:/ymir work/pc/warrior/"),
            entries: vec![PartEntry {
                index: 1,
                special_path: None,
                model: s("warrior_novice.gr2"),
                skins: vec![
                    Skin {
                        source: s("warrior_novice_red.dds"),
                        target: Some(s("warrior_novice_blue.dds")),
                    },
                    Skin {
                        source: s("kept.dds"),
                        target: None,
                    },
                ],
            }],
        },
        hairs: PartList {
            path: s("d:/ymir work/pc/warrior/"),
            entries: vec![PartEntry {
                index: 0,
                special_path: Some(s("d:/ymir work/pc/warrior/hair/")),
                model: s("hair_1_1.gr2"),
                skins: vec![],
            }],
        },
        collisions: vec![
            Collision {
                collision_type: 3,
                attached: true,
                bone: s("Bip01"),
                spheres: vec![Sphere {
                    radius: 90.0,
                    position: [0.0, -10.0, 80.0],
                }],
            },
            Collision {
                collision_type: 1,
                attached: false,
                bone: RawStr::default(),
                spheres: vec![Sphere {
                    radius: 12.345,
                    position: [1.5, 0.0, -0.25],
                }],
            },
        ],
        effects: vec![AttachedEffect {
            attached: true,
            bone: s("Bip01 Head"),
            file: s("d:/ymir work/effect/glow.mse"),
            position: [0.0, 0.0, 20.0],
            rotation: Some([0.0, 90.0, 0.0]),
        }],
        smoke: Some(Smoke {
            bone: s("Bip01"),
            stages: vec![(0, s("d:/ymir work/effect/smoke_0.mse"))],
        }),
    }
}

/// The paths the race's references resolve to, from an actor at the root.
fn resolved(text: &str) -> Result<String, String> {
    resolve("a.actor.toml", text)
}

#[test]
fn an_actor_holds_what_its_model_script_says() {
    let mut actor = ActorFile::from_race(&race(), &mut Paths);
    actor.motions.push(MotionEntry {
        set: "general".to_owned(),
        name: "run".to_owned(),
        file: "/run.motion.toml".to_owned(),
        weight: 1,
    });
    let text = to_toml(&actor, true).unwrap();
    let read = ActorFile::parse(&text).unwrap();
    assert_eq!(read, actor);
    // Centimetres as metres, as few digits as they need.
    assert!(text.contains("radius = 0.9\n"), "{text}");
    assert!(text.contains("radius = 0.12345\n"), "{text}");
    assert!(text.contains("at = [0.015, 0.0, -0.0025]"), "{text}");
    assert_eq!(read.motion(Some("GENERAL"), "RUN"), actor.motions.first());
    assert_eq!(read.motion(None, "walk"), None);

    let back = read.to_race(&resolved).unwrap();
    let expected = race();
    assert_eq!(back.base_model, Some(s("warrior_novice.glb")));
    let shape = &back.shapes.entries[0];
    assert_eq!(shape.index, 1);
    assert_eq!(shape.model, s("warrior_novice.glb"));
    // A skin that keeps its texture is no skin.
    assert_eq!(
        shape.skins,
        [Skin {
            source: s("warrior_novice_red.png"),
            target: Some(s("warrior_novice_blue.png")),
        }]
    );
    assert_eq!(back.hairs.entries[0].model, s("hair_1_1.glb"));
    assert_eq!(back.collisions, expected.collisions);
    assert_eq!(back.effects[0].file, s("glow.mse"));
    assert_eq!(back.effects[0].position, expected.effects[0].position);
    assert_eq!(back.effects[0].rotation, expected.effects[0].rotation);
    assert_eq!(back.smoke.unwrap().stages, [(0, s("smoke_0.mse"))]);
}

fn motion() -> MotionData {
    MotionData {
        motion_file: s("d:/ymir work/pc/warrior/onehand_sword/combo_01.gr2"),
        duration: Some(1.0),
        accumulation: Some([0.0, -120.5, 0.0]),
        combo: Some(ComboInput {
            pre_input: 0.167_094,
            direct_input: 0.533_333,
            input_limit: 0.602_564,
            link: Some(0.7),
        }),
        attacking: Some(Attacking {
            attack_type: Some(0),
            attacking_type: None,
            motion_type: Some(2),
            hitting_type: 2,
            stiffen_time: 0.0,
            invisible_time: 0.1,
            external_force: 8.0,
            hit_limit: Some(3),
            hits: vec![Hit {
                start: 0.192_308,
                end: 0.315_385,
                bone: s("equip_right_hand"),
                weapon_length: 130.0,
                positions: vec![HitPosition {
                    time: 0.192_308,
                    from: [-46.99, -13.97, 146.68],
                    to: [-54.27, 108.74, 188.98],
                }],
            }],
        }),
        events: vec![
            MotionEvent {
                time: 0.1,
                kind: EventKind::Effect(Effect {
                    file: s("d:/ymir work/effect/swing.mse"),
                    position: [0.0, 0.0, 100.0],
                    bone: Some(s("equip_right_hand")),
                    following: true,
                    independent: Some(false),
                    fishing: None,
                }),
            },
            MotionEvent {
                time: 0.2,
                kind: EventKind::SpotEffect(Effect {
                    file: s("d:/ymir work/effect/dust.mse"),
                    position: [0.0; 3],
                    bone: None,
                    following: false,
                    independent: None,
                    fishing: Some(true),
                }),
            },
            MotionEvent {
                time: 0.3,
                kind: EventKind::Shake(Shake {
                    during: 0.5,
                    power: 2.0,
                    range: 1000.0,
                }),
            },
            MotionEvent {
                time: 0.4,
                kind: EventKind::Attack(AttackEvent {
                    during: 0.25,
                    enable_hit: Some(true),
                    attack_type: None,
                    hitting_type: 1,
                    stiffen_time: 0.0,
                    invisible_time: 0.0,
                    external_force: 0.0,
                    hit_limit: None,
                    collision_type: 4,
                    spheres: vec![Sphere {
                        radius: 150.0,
                        position: [0.0, -100.0, 50.0],
                    }],
                }),
            },
            MotionEvent {
                time: 0.5,
                kind: EventKind::Fly(Fly {
                    file: s("d:/ymir work/effect/arrow.msf"),
                    position: [0.0, -50.0, 120.0],
                    bone: None,
                }),
            },
            MotionEvent {
                time: 0.6,
                kind: EventKind::Other(7),
            },
        ],
    }
}

fn sounds() -> SoundScript {
    SoundScript {
        sounds: vec![
            TimedSound {
                time: 0.759,
                file: s("sound/common/walk_dirt_n.wav"),
            },
            TimedSound {
                time: 0.198,
                file: s("sound/common/walk_dirt_n.wav"),
            },
        ],
    }
}

#[test]
fn a_motion_holds_what_its_scripts_say() {
    let file = MotionFile::from_motion(&motion(), Some(&sounds()), &mut Paths);
    let text = to_toml(&file, false).unwrap();
    assert!(!text.starts_with('#'));
    let read = MotionFile::parse(&text).unwrap();
    assert_eq!(read, file);
    assert!(text.contains("moves = [0.0, -1.205, 0.0]"), "{text}");
    assert!(text.contains("kind = \"spot-effect\""), "{text}");
    let (data, sounds) = read.to_motion(&resolved).unwrap();
    let expected = motion();
    assert_eq!(data.motion_file, s("combo_01.glb"));
    assert_eq!(data.duration, expected.duration);
    assert_eq!(data.accumulation, expected.accumulation);
    assert_eq!(data.combo, expected.combo);
    assert_eq!(data.attacking, expected.attacking);
    assert_eq!(data.events.len(), expected.events.len());
    for (a, b) in data.events.iter().zip(&expected.events) {
        assert_eq!(a.time, b.time);
        match (&a.kind, &b.kind) {
            (EventKind::Effect(x), EventKind::Effect(y))
            | (EventKind::SpotEffect(x), EventKind::SpotEffect(y)) => {
                assert_eq!(
                    (x.position, &x.bone, x.following, x.independent, x.fishing),
                    (y.position, &y.bone, y.following, y.independent, y.fishing)
                );
            }
            (EventKind::Fly(x), EventKind::Fly(y)) => assert_eq!(x.position, y.position),
            (x, y) => assert_eq!(x, y),
        }
    }
    // Sounds in time order.
    let times: Vec<f32> = sounds.sounds.iter().map(|s| s.time).collect();
    assert_eq!(times, [0.198, 0.759]);
    assert_eq!(sounds.sounds[0].file, s("walk_dirt_n.wav"));
    let kinds: Vec<&str> = read.references().iter().map(|(kind, _)| *kind).collect();
    assert_eq!(
        kinds,
        [
            "animation",
            "effect",
            "effect",
            "flying effect",
            "sound",
            "sound"
        ]
    );
}

#[test]
fn unknown_keys_are_refused_everywhere() {
    let base = "format = 1\nanimation = \"a.glb\"\n";
    for (extra, word) in [
        ("speed = 3\n", "speed"),
        (
            "[combo]\npre_input = 0.1\ndirect_input = 0.2\ninput_limit = 0.3\nlate = 1\n",
            "late",
        ),
        (
            "[[event]]\nkind = \"shake\"\ntime = 0\nduring = 1\npower = 1\nrange = 1\nloud = 1\n",
            "loud",
        ),
        (
            "[[event]]\nkind = \"effect\"\ntime = 0\nfile = \"e.mse\"\nat = [0, 0, 0]\nglow = 1\n",
            "glow",
        ),
        ("[[event]]\nkind = \"dance\"\ntime = 0\n", "dance"),
    ] {
        let error = MotionFile::parse(&format!("{base}{extra}")).unwrap_err();
        assert!(error.contains(word), "{word}: {error}");
    }
    let parsed = MotionFile::parse(&format!(
        "{base}[[event]]\nkind = \"effect\"\ntime = 0.5\nfile = \"e.mse\"\nat = [0, 0, 1]\n"
    ))
    .unwrap();
    assert!(matches!(&parsed.events[0], EventFile::Effect(e) if e.at == [0.0, 0.0, 1.0]));
    assert!(
        ActorFile::parse("format = 1\n[[shape]]\nindex = 1\nmodel = \"a.glb\"\ncolor = 1\n")
            .is_err()
    );
    let missing: Result<FolderFile, String> = from_toml("format = 1\n", |f: &FolderFile| f.format);
    assert!(missing.unwrap_err().contains("name"));
}

proptest! {
    #[test]
    fn centimetres_read_back_from_metres(cm in proptest::num::f32::NORMAL | proptest::num::f32::ZERO) {
        prop_assume!(cm.abs() < 1.0e30);
        prop_assert_eq!(centimetres(metres(cm)), cm);
    }

    #[test]
    fn references_never_leave_the_folder(from in "[a-z/.]{0,20}", reference in "[a-z/.%]{0,20}") {
        if let Ok(path) = resolve(&from, &reference) {
            prop_assert!(check_path(&path).is_ok());
            prop_assert!(!path.split('/').any(|p| p == ".." || p == "."));
        }
    }
}
