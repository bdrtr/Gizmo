// Dünya'nın gölleri: bir su hacminin üstü.
//
// Motorun özel yüzeyi (`SurfaceMaterial`), ışık geçiren (`transmission`): motor suyun altındaki
// dünyayı kırarak gösteriyor, gökten ve güneşten yansımayı Fresnel'le ekliyor, gece karartıyor. Bu
// fonksiyon suyun ne olduğunu söylüyor:
//
// - **derinlik:** `absorption` metre başına soğurma; motor altını `exp(-soğurma · derinlik)` ile
//   karartıyor (derinlik, yüzeyle altındaki zemin arası: `behind_distance`). Kırmızı en çabuk
//   gidiyor: kıyıda zemin berrak görünüyor, açıldıkça su camgöbeğine, ortada koyu yeşile dönüyor —
//   o renk `albedo`, suyun kendi saçtığı, aydınlatılan renk;
// - **köpük:** zeminin yüzeye 30 cm'den yakın olduğu kıyı şeridinde, dalgacıklarla kırılan beyaz bir
//   köpük — orada su ışık geçirmez olup köpüğün rengini gösteriyor;
// - **dalgacık:** yüzey geometride düz (kayık ve yüzen her şey `su::WATER`'a göre); dalgacık yalnız
//   normalde, birkaç yönde kayan dalgaların eğimi, uzakta sönerek.

#import gizmo::surface::{Surface, SurfaceInput}

// Bir dalganın eğimi (yüksekliğin x ve z türevleri) ve yüksekliği: yön, dalga boyu (m), genlik (m),
// hız (m/s).
fn wave(p: vec2<f32>, t: f32, dir: vec2<f32>, wavelength: f32, amplitude: f32, speed: f32) -> vec3<f32> {
    let d = normalize(dir);
    let k = 6.2831853 / wavelength;
    let phase = k * (dot(d, p) - speed * t);
    return vec3<f32>(d * (k * amplitude * cos(phase)), amplitude * sin(phase));
}

// Göl suyu: metre başına soğurma (kırmızı, yeşil, mavi) ve derinde saçtığı renk.
const ABSORPTION: vec3<f32> = vec3<f32>(0.6, 0.2, 0.15);
const DEEP: vec3<f32> = vec3<f32>(0.015, 0.06, 0.06);
const FOAM: vec3<f32> = vec3<f32>(0.85, 0.88, 0.9);
// Köpük şeridi: zeminin yüzeye bu kadar yakın olduğu yer, metre.
const FOAM_DEPTH: f32 = 0.3;

fn material_surface(input: SurfaceInput) -> Surface {
    let p = input.world_position.xz;
    let t = input.time;
    var w = vec3<f32>(0.0);
    w += wave(p, t, vec2<f32>(1.0, 0.35), 3.1, 0.035, 0.9);
    w += wave(p, t, vec2<f32>(-0.6, 1.0), 1.7, 0.018, 0.7);
    w += wave(p, t, vec2<f32>(0.3, -1.0), 0.9, 0.008, 0.55);
    w += wave(p, t, vec2<f32>(-1.0, -0.4), 0.47, 0.004, 0.4);
    // Uzakta dalgacıklar düzleşiyor.
    let far = length(input.camera_position - input.world_position);
    let g = w.xy / (1.0 + far * 0.025);
    let n = normalize(vec3<f32>(-g.x, 1.0, -g.y));

    // Kıyı köpüğü: sığlıkta, dalganın tepesinde daha geniş, yer yer kopuk.
    var foam = 0.0;
    let depth = input.behind_distance;
    if (depth >= 0.0) {
        let edge = FOAM_DEPTH * (1.0 + 18.0 * w.z);
        let breakup = 0.5 + 0.5 * sin(p.x * 3.1 + t * 0.7) * sin(p.y * 2.7 - t * 0.5);
        foam = (1.0 - smoothstep(0.0, max(edge, 0.05), depth)) * mix(0.6, 1.0, breakup);
    }

    var s: Surface;
    s.base_color = mix(DEEP, FOAM, foam);
    s.albedo = s.base_color;
    s.alpha = 1.0;
    s.alpha_cutoff = 0.0;
    s.N = normalize(mix(n, vec3<f32>(0.0, 1.0, 0.0), foam));
    s.roughness = mix(0.04, 0.8, foam);
    s.metallic = 0.0;
    s.emissive = vec3<f32>(0.0);
    // Köpük ışık geçirmiyor: soğurmayı köpükte büyütmek altını kapatıp köpüğün rengini gösteriyor.
    s.absorption = ABSORPTION + vec3<f32>(foam * 40.0);
    return s;
}
