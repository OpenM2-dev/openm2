//! Text scripts: the syntax, the writer's exact text, race data (`.msm`),
//! motion data (`.msa`) and motion lists. The inputs are written here by
//! hand in the client's style or built with our writers.

use m2_formats::script::environment::{self, Environment, Fog, Light, Material, SkyBox};
use m2_formats::script::fly::{FlyAttach, FlyingData, Tail};
use m2_formats::script::item_list::{self, ItemLine};
use m2_formats::script::motion::{
    self, AttackEvent, Attacking, ComboInput, Effect, EventKind, Fly, Hit, HitPosition, MotionData,
    MotionEvent, MotionListEntry, Shake, Sphere,
};
use m2_formats::script::npc_list::{self, NpcLine, NpcModel};
use m2_formats::script::race::{
    self, AttachedEffect, Collision, PartEntry, PartList, RaceData, Skin, Smoke,
};
use m2_formats::script::skill_desc::{self, SkillLine};
use m2_formats::script::skill_table::{self, SkillTableLine};
use m2_formats::script::spawn_list::{self, SpawnKind, SpawnLine};
use m2_formats::script::sub::{self, SubImage};
use m2_formats::script::{self, Body, Entry, Script, Token};
use m2_formats::{FormatError, RawStr};
use proptest::prelude::*;

fn bare(text: &str) -> Token {
    Token::bare(text)
}

fn quoted(text: &str) -> Token {
    Token::quoted(text)
}

/// The script with every line number set to 0, to compare with one built in
/// code.
fn without_lines(script: &Script) -> Script {
    Script {
        entries: script
            .entries
            .iter()
            .map(|entry| Entry {
                line: 0,
                name: entry.name.clone(),
                body: match &entry.body {
                    Body::Group(group) => Body::Group(without_lines(group)),
                    other => other.clone(),
                },
            })
            .collect(),
    }
}

fn damaged(result: Result<impl std::fmt::Debug, FormatError>) -> String {
    match result {
        Err(FormatError::Damaged { detail, .. }) => detail,
        other => panic!("expected Damaged, got {other:?}"),
    }
}

/// A script in the client's style: aligned keys, tabs, CRLF, a brace with a
/// tab after it, a blank line of spaces, EUC-KR bytes in a quoted value.
const CLIENT_STYLE: &[u8] = b"ScriptType            ItemDataScript\r\n\
\r\n\
Type                  1\r\n\
ModelFileName         \"d:/ymir work/item/test/sword.gr2\"\r\n\
949Description        \"\xb0\xa1\xb3\xaa\"\r\n\
TailColor             12345678d\r\n\
List HitEffectFileName\r\n\
{\t\r\n\
    \"d:/ymir work/effect/test/hit.mse\"\r\n\
    0.000000 \"MOVING_TYPE_DIRECT\" 0.000000\r\n\
}\r\n\
    \r\n\
Group AttachingData\r\n\
{\r\n\
\tAttachingDataCount\t1\r\n\
\tGroup AttachingData00\r\n\
\t{\r\n\
\t\tAttachingBoneName    \"Bone01\"\r\n\
\t\tPosition             1.000000 2.000000 3.000000\r\n\
\t}\r\n\
}\r\n";

#[test]
fn a_script_in_the_clients_style_reads_into_a_tree() {
    let parsed = script::parse(CLIENT_STYLE).unwrap();
    let names: Vec<String> = parsed.entries.iter().map(|e| e.name.to_string()).collect();
    assert_eq!(
        names,
        [
            "ScriptType",
            "Type",
            "ModelFileName",
            "949Description",
            "TailColor",
            "HitEffectFileName",
            "AttachingData"
        ]
    );
    assert_eq!(parsed.entries[0].line, 1);
    assert_eq!(parsed.entries[2].line, 4);
    assert_eq!(parsed.script_type(), Some(&RawStr::from("ItemDataScript")));
    assert_eq!(
        parsed.values("ModelFileName").unwrap(),
        [quoted("d:/ymir work/item/test/sword.gr2")]
    );
    assert_eq!(
        parsed.values("949Description").unwrap()[0].text.as_bytes(),
        b"\xb0\xa1\xb3\xaa"
    );
    assert_eq!(parsed.values("TailColor").unwrap(), [bare("12345678d")]);
    assert_eq!(
        parsed.list("HitEffectFileName").unwrap(),
        [
            vec![quoted("d:/ymir work/effect/test/hit.mse")],
            vec![
                bare("0.000000"),
                quoted("MOVING_TYPE_DIRECT"),
                bare("0.000000")
            ],
        ]
    );
    let attaching = parsed.group("AttachingData").unwrap();
    assert_eq!(
        attaching.number::<u32>("AttachingDataCount", "t").unwrap(),
        Some(1)
    );
    let block = attaching.group("AttachingData00").unwrap();
    assert_eq!(block.entries[0].line, 18);
    assert_eq!(
        block.numbers::<f32, 3>("Position", "t").unwrap(),
        Some([1.0, 2.0, 3.0])
    );
}

#[test]
fn lookups_ignore_case_and_take_the_first_match() {
    let parsed = script::parse(b"Model a\nmodel b\nGROUP g\n{\n}\nlist l\n{\nx\n}\n").unwrap();
    assert_eq!(parsed.values("MODEL").unwrap(), [bare("a")]);
    assert!(parsed.group("G").is_some());
    assert_eq!(parsed.list("L").unwrap(), [vec![bare("x")]]);
    // A lookup finds only entries of its kind.
    assert!(parsed.group("Model").is_none());
    assert!(parsed.values("g").is_none());
    assert_eq!(parsed.text("missing", "t").unwrap(), None);
}

#[test]
fn bare_line_feeds_empty_values_and_keys_alone() {
    let parsed = script::parse(b"PathName \"\"\nCycleLoopEnable\n\n  Key\tv1  v2\t").unwrap();
    assert_eq!(parsed.values("PathName").unwrap(), [quoted("")]);
    assert_eq!(parsed.values("CycleLoopEnable").unwrap(), []);
    assert_eq!(parsed.values("Key").unwrap(), [bare("v1"), bare("v2")]);
    assert_eq!(parsed.entries[2].line, 4);
    // A quote ends a quoted value even when a word follows it directly.
    let parsed = script::parse(b"K \"a b\"c").unwrap();
    assert_eq!(parsed.values("K").unwrap(), [quoted("a b"), bare("c")]);
}

#[test]
fn typed_values_name_the_line_and_key_when_wrong() {
    let parsed = script::parse(b"ScriptType X\nCount 1.5\nPair 1 2\nName\n").unwrap();
    assert_eq!(
        damaged(parsed.number::<u32>("Count", "t")),
        "line 2: Count: `1.5` is not a number"
    );
    assert_eq!(
        damaged(parsed.text("Pair", "t")),
        "line 3: Pair: expected one value, found 2"
    );
    assert_eq!(
        damaged(parsed.text("Name", "t")),
        "line 4: Name: expected one value, found 0"
    );
    assert_eq!(
        damaged(parsed.numbers::<f32, 3>("Pair", "t")),
        "line 3: Pair: expected 3 numbers, found 2"
    );
}

#[test]
fn broken_structure_is_refused_with_its_line() {
    let cases: [(&[u8], &str); 9] = [
        (b"Group A\n{\nKey 1\n", "line 1: the block is not closed"),
        (b"List A\n{\n1 2\n", "line 1: the list is not closed"),
        (b"Key 1\n}\n", "line 2: `}` without an open block"),
        (b"{\n}\n", "line 1: `{` without `Group` or `List` before it"),
        (
            b"Group A\nKey 1\n",
            "line 2: expected `{` after the block name on line 1",
        ),
        (b"Group A\n", "line 1: the file ends after the block name"),
        (b"List A\n{\n{\n}\n", "line 3: `{` inside a list"),
        (b"Key \"open\n", "line 1: a quoted value is not closed"),
        (b"\"Key\" 1\n", "line 1: a key must not be quoted"),
    ];
    for (bytes, expected) in cases {
        assert_eq!(
            damaged(script::parse(bytes)),
            expected,
            "{}",
            bytes.escape_ascii()
        );
    }
}

#[test]
fn nesting_and_size_are_capped() {
    let depth = script::MAX_DEPTH + 1;
    let mut text = Vec::new();
    for _ in 0..depth {
        text.extend_from_slice(b"Group A\n{\n");
    }
    for _ in 0..depth {
        text.extend_from_slice(b"}\n");
    }
    assert!(damaged(script::parse(&text)).contains("blocks nest deeper than 16"));
    let big = vec![b' '; script::MAX_LEN + 1];
    assert!(matches!(
        script::parse(&big),
        Err(FormatError::TooLarge { .. })
    ));
}

/// A script with every kind of entry, and the exact text the writer makes
/// of it.
fn environment() -> Environment {
    Environment {
        light_direction: Some([0.5, 0.5, -0.75]),
        background: Some(Light {
            enabled: true,
            diffuse: [1.0, 0.5, 0.25, 1.0],
            ambient: [0.0, 0.0, 0.0, 1.0],
        }),
        character: None,
        material: Some(Material {
            diffuse: [1.0; 4],
            ambient: [0.75, 0.5, 0.25, 1.0],
            emissive: [0.25, 0.25, 0.5, 1.0],
        }),
        fog: Some(Fog {
            enabled: false,
            near: 5000.0,
            far: 20_000.0,
            colour: [0.5, 0.75, 1.0, 1.0],
        }),
        sky: Some(SkyBox {
            textured: true,
            scale: [3500.0; 3],
            upper_bands: 1,
            lower_bands: 1,
            faces: ["f.dds", "b.dds", "l.dds", "r.dds", "t.dds", ""].map(RawStr::from),
            cloud_scale: [200_000.0; 2],
            cloud_height: 30_000.0,
            cloud_texture_scale: [4.0, 2.0],
            cloud_speed: [0.25, 0.5],
            cloud_texture: RawStr::from("clouds.tga"),
            cloud_colours: vec![[0.0; 4], [1.0, 0.5, 0.25, 0.0]],
            gradient: vec![
                [0.25, 0.5, 0.75, 0.0],
                [0.5, 0.75, 1.0, 0.0],
                [0.5, 0.75, 1.0, 0.0],
                [0.25, 0.25, 0.5, 0.0],
            ],
        }),
    }
}

#[test]
fn environment_byte_for_byte() {
    let text = b"ScriptType\tEnvrionmentData\r\n\
Group DirectionalLight\r\n\
{\r\n\
\x20   Direction\t0.500000 0.500000 -0.750000\r\n\
\x20   Group Background\r\n\
\x20   {\r\n\
\x20       Enable\t1\r\n\
\x20       Diffuse\t1.000000 0.500000 0.250000 1.000000\r\n\
\x20       Ambient\t0.000000 0.000000 0.000000 1.000000\r\n\
\x20   }\r\n\
}\r\n\
Group Material\r\n\
{\r\n\
\x20   Diffuse\t1.000000 1.000000 1.000000 1.000000\r\n\
\x20   Ambient\t0.750000 0.500000 0.250000 1.000000\r\n\
\x20   Emissive\t0.250000 0.250000 0.500000 1.000000\r\n\
}\r\n\
Group Fog\r\n\
{\r\n\
\x20   Enable\t0\r\n\
\x20   NearDistance\t5000.000000\r\n\
\x20   FarDistance\t20000.000000\r\n\
\x20   Color\t0.500000 0.750000 1.000000 1.000000\r\n\
}\r\n\
Group SkyBox\r\n\
{\r\n\
\x20   bTextureRenderMode\t1\r\n\
\x20   Scale\t3500.000000 3500.000000 3500.000000\r\n\
\x20   GradientLevelUpper\t1\r\n\
\x20   GradientLevelLower\t1\r\n\
\x20   FrontFaceFileName\t\"f.dds\"\r\n\
\x20   BackFaceFileName\t\"b.dds\"\r\n\
\x20   LeftFaceFileName\t\"l.dds\"\r\n\
\x20   RightFaceFileName\t\"r.dds\"\r\n\
\x20   TopFaceFileName\t\"t.dds\"\r\n\
\x20   BottomFaceFileName\t\"\"\r\n\
\x20   CloudScale\t200000.000000 200000.000000\r\n\
\x20   CloudHeight\t30000.000000\r\n\
\x20   CloudTextureScale\t4.000000 2.000000\r\n\
\x20   CloudSpeed\t0.250000 0.500000\r\n\
\x20   CloudTextureFileName\t\"clouds.tga\"\r\n\
\x20   List CloudColor\r\n\
\x20   {\r\n\
\x20       0.000000 0.000000 0.000000 0.000000\r\n\
\x20       1.000000 0.500000 0.250000 0.000000\r\n\
\x20   }\r\n\
\x20   List Gradient\r\n\
\x20   {\r\n\
\x20       0.250000 0.500000 0.750000 0.000000\r\n\
\x20       0.500000 0.750000 1.000000 0.000000\r\n\
\x20       0.500000 0.750000 1.000000 0.000000\r\n\
\x20       0.250000 0.250000 0.500000 0.000000\r\n\
\x20   }\r\n\
}\r\n";
    assert_eq!(environment().write().unwrap(), text);
    assert_eq!(Environment::parse(text).unwrap(), environment());
    assert_eq!(environment::SCRIPT_TYPE, "EnvrionmentData");
}

#[test]
fn environment_in_the_clients_style_and_its_errors() {
    // Other groups are left alone; missing groups are None.
    let text = b"ScriptType         EnvrionmentData\r\n\
ScriptVersion      1.0000\r\n\
\r\n\
Group SkyBox\r\n\
{\r\n\
    Scale         3500.000000 3500.000000 3500.000000\r\n\
    GradientLevelUpper    1\r\n\
    CloudTextureFileName  \"D:\\ymir work\\environment\\clouds_zone02.tga\"\r\n\
    List Gradient\r\n\
    {\r\n\
        0.094118 0.286275 0.658824 0.000000\r\n\
        \r\n\
        0.850980 0.901961 1.000000 0.000000\r\n\
    }\r\n\
}\r\n\
Group DirectionalLight\r\n\
{\r\n\
    Group Character\r\n\
    {\r\n\
        Diffuse       1.000000 1.000000 1.000000 1.000000\r\n\
    }\r\n\
}\r\n";
    let parsed = Environment::parse(text).unwrap();
    assert_eq!(parsed.light_direction, None);
    assert_eq!(parsed.background, None);
    let character = parsed.character.unwrap();
    assert!(!character.enabled);
    assert_eq!(character.diffuse, [1.0; 4]);
    assert_eq!(character.ambient, [0.0; 4]);
    assert_eq!((parsed.material, parsed.fog), (None, None));
    // Blank lines in a list are skipped; missing sky values are 0, empty or
    // false, and only the six face keys name faces.
    let sky = parsed.sky.unwrap();
    assert!(!sky.textured);
    assert_eq!(
        (sky.scale, sky.upper_bands, sky.lower_bands),
        ([3500.0; 3], 1, 0)
    );
    assert_eq!(
        sky.cloud_texture.as_bytes(),
        b"D:\\ymir work\\environment\\clouds_zone02.tga"
    );
    assert_eq!(
        sky.gradient,
        [
            [0.094_118, 0.286_275, 0.658_824, 0.0],
            [0.850_98, 0.901_961, 1.0, 0.0]
        ]
    );
    assert!(sky.cloud_colours.is_empty() && sky.faces.iter().all(RawStr::is_empty));
    assert_eq!((sky.cloud_height, sky.cloud_speed), (0.0, [0.0; 2]));

    assert!(matches!(
        Environment::parse(b"ScriptType MotionData\r\n"),
        Err(FormatError::UnknownLayout { .. })
    ));
    assert_eq!(
        damaged(Environment::parse(
            b"ScriptType EnvrionmentData\nGroup DirectionalLight\n{\nDirection 1 2\n}\n"
        )),
        "line 4: Direction: expected 3 numbers, found 2"
    );
    for (list, detail) in [
        (
            "List Gradient\n{\n1 2 3\n}\n",
            "line 4: Gradient: a colour that is not 4 numbers",
        ),
        (
            "List CloudColor\n{\n1 2 x 4\n}\n",
            "line 4: CloudColor: a colour that is not 4 numbers",
        ),
        ("Gradient 1 2 3 4\n", "line 4: Gradient: expected a list"),
    ] {
        let text = format!("ScriptType EnvrionmentData\nGroup SkyBox\n{{\n{list}}}\n");
        assert_eq!(
            damaged(Environment::parse(text.as_bytes())),
            detail,
            "{list}"
        );
    }
    // Without a texture mode or faces, the writer leaves them out.
    let plain = Environment {
        sky: Some(SkyBox::default()),
        ..Environment::default()
    };
    let written = String::from_utf8(plain.write().unwrap()).unwrap();
    assert!(!written.contains("FaceFileName") && !written.contains("bTextureRenderMode"));
    assert_eq!(Environment::parse(written.as_bytes()).unwrap(), plain);
}

fn sample() -> (Script, &'static [u8]) {
    let mut inner = Script::default();
    inner.push_values("HairIndex", vec![bare("0")]);
    inner.push_values("Model", vec![quoted("hair/hair_1_1.gr2")]);
    let mut hair = Script::default();
    hair.push_values("PathName", vec![quoted("d:/x/")]);
    hair.push_values("HairDataCount", vec![bare("1")]);
    hair.push_group("HairData00", inner);
    let mut script = Script::default();
    script.push_values("ScriptType", vec![bare("RaceDataScript")]);
    script.push_values("Accumulation", vec![bare("0.5"), bare("-1"), quoted("")]);
    script.push_values("Flag", vec![]);
    script.push_group("HairData", hair);
    script.push_list(
        "HitEffectFileName",
        vec![vec![quoted("a b.mse")], vec![bare("0.5"), quoted("X")]],
    );
    let text: &[u8] = b"ScriptType\tRaceDataScript\r\n\
Accumulation\t0.5 -1 \"\"\r\n\
Flag\r\n\
Group HairData\r\n\
{\r\n\
\x20   PathName\t\"d:/x/\"\r\n\
\x20   HairDataCount\t1\r\n\
\x20   Group HairData00\r\n\
\x20   {\r\n\
\x20       HairIndex\t0\r\n\
\x20       Model\t\"hair/hair_1_1.gr2\"\r\n\
\x20   }\r\n\
}\r\n\
List HitEffectFileName\r\n\
{\r\n\
\x20   \"a b.mse\"\r\n\
\x20   0.5 \"X\"\r\n\
}\r\n";
    (script, text)
}

#[test]
fn the_writer_makes_exactly_this_text_and_it_reads_back() {
    let (script, text) = sample();
    let written = script::write(&script).unwrap();
    assert_eq!(
        written.escape_ascii().to_string(),
        text.escape_ascii().to_string()
    );
    assert_eq!(without_lines(&script::parse(text).unwrap()), script);
}

#[test]
fn the_writer_quotes_what_needs_it_and_refuses_what_cannot_be_written() {
    let mut script = Script::default();
    script.push_values("K", vec![bare("two words"), bare("{"), bare("")]);
    assert_eq!(
        script::write(&script).unwrap(),
        b"K\t\"two words\" \"{\" \"\"\r\n"
    );

    let refused = |script: Script| match script::write(&script) {
        Err(FormatError::Unsupported { detail, .. }) => detail,
        other => panic!("expected Unsupported, got {other:?}"),
    };
    let mut s = Script::default();
    s.push_values("K", vec![quoted("say \"hi\"")]);
    assert!(refused(s).contains("holds a quote or a line break"));
    let mut s = Script::default();
    s.push_values("two words", vec![]);
    assert!(refused(s).contains("is not a valid name"));
    let mut s = Script::default();
    s.push_values("Group", vec![quoted("A")]);
    assert!(refused(s).contains("would read back as a block"));
    let mut s = Script::default();
    s.push_list("L", vec![vec![bare("}")]]);
    assert!(refused(s).contains("lone brace"));
    let mut s = Script::default();
    s.push_list("L", vec![vec![]]);
    assert!(refused(s).contains("empty row"));
}

/// Race data with a shape in a special folder, two skins, a hair style,
/// two collision spheres, one on a bone, and an effect on a bone.
fn race_data() -> RaceData {
    RaceData {
        base_model: Some(RawStr::from("d:/ymir work/pc/sura/sura_novice.gr2")),
        attribute_file: None,
        motion_list: None,
        shapes: PartList {
            path: RawStr::from("d:/ymir work/pc/sura/"),
            entries: vec![
                PartEntry {
                    index: 6,
                    special_path: None,
                    model: RawStr::from("sura_saryeong.gr2"),
                    skins: vec![
                        Skin {
                            source: RawStr::from("sura_saryeong.dds"),
                            target: Some(RawStr::from("sura_eumyang.dds")),
                        },
                        Skin {
                            source: RawStr::from("sura_saryeong_cloak.dds"),
                            target: None,
                        },
                    ],
                },
                PartEntry {
                    index: 100,
                    special_path: Some(RawStr::from("d:/ymir work/monster/wolf/")),
                    model: RawStr::from("wolf.gr2"),
                    skins: Vec::new(),
                },
            ],
        },
        hairs: PartList {
            path: RawStr::from("d:/ymir work/pc/sura/"),
            entries: vec![PartEntry {
                index: 0,
                special_path: None,
                model: RawStr::from("hair/hair_1_1.gr2"),
                skins: vec![Skin {
                    source: RawStr::from("hair/hair_1_1.dds"),
                    target: Some(RawStr::from("sura_hair_01.dds")),
                }],
            }],
        },
        collisions: vec![
            Collision {
                collision_type: 1,
                attached: true,
                bone: RawStr::from("Bip01"),
                spheres: vec![Sphere {
                    radius: 40.0,
                    position: [0.0, 0.0, -20.0],
                }],
            },
            Collision {
                collision_type: 3,
                attached: false,
                bone: RawStr::from("Bip01"),
                spheres: vec![Sphere {
                    radius: 70.5,
                    position: [0.0, -10.0, 90.0],
                }],
            },
        ],
        effects: vec![AttachedEffect {
            attached: true,
            bone: RawStr::from("Bip01 Head"),
            file: RawStr::from("d:/ymir work/effect/background/loop.mse"),
            position: [0.0, 5.0, 10.0],
            rotation: Some([0.0, 0.0, 90.0]),
        }],
        smoke: None,
    }
}

#[test]
fn race_data_byte_for_byte() {
    let data = race_data();
    let text: &[u8] = b"ScriptType\tRaceDataScript\r\n\
BaseModelFileName\t\"d:/ymir work/pc/sura/sura_novice.gr2\"\r\n\
Group ShapeData\r\n\
{\r\n\
\x20   PathName\t\"d:/ymir work/pc/sura/\"\r\n\
\x20   ShapeDataCount\t2\r\n\
\x20   Group ShapeData00\r\n\
\x20   {\r\n\
\x20       ShapeIndex\t6\r\n\
\x20       Model\t\"sura_saryeong.gr2\"\r\n\
\x20       SourceSkin\t\"sura_saryeong.dds\"\r\n\
\x20       TargetSkin\t\"sura_eumyang.dds\"\r\n\
\x20       SourceSkin2\t\"sura_saryeong_cloak.dds\"\r\n\
\x20   }\r\n\
\x20   Group ShapeData01\r\n\
\x20   {\r\n\
\x20       SpecialPath\t\"d:/ymir work/monster/wolf/\"\r\n\
\x20       ShapeIndex\t100\r\n\
\x20       Model\t\"wolf.gr2\"\r\n\
\x20   }\r\n\
}\r\n\
Group HairData\r\n\
{\r\n\
\x20   PathName\t\"d:/ymir work/pc/sura/\"\r\n\
\x20   HairDataCount\t1\r\n\
\x20   Group HairData00\r\n\
\x20   {\r\n\
\x20       HairIndex\t0\r\n\
\x20       Model\t\"hair/hair_1_1.gr2\"\r\n\
\x20       SourceSkin\t\"hair/hair_1_1.dds\"\r\n\
\x20       TargetSkin\t\"sura_hair_01.dds\"\r\n\
\x20   }\r\n\
}\r\n\
Group AttachingData\r\n\
{\r\n\
\x20   AttachingDataCount\t3\r\n\
\x20   Group AttachingData00\r\n\
\x20   {\r\n\
\x20       AttachingDataType\t1\r\n\
\x20       isAttaching\t1\r\n\
\x20       AttachingBoneName\t\"Bip01\"\r\n\
\x20       CollisionType\t1\r\n\
\x20       SphereDataCount\t1\r\n\
\x20       Group SphereData00\r\n\
\x20       {\r\n\
\x20           Radius\t40.000000\r\n\
\x20           Position\t0.000000 0.000000 -20.000000\r\n\
\x20       }\r\n\
\x20   }\r\n\
\x20   Group AttachingData01\r\n\
\x20   {\r\n\
\x20       AttachingDataType\t1\r\n\
\x20       isAttaching\t0\r\n\
\x20       AttachingBoneName\t\"Bip01\"\r\n\
\x20       CollisionType\t3\r\n\
\x20       SphereDataCount\t1\r\n\
\x20       Group SphereData00\r\n\
\x20       {\r\n\
\x20           Radius\t70.500000\r\n\
\x20           Position\t0.000000 -10.000000 90.000000\r\n\
\x20       }\r\n\
\x20   }\r\n\
\x20   Group AttachingData02\r\n\
\x20   {\r\n\
\x20       AttachingDataType\t2\r\n\
\x20       isAttaching\t1\r\n\
\x20       AttachingBoneName\t\"Bip01 Head\"\r\n\
\x20       EffectScriptName\t\"d:/ymir work/effect/background/loop.mse\"\r\n\
\x20       EffectPosition\t0.000000 5.000000 10.000000\r\n\
\x20       EffectRotation\t0.000000 0.000000 90.000000\r\n\
\x20   }\r\n\
}\r\n";
    assert_eq!(
        data.write().unwrap().escape_ascii().to_string(),
        text.escape_ascii().to_string()
    );
    assert_eq!(RaceData::parse(text).unwrap(), data);
}

#[test]
fn collision_spheres_in_the_clients_style() {
    // A monster's script as the client writes them: an effect block (type
    // 2) among the collision blocks, keys openm2 does not read, blocks out
    // of order.
    let text = b"ScriptType            RaceDataScript\r\n\
BaseModelFileName     \"d:/ymir work/monster/stray_dog/stray_dog.gr2\"\r\n\
Group AttachingData\r\n\
{\r\n\
\x20   AttachingDataCount       3\r\n\
\x20   Group AttachingData01\r\n\
\x20   {\r\n\
\x20       AttachingDataType    2\r\n\
\x20       isAttaching          1\r\n\
\x20       AttachingModelIndex  0\r\n\
\x20       AttachingBoneName    \"Bip01\"\r\n\
\x20       EffectScriptName     \"D:\\Ymir Work\\effect\\background\\loop.mse\"\r\n\
\x20       EffectPosition       -30.000000 0.000000 70.000000\r\n\
\x20       EffectRotation       0.000000 0.000000 0.000000\r\n\
\x20   }\r\n\
\x20   Group AttachingData00\r\n\
\x20   {\r\n\
\x20       AttachingDataType    1\r\n\
\x20       \r\n\
\x20       isAttaching          0\r\n\
\x20       AttachingModelIndex  0\r\n\
\x20       AttachingBoneName    \"Bip01\"\r\n\
\x20       \r\n\
\x20       CollisionType        1\r\n\
\x20       \r\n\
\x20       SphereDataCount      1\r\n\
\x20       Group SphereData00\r\n\
\x20       {\r\n\
\x20           Radius           60.000000\r\n\
\x20           Position         0.000000 -10.000000 100.000000\r\n\
\x20       }\r\n\
\x20   }\r\n\
\x20   Group AttachingData02\r\n\
\x20   {\r\n\
\x20       AttachingDataType    1\r\n\
\x20       CollisionType        3\r\n\
\x20       SphereDataCount      1\r\n\
\x20       Group SphereData00\r\n\
\x20       {\r\n\
\x20           Radius           90.000000\r\n\
\x20       }\r\n\
\x20   }\r\n\
}\r\n";
    let data = RaceData::parse(text).unwrap();
    assert_eq!(
        data.effects,
        [AttachedEffect {
            attached: true,
            bone: RawStr::from("Bip01"),
            file: RawStr::from("D:\\Ymir Work\\effect\\background\\loop.mse"),
            position: [-30.0, 0.0, 70.0],
            rotation: Some([0.0; 3]),
        }]
    );
    // A block without isAttaching, bone or position is placed at the
    // model's origin.
    assert_eq!(
        data.collisions,
        [
            Collision {
                collision_type: 1,
                attached: false,
                bone: RawStr::from("Bip01"),
                spheres: vec![Sphere {
                    radius: 60.0,
                    position: [0.0, -10.0, 100.0],
                }],
            },
            Collision {
                collision_type: 3,
                attached: false,
                bone: RawStr::from(""),
                spheres: vec![Sphere {
                    radius: 90.0,
                    position: [0.0, 0.0, 0.0],
                }],
            },
        ]
    );
}

#[test]
fn metin_stones_smoke_by_stage() {
    let text = b"ScriptType            RaceDataScript\r\n\
BaseModelFileName     \"D:\\Ymir Work\\monster\\metinstone\\metinstone_01.GR2\"\r\n\
SmokeBoneName \t\"ChamferCyl04\"\r\n\
List SmokeFileName\t\r\n\
{\r\n\
\t0\t\t\"D:\\Ymir Work\\effect\\background\\metinstone_loop_1.mse\"\r\n\
\t1\t\t\"D:\\Ymir Work\\effect\\background\\metinstone_loop_2.mse\"\r\n\
}\r\n\
List DefaultHitEffectFileName\r\n\
{\r\n\
}\r\n";
    let data = RaceData::parse(text).unwrap();
    let smoke = Smoke {
        bone: RawStr::from("ChamferCyl04"),
        stages: vec![
            (
                0,
                RawStr::from("D:\\Ymir Work\\effect\\background\\metinstone_loop_1.mse"),
            ),
            (
                1,
                RawStr::from("D:\\Ymir Work\\effect\\background\\metinstone_loop_2.mse"),
            ),
        ],
    };
    assert_eq!(data.smoke.as_ref(), Some(&smoke));
    assert_eq!(RaceData::parse(&data.write().unwrap()).unwrap(), data);
    assert_eq!(
        RaceData::parse(b"ScriptType RaceDataScript\r\n")
            .unwrap()
            .smoke,
        None
    );
    let bad =
        b"ScriptType RaceDataScript\r\nList SmokeFileName\r\n{\r\n    first \"a.mse\"\r\n}\r\n";
    let error = RaceData::parse(bad).unwrap_err().to_string();
    assert!(error.contains("not a stage number"), "{error}");
}

#[test]
fn part_names_resolve_in_their_folder() {
    let data = race_data();
    let shape = data.shapes.find(6).unwrap();
    assert_eq!(
        data.shapes.resolve(shape, &shape.model),
        b"d:/ymir work/pc/sura/sura_saryeong.gr2"
    );
    let wolf = data.shapes.find(100).unwrap();
    assert_eq!(
        data.shapes.resolve(wolf, &wolf.model),
        b"d:/ymir work/monster/wolf/wolf.gr2"
    );
    assert!(data.shapes.find(7).is_none());
}

#[test]
fn race_data_reads_blocks_by_count_and_ignores_other_keys() {
    // Keys in another case, blocks out of order, keys openm2 does not use,
    // a block past the count, a monster without hair and no shapes.
    let text = b"ScriptType RaceDataScript\n\
BaseModelFileName \"d:/ymir work/monster/wolf/wolf.gr2\"\n\
MotionListFileName \"motlist_range.txt\"\n\
Group ShapeData\n{\n\
pathname \"\"\n\
ShapeDataCount 1\n\
Group ShapeData01\n{\nShapeIndex 9\nModel \"unused.gr2\"\n}\n\
Group ShapeData00\n{\nShapeIndex 0\nmodel \"a.gr2\"\nLOCAL_Model \"x.gr2\"\n}\n\
}\n\
Group AttachingData\n{\nAttachingDataCount 0\n}\n";
    let data = RaceData::parse(text).unwrap();
    assert_eq!(data.motion_list, Some(RawStr::from("motlist_range.txt")));
    assert_eq!(data.shapes.path, RawStr::from(""));
    assert_eq!(data.shapes.entries.len(), 1);
    assert_eq!(data.shapes.entries[0].model, RawStr::from("a.gr2"));
    assert!(data.hairs.entries.is_empty());
}

#[test]
fn incomplete_race_data_is_refused() {
    let shape = |body: &str| {
        format!(
            "ScriptType RaceDataScript\nGroup ShapeData\n{{\nShapeDataCount 1\nGroup ShapeData00\n{{\n{body}\n}}\n}}\n"
        )
    };
    assert_eq!(
        damaged(RaceData::parse(shape("ShapeIndex 0").as_bytes())),
        "ShapeData00 has no Model"
    );
    assert_eq!(
        damaged(RaceData::parse(shape("Model \"a.gr2\"").as_bytes())),
        "ShapeData00 has no ShapeIndex"
    );
    assert_eq!(
        damaged(RaceData::parse(
            shape("ShapeIndex x\nModel \"a\"").as_bytes()
        )),
        "line 7: ShapeIndex: `x` is not a number"
    );
    assert!(matches!(
        RaceData::parse(shape("ShapeIndex 0\nLOCAL_Model \"a.gr2\"").as_bytes()),
        Err(FormatError::Unsupported { .. })
    ));
    assert_eq!(
        damaged(RaceData::parse(
            b"ScriptType RaceDataScript\nGroup HairData\n{\nHairDataCount 2\nGroup HairData00\n{\nHairIndex 0\nModel \"a\"\n}\n}\n"
        )),
        "HairDataCount is 2, but there is no block HairData01"
    );
    assert!(matches!(
        RaceData::parse(b"ScriptType RaceDataScript\nGroup HairData\n{\nHairDataCount 5000\n}\n"),
        Err(FormatError::TooLarge { size: 5000, .. })
    ));
    let attaching = |body: &str| {
        format!(
            "ScriptType RaceDataScript\nGroup AttachingData\n{{\nAttachingDataCount 1\nGroup AttachingData00\n{{\nAttachingDataType 1\n{body}\n}}\n}}\n"
        )
    };
    assert_eq!(
        damaged(RaceData::parse(attaching("SphereDataCount 0").as_bytes())),
        "AttachingData00 has no CollisionType"
    );
    assert_eq!(
        damaged(RaceData::parse(
            attaching(
                "CollisionType 1\nSphereDataCount 1\nGroup SphereData00\n{\nPosition 0 0 0\n}"
            )
            .as_bytes()
        )),
        "AttachingData00 has no Radius in SphereData00"
    );
    assert_eq!(
        damaged(RaceData::parse(
            attaching("CollisionType 1\nSphereDataCount 2\nGroup SphereData00\n{\nRadius 1\n}")
                .as_bytes()
        )),
        "SphereDataCount is 2, but there is no block SphereData01"
    );
    assert_eq!(
        damaged(RaceData::parse(
            b"ScriptType RaceDataScript\nGroup AttachingData\n{\nAttachingDataCount 1\n}\n"
        )),
        "AttachingDataCount is 1, but there is no block AttachingData00"
    );
    assert!(matches!(
        RaceData::parse(attaching("CollisionType 1\nSphereDataCount 300").as_bytes()),
        Err(FormatError::TooLarge { size: 300, .. })
    ));
    assert!(matches!(
        RaceData::parse(b"ScriptType MotionData\n"),
        Err(FormatError::UnknownLayout {
            known: race::SCRIPT_TYPE,
            ..
        })
    ));
    assert!(matches!(
        RaceData::parse(b"BaseModelFileName \"a\"\n"),
        Err(FormatError::UnknownLayout { .. })
    ));
}

#[test]
fn more_than_two_skins_cannot_be_written() {
    let mut data = race_data();
    data.hairs.entries[0].skins = vec![Skin::default(); 3];
    assert!(matches!(data.write(), Err(FormatError::Unsupported { .. })));
}

#[test]
fn motion_data_byte_for_byte() {
    let data = MotionData {
        motion_file: RawStr::from("d:/ymir work/pc/warrior/general/run.gr2"),
        duration: Some(0.5),
        accumulation: Some([0.0, -131.5, 0.0]),
        ..MotionData::default()
    };
    let text: &[u8] = b"ScriptType\tMotionData\r\n\
MotionFileName\t\"d:/ymir work/pc/warrior/general/run.gr2\"\r\n\
MotionDuration\t0.500000\r\n\
Accumulation\t0.000000 -131.500000 0.000000\r\n";
    assert_eq!(
        data.write().unwrap().escape_ascii().to_string(),
        text.escape_ascii().to_string()
    );
    assert_eq!(MotionData::parse(text).unwrap(), data);
}

#[test]
fn motion_data_in_the_clients_style_and_its_errors() {
    let data = MotionData::parse(
        b"ScriptType               MotionData\r\n\r\nMotionFileName           \"a.gr2\"\r\nMotionDuration           0.833333\r\nAccumulation             0.00\t-131.76\t0.00\r\nCancelEnableSkill        0\r\n",
    )
    .unwrap();
    assert_eq!(data.motion_file, RawStr::from("a.gr2"));
    assert_eq!(data.duration, Some(0.833333));
    assert_eq!(data.accumulation, Some([0.0, -131.76, 0.0]));
    assert_eq!(
        (data.combo, data.attacking, data.events),
        (None, None, vec![])
    );

    assert_eq!(
        damaged(MotionData::parse(
            b"ScriptType MotionData\nMotionDuration 1\n"
        )),
        "no MotionFileName"
    );
    assert_eq!(
        damaged(MotionData::parse(
            b"ScriptType MotionData\nMotionFileName D:\\Ymir Work\\a.gr2\"\n"
        )),
        "line 2: MotionFileName: expected one value, found 2"
    );
    assert_eq!(
        damaged(MotionData::parse(
            b"ScriptType MotionData\nMotionFileName \"a\"\nAccumulation 1 2\n"
        )),
        "line 3: Accumulation: expected 3 numbers, found 2"
    );
    assert!(matches!(
        MotionData::parse(b"ScriptType RaceDataScript\n"),
        Err(FormatError::UnknownLayout {
            known: motion::SCRIPT_TYPE,
            ..
        })
    ));
}

fn attack_motion() -> MotionData {
    MotionData {
        motion_file: RawStr::from("d:/ymir work/pc/warrior/onehand_sword/combo_01.gr2"),
        duration: Some(1.0),
        accumulation: Some([0.0, -131.75, 0.0]),
        combo: Some(ComboInput {
            pre_input: 0.25,
            direct_input: 0.5,
            input_limit: 0.625,
            link: Some(0.0625),
        }),
        attacking: Some(Attacking {
            attack_type: Some(0),
            attacking_type: None,
            motion_type: Some(2),
            hitting_type: 2,
            stiffen_time: 0.0,
            invisible_time: 0.1,
            external_force: 8.0,
            hit_limit: Some(0),
            hits: vec![Hit {
                start: 0.1875,
                end: 0.3125,
                bone: RawStr::from("equip_right_hand"),
                weapon_length: 130.0,
                positions: vec![
                    HitPosition {
                        time: 0.1875,
                        from: [0.0, -10.0, 100.0],
                        to: [0.0, -140.0, 100.0],
                    },
                    HitPosition {
                        time: 0.25,
                        from: [10.0, -10.0, 100.0],
                        to: [130.0, -60.0, 100.0],
                    },
                ],
            }],
        }),
        events: vec![
            MotionEvent {
                time: 0.5,
                kind: EventKind::Effect(Effect {
                    file: RawStr::from("d:/ymir work/effect/a.mse"),
                    position: [0.0, 0.0, 0.0],
                    bone: Some(RawStr::from("equip_right_hand")),
                    following: true,
                    independent: Some(false),
                    fishing: None,
                }),
            },
            MotionEvent {
                time: 0.625,
                kind: EventKind::Shake(Shake {
                    during: 0.2,
                    power: 300.0,
                    range: 200.0,
                }),
            },
            MotionEvent {
                time: 0.65,
                kind: EventKind::Attack(AttackEvent {
                    during: 0.25,
                    enable_hit: Some(true),
                    attack_type: Some(0),
                    hitting_type: 1,
                    stiffen_time: 0.0,
                    invisible_time: 0.3,
                    external_force: 17.0,
                    hit_limit: Some(0),
                    collision_type: 4,
                    spheres: vec![Sphere {
                        radius: 100.0,
                        position: [0.0, -120.0, 0.0],
                    }],
                }),
            },
            MotionEvent {
                time: 0.75,
                kind: EventKind::Fly(Fly {
                    file: RawStr::from("d:/ymir work/pc/assassin/effect/arrow_01.msf"),
                    position: [0.0, 0.0, 0.0],
                    bone: Some(RawStr::from("equip_box")),
                }),
            },
            MotionEvent {
                time: 0.875,
                kind: EventKind::SpotEffect(Effect {
                    file: RawStr::from("d:/ymir work/effect/b.mse"),
                    position: [0.0, 0.0, 10.0],
                    bone: None,
                    following: false,
                    independent: None,
                    fishing: Some(false),
                }),
            },
            MotionEvent {
                time: 0.9375,
                kind: EventKind::Other(7),
            },
        ],
    }
}

#[test]
fn attack_motion_byte_for_byte() {
    let text: &[u8] = b"ScriptType\tMotionData\r\n\
MotionFileName\t\"d:/ymir work/pc/warrior/onehand_sword/combo_01.gr2\"\r\n\
MotionDuration\t1.000000\r\n\
Accumulation\t0.000000 -131.750000 0.000000\r\n\
Group ComboInputData\r\n\
{\r\n\
\x20   PreInputTime\t0.250000\r\n\
\x20   DirectInputTime\t0.500000\r\n\
\x20   InputLimitTime\t0.625000\r\n\
\x20   LinkTime\t0.062500\r\n\
}\r\n\
Group AttackingData\r\n\
{\r\n\
\x20   AttackType\t0\r\n\
\x20   HittingType\t2\r\n\
\x20   StiffenTime\t0.000000\r\n\
\x20   InvisibleTime\t0.100000\r\n\
\x20   ExternalForce\t8.000000\r\n\
\x20   HitLimitCount\t0\r\n\
\x20   MotionType\t2\r\n\
\x20   HitDataCount\t1\r\n\
\x20   Group HitData00\r\n\
\x20   {\r\n\
\x20       AttackingStartTime\t0.187500\r\n\
\x20       AttackingEndTime\t0.312500\r\n\
\x20       AttackingBone\t\"equip_right_hand\"\r\n\
\x20       WeaponLength\t130.000000\r\n\
\x20       List HitPosition\r\n\
\x20       {\r\n\
\x20           0.187500 0.000000 -10.000000 100.000000 0.000000 -140.000000 100.000000\r\n\
\x20           0.250000 10.000000 -10.000000 100.000000 130.000000 -60.000000 100.000000\r\n\
\x20       }\r\n\
\x20   }\r\n\
}\r\n\
Group MotionEventData\r\n\
{\r\n\
\x20   MotionEventDataCount\t6\r\n\
\x20   Group Event00\r\n\
\x20   {\r\n\
\x20       MotionEventType\t1\r\n\
\x20       StartingTime\t0.500000\r\n\
\x20       IndependentFlag\t0\r\n\
\x20       AttachingEnable\t1\r\n\
\x20       AttachingBoneName\t\"equip_right_hand\"\r\n\
\x20       FollowingEnable\t1\r\n\
\x20       EffectFileName\t\"d:/ymir work/effect/a.mse\"\r\n\
\x20       EffectPosition\t0.000000 0.000000 0.000000\r\n\
\x20   }\r\n\
\x20   Group Event01\r\n\
\x20   {\r\n\
\x20       MotionEventType\t2\r\n\
\x20       StartingTime\t0.625000\r\n\
\x20       DuringTime\t0.200000\r\n\
\x20       Power\t300.000000\r\n\
\x20       AffectingRange\t200.000000\r\n\
\x20   }\r\n\
\x20   Group Event02\r\n\
\x20   {\r\n\
\x20       MotionEventType\t4\r\n\
\x20       StartingTime\t0.650000\r\n\
\x20       DuringTime\t0.250000\r\n\
\x20       EnableHitProcess\t1\r\n\
\x20       AttackType\t0\r\n\
\x20       HittingType\t1\r\n\
\x20       StiffenTime\t0.000000\r\n\
\x20       InvisibleTime\t0.300000\r\n\
\x20       ExternalForce\t17.000000\r\n\
\x20       HitLimitCount\t0\r\n\
\x20       CollisionType\t4\r\n\
\x20       SphereDataCount\t1\r\n\
\x20       Group SphereData00\r\n\
\x20       {\r\n\
\x20           Radius\t100.000000\r\n\
\x20           Position\t0.000000 -120.000000 0.000000\r\n\
\x20       }\r\n\
\x20   }\r\n\
\x20   Group Event03\r\n\
\x20   {\r\n\
\x20       MotionEventType\t6\r\n\
\x20       StartingTime\t0.750000\r\n\
\x20       AttachingEnable\t1\r\n\
\x20       AttachingBoneName\t\"equip_box\"\r\n\
\x20       FlyFileName\t\"d:/ymir work/pc/assassin/effect/arrow_01.msf\"\r\n\
\x20       FlyPosition\t0.000000 0.000000 0.000000\r\n\
\x20   }\r\n\
\x20   Group Event04\r\n\
\x20   {\r\n\
\x20       MotionEventType\t10\r\n\
\x20       StartingTime\t0.875000\r\n\
\x20       FollowingEnable\t0\r\n\
\x20       EffectFileName\t\"d:/ymir work/effect/b.mse\"\r\n\
\x20       EffectPosition\t0.000000 0.000000 10.000000\r\n\
\x20       FishingEffectFlag\t0\r\n\
\x20   }\r\n\
\x20   Group Event05\r\n\
\x20   {\r\n\
\x20       MotionEventType\t7\r\n\
\x20       StartingTime\t0.937500\r\n\
\x20   }\r\n\
}\r\n";
    let data = attack_motion();
    assert_eq!(
        data.write().unwrap().escape_ascii().to_string(),
        text.escape_ascii().to_string()
    );
    assert_eq!(MotionData::parse(text).unwrap(), data);
}

#[test]
fn the_older_attack_layout_has_one_hit_in_the_group() {
    // The client's spacing; a fist: the bone and the "tip" are one point.
    let text = b"ScriptType               MotionData\r\n\
MotionFileName           \"D:\\\\Ymir Work\\\\pc\\\\warrior\\\\general\\\\attack.GR2\"\r\n\
MotionDuration           1.000000\r\n\
\r\n\
Group AttackingData\r\n\
{\r\n\
\x20   AttackingType        1\r\n\
\x20   HittingType          2\r\n\
\x20   StiffenTime          0.000000\r\n\
\x20   InvisibleTime        0.500000\r\n\
\x20   ExternalForce        0.000000\r\n\
\x20   \r\n\
\x20   AttackingStartTime   0.450000\r\n\
\x20   AttackingEndTime     0.600000\r\n\
\x20   AttackingBone        \"equip_right_hand\"\r\n\
\x20   WeaponLength         0.000000\r\n\
\x20   List HitPosition\r\n\
\x20   {\r\n\
\x20       0.450000\t1.000000\t40.000000\t100.000000\t1.000000\t40.000000\t100.000000\r\n\
\x20   }\r\n\
}\r\n";
    let data = MotionData::parse(text).unwrap();
    let attacking = data.attacking.clone().unwrap();
    assert_eq!(
        (
            attacking.attacking_type,
            attacking.attack_type,
            attacking.motion_type
        ),
        (Some(1), None, None)
    );
    let [hit] = attacking.hits.as_slice() else {
        panic!("one hit: {:?}", attacking.hits)
    };
    assert_eq!((hit.start, hit.end, hit.weapon_length), (0.45, 0.6, 0.0));
    assert_eq!(hit.bone, RawStr::from("equip_right_hand"));
    assert_eq!(hit.positions[0].from, hit.positions[0].to);
    // Written back in the same layout.
    let written = String::from_utf8(data.write().unwrap()).unwrap();
    assert!(
        written.contains("\r\n    AttackingType\t1\r\n"),
        "{written}"
    );
    assert!(
        written.contains("\r\n    AttackingStartTime\t0.450000\r\n"),
        "{written}"
    );
    assert!(!written.contains("HitData"), "{written}");
    assert_eq!(MotionData::parse(written.as_bytes()).unwrap(), data);

    let mut two = data;
    if let Some(attacking) = &mut two.attacking {
        attacking.hits.push(Hit::default());
    }
    assert!(matches!(two.write(), Err(FormatError::Unsupported { .. })));
}

#[test]
fn incomplete_attack_data_is_refused() {
    let with = |from: &str, to: &str| {
        let text = String::from_utf8(attack_motion().write().unwrap()).unwrap();
        assert!(text.contains(from), "{from}");
        MotionData::parse(text.replacen(from, to, 1).as_bytes())
    };
    assert_eq!(
        damaged(with("    DirectInputTime\t0.500000\r\n", "")),
        "ComboInputData has no DirectInputTime"
    );
    assert_eq!(
        damaged(with("HitDataCount\t1", "HitDataCount\t2")),
        "AttackingData: HitDataCount is 2, but there is no block HitData01"
    );
    assert_eq!(
        damaged(with("        AttackingEndTime\t0.312500\r\n", "")),
        "AttackingData.HitData00 has no AttackingEndTime"
    );
    assert_eq!(
        damaged(with("        MotionEventType\t2\r\n", "")),
        "MotionEventData.Event01 has no MotionEventType"
    );
    assert_eq!(
        damaged(with("            Radius\t100.000000\r\n", "")),
        "MotionEventData.Event02.SphereData00 has no Radius"
    );
    assert!(matches!(
        with("MotionEventDataCount\t6", "MotionEventDataCount\t300"),
        Err(FormatError::TooLarge { size: 300, .. })
    ));
    // Some flags are written as numbers with decimals.
    let parsed = with("EnableHitProcess\t1", "EnableHitProcess\t0.000000").unwrap();
    let EventKind::Attack(attack) = &parsed.events[2].kind else {
        panic!("{:?}", parsed.events[2])
    };
    assert_eq!(attack.enable_hit, Some(false));
    // Key case does not matter (some scripts write `flyfilename`).
    let parsed = with("FlyFileName", "flyfilename").unwrap();
    assert_eq!(parsed, attack_motion());
    // A hit position that is not seven numbers is left out.
    let parsed = with(
        " 0.000000 -140.000000 100.000000\r\n",
        " 0.000000 -140.000000 100.000000x\r\n",
    )
    .unwrap();
    let positions = &parsed.attacking.unwrap().hits[0].positions;
    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].time, 0.25);
}

#[test]
fn forward_speed_is_the_accumulation_over_the_duration() {
    let run = MotionData {
        duration: Some(0.8),
        accumulation: Some([30.0, -40.0, 7.0]),
        ..MotionData::default()
    };
    // 50 cm along the ground in 0.8 s; the animation's length is not needed.
    assert_eq!(run.forward_speed(Some(2.0)), Some(62.5));
    assert_eq!(run.forward_speed(None), Some(62.5));
    let untimed = MotionData {
        duration: None,
        ..run.clone()
    };
    assert_eq!(untimed.forward_speed(Some(0.5)), Some(100.0));
    assert_eq!(untimed.forward_speed(None), None);
    let still = MotionData {
        accumulation: Some([0.0, 0.0, 5.0]),
        ..run.clone()
    };
    assert_eq!(still.forward_speed(Some(1.0)), None);
    assert_eq!(MotionData::default().forward_speed(Some(1.0)), None);
}

#[test]
fn hits_sweep_over_their_time_and_reach() {
    let data = attack_motion();
    let hit = &data.attacking.as_ref().unwrap().hits[0];
    assert!(hit.strikes());
    assert!(!Hit::default().strikes(), "no time, no hit");
    assert_eq!(hit.segment_at(0.1), None, "before the hit");
    assert_eq!(hit.segment_at(0.32), None, "after it");
    assert_eq!(
        hit.segment_at(0.1875),
        Some(([0.0, -10.0, 100.0], [0.0, -140.0, 100.0]))
    );
    // Halfway between the two rows.
    assert_eq!(
        hit.segment_at(0.21875),
        Some(([5.0, -10.0, 100.0], [65.0, -100.0, 100.0]))
    );
    // Past the last row, within the hit: the last row.
    assert_eq!(
        hit.segment_at(0.3),
        Some(([10.0, -10.0, 100.0], [130.0, -60.0, 100.0]))
    );
    assert_eq!(data.hit_times(), vec![(0.1875, 0.3125), (0.65, 0.9)]);
    // The farthest: the sphere 120 cm ahead with its 100 cm radius.
    assert_eq!(data.reach(), Some(220.0));
    let mut no_events = data.clone();
    no_events.events.clear();
    // Without it: the tip of the second row, (130, −60).
    let reach = no_events.reach().unwrap();
    assert!((reach - 130f32.hypot(60.0)).abs() < 1e-4, "{reach}");
    assert_eq!(MotionData::default().reach(), None);

    let combo = data.combo.unwrap();
    assert!(!combo.accepts(0.2));
    assert!(combo.accepts(0.25) && combo.accepts(0.625));
    assert!(!combo.accepts(0.7));
}

fn list_entry(name: &str, file: &str, weight: u32) -> MotionListEntry {
    MotionListEntry {
        mode: RawStr::from("GENERAL"),
        name: RawStr::from(name),
        file: RawStr::from(file),
        weight,
    }
}

#[test]
fn a_motion_list_byte_for_byte() {
    let entries = vec![
        list_entry("WAIT", "00.msa", 65),
        list_entry("WAIT1", "00_1.msa", 35),
    ];
    let text: &[u8] = b"GENERAL WAIT 00.msa 65\r\nGENERAL WAIT1 00_1.msa 35\r\n";
    assert_eq!(motion::write_list(&entries).unwrap(), text);
    assert_eq!(motion::parse_list(text).unwrap(), entries);
    // Tabs, bare line feeds and blank lines read the same.
    assert_eq!(
        motion::parse_list(b"\nGENERAL\tWAIT\t00.msa\t65\n\n  GENERAL WAIT1 00_1.msa 35").unwrap(),
        entries
    );
}

#[test]
fn bad_motion_list_lines_are_refused() {
    assert_eq!(
        damaged(motion::parse_list(
            b"GENERAL WAIT 00.msa 65\nGENERAL RUN 03.msa\n"
        )),
        "line 2: expected mode, name, file and weight, found 3 values"
    );
    assert_eq!(
        damaged(motion::parse_list(b"GENERAL WAIT 00.msa often\n")),
        "line 1: the weight `often` is not a number"
    );
    assert!(motion::write_list(&[list_entry("TWO WORDS", "a.msa", 1)]).is_err());
}

#[test]
fn numbered_variants_belong_to_their_motion() {
    let list = vec![
        list_entry("NORMAL_ATTACK", "20.msa", 50),
        list_entry("normal_attack1", "20_1.msa", 50),
        list_entry("NORMAL_ATTACKER", "21.msa", 100),
        list_entry("FRONT_DAMAGE", "30.msa", 100),
    ];
    let files: Vec<_> = motion::variants(&list, "NORMAL_ATTACK")
        .into_iter()
        .map(|e| e.file.to_string())
        .collect();
    assert_eq!(files, ["20.msa", "20_1.msa"]);
    assert!(list[3].is_variant_of("front_damage"));
    assert!(!list[3].is_variant_of("FRONT"));
}

#[test]
fn motion_scripts_are_found_beside_their_list() {
    let entry = list_entry("RUN", "03.msa", 100);
    assert_eq!(
        entry.script_path(b"d:/ymir work/monster/wolf/motlist.txt"),
        b"d:/ymir work/monster/wolf/03.msa"
    );
    assert_eq!(
        entry.script_path(b"d:\\npc\\motlist.txt"),
        b"d:\\npc\\03.msa"
    );
    assert_eq!(entry.script_path(b"motlist.txt"), b"03.msa");
}

fn npc(vnum: u32, name: &str, folder: Option<&str>) -> NpcLine {
    NpcLine {
        vnum,
        name: RawStr::from(name),
        folder: folder.map(RawStr::from),
    }
}

#[test]
fn a_monster_list_byte_for_byte() {
    let lines = vec![
        npc(101, "stray_dog", None),
        npc(104, "wolf_blue", None),
        npc(0, "wolf_blue", Some("wolf")),
        npc(693, "#season1/monster/orc_lord_e/", None),
    ];
    let text: &[u8] = b"101\tstray_dog\r\n104\twolf_blue\r\n0\twolf_blue\twolf\r\n\
693\t#season1/monster/orc_lord_e/\r\n";
    assert_eq!(npc_list::write(&lines).unwrap(), text);
    assert_eq!(npc_list::parse(text).unwrap(), lines);
    // The client's style: a space before a tab, a tab at the end, blank
    // lines.
    assert_eq!(
        npc_list::parse(b"101\tstray_dog\t\n\n104 wolf_blue \t\r\n0\twolf_blue \twolf\n693\t#season1/monster/orc_lord_e/\t").unwrap(),
        lines
    );
}

#[test]
fn monsters_find_their_model() {
    let lines = vec![
        npc(101, "stray_dog", None),
        npc(104, "wolf_blue", None),
        npc(0, "WOLF_BLUE", Some("wolf")),
        npc(693, "#season1/monster/orc_lord_e/", None),
        npc(104, "#season1/monster/other/", None),
    ];
    assert_eq!(
        npc_list::find(&lines, 101),
        Some(NpcModel::Named {
            name: b"stray_dog",
            folder: b"stray_dog"
        })
    );
    // A folder line names the folder (in any case); the first line of a
    // number wins.
    assert_eq!(
        npc_list::find(&lines, 104),
        Some(NpcModel::Named {
            name: b"wolf_blue",
            folder: b"wolf"
        })
    );
    assert_eq!(
        npc_list::find(&lines, 693),
        Some(NpcModel::Path(b"season1/monster/orc_lord_e/"))
    );
    assert_eq!(npc_list::find(&lines, 5), None);
    assert_eq!(npc_list::find(&lines, 0), None);
}

#[test]
fn bad_monster_list_lines_are_refused() {
    assert_eq!(
        damaged(npc_list::parse(b"101 stray_dog\n102\n")),
        "line 2: expected a number and a name, found 1 values"
    );
    assert_eq!(
        damaged(npc_list::parse(b"wolf 102\n")),
        "line 1: `wolf` is not a monster number"
    );
    assert_eq!(
        damaged(npc_list::parse(b"102 wolf wolf_folder\n")),
        "line 1: monster 102 has a folder; only lines with number 0 name one"
    );
    assert!(npc_list::write(&[npc(1, "two words", None)]).is_err());
    assert!(npc_list::write(&[npc(1, "", None)]).is_err());
}

fn listed(vnum: u32, kind: &str, icon: &str, model: Option<&str>) -> ItemLine {
    ItemLine {
        vnum,
        kind: RawStr::from(kind),
        icon: RawStr::from(icon),
        model: model.map(RawStr::from),
    }
}

#[test]
fn an_item_list_byte_for_byte() {
    let lines = vec![
        listed(
            10,
            "WEAPON",
            "icon/item/00010.tga",
            Some("d:/ymir work/item/weapon/00010.gr2"),
        ),
        listed(8000, "WEAPON", "icon/item/08000.tga", None),
        listed(18000, "ETC", "icon/item/18000.tga", None),
    ];
    let text: &[u8] = b"10\tWEAPON\ticon/item/00010.tga\td:/ymir work/item/weapon/00010.gr2\n8000\tWEAPON\ticon/item/08000.tga\n18000\tETC\ticon/item/18000.tga\n";
    assert_eq!(item_list::write(&lines).unwrap(), text);
    assert_eq!(item_list::parse(text).unwrap(), lines);
    // The client's style: empty values at the end, `\r\n` in some files,
    // blank lines; paths keep their spaces.
    assert_eq!(
        item_list::parse(
            b"10\tWEAPON\ticon/item/00010.tga\td:/ymir work/item/weapon/00010.gr2\r\n\n8000\tWEAPON\t icon/item/08000.tga\n18000\tETC\ticon/item/18000.tga\t\t"
        )
        .unwrap(),
        lines
    );
    assert_eq!(item_list::find(&lines, 8000), lines.get(1));
    assert_eq!(item_list::find(&lines, 9), None);
}

#[test]
fn bad_item_list_lines_are_refused() {
    assert_eq!(
        damaged(item_list::parse(b"10\tWEAPON\ticon.tga\n11\tWEAPON\n")),
        "line 2: expected a number, a kind, an icon and maybe a model, found 2 values"
    );
    assert_eq!(
        damaged(item_list::parse(b"+10\tWEAPON\ticon.tga\n")),
        "line 1: `+10` is not an item number"
    );
    assert_eq!(
        damaged(item_list::parse(b"10\t\ticon.tga\n")),
        "line 1: a value in the middle is empty"
    );
    assert_eq!(
        damaged(item_list::parse(b"10\tA\tB\tC\tD\n")),
        "line 1: expected a number, a kind, an icon and maybe a model, found 5 values"
    );
    assert!(item_list::write(&[listed(1, "two\twords", "x.tga", None)]).is_err());
    assert!(item_list::write(&[listed(1, "ETC", "", None)]).is_err());
    assert!(item_list::write(&[listed(1, "ETC", " x.tga", None)]).is_err());
}

fn raw(values: &[&str]) -> Vec<RawStr> {
    values.iter().map(|v| RawStr::from(*v)).collect()
}

/// The client's Three-Way Cut and Leadership, shortened.
fn skill_lines() -> Vec<SkillLine> {
    vec![
        SkillLine {
            vnum: 1,
            job: RawStr::from("WARRIOR"),
            names: ["Three-Way Cut", "Triple Cut", "Triple Slash"].map(RawStr::from),
            description: RawStr::from("Cut the enemy with lightning speed."),
            notes: [
                "Frontal Slash Attack 3 times",
                "Attack on Multiple Targets",
                "",
                "",
            ]
            .map(RawStr::from),
            flags: raw(&["ATTACK_SKILL", "NEED_TARGET", "WEAPON_LIMITATION"]),
            weapons: raw(&["SWORD", "TWO_HANDED"]),
            icon: RawStr::from("samyeon"),
            motion: Some(1),
            rest: raw(&[
                "4",
                "",
                "",
                "Total Attack Power %.0f-%.0f",
                "(1.1*MinATK) * 3",
            ]),
        },
        SkillLine {
            vnum: 121,
            job: RawStr::from("SUPPORT"),
            names: ["Leadership", "", ""].map(RawStr::from),
            description: RawStr::from("Ability to lead partners."),
            flags: raw(&["PASSIVE"]),
            icon: RawStr::from("tongsol"),
            ..SkillLine::default()
        },
    ]
}

#[test]
fn skill_descriptions_byte_for_byte() {
    let lines = skill_lines();
    let text: &[u8] = b"1\tWARRIOR\tThree-Way Cut\tTriple Cut\tTriple Slash\tCut the enemy with lightning speed.\tFrontal Slash Attack 3 times\tAttack on Multiple Targets\t\t\tATTACK_SKILL|NEED_TARGET|WEAPON_LIMITATION\tSWORD|TWO_HANDED\tsamyeon\t1\t4\t\t\tTotal Attack Power %.0f-%.0f\t(1.1*MinATK) * 3\n121\tSUPPORT\tLeadership\t\t\tAbility to lead partners.\t\t\t\t\tPASSIVE\t\ttongsol\n";
    assert_eq!(skill_desc::write(&lines).unwrap(), text);
    assert_eq!(skill_desc::parse(text).unwrap(), lines);
    // The client's style: spaces after values, empty values at the end, a
    // blank line; `\r\n` is read too.
    let client: &[u8] = b"1\tWARRIOR\tThree-Way Cut \tTriple Cut\tTriple Slash\tCut the enemy with lightning speed.\tFrontal Slash Attack 3 times\tAttack on Multiple Targets\t\t\tATTACK_SKILL|NEED_TARGET|WEAPON_LIMITATION\tSWORD|TWO_HANDED\tsamyeon\t1\t4\t\t\tTotal Attack Power %.0f-%.0f\t(1.1*MinATK) * 3\t\t\r\n\n121\tSUPPORT\tLeadership\t\t\tAbility to lead partners.\t\t\t\t\tPASSIVE\t\ttongsol\t\t\t\t\t\t\t\t\t\n";
    assert_eq!(skill_desc::parse(client).unwrap(), lines);
    let samyeon = skill_desc::find(&lines, 1).unwrap();
    assert!(samyeon.has_flag("NEED_TARGET") && !samyeon.has_flag("PASSIVE"));
    assert_eq!(skill_desc::find(&lines, 2), None);
}

#[test]
fn bad_skill_description_lines_are_refused() {
    assert_eq!(
        damaged(skill_desc::parse(b"1\tWARRIOR\tCut\n2\tWARRIOR\n")),
        "line 2: expected at least a number, a job and a name, found 2 values"
    );
    assert_eq!(
        damaged(skill_desc::parse(b"x1\tWARRIOR\tCut\n")),
        "line 1: `x1` is not a skill number"
    );
    let bad_motion = b"1\tWARRIOR\tCut\t\t\t\t\t\t\t\t\t\tsamyeon\tone\n";
    assert_eq!(
        damaged(skill_desc::parse(bad_motion)),
        "line 1: `one` is not a motion number"
    );
    let mut line = skill_lines().remove(0);
    line.flags.push(RawStr::from("A|B"));
    assert!(skill_desc::write(&[line]).is_err());
    let mut line = skill_lines().remove(0);
    line.rest.push(RawStr::from(""));
    assert!(skill_desc::write(&[line]).is_err());
    let mut line = skill_lines().remove(0);
    line.description = RawStr::from("two\tlines");
    assert!(skill_desc::write(&[line]).is_err());
    let mut line = skill_lines().remove(0);
    line.names[0] = RawStr::from("");
    assert!(skill_desc::write(&[line]).is_err());
}

/// Two made-up skills: a melee attack and a buff with two points.
fn skill_table_lines() -> Vec<SkillTableLine> {
    let words = |words: &[&str]| words.iter().map(|w| RawStr::from(*w)).collect();
    vec![
        SkillTableLine {
            vnum: 1,
            name: RawStr::from("Cleave"),
            category: 1,
            level_step: 1,
            max_level: 1,
            level_limit: 0,
            point: RawStr::from("HP"),
            amount: RawStr::from("-(2*atk+10*k)"),
            sp_cost: RawStr::from("10+10*k"),
            cooldown: RawStr::from("5"),
            flags: words(&["ATTACK", "SPLASH"]),
            point2: RawStr::from("NONE"),
            kind: RawStr::from("MELEE"),
            max_hit: 3,
            splash_damage: RawStr::from("1"),
            splash_range: 300,
            ..SkillTableLine::default()
        },
        SkillTableLine {
            vnum: 3,
            // Two bytes of another codepage, kept as they are.
            name: RawStr::from_bytes(&[0xB0, 0xA1]),
            category: 1,
            level_step: 1,
            max_level: 1,
            level_limit: 5,
            point: RawStr::from("ATT_SPEED"),
            amount: RawStr::from("20*k"),
            sp_cost: RawStr::from("30+50*k"),
            duration: RawStr::from("60"),
            cooldown: RawStr::from("30"),
            flags: words(&["SELFONLY"]),
            point2: RawStr::from("MOV_SPEED"),
            amount2: RawStr::from("10*k"),
            duration2: RawStr::from("60"),
            prerequisite: 1,
            prerequisite_level: 2,
            kind: RawStr::from("NORMAL"),
            max_hit: 1,
            target_range: 2500,
            ..SkillTableLine::default()
        },
    ]
}

#[test]
fn skill_table_byte_for_byte() {
    let mut text: Vec<u8> = Vec::new();
    // vnum, name, category, level step, max level, level limit
    text.extend_from_slice(b"1\tCleave\t1\t1\t1\t0\t");
    // point, amount, sp cost, duration, duration sp cost, cooldown
    text.extend_from_slice(b"HP\t-(2*atk+10*k)\t10+10*k\t\t\t5\t");
    // master bonus, attack grade, flags, affect
    text.extend_from_slice(b"\t\tATTACK,SPLASH\t\t");
    // point 2, amount 2, duration 2, affect 2
    text.extend_from_slice(b"NONE\t\t\t\t");
    // prerequisite and its level, kind, max hit, splash damage, target
    // range, splash range
    text.extend_from_slice(b"0\t0\tMELEE\t3\t1\t0\t300\r\n");
    text.extend_from_slice(b"3\t\xB0\xA1\t1\t1\t1\t5\tATT_SPEED\t20*k\t30+50*k\t60\t\t30\t");
    text.extend_from_slice(
        b"\t\tSELFONLY\t\tMOV_SPEED\t10*k\t60\t\t1\t2\tNORMAL\t1\t\t2500\t0\r\n",
    );
    let lines = skill_table_lines();
    assert_eq!(skill_table::write(&lines).unwrap(), text);
    assert_eq!(skill_table::parse(&text).unwrap(), lines);
    for line in text.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        assert_eq!(line.split(|b| *b == b'\t').count(), skill_table::VALUES);
    }
    // A header line and a blank one are skipped, `\n` is read too, spaces
    // around values and values left out at the end are allowed.
    let client: &[u8] = b"Vnum\tName\r\n\r\n1\tCleave \t1\t1\t1\t0\tHP\t-(2*atk+10*k)\t10+10*k\t\t\t5\t\t\tATTACK, SPLASH\t\tNONE\t\t\t\t0\t0\tMELEE\t3\t1\t\t300\n3\t\xB0\xA1\t1\t1\t1\t5\tATT_SPEED\t20*k\t30+50*k\t60\t\t30\t\t\tSELFONLY\t\tMOV_SPEED\t10*k\t60\t\t1\t2\tNORMAL\t1\t\t2500\n";
    assert_eq!(skill_table::parse(client).unwrap(), lines);
    let cleave = skill_table::find(&lines, 1).unwrap();
    assert!(cleave.has_flag("SPLASH") && !cleave.has_flag("SELFONLY"));
    assert_eq!(skill_table::find(&lines, 2), None);
}

#[test]
fn bad_skill_table_lines_are_refused() {
    let long = format!("1{}\n", "\tx".repeat(27));
    assert_eq!(
        damaged(skill_table::parse(long.as_bytes())),
        "line 1: expected at most 27 values, found 28"
    );
    assert_eq!(
        damaged(skill_table::parse(b"1x\tCut\n")),
        "line 1: `1x` is not a skill number"
    );
    assert_eq!(
        damaged(skill_table::parse(b"\n1\tCut\tone\n")),
        "line 2: `one` is not a category"
    );
    let mut line = skill_table_lines().remove(0);
    line.flags.push(RawStr::from("A,B"));
    assert!(skill_table::write(&[line]).is_err());
    let mut line = skill_table_lines().remove(0);
    line.amount = RawStr::from("1\t2");
    assert!(skill_table::write(&[line]).is_err());
    let mut line = skill_table_lines().remove(0);
    line.name = RawStr::from(" Cleave");
    assert!(skill_table::write(&[line]).is_err());
}

/// A bare word: no whitespace, quote, or line break, not starting with a
/// brace.
fn bare_word() -> impl Strategy<Value = Token> {
    let byte = any::<u8>().prop_filter("a word byte", |b| {
        !b.is_ascii_whitespace() && !matches!(b, b'"' | b'\r' | b'\n')
    });
    prop::collection::vec(byte, 1..8)
        .prop_filter("not starting with a brace", |w| {
            !matches!(w[0], b'{' | b'}')
        })
        .prop_map(|w| Token::bare(RawStr::from_bytes(&w)))
}

fn quoted_text() -> impl Strategy<Value = Token> {
    let byte = any::<u8>().prop_filter("a quoted byte", |b| !matches!(b, b'"' | b'\r' | b'\n'));
    prop::collection::vec(byte, 0..8).prop_map(|t| Token::quoted(RawStr::from_bytes(&t)))
}

fn token() -> impl Strategy<Value = Token> {
    prop_oneof![bare_word(), quoted_text()]
}

/// Names that are bare words and never `Group` or `List`.
fn name() -> impl Strategy<Value = String> {
    "K[A-Za-z0-9_]{0,6}"
}

fn script_tree() -> impl Strategy<Value = Script> {
    let leaf = prop::collection::vec(
        (
            name(),
            prop_oneof![
                prop::collection::vec(token(), 0..4).prop_map(Body::Values),
                prop::collection::vec(prop::collection::vec(token(), 1..4), 0..3)
                    .prop_map(Body::List),
            ],
        ),
        0..4,
    );
    let to_script = |entries: Vec<(String, Body)>| Script {
        entries: entries
            .into_iter()
            .map(|(name, body)| Entry {
                line: 0,
                name: RawStr::from(name.as_str()),
                body,
            })
            .collect(),
    };
    leaf.prop_map(to_script)
        .prop_recursive(3, 24, 4, move |inner| {
            prop::collection::vec((name(), inner), 0..3).prop_map(move |groups| {
                to_script(
                    groups
                        .into_iter()
                        .map(|(name, group)| (name, Body::Group(group)))
                        .collect(),
                )
            })
        })
}

/// A made-up flying effect: a homing bolt with two attached effects, one
/// with a tail.
fn flying() -> FlyingData {
    FlyingData {
        spreading: false,
        maintain_parallel: true,
        initial_velocity: 3000.0,
        cone_angle: 0.0,
        roll_angle: 0.0,
        angular_velocity: [0.0, 1.0, 0.0],
        gravity: 0.0,
        hit_on_background: false,
        hit_on_another_monster: false,
        pierce_count: 0,
        collision_radius: 0.0,
        bomb_range: 20.0,
        bomb_effect: Some(RawStr::from("bolt_blow.mse")),
        homing: true,
        homing_start: 0.0,
        homing_max_angle: 180.0,
        range: 5000.0,
        acceleration: [0.0, -100.0, 0.0],
        ignore_z: false,
        attached: vec![
            FlyAttach {
                kind: 1,
                fly_type: 1,
                file: RawStr::from("d:/test/bolt.mse"),
                tail: Some(Tail {
                    colour: 1_726_079_457,
                    length: 0.18,
                    size: 1.0,
                    rect: false,
                }),
                roll: 0.0,
                distance: 0.0,
                period: 1.0,
                amplitude: 0.0,
            },
            FlyAttach {
                kind: 1,
                fly_type: 3,
                file: RawStr::from("spark.mse"),
                tail: None,
                roll: 90.0,
                distance: 10.0,
                period: 0.4,
                amplitude: 50.0,
            },
        ],
    }
}

#[test]
fn flying_effects_are_written_exactly() {
    let text = String::from_utf8(flying().write().unwrap()).unwrap();
    let expected = "SpreadingFlag\t0\r\nMaintainParallelFlag\t1\r\nInitialVelocity\t3000.000000\r\nConeAngle\t0.000000\r\nRollAngle\t0.000000\r\nAngularVelocity\t0.000000 1.000000 0.000000\r\nGravity\t0.000000\r\nHitOnBackground\t0\r\nHitOnAnotherMonster\t0\r\nPierceCount\t0\r\nCollisionSphereRadius\t0.000000\r\nBombRange\t20.000000\r\nBombEffect\t\"bolt_blow.mse\"\r\nHomingFlag\t1\r\nHomingStartTime\t0.000000\r\nHomingMaxAngle\t180.000000\r\nRange\t5000.000000\r\nAcceleration\t0.000000 -100.000000 0.000000\r\nGroup AttachData\r\n{\r\n    Type\t1\r\n    FlyType\t1\r\n    AttachFile\t\"d:/test/bolt.mse\"\r\n    TailFlag\t1\r\n    TailColor\t1726079457d\r\n    TailLength\t0.180000\r\n    TailSize\t1.000000\r\n    TailShapeRect\t0\r\n    Roll\t0.000000\r\n    Distance\t0.000000\r\n    Period\t1.000000\r\n    Amplitude\t0.000000\r\n}\r\nGroup AttachData\r\n{\r\n    Type\t1\r\n    FlyType\t3\r\n    AttachFile\t\"spark.mse\"\r\n    TailFlag\t0\r\n    Roll\t90.000000\r\n    Distance\t10.000000\r\n    Period\t0.400000\r\n    Amplitude\t50.000000\r\n}\r\n";
    assert_eq!(text, expected);
    assert_eq!(FlyingData::parse(text.as_bytes()).unwrap(), flying());
    let names: Vec<String> = flying().effects().map(ToString::to_string).collect();
    assert_eq!(names, ["d:/test/bolt.mse", "spark.mse", "bolt_blow.mse"]);
}

#[test]
fn flying_effects_read_the_clients_style() {
    // Aligned with spaces, keys left out (0), an empty bomb effect (none),
    // a colour without its d, a tail's keys after TailFlag 0 (not read).
    let text = b"InitialVelocity         -100.000000\r\nAcceleration            0.000000 -9000.000000 0.000000\r\nBombEffect              \"\"\r\nHomingFlag              0\r\nIgnoreZ                 1\r\nGroup AttachData\r\n{\r\n    Type          1\r\n    FlyType       2\r\n    AttachFile    \"a.mse\"\r\n    TailFlag      1\r\n    TailColor     524032\r\n}\r\nGroup AttachData\r\n{\r\n    TailFlag      0\r\n    TailColor     1d\r\n}\r\n";
    let read = FlyingData::parse(text).unwrap();
    assert_eq!(read.initial_velocity, -100.0);
    assert_eq!(read.acceleration, [0.0, -9000.0, 0.0]);
    assert_eq!(read.bomb_effect, None);
    assert!(!read.homing && read.ignore_z && !read.spreading);
    assert_eq!(read.range, 0.0);
    assert_eq!(read.attached.len(), 2);
    assert_eq!(read.attached[0].fly_type, 2);
    assert_eq!(read.attached[0].tail.unwrap().colour, 524_032);
    assert_eq!(read.attached[1].tail, None);
    assert_eq!(read.attached[1].file, RawStr::from(""));
    assert_eq!(read.effects().count(), 1);
}

#[test]
fn damaged_flying_effects_are_refused() {
    let with = |from: &str, to: &str| {
        let text = String::from_utf8(flying().write().unwrap()).unwrap();
        FlyingData::parse(text.replace(from, to).as_bytes())
    };
    assert!(damaged(with("1726079457d", "pinkd")).contains("TailColor is not a colour"));
    assert!(damaged(with("1726079457d", "d")).contains("TailColor"));
    assert!(
        damaged(with("\t0.000000 1.000000 0.000000", "\t1.000000")).contains("expected 3 numbers")
    );
    assert!(with("\t3000.000000", "\tfast").is_err());
}

proptest! {
    #[test]
    fn written_scripts_read_back(tree in script_tree()) {
        let text = script::write(&tree).unwrap();
        prop_assert_eq!(without_lines(&script::parse(&text).unwrap()), tree);
    }

    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        if let Ok(parsed) = script::parse(&bytes) {
            let _ = script::write(&parsed);
            let _ = RaceData::read(&parsed);
            let _ = MotionData::read(&parsed);
            let _ = Environment::read(&parsed);
        }
        let _ = motion::parse_list(&bytes);
        if let Ok(lines) = npc_list::parse(&bytes) {
            let _ = npc_list::find(&lines, 101);
        }
        if let Ok(lines) = item_list::parse(&bytes) {
            let _ = item_list::find(&lines, 10);
            let _ = item_list::write(&lines);
        }
        if let Ok(lines) = skill_desc::parse(&bytes) {
            let _ = skill_desc::find(&lines, 1);
            let _ = skill_desc::write(&lines);
        }
        if let Ok(lines) = skill_table::parse(&bytes) {
            let _ = skill_table::find(&lines, 1);
            let _ = skill_table::write(&lines);
        }
    }

    #[test]
    fn damaged_skill_tables_never_panic(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)) {
        let mut bytes = skill_table::write(&skill_table_lines()).unwrap();
        for (at, value) in flips {
            let at = at.index(bytes.len());
            bytes[at] = value;
        }
        if let Ok(lines) = skill_table::parse(&bytes) {
            // What reads writes back and reads the same, unless it holds
            // what the writer refuses.
            if let Ok(text) = skill_table::write(&lines) {
                prop_assert_eq!(skill_table::parse(&text).unwrap(), lines);
            }
        }
    }

    #[test]
    fn damaged_skill_descriptions_never_panic(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)) {
        let mut bytes = skill_desc::write(&skill_lines()).unwrap();
        for (at, value) in flips {
            let at = at.index(bytes.len());
            bytes[at] = value;
        }
        if let Ok(lines) = skill_desc::parse(&bytes) {
            // What reads writes back and reads the same, unless it holds
            // what the writer refuses.
            if let Ok(text) = skill_desc::write(&lines) {
                prop_assert_eq!(skill_desc::parse(&text).unwrap(), lines);
            }
        }
    }

    #[test]
    fn damaged_scripts_never_panic(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)) {
        let mut bytes = race_data().write().unwrap();
        for (at, value) in flips {
            let at = at.index(bytes.len());
            bytes[at] = value;
        }
        let _ = RaceData::parse(&bytes);
        let _ = MotionData::parse(&bytes);
        let _ = motion::parse_list(&bytes);
    }

    #[test]
    fn damaged_attack_motions_never_panic(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)) {
        let mut bytes = attack_motion().write().unwrap();
        for (at, value) in flips {
            let at = at.index(bytes.len());
            bytes[at] = value;
        }
        if let Ok(data) = MotionData::parse(&bytes) {
            let _ = data.write();
            let _ = data.reach();
            for hit in data.attacking.iter().flat_map(|a| &a.hits) {
                let _ = hit.segment_at(hit.start);
            }
        }
    }

    #[test]
    fn written_attack_motions_read_back(data in attack_motion_strategy()) {
        let text = data.write().unwrap();
        prop_assert_eq!(MotionData::parse(&text).unwrap(), data);
    }
}

/// Numbers our writer's six decimals keep exactly.
fn decimal() -> impl Strategy<Value = f32> {
    (-200_000i32..200_000).prop_map(|n| n as f32 / 1000.0)
}

fn point() -> impl Strategy<Value = [f32; 3]> {
    [decimal(), decimal(), decimal()]
}

fn word() -> impl Strategy<Value = Option<RawStr>> {
    prop::option::of("[a-z_]{0,12}".prop_map(|w| RawStr::from(w.as_str())))
}

fn hit_strategy() -> impl Strategy<Value = Hit> {
    let row =
        (decimal(), point(), point()).prop_map(|(time, from, to)| HitPosition { time, from, to });
    (
        decimal(),
        decimal(),
        word(),
        decimal(),
        prop::collection::vec(row, 0..4),
    )
        .prop_map(|(start, end, bone, weapon_length, positions)| Hit {
            start,
            end,
            bone: bone.unwrap_or_default(),
            weapon_length,
            positions,
        })
}

fn attack_event_strategy() -> impl Strategy<Value = AttackEvent> {
    let sphere = (decimal(), point()).prop_map(|(radius, position)| Sphere { radius, position });
    (
        (decimal(), prop::option::of(any::<bool>())),
        (prop::option::of(0u32..3), 0u32..3),
        (decimal(), decimal(), decimal()),
        (prop::option::of(0u32..20), 0u32..5),
        prop::collection::vec(sphere, 0..3),
    )
        .prop_map(
            |(
                (during, enable_hit),
                (attack_type, hitting_type),
                (stiffen_time, invisible_time, external_force),
                (hit_limit, collision_type),
                spheres,
            )| AttackEvent {
                during,
                enable_hit,
                attack_type,
                hitting_type,
                stiffen_time,
                invisible_time,
                external_force,
                hit_limit,
                collision_type,
                spheres,
            },
        )
}

fn effect_strategy() -> impl Strategy<Value = Effect> {
    (
        word(),
        point(),
        word(),
        any::<bool>(),
        prop::option::of(any::<bool>()),
    )
        .prop_map(|(file, position, bone, following, flag)| Effect {
            file: file.unwrap_or_default(),
            position,
            bone,
            following,
            independent: flag,
            fishing: flag.map(|f| !f),
        })
}

fn event_strategy() -> impl Strategy<Value = MotionEvent> {
    let kind = prop_oneof![
        effect_strategy().prop_map(EventKind::Effect),
        (decimal(), decimal(), decimal()).prop_map(|(during, power, range)| EventKind::Shake(
            Shake {
                during,
                power,
                range
            }
        )),
        attack_event_strategy().prop_map(EventKind::Attack),
        (word(), point(), word()).prop_map(|(file, position, bone)| EventKind::Fly(Fly {
            file: file.unwrap_or_default(),
            position,
            bone
        })),
        effect_strategy().prop_map(|mut e| {
            e.independent = None;
            EventKind::SpotEffect(e)
        }),
        (7u32..10).prop_map(EventKind::Other),
    ];
    (decimal(), kind).prop_map(|(time, kind)| MotionEvent { time, kind })
}

fn attack_motion_strategy() -> impl Strategy<Value = MotionData> {
    let combo = (decimal(), decimal(), decimal(), prop::option::of(decimal())).prop_map(
        |(pre_input, direct_input, input_limit, link)| ComboInput {
            pre_input,
            direct_input,
            input_limit,
            link,
        },
    );
    let attacking = (
        (
            prop::option::of(0u32..2),
            any::<bool>(),
            prop::option::of(0u32..4),
        ),
        (0u32..3, decimal(), decimal(), decimal()),
        prop::option::of(0u32..20),
        prop::collection::vec(hit_strategy(), 1..4),
    )
        .prop_map(
            |(
                (attack_type, older, motion_type),
                (hitting_type, stiffen_time, invisible_time, external_force),
                hit_limit,
                mut hits,
            )| {
                if older {
                    hits.truncate(1);
                }
                Attacking {
                    attack_type,
                    attacking_type: older.then_some(1),
                    motion_type,
                    hitting_type,
                    stiffen_time,
                    invisible_time,
                    external_force,
                    hit_limit,
                    hits,
                }
            },
        );
    (
        prop::option::of(decimal()),
        prop::option::of(combo),
        prop::option::of(attacking),
        prop::collection::vec(event_strategy(), 0..5),
    )
        .prop_map(|(duration, combo, attacking, events)| MotionData {
            motion_file: RawStr::from("a.gr2"),
            duration,
            accumulation: None,
            combo,
            attacking,
            events,
        })
}

fn sub_image(version: &str) -> SubImage {
    SubImage {
        version: RawStr::from(version),
        image: RawStr::from("Public.dds"),
        left: 194,
        top: 142,
        right: 255,
        bottom: 163,
    }
}

#[test]
fn a_sub_image_byte_for_byte() {
    let text = b"title\tsubImage\r\n\
version\t1.0\r\n\
image\t\"Public.dds\"\r\n\
left\t194\r\n\
top\t142\r\n\
right\t255\r\n\
bottom\t163\r\n";
    assert_eq!(sub_image("1.0").write().unwrap(), text);
    let parsed = SubImage::parse(text).unwrap();
    assert_eq!(parsed, sub_image("1.0"));
    assert_eq!(
        (parsed.width(), parsed.height()),
        (61, 21),
        "right and bottom are just past the rectangle"
    );
    assert_eq!(sub::TITLE, "subImage");
}

#[test]
fn sub_images_in_the_clients_style_and_their_errors() {
    // The client's spacing; version 2.0 has the same keys.
    let text = b"title subImage\r\nversion 2.0\r\nimage \"loading0.jpg\"\r\nleft 0\r\ntop 0\r\nright 1024\r\nbottom 768\r\n";
    let parsed = SubImage::parse(text).unwrap();
    assert_eq!(parsed.version, RawStr::from("2.0"));
    assert_eq!((parsed.width(), parsed.height()), (1024, 768));

    let with = |from: &str, to: &str| {
        let text = sub_image("1.0").write().unwrap();
        let text = String::from_utf8(text).unwrap().replace(from, to);
        SubImage::parse(text.as_bytes())
    };
    assert!(matches!(
        with("subImage", "image"),
        Err(FormatError::UnknownLayout { .. })
    ));
    assert!(matches!(
        with("\t1.0", "\t3.0"),
        Err(FormatError::UnknownLayout { .. })
    ));
    assert!(damaged(with("left\t194\r\n", "")).contains("no `left`"));
    assert!(damaged(with("\t255", "\t194")).contains("an empty rectangle"));
    assert!(damaged(with("\t163", "\t-1")).contains("bottom"));
}

/// Made-up spawn lines, one of each kind.
fn spawn_lines() -> Vec<SpawnLine> {
    let line = |kind, x, y, range_x, range_y, direction, respawn, count, vnum| SpawnLine {
        kind,
        aggressive: false,
        x,
        y,
        range_x,
        range_y,
        z: 0,
        direction,
        respawn,
        chance: 100,
        count,
        vnum,
    };
    vec![
        line(SpawnKind::Monster, 430, 608, 0, 0, 6, 60, 1, 1001),
        line(SpawnKind::Group, 371, 446, 102, 20, 0, 60, 12, 7),
        SpawnLine {
            aggressive: true,
            chance: 50,
            z: -3,
            ..line(
                SpawnKind::GroupCollection,
                12,
                34,
                10,
                10,
                0,
                3_600,
                1,
                2001,
            )
        },
        line(SpawnKind::Exception, 1, 2, 3, 4, 8, 0, 5, 1002),
        line(SpawnKind::Special, 0, 0, 0, 0, 0, 7_200, 1, 1003),
    ]
}

#[test]
fn spawn_lists_are_written_exactly() {
    let text: &[u8] = b"//type\tcx\tcy\tsx\tsy\tz\tdir\ttime\tpercent\tcount\tvnum\r\n\
m\t430\t608\t0\t0\t0\t6\t60s\t100\t1\t1001\r\n\
g\t371\t446\t102\t20\t0\t0\t60s\t100\t12\t7\r\n\
ra\t12\t34\t10\t10\t-3\t0\t3600s\t50\t1\t2001\r\n\
e\t1\t2\t3\t4\t0\t8\t0s\t100\t5\t1002\r\n\
s\t0\t0\t0\t0\t0\t0\t7200s\t100\t1\t1003\r\n";
    assert_eq!(spawn_list::write(&spawn_lines()), text);
    assert_eq!(spawn_list::parse(text).unwrap(), spawn_lines());
}

#[test]
fn spawn_lists_read_the_clients_style() {
    // Comments with a count, blank lines, minutes and hours, a bare time,
    // spaces and tabs, upper case, `\n` alone, no line break at the end.
    let text = b"//MonsterAreaInfoCount 5\n\n//type\tcx\tcy\r\n\
M\t430\t608\t0\t0\t0\t6\t1m\t100\t1\t1001\n\
g  371 446\t102  20 0 0 1M 100 12 7\r\n\
RA\t12\t34\t10\t10\t-3\t0\t1h\t50\t1\t2001\r\n\
\t e\t1\t2\t3\t4\t0\t8\t0\t100\t5\t1002 \r\n\
s\t0\t0\t0\t0\t0\t0\t2h\t100\t1\t1003";
    assert_eq!(spawn_list::parse(text).unwrap(), spawn_lines());
    assert_eq!(spawn_list::parse(b"").unwrap(), Vec::new());
}

#[test]
fn damaged_spawn_lists_name_their_line() {
    let line = |text: &str| {
        damaged(spawn_list::parse(
            format!("// a comment\n{text}").as_bytes(),
        ))
    };
    assert!(
        line("m\t1\t2\t3\t4\t0\t0\t60s\t100\t1").contains("line 2: expected 11 values, found 10")
    );
    assert!(line("m 1 2 3 4 0 0 60s 100 1 1001 9").contains("found 12"));
    assert!(line("x 1 2 3 4 0 0 60s 100 1 1001").contains("`x` is not a kind"));
    assert!(line("mb 1 2 3 4 0 0 60s 100 1 1001").contains("`mb` is not a kind"));
    assert!(line("m one 2 3 4 0 0 60s 100 1 1001").contains("x `one` is not a number"));
    assert!(line("m 1 2 3 4 0 -1 60s 100 1 1001").contains("the direction `-1` is not a number"));
    assert!(line("m 1 2 3 4 0 256 60s 100 1 1001").contains("past 255"));
    assert!(line("m 1 2 3 4 0 0 60d 100 1 1001").contains("the time `60d`"));
    assert!(line("m 1 2 3 4 0 0 s 100 1 1001").contains("the time `s`"));
    assert!(line("m 1 2 3 4 0 0 99999999h 100 1 1001").contains("the time"));
    assert!(line("m 1 2 3 4 0 0 60s 100 +1 1001").contains("the count `+1`"));
    assert!(line("m 1 2 3 4 0 0 60s 100 1 -1001").contains("the number `-1001`"));
    assert!(matches!(
        spawn_list::parse(&vec![b' '; m2_formats::script::MAX_LEN + 1]),
        Err(FormatError::TooLarge { .. })
    ));
}

proptest! {
    #[test]
    fn random_flying_effects_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let _ = FlyingData::parse(&bytes);
    }

    #[test]
    fn random_spawn_lists_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        let _ = spawn_list::parse(&bytes);
    }

    #[test]
    fn spawn_lists_round_trip(
        x in any::<i32>(), y in any::<i32>(), range in any::<i32>(), direction in any::<u8>(),
        respawn in any::<u32>(), count in any::<u32>(), vnum in any::<u32>(), aggressive in any::<bool>(),
    ) {
        let lines: Vec<SpawnLine> = spawn_lines()
            .into_iter()
            .map(|line| SpawnLine { x, y, range_x: range, direction, respawn, count, vnum, aggressive, ..line })
            .collect();
        prop_assert_eq!(spawn_list::parse(&spawn_list::write(&lines)).unwrap(), lines);
    }

    #[test]
    fn random_sub_images_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..200)) {
        let _ = SubImage::parse(&bytes);
    }

    #[test]
    fn sub_images_round_trip(left in 0u32..5000, top in 0u32..5000, w in 1u32..5000, h in 1u32..5000) {
        let sub = SubImage { left, top, right: left + w, bottom: top + h, ..sub_image("2.0") };
        prop_assert_eq!(SubImage::parse(&sub.write().unwrap()).unwrap(), sub);
    }
}
