// ============================================================
// Yelbegen Engine — Post-Processing Shader
// Bloom (Bright Extract + Gaussian Blur) ve ACES Tone Mapping
// ============================================================

// Fullscreen Quad Vertex Shader (Tüm post-processing geçişleri için ortak)
struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FullscreenVertexOutput {
    // 3 vertex ile tam ekran üçgen çizmek (Quad'dan daha verimli)
    // vertex 0: (-1, -1), vertex 1: (3, -1), vertex 2: (-1, 3)
    var out: FullscreenVertexOutput;
    let x = f32(i32(vertex_index) / 2) * 4.0 - 1.0;
    let y = f32(i32(vertex_index) % 2) * 4.0 - 1.0;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    // UV koordinatları (0,0) sol-üst, (1,1) sağ-alt
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// ============================
// Bind Group: Kaynak Texture
// ============================
@group(0) @binding(0)
var t_source: texture_2d<f32>;
@group(0) @binding(1)
var s_source: sampler;

// ============================
// Pass 1: Bright Extract
// ============================
// Parlaklık eşiğini aşan pikselleri ayıklayıp geri kalanları siyaha çeker
@fragment
fn fs_bright_extract(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(t_source, s_source, in.uv);
    
    // İnsan gözünün parlaklık algısına göre ağırlıklı luminance
    let luminance = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    
    // Eşik: 1.0 üzerindeki pikseller bloom'a dahil olacak
    // Yumuşak geçiş (soft knee) için clamp kullanıyoruz
    let threshold = 1.0;
    let soft_threshold = 0.7;
    let knee = max(luminance - soft_threshold, 0.0) / (threshold - soft_threshold + 0.0001);
    let contribution = clamp(knee * knee, 0.0, 1.0);
    
    return vec4<f32>(color.rgb * contribution, 1.0);
}

// ============================
// Pass 2: Gaussian Blur
// ============================
// Bloom eşiğinden geçen texture'ı 9-tap Gaussian ile yumuşatıyoruz
// Yatay ve dikey iki ayrı geçiş yapılarak separable blur uygulanır

struct BlurParams {
    direction: vec2<f32>, // (1/w, 0) yatay veya (0, 1/h) dikey
    _padding: vec2<f32>,
};

@group(1) @binding(0)
var<uniform> blur_params: BlurParams;

@fragment
fn fs_blur(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    var result = textureSample(t_source, s_source, in.uv) * 0.227027;
    
    // Unrolled 9-tap Gaussian (naga constant-index kısıtlaması nedeniyle)
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
// HDR sahne rengini ve bloom'u birleştirip ACES Filmic Tone Mapping uygular

@group(1) @binding(0)
var t_bloom: texture_2d<f32>;
@group(1) @binding(1)
var s_bloom: sampler;

// ACES Filmic Tone Mapping (Hollywood Sinema Standardı)
// x girdisi HDR renk değeri, çıktı [0, 1] aralığında LDR renk
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
    let hdr_color = textureSample(t_source, s_source, in.uv).rgb;
    let bloom_color = textureSample(t_bloom, s_bloom, in.uv).rgb;
    
    // Bloom yoğunluğu (intensity)
    let bloom_intensity = 0.3;
    let combined = hdr_color + bloom_color * bloom_intensity;
    
    // ACES Tone Mapping
    let mapped = aces_tonemap(combined);
    
    // Gamma düzeltmesi (Linear → sRGB)
    let gamma = vec3<f32>(1.0 / 2.2);
    let final_color = pow(mapped, gamma);
    
    return vec4<f32>(final_color, 1.0);
}
