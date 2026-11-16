// Skinned, textured models: one directional light plus ambient, alpha test,
// and the shine of upgraded items: a shine map added to a shining texture,
// times its alpha (a gloss mask) and the part's shine, looked up by where
// the camera's view reflects off the surface and scrolling. Colours stay in
// gamma space, like the 2004 original (no sRGB conversion).

struct Frame {
    view_proj: mat4x4<f32>,
    // xyz: the camera position.
    eye: vec4<f32>,
    // xyz: direction towards the light.
    light: vec4<f32>,
    // rgb: ambient light.
    ambient: vec4<f32>,
    // rgb: diffuse light; w: alpha below this is discarded.
    diffuse: vec4<f32>,
    // x: fog start, y: fog end (no fog when y <= x), in centimetres.
    fog: vec4<f32>,
    // rgb: fog colour.
    fog_colour: vec4<f32>,
    // xyz: the picture's right and up, in the world.
    right: vec4<f32>,
    up: vec4<f32>,
    // x: how far the shine maps have scrolled, 0 to 1.
    shine: vec4<f32>,
};

// x: how much the part shines, 0 to 1.
struct PartLook {
    shine: vec4<f32>,
};

// x: 1 when the texture's alpha is coverage; y: 1 when it shines, its
// alpha saying how much.
struct TextureLook {
    look: vec4<f32>,
};

// Skinning matrices of one part; the last used entry is the identity.
struct Palette {
    joints: array<mat4x4<f32>, 256>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var<uniform> palette: Palette;
@group(1) @binding(1) var<uniform> look: PartLook;
@group(1) @binding(2) var shine_texture: texture_2d<f32>;
@group(1) @binding(3) var shine_sampler: sampler;
@group(2) @binding(0) var colour_texture: texture_2d<f32>;
@group(2) @binding(1) var colour_sampler: sampler;
@group(2) @binding(2) var<uniform> texture_look: TextureLook;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) joints: vec4<u32>,
    @location(4) weights: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    let skin = palette.joints[v.joints.x] * v.weights.x
        + palette.joints[v.joints.y] * v.weights.y
        + palette.joints[v.joints.z] * v.weights.z
        + palette.joints[v.joints.w] * v.weights.w;
    let world = skin * vec4<f32>(v.position, 1.0);
    var out: VertexOut;
    out.clip = frame.view_proj * world;
    out.world = world.xyz;
    out.normal = (skin * vec4<f32>(v.normal, 0.0)).xyz;
    out.uv = v.uv;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let colour = textureSample(colour_texture, colour_sampler, in.uv);
    let coverage = texture_look.look.x > 0.5;
    if coverage && colour.a < frame.diffuse.w {
        discard;
    }
    // Models are drawn from both sides: light the side facing the camera.
    // Normals longer than 4 are a tree crown's (`tree.rs`): they point out
    // of the crown and are lit as they are.
    let len = length(in.normal);
    var n = select(vec3<f32>(0.0, 0.0, 1.0), in.normal / len, len > 1e-6);
    if len < 4.0 && dot(n, frame.eye.xyz - in.world) < 0.0 {
        n = -n;
    }
    let light = frame.ambient.rgb + frame.diffuse.rgb * max(dot(n, frame.light.xyz), 0.0);
    var lit = colour.rgb * min(light, vec3<f32>(1.0));
    // The shine map where the camera's view reflects off the surface, along
    // the picture's right and up, scrolled: its texture coordinates, as
    // the client's camera-space reflection vector gives them.
    let to_eye = normalize(frame.eye.xyz - in.world);
    let reflected = 2.0 * dot(to_eye, n) * n - to_eye;
    let at = vec2<f32>(dot(reflected, frame.right.xyz), dot(reflected, frame.up.xyz))
        + vec2<f32>(frame.shine.x);
    let shine_colour = textureSampleLevel(shine_texture, shine_sampler, at, 0.0).rgb;
    let gloss = select(0.0, colour.a, texture_look.look.y > 0.5);
    lit = min(lit + shine_colour * gloss * look.shine.x, vec3<f32>(1.0));
    if frame.fog.y > frame.fog.x {
        let distance = length(in.world - frame.eye.xyz);
        let amount = clamp((distance - frame.fog.x) / (frame.fog.y - frame.fog.x), 0.0, 1.0);
        lit = mix(lit, frame.fog_colour.rgb, amount);
    }
    return vec4<f32>(lit, 1.0);
}
