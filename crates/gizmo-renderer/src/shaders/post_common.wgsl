// What both post composites share, put in front of either as it is loaded
// (`post_process::post_shader_source`): the parameters and their binding, the tone curves, the
// vignette and the dither. `post_process.wgsl` is the desktop's composite and `post_process_wasm.wgsl`
// the reduced one of the browser and a phone; with these here, the two give a pixel the same colour.

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
    // Tone mapping: eğri seçimi ve beyaz noktası. Ayrı bir vec4, çünkü `fog`'dan sonra blok
    // 16-hizalı bir sınırda ve tek bir f32 eklemek onu bozardı.
    tonemap: vec4<f32>,    // x = eğri (0..5, `TonemapCurve`), y = beyaz nokta, z = deband dither (0/1), w = pad
    // z, w: depth of field, the lens factor (0 = focus band) and the farthest distance (0 = no limit).
    // Camera motion blur: x = shutter fraction (0 = off), y = longest smear in screen fractions.
    motion: vec4<f32>,
    // This frame's clip space → the last frame's (both unjittered).
    reproject: mat4x4<f32>,
    // x: 1 = bloom replaces light rather than adding it.
    // x: 1 = bloom replaces light rather than adding it. y: the colour look-up table's mode (0 none, 1 a
    // grade, 2 Reinhard, 3 log2), z and w: its log2 range (`crate::color_lut`).
    bloom_and_lut: vec4<f32>,
};

@group(2) @binding(0)
var<uniform> params: PostProcessParams;

// The composite's group 1: its linear sampler, and the colour look-up table (`crate::color_lut`), which
// is not read while none is set.
@group(1) @binding(1) var s_bloom: sampler;
@group(1) @binding(3) var t_lut: texture_3d<f32>;

// Düzgün 2B hash (Dave Hoskins). Eski film-grain `fract(sin(dot(uv,K))*M)` düz UV'de
// sin'in ekstremumlarında donup STATİK DİAGONAL BANTLAR üretiyordu; bu, piksel
// koordinatından düzgün beyaz-gürültü verir (yapısal bant yok).
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

// AgX's sigmoid, as a sixth-order polynomial on the log-encoded value (Wrensch's fit, mean error
// below 1e-3 against the reference look-up).
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

// AgX: inset into its working space, log2-encode over [-12.47, 4.03] EV, apply the sigmoid,
// outset, and return linear (the swapchain's sRGB encode follows).
fn agx_tonemap(x: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let outset = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var c = inset * max(x, vec3<f32>(1e-10));
    c = clamp(log2(c), vec3<f32>(min_ev), vec3<f32>(max_ev));
    c = (c - min_ev) / (max_ev - min_ev);
    c = agx_contrast(c);
    c = outset * c;
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// Khronos PBR Neutral: identity below the knee (0.76), a hyperbolic shoulder above it, and a
// little desaturation of what it compresses.
fn pbr_neutral_tonemap(x: vec3<f32>) -> vec3<f32> {
    let start = 0.8 - 0.04;
    let desaturation = 0.15;
    let lowest = min(x.r, min(x.g, x.b));
    let offset = select(0.04, lowest - 6.25 * lowest * lowest, lowest < 0.08);
    var c = x - offset;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) {
        return c;
    }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c = c * (new_peak / peak);
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

// The curve `curve` selects: see `TonemapCurve` on the Rust side, whose numbering this is. ACES is 3
// because it is the default and changing the default would restyle every scene built on the engine.
// The desktop passes `params.tonemap.x`; the reduced composite passes a pipeline constant, so a phone
// compiles one curve and pays for no branch.
fn tonemap_curve(x: vec3<f32>, curve: f32, white_point: f32) -> vec3<f32> {
    let white = max(white_point, 0.0001);

    if (curve < 0.5) {
        // None: clamp. Everything above white clips flat, which is what makes it useful for
        // checking whether a frame was ever really in HDR.
        return clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if (curve < 1.5) {
        // Reinhard: never clips, and pulls midtones down noticeably.
        return x / (1.0 + x);
    }
    if (curve < 2.5) {
        // Reinhard with a white point: reaches exactly 1.0 at `white`, so highlights keep their
        // separation instead of asymptoting together.
        return clamp((x * (1.0 + x / (white * white))) / (1.0 + x), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if (curve < 3.5) {
        return aces_tonemap(x);
    }
    if (curve < 4.5) {
        return clamp(agx_tonemap(x), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if (curve < 5.5) {
        return clamp(pbr_neutral_tonemap(x), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    // Reinhard on luminance. The hue holds, so a channel can pass 1 and clip where the others have not.
    let luminance = dot(x, vec3<f32>(0.2126, 0.7152, 0.0722));
    return clamp(x / (1.0 + luminance), vec3<f32>(0.0), vec3<f32>(1.0));
}

// How much of a pixel's light the vignette leaves at `uv`. `strength` is `PostProcess::vignette`: 0 is no
// vignette at all, as documented, and 1 is the strongest, the corners all but black. It was
// `smoothstep(1.5, 0.3, d * (1 + strength))`, which at 0 still took the corners down to 73 %.
fn vignette(uv: vec2<f32>, strength: f32) -> f32 {
    let falloff = smoothstep(1.5, 0.3, distance(uv, vec2<f32>(0.5, 0.5)) * 2.0);
    return mix(1.0, falloff, clamp(strength, 0.0, 1.0));
}

// Deband dither (`Renderer::deband_dither`): ±half an 8-bit step, added where the steps are — in the
// display encoding, which the target applies after this — so a dark gradient whose neighbouring pixels
// would round to the same code no longer shows as flat bands.
fn deband(color: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    let step_noise = hash12(pixel + vec2<f32>(37.0, 113.0)) - 0.5;
    let encoded = pow(max(color, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    return pow(max(encoded + vec3<f32>(step_noise / 255.0), vec3<f32>(0.0)), vec3<f32>(2.2));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// The table at `c`, each channel in 0..1, between texel centres as `ColorLut::sample` is.
fn lut_lookup(c: vec3<f32>) -> vec3<f32> {
    let size = f32(textureDimensions(t_lut).x);
    let uvw = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)) * ((size - 1.0) / size) + 0.5 / size;
    return textureSampleLevel(t_lut, s_bloom, uvw, 0.0).rgb;
}

// The scene's light, after exposure, as display light: the tone curve, then a grade if the table is one;
// or the table in the curve's place, fed the light as it was shaped for (`LutDomain`).
fn display_mapping(hdr: vec3<f32>, lut_mode: f32, min_ev: f32, max_ev: f32, curve: f32, white: f32) -> vec3<f32> {
    if (lut_mode > 2.5) {
        let ev = (log2(max(hdr, vec3<f32>(1e-10))) - min_ev) / max(max_ev - min_ev, 1e-4);
        return srgb_to_linear(lut_lookup(ev));
    }
    if (lut_mode > 1.5) {
        return lut_lookup(hdr / (1.0 + max(hdr, vec3<f32>(0.0))));
    }
    let mapped = tonemap_curve(hdr, curve, white);
    if (lut_mode > 0.5) {
        return srgb_to_linear(lut_lookup(linear_to_srgb(clamp(mapped, vec3<f32>(0.0), vec3<f32>(1.0)))));
    }
    return mapped;
}
