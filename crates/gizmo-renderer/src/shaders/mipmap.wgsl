// One mip level from the one above it: a full-screen triangle over the level being written, each
// texel sampling the level above bilinearly at the centre of its 2×2 block — the block's mean.

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    // Generate a fullscreen triangle without vertex buffers
    let x = f32((vertex_index & 1u) << 2u);
    let y = f32((vertex_index & 2u) << 1u);
    return vec4<f32>(x - 1.0, 1.0 - y, 0.0, 1.0);
}

@group(0) @binding(0) var img: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // `pos` is a pixel of the level being WRITTEN; `img` is the level above, twice its size along
    // each axis that is not already 1 (wgpu's level size is `max(1, size >> level)`). So the
    // normalised coordinate is `pos` over the written level's size — over the source's size, as
    // this used to divide, it only reaches 0.5 and every level is the top-left quarter of the
    // one above, enlarged. Texel `i` covers source texels `2i` and `2i + 1`, whose shared corner
    // `(i + 0.5) / dst` is what the linear sampler averages around.
    let dst = max(textureDimensions(img) >> vec2<u32>(1u), vec2<u32>(1u));
    return textureSampleLevel(img, samp, pos.xy / vec2<f32>(dst), 0.0);
}
