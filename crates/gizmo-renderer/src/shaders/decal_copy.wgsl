// A decal's texture copied into one level of one layer of the forward-only path's decal array
// (`decal::ForwardDecals`): a triangle over the whole level, the source read at the same uv, so the
// layer reads as the texture did — uv (0, 0) at the top left. `textureSample` picks the source's
// mip by the level's own size: each level is the source filtered down to it, not a copy of the
// level above.

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

struct Copied {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Copied {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Copied;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: Copied) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, in.uv);
}
