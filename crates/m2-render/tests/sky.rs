//! The sky: colour bands by height, a box's pictures and drifting clouds,
//! drawn with a graphics adapter (a software one is enough); without one
//! the tests fail.

use glam::Vec3;
use m2_formats::RgbaImage;
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::{Camera, Clouds, Gpu, GpuOptions, GpuSky, Offscreen, Renderer, Scene, Sky, World};

const RED: [f32; 3] = [1.0, 0.0, 0.0];
const GREEN: [f32; 3] = [0.0, 1.0, 0.0];
const BLUE: [f32; 3] = [0.0, 0.0, 1.0];
const CYAN: [f32; 3] = [0.0, 1.0, 1.0];
const GREY: [f32; 3] = [0.5, 0.5, 0.5];
const DARK: [f32; 3] = [0.0, 0.0, 0.25];

fn gpu() -> (Gpu, Renderer) {
    let gpu = Gpu::headless(GpuOptions::default()).unwrap_or_else(|error| panic!("{error}"));
    let renderer =
        Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT)).unwrap();
    (gpu, renderer)
}

fn near(actual: [u8; 4], expected: [u8; 3], within: u8) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| a.abs_diff(e) <= within)
}

/// Looks towards `yaw` (0 north, π south, −π/2 east) and `pitch` in
/// degrees (negative looks up), with the default 30° field of view.
fn looking(yaw: f32, pitch: f32) -> Scene {
    Scene::new(Camera {
        target: Vec3::new(500.0, -700.0, 300.0),
        yaw,
        pitch: pitch.to_radians(),
        distance: 100.0,
        fov_y: Camera::FOV_Y,
    })
}

fn draw(gpu: &Gpu, renderer: &Renderer, sky: &GpuSky, scene: &Scene, seconds: f32) -> RgbaImage {
    let target = Offscreen::new(gpu, renderer, 16, 16).unwrap();
    target
        .render_world(
            gpu,
            renderer,
            scene,
            &World {
                sky: Some(sky),
                seconds,
                ..World::default()
            },
        )
        .unwrap()
}

#[test]
fn bands_go_by_height_and_their_end_colours_fill_above_and_below() {
    let (gpu, renderer) = gpu();
    // Two bands above the horizon, one colour each: the box's sides are
    // red from their top halfway down, then green. Below the horizon one
    // band from blue to cyan.
    let sky = renderer
        .upload_sky(
            &gpu,
            &Sky {
                upper: vec![[RED, RED], [GREEN, GREEN]],
                lower: vec![[BLUE, CYAN]],
                ..Sky::default()
            },
        )
        .unwrap();
    // Looking level: the top row 15° up (tan 0.25 of the bottom row's
    // centre, in the green band), the bottom row as far down, a quarter of
    // the way from blue to cyan.
    let level = draw(&gpu, &renderer, &sky, &looking(0.0, 0.0), 0.0);
    let top = level.pixel(8, 0).unwrap();
    assert!(near(top, [0, 255, 0], 2), "{top:?}");
    let bottom = level.pixel(8, 15).unwrap();
    assert!(near(bottom, [0, 64, 255], 4), "{bottom:?}");
    // Above the sides (over 45°) the highest band's top colour, below them
    // the lowest band's bottom colour.
    let up = draw(&gpu, &renderer, &sky, &looking(0.0, -60.0), 0.0);
    assert!(near(up.pixel(8, 8).unwrap(), [255, 0, 0], 2));
    let down = draw(&gpu, &renderer, &sky, &looking(0.0, 60.0), 0.0);
    assert!(near(down.pixel(8, 8).unwrap(), [0, 255, 255], 2));
}

/// A picture of `side` pixels, its quarters top left, top right, bottom
/// left and bottom right in these colours.
fn quarters(side: u32, colours: [[u8; 3]; 4]) -> RgbaImage {
    let mut image = RgbaImage::new(side, side).unwrap();
    for y in 0..side {
        for x in 0..side {
            let quarter = usize::from(y >= side / 2) * 2 + usize::from(x >= side / 2);
            let [r, g, b] = colours[quarter];
            image.set_pixel(x, y, [r, g, b, 255]);
        }
    }
    image
}

fn solid(rgb: [u8; 3]) -> Option<RgbaImage> {
    Some(quarters(8, [rgb; 4]))
}

#[test]
fn the_box_pictures_face_their_sides_upright_and_the_bands_show_where_one_is_missing() {
    let (gpu, renderer) = gpu();
    // The front's quarters, and the top's: their halves tell which way up
    // and which way round they are drawn.
    let front = quarters(64, [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]]);
    let top = quarters(64, [[255, 255, 255], [255, 255, 255], [0, 0, 0], [0, 0, 0]]);
    let sky = renderer
        .upload_sky(
            &gpu,
            &Sky {
                upper: vec![[GREY, GREY]],
                lower: vec![[GREY, GREY]],
                faces: Some([
                    Some(front),
                    solid([0, 128, 0]),
                    solid([0, 0, 128]),
                    solid([128, 0, 128]),
                    Some(top),
                    None,
                ]),
                clouds: None,
            },
        )
        .unwrap();
    let pi = std::f32::consts::PI;
    // South: the front, seen from inside as it is drawn (its left half on
    // the left, its top up).
    let south = draw(&gpu, &renderer, &sky, &looking(pi, 0.0), 0.0);
    for ((x, y), colour) in [
        ((2, 2), [255, 0, 0]),
        ((13, 2), [0, 255, 0]),
        ((2, 13), [0, 0, 255]),
        ((13, 13), [255, 255, 0]),
    ] {
        let found = south.pixel(x, y).unwrap();
        assert!(near(found, colour, 2), "({x}, {y}): {found:?}");
    }
    // North the back, east the left side, west the right side.
    for (yaw, colour) in [
        (0.0, [0, 128, 0]),
        (-pi / 2.0, [0, 0, 128]),
        (pi / 2.0, [128, 0, 128]),
    ] {
        let found = draw(&gpu, &renderer, &sky, &looking(yaw, 0.0), 0.0)
            .pixel(8, 8)
            .unwrap();
        assert!(near(found, colour, 2), "yaw {yaw}: {found:?}");
    }
    // Straight up from the south looking north, the top of the view points
    // south: the top's lower half, which meets the front.
    let up = draw(&gpu, &renderer, &sky, &looking(0.0, -89.0), 0.0);
    assert!(near(up.pixel(8, 1).unwrap(), [0, 0, 0], 2));
    assert!(near(up.pixel(8, 14).unwrap(), [255, 255, 255], 2));
    // No bottom picture: the bands.
    let down = draw(&gpu, &renderer, &sky, &looking(0.0, 89.0), 0.0);
    assert!(near(down.pixel(8, 8).unwrap(), [128, 128, 128], 2));
}

#[test]
fn clouds_are_added_overhead_drift_with_time_and_are_gone_at_the_horizon() {
    let (gpu, renderer) = gpu();
    // The texture's left half white, its right half black; it repeats every
    // 100 m (10 times over the layer's 1 km reach) and drifts a quarter of
    // itself a second.
    let mut image = RgbaImage::filled(4, 4, [0, 0, 0, 255]).unwrap();
    for y in 0..4 {
        for x in 0..2 {
            image.set_pixel(x, y, [255, 255, 255, 255]);
        }
    }
    let sky = renderer
        .upload_sky(
            &gpu,
            &Sky {
                upper: vec![[DARK, DARK]],
                lower: vec![[DARK, DARK]],
                faces: None,
                clouds: Some(Clouds {
                    image,
                    height: 10_000.0,
                    reach: [100_000.0; 2],
                    repeats: [10.0; 2],
                    speed: [0.25, 0.0],
                }),
            },
        )
        .unwrap();
    let up = looking(0.0, -89.9);
    // Straight up: at 1 s a quarter of the texture across, in its white
    // half; at 3 s three quarters, in its black half; at 5 s white again.
    for (seconds, colour) in [
        (1.0, [255, 255, 255]),
        (3.0, [0, 0, 64]),
        (5.0, [255, 255, 255]),
    ] {
        let found = draw(&gpu, &renderer, &sky, &up, seconds)
            .pixel(8, 8)
            .unwrap();
        assert!(near(found, colour, 3), "{seconds} s: {found:?}");
    }
    // The horizon is past the layer's edge.
    let level = draw(&gpu, &renderer, &sky, &looking(0.0, 0.0), 1.0);
    assert!(near(level.pixel(8, 8).unwrap(), [0, 0, 64], 2));
}
