// Dünya'nın arazisi: beş doku, köşe renginin ağırlıklarıyla karışık.
//
// Motorun özel yüzeyi (`SurfaceMaterial`): bu fonksiyon yüzeyin ne olduğunu söylüyor, ışığı,
// gölgeyi, sisi motor veriyor — masaüstünde G-tamponuna, telefonda ileri gölgelendiriciye.
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

fn decode(c: vec3<f32>) -> vec3<f32> {
    return pow(c, vec3<f32>(2.2));
}

// İki ölçekte: yakında ayrıntı, uzakta tekrarın deseni kırık.
fn two_scales(t: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
    let near = textureSample(t, s_tile, uv).rgb;
    let far = textureSample(t, s_tile, uv * 0.23 + vec2<f32>(0.37, 0.71)).rgb;
    return mix(near, far, 0.35);
}

fn material_surface(input: SurfaceInput) -> Surface {
    let p = input.world_position;
    let n = normalize(input.N);
    let uv = p.xz / TILE;

    let grass = two_scales(t_grass, uv);
    let dirt = decode(two_scales(t_dirt, uv));
    let snow = decode(two_scales(t_snow, uv));
    let sand = decode(two_scales(t_sand, uv));
    // Kaya: üç düzlemden, normale göre.
    var k = pow(abs(n), vec3<f32>(4.0));
    k = k / (k.x + k.y + k.z);
    let rock = textureSample(t_rock, s_tile, p.zy / TILE).rgb * k.x
        + textureSample(t_rock, s_tile, uv).rgb * k.y
        + textureSample(t_rock, s_tile, p.xy / TILE).rgb * k.z;

    // Çimin geniş lekeleri: dokunun kendisinden, çok büyük ölçekte okunan parlaklık.
    let blotch = dot(textureSample(t_dirt, s_tile, uv * 0.031).rgb, vec3<f32>(0.333));
    let lush = grass * mix(0.75, 1.2, smoothstep(0.25, 0.65, blotch));

    let w = clamp(input.vertex_color, vec3<f32>(0.0), vec3<f32>(1.0));
    var w_grass = max(1.0 - w.r - w.g - w.b, 0.0);
    var w_dirt = w.r;
    // Kıyı: suyun hemen üstü kum, çimin ve toprağın yerine.
    let shore = 1.0 - smoothstep(WATER + 0.3, WATER + 1.4, p.y);
    let w_sand = (w_grass + w_dirt) * shore;
    w_grass = w_grass * (1.0 - shore);
    w_dirt = w_dirt * (1.0 - shore);

    let colour = lush * w_grass + dirt * w_dirt + rock * w.g + snow * w.b + sand * w_sand;
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
