//! The interface pipeline: a draw list over an offscreen target, read back.
//! Needs a graphics adapter (a software one is enough); without one the tests
//! fail.

use glam::Vec3;
use m2_formats::RgbaImage;
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::{Camera, Gpu, GpuOptions, Offscreen, Renderer, Scene, UiFilter, UiRenderer};
use m2_ui::{DrawList, ImageId, Rect, Sprite};

const SIZE: u32 = 32;

fn gpu() -> Gpu {
    Gpu::headless(GpuOptions::default()).unwrap_or_else(|error| panic!("{error}"))
}

fn pixel(image: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
    image.pixel(x, y).unwrap()
}

#[test]
fn quads_blend_over_the_world_in_order_and_clip() {
    let gpu = gpu();
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT, 1).unwrap();
    let mut ui = UiRenderer::new(&gpu, OFFSCREEN_FORMAT, UiFilter::Sharp).unwrap();
    let target = Offscreen::new(&gpu, &renderer, SIZE, SIZE).unwrap();
    // A 2 × 1 image: red, then green.
    let image = RgbaImage::from_pixels(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 255]).unwrap();
    ui.set_image(&gpu, ImageId(3), &image).unwrap();
    assert!(ui.has_image(ImageId(3)) && !ui.has_image(ImageId(4)));

    let mut list = DrawList::default();
    // the green half of the image over the left half of the picture
    let green = Sprite {
        image: ImageId(3),
        uv: Rect::new(0.5, 0.0, 0.5, 1.0),
        size: (1.0, 1.0),
    };
    list.sprite(Rect::new(0.0, 0.0, 16.0, 32.0), &green, [255; 4], None);
    // half-transparent white over the top half, clipped to the top left quarter
    list.fill(
        Rect::new(0.0, 0.0, 32.0, 16.0),
        [255, 255, 255, 128],
        Some(Rect::new(0.0, 0.0, 16.0, 16.0)),
    );
    // an image never uploaded: skipped
    list.sprite(
        Rect::new(0.0, 0.0, 32.0, 32.0),
        &Sprite {
            image: ImageId(4),
            ..green
        },
        [255; 4],
        None,
    );

    let mut scene = Scene::new(Camera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance: 100.0,
        fov_y: Camera::FOV_Y,
    });
    scene.background = [0.0, 0.0, 1.0, 1.0];
    renderer
        .draw(&gpu, target.attachments(), target.view(), &scene, &[])
        .unwrap();
    ui.draw(&gpu, target.view(), (SIZE, SIZE), &list, &[])
        .unwrap();
    let picture = target.read(&gpu).unwrap();

    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2);
    assert!(
        near(pixel(&picture, 4, 24), [0, 255, 0, 255]),
        "green image"
    );
    assert!(
        near(pixel(&picture, 24, 24), [0, 0, 255, 255]),
        "background"
    );
    assert!(
        near(pixel(&picture, 4, 4), [128, 255, 128, 255]),
        "white blended over green"
    );
    assert!(
        near(pixel(&picture, 24, 4), [0, 0, 255, 255]),
        "clipped away"
    );
}

#[test]
fn smoothed_images_blend_their_texels_but_never_their_neighbours() {
    let gpu = gpu();
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT, 1).unwrap();
    // A 4 × 1 image: red, green, blue, white. The middle two are the part
    // drawn, 8 times their size.
    let pixels = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255; 4],
    ];
    let image = RgbaImage::from_pixels(4, 1, pixels.concat()).unwrap();
    let part = Sprite {
        image: ImageId(1),
        uv: Rect::new(0.25, 0.0, 0.5, 1.0),
        size: (2.0, 1.0),
    };
    let draw = |filter| {
        let mut ui = UiRenderer::new(&gpu, OFFSCREEN_FORMAT, filter).unwrap();
        ui.set_image(&gpu, ImageId(1), &image).unwrap();
        let target = Offscreen::new(&gpu, &renderer, SIZE, SIZE).unwrap();
        let mut list = DrawList::default();
        list.sprite(Rect::new(0.0, 0.0, 16.0, 8.0), &part, [255; 4], None);
        ui.draw(&gpu, target.view(), (SIZE, SIZE), &list, &[])
            .unwrap();
        target.read(&gpu).unwrap()
    };
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2);
    let (green, blue) = ([0, 255, 0, 255], [0, 0, 255, 255]);

    // Sharp: each texel 8 pixels wide.
    let sharp = draw(UiFilter::Sharp);
    for (x, colour) in [(0, green), (7, green), (8, blue), (15, blue)] {
        assert!(near(pixel(&sharp, x, 4), colour), "sharp at {x}");
    }
    // Smooth: green fades into blue in the middle, and the edges keep to the
    // part: no red on the left, no white on the right.
    let smooth = draw(UiFilter::Smooth);
    assert!(
        near(pixel(&smooth, 0, 4), green),
        "{:?}",
        pixel(&smooth, 0, 4)
    );
    assert!(
        near(pixel(&smooth, 15, 4), blue),
        "{:?}",
        pixel(&smooth, 15, 4)
    );
    for x in [7, 8] {
        let [r, g, b, _] = pixel(&smooth, x, 4);
        assert!(r <= 2 && g > 80 && b > 80, "blended at {x}: {r} {g} {b}");
    }
}

#[test]
fn a_picture_the_renderer_draws_shows_as_a_premultiplied_image() {
    let gpu = gpu();
    // A window's format: the picture and the screen are BGRA.
    let format = wgpu::TextureFormat::Bgra8Unorm;
    let renderer = Renderer::new(&gpu, format, 1).unwrap();
    let mut ui = UiRenderer::new(&gpu, format, UiFilter::Sharp).unwrap();
    let camera = Camera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance: 100.0,
        fov_y: Camera::FOV_Y,
    };
    // The picture: see-through red, as a resolved anti-aliased edge is
    // (colour already multiplied by its alpha of a half).
    let picture = Offscreen::new(&gpu, &renderer, 8, 8).unwrap();
    let mut scene = Scene::new(camera);
    scene.background = [0.5, 0.0, 0.0, 0.5];
    renderer
        .draw(&gpu, picture.attachments(), picture.view(), &scene, &[])
        .unwrap();
    ui.set_picture(&gpu, ImageId(7), picture.view());
    assert!(ui.has_image(ImageId(7)));

    let screen = Offscreen::new(&gpu, &renderer, SIZE, SIZE).unwrap();
    scene.background = [0.0, 0.0, 1.0, 1.0];
    renderer
        .draw(&gpu, screen.attachments(), screen.view(), &scene, &[])
        .unwrap();
    let mut list = DrawList::default();
    let sprite = Sprite {
        image: ImageId(7),
        uv: Rect::new(0.0, 0.0, 1.0, 1.0),
        size: (8.0, 8.0),
    };
    list.sprite(Rect::new(0.0, 0.0, 16.0, 16.0), &sprite, [255; 4], None);
    ui.draw(&gpu, screen.view(), (SIZE, SIZE), &list, &[])
        .unwrap();
    let image = screen.read(&gpu).unwrap();
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2);
    // Half red over blue, not a quarter (as straight alpha would make it).
    assert!(
        near(pixel(&image, 4, 4), [128, 0, 128, 255]),
        "{:?}",
        pixel(&image, 4, 4)
    );
    assert!(
        near(pixel(&image, 24, 24), [0, 0, 255, 255]),
        "BGRA read back as RGBA: {:?}",
        pixel(&image, 24, 24)
    );
}

#[test]
fn an_interface_alone_is_drawn_over_its_own_colour() {
    let gpu = gpu();
    let mut ui = UiRenderer::new(&gpu, OFFSCREEN_FORMAT, UiFilter::Sharp).unwrap();
    // No 3D renderer: a target for the interface, cleared by it.
    let target = Offscreen::for_interface(&gpu, SIZE, SIZE).unwrap();
    ui.clear(&gpu, target.view(), [1.0, 0.0, 0.0, 1.0]).unwrap();
    let mut list = DrawList::default();
    list.fill(Rect::new(0.0, 0.0, 16.0, 32.0), [0, 255, 0, 255], None);
    ui.draw(&gpu, target.view(), (SIZE, SIZE), &list, &[])
        .unwrap();
    let image = target.read(&gpu).unwrap();
    let near = |a: [u8; 4], b: [u8; 4]| a.iter().zip(b).all(|(x, y)| x.abs_diff(y) <= 2);
    assert!(near(pixel(&image, 4, 4), [0, 255, 0, 255]), "the fill");
    assert!(
        near(pixel(&image, 24, 24), [255, 0, 0, 255]),
        "the colour under it"
    );
    // Cleared again, the fill is gone.
    ui.clear(&gpu, target.view(), [0.0, 0.0, 1.0, 1.0]).unwrap();
    assert!(near(
        pixel(&target.read(&gpu).unwrap(), 4, 4),
        [0, 0, 255, 255]
    ));
    assert!(Offscreen::for_interface(&gpu, 0, SIZE).is_err());
}
