//! The launcher's window drawn offscreen, as `--screenshot` draws it: its
//! background, its gauge in the accent, the game's picture across the top.
//! Needs a graphics adapter (a software one is enough); without one the
//! tests fail. No fonts: text draws nothing, so the pixels are the same on
//! every machine.

use m2_launcher::window::{Look, SaveRow, Stage, load_look, new_ui, render, show};
use m2_launcher::{Log, WindowSettings};
use m2_ui::Fonts;

/// Draws `stage` in `look` at scale 1 and decodes it.
fn drawn(look: &Look, stage: &Stage) -> (Vec<u8>, impl Fn(u32, u32) -> [u8; 4]) {
    let mut ui = new_ui(look, Fonts::empty()).unwrap();
    show(&mut ui, stage, look);
    let (png, _) = render(&mut ui, look, 1.0, false).unwrap_or_else(|error| panic!("{error:#}"));
    let image = m2_render::png::decode(&png).unwrap();
    assert_eq!((image.width(), image.height()), (640, 400));
    (png, move |x, y| image.pixel(x, y).unwrap())
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 3)
}

#[test]
fn the_window_draws_in_the_games_colours_with_its_picture() {
    let half = Stage::Downloading {
        done: 50,
        total: 100,
    };
    // First without a picture: a red window, whose picture serves next.
    let red = Look::new(
        &WindowSettings {
            background: Some([200, 0, 0]),
            ..WindowSettings::default()
        },
        None,
    );
    let (picture, pixel) = drawn(&red, &half);
    assert!(near(pixel(5, 300), [200, 0, 0, 255]), "{:?}", pixel(5, 300));
    // The default accent, a blue, fills the gauge's first half.
    assert!(
        near(pixel(100, 218), [61, 123, 217, 255]),
        "{:?}",
        pixel(100, 218)
    );

    let look = Look::new(
        &WindowSettings {
            name: Some("My Game".to_owned()),
            picture: None,
            accent: Some([0xdf, 0xae, 0x4f]),
            background: Some([6, 7, 9]),
        },
        Some(picture),
    );
    let (_, pixel) = drawn(&look, &half);
    assert!(
        near(pixel(5, 300), [6, 7, 9, 255]),
        "background: {:?}",
        pixel(5, 300)
    );
    assert!(
        near(pixel(100, 218), [0xdf, 0xae, 0x4f, 255]),
        "accent: {:?}",
        pixel(100, 218)
    );
    // Past the half: the slot, a little lighter than the background.
    let slot = pixel(500, 218);
    assert!(slot[0] > 6 && slot[0] < 60, "slot: {slot:?}");
    // The picture across the top: the red window, squeezed into 640 × 160.
    assert!(
        near(pixel(5, 120), [200, 0, 0, 255]),
        "picture: {:?}",
        pixel(5, 120)
    );
    assert!(
        near(pixel(5, 170), [6, 7, 9, 255]),
        "under it: {:?}",
        pixel(5, 170)
    );
}

/// The installed version's look (`app/window.toml`, from the channel) wins
/// over the download's (`launcher.toml`'s `[window]`), and a broken one
/// leaves the download's.
#[test]
fn the_installed_versions_look_comes_first() {
    let root = std::env::temp_dir().join(format!("openm2-launcher-looks-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("app/art")).unwrap();
    std::fs::write(
        root.join("launcher.toml"),
        "format = 1
url = \"https://a\"
channel = \"test\"
key = \"ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a\"

[window]
name = \"From the download\"
accent = \"#010203\"
",
    )
    .unwrap();
    let log = Log::new();
    let look = load_look(&root, &log);
    assert_eq!(look.name, "From the download");
    assert_eq!(look.accent, [1, 2, 3]);

    // A picture of the window itself serves as the version's banner.
    let (png, _) = drawn(&look, &Stage::Looking);
    std::fs::write(root.join("app/art/banner.png"), png).unwrap();
    std::fs::write(
        root.join("app/window.toml"),
        "format = 1
name = \"From the channel\"
picture = \"art/banner.png\"
",
    )
    .unwrap();
    let look = load_look(&root, &log);
    assert_eq!(look.name, "From the channel");
    assert!(look.picture.is_some(), "read from app/");
    assert_eq!(
        look.accent,
        Look::default().accent,
        "the whole look is the version's"
    );

    std::fs::write(
        root.join("app/window.toml"),
        "format = 1
logo = 1
",
    )
    .unwrap();
    assert_eq!(load_look(&root, &log).name, "From the download");
    std::fs::remove_dir_all(&root).unwrap();
}

/// The local saves' rows, the one chosen marked in the game's accent.
#[test]
fn the_save_chosen_is_marked_in_the_accent() {
    let look = Look::new(
        &WindowSettings {
            accent: Some([0xdf, 0xae, 0x4f]),
            background: Some([6, 7, 9]),
            ..WindowSettings::default()
        },
        None,
    );
    let rows = ["Lume", "Spring"].map(|name| SaveRow {
        name: name.to_owned(),
        detail: "new".to_owned(),
    });
    let stage = Stage::Saves {
        rows: rows.to_vec(),
        first: 0,
        selected: Some(1),
    };
    let (_, pixel) = drawn(&look, &stage);
    // The second row (from y 242), its mark at its left edge.
    assert!(
        near(pixel(33, 256), [0xdf, 0xae, 0x4f, 255]),
        "mark: {:?}",
        pixel(33, 256)
    );
    assert!(
        !near(pixel(33, 226), [0xdf, 0xae, 0x4f, 255]),
        "the first isn't chosen: {:?}",
        pixel(33, 226)
    );
    // A row is a little lighter than the background; between rows, none.
    let row = pixel(300, 226);
    assert!(row[0] > 6 && row[0] < 40, "row: {row:?}");
    assert!(
        near(pixel(300, 300), [6, 7, 9, 255]),
        "{:?}",
        pixel(300, 300)
    );
}
