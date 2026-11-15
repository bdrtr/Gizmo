//! Ölçüm: FPS göstergesi ve darboğaz dökümü.
//!
//! Motorun kare profilcisi (`FrameProfiler`) pencereli döngünün kendi kısımlarını ölçüyor —
//! `update` (oyunun güncellemesi), `render` (karenin hazırlanması ve GPU'ya verilmesi), `draw` (çizimin
//! kodlanması, render iş parçacığındaysa orada). Bu modül oyunun kendi adımlarını da ölçüyor
//! ([`timed`]: hücreler, fizik, canlılar, animasyon, HUD…) ve hepsini gösteriyor:
//!
//! - **F3** sol altta gösterge: FPS, ortalama ve en kötü kare süresi, kısımlar, oyunun en pahalı
//!   adımları ve son karenin çizim sayıları (çizim çağrısı, üçgen, gölge geçişleri);
//! - **`GIZMO_PROFILE=1`** ile beş saniyede bir aynı döküm standart hataya — oyun oynanırken arkada.
//!
//! Okuma: kare süresi `update + render`'dan çok büyükse kare GPU'yu (ya da dikey eşitlemeyi) bekliyor;
//! `update` büyükse oyunun bir adımı, hangisi olduğunu adımlar söylüyor.

use super::*;
use gizmo::core::profiler::FrameProfiler;
use gizmo::renderer::components::RenderStats;

/// Ortalamanın alındığı kare sayısı (profilci 300 kare tutuyor).
const WINDOW: usize = 120;
/// Oyunun ölçülen adımları, göstergedeki sırasıyla.
const STEPS: [&str; 9] = ["hücreler", "canlılar", "fizik", "beden", "kristal", "panel", "harita", "ses", "hud"];
/// Fiziğin kısımları (motorun `FixedPhysics`'i ölçüyor), adım başına; göstergede kare başına toplam.
const PHYSICS: [(&str, &str); 4] =
    [("physics.vehicles", "araçlar"), ("physics.characters", "karakterler"), ("physics.step", "katı cisim"), ("physics.history", "geçmiş")];

pub(super) struct Meter {
    text: Entity,
    pub(super) shown: bool,
    log: bool,
    log_in: f32,
}

/// Göstergeyi kurar; profilci yoksa onu da.
pub(super) fn install(world: &mut World, font: FontId) -> Meter {
    if world.get_resource::<FrameProfiler>().is_none() {
        world.insert_resource(FrameProfiler::new());
    }
    let text = world.spawn_bundle((Text::pinned("", font, 16.0, TextAnchor::BottomLeft, Vec2::new(16.0, -16.0))
        .with_color(Vec4::new(1.0, 1.0, 0.75, 1.0)),));
    let log = std::env::var("GIZMO_PROFILE").is_ok_and(|v| v != "0");
    Meter { text, shown: false, log, log_in: 5.0 }
}

/// `f`'i ölçer ve profilciye `name` adıyla yazar.
pub(super) fn timed<R>(world: &mut World, name: &'static str, f: impl FnOnce(&mut World) -> R) -> R {
    let start = std::time::Instant::now();
    let r = f(world);
    if let Some(mut p) = world.get_resource_mut::<FrameProfiler>() {
        p.record_scope(name, start.elapsed());
    }
    r
}

/// Son [`WINDOW`] karenin dökümü.
pub(super) fn report(world: &World) -> Option<String> {
    let p = world.get_resource::<FrameProfiler>()?;
    let n = WINDOW.min(p.history().len());
    if n == 0 {
        return None;
    }
    let frame = p.avg_frame_ms(n);
    let worst = p.history().iter().rev().take(n).map(|f| f.total_ms).fold(0.0, f64::max);
    let part = |name: &str| p.avg_scope_ms(name, n);
    let mut steps: Vec<(&str, f64)> = STEPS.iter().map(|s| (*s, part(s))).filter(|(_, ms)| *ms > 0.0).collect();
    steps.sort_by(|a, b| b.1.total_cmp(&a.1));
    let steps = steps.iter().take(5).map(|(s, ms)| format!("{s} {ms:.1}")).collect::<Vec<_>>().join(" · ");
    // Fiziğin kısımları kare başına: bir karede birkaç adım olabiliyor.
    let per_frame = |name: &str| p.history().iter().rev().take(n).flat_map(|f| f.scopes.iter()).filter(|s| s.name == name).map(|s| s.duration_ms()).sum::<f64>() / n as f64;
    let physics = PHYSICS.iter().map(|(k, label)| format!("{label} {:.1}", per_frame(k))).collect::<Vec<_>>().join(" · ");
    let stats = world
        .get_resource::<RenderStats>()
        .map(|s| {
            format!(
                "\nçizim çağrısı {} · üçgen {:.2} M · örnek {} · gölge {} geçiş, {} örnek",
                s.draw_calls,
                s.triangles as f64 / 1.0e6,
                s.instances,
                s.shadow_passes,
                s.shadow_instances
            )
        })
        .unwrap_or_default();
    Some(format!(
        "FPS {:.0} · kare {frame:.1} ms (en kötü {worst:.1}) · güncelleme {:.1} · render {:.1} · çizim {:.1} ms\noyun: {steps}\nfizik: {physics}{stats}",
        1000.0 / frame.max(0.001),
        part("update"),
        part("render"),
        part("draw"),
    ))
}

/// Her kare: F3 göstergeyi açar, kapar; açıksa yazar; istenmişse beş saniyede bir döker.
pub(super) fn update(world: &mut World, meter: &mut Meter, toggle: bool, dt: f32) {
    if toggle {
        meter.shown = !meter.shown;
    }
    let text = if meter.shown { report(world).unwrap_or_default() } else { String::new() };
    set_text(world, meter.text, &text);
    if meter.log {
        meter.log_in -= dt;
        if meter.log_in <= 0.0 {
            meter.log_in = 5.0;
            if let Some(r) = report(world) {
                eprintln!("[dunya ölçüm] {}", r.replace('\n', " | "));
            }
        }
    }
}
