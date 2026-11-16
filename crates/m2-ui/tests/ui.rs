//! The interface without fonts or a GPU: layouts from RON, validation,
//! placement, input and events, and the quads the skins draw.

use m2_ui::{
    Cooldown, DrawList, Fonts, ImageId, Key, Label, LabelImage, Layout, Length, PointerButton,
    Rect, Skins, SlotItem, Sprite, TextureRef, Ui, UiError, UiEvent,
};

const SKINS: &str = r#"(
    font: (families: ["Tahoma"], size: 12),
    text_colour: (230, 230, 230, 255),
    skins: {
        "board": Frame(
            corners: ("lt.tga", "rt.tga", "lb.tga", "rb.tga"),
            edges: ("top.tga", "bottom.tga", "left.tga", "right.tga"),
            fill: "base.tga",
            fallback: (10, 10, 10, 200),
        ),
        "dark": Colour((0, 0, 0, 128)),
        "slot": Bar(left: "sl.tga", middle: "sm.tga", right: "sr.tga", fallback: (40, 40, 40, 255)),
        "button": Button(normal: "b1.sub", hover: "b2.sub", down: "b3.sub",
                         text_colour: (255, 255, 255, 255), fallback: (100, 80, 60, 255)),
        "red": Gauge(slot: "slot", fill: "red.tga", inset: (2, 2, 2, 2), fallback: (200, 0, 0, 255)),
    },
)"#;

const LAYOUT: &str = r#"(
    windows: [
        (
            id: "login",
            anchor: Center,
            size: (200, 120),
            kind: Panel(skin: "board"),
            children: [
                (id: "login.title", pos: (0, 4), anchor: Top, size: (Fill(10), 16), kind: Text(text: "Login", align: Center)),
                (id: "login.user", pos: (10, 30), size: (Fill(10), 18), kind: Edit(skin: "dark", max_chars: 8)),
                (id: "login.password", pos: (10, 52), size: (Fill(10), 18), kind: Edit(max_chars: 16, secret: true)),
                (id: "login.ok", anchor: BottomRight, pos: (10, 8), size: (60, 20), kind: Button(skin: "button", text: "OK")),
            ],
        ),
        (
            id: "hud",
            pos: (4, 4),
            size: (104, 14),
            kind: Gauge(skin: "red", value: 0.5),
        ),
        (
            id: "chat",
            anchor: BottomLeft,
            size: (300, 60),
            hidden: true,
            click_through: true,
            kind: Group,
            children: [
                (id: "chat.log", size: (Fill(0), 40), kind: Log(max_lines: 3)),
                (id: "chat.input", anchor: BottomLeft, size: (Fill(0), 20), kind: Edit(max_chars: 10)),
            ],
        ),
    ],
)"#;

fn ui() -> Ui {
    let mut ui = Ui::new(Skins::parse(SKINS).unwrap(), Fonts::empty());
    ui.add(&Layout::parse(LAYOUT).unwrap()).unwrap();
    ui.layout(800.0, 600.0);
    ui
}

fn sprite(id: u32, w: f32, h: f32) -> Sprite {
    Sprite {
        image: ImageId(id),
        uv: Rect::new(0.0, 0.0, 1.0, 1.0),
        size: (w, h),
    }
}

/// Clicks at a point: move, press, release.
fn click(ui: &mut Ui, x: f32, y: f32) -> bool {
    ui.pointer_moved(x, y);
    let over = ui.press();
    ui.release();
    over
}

#[test]
fn layouts_parse_and_nodes_are_placed_by_anchor() {
    let layout = Layout::parse(LAYOUT).unwrap();
    assert_eq!(layout.windows.len(), 3);
    let mut ui = ui();
    // The login board is centred: (300, 240) to (500, 360). Its OK button sits
    // 10 px from the right and 8 px from the bottom.
    assert!(ui.is_over(300.0, 240.0) && !ui.is_over(299.0, 240.0));
    assert!(click(&mut ui, 500.0 - 10.0 - 30.0, 360.0 - 8.0 - 10.0));
    assert_eq!(ui.take_events(), [UiEvent::Clicked("login.ok".to_owned())]);
    // the hidden chat takes no clicks; shown, only its field does: clicks on
    // its log go through to the game
    assert!(!ui.is_over(10.0, 590.0));
    ui.set_visible("chat", true);
    assert!(ui.is_over(10.0, 590.0));
    assert!(!click(&mut ui, 10.0, 550.0));
    assert!(ui.is_visible("chat.log") && !ui.is_over(10.0, 550.0));
}

#[test]
fn a_window_takes_a_new_size_and_its_children_follow() {
    let mut ui = ui();
    // The login board 40 px shorter: centred, (300, 260) to (500, 340), its
    // OK button still 8 px from the bottom.
    assert!(ui.set_size("login", (Length::Px(200.0), Length::Px(80.0))));
    ui.layout(800.0, 600.0);
    assert!(!ui.is_over(300.0, 250.0) && ui.is_over(300.0, 260.0));
    assert!(click(&mut ui, 500.0 - 10.0 - 30.0, 340.0 - 8.0 - 10.0));
    assert_eq!(ui.take_events(), [UiEvent::Clicked("login.ok".to_owned())]);
    assert!(!ui.set_size("nowhere", (Length::Px(1.0), Length::Px(1.0))));
}

#[test]
fn bad_layouts_are_refused() {
    let skins = || Skins::parse(SKINS).unwrap();
    let add = |text: &str| Ui::new(skins(), Fonts::empty()).add(&Layout::parse(text).unwrap());
    assert_eq!(
        add(r#"(windows: [(id: "a", size: (1, 1), kind: Panel(skin: "nope"))])"#),
        Err(UiError::UnknownSkin {
            skin: "nope".to_owned(),
            used_by: "node `a`".to_owned()
        })
    );
    assert!(matches!(
        add(r#"(windows: [(size: (1, 1), kind: Button(skin: "board"))])"#),
        Err(UiError::WrongSkin {
            expected: "button",
            ..
        })
    ));
    assert!(matches!(
        add(r#"(windows: [(size: (1, 1), kind: Panel(skin: "red"))])"#),
        Err(UiError::WrongSkin {
            expected: "panel",
            ..
        })
    ));
    assert_eq!(
        add(
            r#"(windows: [(id: "a", size: (1, 1), kind: Group), (id: "a", size: (1, 1), kind: Group)])"#
        ),
        Err(UiError::DuplicateId("a".to_owned()))
    );
    let mut ui = ui();
    assert_eq!(
        ui.add(&Layout::parse(r#"(windows: [(id: "hud", size: (1, 1), kind: Group)])"#).unwrap()),
        Err(UiError::DuplicateId("hud".to_owned()))
    );
    assert!(matches!(
        Layout::parse(r#"(windows: [(size: (1, 1), kind: Group, colour: 3)])"#),
        Err(UiError::Parse { what: "layout", .. })
    ));
    assert!(matches!(
        Skins::parse(&SKINS.replace(r#"slot: "slot""#, r#"slot: "none""#)),
        Err(UiError::UnknownSkin { .. })
    ));
}

#[test]
fn a_button_clicks_only_when_pressed_and_released_on_it() {
    let mut ui = ui();
    ui.pointer_moved(450.0, 340.0);
    assert!(ui.press());
    ui.pointer_moved(10.0, 10.0);
    ui.release();
    assert!(ui.take_events().is_empty(), "released elsewhere");
    // a press outside every window is the game's
    assert!(!click(&mut ui, 700.0, 100.0));
    assert!(ui.take_events().is_empty());
}

#[test]
fn fields_take_typing_and_submit() {
    let mut ui = ui();
    assert!(!ui.key(Key::Enter), "no field has the keyboard yet");
    click(&mut ui, 320.0, 280.0);
    assert_eq!(ui.focused(), Some("login.user"));
    assert!(ui.text_input("admin\u{8}\nistrator"));
    assert_eq!(
        ui.text("login.user"),
        Some("administ"),
        "control characters dropped, 8 at most"
    );
    ui.key(Key::Backspace);
    ui.key(Key::Home);
    ui.key(Key::Delete);
    ui.key(Key::End);
    ui.key(Key::Left);
    ui.text_input("X");
    assert_eq!(ui.text("login.user"), Some("dminiXs"));
    ui.key(Key::Enter);
    assert_eq!(
        ui.take_events(),
        [UiEvent::Submitted {
            id: "login.user".to_owned(),
            text: "dminiXs".to_owned()
        }]
    );
    ui.key(Key::Tab);
    assert_eq!(ui.focused(), Some("login.password"));
    ui.text_input("pä");
    assert_eq!(ui.text("login.password"), Some("pä"));
    ui.key(Key::Tab);
    assert_eq!(
        ui.focused(),
        Some("login.user"),
        "Tab goes round the window's fields"
    );
    ui.key(Key::Escape);
    assert_eq!(ui.focused(), None);
    assert_eq!(
        ui.take_events(),
        [UiEvent::Cancelled("login.user".to_owned())]
    );
    ui.focus("login.user");
    click(&mut ui, 700.0, 100.0);
    assert_eq!(
        ui.focused(),
        None,
        "a click elsewhere takes the keyboard away"
    );
    assert!(ui.set_text("login.user", "a very long name"));
    assert_eq!(ui.text("login.user"), Some("a very l"));
}

#[test]
fn pasting_puts_one_line_at_the_caret_and_copying_takes_the_field() {
    let mut ui = ui();
    assert!(!ui.paste("x"), "no field has the keyboard");
    assert_eq!(ui.copy(), None);
    ui.set_visible("chat", true);
    ui.focus("chat.input");
    ui.text_input("az");
    ui.key(Key::Left);
    assert!(ui.paste("\r\nb\tc\r\nd\n"));
    assert_eq!(
        ui.text("chat.input"),
        Some("ab c dz"),
        "line breaks and tabs as spaces, those at the ends left out"
    );
    ui.paste("0123456789");
    assert_eq!(ui.text("chat.input"), Some("ab c d012z"), "10 at most");
    assert_eq!(ui.copy().as_deref(), Some("ab c d012z"));
    assert!(ui.take_events().is_empty(), "nothing is submitted");
    ui.focus("login.password");
    ui.paste("hunter2\n");
    assert_eq!(ui.copy(), None, "never from a secret field");
    assert_eq!(ui.take_text("login.password").as_deref(), Some("hunter2"));
}

#[test]
fn gauges_and_logs_keep_their_values() {
    let mut ui = ui();
    assert!(ui.set_value("hud", 2.0));
    assert_eq!(ui.value("hud"), Some(1.0), "clamped");
    assert!(!ui.set_value("login.ok", 0.5), "not a gauge");
    assert_eq!(ui.value("login.ok"), None);
    ui.set_visible("chat", true);
    for line in ["one", "two", "three", "four"] {
        assert!(ui.push_line("chat.log", line, [255; 4]));
    }
    assert_eq!(ui.lines("chat.log"), ["two", "three", "four"]);
    ui.pointer_moved(10.0, 550.0);
    assert!(ui.wheel(5.0), "the wheel over a log scrolls it");
    assert!(!ui.set_text("hud", "x"));
    assert!(!ui.has("nothing") && ui.has("chat"));
    assert!(ui.clear_lines("chat.log"));
    assert!(ui.lines("chat.log").is_empty());
    assert!(ui.push_line("chat.log", "five", [255; 4]));
    assert_eq!(ui.lines("chat.log"), ["five"]);
    assert!(!ui.clear_lines("hud"), "not a log");
}

#[test]
fn image_paths_list_what_the_skins_and_images_name() {
    let ui = ui();
    let paths = ui.image_paths();
    assert!(paths.contains("lt.tga") && paths.contains("b3.sub") && paths.contains("red.tga"));
    assert_eq!(paths.len(), 9 + 3 + 3 + 1);
}

fn quads_of(list: &DrawList, texture: TextureRef) -> Vec<[f32; 4]> {
    list.batches
        .iter()
        .filter(|b| b.texture == texture)
        .flat_map(|b| {
            list.vertices[b.vertices.start as usize..b.vertices.end as usize]
                .chunks(6)
                .map(|q| [q[0].pos[0], q[0].pos[1], q[5].pos[0], q[5].pos[1]])
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn skins_without_art_draw_their_fallback_colours() {
    let mut ui = ui();
    let list = ui.draw(800.0, 600.0).0.clone();
    // board, the user field's dark panel, the button, the gauge's slot and
    // fill: flat quads only (no text without fonts).
    assert!(list.batches.iter().all(|b| b.texture == TextureRef::White));
    let quads = quads_of(&list, TextureRef::White);
    assert_eq!(
        quads[0],
        [300.0, 240.0, 500.0, 360.0],
        "the board's fallback"
    );
    assert!(quads.contains(&[4.0, 4.0, 108.0, 18.0]), "the gauge's slot");
    assert!(
        quads.contains(&[6.0, 6.0, 56.0, 16.0]),
        "half of the gauge's 100 px fill"
    );
    let colours: Vec<_> = list.vertices.iter().step_by(6).map(|v| v.colour).collect();
    assert!(colours.contains(&[100, 80, 60, 255]), "the button at rest");
    ui.pointer_moved(450.0, 340.0);
    let list = ui.draw(800.0, 600.0).0.clone();
    let colours: Vec<_> = list.vertices.iter().step_by(6).map(|v| v.colour).collect();
    assert!(
        colours.contains(&[125, 100, 75, 255]),
        "lighter under the pointer"
    );
}

#[test]
fn a_board_draws_corners_edges_and_its_middle() {
    let mut ui = ui();
    for (n, path) in ["lt.tga", "rt.tga", "lb.tga", "rb.tga"].iter().enumerate() {
        ui.set_sprite(path, sprite(n as u32, 16.0, 16.0));
    }
    ui.set_sprite("top.tga", sprite(10, 32.0, 16.0));
    ui.set_sprite("bottom.tga", sprite(11, 32.0, 16.0));
    ui.set_sprite("left.tga", sprite(12, 16.0, 32.0));
    ui.set_sprite("right.tga", sprite(13, 16.0, 32.0));
    ui.set_sprite("base.tga", sprite(14, 128.0, 128.0));
    ui.set_sprite("b2.sub", sprite(20, 60.0, 20.0));
    let list = ui.draw(800.0, 600.0).0.clone();
    let image = |id| TextureRef::Image(ImageId(id));
    assert_eq!(
        quads_of(&list, image(14)),
        [[316.0, 256.0, 444.0, 344.0], [444.0, 256.0, 484.0, 344.0]]
    );
    assert_eq!(quads_of(&list, image(0)), [[300.0, 240.0, 316.0, 256.0]]);
    assert_eq!(quads_of(&list, image(3)), [[484.0, 344.0, 500.0, 360.0]]);
    // top edge: 168 px between the corners in 32 px tiles, the last one cut
    let top = quads_of(&list, image(10));
    assert_eq!(top.len(), 6);
    assert_eq!(top[5], [476.0, 240.0, 484.0, 256.0]);
    assert_eq!(quads_of(&list, image(12)).len(), 3);
    // hover look of the button
    assert!(quads_of(&list, image(20)).is_empty());
    ui.pointer_moved(450.0, 340.0);
    let list = ui.draw(800.0, 600.0).0.clone();
    assert_eq!(quads_of(&list, image(20)), [[430.0, 332.0, 490.0, 352.0]]);
}

#[test]
fn a_secret_field_keeps_its_text_out_of_events_and_debug_output() {
    let mut ui = ui();
    ui.focus("login.password");
    ui.text_input("hunter2");
    ui.key(Key::Enter);
    assert_eq!(
        ui.take_events(),
        [UiEvent::Submitted {
            id: "login.password".to_owned(),
            text: String::new()
        }]
    );
    let debug = format!("{ui:?}");
    assert!(!debug.contains("hunter2"), "{debug}");
    assert!(debug.contains("<redacted>"));
    assert_eq!(ui.take_text("login.password").as_deref(), Some("hunter2"));
    assert_eq!(ui.text("login.password"), Some(""), "taking empties it");
    assert_eq!(ui.take_text("login.ok"), None, "not a field");
}

#[test]
fn a_tile_skin_repeats_its_image_from_the_top_left() {
    let skins = SKINS.replace(
        r#""dark": Colour((0, 0, 0, 128)),"#,
        r#""dark": Colour((0, 0, 0, 128)),
        "pattern": Tile(image: "pattern.tga", fallback: (90, 20, 20, 255)),"#,
    );
    let layout =
        r#"(windows: [(id: "back", size: (Fill(0), Fill(0)), kind: Panel(skin: "pattern"))])"#;
    let mut ui = Ui::new(Skins::parse(&skins).unwrap(), Fonts::empty());
    ui.add(&Layout::parse(layout).unwrap()).unwrap();
    assert!(ui.image_paths().contains("pattern.tga"));
    let list = ui.draw(300.0, 200.0).0.clone();
    assert_eq!(
        quads_of(&list, TextureRef::White),
        [[0.0, 0.0, 300.0, 200.0]],
        "the fallback colour without the art"
    );
    ui.set_sprite("pattern.tga", sprite(7, 128.0, 128.0));
    let list = ui.draw(300.0, 200.0).0.clone();
    let quads = quads_of(&list, TextureRef::Image(ImageId(7)));
    assert_eq!(quads.len(), 3 * 2, "3 across, 2 down");
    assert_eq!(quads[0], [0.0, 0.0, 128.0, 128.0]);
    assert!(
        quads.contains(&[256.0, 128.0, 300.0, 200.0]),
        "the last one cut"
    );
}

/// An orb filled from the bottom in a bar, a window over the bar's corner,
/// and a hidden orb.
const ORBS: &str = r#"(
    windows: [
        (
            id: "bar",
            pos: (100, 100),
            size: (60, 30),
            kind: Panel(skin: "dark"),
            children: [
                (id: "bar.orb", pos: (5, 5), size: (20, 20), kind: Gauge(skin: "orb", value: 0.65, from: Bottom)),
                (id: "bar.hidden", pos: (30, 5), size: (20, 20), hidden: true, kind: Gauge(skin: "orb", from: Bottom)),
            ],
        ),
        (id: "over", pos: (100, 100), size: (12, 12), kind: Panel(skin: "dark")),
    ],
)"#;

fn orbs() -> Ui {
    let skins = SKINS.replace(
        r#""dark": Colour((0, 0, 0, 128)),"#,
        r#""dark": Colour((0, 0, 0, 128)),
        "orb": Gauge(fill: "orb.tga", fallback: (200, 160, 40, 255)),"#,
    );
    let mut ui = Ui::new(Skins::parse(&skins).unwrap(), Fonts::empty());
    ui.add(&Layout::parse(ORBS).unwrap()).unwrap();
    ui
}

#[test]
fn a_gauge_from_the_bottom_fills_upwards() {
    let mut ui = orbs();
    // The orb's 20 px from y 105 to 125, 13 px of them filled.
    let list = ui.draw(800.0, 600.0).0.clone();
    assert!(
        quads_of(&list, TextureRef::White).contains(&[105.0, 112.0, 125.0, 125.0]),
        "the fallback colour without the art"
    );
    // An 8 px pattern from the bottom up: a whole one, then the lower 5 px
    // of the next.
    ui.set_sprite("orb.tga", sprite(7, 20.0, 8.0));
    let list = ui.draw(800.0, 600.0).0.clone();
    assert_eq!(
        quads_of(&list, TextureRef::Image(ImageId(7))),
        [[105.0, 117.0, 125.0, 125.0], [105.0, 112.0, 125.0, 117.0]]
    );
    let top = list
        .vertices
        .iter()
        .find(|v| v.pos == [105.0, 112.0])
        .unwrap();
    assert_eq!(top.uv, [0.0, 0.375], "the pattern's lower part");
    ui.set_value("bar.orb", 0.0);
    let list = ui.draw(800.0, 600.0).0.clone();
    assert!(quads_of(&list, TextureRef::Image(ImageId(7))).is_empty());
}

#[test]
fn panels_and_gauges_take_other_looks_of_their_kind() {
    let mut ui = orbs();
    assert_eq!(ui.look("bar"), Some("dark"));
    assert!(ui.set_look("bar", "board"), "a panel takes a board");
    assert!(ui.set_look("bar.orb", "red"), "a gauge another gauge");
    assert_eq!(ui.look("bar.orb"), Some("red"));
    let list = ui.draw(800.0, 600.0).0.clone();
    // The red gauge's slot (a bar) and its inset fill, 65 % of 16 px from
    // the bottom.
    let flat = quads_of(&list, TextureRef::White);
    assert!(flat.contains(&[105.0, 105.0, 125.0, 125.0]), "{flat:?}");
    assert!(flat.contains(&[107.0, 113.0, 123.0, 123.0]), "{flat:?}");
    assert!(!ui.set_look("bar", "red"), "a panel takes no gauge");
    assert!(!ui.set_look("bar.orb", "dark"), "a gauge no panel");
    assert!(!ui.set_look("bar", "button") && !ui.set_look("bar", "nope"));
    assert!(!ui.set_look("nothing", "dark"));
}

#[test]
fn a_node_is_hovered_only_where_its_window_is_on_top() {
    let mut ui = orbs();
    ui.layout(800.0, 600.0);
    assert!(!ui.is_hovered("bar.orb"), "no pointer yet");
    ui.pointer_moved(110.0, 120.0);
    assert!(ui.is_hovered("bar.orb") && ui.is_hovered("bar"));
    ui.pointer_moved(106.0, 106.0);
    assert!(!ui.is_hovered("bar.orb"), "the window over it");
    assert!(ui.is_hovered("over"));
    ui.pointer_moved(135.0, 110.0);
    assert!(!ui.is_hovered("bar.hidden") && !ui.is_hovered("bar.orb"));
    ui.set_visible("bar", false);
    ui.pointer_moved(110.0, 120.0);
    assert!(!ui.is_hovered("bar.orb"), "its window hidden");
    ui.set_visible("bar", true);
    ui.pointer_left();
    assert!(!ui.is_hovered("bar.orb") && !ui.is_hovered("nothing"));
}

#[test]
fn pictures_are_the_applications_and_nodes_tell_where_they_are() {
    let layout = r#"(windows: [(id: "model", anchor: Center, pos: (-10, 0), size: (100, 200), kind: Image(image: "@character"))])"#;
    let mut ui = ui();
    ui.add(&Layout::parse(layout).unwrap()).unwrap();
    assert!(
        !ui.image_paths().iter().any(|p| p.starts_with('@')),
        "not a game path"
    );
    ui.layout(800.0, 600.0);
    assert_eq!(
        ui.rect("model"),
        Some(Rect::new(340.0, 200.0, 100.0, 200.0))
    );
    assert_eq!(ui.rect("nothing"), None);
    ui.set_sprite("@character", sprite(9, 100.0, 200.0));
    let list = ui.draw(800.0, 600.0).0.clone();
    assert_eq!(
        quads_of(&list, TextureRef::Image(ImageId(9))),
        [[340.0, 200.0, 440.0, 400.0]]
    );
}

/// A bag of 2 × 3 cells of 32 px at (100, 100), with a slot skin, and a
/// single worn slot two cells tall beside it.
const BAG: &str = r#"(windows: [(id: "bag", pos: (90, 90), size: (200, 200), kind: Panel(skin: "dark"), children: [
    (id: "bag.cells", pos: (10, 10), size: (64, 96), kind: Slots(skin: "dark", columns: 2, rows: 3)),
    (id: "bag.weapon", pos: (100, 10), size: (32, 96), kind: Slots()),
])])"#;

fn bag() -> Ui {
    let mut ui = ui();
    ui.add(&Layout::parse(BAG).unwrap()).unwrap();
    ui.layout(800.0, 600.0);
    ui
}

fn icon(path: &str, height: u16, count: u32) -> Option<SlotItem> {
    Some(SlotItem {
        height,
        count,
        ..SlotItem::new(path)
    })
}

#[test]
fn slot_grids_hold_items_and_report_cells() {
    let mut ui = bag();
    assert!(ui.set_slot("bag.cells", 1, icon("sword.tga", 2, 1)));
    assert!(ui.set_slot("bag.cells", 4, icon("potion.tga", 1, 5)));
    assert!(!ui.set_slot("bag.cells", 6, icon("x.tga", 1, 1)), "2 × 3");
    assert!(!ui.set_slot("login", 0, icon("x.tga", 1, 1)), "not slots");
    assert_eq!(ui.slot("bag.cells", 4).map(|i| i.count), Some(5));
    // Pressing the lower half of the sword is pressing the sword.
    ui.pointer_moved(140.0, 140.0);
    assert_eq!(ui.hovered_slot(), Some(("bag.cells", 1)));
    assert!(ui.press());
    ui.pointer_moved(110.0, 170.0);
    assert!(ui.release());
    assert_eq!(
        ui.take_events(),
        [
            UiEvent::SlotPressed {
                id: "bag.cells".to_owned(),
                cell: 1,
                button: PointerButton::Main
            },
            UiEvent::SlotReleased {
                id: "bag.cells".to_owned(),
                cell: 4
            },
        ]
    );
    // The secondary button reports presses too; outside the cells nothing.
    ui.pointer_moved(200.0, 150.0);
    assert!(ui.press_secondary());
    ui.pointer_moved(250.0, 250.0);
    assert!(ui.press_secondary());
    assert_eq!(
        ui.take_events(),
        [UiEvent::SlotPressed {
            id: "bag.weapon".to_owned(),
            cell: 0,
            button: PointerButton::Secondary
        }]
    );
    assert_eq!(ui.hovered_slot(), None);
    // Hiding the window under the pointer ends the hover.
    ui.pointer_moved(110.0, 110.0);
    assert_eq!(ui.hovered_slot(), Some(("bag.cells", 0)));
    ui.set_visible("bag", false);
    assert_eq!(ui.hovered_slot(), None);
    ui.set_visible("bag", true);
    assert!(ui.clear_slots("bag.cells"));
    assert_eq!(ui.slot("bag.cells", 1), None);
}

#[test]
fn bad_slot_grids_are_refused() {
    let add = |kind: &str| {
        let text = format!(r#"(windows: [(id: "s", size: (32, 32), kind: {kind})])"#);
        Ui::new(Skins::parse(SKINS).unwrap(), Fonts::empty()).add(&Layout::parse(&text).unwrap())
    };
    assert!(add("Slots(columns: 5, rows: 9)").is_ok());
    assert_eq!(
        add("Slots(columns: 0)"),
        Err(UiError::SlotCount {
            used_by: "node `s`".to_owned(),
            cells: 0,
            max: 1024
        })
    );
    assert!(matches!(
        add("Slots(columns: 100, rows: 100)"),
        Err(UiError::SlotCount { cells: 10_000, .. })
    ));
    assert!(matches!(
        add(r#"Slots(skin: "button")"#),
        Err(UiError::WrongSkin {
            expected: "panel",
            ..
        })
    ));
}

#[test]
fn items_draw_centred_with_their_count_and_the_held_one_follows_the_pointer() {
    let mut ui = bag();
    ui.set_slot("bag.cells", 1, icon("sword.tga", 2, 1));
    ui.set_slot("bag.weapon", 0, icon("sword.tga", 2, 1));
    ui.set_slot("bag.cells", 4, icon("potion.tga", 1, 5));
    let missing = ui.missing_images();
    assert!(missing.contains("sword.tga") && missing.contains("potion.tga"));
    ui.set_sprite("sword.tga", sprite(3, 32.0, 64.0));
    assert!(!ui.missing_images().contains("sword.tga"));
    ui.pointer_moved(700.0, 500.0);
    let list = ui.draw(800.0, 600.0).0.clone();
    // In the bag the sword fills its two cells; in the worn slot (96 high)
    // it is centred.
    assert_eq!(
        quads_of(&list, TextureRef::Image(ImageId(3))),
        [[132.0, 100.0, 164.0, 164.0], [190.0, 116.0, 222.0, 180.0]]
    );
    // The potion has no art yet: a mark instead (fonts are off, so no
    // count is drawn).
    assert!(quads_of(&list, TextureRef::White).contains(&[104.0, 168.0, 128.0, 192.0]));

    // Carried, it is drawn over everything at the pointer, dimmed where it
    // was; a tooltip waits until nothing is carried.
    ui.set_slot(
        "bag.cells",
        1,
        Some(SlotItem {
            dimmed: true,
            ..icon("sword.tga", 2, 1).unwrap()
        }),
    );
    ui.set_held(icon("sword.tga", 2, 1));
    ui.set_tooltip(vec![("Sword+0".to_owned(), [255, 255, 255, 255])]);
    let list = ui.draw(800.0, 600.0).0.clone();
    let swords = quads_of(&list, TextureRef::Image(ImageId(3)));
    assert_eq!(swords.last(), Some(&[684.0, 468.0, 716.0, 532.0]));
    let tints: Vec<_> = list
        .batches
        .iter()
        .filter(|b| b.texture == TextureRef::Image(ImageId(3)))
        .flat_map(|b| list.vertices[b.vertices.start as usize..b.vertices.end as usize].chunks(6))
        .map(|q| q[0].colour)
        .collect();
    assert_eq!(
        tints,
        [[110, 110, 110, 255], [255; 4], [255; 4]],
        "dimmed in the bag, plain in the worn slot and at the pointer"
    );
    let before = list.quads();
    ui.set_held(None);
    let list = ui.draw(800.0, 600.0).0.clone();
    assert_eq!(list.quads(), before, "the tooltip box instead of the sword");
}

#[test]
fn a_cooling_item_is_darkened_by_what_is_left() {
    let mut ui = bag();
    ui.set_slot(
        "bag.cells",
        0,
        Some(SlotItem {
            cooldown: Some(Cooldown {
                left: 250,
                seconds: 3,
            }),
            ..icon("potion.tga", 1, 1).unwrap()
        }),
    );
    let list = ui.draw(800.0, 600.0).0.clone();
    let shades: Vec<_> = list
        .batches
        .iter()
        .filter(|b| b.texture == TextureRef::White)
        .flat_map(|b| list.vertices[b.vertices.start as usize..b.vertices.end as usize].chunks(6))
        .filter(|q| q[0].colour == [0, 0, 0, 160])
        .map(|q| q[5].pos[1] - q[0].pos[1])
        .collect();
    // A quarter left: the top quarter of the 32 px cell.
    assert_eq!(shades, [8.0]);
}

#[test]
fn tooltips_stay_on_the_screen() {
    let mut ui = bag();
    ui.set_tooltip(vec![
        ("one".to_owned(), [255; 4]),
        ("two".to_owned(), [255; 4]),
    ]);
    // No fonts: the box is the padding and two lines of 15 px.
    ui.pointer_moved(100.0, 100.0);
    let list = ui.draw(800.0, 600.0).0.clone();
    let boxes = quads_of(&list, TextureRef::White);
    assert_eq!(boxes.last(), Some(&[114.0, 118.0, 126.0, 160.0]));
    ui.pointer_moved(795.0, 590.0);
    let list = ui.draw(800.0, 600.0).0.clone();
    let boxes = quads_of(&list, TextureRef::White);
    assert_eq!(boxes.last(), Some(&[777.0, 558.0, 789.0, 600.0]));
    // The skin file may name a panel skin for them.
    let skins = SKINS.replace("skins: {", r#"tooltip: "dark", skins: {"#);
    assert_eq!(
        Skins::parse(&skins).unwrap().tooltip.as_deref(),
        Some("dark")
    );
    let skins = SKINS.replace("skins: {", r#"tooltip: "button", skins: {"#);
    assert!(matches!(
        Skins::parse(&skins),
        Err(UiError::WrongSkin { .. })
    ));
}

#[test]
fn labels_are_kept_until_replaced_and_are_only_text() {
    let mut ui = Ui::new(Skins::parse(SKINS).unwrap(), Fonts::empty());
    let label = Label {
        at: (400.0, 300.0),
        text: "536".to_owned(),
        colour: [255, 230, 120, 255],
        size: 0.0,
        lead: None,
        image: None,
    };
    let named = Label {
        text: "Wild Dog".to_owned(),
        lead: Some(("Lv 1 ".to_owned(), [235, 205, 110, 255])),
        ..label.clone()
    };
    ui.set_labels(vec![label.clone(), named.clone()]);
    assert_eq!(ui.labels(), [label, named]);
    // No fonts: a label draws nothing, and takes no clicks.
    assert_eq!(ui.draw(800.0, 600.0).0.quads(), 0);
    assert!(!ui.is_over(400.0, 300.0));
    ui.set_labels(Vec::new());
    assert!(ui.labels().is_empty());
}

#[test]
fn a_labels_picture_stands_on_its_line_and_the_text_shows_without_it() {
    let skins = SKINS.replace(
        "skins: {",
        r#"skins: { "mark": Image(image: "mark.tga", fallback: (0, 0, 0, 0)),"#,
    );
    let mut ui = Ui::new(Skins::parse(&skins).unwrap(), Fonts::empty());
    let label = Label {
        at: (400.0, 300.0),
        text: "GM".to_owned(),
        colour: [255, 185, 40, 255],
        size: 0.0,
        lead: None,
        image: Some(LabelImage {
            look: "mark".to_owned(),
            size: (64.0, 33.0),
            alpha: 128,
        }),
    };
    ui.set_labels(vec![label]);
    // Without its image the text shows: no fonts here, so nothing.
    assert_eq!(ui.draw(800.0, 600.0).0.quads(), 0);
    ui.set_sprite("mark.tga", sprite(30, 246.0, 127.0));
    let list = ui.draw(800.0, 600.0).0.clone();
    // Its line of 12 px text is 15 px high, centred on 300: the picture's
    // bottom is at 307.5, at the size asked, not the image's.
    let picture = TextureRef::Image(ImageId(30));
    assert_eq!(quads_of(&list, picture), [[368.0, 275.0, 432.0, 308.0]]);
    assert!(
        list.vertices
            .iter()
            .all(|v| v.colour == [255, 255, 255, 128])
    );
    // Twice as large at a scale of 2.
    ui.set_scale(2.0);
    let list = ui.draw(800.0, 600.0).0.clone();
    assert_eq!(quads_of(&list, picture), [[336.0, 249.0, 464.0, 315.0]]);
}

#[test]
fn the_label_under_a_point_is_the_last_drawn_there() {
    let skins = SKINS.replace(
        "skins: {",
        r#"skins: { "mark": Image(image: "mark.tga", fallback: (0, 0, 0, 0)),"#,
    );
    let mut ui = Ui::new(Skins::parse(&skins).unwrap(), Fonts::empty());
    ui.set_sprite("mark.tga", sprite(30, 246.0, 127.0));
    let picture = |at: (f32, f32), size: (f32, f32)| Label {
        at,
        text: "GM".to_owned(),
        colour: [255, 255, 255, 255],
        size: 0.0,
        lead: None,
        image: Some(LabelImage {
            look: "mark".to_owned(),
            size,
            alpha: 255,
        }),
    };
    // 368 to 432 across, 275 to 308 down (as drawn above); a smaller one
    // drawn over its right half, 400 to 440 and 288 to 308.
    ui.set_labels(vec![
        picture((400.0, 300.0), (64.0, 33.0)),
        picture((420.0, 300.0), (40.0, 20.0)),
    ]);
    assert_eq!(ui.label_at(370.0, 280.0), Some(0));
    assert_eq!(ui.label_at(410.0, 300.0), Some(1), "the one on top");
    assert_eq!(ui.label_at(410.0, 280.0), Some(0));
    assert_eq!(ui.label_at(436.0, 300.0), Some(1));
    assert_eq!(ui.label_at(450.0, 300.0), None);
    assert_eq!(ui.label_at(400.0, 270.0), None);
    // No fonts: a text has no width, so it is nowhere; nor is an empty
    // label.
    let text = Label {
        image: None,
        ..picture((100.0, 100.0), (0.0, 0.0))
    };
    let empty = Label {
        text: String::new(),
        ..text.clone()
    };
    ui.set_labels(vec![text, empty]);
    assert_eq!(ui.label_at(100.0, 100.0), None);
}

/// With the system's fonts (none on some machines, which skip it): a
/// name's line is found where it is drawn, its lead too.
#[test]
fn a_labels_line_of_text_is_found_where_it_is_drawn() {
    let fonts = Fonts::system(&["Tahoma".to_owned(), "Arial".to_owned()]);
    if fonts.faces() == 0 {
        return;
    }
    let mut ui = Ui::new(Skins::parse(SKINS).unwrap(), fonts);
    let named = Label {
        at: (400.0, 300.0),
        text: "Wild Dog".to_owned(),
        colour: [255, 255, 255, 255],
        size: 0.0,
        lead: Some(("Lv 1 ".to_owned(), [235, 205, 110, 255])),
        image: None,
    };
    ui.set_labels(vec![named]);
    let list = ui.draw(800.0, 600.0).0.clone();
    // The glyphs drawn, shadows and all, lie in the box that is found.
    let mut left = f32::MAX;
    let mut right = f32::MIN;
    for vertex in &list.vertices {
        left = left.min(vertex.pos[0]);
        right = right.max(vertex.pos[0]);
    }
    assert!(left < 380.0 && right > 420.0, "{left} to {right}");
    assert_eq!(ui.label_at(left + 1.0, 300.0), Some(0), "the lead");
    assert_eq!(ui.label_at(right - 2.0, 300.0), Some(0), "the text");
    assert_eq!(ui.label_at(400.0, 300.0), Some(0));
    assert_eq!(ui.label_at(left - 5.0, 300.0), None);
    assert_eq!(ui.label_at(right + 5.0, 300.0), None);
    assert_eq!(ui.label_at(400.0, 320.0), None, "below its line");
}

#[test]
fn windows_come_to_the_top_when_asked() {
    let mut ui = bag();
    let cover =
        r#"(windows: [(id: "cover", pos: (90, 90), size: (200, 200), kind: Panel(skin: "dark"))])"#;
    ui.add(&Layout::parse(cover).unwrap()).unwrap();
    ui.layout(800.0, 600.0);
    ui.pointer_moved(110.0, 110.0);
    ui.press_secondary();
    assert!(
        ui.take_events().is_empty(),
        "the cover is on top of the bag"
    );
    assert!(ui.raise("bag"));
    ui.press_secondary();
    assert_eq!(ui.take_events().len(), 1, "now the bag's cell is pressed");
    assert!(!ui.raise("bag.cells"), "not a window");
    assert!(!ui.raise("nothing"));
}

#[test]
fn a_latched_button_looks_pressed() {
    let mut ui = ui();
    ui.set_sprite("b1.sub", sprite(1, 60.0, 20.0));
    ui.set_sprite("b3.sub", sprite(3, 60.0, 20.0));
    ui.pointer_moved(10.0, 10.0);
    let button = |ui: &mut Ui, image| {
        !quads_of(
            &ui.draw(800.0, 600.0).0.clone(),
            TextureRef::Image(ImageId(image)),
        )
        .is_empty()
    };
    assert!(button(&mut ui, 1));
    assert!(ui.set_latched("login.ok", true));
    assert!(button(&mut ui, 3) && !button(&mut ui, 1));
    assert!(ui.set_latched("login.ok", false));
    assert!(button(&mut ui, 1));
    assert!(!ui.set_latched("login.user", true), "not a button");
}

#[test]
fn a_scaled_interface_grows_its_windows_and_art_and_is_clicked_where_they_are() {
    let mut ui = ui();
    for (n, path) in ["lt.tga", "rt.tga", "lb.tga", "rb.tga"].iter().enumerate() {
        ui.set_sprite(path, sprite(n as u32, 16.0, 16.0));
    }
    ui.set_sprite("top.tga", sprite(10, 32.0, 16.0));
    ui.set_sprite("bottom.tga", sprite(11, 32.0, 16.0));
    ui.set_sprite("left.tga", sprite(12, 16.0, 32.0));
    ui.set_sprite("right.tga", sprite(13, 16.0, 32.0));
    ui.set_sprite("base.tga", sprite(14, 128.0, 128.0));
    ui.set_scale(2.0);
    assert_eq!(ui.scale(), 2.0);
    let list = ui.draw(1600.0, 1200.0).0.clone();
    let image = |id| TextureRef::Image(ImageId(id));
    // The 200 × 120 board twice as large, centred on the twice as large
    // screen; its pieces twice their size.
    assert_eq!(
        ui.rect("login"),
        Some(Rect::new(600.0, 480.0, 400.0, 240.0))
    );
    assert_eq!(quads_of(&list, image(0)), [[600.0, 480.0, 632.0, 512.0]]);
    assert_eq!(quads_of(&list, image(3)), [[968.0, 688.0, 1000.0, 720.0]]);
    let top = quads_of(&list, image(10));
    assert_eq!(top.len(), 6, "336 px in 64 px tiles");
    assert_eq!(top[5], [952.0, 480.0, 968.0, 512.0]);
    assert_eq!(
        quads_of(&list, image(14)),
        [[632.0, 512.0, 888.0, 688.0], [888.0, 512.0, 968.0, 688.0]]
    );
    // The gauge: offset, size and inset doubled, half of its 200 px fill.
    let flat = quads_of(&list, TextureRef::White);
    assert!(flat.contains(&[8.0, 8.0, 216.0, 36.0]), "{flat:?}");
    assert!(flat.contains(&[12.0, 12.0, 112.0, 32.0]), "{flat:?}");
    // The pointer stays in the screen's pixels: the OK button is 20 px from
    // the board's right and 16 px from its bottom now.
    assert_eq!(
        ui.rect("login.ok"),
        Some(Rect::new(860.0, 664.0, 120.0, 40.0))
    );
    assert!(click(&mut ui, 970.0, 690.0));
    assert_eq!(ui.take_events(), [UiEvent::Clicked("login.ok".to_owned())]);
    // Kept within reason.
    ui.set_scale(10.0);
    assert_eq!(ui.scale(), 4.0);
    ui.set_scale(f32::NAN);
    assert_eq!(ui.scale(), 1.0);
}

/// A slider 200 × 12 at (100, 100), its knob an image 40 × 12, at half.
fn slider() -> Ui {
    let skins = r#"(
        font: (families: ["Tahoma"], size: 12),
        text_colour: (230, 230, 230, 255),
        skins: {
            "track": Image(image: "track.sub", fallback: (40, 40, 40, 255)),
            "knob": Image(image: "knob.sub", fallback: (200, 200, 200, 255)),
            "button": Button(normal: "b1.sub", hover: "b2.sub", down: "b3.sub",
                             text_colour: (255, 255, 255, 255), fallback: (100, 80, 60, 255)),
        },
    )"#;
    let layout = r#"(windows: [(
        id: "volume", pos: (100, 100), size: (200, 12),
        kind: Slider(bar: "track", knob: "knob", value: 0.5),
    )])"#;
    let mut ui = Ui::new(Skins::parse(skins).unwrap(), Fonts::empty());
    ui.add(&Layout::parse(layout).unwrap()).unwrap();
    ui.set_sprite("knob.sub", sprite(2, 40.0, 12.0));
    ui.layout(800.0, 600.0);
    ui
}

#[test]
fn a_slider_follows_the_pointer_from_a_press_until_the_release() {
    let mut ui = slider();
    assert_eq!(ui.value("volume"), Some(0.5));
    let changed = || vec![UiEvent::Changed("volume".to_owned())];
    // The knob's middle travels from 120 to 280: the bar less half the knob
    // on each side.
    ui.pointer_moved(160.0, 106.0);
    assert!(ui.press());
    assert_eq!(ui.value("volume"), Some(0.25));
    assert_eq!(ui.take_events(), changed());
    // Dragged past its end and off it: held at the end.
    ui.pointer_moved(500.0, 300.0);
    assert_eq!(ui.value("volume"), Some(1.0));
    ui.pointer_moved(520.0, 300.0);
    assert!(ui.release());
    // One change for the drag, no click, and nothing once let go.
    assert_eq!(ui.take_events(), changed());
    ui.pointer_moved(120.0, 106.0);
    assert_eq!(ui.value("volume"), Some(1.0));
    assert!(ui.take_events().is_empty());
    // The application sets it too, without an event.
    assert!(ui.set_value("volume", 0.0));
    assert_eq!(ui.value("volume"), Some(0.0));
    assert!(ui.take_events().is_empty());
}

#[test]
fn a_slider_draws_its_bar_and_its_knob_where_the_value_is() {
    let mut ui = slider();
    ui.set_value("volume", 0.25);
    let list = ui.draw(800.0, 600.0).0.clone();
    // The bar has no art: its fallback over the node.
    assert_eq!(
        quads_of(&list, TextureRef::White),
        vec![[100.0, 100.0, 300.0, 112.0]]
    );
    // The knob at its own size, a quarter of the way: 100 + 0.25 × 160.
    assert_eq!(
        quads_of(&list, TextureRef::Image(ImageId(2))),
        vec![[140.0, 100.0, 180.0, 112.0]]
    );
    // Twice as large, it is clicked where it is drawn.
    ui.set_scale(2.0);
    ui.layout(800.0, 600.0);
    ui.pointer_moved(200.0 + 40.0 + 320.0, 212.0);
    ui.press();
    assert_eq!(ui.value("volume"), Some(1.0));
}

#[test]
fn a_slider_needs_panel_skins_for_its_bar_and_knob() {
    let skins = r#"(
        font: (families: ["Tahoma"], size: 12),
        text_colour: (230, 230, 230, 255),
        skins: {
            "track": Colour((0, 0, 0, 255)),
            "button": Button(normal: "b1.sub", hover: "b2.sub", down: "b3.sub",
                             text_colour: (255, 255, 255, 255), fallback: (100, 80, 60, 255)),
        },
    )"#;
    let add = |text: &str| {
        Ui::new(Skins::parse(skins).unwrap(), Fonts::empty()).add(&Layout::parse(text).unwrap())
    };
    assert_eq!(
        add(r#"(windows: [(id: "s", size: (9, 9), kind: Slider(bar: "track", knob: "nope"))])"#),
        Err(UiError::UnknownSkin {
            skin: "nope".to_owned(),
            used_by: "node `s`".to_owned()
        })
    );
    assert!(matches!(
        add(r#"(windows: [(size: (9, 9), kind: Slider(bar: "button", knob: "track"))])"#),
        Err(UiError::WrongSkin {
            expected: "panel",
            ..
        })
    ));
    // A colour knob is a square as tall as the slider.
    let mut ui = Ui::new(Skins::parse(skins).unwrap(), Fonts::empty());
    let layout = r#"(windows: [(pos: (0, 0), size: (100, 10), kind: Slider(bar: "track", knob: "track", value: 1.0))])"#;
    ui.add(&Layout::parse(layout).unwrap()).unwrap();
    let list = ui.draw(800.0, 600.0).0.clone();
    assert!(quads_of(&list, TextureRef::White).contains(&[90.0, 0.0, 100.0, 10.0]));
}
