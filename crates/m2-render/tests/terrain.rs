//! Terrain: meshes built from synthetic height, tile and water maps, the
//! texture helpers, and (with a graphics adapter) areas drawn from above.
//! Without an adapter the GPU tests fail.

use glam::{Mat4, Vec3};
use m2_formats::RgbaImage;
use m2_formats::map::grid::{Grid, HeightMap, NO_WATER, ShadowMap, TileMap, WaterMap};
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::terrain::{AREA_VERTICES, layer_side, mip_chain, resample};
use m2_render::{
    AreaInput, AreaMesh, Camera, Gpu, GpuOptions, MapScale, Offscreen, Renderer, Scene,
    TerrainLayer, World,
};

const SCALE: MapScale = MapScale {
    cell: 200.0,
    height: 0.5,
};

fn area(x: u32, y: u32, heights: &HeightMap) -> AreaMesh {
    AreaMesh::new(&AreaInput {
        x,
        y,
        heights,
        tiles: &TileMap::filled(1),
        shadows: None,
        water: None,
        scale: SCALE,
    })
    .unwrap()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn a_flat_area_lies_in_its_place_in_the_world() {
    let mesh = area(1, 2, &HeightMap::flat(100));
    // Area (1, 2): x from 25600, y from −51200 going south (negative y).
    assert_eq!(mesh.origin, [25_600.0, -51_200.0]);
    assert_eq!(mesh.vertices.len(), 129 * 129);
    assert_eq!(AREA_VERTICES, 129);
    assert_eq!(mesh.vertices[0].position, [25_600.0, -51_200.0, 50.0]);
    assert_eq!(mesh.vertices[1].position, [25_800.0, -51_200.0, 50.0]);
    assert_eq!(mesh.vertices[129].position, [25_600.0, -51_400.0, 50.0]);
    assert_eq!(
        mesh.vertices.last().unwrap().position,
        [51_200.0, -76_800.0, 50.0]
    );
    assert!(mesh.vertices.iter().all(|v| v.normal == [0.0, 0.0, 1.0]));
    assert_eq!(mesh.indices.len(), 128 * 128 * 6);
    assert!(
        mesh.indices
            .iter()
            .all(|&i| (i as usize) < mesh.vertices.len())
    );
    assert_eq!(mesh.bounds.min, Vec3::new(25_600.0, -76_800.0, 50.0));
    assert_eq!(mesh.bounds.max, Vec3::new(51_200.0, -51_200.0, 50.0));
    assert_eq!(mesh.tiles.len(), 258 * 258);
    assert!(mesh.water.is_empty() && mesh.shadows.is_none());
}

#[test]
fn slopes_tilt_the_normals_and_heights_follow_the_triangles() {
    // Rising 10 steps (5 cm) per vertex eastwards, border included.
    let mut east = HeightMap::flat(0);
    for y in 0..131 {
        for x in 0..131 {
            east.grid.set(x, y, (x * 10) as u16);
        }
    }
    let mesh = area(0, 0, &east);
    let n = Vec3::from_array(mesh.vertices[129 * 64 + 64].normal);
    // Higher to the east: the normal leans west.
    let expected = Vec3::new(-5.0 / 200.0, 0.0, 1.0).normalize();
    assert!(n.abs_diff_eq(expected, 1e-5), "{n}");
    // Vertex (i, j) is stored value (i + 1, j + 1): 5 cm × (i + 1).
    assert!(close(mesh.height_at(0.0, 0.0).unwrap(), 5.0));
    assert!(close(mesh.height_at(200.0, -400.0).unwrap(), 10.0));
    assert!(close(mesh.height_at(100.0, -50.0).unwrap(), 7.5));
    assert_eq!(mesh.height_at(-1.0, 0.0), None);
    assert_eq!(mesh.height_at(0.0, 1.0), None);
    assert!(mesh.height_at(25_600.0, -25_600.0).is_some());

    // Rising southwards: higher towards −y, so the normal leans north (+y).
    let mut south = HeightMap::flat(0);
    for y in 0..131 {
        for x in 0..131 {
            south.grid.set(x, y, (y * 10) as u16);
        }
    }
    let n = Vec3::from_array(area(0, 0, &south).vertices[129 * 64 + 64].normal);
    assert!(n.y > 0.0 && close(n.x, 0.0), "{n}");
}

#[test]
fn water_cells_become_flat_squares_at_their_layer_height() {
    let mut layers = Grid::filled(128, 128, NO_WATER);
    layers.set(3, 5, 0);
    let water = WaterMap {
        layers,
        heights: vec![300],
    };
    let mesh = AreaMesh::new(&AreaInput {
        x: 0,
        y: 0,
        heights: &HeightMap::flat(0),
        tiles: &TileMap::filled(1),
        shadows: Some(&ShadowMap::none().to_image().unwrap()),
        water: Some(&water),
        scale: SCALE,
    })
    .unwrap();
    assert_eq!(mesh.water.len(), 6);
    let xs: Vec<f32> = mesh.water.iter().map(|p| p[0]).collect();
    let ys: Vec<f32> = mesh.water.iter().map(|p| p[1]).collect();
    assert!(mesh.water.iter().all(|p| p[2] == 150.0));
    assert_eq!(xs.iter().copied().fold(f32::MAX, f32::min), 600.0);
    assert_eq!(xs.iter().copied().fold(f32::MIN, f32::max), 800.0);
    assert_eq!(ys.iter().copied().fold(f32::MAX, f32::min), -1200.0);
    assert_eq!(ys.iter().copied().fold(f32::MIN, f32::max), -1000.0);
    assert_eq!(mesh.bounds.max.z, 150.0);
    let shadows = mesh.shadows.unwrap();
    assert_eq!(shadows.pixel(0, 0), Some([255, 255, 255, 255]));
}

#[test]
fn bad_scales_are_refused() {
    let input = AreaInput {
        x: 0,
        y: 0,
        heights: &HeightMap::flat(0),
        tiles: &TileMap::filled(0),
        shadows: None,
        water: None,
        scale: MapScale {
            cell: 0.0,
            height: 0.5,
        },
    };
    assert!(AreaMesh::new(&input).is_err());
}

#[test]
fn layers_share_one_power_of_two_side() {
    let layer = |side| {
        Some(TerrainLayer {
            image: RgbaImage::new(side, side).unwrap(),
            scale: [1.0, 1.0],
        })
    };
    assert_eq!(layer_side(&[]), 1);
    assert_eq!(layer_side(&[None, layer(64)]), 64);
    assert_eq!(layer_side(&[layer(64), layer(300)]), 512);
    assert_eq!(layer_side(&[layer(2048)]), 512);
}

#[test]
fn resampling_and_mip_levels() {
    // A 2 × 2 image scaled to 4 × 4 keeps its colours in the corners.
    let image = RgbaImage::from_pixels(
        2,
        2,
        [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255, 255, 255, 255],
        ]
        .concat(),
    )
    .unwrap();
    assert_eq!(resample(&image, 2).unwrap(), image);
    let big = resample(&image, 4).unwrap();
    assert_eq!((big.width(), big.height()), (4, 4));
    // Texel (0, 0) lies between the red texel and the wrapped-around
    // neighbours; a quarter step from red's centre it is mostly red.
    let corner = big.pixel(0, 0).unwrap();
    assert!(corner[0] > corner[1] && corner[0] > corner[2], "{corner:?}");

    let levels = mip_chain(RgbaImage::filled(4, 4, [10, 20, 30, 255]).unwrap()).unwrap();
    assert_eq!(
        levels.iter().map(|l| l.width()).collect::<Vec<_>>(),
        [4, 2, 1]
    );
    assert_eq!(levels[2].pixel(0, 0), Some([10, 20, 30, 255]));
    let averaged = mip_chain(image).unwrap();
    assert_eq!(averaged[1].pixel(0, 0), Some([128, 128, 128, 255]));
}

// ---------------------------------------------------------------- on the GPU

fn gpu() -> Gpu {
    Gpu::headless(GpuOptions::default()).unwrap_or_else(|error| panic!("{error}"))
}

fn solid(rgba: [u8; 4]) -> Option<TerrainLayer> {
    Some(TerrainLayer {
        image: RgbaImage::filled(4, 4, rgba).unwrap(),
        scale: [5.0, 5.0],
    })
}

fn near(actual: [u8; 4], expected: [u8; 4], within: u8) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| a.abs_diff(e) <= within)
}

/// Looks straight down on area (0, 0), unlit, the area filling the picture.
fn top_down() -> Scene {
    let mut scene = Scene::new(Camera {
        target: Vec3::new(12_800.0, -12_800.0, 0.0),
        yaw: 0.0,
        pitch: 89.9f32.to_radians(),
        distance: 12_800.0 / (Camera::FOV_Y * 0.5).tan(),
        fov_y: Camera::FOV_Y,
    });
    scene.ambient = [1.0; 3];
    scene.diffuse = [0.0; 3];
    scene
}

#[test]
fn tiles_choose_the_texture_and_shadows_darken_it() {
    let gpu = gpu();
    let renderer =
        Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT)).unwrap();
    // West half texture 1 (red), east half texture 2 (green); texture 3 is
    // named by the south-east quarter but missing from the set, so those
    // tiles blend only their neighbours.
    let mut tiles = TileMap::filled(1);
    for y in 0..258 {
        for x in 129..258 {
            tiles.grid.set(x, y, if y > 129 { 3 } else { 2 });
        }
    }
    // The north-west quarter's shadow texels are half dark.
    let mut shadows = ShadowMap::none();
    for y in 0..128 {
        for x in 0..128 {
            shadows.grid.set(x, y, 0x4210);
        }
    }
    let mesh = AreaMesh::new(&AreaInput {
        x: 0,
        y: 0,
        heights: &HeightMap::flat(0),
        tiles: &tiles,
        shadows: Some(&shadows.to_image().unwrap()),
        water: None,
        scale: SCALE,
    })
    .unwrap();
    let set = renderer
        .upload_layers(
            &gpu,
            &[solid([255, 0, 0, 255]), solid([0, 255, 0, 255])],
            3200.0,
        )
        .unwrap();
    let area = renderer.upload_area(&gpu, &mesh).unwrap();
    let target = Offscreen::new(&gpu, &renderer, 64, 64).unwrap();
    let image = target
        .render_world(
            &gpu,
            &renderer,
            &top_down(),
            &World {
                terrain: Some(&set),
                areas: &[&area],
                parts: &[],
                ..World::default()
            },
        )
        .unwrap();
    // North is up: the shadowed red quarter top left, plain red bottom
    // left, green top right.
    let shaded = image.pixel(16, 16).unwrap();
    assert!(near(shaded, [132, 0, 0, 255], 3), "{shaded:?}");
    let red = image.pixel(16, 48).unwrap();
    assert!(near(red, [255, 0, 0, 255], 2), "{red:?}");
    let green = image.pixel(48, 16).unwrap();
    assert!(near(green, [0, 255, 0, 255], 2), "{green:?}");
    // Tiles naming a missing texture: grey where no neighbour has one.
    let missing = image.pixel(52, 52).unwrap();
    assert!(near(missing, [128, 128, 128, 255], 2), "{missing:?}");
}

#[test]
fn water_is_drawn_over_the_terrain_and_buildings_go_where_their_palette_puts_them() {
    let gpu = gpu();
    let renderer =
        Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT)).unwrap();
    let mut layers = Grid::filled(128, 128, NO_WATER);
    for y in 64..128 {
        for x in 64..128 {
            layers.set(x, y, 0);
        }
    }
    let mesh = AreaMesh::new(&AreaInput {
        x: 0,
        y: 0,
        heights: &HeightMap::flat(0),
        tiles: &TileMap::filled(1),
        shadows: None,
        water: Some(&WaterMap {
            layers,
            heights: vec![200],
        }),
        scale: SCALE,
    })
    .unwrap();
    let set = renderer
        .upload_layers(&gpu, &[solid([255, 255, 255, 255])], 3200.0)
        .unwrap();
    let area = renderer.upload_area(&gpu, &mesh).unwrap();
    let target = Offscreen::new(&gpu, &renderer, 64, 64).unwrap();
    let image = target
        .render_world(
            &gpu,
            &renderer,
            &top_down(),
            &World {
                terrain: Some(&set),
                areas: &[&area],
                parts: &[],
                ..World::default()
            },
        )
        .unwrap();
    let dry = image.pixel(16, 16).unwrap();
    assert!(near(dry, [255, 255, 255, 255], 2), "{dry:?}");
    // Water blends its colour over the white ground.
    let wet = image.pixel(48, 48).unwrap();
    assert!(wet[2] > wet[0] && wet[0] < 200, "{wet:?}");

    // Instances share a part's buffers but each has its own palette.
    let part = m2_render::Part {
        vertices: [
            [-1000.0, -1000.0],
            [1000.0, -1000.0],
            [1000.0, 1000.0],
            [-1000.0, 1000.0],
        ]
        .iter()
        .map(|&[x, y]| m2_render::Vertex {
            position: [x, y, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            joints: [0; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
        })
        .collect(),
        indices: vec![0, 1, 2, 0, 2, 3],
        draws: vec![m2_render::model::Draw {
            first: 0,
            count: 6,
            texture: None,
        }],
        ..m2_render::Part::default()
    };
    let uploaded = renderer.upload(&gpu, &part, &[]).unwrap();
    let instance = renderer.instance(&gpu, &uploaded);
    // One square over the north-west quarter, the other over the north-east,
    // both above the ground; untextured parts draw white, so make the ground
    // dark first.
    let set = renderer
        .upload_layers(&gpu, &[solid([0, 0, 0, 255])], 3200.0)
        .unwrap();
    let place = |x: f32| Mat4::from_translation(Vec3::new(x, -6400.0, 100.0));
    renderer.set_palette(&gpu, &uploaded, &[place(6400.0)]);
    renderer.set_palette(&gpu, &instance, &[place(19_200.0)]);
    let image = target
        .render_world(
            &gpu,
            &renderer,
            &top_down(),
            &World {
                terrain: Some(&set),
                areas: &[&area],
                parts: &[&uploaded, &instance],
                ..World::default()
            },
        )
        .unwrap();
    assert!(near(image.pixel(16, 16).unwrap(), [255, 255, 255, 255], 2));
    assert!(near(image.pixel(48, 16).unwrap(), [255, 255, 255, 255], 2));
    assert!(near(image.pixel(32, 16).unwrap(), [0, 0, 0, 255], 2));
}
