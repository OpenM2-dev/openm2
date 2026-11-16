//! Walk masks: squares, straight walks across them, paths
//! around blocked squares, the masks as PNG files, and a folder's masks.

use m2_content::{Header, Sources, WalkMap, parse};
use proptest::prelude::*;

/// A mask drawn as text, a character a square: `#` blocked, anything else
/// open.
fn grid(rows: &[&str]) -> WalkMap {
    let height = rows.len() as u32;
    let width = rows[0].len() as u32;
    let mut map = WalkMap::open(width, height).unwrap();
    for (y, row) in rows.iter().enumerate() {
        assert_eq!(row.len() as u32, width, "rows of one width");
        for (x, c) in row.chars().enumerate() {
            map.set_blocked(x as u32, y as u32, c == '#');
        }
    }
    map
}

/// The centre of square (x, y), in centimetres.
fn at(x: i32, y: i32) -> (i32, i32) {
    (x * 100 + 50, y * 100 + 50)
}

#[test]
fn squares_are_open_until_blocked_and_off_the_map_is_blocked() {
    let mut map = WalkMap::open(3, 2).unwrap();
    assert_eq!(
        (map.width(), map.height(), map.blocked_squares()),
        (3, 2, 0)
    );
    map.set_blocked(2, 1, true);
    assert!(map.is_blocked(2, 1) && !map.is_blocked(1, 1));
    assert!(map.blocked_at((250, 199)) && !map.blocked_at((250, 99)));
    assert!(map.is_blocked(-1, 0) && map.is_blocked(3, 0) && map.is_blocked(0, 2));
    assert!(map.blocked_at((-1, 50)), "-1 cm is in square -1");
    map.set_blocked(2, 1, false);
    map.set_blocked(9, 9, true);
    assert_eq!(map.blocked_squares(), 0);
    assert_eq!(
        format!("{map:?}"),
        "WalkMap { width: 3, height: 2, blocked: 0 }"
    );
}

#[test]
fn sides_are_capped() {
    assert!(WalkMap::open(0, 5).is_err());
    assert!(WalkMap::open(5, 8_193).is_err());
    assert!(WalkMap::open(8_192, 1).is_ok());
}

#[test]
fn straight_walks_stop_at_blocked_squares() {
    let map = grid(&[
        "......", //
        "..#...", //
        "......", //
    ]);
    assert!(map.clear(at(0, 0), at(5, 0)), "along the top row");
    assert!(!map.clear(at(0, 1), at(5, 1)), "through the block");
    assert!(!map.clear(at(2, 0), at(2, 2)), "down through it");
    assert!(map.clear(at(1, 0), at(1, 2)), "beside it");
    assert!(map.clear(at(3, 2), at(3, 2)), "standing still");
    assert!(!map.clear(at(0, 0), at(2, 1)), "ending in it");
    assert!(!map.clear(at(0, 0), (600, 50)), "off the map");
}

#[test]
fn walks_never_slip_between_squares_touching_at_a_corner() {
    let map = grid(&[
        "#.", //
        ".#", //
    ]);
    // From (100, 100), the shared corner, the line to the far corner of
    // the open squares goes exactly through it.
    assert!(!map.clear((150, 50), (50, 150)));
    let map = grid(&[
        "..", //
        ".#", //
    ]);
    assert!(
        !map.clear((50, 50), (150, 150)),
        "ends in the blocked square"
    );
    let map = grid(&[
        "#..", //
        "...", //
        "..#", //
    ]);
    assert!(
        !map.clear(at(1, 0), at(0, 1)),
        "a corner with a blocked square beside it"
    );
    assert!(
        map.clear(at(1, 0), at(2, 1)),
        "a corner between open squares"
    );
}

#[test]
fn a_walker_standing_in_a_blocked_square_may_walk_out() {
    let map = grid(&[
        "##....", //
        "##....", //
    ]);
    assert!(map.clear(at(0, 0), at(4, 0)), "out to open ground");
    assert!(
        !map.clear(at(0, 0), at(1, 1)),
        "but not to another blocked square"
    );
    let map = grid(&["#.#."]);
    assert!(
        !map.clear(at(0, 0), at(3, 0)),
        "once out, blocked squares stop it again"
    );
}

#[test]
fn lines_through_corners_and_along_grid_lines_end_where_they_should() {
    let map = WalkMap::open(10, 10).unwrap();
    for (from, to) in [
        ((0, 0), (1000 - 1, 1000 - 1)),
        ((999, 0), (0, 999)),
        ((100, 100), (900, 900)),
        ((900, 900), (100, 100)),
        ((250, 250), (100, 150)),
        ((500, 0), (500, 999)),
        ((0, 500), (999, 500)),
        ((999, 999), (0, 0)),
        ((100, 0), (100, 900)),
        ((300, 300), (300, 300)),
        // Ending on a corner, coming from the north-east and the
        // south-west: the end's square is beside the last one crossed.
        ((900, 100), (100, 900)),
        ((100, 900), (900, 100)),
        ((750, 120), (500, 400)),
        ((560, 30), (400, 500)),
    ] {
        assert!(map.clear(from, to), "{from:?} → {to:?}");
    }
}

/// Every leg of `path` from `from` is clear, and it ends at `to`.
fn walkable(map: &WalkMap, from: (i32, i32), to: (i32, i32), path: &[(i32, i32)]) {
    assert_eq!(path.last(), Some(&to));
    let mut here = from;
    for &next in path {
        assert!(map.clear(here, next), "{here:?} → {next:?} in {path:?}");
        here = next;
    }
}

#[test]
fn paths_go_around_walls() {
    let map = grid(&[
        "..........", //
        "..........", //
        "....#.....", //
        "....#.....", //
        "....#.....", //
        "....#.....", //
        "..........", //
    ]);
    let (from, to) = (at(1, 4), at(8, 4));
    assert!(!map.clear(from, to));
    let path = map.find_path(from, to).unwrap();
    walkable(&map, from, to, &path);
    assert!(path.len() >= 2 && path.len() <= 3, "{path:?}");
    assert_eq!(map.find_path(from, to), Some(path), "the same every time");
    assert_eq!(
        map.find_path(at(1, 1), at(8, 1)),
        Some(vec![at(8, 1)]),
        "a clear line is one leg"
    );
}

#[test]
fn paths_find_the_way_through_a_maze() {
    let map = grid(&[
        "............", //
        ".##########.", //
        ".#........#.", //
        ".#.######.#.", //
        ".#.#....#.#.", //
        ".#.#.##.#.#.", //
        ".#.#..#.#.#.", //
        ".#.####.#.#.", //
        ".#......#.#.", //
        ".##########.", //
        "............", //
    ]);
    // From inside the spiral to outside: there is no way out.
    assert_eq!(map.find_path(at(4, 4), at(0, 0)), None);
    let map = grid(&[
        "............", //
        ".##########.", //
        ".#..........", //
        ".#.########.", //
        ".#.#......#.", //
        ".#.#.####.#.", //
        ".#...#..#.#.", //
        ".#####..#.#.", //
        "........#...", //
    ]);
    let (from, to) = (at(6, 6), at(0, 0));
    let path = map.find_path(from, to).unwrap();
    walkable(&map, from, to, &path);
}

#[test]
fn paths_keep_their_distance_from_walls() {
    let map = grid(&[
        "........", //
        "........", //
        "..####..", //
        "........", //
        "........", //
    ]);
    let (from, to) = (at(0, 3), at(7, 1));
    let path = map.find_path(from, to).unwrap();
    walkable(&map, from, to, &path);
    // No leg grazes the wall: points 30 cm to either side of each leg's
    // middle are not in blocked squares.
    let mut here = from;
    for &next in &path {
        let (mx, my) = ((here.0 + next.0) / 2, (here.1 + next.1) / 2);
        for (ox, oy) in [(0, 30), (0, -30), (30, 0), (-30, 0)] {
            assert!(!map.blocked_at((mx + ox, my + oy)), "{here:?} → {next:?}");
        }
        here = next;
    }
}

#[test]
fn paths_refuse_blocked_ends_and_far_ends() {
    let map = grid(&[
        "...", //
        ".#.", //
        "...", //
    ]);
    assert_eq!(
        map.find_path(at(0, 0), at(1, 1)),
        None,
        "the end is blocked"
    );
    assert_eq!(map.find_path(at(0, 0), (900, 50)), None, "off the map");
    let mut wide = WalkMap::open(2_000, 3).unwrap();
    wide.set_blocked(1_000, 1, true);
    assert_eq!(
        wide.find_path(at(0, 1), at(1_999, 1)),
        None,
        "farther than a search goes, and not a clear line"
    );
    assert_eq!(
        wide.find_path(at(0, 0), at(1_999, 0)),
        Some(vec![at(1_999, 0)]),
        "a clear line goes as far as it likes"
    );
}

#[test]
fn masks_go_through_png_and_back() {
    let map = grid(&[
        "#........#", //
        ".#..##..#.", //
        "..........", //
    ]);
    let png = map.to_png(None).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(WalkMap::from_png(&png).unwrap(), map);
    let marked = map.to_png(Some("made by a test")).unwrap();
    assert!(
        marked
            .windows(b"made by a test".len())
            .any(|w| w == b"made by a test")
    );
    assert_eq!(WalkMap::from_png(&marked).unwrap(), map);
}

/// The CRC-32 a PNG chunk ends with.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A PNG built byte by byte, its pixels stored without compression: each
/// row is a filter byte (0) and the row's bytes.
fn png(width: u32, height: u32, depth: u8, colour: u8, interlace: u8, rows: &[&[u8]]) -> Vec<u8> {
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[depth, colour, 0, 0, interlace]);
    chunk(&mut out, b"IHDR", &header);
    let mut raw = Vec::new();
    for row in rows {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    // zlib: header, one stored block, Adler-32.
    let mut zlib = vec![0x78, 0x01, 0x01];
    zlib.extend_from_slice(&(raw.len() as u16).to_le_bytes());
    zlib.extend_from_slice(&(!(raw.len() as u16)).to_le_bytes());
    zlib.extend_from_slice(&raw);
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
    chunk(&mut out, b"IDAT", &zlib);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn masks_read_any_kind_of_png() {
    let want = grid(&[
        "#..", //
        ".#.", //
    ]);
    // 8-bit grey: under 128 is blocked.
    let grey = png(3, 2, 8, 0, 0, &[&[0, 255, 128], &[200, 127, 255]]);
    assert_eq!(WalkMap::from_png(&grey).unwrap(), want);
    // RGB: the mean of the three.
    let rgb = png(
        3,
        2,
        8,
        2,
        0,
        &[
            &[255, 0, 0, 255, 255, 255, 0, 255, 255],
            &[255, 255, 255, 0, 0, 0, 200, 200, 200],
        ],
    );
    assert_eq!(WalkMap::from_png(&rgb).unwrap(), want);
    // RGBA: alpha is not read.
    let rgba = png(
        3,
        2,
        8,
        6,
        0,
        &[
            &[0, 0, 0, 0, 255, 255, 255, 0, 255, 255, 255, 255],
            &[255, 255, 255, 255, 10, 10, 10, 255, 255, 255, 255, 0],
        ],
    );
    assert_eq!(WalkMap::from_png(&rgba).unwrap(), want);
    // 1-bit grey, as openm2 writes them: 0 blocked.
    let bits = png(3, 2, 1, 0, 0, &[&[0b0110_0000], &[0b1010_0000]]);
    assert_eq!(WalkMap::from_png(&bits).unwrap(), want);
}

#[test]
fn bad_pngs_are_refused() {
    let interlaced = png(1, 1, 8, 0, 1, &[&[255]]);
    assert!(
        WalkMap::from_png(&interlaced)
            .unwrap_err()
            .to_string()
            .contains("interlaced")
    );
    assert!(WalkMap::from_png(b"not a png").is_err());
    let too_wide = png(8_193, 1, 1, 0, 0, &[&[0xFF; 1_025]]);
    assert!(
        WalkMap::from_png(&too_wide)
            .unwrap_err()
            .to_string()
            .contains("8192")
    );
    let short = png(3, 2, 8, 0, 0, &[&[0, 255, 128]]);
    assert!(WalkMap::from_png(&short).is_err());
}

proptest! {
    #[test]
    fn damaged_pngs_never_panic(cut in 0usize..200, flip in 0usize..200, bit in 0u8..8) {
        let map = grid(&["#...#", ".#.#.", "..#.."]);
        let mut png = map.to_png(None).unwrap();
        if let Some(byte) = png.get_mut(flip) {
            *byte ^= 1 << bit;
        }
        png.truncate(png.len().saturating_sub(cut));
        let _ = WalkMap::from_png(&png);
    }

    #[test]
    fn found_paths_are_walkable(
        blocked in proptest::collection::vec(any::<bool>(), 20 * 20),
        from in (0i32..2_000, 0i32..2_000),
        to in (0i32..2_000, 0i32..2_000),
    ) {
        let mut map = WalkMap::open(20, 20).unwrap();
        for (i, b) in blocked.iter().enumerate() {
            // About a third blocked.
            if *b && i % 3 == 0 {
                map.set_blocked(i as u32 % 20, i as u32 / 20, true);
            }
        }
        if let Some(path) = map.find_path(from, to) {
            prop_assert_eq!(path.last(), Some(&to));
            let mut here = from;
            for &next in &path {
                prop_assert!(map.clear(here, next));
                here = next;
            }
        }
        let a = map.clear(from, to);
        prop_assert_eq!(a, map.clear(from, to));
    }
}

const CONTENT: &str = "format = 1\nname = \"walls\"\n";

fn maps(walk: &str) -> String {
    format!(
        "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [100.0, 200.0]\nsize = [5.0, 3.5]\n{walk}"
    )
}

fn mask() -> WalkMap {
    grid(&[
        "#....", //
        "..#..", //
        ".....", //
        "....#", //
    ])
}

fn folder(walk: &str, files: &[(&str, Vec<u8>)]) -> Sources {
    let mut sources = Sources::new();
    sources.insert("content.toml", CONTENT).unwrap();
    sources.insert("maps.toml", maps(walk)).unwrap();
    for (path, bytes) in files {
        sources.insert_walk(*path, bytes.clone()).unwrap();
    }
    sources
}

#[test]
fn a_folder_gives_each_map_its_mask() {
    let png = mask().to_png(None).unwrap();
    let sources = folder(
        "walk = \"walk/test_map.png\"\n",
        &[("walk/test_map.png", png)],
    );
    let content = parse(&sources).unwrap();
    assert_eq!(content.walk("test_map"), Some(&mask()));
    assert_eq!(
        content.map("test_map").unwrap().walk.as_deref(),
        Some("walk/test_map.png")
    );

    // Written back, it reads the same.
    let mut again = Sources::new();
    for (path, text) in content.to_files(Header::Plain) {
        again.insert(path, text).unwrap();
    }
    let walks = content.walk_files(Header::Imported).unwrap();
    assert_eq!(walks.len(), 1);
    for (path, bytes) in walks {
        assert!(
            bytes
                .windows(b"openm2 content import".len())
                .any(|w| w == b"openm2 content import")
        );
        again.insert_walk(path, bytes).unwrap();
    }
    assert_eq!(parse(&again).unwrap(), content);

    let open = parse(&folder("", &[])).unwrap();
    assert_eq!(open.walk("test_map"), None, "no mask: open everywhere");
}

#[test]
fn points_find_the_map_they_lie_on() {
    let content = parse(&folder("", &[])).unwrap();
    let map = content.map("test_map").unwrap();
    // 100 m and 200 m from the world's corner, 5 m by 3.5 m.
    assert_eq!(map.origin_cm(), (10_000, 20_000));
    for inside in [(10_000, 20_000), (10_499, 20_349), (10_250, 20_100)] {
        assert_eq!(
            content.map_at(inside).map(|m| m.name.as_str()),
            Some("test_map")
        );
    }
    for outside in [(9_999, 20_000), (10_500, 20_000), (10_000, 20_350), (0, 0)] {
        assert!(content.map_at(outside).is_none(), "{outside:?}");
    }
}

#[test]
fn every_broken_walk_rule_is_named() {
    let errors = |sources: Sources| parse(&sources).unwrap_err().to_string();
    let png = mask().to_png(None).unwrap();
    assert_eq!(
        errors(folder("walk = \"walk/none.png\"\n", &[])),
        "maps.toml:3:1: no file walk/none.png in the folder"
    );
    assert_eq!(
        errors(folder("walk = \"masks/a.png\"\n", &[])),
        "maps.toml:3:1: walk mask \"masks/a.png\": it is walk/<name>.png, the name ASCII letters, digits, _ - and ., at most 64"
    );
    let small = grid(&["....."]).to_png(None).unwrap();
    assert_eq!(
        errors(folder("walk = \"walk/a.png\"\n", &[("walk/a.png", small)])),
        "maps.toml:3:1: walk/a.png is 5 × 1 pixels; the map is 5 × 3.5 m, so its mask is 5 × 4 (a pixel a metre)"
    );
    assert_eq!(
        errors(folder("", &[("walk/stray.png", png.clone())])),
        "walk/stray.png: no map in maps.toml names it (walk = \"walk/stray.png\")"
    );
    let broken = errors(folder(
        "walk = \"walk/a.png\"\n",
        &[("walk/a.png", b"not a png".to_vec())],
    ));
    assert!(
        broken.starts_with("walk/a.png: not a PNG image the server reads"),
        "{broken}"
    );
    assert_eq!(
        broken.lines().count(),
        1,
        "the map's line is not named again"
    );
}
