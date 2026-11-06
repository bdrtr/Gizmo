// One step of the mip chain under glass (`transmission.rs`): each level is the one above it,
// averaged by four bilinear taps a texel apart. Wider than a plain 2x2 box, so a rough pane reads
// a soft picture rather than blocks.

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOut {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOut;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(t_source));
    let a = textureSample(t_source, s_source, in.uv + texel * vec2<f32>(-1.0, -1.0));
    let b = textureSample(t_source, s_source, in.uv + texel * vec2<f32>(1.0, -1.0));
    let c = textureSample(t_source, s_source, in.uv + texel * vec2<f32>(-1.0, 1.0));
    let d = textureSample(t_source, s_source, in.uv + texel * vec2<f32>(1.0, 1.0));
    return (a + b + c + d) * 0.25;
}
