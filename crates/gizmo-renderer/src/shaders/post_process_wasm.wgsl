// ============================================================
// Yelbegen Engine — Post-Processing Shader (FAST FOR WASM)
// ============================================================

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertexOutput {
    var out: FullscreenVertexOutput;
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@group(0) @binding(0) var t_source: texture_2d<f32>;
@group(0) @binding(1) var s_source: sampler;

@fragment
fn fs_bright_extract(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

@fragment
fn fs_blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

@group(2) @binding(0) var t_bloom: texture_2d<f32>;
@group(2) @binding(1) var s_bloom: sampler;
@group(2) @binding(2) var t_depth: texture_depth_2d;

@fragment
fn fs_composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let hdr_color = textureSample(t_source, s_source, in.uv).rgb;
    // VERY FAST ACES Tone Mapping + Gamma (NO DoF, NO Chromatic Aberration, NO Film Grain)
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    let mapped = clamp((hdr_color * (a * hdr_color + b)) / (hdr_color * (c * hdr_color + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
    let gamma = vec3<f32>(1.0 / 2.2);
    let final_color = pow(mapped, gamma);
    return vec4<f32>(final_color, 1.0);
}
