// Textures a material's group is filled with that are made of others (`material_pack.rs`): drawn
// with a full-screen triangle over the level being written, each texel sampling its sources at its
// centre, so sources of any size land on the target's.

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vertex_index & 1u) << 2u);
    let y = f32((vertex_index & 2u) << 1u);
    return vec4<f32>(x - 1.0, 1.0 - y, 0.0, 1.0);
}

@group(0) @binding(0) var first: texture_2d<f32>;
@group(0) @binding(1) var second: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;
// xy: the size of the level being written.
@group(0) @binding(3) var<uniform> target_size: vec4<f32>;

fn at(pos: vec4<f32>) -> vec2<f32> {
    return pos.xy / target_size.xy;
}

// Occlusion, roughness, metalness: glTF's ORM packing. `first` is the occlusion map (red), `second`
// the metallic-roughness map (green and blue); either is white where a material has none.
@fragment
fn fs_orm(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = at(pos);
    let ao = textureSampleLevel(first, samp, uv, 0.0).r;
    let mr = textureSampleLevel(second, samp, uv, 0.0);
    return vec4<f32>(ao, mr.g, mr.b, 1.0);
}

// `first` as it is, at the target's size: one layer of a surface's own textures.
@fragment
fn fs_copy(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return textureSampleLevel(first, samp, at(pos), 0.0);
}
