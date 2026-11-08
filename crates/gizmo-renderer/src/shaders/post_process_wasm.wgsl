// ============================================================
// Gizmo Engine — the reduced post composite, for the browser and a phone
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

// `PostProcessParams`, `params` (group 2), the tone curves, the vignette and the dither are in
// `post_common.wgsl`, which `post_process::post_shader_source` puts in front of this file, so this
// composite gives a pixel the colour the desktop's does. What it leaves out needs a pass or a texture
// this path does not have: bloom, depth of field, motion blur, chromatic aberration, the underwater fog,
// auto exposure.

/// `Renderer::tonemap_curve`, `Renderer::deband_dither`, and whether there is any vignette or film grain
/// at all, set when the pipeline is built and built again when they change
/// (`post_process::sync_composite`): constants, so a phone compiles one curve and nothing it is not using.
/// How strong a vignette or a grain is stays in `params`.
override TONEMAP_CURVE: f32 = 3.0;
override DEBAND_DITHER: bool = false;
override VIGNETTE: bool = false;
override FILM_GRAIN: bool = false;

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
    var color = tonemap_curve(hdr_color * params.exposure, TONEMAP_CURVE, params.tonemap.y);
    if (VIGNETTE) {
        color *= vignette(in.uv, params.vignette_intensity);
    }
    if (FILM_GRAIN) {
        let noise = hash12(in.position.xy) - 0.5;
        color += color * noise * params.film_grain_intensity;
    }
    if (DEBAND_DITHER) {
        color = deband(color, in.position.xy);
    }
    // An sRGB surface encodes on write; any other gets it done here.
    if (ENCODE_SRGB) {
        return vec4<f32>(linear_to_srgb(color), 1.0);
    }
    return vec4<f32>(color, 1.0);
}
