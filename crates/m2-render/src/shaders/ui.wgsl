// The interface: textured, coloured quads placed in pixels, blended over the
// world. Colours stay in gamma space like the rest of the renderer.

struct Screen {
    // xy: the target's size in pixels.
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var ui_texture: texture_2d<f32>;
@group(1) @binding(1) var ui_sampler: sampler;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) colour: vec4<f32>,
    // The quad's texture coordinates as a rectangle: left, top, right, bottom.
    @location(3) bounds: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) colour: vec4<f32>,
    @location(2) bounds: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let ndc = vec2<f32>(
        in.position.x / screen.size.x * 2.0 - 1.0,
        1.0 - in.position.y / screen.size.y * 2.0,
    );
    out.clip = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = in.uv;
    out.colour = in.colour;
    out.bounds = in.bounds;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Sample no nearer to the quad's edges than half a texel: a smoothed
    // part of an image then never blends in its neighbours in the file.
    let half = 0.5 / vec2<f32>(textureDimensions(ui_texture));
    let low = in.bounds.xy + half;
    let high = max(in.bounds.zw - half, low);
    // At a scale like 2.5 some pixel centres fall exactly between two
    // texels; a hair's shift makes nearest sampling pick the same one all
    // along a row, not one or the other as rounding goes.
    let uv = in.uv + half * 0.002;
    return textureSample(ui_texture, ui_sampler, clamp(uv, low, high)) * in.colour;
}
