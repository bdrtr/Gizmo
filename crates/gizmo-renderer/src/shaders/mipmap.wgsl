// WGSL Shader for Mipmap Generation (Blit/Downsample)

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
    let size = vec2<f32>(textureDimensions(img));
    // Lineer sampler kullanıyoruz, ortalayıp okumak the en iyi blit'i sağlar
    return textureSample(img, samp, pos.xy / size);
}
