//! Drawing offscreen and reading the pixels back. Needs a graphics adapter
//! (a software one is enough: WARP on Windows, llvmpipe/lavapipe on Linux);
//! without one the tests fail.

mod common;

use common::quad;
use glam::{Mat4, Vec3};
use m2_formats::RgbaImage;
use m2_render::effect::EffectVertex;
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::{
    Camera, EffectGeometry, Gpu, GpuOptions, GpuPart, Offscreen, Part, Renderer, Scene,
    TextureData, World,
};

const SIZE: u32 = 64;
const BACKGROUND: [u8; 4] = [0, 0, 255, 255];

fn gpu() -> Gpu {
    Gpu::headless(GpuOptions::default()).unwrap_or_else(|error| panic!("{error}"))
}

/// Unlit (ambient 1, no diffuse) so pixels are the texture's colours; the
/// quad covers the middle half of the picture.
fn scene() -> Scene {
    let mut scene = Scene::new(Camera {
        target: Vec3::new(0.0, 0.0, 50.0),
        yaw: 0.0,
        pitch: 0.0,
        distance: 50.0 / (Camera::FOV_Y * 0.5).tan() * 2.0,
        fov_y: Camera::FOV_Y,
    });
    scene.background = [0.0, 0.0, 1.0, 1.0];
    scene.ambient = [1.0; 3];
    scene.diffuse = [0.0; 3];
    scene
}

struct Setup {
    gpu: Gpu,
    renderer: Renderer,
    target: Offscreen,
}

fn setup(gpu: Gpu) -> Setup {
    let renderer =
        Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT)).unwrap();
    let target = Offscreen::new(&gpu, &renderer, SIZE, SIZE).unwrap();
    Setup {
        gpu,
        renderer,
        target,
    }
}

impl Setup {
    fn upload(&self, texture: Option<TextureData>) -> GpuPart {
        let part = Part::from_model(&quad()).unwrap();
        self.renderer.upload(&self.gpu, &part, &[texture]).unwrap()
    }

    fn render(&self, parts: &[&GpuPart]) -> RgbaImage {
        self.target
            .render(&self.gpu, &self.renderer, &scene(), parts)
            .unwrap()
    }
}

fn solid(rgba: [u8; 4]) -> TextureData {
    TextureData::new(vec![RgbaImage::filled(2, 2, rgba).unwrap()]).unwrap()
}

fn near(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 2)
}

#[test]
fn a_textured_quad_lands_in_the_middle_of_the_picture() {
    let gpu = gpu();
    let setup = setup(gpu);
    let part = setup.upload(Some(solid([200, 100, 50, 255])));
    let image = setup.render(&[&part]);
    assert_eq!((image.width(), image.height()), (SIZE, SIZE));
    let centre = image.pixel(SIZE / 2, SIZE / 2).unwrap();
    assert!(near(centre, [200, 100, 50, 255]), "{centre:?}");
    let corner = image.pixel(2, 2).unwrap();
    assert!(near(corner, BACKGROUND), "{corner:?}");
}

#[test]
fn a_missing_texture_draws_white() {
    let gpu = gpu();
    let setup = setup(gpu);
    let part = setup.upload(None);
    let centre = setup.render(&[&part]).pixel(SIZE / 2, SIZE / 2).unwrap();
    assert!(near(centre, [255, 255, 255, 255]), "{centre:?}");
}

#[test]
fn the_palette_moves_the_vertices() {
    let gpu = gpu();
    let setup = setup(gpu);
    let part = setup.upload(Some(solid([0, 255, 0, 255])));
    // Joint 1 (`arm`) moves 100 to the right: the quad leaves the middle and
    // covers the right edge.
    let mut palette = vec![Mat4::IDENTITY; 3];
    palette[1] = Mat4::from_translation(Vec3::new(100.0, 0.0, 0.0));
    setup.renderer.set_palette(&setup.gpu, &part, &palette);
    let image = setup.render(&[&part]);
    let centre = image.pixel(SIZE / 2, SIZE / 2).unwrap();
    assert!(near(centre, BACKGROUND), "{centre:?}");
    let right = image.pixel(SIZE * 7 / 8, SIZE / 2).unwrap();
    assert!(near(right, [0, 255, 0, 255]), "{right:?}");
}

#[test]
fn alpha_below_the_cutoff_is_not_drawn_when_it_is_coverage() {
    let gpu = gpu();
    let setup = setup(gpu);
    // Left texel opaque red, right texel transparent red.
    let image = RgbaImage::from_pixels(2, 1, vec![255, 0, 0, 255, 255, 0, 0, 0]).unwrap();
    let covered = TextureData::new(vec![image]).unwrap();
    let left = (SIZE * 5 / 16, SIZE / 2);
    let right = (SIZE * 11 / 16, SIZE / 2);

    let part = setup.upload(Some(covered.clone()));
    let drawn = setup.render(&[&part]);
    assert!(near(drawn.pixel(left.0, left.1).unwrap(), [255, 0, 0, 255]));
    assert!(near(drawn.pixel(right.0, right.1).unwrap(), BACKGROUND));

    // The same texture with alpha that is not coverage draws everywhere.
    let part = setup.upload(Some(covered.with_coverage(false)));
    let drawn = setup.render(&[&part]);
    assert!(near(
        drawn.pixel(right.0, right.1).unwrap(),
        [255, 0, 0, 255]
    ));
}

#[test]
fn a_shining_part_adds_its_shine_map_where_its_gloss_mask_says() {
    let gpu = gpu();
    let setup = setup(gpu);
    // Grey, the left half glossy (alpha 255), the right not (alpha 0): its
    // alpha a gloss mask, not coverage, so all of it draws.
    let grey = [40, 40, 40];
    let mut pixels = Vec::new();
    for alpha in [255, 255, 0, 0] {
        pixels.extend_from_slice(&[grey[0], grey[1], grey[2], alpha]);
    }
    let image = RgbaImage::from_pixels(4, 1, pixels).unwrap();
    let texture = TextureData::new(vec![image])
        .unwrap()
        .with_coverage(false)
        .with_shine(true);
    // Inside each half, away from where the texture wraps round.
    let left = (SIZE * 5 / 16, SIZE / 2);
    let right = (SIZE * 10 / 16, SIZE / 2);
    let mut part = setup.upload(Some(texture));
    let plain = [grey[0], grey[1], grey[2], 255];
    // Without a shine: grey all over.
    let drawn = setup.render(&[&part]);
    for (x, y) in [left, right] {
        assert!(
            near(drawn.pixel(x, y).unwrap(), plain),
            "{:?}",
            drawn.pixel(x, y)
        );
    }
    // Half its shine: half the map's colour (one colour, wherever the view
    // reflects) added where it is glossy.
    let map = setup
        .renderer
        .upload_shine_map(&setup.gpu, &solid([200, 100, 0, 255]))
        .unwrap();
    setup.renderer.set_shine(&setup.gpu, &mut part, 0.5, &map);
    let shining = [140, 90, 40, 255];
    let drawn = setup.render(&[&part]);
    assert!(
        near(drawn.pixel(left.0, left.1).unwrap(), shining),
        "{:?}",
        drawn.pixel(left.0, left.1)
    );
    assert!(
        near(drawn.pixel(right.0, right.1).unwrap(), plain),
        "{:?}",
        drawn.pixel(right.0, right.1)
    );
    // An instance shines the same.
    let copy = setup.renderer.instance(&setup.gpu, &part);
    let covered = setup.upload(Some(solid([40, 40, 40, 255])));
    let mut covered = covered;
    // A texture that doesn't shine (a face's) never does, on a shining
    // part.
    setup
        .renderer
        .set_shine(&setup.gpu, &mut covered, 1.0, &map);
    let drawn = setup.render(&[&covered]);
    assert!(
        near(drawn.pixel(left.0, left.1).unwrap(), plain),
        "{:?}",
        drawn.pixel(left.0, left.1)
    );
    let drawn = setup.render(&[&copy]);
    assert!(
        near(drawn.pixel(left.0, left.1).unwrap(), shining),
        "{:?}",
        drawn.pixel(left.0, left.1)
    );
}

#[test]
fn the_nearer_part_hides_the_one_behind() {
    let gpu = gpu();
    let setup = setup(gpu);
    let red = setup.upload(Some(solid([255, 0, 0, 255])));
    let green = setup.upload(Some(solid([0, 255, 0, 255])));
    // Green moves 10 towards the camera (−Y); drawing order must not matter.
    let mut palette = vec![Mat4::IDENTITY; 3];
    palette[1] = Mat4::from_translation(Vec3::new(0.0, -10.0, 0.0));
    setup.renderer.set_palette(&setup.gpu, &green, &palette);
    for parts in [[&red, &green], [&green, &red]] {
        let centre = setup.render(&parts).pixel(SIZE / 2, SIZE / 2).unwrap();
        assert!(near(centre, [0, 255, 0, 255]), "{centre:?}");
    }
}

#[test]
fn offscreen_targets_refuse_bad_sizes() {
    let gpu = gpu();
    let setup = setup(gpu);
    assert!(Offscreen::new(&setup.gpu, &setup.renderer, 0, 10).is_err());
    assert!(Offscreen::new(&setup.gpu, &setup.renderer, 10, 1 << 20).is_err());
}

/// A square of effect triangles in the plane of the quad model (y = `y`),
/// covering its middle.
fn effect_square(y: f32, colour: [f32; 4], blend: Option<(u32, u32)>) -> EffectGeometry {
    let corner = |x: f32, z: f32| EffectVertex {
        position: [x, y, z],
        uv: [0.5, 0.5],
        colour,
        operation: 4,
    };
    let (a, b, c, d) = (
        corner(-25.0, 25.0),
        corner(25.0, 25.0),
        corner(25.0, 75.0),
        corner(-25.0, 75.0),
    );
    let mut geometry = EffectGeometry::default();
    geometry.push(Some(0), blend, &[a, b, c, a, c, d]);
    geometry
}

impl Setup {
    fn render_effects(&self, parts: &[&GpuPart], geometry: &EffectGeometry) -> RgbaImage {
        let texture = self
            .renderer
            .upload_texture(&self.gpu, &solid([255, 255, 255, 255]))
            .unwrap();
        let effects = self.renderer.upload_effects(&self.gpu, geometry).unwrap();
        self.target
            .render_world(
                &self.gpu,
                &self.renderer,
                &scene(),
                &World {
                    parts,
                    effects: Some(&effects),
                    effect_textures: &[texture],
                    ..World::default()
                },
            )
            .unwrap()
    }
}

#[test]
fn effects_add_to_or_blend_over_what_is_behind_them() {
    let gpu = gpu();
    let setup = setup(gpu);
    let middle = (SIZE / 2, SIZE / 2);
    // One and one: added to the blue background.
    let added = setup.render_effects(&[], &effect_square(0.0, [0.5, 0.0, 0.0, 1.0], Some((2, 2))));
    let pixel = added.pixel(middle.0, middle.1).unwrap();
    assert!(near(pixel, [128, 0, 255, 255]), "{pixel:?}");
    // Source alpha and its inverse: half red, half blue.
    let blended =
        setup.render_effects(&[], &effect_square(0.0, [1.0, 0.0, 0.0, 0.5], Some((5, 6))));
    let pixel = blended.pixel(middle.0, middle.1).unwrap();
    assert!(near(pixel, [128, 0, 128, 255]), "{pixel:?}");
    let corner = blended.pixel(2, 2).unwrap();
    assert!(near(corner, BACKGROUND), "{corner:?}");
}

#[test]
fn effects_behind_a_model_are_hidden_and_do_not_hide_it() {
    let gpu = gpu();
    let setup = setup(gpu);
    let part = setup.upload(Some(solid([0, 255, 0, 255])));
    let middle = (SIZE / 2, SIZE / 2);
    // Behind the green quad (farther along +Y): not seen.
    let behind = setup.render_effects(&[&part], &effect_square(20.0, [1.0, 0.0, 0.0, 1.0], None));
    let pixel = behind.pixel(middle.0, middle.1).unwrap();
    assert!(near(pixel, [0, 255, 0, 255]), "{pixel:?}");
    // In front: added over it, and the quad still shows around it.
    let front = setup.render_effects(
        &[&part],
        &effect_square(-20.0, [1.0, 0.0, 0.0, 1.0], Some((2, 2))),
    );
    let pixel = front.pixel(middle.0, middle.1).unwrap();
    assert!(near(pixel, [255, 255, 0, 255]), "{pixel:?}");
}
