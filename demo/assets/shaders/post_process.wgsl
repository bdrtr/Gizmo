// ============================================================
// Yelbegen Engine — Post-Processing Shader
// Bloom (Bright Extract + Gaussian Blur) ve ACES Tone Mapping + Sinematikler
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

struct PostProcessParams {
    bloom_intensity: f32,
    bloom_threshold: f32,
    exposure: f32,
    chromatic_aberration: f32,
    vignette_intensity: f32,
    _padding0: f32,
    _padding1: f32,
    _padding2: f32,
};

@group(2) @binding(0)
var<uniform> params: PostProcessParams;

// ============================
// Pass 1: Bright Extract
// ============================
@fragment
fn fs_bright_extract(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_source, s_source, in.uv);
    let luminance = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    
    let threshold = params.bloom_threshold;
    let soft_threshold = threshold * 0.7; // Yumuşatılmış alt sınır
    let knee = max(luminance - soft_threshold, 0.0) / (threshold - soft_threshold + 0.0001);
    let contribution = clamp(knee * knee, 0.0, 1.0);
    
    return vec4<f32>(color.rgb * contribution, 1.0);
}

// ============================
// Pass 2: Gaussian Blur
// ============================
struct BlurParams {
    direction: vec2<f32>,
    _padding: vec2<f32>,
};

@group(1) @binding(0) var<uniform> blur_params: BlurParams;

@fragment
fn fs_blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var result = textureSample(t_source, s_source, in.uv) * 0.227027;
    
    let o1 = blur_params.direction * 1.0;
    result += textureSample(t_source, s_source, in.uv + o1) * 0.1945946;
    result += textureSample(t_source, s_source, in.uv - o1) * 0.1945946;
    
    let o2 = blur_params.direction * 2.0;
    result += textureSample(t_source, s_source, in.uv + o2) * 0.1216216;
    result += textureSample(t_source, s_source, in.uv - o2) * 0.1216216;
    
    let o3 = blur_params.direction * 3.0;
    result += textureSample(t_source, s_source, in.uv + o3) * 0.054054;
    result += textureSample(t_source, s_source, in.uv - o3) * 0.054054;
    
    let o4 = blur_params.direction * 4.0;
    result += textureSample(t_source, s_source, in.uv + o4) * 0.016216;
    result += textureSample(t_source, s_source, in.uv - o4) * 0.016216;
    
    return vec4<f32>(result.rgb, 1.0);
}

// ============================
// Pass 3: Composite + Tone Mapping
// ============================
@group(1) @binding(0) var t_bloom: texture_2d<f32>;
@group(1) @binding(1) var s_bloom: sampler;

fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_composite(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    // 1. Chromatic Aberration
    let center_dist = distance(in.uv, vec2<f32>(0.5, 0.5));
    let ca_offset = params.chromatic_aberration * center_dist * 0.05;
    
    let r = textureSample(t_source, s_source, in.uv + vec2<f32>(ca_offset, 0.0)).r;
    let g = textureSample(t_source, s_source, in.uv).g;
    let b = textureSample(t_source, s_source, in.uv - vec2<f32>(ca_offset, 0.0)).b;
    let hdr_color = vec3<f32>(r, g, b);

    // 2. Bloom Addition
    let bloom_color = textureSample(t_bloom, s_bloom, in.uv).rgb;
    let combined = (hdr_color + bloom_color * params.bloom_intensity) * params.exposure;
    
    // 3. ACES Tone Mapping
    let mapped = aces_tonemap(combined);
    
    // 4. Gamma Correction
    let gamma = vec3<f32>(1.0 / 2.2);
    var final_color = pow(mapped, gamma);
    
    // 5. Vignette
    let vignette = smoothstep(1.5, 0.3, center_dist * (1.0 + params.vignette_intensity));
    final_color *= vignette;
    
    return vec4<f32>(final_color, 1.0);
}
