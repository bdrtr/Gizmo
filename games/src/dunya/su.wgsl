// Dünya'nın gölleri: düz bir yüzey, üstünde kayan dalgacıklar.
//
// Motorun özel yüzeyi (`SurfaceMaterial`), saydam: ışığı, gökten ve güneşten yansımayı, gece
// kararmayı, sisi motorun aydınlatması veriyor — gök yansıması bir sabit renk değil, o anki gök.
// Yüzey geometride düz kalıyor (su seviyesi `su::WATER`, kayık ve yüzen her şey ona göre); dalgacık
// yalnız normalde: birkaç yönde, birkaç boyda, zamanla kayan dalgaların eğimi. Uzakta dalgacıklar
// sönüyor — piksel başına birden çok dalga düşünce yalnız titreşirdi.
//
// Yakından bakınca su saydam, sığ açıyla bakınca (Fresnel) aynalaşıp örtüyor.

#import gizmo::surface::{Surface, SurfaceInput}

// Bir dalganın eğimi (yüksekliğin x ve z türevleri): yön, dalga boyu (m), genlik (m), hız (m/s).
fn slope(p: vec2<f32>, t: f32, dir: vec2<f32>, wavelength: f32, amplitude: f32, speed: f32) -> vec2<f32> {
    let d = normalize(dir);
    let k = 6.2831853 / wavelength;
    let phase = k * (dot(d, p) - speed * t);
    return d * (k * amplitude * cos(phase));
}

fn material_surface(input: SurfaceInput) -> Surface {
    let p = input.world_position.xz;
    let t = input.time;
    var g = vec2<f32>(0.0);
    g += slope(p, t, vec2<f32>(1.0, 0.35), 3.1, 0.035, 0.9);
    g += slope(p, t, vec2<f32>(-0.6, 1.0), 1.7, 0.018, 0.7);
    g += slope(p, t, vec2<f32>(0.3, -1.0), 0.9, 0.008, 0.55);
    g += slope(p, t, vec2<f32>(-1.0, -0.4), 0.47, 0.004, 0.4);
    // Uzakta dalgacıklar düzleşiyor.
    let far = length(input.camera_position - input.world_position);
    g *= 1.0 / (1.0 + far * 0.025);
    let n = normalize(vec3<f32>(-g.x, 1.0, -g.y));

    let view = normalize(input.camera_position - input.world_position);
    let fresnel = pow(1.0 - clamp(dot(n, view), 0.0, 1.0), 5.0);

    var s: Surface;
    // Suyun kendi rengi: koyu, yeşile çalan bir mavi; aydınlığı ve yansıması motordan.
    s.base_color = vec3<f32>(0.03, 0.12, 0.14);
    s.albedo = s.base_color;
    s.alpha = mix(0.62, 0.97, fresnel);
    s.alpha_cutoff = 0.0;
    s.N = n;
    s.roughness = 0.05;
    s.metallic = 0.0;
    s.emissive = vec3<f32>(0.0);
    return s;
}
