// Terrain and water. Colours stay in gamma space, like the models.
//
// The terrain blends the textures of the four tiles nearest to each pixel
// (a tile is half a cell), lights the result and darkens it by the area's
// baked shadow map. Water is a flat, see-through colour.

struct Frame {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    // x: fog start, y: fog end (no fog when y <= x), in centimetres.
    fog: vec4<f32>,
    // rgb: fog colour.
    fog_colour: vec4<f32>,
};

// The texture set: x of `info` is 1 / the span of one repeat at scale 1.
// Per texture number: xy its scale, z 1 if the set has it.
struct Layers {
    info: vec4<f32>,
    layers: array<vec4<f32>, 256>,
};

// One area: xy its north-west corner in the world, z the side of a cell,
// w 1 / the side of the area.
struct Area {
    place: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var layer_textures: texture_2d_array<f32>;
@group(1) @binding(1) var layer_sampler: sampler;
@group(1) @binding(2) var<uniform> terrain_set: Layers;
@group(2) @binding(0) var tiles: texture_2d<u32>;
@group(2) @binding(1) var shadow_texture: texture_2d<f32>;
@group(2) @binding(2) var shadow_sampler: sampler;
@group(2) @binding(3) var<uniform> area: Area;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
};

@vertex
fn vs_terrain(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = frame.view_proj * vec4<f32>(v.position, 1.0);
    out.world = v.position;
    out.normal = v.normal;
    return out;
}

fn fogged(colour: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if frame.fog.y <= frame.fog.x {
        return colour;
    }
    let distance = length(world - frame.eye.xyz);
    let amount = clamp((distance - frame.fog.x) / (frame.fog.y - frame.fog.x), 0.0, 1.0);
    return mix(colour, frame.fog_colour.rgb, amount);
}

// The texture number of tile (x, y) of the stored map (border included).
fn tile(at: vec2<i32>) -> u32 {
    let side = i32(textureDimensions(tiles).x);
    return textureLoad(tiles, clamp(at, vec2<i32>(0), vec2<i32>(side - 1)), 0).r;
}

// One tile's texture and how much it counts (0 for no texture).
fn sample_layer(number: u32, map_cm: vec2<f32>) -> vec4<f32> {
    let index = max(i32(number) - 1, 0);
    let layer = terrain_set.layers[clamp(index, 0, 255)];
    let uv = map_cm * layer.xy * terrain_set.info.x;
    let colour = textureSample(layer_textures, layer_sampler, uv, index);
    let present = select(0.0, layer.z, number > 0u);
    return vec4<f32>(colour.rgb, present);
}

@fragment
fn fs_terrain(in: VertexOut) -> @location(0) vec4<f32> {
    // Centimetres from the area's north-west corner, y growing south.
    let local = vec2<f32>(in.world.x - area.place.x, area.place.y - in.world.y);
    // Tile coordinates from tile centres; the stored map has a border of one.
    let t = local / (area.place.z * 0.5) - vec2<f32>(0.5);
    let t0 = floor(t);
    let f = t - t0;
    let base = vec2<i32>(t0) + vec2<i32>(1);
    let map_cm = vec2<f32>(in.world.x, -in.world.y);

    let s00 = sample_layer(tile(base), map_cm);
    let s10 = sample_layer(tile(base + vec2<i32>(1, 0)), map_cm);
    let s01 = sample_layer(tile(base + vec2<i32>(0, 1)), map_cm);
    let s11 = sample_layer(tile(base + vec2<i32>(1, 1)), map_cm);
    let w00 = (1.0 - f.x) * (1.0 - f.y) * s00.a;
    let w10 = f.x * (1.0 - f.y) * s10.a;
    let w01 = (1.0 - f.x) * f.y * s01.a;
    let w11 = f.x * f.y * s11.a;
    let total = w00 + w10 + w01 + w11;
    var colour = vec3<f32>(0.5);
    if total > 1e-4 {
        colour = (s00.rgb * w00 + s10.rgb * w10 + s01.rgb * w01 + s11.rgb * w11) / total;
    }

    let n = normalize(in.normal);
    let light = frame.ambient.rgb + frame.diffuse.rgb * max(dot(n, frame.light.xyz), 0.0);
    let shadow = textureSample(shadow_texture, shadow_sampler, local * area.place.w).rgb;
    let lit = colour * min(light, vec3<f32>(1.0)) * shadow;
    return vec4<f32>(fogged(lit, in.world), 1.0);
}

struct WaterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
};

@vertex
fn vs_water(@location(0) position: vec3<f32>) -> WaterOut {
    var out: WaterOut;
    out.clip = frame.view_proj * vec4<f32>(position, 1.0);
    out.world = position;
    return out;
}

@fragment
fn fs_water(in: WaterOut) -> @location(0) vec4<f32> {
    let colour = vec3<f32>(0.18, 0.32, 0.45) * min(frame.ambient.rgb + frame.diffuse.rgb, vec3<f32>(1.0));
    return vec4<f32>(fogged(colour, in.world), 0.6);
}
