//! # Ekran uzayı yansımaları
//!
//! Ekran uzayı yansımaları: parlak bir yüzeyin, karşısındaki nesneleri **ekranda zaten çizilmiş**
//! pikselleri tarayarak yansıtması. Ucuz, ve sınırı da oradan geliyor — ekranda olmayan bir şey
//! yansıyamaz.
//!
//! ## Envanter: sıfır düğme
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | kare kare açıp kapamak | **var** (2026-08-23, `SsrState::enabled`) |
//! | yansımanın vazgeçeceği pürüz eşiği | **var** (2026-08-24, `SsrParams::roughness_cutoff`) |
//! | yüzey kalınlığı varsayımı | **var** — `::thickness` |
//! | doğrusal tarama adımı sayısı | **var** — `::max_steps` |
//! | tarama adım boyu | **var** — `::step_size` |
//! | pürüz solma aralığı | **var** — `::fade_start` / `::fade_end` |
//! | başlangıç ofseti, kenar solma | **var** — `::start_offset` / `::edge_fade` |
//! | doğrusal tarama adımının üsteli | **yok** |
//! | ikiye bölerek incelme adımı sayısı | **yok** — motorda hiç yazılmamış |
//! | kesişimi sekantla arama | **yok** — motorda hiç yazılmamış |
//!
//! Sekiz sayı 32 baytlık tek bir uniform'da ve her kare yükleniyor. Varsayılanlar shader'da duran
//! değerlerin ta kendisi — motorun `the_ssr_defaults_are_the_shader_literals` testi bunu piksel
//! piksel kilitliyor.
//!
//! ### Ölçülen: beş düğmenin beşi de kareye ulaşıyor
//!
//! `GIZMO_SSR_KNOB=<0..5>` bir ayarı sabitliyor (anahtar hâlâ 60 karede bir çevirdiği için ölçüm
//! bunu kullanmak zorunda). Kare 510, sol yarı, alt yarı (2026-08-24):
//!
//! | ayar | farklı piksel | en büyük kanal farkı |
//! |------|---------------|----------------------|
//! | varsayılan (referans) | %0,00 | 0 |
//! | pürüz eşiği 0,0 | **%15,53** | 167 |
//! | kalınlık 0,001 | **%15,53** | 167 |
//! | kalınlık 12,0 | **%18,81** | 104 |
//! | başlangıç ofseti 3,0 | **%11,49** | 167 |
//! | kenar solma 0,45 | **%18,62** | 167 |
//!
//! İlk ikisi aynı sayıyı veriyor ve bu doğru: ikisi de yansımayı tamamen kapatıyor — biri ışını
//! hiç göndermeyerek, öteki hiçbir isabeti kabul etmeyerek.
//!
//! `max_steps` bu ölçümde yok, ve sebebi bir eksiklik değil: bu sahnede yansıma **ilk adımda**
//! bulunuyor, yani 20'den 4'e — hatta 1'e — inmek hiçbir pikseli değiştirmiyor. Menzil düğmesi, ve
//! bu sahnenin kaybedecek menzili yok.
//!
//! ## Açıp kapatmak artık iki yönlü
//!
//! TAA ve FXAA'nın `enabled` bayrağı vardı (bkz. `anti_aliasing`), yani kare kare açılıp
//! kapanabiliyorlardı. **SSR'de yoktu:** `Renderer::ssr` bir `Option<SsrState>`, ve bir
//! uygulamanın elindeki tek hareket onu `None` yapmaktı — yani durumu **yok etmek**. Geri açmak
//! `SsrState::new` ile cihaz, format ve boyutla yeniden kurmayı gerektiriyordu, o yüzden bu demo
//! kipi çalışma anında değil başlangıçta seçiyordu.
//!
//! **2026-08-23: beş efektin de `enabled` bayrağı var** — `ssr`, `ssgi`, `volumetric`, `ssao`,
//! `decal`. Bayrak kapalıyken geçiş atlanıyor, durum ayakta kalıyor, yeniden boyutlandırma yine
//! işliyor; geri açmak bir atama.
//!
//! ### Ölçüldü: aynı koşuda kapanıyor **ve geri açılıyor**
//!
//! Demo her 60 karede bir kendiliğinden çeviriyor. 420 karelik koşuda (2026-08-23):
//!
//! | | |
//! |---|---|
//! | kapanma | **4** |
//! | **geri** açılma | **3** |
//!
//! `= None` ile ikinci sayı hep 0 olurdu — durum yok olduğu için ikinci bir açılma yok.
//!
//! Ve bayrak gerçekten kareye ulaşıyor. Kare 450 (kapalı) ile kare 510 (açık) — aynı koşu, aynı
//! kamera, tek değişen bayrak. Zeminin küplerin altındaki bölgesinde ortalama RGB:
//!
//! | bölge | kapalı | açık | kanal farkı |
//! |-------|--------|------|-------------|
//! | kırmızı küpün altı | (32,2 · 34,3 · 38,9) | (69,8 · 43,1 · 47,7) | (**+37,6** · +8,8 · +8,8) |
//! | yeşil küpün altı | (29,3 · 31,3 · 35,9) | (43,6 · 92,4 · 53,8) | (+14,3 · **+61,1** · +18,0) |
//! | mavi küpün altı | (32,6 · 34,7 · 39,4) | (44,4 · 53,1 · 78,1) | (+11,8 · +18,5 · **+38,7**) |
//!
//! Karenin alt yarısında (arayüz dışında) **%5,85** piksel değişiyor.
//!
//! Motorun kendi golden testi de aynı iddiayı daha sert kuruyor: `enabled = false` ile
//! `= None` **piksel piksel aynı** kareyi üretiyor
//! (`switching_an_effect_off_reversibly_renders_the_same_frame_as_destroying_it`). Yani bayrak
//! kapatmanın *başka* bir yolu değil, **aynı** yolu — tersine çevrilebilir olanı.
//!
//! Serideki bütün demoların `set_render` kapanışında hâlâ `renderer.ssr = None` yazması artık bir
//! zorunluluk değil, alışkanlık: onların SSR'yi geri açma ihtiyacı yok.
//!
//! ## Ölçülen: yansıma bir sayıdır
//!
//! Sahne, parlak (düşük pürüzlü) bir zeminin üstüne konmuş üç renkli küp. SSR çalışıyorsa
//! zeminin **küplerin tam altındaki** bölgesi küplerin rengini almalı; SSR kapalıyken orası
//! zeminin kendi rengi olmalı.
//!
//! Ölçüt de bu: zeminin o bölgesindeki renk kanallarının küpün rengine kayması.
//!
//! Ölçüldü (2026-08-23, kare 240, `GIZMO_SSR=<0|1>` ile iki ayrı koşu — yukarıdaki tablo aynı
//! şeyi tek koşuda ölçüyor). Zeminin, küpün tam altındaki bölgesinde ortalama RGB:
//!
//! | bölge | SSR kapalı | SSR açık | kanal farkı |
//! |-------|------------|----------|-------------|
//! | kırmızı küpün altı | (31,3 · 33,4 · 38,0) | (91,0 · 47,5 · 52,1) | (**+59,7** · +14,1 · +14,1) |
//! | yeşil küpün altı | (31,0 · 33,0 · 37,6) | (45,3 · 93,9 · 55,6) | (+14,3 · **+60,9** · +18,0) |
//! | mavi küpün altı | (31,4 · 33,5 · 38,0) | (47,5 · 58,6 · 91,0) | (+16,1 · +25,2 · **+53,0**) |
//!
//! SSR kapalıyken üç bölge de aynı nötr karanlık — (31 · 33 · 38), yani zeminin kendi rengi.
//! Açıldığında her bölge **kendi küpünün** kanalını alıyor, ve baskın kanal ötekilerin üç-dört
//! katı. Yansıma doğru rengi doğru yere taşıyor.
//!
//! ## Kontroller
//!   * `GIZMO_SSR=0` — **başlangıçta** kapalı başlat (anahtar yine 60 karede bir çeviriyor)
//!   * `GIZMO_SSR_KNOB=<0..5>` — bir şekillendirme ayarını sabitle (ölçüm için)
//!   * `GIZMO_SSR_SELFTEST=1` — her çevirmede sayacı konsola yaz
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Küplerin renkleri — kanal kanal ayrıldıkları için yansımada hangisinin geldiği okunabiliyor.
const CUBES: [(f32, Vec4); 3] = [
    (-3.2, Vec4::new(0.95, 0.15, 0.15, 1.0)),
    (0.0, Vec4::new(0.15, 0.95, 0.20, 1.0)),
    (3.2, Vec4::new(0.20, 0.35, 0.95, 1.0)),
];

/// SSR başlangıçta açık mı. Artık yalnız *başlangıç* değeri — anahtar çalışma anında da var.
fn ssr_enabled() -> bool {
    std::env::var("GIZMO_SSR").map(|v| v != "0").unwrap_or(true)
}

/// Kaç karede bir kendiliğinden açılıp kapanılacağı. Ölçüm koşusunda kapanma ve **geri açılma**
/// bunun sayesinde tek koşuda görülüyor; eskiden geri açılma hiç görülemiyordu.
const TOGGLE_EVERY: u32 = 60;

/// Ayar turu: her adım bir alanı varsayılanından uzağa itiyor.
const KNOBS: [(&str, fn(&mut gizmo::renderer::ssr::SsrParams)); 6] = [
    ("varsayılan", |_| {}),
    ("pürüz eşiği 0,0 (kapalı)", |p| p.roughness_cutoff = 0.0),
    ("kalınlık 0,001", |p| p.thickness = 0.001),
    ("kalınlık 12,0", |p| p.thickness = 12.0),
    ("başlangıç offseti 3,0", |p| p.start_offset = 3.0),
    ("kenar solma 0,45", |p| p.edge_fade = 0.45),
];

/// Ölçüm defteri: SSR'nin kaç kez kapanıp açıldığı.
#[derive(Default, Clone)]
struct SsrToggleReport {
    frame: u32,
    on: bool,
    /// Kapanma sayısı.
    turned_off: u32,
    /// **Geri** açılma sayısı — durumu yok etmek bunu imkânsız kılıyordu.
    turned_back_on: u32,
}
gizmo::core::impl_component!(SsrToggleReport);

fn main() {
    let enabled = ssr_enabled();

    App::<SimpleSceneState>::new("Gizmo Engine - SSR", 1280, 720)
        .with_simple_scene(move |scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;

            // Ayna gibi zemin: pürüz düşük, metaliklik yüksek. SSR'nin bakacağı yüzey bu.
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 40.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.06, 0.06, 0.07, 1.0), 0.08, 0.9),
                MeshRenderer::new(),
            ));

            let cube = AssetManager::create_cube(device);
            for (x, albedo) in CUBES {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 1.1, 0.0)).with_scale(Vec3::splat(0.55)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(albedo, 0.35, 0.0),
                    MeshRenderer::new(),
                ));
            }

            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.4) * Quat::from_rotation_x(-0.8),
                intensity: 2.4,
                ..Default::default()
            });

            scene.world.insert_resource(SsrToggleReport {
                on: enabled,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, 2.2, 8.5), Vec3::new(0.0, 0.6, 0.0));
        })
        .set_render(move |world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssgi = None;

            // SSR artık bir bayrakla kapanıyor: durum ayakta kalıyor, dokular ve boru hattı
            // yerinde. `= None` hâlâ mümkün ama geri dönüşü yok — aradaki fark tam olarak
            // aşağıdaki sayaç.
            if let Some(mut report) = world.get_resource::<SsrToggleReport>().map(|r| r.clone()) {
                report.frame += 1;
                if report.frame.is_multiple_of(TOGGLE_EVERY) {
                    report.on = !report.on;
                    if report.on {
                        report.turned_back_on += 1;
                    } else {
                        report.turned_off += 1;
                    }
                    if std::env::var("GIZMO_SSR_SELFTEST").is_ok() {
                        gizmo::gizmo_log!(
                            Info,
                            "kare {:>4} · SSR {} · kapandı {} · GERİ açıldı {}",
                            report.frame,
                            if report.on { "AÇIK " } else { "kapalı" },
                            report.turned_off,
                            report.turned_back_on
                        );
                    }
                }
                if let Some(ssr) = renderer.ssr.as_mut() {
                    ssr.enabled = report.on;
                    // `GIZMO_SSR_KNOB=<0..5>` bir ayarı sabitliyor. Ölçüm bunu kullanmak zorunda:
                    // anahtar 60 karede bir çevrildiği için iki farklı karenin farkı ayarın değil
                    // anahtarın farkı olurdu.
                    let k = std::env::var("GIZMO_SSR_KNOB")
                        .ok()
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0)
                        .min(KNOBS.len() - 1);
                    let mut p = gizmo::renderer::ssr::SsrParams::default();
                    KNOBS[k].1(&mut p);
                    ssr.params = p;
                }
                world.insert_resource(report);
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .set_ui(move |world, _state, ctx| {
            let report = world
                .get_resource::<SsrToggleReport>()
                .map(|r| r.clone())
                .unwrap_or_default();
            gizmo::egui::Area::new("ssr".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(430.0);
                        ui.heading("Ekran uzayı yansıması");
                        ui.label(format!(
                            "SSR: {} · kare {}",
                            if report.on { "açık" } else { "kapalı" },
                            report.frame
                        ));
                        ui.label("zemin: pürüz 0,08 · metaliklik 0,9");
                        ui.separator();
                        ui.label("SsrParams: sekiz şekillendirme alanı (2026-08-24).");
                        ui.label("pürüz eşiği · kalınlık · adım sayısı · adım boyu");
                        ui.label("solma aralığı · başlangıç ofseti · kenar solma");
                        ui.label("adım üsteli, ikiye bölme, sekant: motorda hiç yok.");
                        ui.separator();
                        ui.label(format!(
                            "SsrState::enabled ile {} kez kapandı, {} kez GERİ açıldı",
                            report.turned_off, report.turned_back_on
                        ));
                        ui.label("`= None` ile geri açılma sayısı hep 0 olurdu: durum yok olur.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}
