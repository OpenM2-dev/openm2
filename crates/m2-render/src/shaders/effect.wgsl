// Effects: particles and meshes as world-space triangles, a texture combined
// with each vertex's colour by Direct3D's texture operation numbers, blended
// by the pipeline. Depth is tested, not written. Gamma space, like the rest.

struct Frame {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    ambient: vec4<f32>,
    diffuse: vec4<f32>,
    // x: fog start, y: fog end (no fog when y <= x), in centimetres.
    fog: vec4<f32>,
    fog_colour: vec4<f32>,
};

@group(0) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(0) var colour_texture: texture_2d<f32>;
@group(1) @binding(1) var colour_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) operation: u32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) @interpolate(flat) operation: u32,
};

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = frame.view_proj * vec4<f32>(v.position, 1.0);
    out.world = v.position;
    out.uv = v.uv;
    out.colour = v.colour;
    out.operation = v.operation;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = textureSample(colour_texture, colour_sampler, in.uv);
    var rgb: vec3<f32>;
    switch in.operation {
        // Select the texture.
        case 2u: { rgb = texel.rgb; }
        // Select the colour.
        case 3u: { rgb = in.colour.rgb; }
        // Modulate, twice, four times.
        case 5u: { rgb = texel.rgb * in.colour.rgb * 2.0; }
        case 6u: { rgb = texel.rgb * in.colour.rgb * 4.0; }
        // Add, add signed.
        case 7u: { rgb = texel.rgb + in.colour.rgb; }
        case 8u: { rgb = texel.rgb + in.colour.rgb - vec3<f32>(0.5); }
        // Modulate (4) and anything else.
        default: { rgb = texel.rgb * in.colour.rgb; }
    }
    var alpha = texel.a * in.colour.a;
    // Effects fade out in the fog instead of taking its colour, so glowing
    // ones do not light it up.
    if frame.fog.y > frame.fog.x {
        let distance = length(in.world - frame.eye.xyz);
        let amount = clamp((distance - frame.fog.x) / (frame.fog.y - frame.fog.x), 0.0, 1.0);
        rgb = rgb * (1.0 - amount);
        alpha = alpha * (1.0 - amount);
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), clamp(alpha, 0.0, 1.0));
}
