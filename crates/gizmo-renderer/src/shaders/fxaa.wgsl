// FXAA (Fast Approximate Anti-Aliasing) — Timothy Lottes FXAA 3.11
// Nvidia FXAA implementasyonu. Kenar tespiti luminance'a dayalıdır.
// Composite sonrası, son pass olarak çalışır.

// ─── VERTEX SHADER ───
// Tam ekran üçgen (fullscreen triangle) — vertex buffer gerektirmez
struct VertexOutput {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var out: VertexOutput;
    // Fullscreen triangle: 3 vertex → 2 triangle strip
    let x = f32(i32(idx & 1u)) * 4.0 - 1.0;
    let y = f32(i32(idx >> 1u)) * 4.0 - 1.0;
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// ─── BINDINGS ───
@group(0) @binding(0) var input_tex: texture_2d<f32>;
@group(0) @binding(1) var input_sampler: sampler;
@group(0) @binding(2) var<uniform> params: FxaaParams;

struct FxaaParams {
    inv_screen_size: vec2<f32>,  // 1.0 / vec2(width, height)
    fxaa_enabled: f32,           // 1.0 = açık, 0.0 = kapalı (bypass)
    _padding: f32,
};

// ─── YARDIMCI FONKSİYONLAR ───

// Algılanan parlaklık (luma) hesaplama — insan gözü yeşile daha duyarlıdır
fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

// ─── FXAA FRAGMENT SHADER ───
// FXAA 3.11 Quality — Sub-pixel aliasing ve kenar yumuşatma
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let inv = params.inv_screen_size;

    // Bypass modu
    if (params.fxaa_enabled < 0.5) {
        return textureSample(input_tex, input_sampler, uv);
    }

    // ─── KENAR TESPİTİ (Edge Detection) ───
    // Merkez ve 4 komşu pikselin luma'sını al
    let rgbM  = textureSample(input_tex, input_sampler, uv).rgb;
    let rgbN  = textureSample(input_tex, input_sampler, uv + vec2<f32>( 0.0, -inv.y)).rgb;
    let rgbS  = textureSample(input_tex, input_sampler, uv + vec2<f32>( 0.0,  inv.y)).rgb;
    let rgbW  = textureSample(input_tex, input_sampler, uv + vec2<f32>(-inv.x,  0.0)).rgb;
    let rgbE  = textureSample(input_tex, input_sampler, uv + vec2<f32>( inv.x,  0.0)).rgb;

    let lumaM = luma(rgbM);
    let lumaN = luma(rgbN);
    let lumaS = luma(rgbS);
    let lumaW = luma(rgbW);
    let lumaE = luma(rgbE);

    let lumaMin = min(lumaM, min(min(lumaN, lumaS), min(lumaW, lumaE)));
    let lumaMax = max(lumaM, max(max(lumaN, lumaS), max(lumaW, lumaE)));
    let lumaRange = lumaMax - lumaMin;

    // Kontrast eşiği — düşük kontrastlı alanları atla (performans)
    let FXAA_EDGE_THRESHOLD: f32 = 0.125;
    let FXAA_EDGE_THRESHOLD_MIN: f32 = 0.0625;

    if (lumaRange < max(FXAA_EDGE_THRESHOLD_MIN, lumaMax * FXAA_EDGE_THRESHOLD)) {
        return vec4<f32>(rgbM, 1.0);
    }

    // ─── KENAR YÖNÜ TESPİTİ (Edge Direction) ───
    // Köşe luma'ları
    let rgbNW = textureSample(input_tex, input_sampler, uv + vec2<f32>(-inv.x, -inv.y)).rgb;
    let rgbNE = textureSample(input_tex, input_sampler, uv + vec2<f32>( inv.x, -inv.y)).rgb;
    let rgbSW = textureSample(input_tex, input_sampler, uv + vec2<f32>(-inv.x,  inv.y)).rgb;
    let rgbSE = textureSample(input_tex, input_sampler, uv + vec2<f32>( inv.x,  inv.y)).rgb;

    let lumaNW = luma(rgbNW);
    let lumaNE = luma(rgbNE);
    let lumaSW = luma(rgbSW);
    let lumaSE = luma(rgbSE);

    // Sobel benzeri gradyan hesaplaması
    let edgeH = abs(-2.0 * lumaW + lumaNW + lumaSW)
              + abs(-2.0 * lumaM + lumaN  + lumaS ) * 2.0
              + abs(-2.0 * lumaE + lumaNE + lumaSE);

    let edgeV = abs(-2.0 * lumaN + lumaNW + lumaNE)
              + abs(-2.0 * lumaM + lumaW  + lumaE ) * 2.0
              + abs(-2.0 * lumaS + lumaSW + lumaSE);

    let isHorizontal = edgeH >= edgeV;

    // ─── ALT-PİKSEL KAYDIRMA (Sub-pixel Shift) ───
    // Kenarın hangi tarafına daha yakın olduğumuzu bul
    let luma1: f32 = select(lumaW, lumaN, isHorizontal);
    let luma2: f32 = select(lumaE, lumaS, isHorizontal);

    let grad1 = abs(luma1 - lumaM);
    let grad2 = abs(luma2 - lumaM);

    let step_length: f32 = select(inv.x, inv.y, isHorizontal);

    var luma_local_avg: f32;
    var correct_dir: f32;

    if (grad1 >= grad2) {
        correct_dir = -step_length;
        luma_local_avg = 0.5 * (luma1 + lumaM);
    } else {
        correct_dir = step_length;
        luma_local_avg = 0.5 * (luma2 + lumaM);
    }

    // UV'yi kenar boyunca kaydır
    var current_uv = uv;
    if (isHorizontal) {
        current_uv.y += correct_dir * 0.5;
    } else {
        current_uv.x += correct_dir * 0.5;
    }

    // ─── KENAR BOYUNCA ARAMA (Edge Walking) ───
    let step: vec2<f32> = select(
        vec2<f32>(0.0, inv.y),
        vec2<f32>(inv.x, 0.0),
        isHorizontal
    );

    var uv_neg = current_uv - step;
    var uv_pos = current_uv + step;

    let FXAA_SEARCH_STEPS: i32 = 6;
    let FXAA_SEARCH_THRESHOLD: f32 = 0.25;

    var luma_end_neg = luma(textureSample(input_tex, input_sampler, uv_neg).rgb) - luma_local_avg;
    var luma_end_pos = luma(textureSample(input_tex, input_sampler, uv_pos).rgb) - luma_local_avg;

    var reached_neg = abs(luma_end_neg) >= FXAA_SEARCH_THRESHOLD;
    var reached_pos = abs(luma_end_pos) >= FXAA_SEARCH_THRESHOLD;

    for (var i = 1; i < FXAA_SEARCH_STEPS; i = i + 1) {
        if (!reached_neg) {
            uv_neg -= step * 1.5;
            luma_end_neg = luma(textureSample(input_tex, input_sampler, uv_neg).rgb) - luma_local_avg;
            reached_neg = abs(luma_end_neg) >= FXAA_SEARCH_THRESHOLD;
        }
        if (!reached_pos) {
            uv_pos += step * 1.5;
            luma_end_pos = luma(textureSample(input_tex, input_sampler, uv_pos).rgb) - luma_local_avg;
            reached_pos = abs(luma_end_pos) >= FXAA_SEARCH_THRESHOLD;
        }
        if (reached_neg && reached_pos) { break; }
    }

    // ─── SON BLEND HESABI ───
    var dist_neg: f32;
    var dist_pos: f32;

    if (isHorizontal) {
        dist_neg = uv.x - uv_neg.x;
        dist_pos = uv_pos.x - uv.x;
    } else {
        dist_neg = uv.y - uv_neg.y;
        dist_pos = uv_pos.y - uv.y;
    }

    let total_dist = dist_neg + dist_pos;
    let pixel_offset = -min(dist_neg, dist_pos) / total_dist + 0.5;

    // Yanlış taraftaysa karıştırma yapma
    let is_luma_center_smaller = lumaM < luma_local_avg;
    let correct_variation_neg = (luma_end_neg < 0.0) != is_luma_center_smaller;
    let correct_variation_pos = (luma_end_pos < 0.0) != is_luma_center_smaller;

    var final_offset: f32;
    if (!correct_variation_neg && !correct_variation_pos) {
        final_offset = 0.0;
    } else {
        final_offset = pixel_offset;
    }

    // ─── ALT-PİKSEL YUMUŞATMA (Sub-pixel AA) ───
    let luma_avg = (1.0/12.0) * (2.0 * (lumaN + lumaS + lumaW + lumaE)
                    + lumaNW + lumaNE + lumaSW + lumaSE);
    let sub_pixel_offset1 = clamp(abs(luma_avg - lumaM) / lumaRange, 0.0, 1.0);
    let sub_pixel_offset2 = (-2.0 * sub_pixel_offset1 + 3.0) * sub_pixel_offset1 * sub_pixel_offset1;
    let sub_pixel_offset = sub_pixel_offset2 * sub_pixel_offset2 * 0.75;

    final_offset = max(final_offset, sub_pixel_offset);

    // Son UV'yi hesapla ve sample et
    var final_uv = uv;
    if (isHorizontal) {
        final_uv.y += final_offset * correct_dir;
    } else {
        final_uv.x += final_offset * correct_dir;
    }

    let final_color = textureSample(input_tex, input_sampler, final_uv).rgb;
    return vec4<f32>(final_color, 1.0);
}
