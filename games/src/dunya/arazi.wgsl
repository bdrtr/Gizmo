// Dünya'nın arazisi: beş doku, köşe renginin ağırlıklarıyla karışık.
//
// Motorun özel yüzeyi (`SurfaceMaterial`), katı kaydedilmiş (`add_solid_surface_material`): hiç
// kesmiyor, gölgesini ve derinliğini motorun kendi boru hatları çiziyor — bu fonksiyon gölge
// katmanlarında çalışmıyor. Işığı, gölgeyi, sisi motor veriyor.
//
// Materyal grubunun (1) yuvaları burada başka anlamda — motorun `MaterialDesc`'i doldurduğu gibi:
//   taban renk  (sRGB)    çim
//   ışıyan      (sRGB)    kaya
//   normal      (doğrusal) toprak   — sRGB kodlu bir fotoğraf, burada çözülüyor
//   metal-pürüz (doğrusal) kar      — aynı
//   örtüşme     (doğrusal) kum      — aynı
//
// Köşe rengi ağırlık: kırmızı toprak, yeşil kaya, mavi kar; kalanı çim. Kum, su seviyesinin hemen
// üstünde, yükseklikten. Doku dünyada dört metrede bir tekrar ediyor; tekrar belli olmasın diye her
// doku iki ölçekte okunup karışıyor ve çim geniş, yavaş bir lekelenmeyle açılıp koyulaşıyor. Kaya
// dik yamaçta yandan da okunuyor (üç düzlemli), yoksa uçurumda dikey çizgilere uzardı.
//
// **Yalnız görünen katman okunuyor.** Bir pikselde çoğu zaman bir iki katman var; ağırlığı sıfır olan
// okunmuyor. Dalın içinde okumak için türevler önceden, dalsız kodda alınıyor (`textureSampleGrad`).
// Her şey her pikselde okunurken arazi karenin GPU süresinin üçte birine yakındı (2026-11-16).

#import gizmo::surface::{Surface, SurfaceInput}

@group(1) @binding(0) var t_grass: texture_2d<f32>;
@group(1) @binding(1) var s_tile: sampler;
@group(1) @binding(2) var t_dirt: texture_2d<f32>;
@group(1) @binding(3) var t_snow: texture_2d<f32>;
@group(1) @binding(4) var t_rock: texture_2d<f32>;
@group(1) @binding(5) var t_sand: texture_2d<f32>;

// Su seviyesi: oyunun `su::WATER`'ı.
const WATER: f32 = 3.5;
// Bir dokunun dünyada kapladığı, metre.
const TILE: f32 = 4.0;
// Bundan hafif bir katman okunmuyor.
const SEEN: f32 = 0.004;
// Uzak ölçek ve kayması.
const FAR: f32 = 0.23;
const FAR_SHIFT: vec2<f32> = vec2<f32>(0.37, 0.71);

fn decode(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}

// İki ölçekte: yakında ayrıntı, uzakta tekrarın deseni kırık. `dx`, `dy`: `uv`'nin ekran türevleri.
fn two_scales(t: texture_2d<f32>, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec3<f32> {
    let near = textureSampleGrad(t, s_tile, uv, dx, dy).rgb;
    let far = textureSampleGrad(t, s_tile, uv * FAR + FAR_SHIFT, dx * FAR, dy * FAR).rgb;
    return mix(near, far, 0.35);
}

fn material_surface(input: SurfaceInput) -> Surface {
    let p = input.world_position;
    let n = normalize(input.N);
    let uv = p.xz / TILE;
    // Türevler dalsız kodda: dallarda okunanlar bunlarla.
    let dx = dpdx(uv);
    let dy = dpdy(uv);
    let side_x = p.zy / TILE;
    let side_z = p.xy / TILE;
    let sx_dx = dpdx(side_x);
    let sx_dy = dpdy(side_x);
    let sz_dx = dpdx(side_z);
    let sz_dy = dpdy(side_z);

    // Ağırlıklar önce.
    let w = clamp(input.vertex_color, vec3<f32>(0.0), vec3<f32>(1.0));
    var w_grass = max(1.0 - w.r - w.g - w.b, 0.0);
    var w_dirt = w.r;
    // Kıyı: suyun hemen üstü kum, çimin ve toprağın yerine.
    let shore = 1.0 - smoothstep(WATER + 0.3, WATER + 1.4, p.y);
    let w_sand = (w_grass + w_dirt) * shore;
    w_grass = w_grass * (1.0 - shore);
    w_dirt = w_dirt * (1.0 - shore);

    var colour = vec3<f32>(0.0);
    if (w_grass > SEEN) {
        let grass = two_scales(t_grass, uv, dx, dy);
        // Çimin geniş lekeleri: toprak dokusundan, çok büyük ölçekte okunan parlaklık.
        let blotch = dot(textureSampleGrad(t_dirt, s_tile, uv * 0.031, dx * 0.031, dy * 0.031).rgb, vec3<f32>(0.333));
        colour += grass * mix(0.75, 1.2, smoothstep(0.25, 0.65, blotch)) * w_grass;
    }
    if (w_dirt > SEEN) {
        colour += decode(two_scales(t_dirt, uv, dx, dy)) * w_dirt;
    }
    if (w.g > SEEN) {
        // Kaya: üç düzlemden, normale göre; hafif kalan düzlem okunmuyor.
        var k = pow(abs(n), vec3<f32>(4.0));
        k = k / (k.x + k.y + k.z);
        var rock = vec3<f32>(0.0);
        if (k.x > 0.02) {
            rock += textureSampleGrad(t_rock, s_tile, side_x, sx_dx, sx_dy).rgb * k.x;
        }
        if (k.y > 0.02) {
            rock += textureSampleGrad(t_rock, s_tile, uv, dx, dy).rgb * k.y;
        }
        if (k.z > 0.02) {
            rock += textureSampleGrad(t_rock, s_tile, side_z, sz_dx, sz_dy).rgb * k.z;
        }
        colour += rock * w.g;
    }
    if (w.b > SEEN) {
        colour += decode(two_scales(t_snow, uv, dx, dy)) * w.b;
    }
    if (w_sand > SEEN) {
        colour += decode(two_scales(t_sand, uv, dx, dy)) * w_sand;
    }
    let total = w_grass + w_dirt + w.g + w.b + w_sand;
    let albedo = colour / max(total, 0.001);

    var s: Surface;
    s.base_color = albedo;
    s.albedo = albedo;
    s.alpha = 1.0;
    s.alpha_cutoff = 0.0;
    s.N = n;
    s.roughness = (0.95 * (w_grass + w_dirt + w_sand) + 0.8 * w.g + 0.55 * w.b) / max(total, 0.001);
    s.metallic = 0.0;
    s.emissive = vec3<f32>(0.0);
    return s;
}
