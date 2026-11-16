// The sky, behind everything: one triangle over the whole target. Colours
// stay in gamma space, like the rest.
//
// Each pixel's view ray picks its colour: from the colour bands by how high
// it points, or from the box's picture it meets; then the cloud layer above
// the camera adds its texture where the ray reaches it.

struct Sky {
    // The view rays: forward through the middle of the picture, plus right
    // and up as far as its right and top edges.
    forward: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    // x: bands above the horizon, y: bands below it, z: 1 when the box's
    // pictures are drawn, w: 1 with clouds.
    info: vec4<f32>,
    // x: the cloud layer's height over the camera, yz: 1 / how far it
    // reaches each way, in centimetres, w: x / the shorter reach, how much
    // the layer bends down.
    clouds: vec4<f32>,
    // xy: texture repeats per centimetre, zw: how far the clouds drifted.
    cloud_uv: vec4<f32>,
    // 1 where the box has a picture: front, back, left, right, then top,
    // bottom.
    faces: array<vec4<f32>, 2>,
    // Two colours per band, its top then its bottom: the bands above the
    // horizon from the highest down, then those below it from the horizon
    // down.
    bands: array<vec4<f32>, 32>,
};

@group(0) @binding(0) var<uniform> sky: Sky;
@group(0) @binding(1) var face_textures: texture_2d_array<f32>;
@group(0) @binding(2) var cloud_texture: texture_2d<f32>;
@group(0) @binding(3) var face_sampler: sampler;
@group(0) @binding(4) var cloud_sampler: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u)) * 2.0 - 1.0;
    var out: VertexOut;
    out.position = vec4<f32>(corner, 0.0, 1.0);
    out.ndc = corner;
    return out;
}

fn band(index: u32, along: f32) -> vec3<f32> {
    return mix(sky.bands[2u * index].rgb, sky.bands[2u * index + 1u].rgb, along);
}

// The colour bands lie on the box's sides, which reach as high and as low
// as they are far: the upper bands share the height from the top of the
// sides down to the horizon, the lower ones from there to the bottom. Above
// and below the sides is the nearest band's end colour.
fn gradient(dir: vec3<f32>) -> vec3<f32> {
    let upper = u32(sky.info.x);
    let lower = u32(sky.info.y);
    if upper + lower == 0u {
        return vec3<f32>(0.0);
    }
    let height = dir.z / max(length(dir.xy), 0.0001);
    if (height >= 0.0 && upper > 0u) || lower == 0u {
        let at = (1.0 - clamp(height, 0.0, 1.0)) * f32(upper);
        let index = min(u32(at), upper - 1u);
        return band(index, at - f32(index));
    }
    let at = clamp(-height, 0.0, 1.0) * f32(lower);
    let index = min(u32(at), lower - 1u);
    return band(upper + index, at - f32(index));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let dir = normalize(sky.forward.xyz + in.ndc.x * sky.right.xyz + in.ndc.y * sky.up.xyz);
    var colour = gradient(dir);

    // The box's pictures. The front faces south (−y), the left side east;
    // each picture is seen from inside, its top up (the top's towards the
    // back), as the client's pictures join.
    let a = abs(dir);
    var layer = 0u;
    var side = vec2<f32>(0.0);
    var depth = 1.0;
    if a.z >= a.x && a.z >= a.y {
        depth = a.z;
        if dir.z > 0.0 {
            layer = 4u;
            side = vec2<f32>(-dir.x, dir.y);
        } else {
            layer = 5u;
            side = vec2<f32>(-dir.x, -dir.y);
        }
    } else if a.y >= a.x {
        depth = a.y;
        if dir.y < 0.0 {
            layer = 0u;
            side = vec2<f32>(-dir.x, dir.z);
        } else {
            layer = 1u;
            side = vec2<f32>(dir.x, dir.z);
        }
    } else {
        depth = a.x;
        if dir.x > 0.0 {
            layer = 2u;
            side = vec2<f32>(-dir.y, dir.z);
        } else {
            layer = 3u;
            side = vec2<f32>(dir.y, dir.z);
        }
    }
    let uv = vec2<f32>(0.5 + 0.5 * side.x / depth, 0.5 - 0.5 * side.y / depth);
    // One level, so the seams between pictures take no smaller one.
    let picture = textureSampleLevel(face_textures, face_sampler, uv, layer, 0.0);
    let has = sky.faces[layer / 4u][layer % 4u] * sky.info.z;
    colour = mix(colour, picture.rgb, has);

    // The clouds: a layer over the camera that bends down to meet the
    // horizon at its reach, so the clouds low in the sky are not squeezed
    // as a flat layer's would be. A ray rising at angle a meets it
    // height / (tan a + bend) away along the ground. It fades out over the
    // outer two fifths of its reach and is added to what is behind it.
    let along = max(dir.z + sky.clouds.w * length(dir.xy), 0.0001);
    let hit = dir.xy * (sky.clouds.x / along);
    let edge = length(hit * sky.clouds.yz);
    let fade = (1.0 - smoothstep(0.6, 1.0, edge)) * step(0.001, dir.z) * sky.info.w;
    let texel = textureSample(cloud_texture, cloud_sampler, hit * sky.cloud_uv.xy + sky.cloud_uv.zw);
    colour = colour + texel.rgb * texel.a * fade;
    return vec4<f32>(min(colour, vec3<f32>(1.0)), 1.0);
}
