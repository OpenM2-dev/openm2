//! What the renderer computes on the CPU: poses, parts, the camera, PNG.
//! No GPU needed.

mod common;

use common::{IDENTITY, bone, moved, moving, quad, quad_file, skeleton};
use glam::{Mat4, Vec3, Vec4};
use m2_formats::dds::{self, Dds, DdsFormat, Masks};
use m2_formats::gr2::model::{Gr2File, Material, Texture};
use m2_formats::model::{self as layer, Animation, Skeleton, Transform};
use m2_formats::{RawStr, RgbaImage};
use m2_render::model::{Draw, MAX_BONES};
use m2_render::pose::transform_matrix;
use m2_render::{AnimationBinding, Bounds, Camera, Part, Pose, RenderError, TextureData};

fn close(a: Vec3, b: Vec3) -> bool {
    (a - b).length() < 1e-4
}

fn translation(m: Mat4) -> Vec3 {
    m.w_axis.truncate()
}

#[test]
fn a_transform_scales_then_rotates_then_moves() {
    let quarter = std::f32::consts::FRAC_PI_4.sin();
    let t = Transform {
        flags: 7,
        position: [10.0, 0.0, 0.0],
        orientation: [0.0, 0.0, quarter, quarter], // 90° around Z
        scale_shear: [[2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    };
    // (1, 0, 0) → scaled (2, 0, 0) → turned (0, 2, 0) → moved (10, 2, 0)
    let p = transform_matrix(&t) * Vec4::new(1.0, 0.0, 0.0, 1.0);
    assert!(close(p.truncate(), Vec3::new(10.0, 2.0, 0.0)), "{p}");
}

#[test]
fn scale_shear_is_stored_by_rows() {
    let t = Transform {
        scale_shear: [[1.0, 3.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        ..Transform::default()
    };
    // Row 0 is (1, 3, 0): x' = x + 3y.
    let p = transform_matrix(&t) * Vec4::new(0.0, 1.0, 0.0, 1.0);
    assert!(close(p.truncate(), Vec3::new(3.0, 1.0, 0.0)), "{p}");
}

#[test]
fn a_zero_or_broken_quaternion_counts_as_no_rotation() {
    for orientation in [[0.0; 4], [f32::NAN, 0.0, 0.0, 1.0]] {
        let t = Transform {
            orientation,
            ..Transform::default()
        };
        assert_eq!(transform_matrix(&t), Mat4::IDENTITY);
    }
}

#[test]
fn the_rest_pose_chains_bones_and_its_palette_is_the_identity() {
    let skeleton = skeleton();
    let pose = Pose::rest(&skeleton);
    assert!(close(
        translation(pose.world()[1]),
        Vec3::new(0.0, 0.0, 50.0)
    ));
    let palette = pose.palette(&skeleton);
    // One matrix per bone, then the identity for unbound meshes.
    assert_eq!(palette.len(), 3);
    for m in palette {
        assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-5), "{m}");
    }
}

#[test]
fn an_animation_moves_the_bones_it_names_and_children_follow() {
    let mut skeleton = skeleton();
    skeleton.bones.push(bone(
        "hand",
        Some(1),
        moved(10.0, 0.0, 0.0),
        [10.0, 0.0, 50.0],
    ));
    let animation = moving("ROOT", [0.0; 3], [100.0, 0.0, 0.0]);
    let binding = AnimationBinding::new(&skeleton, &animation);
    // Names match ignoring case.
    assert_eq!(binding.matched(), 1);

    let pose = Pose::animated(&skeleton, &animation, &binding, 0.5);
    assert!(close(
        translation(pose.world()[0]),
        Vec3::new(50.0, 0.0, 0.0)
    ));
    assert!(close(
        translation(pose.world()[2]),
        Vec3::new(60.0, 0.0, 50.0)
    ));
    // The skinning matrix moves rest positions by the same amount.
    let palette = pose.palette(&skeleton);
    assert!(close(translation(palette[2]), Vec3::new(50.0, 0.0, 0.0)));

    // Times past the end are clamped.
    let late = Pose::animated(&skeleton, &animation, &binding, 7.0);
    assert!(close(
        translation(late.world()[0]),
        Vec3::new(100.0, 0.0, 0.0)
    ));
}

#[test]
fn the_skeletons_own_track_group_moves_it_before_others() {
    let skeleton = skeleton();
    // A helper object's tracks first, with the same bone names.
    let mut animation = moving("root", [0.0; 3], [0.0, 0.0, -100.0]);
    animation.track_groups[0].name = RawStr::from("Dummy01");
    let own = moving("root", [0.0; 3], [100.0, 0.0, 0.0]);
    animation.track_groups.extend(own.track_groups);
    let at = |animation: &Animation| {
        let binding = AnimationBinding::new(&skeleton, animation);
        translation(Pose::animated(&skeleton, animation, &binding, 1.0).world()[0])
    };
    // The group named like the skeleton (`root`, any case) wins.
    assert!(close(at(&animation), Vec3::new(100.0, 0.0, 0.0)));
    // Without one, the first group that has the bone.
    animation.track_groups[1].name = RawStr::from("Other");
    assert!(close(at(&animation), Vec3::new(0.0, 0.0, -100.0)));
}

#[test]
fn values_that_are_not_finite_keep_the_rest_pose() {
    let skeleton = skeleton();
    let mut animation = moving("arm", [f32::NAN, 0.0, 0.0], [f32::NAN, 0.0, 0.0]);
    let track = &mut animation.track_groups[0].tracks[0];
    track.orientation = common::linear(4, &[(0.0, &[f32::NAN, 0.0, 0.0, 1.0])]);
    let binding = AnimationBinding::new(&skeleton, &animation);
    let pose = Pose::animated(&skeleton, &animation, &binding, 0.5);
    assert_eq!(pose, Pose::rest(&skeleton));
}

#[test]
fn a_part_follows_bones_of_the_same_name_and_keeps_the_rest() {
    let main = skeleton();
    let animation = moving("arm", [0.0, 0.0, 50.0], [0.0, 0.0, 80.0]);
    let pose = Pose::animated(
        &main,
        &animation,
        &AnimationBinding::new(&main, &animation),
        1.0,
    );

    // Hair: an `Arm` bone (same name, other case) and a child the main
    // skeleton does not have.
    let hair = Skeleton {
        name: RawStr::from("hair"),
        bones: vec![
            bone("Arm", None, moved(0.0, 0.0, 50.0), [0.0, 0.0, 50.0]),
            bone("tip", Some(0), moved(0.0, 0.0, 5.0), [0.0, 0.0, 55.0]),
        ],
    };
    let followed = pose.follow(&hair);
    assert!(close(
        translation(followed.world()[0]),
        Vec3::new(0.0, 0.0, 80.0)
    ));
    assert!(close(
        translation(followed.world()[1]),
        Vec3::new(0.0, 0.0, 85.0)
    ));
    assert!(pose.find(&RawStr::from("ARM")).is_some());
    assert!(pose.find(&RawStr::from("leg")).is_none());
}

#[test]
fn a_held_model_goes_where_the_bone_is() {
    let main = skeleton();
    let animation = moving("arm", [0.0, 0.0, 50.0], [0.0, 0.0, 80.0]);
    let pose = Pose::animated(
        &main,
        &animation,
        &AnimationBinding::new(&main, &animation),
        1.0,
    );
    // A sword: its own bone 10 cm above its origin, a tip above that.
    let sword = Skeleton {
        name: RawStr::from("sword"),
        bones: vec![
            bone("blade", None, moved(0.0, 0.0, 10.0), [0.0, 0.0, 10.0]),
            bone("tip", Some(0), moved(0.0, 0.0, 90.0), [0.0, 0.0, 100.0]),
        ],
    };
    let held = pose.held(&RawStr::from("ARM"), &sword).unwrap();
    assert!(close(
        translation(held.world()[0]),
        Vec3::new(0.0, 0.0, 90.0)
    ));
    assert!(close(
        translation(held.world()[1]),
        Vec3::new(0.0, 0.0, 180.0)
    ));
    // Its vertices, given where the model is at rest, move with the bone.
    for matrix in &held.palette(&sword)[..2] {
        assert!(close(translation(*matrix), Vec3::new(0.0, 0.0, 80.0)));
    }
    assert!(pose.held(&RawStr::from("leg"), &sword).is_none());
}

#[test]
fn a_model_becomes_one_vertex_list_with_joints_into_its_skeleton() {
    let part = Part::from_model(&quad()).unwrap();
    assert_eq!(part.name, RawStr::from("quad"));
    assert_eq!(part.vertices.len(), 4);
    assert_eq!(part.indices, [0, 1, 2, 0, 2, 3]);
    // Mesh bone 0 is `arm`, bone 1 of the skeleton (the file's other three
    // indexes are 0 too, with weight 0).
    assert_eq!(part.vertices[0].joints, [1, 1, 1, 1]);
    assert_eq!(part.vertices[0].weights, [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(
        part.draws,
        [Draw {
            first: 0,
            count: 6,
            texture: Some(0)
        }]
    );
    assert_eq!(part.textures, [layer::Texture::file("skin.dds")]);
}

#[test]
fn meshes_are_appended_with_their_indexes_offset() {
    let mut file = quad_file();
    let mut second = file.meshes[0].clone();
    // Bound to a bone the skeleton lacks, without weights: rigid on the
    // identity entry. Weights that do not add up to 1 are normalized.
    second.bones = vec![RawStr::from("tail")];
    second.vertices.bone_weights.clear();
    file.meshes[0].vertices.bone_weights[1] = [0.5, 0.5, 0.0, 0.0];
    file.meshes[0].vertices.bone_indices[1] = [0, 0, 0, 0];
    file.meshes.push(second);
    file.models[0].meshes.push(1);

    let part = Part::from_model(&file.model(0).unwrap()).unwrap();
    assert_eq!(part.vertices.len(), 8);
    assert_eq!(&part.indices[6..], &[4, 5, 6, 4, 6, 7]);
    assert_eq!(part.vertices[4].joints, [2, 2, 2, 2]);
    assert_eq!(part.vertices[4].weights, [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(part.vertices[1].weights, [0.5, 0.5, 0.0, 0.0]);
    assert_eq!(part.draws.len(), 2);
    assert_eq!(part.draws[1].first, 6);
    // Both meshes use the same texture: listed once.
    assert_eq!(part.textures.len(), 1);
}

#[test]
fn a_material_without_a_texture_uses_its_maps() {
    let mut file = quad_file();
    file.textures.push(Texture {
        file_name: RawStr::from("other.dds"),
    });
    file.materials = vec![
        Material {
            name: RawStr::from("mesh material"),
            texture: None,
            maps: vec![(RawStr::from("Diffuse Color"), Some(1))],
        },
        Material {
            name: RawStr::from("map"),
            texture: Some(1),
            maps: Vec::new(),
        },
    ];
    let part = Part::from_model(&file.model(0).unwrap()).unwrap();
    assert_eq!(part.textures, [layer::Texture::file("other.dds")]);
}

#[test]
fn models_without_a_skeleton_or_with_too_many_bones_are_handled() {
    let mut file = quad_file();
    file.models.clear();
    let part = Part::from_model(&file.model(0).unwrap()).unwrap();
    assert!(part.skeleton.bones.is_empty());
    // No skeleton: every joint is the identity entry 0.
    assert_eq!(part.vertices[0].joints, [0, 0, 0, 0]);

    let mut big = quad();
    big.skeleton.bones = (0..=MAX_BONES)
        .map(|i| bone(&format!("b{i}"), None, Transform::default(), [0.0; 3]))
        .collect();
    assert!(matches!(Part::from_model(&big), Err(RenderError::Model(_))));
}

#[test]
fn bounds_follow_the_palette() {
    let part = Part::from_model(&quad()).unwrap();
    let mut palette = vec![Mat4::IDENTITY; 3];
    let rest = part.bounds(&palette).unwrap();
    assert!(close(rest.min, Vec3::new(-50.0, 0.0, 0.0)));
    assert!(close(rest.max, Vec3::new(50.0, 0.0, 100.0)));
    palette[1] = Mat4::from_translation(Vec3::new(0.0, 0.0, 10.0));
    let moved = part.bounds(&palette).unwrap();
    assert!(close(moved.max, Vec3::new(50.0, 0.0, 110.0)));
    assert!(Part::default().bounds(&palette).is_none());
}

#[test]
fn texture_levels_must_halve() {
    let image = |w, h| RgbaImage::new(w, h).unwrap();
    assert!(TextureData::new(vec![image(4, 2), image(2, 1), image(1, 1)]).is_ok());
    assert!(TextureData::new(vec![image(4, 2), image(2, 2)]).is_err());
    assert!(TextureData::new(vec![image(1, 1), image(1, 1)]).is_err());
    assert!(TextureData::new(Vec::new()).is_err());
}

#[test]
fn only_one_bit_alpha_is_coverage() {
    let texture = |format: DdsFormat| {
        let data = vec![0u8; format.level_len(4, 4) as usize];
        let bytes = dds::write(4, 4, &format, &[&data]).unwrap();
        TextureData::from_dds(&Dds::parse(&bytes).unwrap()).unwrap()
    };
    assert!(texture(DdsFormat::Bc1).coverage());
    assert!(!texture(DdsFormat::Bc2).coverage());
    assert!(!texture(DdsFormat::Bc3).coverage());
    assert!(!texture(DdsFormat::Masked(Masks::BGRA8)).coverage());
    let a1r5g5b5 = Masks {
        bits: 16,
        r: 0x7c00,
        g: 0x03e0,
        b: 0x001f,
        a: 0x8000,
        luminance: false,
    };
    assert!(texture(DdsFormat::Masked(a1r5g5b5)).coverage());
}

#[test]
fn the_camera_frames_every_corner_of_the_box() {
    let bounds = Bounds {
        min: Vec3::new(-30.0, -10.0, 0.0),
        max: Vec3::new(30.0, 10.0, 180.0),
    };
    for (yaw, pitch, aspect) in [(0.0, 0.0, 1.0), (1.0, 0.3, 0.5), (-2.0, -0.4, 2.0)] {
        let camera = Camera::framing(bounds, yaw, pitch, aspect);
        let view_proj = camera.view_proj(aspect);
        for corner in 0..8 {
            let p = Vec3::new(
                if corner & 1 == 0 {
                    bounds.min.x
                } else {
                    bounds.max.x
                },
                if corner & 2 == 0 {
                    bounds.min.y
                } else {
                    bounds.max.y
                },
                if corner & 4 == 0 {
                    bounds.min.z
                } else {
                    bounds.max.z
                },
            );
            let clip = view_proj * p.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0,
                "{yaw} {pitch}: {ndc}"
            );
            assert!((0.0..=1.0).contains(&ndc.z), "{ndc}");
        }
    }
}

#[test]
fn yaw_zero_looks_from_minus_y_and_turns_counter_clockwise() {
    let mut camera = Camera {
        target: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        distance: 10.0,
        fov_y: Camera::FOV_Y,
    };
    assert!(close(camera.eye(), Vec3::new(0.0, -10.0, 0.0)));
    camera.yaw = std::f32::consts::FRAC_PI_2;
    assert!(close(camera.eye(), Vec3::new(10.0, 0.0, 0.0)));
    // The light comes from the camera's side.
    assert!(camera.light_direction().x > 0.0);
}

#[test]
fn a_ray_through_a_spot_of_the_picture_projects_back_onto_it() {
    let camera = Camera {
        target: Vec3::new(1000.0, -2000.0, 50.0),
        yaw: 0.7,
        pitch: 0.6,
        distance: 1500.0,
        fov_y: Camera::FOV_Y,
    };
    let aspect = 16.0 / 9.0;
    let view_proj = camera.view_proj(aspect);
    // The middle of the picture looks at the target.
    let (eye, direction) = camera.ray(aspect, 0.0, 0.0);
    assert!(close(eye, camera.eye()));
    assert!(close(eye + direction * 1500.0, camera.target));
    for (x, y) in [(-0.9, 0.8), (0.5, -0.3), (1.0, 1.0)] {
        let (eye, direction) = camera.ray(aspect, x, y);
        assert!((direction.length() - 1.0).abs() < 1e-5);
        for along in [500.0, 3000.0] {
            let clip = view_proj * (eye + direction * along).extend(1.0);
            let ndc = clip / clip.w;
            assert!(
                (ndc.x - x).abs() < 1e-3 && (ndc.y - y).abs() < 1e-3,
                "{x} {y}: {ndc}"
            );
        }
    }
}

#[test]
fn png_round_trips() {
    let mut image = RgbaImage::new(3, 2).unwrap();
    image.set_pixel(2, 1, [1, 2, 3, 4]);
    let bytes = m2_render::png::encode(&image).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(m2_render::png::decode(&bytes).unwrap(), image);
    assert!(m2_render::png::decode(b"not a png").is_err());
}

#[test]
fn grey_pngs_keep_their_samples() {
    use m2_render::png::{decode_grey8, decode_grey16, encode_grey8, encode_grey16};
    let heights = [0u16, 1, 0x1234, 0xFFFF, 300, 7];
    let bytes = encode_grey16(3, 2, &heights, Some("marked")).unwrap();
    assert_eq!(decode_grey16(&bytes).unwrap(), (3, 2, heights.to_vec()));
    let tiles = [0u8, 1, 2, 255, 17, 3];
    let bytes = encode_grey8(2, 3, &tiles, None).unwrap();
    assert_eq!(decode_grey8(&bytes).unwrap(), (2, 3, tiles.to_vec()));
    // Each reads only its own depth, and grey only.
    let error = decode_grey16(&bytes).unwrap_err().to_string();
    assert!(error.contains("only 16-bit grey"), "{error}");
    let rgba = m2_render::png::encode(&RgbaImage::new(2, 2).unwrap()).unwrap();
    let error = decode_grey8(&rgba).unwrap_err().to_string();
    assert!(error.contains("Rgba"), "{error}");
    // The samples must fill the picture.
    assert!(encode_grey8(2, 2, &tiles, None).is_err());
}

#[test]
fn identity_constant_is_a_column_major_identity() {
    assert_eq!(Mat4::from_cols_array(&IDENTITY), Mat4::IDENTITY);
    let _: Gr2File = quad_file();
}
