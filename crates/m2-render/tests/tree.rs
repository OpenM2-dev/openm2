//! Trees grown from openm2's recipes: the book, the growth
//! and the textures on the CPU, and a grown tree drawn with a graphics
//! adapter (the test fails without one).

use glam::Vec3;
use m2_formats::RgbaImage;
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::terrain::mip_chain;
use m2_render::tree::{CROWN_NORMAL, Form, LeafKind, MAX_TREE_VERTICES};
use m2_render::tree_texture::{
    self, BARK_SIZE, BLADE_SIZE, CUTOFF, FROND_SIZE, LEAF_SIDE, coverage, coverage_mips,
};
use m2_render::{
    Bounds, Camera, Gpu, GpuOptions, Offscreen, RenderError, Renderer, Scene, Season, TreeBook,
    grow,
};

/// One recipe of each form; the round tree's bark red and its leaves green.
const BOOK: &str = r#"(
    default: "round",
    names: [
        (word: "fir", recipe: "cone"),
        (word: "palm", recipe: "palm"),
        (word: "fern", recipe: "low"),
    ],
    seasons: [(word: "winter", season: Winter), (word: "fall", season: Fall)],
    recipes: {
        "round": (
            form: Broadleaf,
            height: (800, 1000),
            trunk: (radius: 30, taper: 0.4, lean: 5, bark: (200, 0, 0), grooves: 0.5),
            crown: (from: 0.4, width: 0.4, shape: Round),
            branches: (count: 8, angle: 55, droop: 0.1, twigs: 3),
            leaves: (kind: Broad, colour: (0, 200, 0), size: 250, count: 4),
            fall: (leaves: (200, 100, 0)),
            winter: (bare: true),
            flowers: (flowers: (250, 250, 250)),
        ),
        "cone": (
            form: Conifer,
            height: (900, 1100),
            trunk: (radius: 25, taper: 0.2, bark: (90, 70, 60), grooves: 0.6),
            crown: (from: 0.1, width: 0.25, shape: Cone),
            branches: (count: 30, angle: 95, droop: 0.2),
            leaves: (kind: Needles, colour: (40, 90, 60), size: 200, count: 5),
            winter: (snow: true),
        ),
        "palm": (
            form: Palm,
            height: (800, 1000),
            trunk: (radius: 20, taper: 0.8, bend: 25, bark: (130, 110, 90)),
            branches: (count: 0, angle: 0, droop: 0.5),
            leaves: (kind: Frond, colour: (80, 130, 50), size: 400, count: 18),
        ),
        "low": (
            form: Rosette,
            height: (80, 120),
            trunk: (radius: 0, taper: 1, bark: (90, 80, 60)),
            branches: (count: 0, angle: 0, droop: 0.9),
            leaves: (kind: Blade, colour: (90, 140, 90), size: 100, count: 20),
        ),
    },
)"#;

fn book() -> TreeBook {
    TreeBook::parse(BOOK).unwrap()
}

#[test]
fn a_book_picks_recipes_and_seasons_by_the_words_of_a_file_name() {
    let book = book();
    assert_eq!(
        book.pick(br"d:\ymir work\tree\x_fir_winter.spt"),
        ("cone", Season::Winter)
    );
    assert_eq!(book.pick(b"X_PALM_Fall_2.SPT"), ("palm", Season::Fall));
    assert_eq!(book.pick(b"y_fern_1.spt"), ("low", Season::Summer));
    // Nothing known: the default. Only the file's own name counts.
    assert_eq!(book.pick(b"oak.spt"), ("round", Season::Summer));
    assert_eq!(book.pick(b"palm/oak.spt"), ("round", Season::Summer));
}

#[test]
fn a_bad_book_is_refused_with_its_reason() {
    let broken = |from: &str, to: &str| {
        assert_eq!(BOOK.matches(from).count(), 1, "{from}");
        match TreeBook::parse(&BOOK.replace(from, to)) {
            Err(RenderError::Trees(why)) => why,
            other => panic!("{from} → {to}: {other:?}"),
        }
    };
    assert!(broken("lean: 5,", "lean: 5, colour: 1,").contains("colour"));
    assert!(broken("default: \"round\"", "default: \"square\"").contains("`square`"));
    assert!(broken("recipe: \"cone\"", "recipe: \"pine\"").contains("`pine`"));
    let why = broken("count: 8,", "count: 65,");
    assert!(
        why.contains("recipe `round`") && why.contains("branches.count"),
        "{why}"
    );
    assert!(broken("height: (900, 1100)", "height: (1100, 900)").contains("highest height"));
    assert!(broken("size: 250", "size: 0").contains("leaves.size"));
    let why = broken(
        "count: 8, angle: 55, droop: 0.1, twigs: 3",
        "count: 64, angle: 55, droop: 0.1, twigs: 16",
    );
    assert!(why.contains("at most 2048"), "{why}");
    assert!(matches!(
        TreeBook::parse("(default: \"x\""),
        Err(RenderError::Trees(_))
    ));
}

#[test]
fn the_same_seed_grows_the_same_tree_and_another_a_different_one() {
    let book = book();
    let round = book.recipe("round").unwrap();
    assert_eq!(
        grow(round, Season::Summer, 7),
        grow(round, Season::Summer, 7)
    );
    assert_ne!(
        grow(round, Season::Summer, 7),
        grow(round, Season::Summer, 8)
    );
}

#[test]
fn every_form_grows_a_whole_tree_in_every_season() {
    let book = book();
    for (name, recipe) in &book.recipes {
        for season in Season::ALL {
            let part = grow(recipe, season, 3);
            let what = format!("{name} in {season:?}");
            let count = part.vertices.len();
            assert!(count > 0 && count <= MAX_TREE_VERTICES, "{what}: {count}");
            assert_eq!(part.indices.len() % 3, 0, "{what}");
            assert!(part.indices.iter().all(|&i| (i as usize) < count), "{what}");
            // The draws cover the triangles in order: bark, then leaves.
            let mut next = 0;
            for draw in &part.draws {
                assert_eq!(draw.first, next, "{what}");
                next += draw.count;
            }
            assert_eq!(next as usize, part.indices.len(), "{what}");
            assert_eq!(part.textures.len(), 2, "{what}");
            let (low, high) = recipe.height;
            let mut top: f32 = 0.0;
            for vertex in &part.vertices {
                let position = Vec3::from(vertex.position);
                assert!(position.is_finite(), "{what}");
                assert!(position.z > -50.0, "{what}: {position}");
                top = top.max(position.z);
                let length = Vec3::from(vertex.normal).length();
                assert!(
                    (length - 1.0).abs() < 1e-3 || (length - CROWN_NORMAL).abs() < 1e-2,
                    "{what}: a normal {length} long"
                );
            }
            // A rosette is as tall as its leaves rise; the others about
            // their height.
            let lowest = if recipe.form == Form::Rosette {
                0.0
            } else {
                low * 0.4
            };
            assert!(top > lowest && top < high * 1.5, "{what}: {top} cm tall");
        }
    }
    // Bare in winter: bark only.
    let round = book.recipe("round").unwrap();
    let bare = grow(round, Season::Winter, 3);
    assert_eq!(bare.draws.len(), 1);
    assert_eq!(bare.draws.first().unwrap().texture, Some(0));
    assert_eq!(grow(round, Season::Summer, 3).draws.len(), 2);
    // A rosette has no trunk.
    let low = book.recipe("low").unwrap();
    assert_eq!(low.form, Form::Rosette);
    let draws = grow(low, Season::Summer, 3).draws;
    assert_eq!(draws.len(), 1);
    assert_eq!(draws.first().unwrap().texture, Some(1));
}

#[test]
fn a_crowns_normals_point_out_of_it() {
    let book = book();
    let part = grow(book.recipe("round").unwrap(), Season::Summer, 5);
    let leaves = part.draws.get(1).unwrap();
    let corners: Vec<usize> = part
        .indices
        .get(leaves.first as usize..)
        .unwrap()
        .iter()
        .map(|&i| i as usize)
        .collect();
    let points: Vec<Vec3> = corners
        .iter()
        .map(|&i| Vec3::from(part.vertices.get(i).unwrap().position))
        .collect();
    let middle = points.iter().sum::<Vec3>() / points.len() as f32;
    let outward = corners
        .iter()
        .zip(&points)
        .filter(|&(&i, &p)| {
            let normal = Vec3::from(part.vertices.get(i).unwrap().normal);
            (normal.length() - CROWN_NORMAL).abs() < 1e-2 && normal.dot(p - middle) > 0.0
        })
        .count();
    assert!(
        outward * 10 > points.len() * 9,
        "{outward} of {}",
        points.len()
    );
}

fn level(texture: &m2_render::TextureData) -> &RgbaImage {
    texture.levels().first().unwrap()
}

#[test]
fn bark_is_opaque_its_size_and_tiles() {
    let bark = tree_texture::bark([120, 100, 80], 0.8, false, 9).unwrap();
    let image = level(&bark);
    assert_eq!((image.width(), image.height()), BARK_SIZE);
    assert!(image.pixels().iter().skip(3).step_by(4).all(|&a| a == 255));
    assert!(!bark.coverage());
    // Across the seam the colour changes no more than between neighbours.
    let step = |a: u32, b: u32| -> f32 {
        (0..image.height())
            .map(|y| {
                let p = image.pixel(a, y).unwrap();
                let q = image.pixel(b, y).unwrap();
                f32::from(p[1].abs_diff(q[1]))
            })
            .sum::<f32>()
            / image.height() as f32
    };
    let (width, _) = BARK_SIZE;
    assert!(step(width - 1, 0) < step(10, 11) * 3.0 + 4.0);
    // Palms are ringed: rows differ more than columns do.
    let ringed = tree_texture::bark([120, 100, 80], 0.0, true, 9).unwrap();
    let ringed = level(&ringed);
    let row_mean = |y: u32| {
        (0..ringed.width())
            .map(|x| f32::from(ringed.pixel(x, y).unwrap()[1]))
            .sum::<f32>()
            / ringed.width() as f32
    };
    let rows: Vec<f32> = (0..32).map(row_mean).collect();
    let spread = rows.iter().cloned().fold(f32::MIN, f32::max)
        - rows.iter().cloned().fold(f32::MAX, f32::min);
    assert!(spread > 10.0, "{spread}");
}

#[test]
fn leaves_are_cut_out_by_alpha_without_dark_edges() {
    for (kind, size) in [
        (LeafKind::Broad, (LEAF_SIDE, LEAF_SIDE)),
        (LeafKind::Needles, (LEAF_SIDE, LEAF_SIDE)),
        (LeafKind::Tuft, (LEAF_SIDE, LEAF_SIDE)),
        (LeafKind::Frond, FROND_SIZE),
        (LeafKind::Blade, BLADE_SIZE),
    ] {
        let leaves = tree_texture::leaves(kind, [60, 160, 40], None, false, 4).unwrap();
        let image = level(&leaves);
        assert_eq!((image.width(), image.height()), size, "{kind:?}");
        assert!(leaves.coverage(), "{kind:?}");
        let share = coverage(image, CUTOFF);
        assert!(share > 0.05 && share < 0.95, "{kind:?}: {share}");
        // Transparent texels carry the leaves' colour, not black.
        for texel in image.pixels().chunks(4) {
            if texel.get(3) == Some(&0) {
                assert!(texel.get(1).copied().unwrap() > 60, "{kind:?}: {texel:?}");
            }
        }
    }
    // Snow whitens some of them; blossoms add their colour.
    let white = |image: &RgbaImage| {
        image
            .pixels()
            .chunks(4)
            .filter(|t| t.iter().take(3).all(|&c| c > 200) && t.get(3) == Some(&255))
            .count()
    };
    let plain = tree_texture::leaves(LeafKind::Needles, [60, 90, 60], None, false, 4).unwrap();
    let snowy = tree_texture::leaves(LeafKind::Needles, [60, 90, 60], None, true, 4).unwrap();
    assert_eq!(white(level(&plain)), 0);
    assert!(white(level(&snowy)) > 500);
    let flowers = tree_texture::leaves(
        LeafKind::Broad,
        [60, 90, 60],
        Some([250, 250, 250]),
        false,
        4,
    )
    .unwrap();
    assert!(white(level(&flowers)) > 500);
}

#[test]
fn coverage_is_kept_down_the_mip_chain() {
    // One texel in four drawn, in a grid: averaging alone loses them all
    // after one level.
    let mut image = RgbaImage::filled(64, 64, [0, 120, 0, 0]).unwrap();
    for y in (0..64).step_by(2) {
        for x in (0..64).step_by(2) {
            image.set_pixel(x, y, [0, 120, 0, 255]);
        }
    }
    let share = coverage(&image, CUTOFF);
    assert!((share - 0.25).abs() < 1e-6);
    let plain = mip_chain(image.clone()).unwrap();
    assert_eq!(coverage(plain.get(1).unwrap(), CUTOFF), 0.0);
    let kept = coverage_mips(image, CUTOFF).unwrap();
    assert_eq!(kept.len(), plain.len());
    for level in kept.iter().take(4) {
        let at = coverage(level, CUTOFF);
        assert!(
            at >= share - 0.01,
            "{}×{}: {at}",
            level.width(),
            level.height()
        );
    }
}

fn gpu() -> (Gpu, Renderer) {
    let gpu = Gpu::headless(GpuOptions::default()).unwrap_or_else(|error| panic!("{error}"));
    let renderer =
        Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT)).unwrap();
    (gpu, renderer)
}

#[test]
fn a_grown_tree_is_drawn_with_its_bark_below_and_its_leaves_above() {
    let (gpu, renderer) = gpu();
    let book = book();
    let recipe = book.recipe("round").unwrap();
    let part = grow(recipe, Season::Summer, 2);
    let textures = tree_texture::textures(recipe, Season::Summer, 2).unwrap();
    let mut points = part.vertices.iter().map(|v| Vec3::from(v.position));
    let first = points.next().unwrap();
    let bounds = points.fold(
        Bounds {
            min: first,
            max: first,
        },
        Bounds::including,
    );
    let mut scene = Scene::new(Camera::framing(bounds, 0.0, 0.0, 1.0));
    scene.background = [0.0, 0.0, 1.0, 1.0];
    let uploaded = renderer.upload(&gpu, &part, &textures).unwrap();
    let target = Offscreen::new(&gpu, &renderer, 64, 64).unwrap();
    let image = target
        .render(&gpu, &renderer, &scene, &[&uploaded])
        .unwrap();
    let count = |rows: std::ops::Range<u32>, test: fn([u8; 4]) -> bool| {
        rows.flat_map(|y| (0..64).map(move |x| (x, y)))
            .filter(|&(x, y)| test(image.pixel(x, y).unwrap()))
            .count()
    };
    let red = |[r, g, b, _]: [u8; 4]| r > g.saturating_add(40) && r > b.saturating_add(40);
    let green = |[r, g, b, _]: [u8; 4]| g > r.saturating_add(40) && g > b.saturating_add(40);
    // The trunk at the bottom, the crown over the top half.
    assert!(count(48..64, red) > 4, "no bark at the bottom");
    assert!(count(0..32, green) > 300, "no leaves at the top");
    assert_eq!(count(56..64, green), 0, "leaves at the foot");
}
