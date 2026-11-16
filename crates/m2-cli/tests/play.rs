//! End to end: `openm2 play --screenshot` against a scripted game server and
//! a synthetic client, imported into an asset folder first as the launcher
//! imports a player's (`play` reads only asset folders). The client has one
//! map of green ground, a player
//! model (a red square 2 m above the ground) whose run motion covers 3 m a
//! second, and a wolf (a blue square) whose attack moves it 3 m east and
//! whose fall moves it 3 m north. The server lets Alice in next to the
//! wolf; `play` walks 2 m south, tells the server, draws one picture from
//! above and logs out. In another game, one wolf bites and another dies:
//! the picture shows both moved by their motions. In a third, `play
//! --fight` chooses a wolf 2 m east of Alice, turns to it and swings: her
//! bare-handed attack's blade reaches 2.5 m ahead, so it hits, and the
//! wolf dies. In a fourth, two items lie on the ground (the bag every item
//! shows without an item list: a yellow square): `play --pick-up` picks up
//! the one beside Alice and leaves Bob's. In a fifth, `play --shop` talks
//! to a shopkeeper 2 m from Alice and draws its shop beside the inventory.
//! In a sixth, the map's attributes wall off the way south: `play --walk`
//! goes around the wall, a leg at a time. Another server only lists the
//! characters: `play` draws the select screen with Bob (a warrior: a red
//! wall facing the camera) and logs out without picking one. The login
//! screen needs no server.
//!
//! The picture needs a graphics adapter (a software one is enough: WARP on
//! Windows, Mesa's lavapipe on Linux); without one the tests fail.

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_content::WalkMap;
use m2_core::Point;
use m2_formats::dds::{self, DdsFormat, Masks};
use m2_formats::gr2::model::{
    Animation, Bone, Curve, Gr2File, Material, Mesh, Model, Skeleton, Texture, TrackGroup,
    TransformTrack, Vertices,
};
use m2_formats::gr2::write::write_model;
use m2_formats::gr2::{Compression, Transform};
use m2_formats::map::atlas::{self, AtlasEntry};
use m2_formats::map::grid::{AttrMap, Grid, HeightMap, TileMap};
use m2_formats::map::setting::MapSetting;
use m2_formats::map::texture_set::{TerrainTexture, TextureSet};
use m2_formats::script::motion::{
    self, AttackEvent, Attacking, EventKind, Hit, HitPosition, MotionData, MotionEvent,
    MotionListEntry, Sphere,
};
use m2_formats::script::npc_list::{self, NpcLine};
use m2_formats::script::race::RaceData;
use m2_formats::script::skill_desc::{self, SkillLine};
use m2_formats::script::skill_table::{self, SkillTableLine};
use m2_formats::{ArchiveWriter, EntryKind, Keys, RawStr};
use m2_protocol::profile::openm2::{
    Character, CharacterList, CharacterMove, ClientMessage, Damage, Empire, EnterGame, EntityDied,
    EntityMove, EntitySpawn, GameTime, GroundItemAdd, GroundItemRemove, Hello, ItemOwnership,
    ItemSet, MainCharacter, PROTOCOL, Phase, PickUp, PointValue, PointsChanged, SelectCharacter,
    SendChat, ServerMessage, SetTarget, ShopGood, ShopOpened, SkillState, Skills, Talk,
    TargetHealth, TicketLogin, UseSkill, Welcome,
};
use support::{ACCOUNT, CODE, Peer, Step, c2s, c2s_except, s2c};

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
const MY_VID: u32 = 0x1234;
const WOLF_VID: u32 = 9001;
/// Alice enters 64 m east and 64 m south of the map's corner; `--walk 0,2`
/// takes her 2 m south.
const START: (i32, i32) = (6_400, 6_400);
const WALK_END: (i32, i32) = (6_400, 6_600);
/// The wolf stands 3 m east and 6 m north of her.
const WOLF_AT: (i32, i32) = (6_700, 5_800);
/// Another wolf, 6 m west of her.
const OTHER_WOLF_VID: u32 = 9002;
const OTHER_WOLF_AT: (i32, i32) = (5_800, 6_400);
/// The wolf `--fight` goes for: 2 m east of her.
const FOE_AT: (i32, i32) = (6_600, 6_400);
/// An item anyone may take, 1.4 m east of her; Bob's, 5 m north.
const LOOT_VID: u32 = 7001;
const LOOT_AT: (i32, i32) = (6_540, 6_400);
const BOBS_VID: u32 = 7002;
const BOBS_AT: (i32, i32) = (6_400, 5_900);
/// The wall [`client_with_wall`] puts 3 m south of her: the squares of row
/// 67 from 58 to 66 m east.
const WALL_ROW: usize = 67;
const WALL_FROM: usize = 58;
const WALL_TO: usize = 66;
/// Where `--walk 0,6` takes her, beyond the wall.
const BEYOND_WALL: (i32, i32) = (6_400, 7_000);

/// A temp folder, removed afterwards.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "openm2-play-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// A 4 × 4 texture of one colour (A8R8G8B8).
fn solid_dds(rgba: [u8; 4]) -> Vec<u8> {
    let [r, g, b, a] = rgba;
    let level = [b, g, r, a].repeat(16);
    dds::write(4, 4, &DdsFormat::Masked(Masks::BGRA8), &[&level]).unwrap()
}

/// A flat square `side` cm wide, `height` cm up, facing up, on the bone
/// `root`, with the texture `texture`.
fn square(texture: &str, side: f32, height: f32) -> Gr2File {
    let h = side / 2.0;
    Gr2File {
        textures: vec![Texture {
            file_name: RawStr::from(texture),
        }],
        materials: vec![Material {
            name: RawStr::from("m"),
            texture: Some(0),
            maps: Vec::new(),
        }],
        skeletons: vec![Skeleton {
            name: RawStr::from("root"),
            bones: vec![Bone {
                name: RawStr::from("root"),
                parent: None,
                local: Transform::default(),
                inverse_world: IDENTITY,
            }],
        }],
        meshes: vec![Mesh {
            name: RawStr::from("square"),
            vertices: Vertices {
                positions: vec![
                    [-h, -h, height],
                    [h, -h, height],
                    [h, h, height],
                    [-h, h, height],
                ],
                normals: vec![[0.0, 0.0, 1.0]; 4],
                uvs: vec![[0.0, 0.0]; 4],
                bone_indices: Vec::new(),
                bone_weights: Vec::new(),
            },
            indices: vec![0, 1, 2, 0, 2, 3],
            groups: Vec::new(),
            materials: vec![Some(0)],
            bones: vec![RawStr::from("root")],
        }],
        models: vec![Model {
            name: RawStr::from("square"),
            skeleton: Some(0),
            placement: Transform::default(),
            meshes: vec![0],
        }],
        ..Gr2File::default()
    }
}

/// A square `side` cm wide standing on the ground, facing −Y (where the
/// select screen looks from), with the texture `texture`.
fn wall(texture: &str, side: f32) -> Gr2File {
    let mut file = square(texture, side, 0.0);
    let h = side / 2.0;
    let mesh = &mut file.meshes[0];
    mesh.vertices.positions = vec![
        [-h, 0.0, 0.0],
        [h, 0.0, 0.0],
        [h, 0.0, side],
        [-h, 0.0, side],
    ];
    mesh.vertices.normals = vec![[0.0, -1.0, 0.0]; 4];
    file
}

/// A second-long animation that keeps `root` where it is.
fn standing() -> Gr2File {
    let empty = Curve {
        degree: 0,
        dimension: 3,
        knots: Vec::new(),
        controls: Vec::new(),
    };
    Gr2File {
        animations: vec![Animation {
            name: RawStr::from("run"),
            duration: 1.0,
            time_step: 1.0 / 30.0,
            track_groups: vec![TrackGroup {
                name: RawStr::from("root"),
                tracks: vec![TransformTrack {
                    name: RawStr::from("root"),
                    position: empty.clone(),
                    orientation: Curve {
                        dimension: 4,
                        ..empty.clone()
                    },
                    scale_shear: Curve {
                        dimension: 9,
                        ..empty
                    },
                }],
            }],
        }],
        ..Gr2File::default()
    }
}

/// An animation of `duration` seconds that holds `root` at `position`.
fn posed(duration: f32, position: [f32; 3]) -> Gr2File {
    let mut file = standing();
    let animation = &mut file.animations[0];
    animation.duration = duration;
    animation.track_groups[0].tracks[0].position = Curve {
        degree: 0,
        dimension: 3,
        knots: vec![0.0],
        controls: position.to_vec(),
    };
    file
}

/// The client: map `metin2_map_test` (green ground), Alice's class (shaman,
/// female: `shaman_w.msm`), Bob's (warrior, male: `warrior_m.msm`) and the
/// wolf (monster 102).
fn client(dir: &Dir) -> String {
    let setting = MapSetting {
        size: [2, 1],
        base_position: Some([0, 0]),
        texture_set: RawStr::from("textureset/test.txt"),
        ..MapSetting::default()
    };
    let set = TextureSet {
        textures: vec![Some(TerrainTexture {
            file: RawStr::from("d:/ymir work/terrainmaps/green.dds"),
            scale: [5.0, 5.0],
            ..TerrainTexture::default()
        })],
    };
    let atlas = atlas::write(&[AtlasEntry {
        map: RawStr::from("metin2_map_test"),
        base: [0, 0],
        size: [2, 1],
    }])
    .unwrap();
    let shaman = RaceData {
        base_model: Some(RawStr::from("d:/ymir work/pc2/shaman/shaman_novice.gr2")),
        ..RaceData::default()
    };
    let run = MotionData {
        motion_file: RawStr::from("d:/ymir work/pc2/shaman/general/run.gr2"),
        duration: Some(1.0),
        accumulation: Some([0.0, -300.0, 0.0]),
        ..MotionData::default()
    };
    // Her bare-handed attack: a second long, a blade from 0.5 to 2.5 m
    // ahead of her, 1 m up, from 0.1 to 0.3 s.
    let row = |time: f32| HitPosition {
        time,
        from: [0.0, -50.0, 100.0],
        to: [0.0, -250.0, 100.0],
    };
    let attack = MotionData {
        motion_file: RawStr::from("d:/ymir work/pc2/shaman/general/attack.gr2"),
        duration: Some(1.0),
        attacking: Some(Attacking {
            hits: vec![Hit {
                start: 0.1,
                end: 0.3,
                bone: RawStr::from("root"),
                weapon_length: 200.0,
                positions: vec![row(0.1), row(0.2), row(0.3)],
            }],
            ..Attacking::default()
        }),
        ..MotionData::default()
    };
    let warrior = RaceData {
        base_model: Some(RawStr::from("d:/ymir work/pc/warrior/warrior_novice.gr2")),
        ..RaceData::default()
    };
    let wolf = RaceData {
        base_model: Some(RawStr::from("d:/ymir work/monster/wolf/wolf.gr2")),
        ..RaceData::default()
    };
    // The wolf bites for a minute, 3 m east of where it stands; it falls
    // 3 m north.
    let wolf_motions = motion::write_list(&[
        MotionListEntry {
            mode: RawStr::from("GENERAL"),
            name: RawStr::from("NORMAL_ATTACK"),
            file: RawStr::from("20.msa"),
            weight: 100,
        },
        MotionListEntry {
            mode: RawStr::from("GENERAL"),
            name: RawStr::from("FRONT_DEAD"),
            file: RawStr::from("31.msa"),
            weight: 100,
        },
    ])
    .unwrap();
    let wolf_motion = |name: &str, duration: f32| MotionData {
        motion_file: RawStr::from(format!("d:/ymir work/monster/wolf/{name}.gr2").as_str()),
        duration: Some(duration),
        ..MotionData::default()
    };
    let npcs = npc_list::write(&[NpcLine {
        vnum: 102,
        name: RawStr::from("wolf"),
        folder: None,
    }])
    .unwrap();
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("metin2_map_test/setting.txt", setting.write().unwrap()),
        (
            "metin2_map_test/000000/height.raw",
            HeightMap::flat(2000).write(),
        ),
        (
            "metin2_map_test/000000/tile.raw",
            TileMap::filled(1).write(),
        ),
        ("textureset/test.txt", set.write().unwrap()),
        (
            "d:/ymir work/terrainmaps/green.dds",
            solid_dds([0, 255, 0, 255]),
        ),
        ("atlasinfo.txt", atlas),
        // The first area from above, for the minimap: all magenta.
        (
            "metin2_map_test/000000/minimap.dds",
            solid_dds([255, 0, 255, 255]),
        ),
        ("shaman_w.msm", shaman.write().unwrap()),
        (
            "d:/ymir work/pc2/shaman/shaman_novice.gr2",
            write_model(
                &square("d:/ymir work/pc2/shaman/red.dds", 200.0, 200.0),
                Compression::Oodle1,
            )
            .unwrap(),
        ),
        (
            "d:/ymir work/pc2/shaman/red.dds",
            solid_dds([255, 0, 0, 255]),
        ),
        (
            "d:/ymir work/pc2/shaman/general/run.msa",
            run.write().unwrap(),
        ),
        (
            "d:/ymir work/pc2/shaman/general/run.gr2",
            write_model(&standing(), Compression::Oodle1).unwrap(),
        ),
        (
            "d:/ymir work/pc2/shaman/general/attack.msa",
            attack.write().unwrap(),
        ),
        (
            "d:/ymir work/pc2/shaman/general/attack.gr2",
            write_model(&standing(), Compression::Oodle1).unwrap(),
        ),
        ("warrior_m.msm", warrior.write().unwrap()),
        (
            "d:/ymir work/pc/warrior/warrior_novice.gr2",
            write_model(
                &wall("d:/ymir work/pc/warrior/red.dds", 200.0),
                Compression::Oodle1,
            )
            .unwrap(),
        ),
        (
            "d:/ymir work/pc/warrior/red.dds",
            solid_dds([255, 0, 0, 255]),
        ),
        ("npclist.txt", npcs),
        ("d:/ymir work/monster/wolf/wolf.msm", wolf.write().unwrap()),
        (
            "d:/ymir work/monster/wolf/wolf.gr2",
            write_model(
                &square("d:/ymir work/monster/wolf/blue.dds", 200.0, 150.0),
                Compression::Oodle1,
            )
            .unwrap(),
        ),
        (
            "d:/ymir work/monster/wolf/blue.dds",
            solid_dds([0, 0, 255, 255]),
        ),
        (
            "d:/ymir work/item/etc/item_bag.gr2",
            write_model(
                &square("d:/ymir work/item/etc/yellow.dds", 100.0, 50.0),
                Compression::Oodle1,
            )
            .unwrap(),
        ),
        (
            "d:/ymir work/item/etc/yellow.dds",
            solid_dds([255, 255, 0, 255]),
        ),
        ("d:/ymir work/monster/wolf/motlist.txt", wolf_motions),
        (
            "d:/ymir work/monster/wolf/20.msa",
            wolf_motion("20", 60.0).write().unwrap(),
        ),
        (
            "d:/ymir work/monster/wolf/20.gr2",
            write_model(&posed(60.0, [300.0, 0.0, 0.0]), Compression::Oodle1).unwrap(),
        ),
        (
            "d:/ymir work/monster/wolf/31.msa",
            wolf_motion("31", 1.0).write().unwrap(),
        ),
        (
            "d:/ymir work/monster/wolf/31.gr2",
            write_model(&posed(1.0, [0.0, 300.0, 0.0]), Compression::Oodle1).unwrap(),
        ),
    ];
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    for (name, bytes) in &files {
        pack.add(name.as_bytes(), bytes, EntryKind::Lzo, &keys)
            .unwrap();
    }
    std::fs::create_dir_all(dir.0.join("client/pack")).unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    std::fs::write(dir.0.join("client/pack/play.eix"), eix).unwrap();
    std::fs::write(dir.0.join("client/pack/play.epk"), epk).unwrap();
    dir.path("client")
}

/// [`client`] with the English locale's skills: Flying Talisman (91),
/// whose motion hits what is 1 to 2.5 m ahead from 0.1 to 0.3 s, and a
/// made-up skill table that gives it a cooldown of 20 + 40 × k seconds.
fn client_with_skills(dir: &Dir) -> String {
    let client = client(dir);
    std::fs::write(dir.0.join("client/locale.cfg"), "10002 1252 en").unwrap();
    let skills = skill_desc::write(&[SkillLine {
        vnum: 91,
        job: RawStr::from("SHAMAN"),
        names: ["Flying Talisman", "", ""].map(RawStr::from),
        description: RawStr::from("Throw a talisman."),
        flags: vec![RawStr::from("ATTACK_SKILL"), RawStr::from("NEED_TARGET")],
        icon: RawStr::from("bipabu"),
        motion: Some(1),
        ..SkillLine::default()
    }])
    .unwrap();
    let talisman = MotionData {
        motion_file: RawStr::from("d:/ymir work/pc2/shaman/skill/bipabu.gr2"),
        duration: Some(1.0),
        events: vec![MotionEvent {
            time: 0.1,
            kind: EventKind::Attack(AttackEvent {
                during: 0.2,
                spheres: vec![Sphere {
                    position: [0.0, -175.0, 100.0],
                    radius: 100.0,
                }],
                ..AttackEvent::default()
            }),
        }],
        ..MotionData::default()
    };
    let table = skill_table::write(&[SkillTableLine {
        vnum: 91,
        name: RawStr::from("Talisman"),
        category: 4,
        cooldown: RawStr::from("20+40*k"),
        sp_cost: RawStr::from("30+60*k"),
        target_range: 1_500,
        // The state of a weapon's aura, to show its look.
        affect: RawStr::from("15"),
        ..SkillTableLine::default()
    }])
    .unwrap();
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("locale/en/skilldesc.txt", skills),
        ("locale/en/skilltable.txt", table),
        // The name of the shaman's first skill group.
        (
            "locale/en/locale_game.txt",
            b"SKILL_GROUP_SHAMAN_1\tDragon\t\r\nSKILL_GROUP_HORSE\tRiding\r\n".to_vec(),
        ),
        (
            "d:/ymir work/pc2/shaman/skill/bipabu.msa",
            talisman.write().unwrap(),
        ),
        (
            "d:/ymir work/pc2/shaman/skill/bipabu.gr2",
            write_model(&standing(), Compression::Oodle1).unwrap(),
        ),
    ];
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    for (name, bytes) in &files {
        pack.add(name.as_bytes(), bytes, EntryKind::Lzo, &keys)
            .unwrap();
    }
    let (eix, epk) = pack.finish(None).unwrap();
    std::fs::write(dir.0.join("client/pack/skills.eix"), eix).unwrap();
    std::fs::write(dir.0.join("client/pack/skills.epk"), epk).unwrap();
    client
}

/// [`client`] with attributes for the map's first area: open but for a
/// wall across the way 3 m south of Alice ([`WALL_ROW`]). The second area
/// has none, so it is all blocked.
fn client_with_wall(dir: &Dir) -> String {
    let client = client(dir);
    let mut flags = vec![0u8; 256 * 256];
    for x in WALL_FROM..=WALL_TO {
        flags[WALL_ROW * 256 + x] = 0x01;
    }
    let attributes = AttrMap {
        grid: Grid::new(256, 256, flags).unwrap(),
    };
    let mut pack = ArchiveWriter::new();
    pack.add(
        b"metin2_map_test/000000/attr.atr",
        &attributes.write().unwrap(),
        EntryKind::Lzo,
        &Keys::none(),
    )
    .unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    std::fs::write(dir.0.join("client/pack/walls.eix"), eix).unwrap();
    std::fs::write(dir.0.join("client/pack/walls.epk"), epk).unwrap();
    client
}

/// The path around [`client_with_wall`]'s wall from `from` to `to`, as
/// `play` finds it: the map is 512 × 256 m, its second area blocked.
fn path_around_the_wall(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let mut walk = WalkMap::open(512, 256).unwrap();
    for y in 0..256 {
        for x in 256..512 {
            walk.set_blocked(x, y, true);
        }
    }
    for x in WALL_FROM..=WALL_TO {
        walk.set_blocked(x as u32, WALL_ROW as u32, true);
    }
    walk.find_path(from, to).unwrap()
}

fn phase(phase: u8) -> Step {
    s2c("Phase", ServerMessage::Phase(Phase { phase }))
}

/// The auth server's ticket: 0xA0, 0xA1, … 0xBF.
fn ticket() -> Vec<u8> {
    (0xA0..=0xBF).collect()
}

/// The client's Hello, and our Welcome.
fn hello() -> [Step; 2] {
    let build = concat!("openm2 ", env!("CARGO_PKG_VERSION")).to_owned();
    [
        c2s(
            "Hello",
            ClientMessage::Hello(Hello {
                protocol: PROTOCOL,
                build,
            }),
        ),
        s2c(
            "Welcome",
            ServerMessage::Welcome(Welcome {
                protocol: PROTOCOL,
                server: "script".to_owned(),
            }),
        ),
    ]
}

/// The auth server: the provider's sign-in, then a ticket.
fn auth_script() -> Vec<Step> {
    let mut steps = Vec::from(hello());
    steps.extend(support::sign_in(support::signed_in(ticket())));
    steps.push(Step::ExpectClose);
    steps
}

fn character(slot: u8, id: u32, name: &str, class: u8) -> Character {
    Character {
        slot,
        id,
        name: name.to_owned(),
        class,
        level: 10,
        playtime: 0,
        st: 0,
        ht: 0,
        dx: 0,
        iq: 0,
        body_part: 0,
        hair_part: 0,
        guild: String::new(),
    }
}

fn spawn(vid: u32, kind: u8, race: u16, (x, y): (i32, i32)) -> Step {
    s2c(
        "EntitySpawn",
        ServerMessage::EntitySpawn(EntitySpawn {
            vid,
            angle: 0,
            x,
            y,
            z: 0,
            kind,
            race,
            move_speed: 100,
            attack_speed: 100,
            state: 0,
            affects: 0,
        }),
    )
}

/// The clock field of CharacterMove in its wire bytes.
const MOVE_TIME: std::ops::Range<usize> = 14..18;

/// Our CharacterMove of `kind` at `(x, y)`, facing south, any clock reading.
fn our_move(label: &'static str, kind: u8, (x, y): (i32, i32)) -> Step {
    c2s_except(
        label,
        ClientMessage::CharacterMove(CharacterMove {
            kind,
            argument: 0,
            angle: 0,
            x,
            y,
            time: 0,
        }),
        MOVE_TIME,
    )
}

/// Our CharacterMove of `kind` at `(x, y)` facing `angle`, any clock
/// reading.
fn our_move_facing(label: &'static str, kind: u8, angle: u16, (x, y): (i32, i32)) -> Step {
    c2s_except(
        label,
        ClientMessage::CharacterMove(CharacterMove {
            kind,
            argument: 0,
            angle,
            x,
            y,
            time: 0,
        }),
        MOVE_TIME,
    )
}

/// The game server up to the character list with these characters.
fn up_to_the_list(characters: Vec<Character>) -> Vec<Step> {
    let mut steps = Vec::from(hello());
    steps.extend([
        c2s(
            "TicketLogin",
            ClientMessage::TicketLogin(TicketLogin {
                username: ACCOUNT.to_owned(),
                ticket: ticket(),
            }),
        ),
        s2c("Empire", ServerMessage::Empire(Empire { empire: 2 })),
        phase(1),
        s2c(
            "CharacterList",
            ServerMessage::CharacterList(CharacterList { characters }),
        ),
    ]);
    steps
}

/// Lists Bob and Alice, then expects the client to leave without picking.
fn select_script() -> Vec<Step> {
    let mut steps = up_to_the_list(vec![character(0, 1, "Bob", 0), character(1, 2, "Alice", 3)]);
    steps.push(Step::ExpectClose);
    steps
}

/// Alice in the world: the list, her choice, the loading, her spawn.
fn into_the_world() -> Vec<Step> {
    let mut steps = up_to_the_list(vec![character(0, 1, "Alice", 3)]);
    steps.extend([
        c2s(
            "SelectCharacter",
            ClientMessage::SelectCharacter(SelectCharacter { slot: 0 }),
        ),
        phase(2),
        s2c(
            "MainCharacter",
            ServerMessage::MainCharacter(MainCharacter {
                vid: MY_VID,
                class: 3,
                name: "Alice".to_owned(),
                x: START.0,
                y: START.1,
                z: 0,
                empire: 2,
                skill_group: 0,
                run_speed: 300,
            }),
        ),
        c2s("EnterGame", ClientMessage::EnterGame(EnterGame)),
        phase(3),
        s2c(
            "GameTime",
            ServerMessage::GameTime(GameTime { time: 10_000 }),
        ),
        spawn(MY_VID, 6, 3, START),
    ]);
    steps
}

/// One wolf bites where it stands, the other dies; `play` says its line.
fn fight_script() -> Vec<Step> {
    let mut steps = into_the_world();
    steps.extend([
        spawn(WOLF_VID, 0, 102, WOLF_AT),
        spawn(OTHER_WOLF_VID, 0, 102, OTHER_WOLF_AT),
        s2c(
            "EntityMove (attack)",
            ServerMessage::EntityMove(EntityMove {
                vid: WOLF_VID,
                kind: 2,
                argument: 0,
                angle: 0,
                x: WOLF_AT.0,
                y: WOLF_AT.1,
                duration: 0,
            }),
        ),
        s2c(
            "EntityDied",
            ServerMessage::EntityDied(EntityDied {
                vid: OTHER_WOLF_VID,
            }),
        ),
        c2s(
            "SendChat",
            ClientMessage::SendChat(SendChat {
                kind: 0,
                text: "hello there".to_owned(),
            }),
        ),
        Step::ExpectClose,
    ]);
    steps
}

/// A wolf 2 m east of Alice: `play --fight` chooses it, swings facing east
/// (90 degrees) where she stands, and hits it; it dies.
fn fight_back_script() -> Vec<Step> {
    let mut steps = into_the_world();
    steps.extend([
        spawn(WOLF_VID, 0, 102, FOE_AT),
        c2s(
            "SetTarget",
            ClientMessage::SetTarget(SetTarget { vid: WOLF_VID }),
        ),
        c2s_except(
            "CharacterMove (attack)",
            ClientMessage::CharacterMove(CharacterMove {
                kind: 2,
                argument: 0,
                angle: 90,
                x: START.0,
                y: START.1,
                time: 0,
            }),
            MOVE_TIME,
        ),
        c2s(
            "Hit",
            ClientMessage::Hit(m2_protocol::profile::openm2::Hit {
                vid: WOLF_VID,
                skill: 0,
            }),
        ),
        s2c(
            "Damage",
            ServerMessage::Damage(Damage {
                vid: WOLF_VID,
                flags: 1,
                amount: 536,
            }),
        ),
        s2c(
            "TargetHealth",
            ServerMessage::TargetHealth(TargetHealth {
                vid: WOLF_VID,
                percent: 0,
            }),
        ),
        s2c(
            "EntityDied",
            ServerMessage::EntityDied(EntityDied { vid: WOLF_VID }),
        ),
        Step::ExpectClose,
    ]);
    steps
}

fn ground_item(label: &'static str, vid: u32, (x, y): (i32, i32)) -> Step {
    s2c(
        label,
        ServerMessage::GroundItemAdd(GroundItemAdd {
            vid,
            vnum: 27_001,
            x,
            y,
        }),
    )
}

fn ownership(label: &'static str, vid: u32, owner: &str) -> Step {
    s2c(
        label,
        ServerMessage::ItemOwnership(ItemOwnership {
            vid,
            owner: owner.to_owned(),
        }),
    )
}

/// Two items on the ground: `play --pick-up` asks for the one beside
/// Alice, which goes into her bag, and leaves Bob's.
fn pick_up_script() -> Vec<Step> {
    let mut steps = into_the_world();
    steps.extend([
        ground_item("GroundItemAdd", LOOT_VID, LOOT_AT),
        ownership("ItemOwnership", LOOT_VID, ""),
        ground_item("GroundItemAdd (Bob's)", BOBS_VID, BOBS_AT),
        ownership("ItemOwnership (Bob's)", BOBS_VID, "Bob"),
        c2s("PickUp", ClientMessage::PickUp(PickUp { vid: LOOT_VID })),
        s2c(
            "ItemSet",
            ServerMessage::ItemSet(ItemSet {
                window: 1,
                slot: 0,
                vnum: 27_001,
                count: 1,
                flags: 0,
                anti_flags: 0,
                sockets: [0; 3],
                bonuses: Vec::new(),
            }),
        ),
        s2c(
            "GroundItemRemove",
            ServerMessage::GroundItemRemove(GroundItemRemove { vid: LOOT_VID }),
        ),
        Step::ExpectClose,
    ]);
    steps
}

/// A shopkeeper 2 m east of Alice; its shop opens when she talks to it.
const SHOPKEEPER_VID: u32 = 8001;

fn shop_script() -> Vec<Step> {
    let mut steps = into_the_world();
    let potions = ShopGood {
        position: 0,
        vnum: 27_001,
        price: 50,
        count: 5,
    };
    steps.extend([
        spawn(SHOPKEEPER_VID, 1, 9_003, (START.0 + 200, START.1)),
        c2s(
            "Talk",
            ClientMessage::Talk(Talk {
                vid: SHOPKEEPER_VID,
            }),
        ),
        s2c(
            "ShopOpened",
            ServerMessage::ShopOpened(ShopOpened {
                vid: SHOPKEEPER_VID,
                goods: vec![potions],
            }),
        ),
        Step::ExpectClose,
    ]);
    steps
}

/// Alice knows Flying Talisman at level 3; `play --skill 91` targets the
/// wolf 2 m east, uses the skill on it, and its motion hits it.
fn skill_script() -> Vec<Step> {
    let mut steps = into_the_world();
    let talisman = SkillState {
        skill: 91,
        level: 3,
        grade: 0,
        next_read: 0,
    };
    steps.extend([
        // Mana for its cost, which `play` checks first.
        s2c(
            "PointsChanged",
            ServerMessage::PointsChanged(PointsChanged {
                points: vec![PointValue {
                    point: 7,
                    value: 500,
                }],
            }),
        ),
        s2c(
            "Skills",
            ServerMessage::Skills(Skills {
                skills: vec![talisman],
            }),
        ),
        spawn(WOLF_VID, 0, 102, FOE_AT),
        c2s(
            "SetTarget",
            ClientMessage::SetTarget(SetTarget { vid: WOLF_VID }),
        ),
        c2s_except(
            "CharacterMove (skill)",
            ClientMessage::CharacterMove(CharacterMove {
                kind: 0x81,
                argument: 0,
                angle: 90,
                x: START.0,
                y: START.1,
                time: 0,
            }),
            MOVE_TIME,
        ),
        c2s(
            "UseSkill",
            ClientMessage::UseSkill(UseSkill {
                skill: 91,
                target: Some(WOLF_VID),
            }),
        ),
        c2s(
            "Hit (skill)",
            ClientMessage::Hit(m2_protocol::profile::openm2::Hit {
                vid: WOLF_VID,
                skill: 91,
            }),
        ),
        Step::ExpectClose,
    ]);
    steps
}

fn game_script() -> Vec<Step> {
    let mut steps = into_the_world();
    steps.extend([
        spawn(WOLF_VID, 0, 102, WOLF_AT),
        // 2 m at 300 cm/s (the run speed MainCharacter gave), then the wait
        // at the end.
        our_move("CharacterMove (move)", 1, WALK_END),
        our_move("CharacterMove (wait)", 0, WALK_END),
        Step::ExpectClose,
    ]);
    steps
}

/// Alice walks around the wall, a walk for each leg, and waits at the end.
fn wall_script() -> Vec<Step> {
    let mut steps = into_the_world();
    let mut from = Point::new(START.0, START.1);
    let mut angle = 0;
    for (x, y) in path_around_the_wall(START, BEYOND_WALL) {
        let to = Point::new(x, y);
        angle = from.angle_to(to);
        steps.push(our_move_facing("CharacterMove (a leg)", 1, angle, (x, y)));
        from = to;
    }
    steps.push(our_move_facing(
        "CharacterMove (wait)",
        0,
        angle,
        BEYOND_WALL,
    ));
    steps.push(Step::ExpectClose);
    steps
}

/// Runs the binary with the peers' key in `OPENM2_SERVER_KEY`.
fn openm2(args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_openm2"));
    command
        .args(args)
        .env("OPENM2_SERVER_KEY", support::server_key())
        .env_remove("RUST_LOG")
        .env_remove("OPENM2_KEYS");
    support::run(command, |address| {
        assert_eq!(support::come_back(address), "HTTP/1.1 200 OK");
    })
}

/// `client` imported into `dir`'s asset folder `assets`, as the launcher
/// imports a player's client before `play`: the folder's path.
fn import(dir: &Dir, client: &str) -> String {
    let assets = dir.path("assets");
    let output = openm2(&["assets", "import", "--client", client, "--out", &assets]);
    assert!(
        output.status.success(),
        "{}{}",
        text(&output.stdout),
        text(&output.stderr)
    );
    assets
}

#[test]
fn play_walks_and_draws_the_map_with_the_actors() {
    let dir = Dir::new("shot");
    let assets = import(&dir, &client(&dir));
    let out = dir.path("play.png");
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", game_script());
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
        "--assets",
        &assets,
        "--walk",
        "0,2",
        "--pitch",
        "90",
        "--distance",
        "40",
        "--size",
        "128x128",
        // the world only: the interface would cover this small picture
        "--no-interface",
        "--stay",
        "0.3",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in [
        "game: entered as Alice (vid 4660)",
        "Monster 102 (vid 9001) appeared at (6700, 5800)",
        "walking from (6400, 6400) to (6400, 6600) at 300 cm/s",
        "arrived at (6400, 6600)",
        "(128×128: Alice on metin2_map_test (64 m, 66 m), 1 around)",
        "logged out",
    ] {
        assert!(report.contains(line), "missing `{line}`\n{report}");
    }
    assert!(
        stderr.contains("loading metin2_map_test around (64 m, 64 m)"),
        "{report}"
    );
    assert!(!report.contains(CODE), "the code printed\n{report}");

    // From above, north up, 40 m away: about 6 pixels a metre. Alice's red
    // square in the middle, the wolf's blue one 3 m east and 8 m north of
    // her, green ground around.
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x: u32, y: u32| image.pixel(x, y).unwrap();
    let red = |[r, g, b, _]: [u8; 4]| r > 100 && g < 60 && b < 60;
    let blue = |[r, g, b, _]: [u8; 4]| b > 100 && r < 60 && g < 60;
    let green = |[r, g, b, _]: [u8; 4]| g > 100 && r < 60 && b < 60;
    assert!(red(pixel(64, 64)), "{:?}", pixel(64, 64));
    assert!(blue(pixel(82, 16)), "{:?}", pixel(82, 16));
    assert!(green(pixel(20, 100)), "{:?}", pixel(20, 100));
}

#[test]
fn play_walks_around_a_wall_a_leg_at_a_time() {
    let path = path_around_the_wall(START, BEYOND_WALL);
    assert!(path.len() > 1, "the wall is in the way: {path:?}");
    let dir = Dir::new("wall");
    let assets = import(&dir, &client_with_wall(&dir));
    let out = dir.path("wall.png");
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", wall_script());
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
        "--assets",
        &assets,
        "--walk",
        "0,6",
        "--size",
        "64x64",
        "--no-interface",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    let walking = format!(
        "walking from (6400, 6400) to (6400, 7000) at 300 cm/s, {} legs",
        path.len()
    );
    for line in [
        "walls of metin2_map_test: 65545 of 512 × 256 squares blocked",
        &walking,
        "arrived at (6400, 7000)",
        "logged out",
    ] {
        assert!(report.contains(line), "missing `{line}`\n{report}");
    }
}

#[test]
fn play_shows_the_map_around_us_in_the_minimap() {
    let dir = Dir::new("minimap");
    let assets = import(&dir, &client(&dir));
    let out = dir.path("minimap.png");
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", game_script());
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
        "--assets",
        &assets,
        "--walk",
        "0,2",
        "--size",
        "480x360",
        "--stay",
        "0.3",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    assert!(report.contains("arrived at (6400, 6600)"), "{report}");

    // The minimap's picture: 128 px at the top right (8 px in, 4 more in
    // its frame), so its middle at (404, 76), a pixel a metre, north up.
    // Alice is in the middle (a white mark: the client has no minimap
    // art), the wolf's red mark 3 m east and 8 m north of her, the first
    // area's magenta around; the corners outside the circle show the world.
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x: u32, y: u32| image.pixel(x, y).unwrap();
    let [r, g, b, _] = pixel(404, 76);
    assert!(r > 240 && g > 240 && b > 240, "{:?}", pixel(404, 76));
    let [r, g, b, _] = pixel(407, 68);
    assert!(r > 200 && g < 80 && b < 80, "{:?}", pixel(407, 68));
    assert_eq!(pixel(374, 96), [255, 0, 255, 255]);
    assert_ne!(pixel(341, 13), [255, 0, 255, 255]);
}

#[test]
fn play_shows_monsters_biting_and_falling() {
    let dir = Dir::new("fight");
    let assets = import(&dir, &client(&dir));
    let out = dir.path("fight.png");
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", fight_script());
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
        "--assets",
        &assets,
        "--pitch",
        "90",
        "--distance",
        "40",
        "--size",
        "128x128",
        "--no-interface",
        "--say",
        "hello there",
        "--stay",
        "0.5",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    assert!(report.contains("vid 9002 died"), "{report}");
    assert!(!report.contains("note: Monster"), "{report}");

    // About 6 pixels a metre, Alice in the middle. The biting wolf's square
    // is 3 m east of where it stands (6 m north, 3 m east of her); the dead
    // one's 3 m north of where it fell (6 m west of her).
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x: u32, y: u32| image.pixel(x, y).unwrap();
    let blue = |[r, g, b, _]: [u8; 4]| b > 100 && r < 60 && g < 60;
    let green = |[r, g, b, _]: [u8; 4]| g > 100 && r < 60 && b < 60;
    for (x, y, want_blue) in [
        (100, 28, true),
        (82, 28, false),
        (28, 46, true),
        (28, 64, false),
    ] {
        let found = pixel(x, y);
        let ok = if want_blue { blue(found) } else { green(found) };
        assert!(ok, "({x}, {y}): {found:?}\n{report}");
    }
}

#[test]
fn play_fights_the_nearest_monster_with_its_motions() {
    let dir = Dir::new("fight-back");
    let assets = import(&dir, &client(&dir));
    let out = dir.path("fight-back.png");
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", fight_back_script());
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
        "--assets",
        &assets,
        "--pitch",
        "90",
        "--distance",
        "40",
        "--size",
        "128x128",
        "--no-interface",
        "--fight",
        "--stay",
        "1.5",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in ["fighting vid 9001", "vid 9001 died"] {
        assert!(report.contains(line), "missing `{line}`\n{report}");
    }

    // About 6 pixels a metre, Alice in the middle: the wolf fell, so its
    // square is 3 m north of where it stood (2 m east of her).
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x: u32, y: u32| image.pixel(x, y).unwrap();
    let blue = |[r, g, b, _]: [u8; 4]| b > 100 && r < 60 && g < 60;
    let green = |[r, g, b, _]: [u8; 4]| g > 100 && r < 60 && b < 60;
    assert!(blue(pixel(76, 46)), "{:?}\n{report}", pixel(76, 46));
    assert!(green(pixel(78, 64)), "{:?}\n{report}", pixel(78, 64));
}

#[test]
fn play_picks_up_what_lies_around_and_leaves_what_is_not_ours() {
    let dir = Dir::new("pick-up");
    let assets = import(&dir, &client(&dir));
    let out = dir.path("pick-up.png");
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", pick_up_script());
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
        "--assets",
        &assets,
        "--pitch",
        "90",
        "--distance",
        "40",
        "--size",
        "128x128",
        "--no-interface",
        "--pick-up",
        "--stay",
        "0.5",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!(
        "stdout:
{stdout}
stderr:
{stderr}"
    );
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in [
        "item 27001 (vid 7001) lies at (6540, 6400)",
        "picking up vid 7001",
        "item vid 7001 is gone from the ground",
    ] {
        assert!(report.contains(line), "missing `{line}`\n{report}");
    }
    assert!(!report.contains("picking up vid 7002"), "{report}");

    // About 6 pixels a metre, Alice in the middle: Bob's yellow square 5 m
    // north; where the one she took lay, 1.4 m east, the ground.
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x: u32, y: u32| image.pixel(x, y).unwrap();
    let yellow = |[r, g, b, _]: [u8; 4]| r > 100 && g > 100 && b < 60;
    let green = |[r, g, b, _]: [u8; 4]| g > 100 && r < 60 && b < 60;
    assert!(yellow(pixel(64, 34)), "{:?}\n{report}", pixel(64, 34));
    assert!(green(pixel(75, 64)), "{:?}\n{report}", pixel(75, 64));
}

#[test]
fn play_talks_to_an_npc_and_shows_its_shop() {
    let dir = Dir::new("shop");
    let assets = import(&dir, &client(&dir));
    let out = dir.path("shop.png");
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", shop_script());
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
        "--assets",
        &assets,
        "--size",
        "800x600",
        "--shop",
        "--stay",
        "0.5",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in ["talking to vid 8001", "the shop of vid 8001 opened"] {
        assert!(report.contains(line), "missing `{line}`\n{report}");
    }

    // The shop (176 × 312, its right side 192 px from the screen's, its top
    // with the inventory's at 12) and the inventory opened beside it: an
    // empty place of the shop's last row looks like an empty cell of the
    // bag's first row, and not like the world at the left.
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x: u32, y: u32| image.pixel(x, y).unwrap();
    let shop_place = pixel(456, 296);
    assert_eq!(shop_place, pixel(640, 284), "{report}");
    assert_ne!(shop_place, pixel(60, 300), "{report}");
}

#[test]
fn play_uses_a_skill_and_shows_the_skills_window() {
    let dir = Dir::new("skill");
    let assets = import(&dir, &client_with_skills(&dir));
    let out = dir.path("skill.png");
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", skill_script());
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
        "--assets",
        &assets,
        "--size",
        "800x600",
        "--skill",
        "91",
        "--skills",
        "--stay",
        "1",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in [
        "skills: 1 from skills.toml",
        "skill cooldowns: 1 from skills.toml",
        "skill 91: level 3",
        "using skill 91",
    ] {
        assert!(report.contains(line), "missing `{line}`\n{report}");
    }

    // The skills window (240 px wide, 16 px from the left, in the middle
    // of the screen's height): its board, not the world behind it.
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x: u32, y: u32| image.pixel(x, y).unwrap();
    assert_ne!(pixel(130, 300), pixel(400, 300), "{report}");
}

#[test]
fn play_draws_the_select_screen_with_the_character() {
    let dir = Dir::new("select");
    let assets = import(&dir, &client(&dir));
    let out = dir.path("select.png");
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", select_script());
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
        "--assets",
        &assets,
        "--size",
        "800x640",
        "--screen",
        "select",
        "--stay",
        "0.2",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in [
        "game: 2 character(s)",
        "(800×640: choosing a character)",
        "logged out",
    ] {
        assert!(report.contains(line), "missing `{line}`\n{report}");
    }
    assert!(!report.contains("playing slot"), "{report}");
    assert!(!report.contains(CODE), "the code printed\n{report}");

    // Bob's picture: 400 × 560 with its centre 150 px left of and 20 px
    // above the screen's, so at (250, 300). His red wall fills its middle;
    // the corner shows the screen's background (its fallback colour: the
    // client has no interface art, so neither has its import).
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x: u32, y: u32| image.pixel(x, y).unwrap();
    let [r, g, b, _] = pixel(250, 300);
    assert!(r > 180 && g < 60 && b < 60, "{:?}", pixel(250, 300));
    let [r, g, b, _] = pixel(4, 4);
    assert!(
        (90..140).contains(&r) && g < 50 && b < 50,
        "{:?}",
        pixel(4, 4)
    );
}

#[test]
fn play_draws_the_login_screen_without_a_server() {
    let dir = Dir::new("login");
    let assets = import(&dir, &client(&dir));
    let out = dir.path("login.png");
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        // nothing listens there: the login screen does not connect
        "--auth-port",
        "1",
        "--assets",
        &assets,
        "--size",
        "640x480",
        "--screen",
        "login",
        "--screenshot",
        &out,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("(640×480: sign in)"), "{stdout}");
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (640, 480));
}

#[test]
fn play_arguments_are_checked() {
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--assets",
        "nowhere",
        "--radius",
        "0",
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("--radius must be more than 0 metres"),
        "{}",
        text(&output.stderr)
    );
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--assets",
        "nowhere",
        "--stay",
        "3",
    ]);
    assert_eq!(output.status.code(), Some(2), "--stay needs --screenshot");
    let output = openm2(&[
        "play",
        "--host",
        "127.0.0.1",
        "--assets",
        "nowhere",
        "--screen",
        "select",
    ]);
    assert_eq!(output.status.code(), Some(2), "--screen needs --screenshot");
    // Without an asset folder, it says how to make one.
    let output = openm2(&["play", "--host", "127.0.0.1"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("`openm2 assets import` makes one of your client"),
        "{}",
        text(&output.stderr)
    );
    // A client folder is imported first: `play` doesn't read one.
    let output = openm2(&["play", "--host", "127.0.0.1", "--client", "nowhere"]);
    assert_eq!(output.status.code(), Some(2), "{}", text(&output.stderr));
    assert!(
        text(&output.stderr).contains("--client"),
        "{}",
        text(&output.stderr)
    );
}

/// [`client_with_skills`] with interface art and the locale's monster
/// table: the minimap's frame and arrow cut from one see-through orange and
/// white atlas, Flying Talisman's icons by grade cut from another, and the
/// wolf's name and level.
fn client_for_assets(dir: &Dir) -> String {
    use m2_formats::script::sub::SubImage;
    use m2_formats::{MobProto, MobRecord};
    let client = client_with_skills(dir);
    // 16 × 16: the left half see-through orange, the right half white.
    let mut minimap = Vec::new();
    for _ in 0..16 {
        for x in 0..16 {
            minimap.extend(if x < 8 {
                [0, 128, 255, 128]
            } else {
                [255, 255, 255, 255]
            });
        }
    }
    let minimap = dds::write(16, 16, &DdsFormat::Masked(Masks::BGRA8), &[&minimap]).unwrap();
    // 12 × 4: red, green, blue squares, the icons of the three grades.
    let mut icons = Vec::new();
    for _ in 0..4 {
        for x in 0..12 {
            icons.extend(match x / 4 {
                0 => [0, 0, 255, 255],
                1 => [0, 255, 0, 255],
                _ => [255, 0, 0, 255],
            });
        }
    }
    let icons = dds::write(12, 4, &DdsFormat::Masked(Masks::BGRA8), &[&icons]).unwrap();
    let sub = |image: &str, left: u32, top: u32, right: u32, bottom: u32| {
        SubImage {
            version: RawStr::from("1.0"),
            image: RawStr::from(image),
            left,
            top,
            right,
            bottom,
        }
        .write()
        .unwrap()
    };
    let mobs = MobProto {
        records: vec![MobRecord {
            vnum: 102,
            locale_name: RawStr::from("Wolf"),
            level: 5,
            ..MobRecord::default()
        }],
    }
    .to_bytes(None)
    .unwrap();
    let mut files: Vec<(String, Vec<u8>)> = vec![
        (
            "d:/ymir work/effect/etc/dropitem/dropitem.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "d:/ymir work/effect/etc/dropitem/white.dds".to_owned(),
            solid_dds([255, 255, 255, 255]),
        ),
        // Glows of +7 items: the body armour's and the long
        // blades', none of the other kinds or levels.
        (
            "d:/ymir work/pc/common/effect/armor/armor_7.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "d:/ymir work/pc/common/effect/sword/sword_7.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        // The rings under what the pointer is over and under the target,
        // and the golden pieces a kill's experience flies as, which name
        // their effect beside them.
        (
            "d:/ymir work/effect/etc/click/click_select.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "d:/ymir work/effect/etc/click/click_glow_select.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "d:/ymir work/effect/etc/gathering/ga_piece_yellow_small2.msf".to_owned(),
            m2_formats::script::fly::FlyingData {
                spreading: true,
                initial_velocity: -100.0,
                cone_angle: 90.0,
                range: 10_000.0,
                bomb_range: 40.0,
                attached: vec![m2_formats::script::fly::FlyAttach {
                    kind: 1,
                    fly_type: 2,
                    file: RawStr::from("ga_piece_yellow2.mse"),
                    ..m2_formats::script::fly::FlyAttach::default()
                }],
                ..m2_formats::script::fly::FlyingData::default()
            }
            .write()
            .unwrap(),
        ),
        (
            "d:/ymir work/effect/etc/gathering/ga_piece_yellow2.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        // What a potion's health and mana and a level gained show, and the
        // sound of a potion drunk.
        (
            "d:/ymir work/effect/etc/recuperation/drugup_red.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "d:/ymir work/effect/etc/recuperation/drugup_blue.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "d:/ymir work/effect/etc/levelup_1/level_up.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "sound/ui/eat_potion.wav".to_owned(),
            m2_formats::wav::write(&m2_formats::wav::Wave {
                channels: 1,
                sample_rate: 22_050,
                bits: 16,
                data: [0i16, 8_000, 0, -8_000]
                    .iter()
                    .flat_map(|s| s.to_le_bytes())
                    .collect(),
            })
            .unwrap(),
        ),
        // A skill's look (its two-handed one missing), the burning state's
        // loop, and the damage numbers of a hit on the target, with their
        // digits, and of a miss on us.
        (
            "d:/ymir work/pc/warrior/effect/geom_sword_loop.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "d:/ymir work/effect/hit/blow_flame/flame_loop.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "d:/ymir work/effect/affect/damagevalue/target.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        (
            "d:/ymir work/effect/affect/damagevalue/miss.mse".to_owned(),
            twinkle().write().unwrap(),
        ),
        // The picture shining items reflect, a baseline JPEG.
        (
            "d:/ymir work/special/spheremap01.jpg".to_owned(),
            m2_formats::jpeg::encode(
                &m2_formats::RgbaImage::filled(16, 16, [200, 200, 220, 255]).unwrap(),
                &[(2, 2), (1, 1), (1, 1)],
                2,
                0,
            )
            .unwrap(),
        ),
        ("d:/ymir work/ui/minimap/minimap.dds".to_owned(), minimap),
        (
            "d:/ymir work/ui/minimap/minimap.sub".to_owned(),
            sub("minimap/minimap.dds", 0, 0, 8, 16),
        ),
        (
            "d:/ymir work/ui/minimap/playermark.sub".to_owned(),
            sub("minimap/minimap.dds", 8, 0, 12, 4),
        ),
        ("d:/ymir work/ui/skill.dds".to_owned(), icons),
        ("locale/en/mob_proto".to_owned(), mobs),
    ];
    for digit in 0..10 {
        files.push((
            format!("d:/ymir work/effect/affect/damagevalue/target_{digit}.dds"),
            solid_dds([255, 220, 0, 255]),
        ));
    }
    for grade in 0..3 {
        files.push((
            format!("d:/ymir work/ui/skill/shaman/bipabu_0{}.sub", grade + 1),
            sub("skill.dds", grade * 4, 0, grade * 4 + 4, 4),
        ));
    }
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    for (name, bytes) in &files {
        pack.add(name.as_bytes(), bytes, EntryKind::Lzo, &keys)
            .unwrap();
    }
    let (eix, epk) = pack.finish(None).unwrap();
    std::fs::write(dir.0.join("client/pack/art.eix"), eix).unwrap();
    std::fs::write(dir.0.join("client/pack/art.epk"), epk).unwrap();
    client
}

/// The twinkle over items on the ground (the client's program names it): a
/// magenta square 2 m wide lying on the ground, there for good.
fn twinkle() -> m2_formats::script::effect::EffectData {
    use m2_formats::script::effect::{
        Curve, EffectData, Element, Emitter, Moving, ParticleElement, ParticleLook, PositionKey,
    };
    EffectData {
        elements: vec![Element::Particle(Box::new(ParticleElement {
            positions: vec![PositionKey {
                time: 0.0,
                moving: Moving::Direct,
                position: [0.0, 0.0, 10.0],
            }],
            emitter: Emitter {
                max_emission_count: 1,
                cycle_length: 1.0,
                cycle_loop: true,
                rate: Curve::constant(1000.0),
                life_time: Curve::constant(1000.0),
                size_x: Curve::constant(200.0),
                size_y: Curve::constant(200.0),
                ..Emitter::default()
            },
            particle: ParticleLook {
                src_blend: 5,
                dest_blend: 6,
                colour_op: 4,
                billboard: 3,
                colour: [
                    Curve::constant(1.0),
                    Curve::constant(0.0),
                    Curve::constant(1.0),
                ],
                textures: vec![RawStr::from("white.dds")],
                ..ParticleLook::default()
            },
            ..ParticleElement::default()
        }))],
        ..EffectData::default()
    }
}

/// `play --screenshot` against the scripted servers, with `args` after the
/// server's: the picture, and what it printed.
fn shot(script: Vec<Step>, args: &[&str], out: &str) -> (m2_formats::RgbaImage, String) {
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script());
    let game = game.play("game", script);
    let mut all = vec![
        "play",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--no-browser",
    ];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--screenshot", out]);
    let output = openm2(&all);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    let servers = (auth.join().unwrap(), game.join().unwrap());
    servers.0.unwrap_or_else(|e| panic!("{e}\n{report}"));
    servers.1.unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    let image = m2_render::png::decode(&std::fs::read(out).unwrap()).unwrap();
    (image, report)
}

/// The whole synthetic client imported into an asset folder, then `play --assets` draws the game (the map, Alice and the wolf,
/// the status, chat, minimap with its art), the twinkle over items on the
/// ground and the select screen (Bob's actor); `view ui --assets` the
/// skills window with Flying Talisman's icon.
#[test]
fn play_draws_an_imported_client() {
    let dir = Dir::new("assets");
    let client = client_for_assets(&dir);
    let assets = dir.path("assets");
    let output = openm2(&["assets", "import", "--client", &client, "--out", &assets]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    assert!(output.status.success(), "{stdout}{stderr}");
    for expected in [
        "imported 3 actors (2 player races, 1 monsters) and 1 maps into",
        "skills: 1; monsters named: 1; music: 0; hit sounds: 0; potion sound: 1",
        // The target's effect and digits, and the miss on us.
        "; damage numbers: 12\n",
    ] {
        assert!(stdout.contains(expected), "{expected:?} in:\n{stdout}");
    }
    let read = |path: &str| std::fs::read_to_string(Path::new(&assets).join(path)).unwrap();
    let folder = read("assets.toml");
    assert!(
        folder.contains("[[monster]]\nvnum = 102\nname = \"Wolf\"\nlevel = 5\nactor = "),
        "{folder}"
    );
    let skills = read("skills.toml");
    for expected in [
        "[[skill]]\nvnum = 91\njob = \"shaman\"\nname = \"Flying Talisman\"\n",
        "flags = [\"attack_skill\", \"need_target\"]\n",
        "icons = [\"ymir work/ui/skill.png#xywh=0,0,4,4\", \"ymir work/ui/skill.png#xywh=4,0,4,4\", \"ymir work/ui/skill.png#xywh=8,0,4,4\"]\n",
        "motion = 1\nmotions = \"bipabu\"\ncooldown = \"20+40*k\"\nsp_cost = \"30+60*k\"\ntarget_range = 15.0\n",
        // Its look while it is on, on the right hand's weapon bone; the
        // two-handed one the client lacks is left out.
        "[skill.look]\neffect = \"ymir work/pc/warrior/effect/geom_sword_loop.effect.toml\"\nbone = \"equip_right_hand\"\n",
        // The shaman's first group's name, from the game texts.
        "[[group]]\njob = \"shaman\"\ngroup = 1\nname = \"Dragon\"\n",
    ] {
        assert!(skills.contains(expected), "{expected:?} in:\n{skills}");
    }
    let output = openm2(&["assets", "check", &assets]);
    assert!(output.status.success(), "{}", text(&output.stdout));

    // The game, with the interface.
    let out = dir.path("game.png");
    let args = [
        "--assets", &assets, "--walk", "0,2", "--size", "320x240", "--stay", "0.3",
    ];
    let (game, report) = shot(game_script(), &args, &out);
    for line in [
        "(320×240: Alice on metin2_map_test (64 m, 66 m), 1 around)",
        "skill cooldowns: 1 from skills.toml",
    ] {
        assert!(report.contains(line), "missing `{line}`\n{report}");
    }
    // The minimap's frame, top right: see-through orange over the
    // green ground.
    let [r, _, b, _] = game.pixel(320 - 8 - 136 + 2, 8 + 70).unwrap();
    assert!(r > 100 && b < 50, "{:?}", game.pixel(178, 78));

    // Items on the ground, each with the twinkle `[effects]` names: Bob's
    // stays, 5 m north of Alice, a magenta square under its bag.
    let folder = read("assets.toml");
    assert!(
        folder
            .contains("[effects]\nitem = \"ymir work/effect/etc/dropitem/dropitem.effect.toml\"\n"),
        "{folder}"
    );
    // The rings, and the experience's flight with its effect.
    for expected in [
        "hovered = \"ymir work/effect/etc/click/click_select.effect.toml\"\n\
         targeted = \"ymir work/effect/etc/click/click_glow_select.effect.toml\"\n",
        "[flies]\nexperience = \"ymir work/effect/etc/gathering/ga_piece_yellow_small2.fly.toml\"\n",
        // A potion's health and mana, a level, and the potion's sound.
        "health_potion = \"ymir work/effect/etc/recuperation/drugup_red.effect.toml\"\n\
         mana_potion = \"ymir work/effect/etc/recuperation/drugup_blue.effect.toml\"\n\
         level_up = \"ymir work/effect/etc/levelup_1/level_up.effect.toml\"\n",
        "potion_sound = \"sound/ui/eat_potion.wav\"\n",
        // The burning state's loop, and the numbers.
        "burning = \"ymir work/effect/hit/blow_flame/flame_loop.effect.toml\"\n",
        "[numbers]\ntarget = \"ymir work/effect/affect/damagevalue/target.effect.toml\"\n\
         target_digits = [\"ymir work/effect/affect/damagevalue/target_0.png\", ",
        "\"ymir work/effect/affect/damagevalue/target_9.png\"]\n\
         miss = \"ymir work/effect/affect/damagevalue/miss.effect.toml\"\n",
    ] {
        assert!(folder.contains(expected), "{expected:?} in:\n{folder}");
    }
    let flight = read("ymir work/effect/etc/gathering/ga_piece_yellow_small2.fly.toml");
    assert!(
        flight.contains("spread = 90.0\n")
            && flight.contains("effect = \"ga_piece_yellow2.effect.toml\""),
        "{flight}"
    );
    // The glows of +7 items the client has: the body armour's, and a long
    // blade's for swords and two-handed weapons.
    let glow = "[[effects.upgraded]]\nlevel = 7\n\
        armour = \"ymir work/pc/common/effect/armor/armor_7.effect.toml\"\n\
        sword = \"ymir work/pc/common/effect/sword/sword_7.effect.toml\"\n\
        two_handed = \"ymir work/pc/common/effect/sword/sword_7.effect.toml\"\n";
    assert!(folder.contains(glow), "{folder}");
    assert_eq!(
        folder.matches("[[effects.upgraded]]").count(),
        1,
        "{folder}"
    );
    // The weapons' shine map, converted from its JPEG (the synthetic client
    // has no armours' one).
    assert!(
        folder.contains("[shine]\nweapon = \"ymir work/special/spheremap01.png\"\n"),
        "{folder}"
    );
    let shine =
        std::fs::read(Path::new(&assets).join("ymir work/special/spheremap01.png")).unwrap();
    let shine = m2_render::png::decode(&shine).unwrap();
    let [r, g, b, _] = shine.pixel(8, 8).unwrap();
    assert!(
        r.abs_diff(200) < 8 && g.abs_diff(200) < 8 && b.abs_diff(220) < 8,
        "{r} {g} {b}"
    );
    let out = dir.path("items.png");
    let args = [
        "--assets",
        &assets,
        "--pitch",
        "90",
        "--distance",
        "40",
        "--size",
        "128x128",
        "--no-interface",
        "--pick-up",
        "--stay",
        "0.5",
    ];
    let (items, _) = shot(pick_up_script(), &args, &out);
    let magenta = |[r, g, b, _]: [u8; 4]| r > 200 && g < 60 && b > 200;
    let [x, y] = [64, 64 - 30 + 5];
    assert!(
        magenta(items.pixel(x, y).unwrap()),
        "{:?}",
        items.pixel(x, y)
    );

    // The select screen: Bob's red wall, from his actor, in the middle of
    // his picture (as in `play_draws_the_select_screen_with_the_character`).
    let out = dir.path("select.png");
    let args = [
        "--assets", &assets, "--size", "800x640", "--screen", "select", "--stay", "0.2",
    ];
    let (select, _) = shot(select_script(), &args, &out);
    let [r, g, b, _] = select.pixel(250, 300).unwrap();
    assert!(r > 180 && g < 60 && b < 60, "{:?}", select.pixel(250, 300));

    // The skills window with Flying Talisman at grand master: its third
    // icon, blue.
    let out = dir.path("skills.png");
    let output = openm2(&[
        "view",
        "ui",
        "--screen",
        "game",
        "--show",
        "skills",
        "--class",
        "3",
        "--skill",
        "91=35",
        "--assets",
        &assets,
        "--size",
        "640x480",
        "--screenshot",
        &out,
    ]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let skills = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let blue = (0..skills.height())
        .flat_map(|y| (0..skills.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let [r, g, b, _] = skills.pixel(x, y).unwrap();
            b > 240 && r < 16 && g < 16
        })
        .count();
    assert!(blue >= 16, "{blue} blue pixels");
}

/// Every file under `root` and its bytes, by path with `/`.
fn files_under(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut files = std::collections::BTreeMap::new();
    let mut folders = vec![(root.to_path_buf(), String::new())];
    while let Some((folder, prefix)) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}{}", entry.file_name().to_str().unwrap());
            if entry.file_type().unwrap().is_dir() {
                folders.push((entry.path(), format!("{name}/")));
            } else {
                files.insert(name, std::fs::read(entry.path()).unwrap());
            }
        }
    }
    files
}

/// The import writes the same files, and says the same, on one thread as
/// on many; `assets current` finds the folder this openm2's import.
#[test]
fn the_import_writes_the_same_on_one_thread_as_on_many() {
    let dir = Dir::new("threads");
    let client = client_for_assets(&dir);
    let mut reports = Vec::new();
    for (out, threads) in [("one", "1"), ("many", "4")] {
        let out = dir.path(out);
        let output = openm2(&[
            "assets",
            "import",
            "--client",
            &client,
            "--out",
            &out,
            "--threads",
            threads,
        ]);
        let stdout = text(&output.stdout);
        assert!(output.status.success(), "{stdout}{}", text(&output.stderr));
        reports.push(stdout.replace(&out, "OUT"));
    }
    assert_eq!(reports[0], reports[1]);
    let (one, many) = (
        files_under(&dir.0.join("one")),
        files_under(&dir.0.join("many")),
    );
    assert!(one.len() > 20, "{:?}", one.keys());
    assert_eq!(
        one.keys().collect::<Vec<_>>(),
        many.keys().collect::<Vec<_>>()
    );
    for (path, bytes) in &one {
        assert!(many[path] == *bytes, "{path} differs");
    }
    let output = openm2(&["assets", "current", &dir.path("many")]);
    assert!(output.status.success(), "{}", text(&output.stdout));
    assert!(
        text(&output.stdout).starts_with("this openm2's import ("),
        "{}",
        text(&output.stdout)
    );
    let output = openm2(&["assets", "current", &dir.path("nothing")]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "no import yet\n");
}
