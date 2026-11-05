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

/// Whether the image has to be sRGB-encoded here, set when the pipeline is built from the surface's
/// format. A browser's canvas offers no `*-srgb` format, so on the web the surface is `bgra8unorm`
/// and the hardware encodes nothing: the linear colour went to the screen as it was, and every
/// game in the browser was darker than on the desktop. Measured on `pinbol`'s table (`#1B4F72`):
/// (1, 22, 63) in headless Chrome before, (41, 89, 125) with this on — and (40, 89, 125) in the
/// Android APK, whose Vulkan surface is sRGB and leaves this off.
override ENCODE_SRGB: bool = false;

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

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
    // An sRGB surface encodes on write; any other gets it done here.
    if (ENCODE_SRGB) {
        return vec4<f32>(linear_to_srgb(mapped), 1.0);
    }
    return vec4<f32>(mapped, 1.0);
}
