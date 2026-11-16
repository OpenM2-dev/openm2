//! End to end: `openm2 view ui --screenshot` draws the built-in in-game
//! layout with fallback colours, and with art from a synthetic client
//! imported into an asset folder: a TGA gauge pattern, a button cut from a
//! DDS atlas by a version-1.0 sub-image, and an item's icon in the
//! inventory and on the quickslot bar.
//!
//! The pictures need a graphics adapter (a software one is enough: WARP on
//! Windows, Mesa's lavapipe on Linux); without one the tests fail.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_formats::dds::{self, DdsFormat, Masks};
use m2_formats::script::item_list::{self, ItemLine};
use m2_formats::script::sub::SubImage;
use m2_formats::tga::{self, Encoding};
use m2_formats::{ArchiveWriter, EntryKind, ItemProto, ItemRecord, Keys, RawStr, RgbaImage};

/// The background `view ui` draws behind the windows: (0.18, 0.21, 0.24).
const BACKGROUND: [u8; 4] = [46, 54, 61, 255];

/// A temp folder, removed afterwards.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "openm2-ui-{name}-{}-{}",
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

fn openm2(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_openm2"))
        .args(args)
        .env_remove("RUST_LOG")
        .env_remove("OPENM2_KEYS")
        .output()
        .unwrap()
}

/// `client` imported into `dir`'s asset folder `assets`: the folder's path,
/// and what the import said.
fn import(dir: &Dir, client: &str) -> (String, String) {
    let assets = dir.path("assets");
    let output = openm2(&["assets", "import", "--client", client, "--out", &assets]);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(output.status.success(), "{stdout}{stderr}");
    (assets, stdout)
}

/// Runs `view ui` into `out`.
fn shoot(args: &[&str], out: &str) -> RgbaImage {
    let mut all = vec!["view", "ui", "--size", "640x400", "--screenshot", out];
    all.extend(args);
    let output = openm2(&all);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(output.status.success(), "{stderr}");
    m2_render::png::decode(&std::fs::read(out).unwrap()).unwrap()
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3)
}

#[test]
fn the_in_game_layout_draws_with_fallback_colours() {
    let dir = Dir::new("fallback");
    let out = dir.path("ui.png");
    let image = shoot(&["--screen", "game", "--value", "taskbar.health=0.5"], &out);
    let pixel = |x, y| image.pixel(x, y).unwrap();
    assert!(near(pixel(400, 100), BACKGROUND), "{:?}", pixel(400, 100));
    // The chat's back (8, 148)-(488, 314): see-through black over the
    // background.
    let back = pixel(240, 200);
    assert!(
        back[0] < BACKGROUND[0] && back[2] < BACKGROUND[2],
        "{back:?}"
    );
    // The taskbar (699 × 48) is centred 4 px above the bottom, wider than
    // this picture: from x -29 or -30; its health gauge, 95 × 11, is 61 px
    // in and 14 down, from x 31 or 32: the left half is the fallback red,
    // the right half not.
    assert!(
        near(pixel(60, 367), [200, 30, 30, 255]),
        "{:?}",
        pixel(60, 367)
    );
    assert!(pixel(110, 367)[0] < 100, "{:?}", pixel(110, 367));
}

#[test]
fn plays_screens_draw_their_windows() {
    let dir = Dir::new("screens");
    let out = dir.path("login.png");
    let image = shoot(
        &[
            "--screen",
            "login",
            "--set",
            "login.status=Asking the server...",
        ],
        &out,
    );
    let pixel = |x, y| image.pixel(x, y).unwrap();
    // The intro background's fallback red over the whole screen, and the
    // login board's (see-through dark brown) in the middle, 300 × 236, 40 px
    // below the centre.
    assert!(
        near(pixel(10, 10), [110, 20, 20, 255]),
        "{:?}",
        pixel(10, 10)
    );
    let board = pixel(320, 300);
    assert!(board[0] < 60 && board[1] < 40, "{board:?}");
    // The select screen without characters shows no board but the buttons.
    let out = dir.path("select.png");
    shoot(&["--screen", "select"], &out);
    // `--screen` takes play's layouts; layout files cannot come with it.
    let output = openm2(&["view", "ui", "nothing.ron", "--screen", "login"]);
    assert_eq!(output.status.code(), Some(2));
}

/// The client: the health gauge's pattern as a green TGA, and the public
/// atlas (`d:/ymir work/ui/public.dds`, yellow left half, magenta right
/// half) with the button sub-images cutting its right half.
fn client(dir: &Dir) -> String {
    let green = RgbaImage::filled(16, 8, [0, 200, 0, 255]).unwrap();
    let gauge = tga::encode(
        &green,
        Encoding {
            depth: 32,
            rle: true,
            top_down: false,
        },
    )
    .unwrap();
    let mut atlas = Vec::new();
    for _ in 0..8 {
        for x in 0..8 {
            atlas.extend(if x < 4 {
                [0, 255, 255, 255]
            } else {
                [255, 0, 255, 255]
            });
        }
    }
    let public = dds::write(8, 8, &DdsFormat::Masked(Masks::BGRA8), &[&atlas]).unwrap();
    let button = SubImage {
        version: RawStr::from("1.0"),
        image: RawStr::from("Public.dds"),
        left: 4,
        top: 0,
        right: 8,
        bottom: 8,
    }
    .write()
    .unwrap();
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("d:/ymir work/ui/pattern/gauge_red.tga".to_owned(), gauge),
        ("d:/ymir work/ui/public.dds".to_owned(), public),
    ];
    for n in 1..=3 {
        files.push((
            format!("d:/ymir work/ui/public/middle_button_0{n}.sub"),
            button.clone(),
        ));
    }
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    for (name, bytes) in &files {
        pack.add(name.as_bytes(), bytes, EntryKind::Lzo, &keys)
            .unwrap();
    }
    std::fs::create_dir_all(dir.0.join("client/pack")).unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    std::fs::write(dir.0.join("client/pack/ui.eix"), eix).unwrap();
    std::fs::write(dir.0.join("client/pack/ui.epk"), epk).unwrap();
    dir.path("client")
}

#[test]
fn layouts_draw_the_clients_art_imported_from_tga_and_sub_images() {
    let dir = Dir::new("art");
    let (assets, _) = import(&dir, &client(&dir));
    let layout = dir.path("button.ron");
    std::fs::write(
        &layout,
        r#"(windows: [
            (id: "bar", pos: (10, 10), size: (100, 8), kind: Gauge(skin: "health", value: 1.0)),
            (id: "ok", pos: (10, 40), size: (61, 21), kind: Button(skin: "button")),
        ])"#,
    )
    .unwrap();
    let out = dir.path("ui.png");
    let image = shoot(&[&layout, "--assets", &assets], &out);
    let pixel = |x, y| image.pixel(x, y).unwrap();
    assert!(
        near(pixel(50, 14), [0, 200, 0, 255]),
        "the TGA pattern: {:?}",
        pixel(50, 14)
    );
    assert!(
        near(pixel(40, 50), [255, 0, 255, 255]),
        "the sub-image's half: {:?}",
        pixel(40, 50)
    );
    assert!(near(pixel(300, 300), BACKGROUND));

    // Twice the size: the gauge at (20, 20), 200 × 16, the button at (20, 80),
    // 122 × 42, its art keeping to its half of the atlas up to its edge, with
    // either filter.
    for filter in ["smooth", "sharp"] {
        let out = dir.path(&format!("ui-{filter}.png"));
        let args = [&layout, "--assets", &assets, "--ui-scale", "2"];
        let image = shoot(&[&args[..], &["--ui-filter", filter]].concat(), &out);
        let pixel = |x, y| image.pixel(x, y).unwrap();
        for (x, y, colour) in [
            (210, 34, [0, 200, 0, 255]),
            (21, 100, [255, 0, 255, 255]),
            (140, 120, [255, 0, 255, 255]),
            (40, 50, BACKGROUND),
            (150, 100, BACKGROUND),
        ] {
            assert!(
                near(pixel(x, y), colour),
                "{filter} at ({x}, {y}): {:?}",
                pixel(x, y)
            );
        }
    }
    let output = openm2(&["view", "ui", "--ui-scale", "9", "--screenshot", &out]);
    assert_eq!(output.status.code(), Some(2), "out of range");
}

#[test]
fn bad_ids_and_layouts_are_reported() {
    let dir = Dir::new("bad");
    let out = dir.path("ui.png");
    let output = openm2(&["view", "ui", "--set", "nothing=1", "--screenshot", &out]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no text, button or field has the id `nothing`"),
        "{stderr}"
    );
    let layout = dir.path("bad.ron");
    std::fs::write(
        &layout,
        "(windows: [(size: (1, 1), kind: Panel(skin: \"nope\"))])",
    )
    .unwrap();
    let output = openm2(&["view", "ui", &layout, "--screenshot", &out]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("names the skin `nope`"), "{stderr}");
}

/// A client with one item: a sword (item 10, two cells tall) with a green
/// 32 × 64 icon, in the item table and item list of locale `en`, which
/// `locale.cfg` names.
fn item_client(dir: &Dir) -> String {
    let keys = Keys::none();
    let green = RgbaImage::filled(32, 64, [0, 200, 0, 255]).unwrap();
    let encoding = Encoding {
        depth: 32,
        rle: false,
        top_down: true,
    };
    let icon = tga::encode(&green, encoding).unwrap();
    let proto = ItemProto {
        version: 1,
        records: vec![ItemRecord {
            vnum: 10,
            locale_name: RawStr::from("Sword+0"),
            size: 2,
            ..ItemRecord::default()
        }],
    }
    .to_bytes(None)
    .unwrap();
    let list = item_list::write(&[ItemLine {
        vnum: 10,
        kind: RawStr::from("WEAPON"),
        icon: RawStr::from("icon/item/00010.tga"),
        model: None,
    }])
    .unwrap();
    let mut pack = ArchiveWriter::new();
    for (name, bytes) in [
        ("icon/item/00010.tga", &icon),
        ("locale/en/item_proto", &proto),
        ("locale/en/item_list.txt", &list),
    ] {
        pack.add(name.as_bytes(), bytes, EntryKind::Lzo, &keys)
            .unwrap();
    }
    std::fs::create_dir_all(dir.0.join("client/pack")).unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    std::fs::write(dir.0.join("client/pack/items.eix"), eix).unwrap();
    std::fs::write(dir.0.join("client/pack/items.epk"), epk).unwrap();
    std::fs::write(dir.0.join("client/locale.cfg"), "10002 1252 en").unwrap();
    dir.path("client")
}

#[test]
fn the_inventory_shows_the_clients_item_icons_imported() {
    let dir = Dir::new("items");
    let (assets, _) = import(&dir, &item_client(&dir));
    let out = dir.path("inventory.png");
    let output = openm2(&[
        "view",
        "ui",
        "--screen",
        "game",
        "--show",
        "inventory",
        "--assets",
        &assets,
        "--item",
        "0=10",
        "--quick",
        "3=0",
        "--size",
        "1280x800",
        "--screenshot",
        &out,
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(output.status.success(), "{stderr}");
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x, y| image.pixel(x, y).unwrap();
    // The inventory (176 × 576) stands 8 px from the right, centred: its
    // bag's first cell is at (1104, 368), and the sword covers it and the
    // cell below.
    for (x, y) in [(1110, 372), (1130, 428)] {
        assert!(
            near(pixel(x, y), [0, 200, 0, 255]),
            "({x}, {y}): {:?}",
            pixel(x, y)
        );
    }
    assert!(!near(pixel(1130, 440), [0, 200, 0, 255]), "two cells tall");
    // Quickslot 4 on the bar at the bottom shows it too, inside its 32 × 32
    // cell: the bar (685 × 48) is centred 4 px above the bottom, its slots
    // 275 px in and 12 down, after the health, mana and experience.
    assert!(near(pixel(297 + 275 + 3 * 32 + 16, 776), [0, 200, 0, 255]));
    // Items need the game screen.
    let output = openm2(&["view", "ui", "--item", "0=10", "--screenshot", &out]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("need --screen game"));
    // A client's art is imported first.
    let output = openm2(&[
        "view",
        "ui",
        "--client",
        &item_client(&dir),
        "--screenshot",
        &out,
    ]);
    assert_eq!(output.status.code(), Some(2));
}

/// The interface imported into an asset folder: [`client`]'s art and
/// [`item_client`]'s sword become a skin file naming PNG files and regions
/// of them, and an item table.
#[test]
fn the_interface_imports_into_a_skin_file_and_an_item_table() {
    let dir = Dir::new("assets");
    let client = client(&dir);
    item_client(&dir);
    let (assets, stdout) = import(&dir, &client);
    assert!(
        stdout.contains(", 4 images; items: 1 (1 icons, 0 models); skills: 0;"),
        "{stdout}"
    );
    let read = |path: &str| std::fs::read_to_string(Path::new(&assets).join(path)).unwrap();
    let folder = read("assets.toml");
    assert!(
        folder.contains("skin = \"skin.toml\"\nitems = \"items.toml\"\n"),
        "{folder}"
    );
    let skin = read("skin.toml");
    for expected in [
        "[look.button]\nkind = \"button\"\nnormal = \"ymir work/ui/public.png#xywh=4,0,4,8\"\n",
        "[look.health]\nkind = \"gauge\"\nfill = \"ymir work/ui/pattern/gauge_red.png\"\nslot = \"gauge_slot\"\n",
        // Art the client lacks is left out: the fallback draws.
        "[look.mana]\nkind = \"gauge\"\nslot = \"gauge_slot\"\nfallback = [40, 80, 220, 255]\n",
    ] {
        assert!(skin.contains(expected), "{expected:?} in:\n{skin}");
    }
    let items = read("items.toml");
    assert!(
        items.contains(
            "[[item]]\nvnum = 10\nname = \"Sword+0\"\nicon = \"icon/item/00010.png\"\nsize = 2\n"
        ),
        "{items}"
    );
    let output = openm2(&["assets", "check", &assets]);
    let checked = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(output.status.success(), "{checked}");
    assert!(checked.contains("1 skins\n"), "{checked}");
    assert!(checked.contains("1 item tables\n"), "{checked}");
}
