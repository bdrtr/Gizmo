// ============================================================
// Yelbegen Engine — Post-Processing Shader
// Bloom (Bright Extract + Gaussian Blur) ve ACES Tone Mapping + Sinematikler


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
    film_grain_intensity: f32,
    dof_focus_dist: f32,
    dof_focus_range: f32,
    dof_blur_size: f32,
    cam_near: f32,
    cam_far: f32,
    underwater: f32,       // 0 = havada, 1 = kamera su altında
    fog: vec4<f32>,        // rgb = su-altı sis rengi, a = yoğunluk (offset 48, 16-hizalı)
};

@group(2) @binding(0)
var<uniform> params: PostProcessParams;

// Düzgün 2B hash (Dave Hoskins). Eski film-grain `fract(sin(dot(uv,K))*M)` düz UV'de
// sin'in ekstremumlarında donup STATİK DİAGONAL BANTLAR üretiyordu; bu, piksel
// koordinatından düzgün beyaz-gürültü verir (yapısal bant yok).
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

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
@group(1) @binding(2) var t_depth: texture_depth_2d;

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

    // 2. Depth of Field (DoF)
    let depth_dims = textureDimensions(t_depth, 0);
    let depth_uv = vec2<i32>(i32(in.uv.x * f32(depth_dims.x)), i32(in.uv.y * f32(depth_dims.y)));
    let depth_val = textureLoad(t_depth, depth_uv, 0);
    // Linearize depth. wgpu/glam perspective_rh writes NDC depth in [0,1] (NOT the
    // OpenGL [-1,1] range), so the [0,1] reconstruction must be used:
    //   view_dist = n*f / (f - d*(f - n))   →  d=0 → n, d=1 → f.
    // (The old (2n)/(f+n-d(f-n)) was the OpenGL [-1,1] formula and was wrong here.)
    let n = params.cam_near;
    let f = params.cam_far;
    let view_dist = (n * f) / (f - depth_val * (f - n));
    
    let coc = clamp(abs(view_dist - params.dof_focus_dist) / params.dof_focus_range, 0.0, 1.0);
    
    var dof_color = hdr_color;
    if (coc > 0.01 && params.dof_blur_size > 0.0) {
        var blurred = vec3<f32>(0.0);
        var total_weight = 0.0;
        let radius = coc * params.dof_blur_size;
        
        // Simple Poisson-like disk samples
        
        let aspect = vec2<f32>(1.0, 1.0); // Aspect correction might be needed for perfect circles
        let step_size = vec2<f32>(1.0 / 1920.0, 1.0 / 1080.0) * radius;
        
        // Unrolled Poisson disk
        blurred += textureSampleLevel(t_source, s_source, in.uv + vec2<f32>( 0.000,  1.000) * step_size, 0.0).rgb;
        blurred += textureSampleLevel(t_source, s_source, in.uv + vec2<f32>( 0.866,  0.500) * step_size, 0.0).rgb;
        blurred += textureSampleLevel(t_source, s_source, in.uv + vec2<f32>( 0.866, -0.500) * step_size, 0.0).rgb;
        blurred += textureSampleLevel(t_source, s_source, in.uv + vec2<f32>( 0.000, -1.000) * step_size, 0.0).rgb;
        blurred += textureSampleLevel(t_source, s_source, in.uv + vec2<f32>(-0.866, -0.500) * step_size, 0.0).rgb;
        blurred += textureSampleLevel(t_source, s_source, in.uv + vec2<f32>(-0.866,  0.500) * step_size, 0.0).rgb;
        blurred += textureSampleLevel(t_source, s_source, in.uv + vec2<f32>( 0.433,  0.750) * step_size, 0.0).rgb;
        blurred += textureSampleLevel(t_source, s_source, in.uv + vec2<f32>(-0.433, -0.750) * step_size, 0.0).rgb;
        
        total_weight = 8.0;
        blurred = blurred / total_weight;
        dof_color = mix(hdr_color, blurred, coc);
    }

    // 3. Bloom Addition
    let bloom_color = textureSample(t_bloom, s_bloom, in.uv).rgb;
    let combined = (dof_color + bloom_color * params.bloom_intensity) * params.exposure;
    
    // 3. ACES Tone Mapping
    let mapped = aces_tonemap(combined);
    
    // Swapchain is configured as an sRGB format (Bgra8UnormSrgb/Rgba8UnormSrgb)
    // Hardware automatically applies gamma correction upon writing.
    var final_color = mapped;

    // ── Su-altı atmosferi: kamera bir su hacmindeyken derinlik-bazlı sis (Beer-Lambert) ──
    // view_dist (lineer sahne derinliği) kullanılır; uzak geometri sis rengine gömülür → mavi-yeşil
    // su hissi. underwater=0 iken tamamen atlanır (etkisiz).
    if (params.underwater > 0.5) {
        let fog_amount = clamp(1.0 - exp(-view_dist * params.fog.a), 0.0, 1.0);
        final_color = mix(final_color, params.fog.rgb, fog_amount);
    }
    
    // 5. Vignette
    let vignette = smoothstep(1.5, 0.3, center_dist * (1.0 + params.vignette_intensity));
    final_color *= vignette;
    
    // 6. Film Grain — piksel-koordinatı tabanlı düzgün hash (statik diagonal bant yok).
    let noise = hash12(in.position.xy) - 0.5;
    final_color += final_color * noise * params.film_grain_intensity;
    
    return vec4<f32>(final_color, 1.0);
}
