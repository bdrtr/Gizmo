//! # Çizelgenin iç yapısı
//!
//! Bu demo dört ayrı yeteneği birden ele alıyor, çünkü dördünün de cevabı aynı yerden —
//! **çizelgenin kendisinden** — geliyor: uygulamaya özel çizelge eklemek, yürütücüyü
//! değiştirmek, kısıtsız sistem sırasının değişkenliği, ve sistem sistem adımlama.
//!
//! ## Motorda çizelgenin neresi açık
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | uygulamaya özel bir çizelge eklemek | **yok** — [`Phase`] beş varyantlı **kapalı** bir enum |
//! | yürütücüyü seçmek ya da değiştirmek | **yok** — yürütücü `Schedule::run_batches`, özel ve değiştirilemez |
//! | kısıtsız sistem sırasının değişkenliği | **kısmen** — aşağıda ölçüldü |
//! | sistem sistem ilerletme (adımlama) | **yok** — ama `Schedule::run` açık, adımlamayı uygulama kurabilir |
//!
//! ## Ölçülen 1: yığın düzeni deterministik, yığın **içi** değil
//!
//! "Kısıt koymazsan sıra değişir" kuralı Gizmo'da **ikiye ayrılıyor**, ve ayrımı çizelgenin
//! kendi belgesi yazıyor:
//!
//! * **Yığınlara dağılım deterministik.** `build()` topolojik sıralamayı Kahn ile yapıp
//!   sistemleri açgözlü paketliyor, ve tek eşitlik bozucu **ekleme sırası** — belgenin kendi
//!   ifadesiyle *"insertion order is the only tie-break, which is what makes the layout
//!   deterministic."* Yani aynı program aynı düzeni kuruyor.
//!
//! * **Bir yığının içi değil.** Aynı yığındaki sistemler rayon ile **eşzamanlı ve sırasız**
//!   koşuyor. Erişimleri çakışmadığı için bu normalde görünmez — ama belge bir deliği açıkça
//!   adlandırıyor: `Commands` alan iki sistem yalnız komut kuyruğunun **okunduğunu** bildiriyor,
//!   yani aynı yığına düşebiliyorlar ve **kuyruğa yazma sıraları yeniden üretilemez**.
//!
//! Demo tam olarak o deliği ölçüyor: kısıtsız, `Commands` alan iki sistem, her karede hangisinin
//! önce yazdığını sayıyor.
//!
//! ## Ölçüldü
//!
//! Sekiz kısıtsız yazıcı, aralarında hiçbir `before`/`after` yok, hepsi `Commands` alıyor.
//! Ölçüldü (2026-08-23, 600 kare, iki ayrı koşu):
//!
//! | | farklı sıralama | sıra değişimi | en sık sıralama |
//! |---|-----------------|---------------|-----------------|
//! | koşu 1 | **78** | 373 / 600 | `ABCDEFGH` ×321 (%53) |
//! | koşu 2 | **54** | 309 / 600 | `ABCDEFGH` ×379 (%63) |
//! | `GIZMO_SCHEDULE_CHAIN=1` | **1** | **0** / 600 | `ABCDEFGH` ×600 (%100) |
//!
//! Yani kısıtsız sıra gerçekten yeniden üretilemez: 600 karede onlarca farklı sıralama, ve iki
//! koşu birbirini tutmuyor. Ekleme sırası (`ABCDEFGH`) en sık çıkan hâl ama yarıdan biraz fazla —
//! **garanti değil**.
//!
//! ### Ölçüm notu: iki yazıcı bunu göstermiyor
//!
//! İlk kurulumda yalnız **iki** kısıtsız yazıcı vardı ve sonuç iki koşuda da 600/600 aynıydı —
//! sıfır değişim. Bu, sıranın garantili olduğu anlamına gelmiyordu; iki öğelik bir dilimde
//! rayon'un iş çalmasına gerek kalmıyor, yani ikisi de çağıran iş parçacığında sırayla koşuyor.
//! Yazıcı sayısı sekize çıkınca havuz gerçekten bölünüyor ve değişkenlik ortaya çıkıyor.
//!
//! Dersi genel: **"ölçtüm, değişmedi" ile "değişmez" aynı şey değil.** Küçük bir çizelgede
//! kısıtsız sıra kararlı görünüp üretimde bozulabilir.
//!
//! ### Ve çare gerçekten çalışıyor
//!
//! Aynı sekiz sistem `after` zinciriyle bağlandığında (`GIZMO_SCHEDULE_CHAIN=1`) 600 karede
//! **tek** sıralama görülüyor, sıfır değişim. Yani `label`/`after` kısıtları sırayı gerçekten
//! sabitliyor — sadece elle yazmak gerekiyor: bir sistem listesini tek çağrıda zincirleyen
//! kolaylık **yok**.
//!
//! Bir uyarı: kısıtı **çalıştırma anında** takıp çıkarmak mümkün değil, çizelge kurulurken
//! belirleniyor. Karşılaştırma bu yüzden iki ayrı koşu.
//!
//! ## Kontroller
//!   * `GIZMO_SCHEDULE_CHAIN=1` ile başlatın — sekiz yazıcı `after` zinciriyle bağlanır
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::core::Commands;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};
use std::sync::{Mutex, OnceLock};

/// İki sistemin karede hangi sırayla yazdığını tutan defter.
///
/// Kaynak (`ResMut`) DEĞİL, bilerek: iki sistem aynı kaynağa yazsaydı erişimleri çakışır ve
/// çizelge onları ayrı yığınlara koyardı — yani ölçmek istediğim durumu ölçüm aracı yok ederdi.
static ORDER_LOG: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();

fn order_log() -> &'static Mutex<Vec<u8>> {
    ORDER_LOG.get_or_init(|| Mutex::new(Vec::new()))
}

/// Kaç kısıtsız yazıcı var. İkisi rayon'un iş çalmasına yetmiyor — aşağıdaki ölçüm notuna bakın.
const WRITERS: usize = 8;

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct ScheduleReport {
    frame: u32,
    /// Bu koşuda görülen farklı sıralamalar ve kaçar kez görüldükleri.
    seen: Vec<(String, u32)>,
    /// Sıranın bir önceki kareye göre değiştiği kare sayısı.
    flips: u32,
    last: String,
}
gizmo::core::impl_component!(ScheduleReport);

/// Zincir modu açıksa sisteme `after` kısıtını takar, kapalıysa dokunmaz.
///
/// Kısıtı **çalıştırma anında** eklemek mümkün değil — çizelge kurulurken belirleniyor. Bu yüzden
/// karşılaştırma iki ayrı koşuyla yapılıyor, tek koşu içinde bir tuşla değil.
fn chained(
    config: gizmo::core::system::SystemConfig,
    after: Option<&'static str>,
) -> gizmo::core::system::SystemConfig {
    match (chain_enabled(), after) {
        (true, Some(label)) => config.after(label),
        _ => config,
    }
}

/// Zincir modu açık mı.
fn chain_enabled() -> bool {
    std::env::var("GIZMO_SCHEDULE_CHAIN").is_ok()
}

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Schedule Internals", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Beş faz, beş küp — `Phase` kapalı bir enum, altıncısı eklenemiyor.
            for (i, phase) in Phase::ALL.iter().enumerate() {
                let x = (i as f32 - 2.0) * 1.6;
                let _ = phase;
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.55)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.30 + i as f32 * 0.13, 0.55, 0.85 - i as f32 * 0.11, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                ));
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.0, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 30.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.5) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });
            scene.world.insert_resource(ScheduleReport::default());
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 9.0), Vec3::ZERO);
        })
        // Kasıtlı olarak KISITSIZ ve hepsi `Commands` alıyor: belgenin adlandırdığı delik bu.
        // `GIZMO_SCHEDULE_CHAIN=1` ile aynı sekiz sistem `after` zinciriyle bağlanıyor —
        // karşılaştırma böyle kuruluyor.
        .add_update_system(chained(writer_a.in_phase(Phase::Update).label("w_a"), None))
        .add_update_system(chained(writer_b.in_phase(Phase::Update).label("w_b"), Some("w_a")))
        .add_update_system(chained(writer_c.in_phase(Phase::Update).label("w_c"), Some("w_b")))
        .add_update_system(chained(writer_d.in_phase(Phase::Update).label("w_d"), Some("w_c")))
        .add_update_system(chained(writer_e.in_phase(Phase::Update).label("w_e"), Some("w_d")))
        .add_update_system(chained(writer_f.in_phase(Phase::Update).label("w_f"), Some("w_e")))
        .add_update_system(chained(writer_g.in_phase(Phase::Update).label("w_g"), Some("w_f")))
        .add_update_system(chained(writer_h.in_phase(Phase::Update).label("w_h"), Some("w_g")))
        // Sırayı okuyan sistem `PostUpdate`'te — faz sınırı, yığın sınırından güçlü.
        .add_update_system(tally.in_phase(Phase::PostUpdate))
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<ScheduleReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("sched".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(480.0);
                        ui.heading("Çizelgenin iç yapısı");
                        ui.label("Phase: kapalı 5 varyant — özel çizelge eklenemiyor.");
                        ui.label("yürütücü: Schedule::run_batches, özel ve değiştirilemez.");
                        ui.label("adımlama (stepping) API'si yok.");
                        ui.separator();
                        ui.label(format!(
                            "kare: {} · {} yazıcı · zincir: {}",
                            r.frame,
                            WRITERS,
                            if chain_enabled() { "AÇIK" } else { "kapalı" }
                        ));
                        ui.label(format!("farklı sıralama: {}", r.seen.len()));
                        ui.label(format!("sıra değişimi  : {} kez", r.flips));
                        let mut top = r.seen.clone();
                        top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
                        for (order, n) in top.iter().take(4) {
                            ui.monospace(format!("  {order}  ×{n}"));
                        }
                        ui.separator();
                        if r.seen.len() > 1 {
                            ui.colored_label(
                                gizmo::egui::Color32::from_rgb(230, 160, 80),
                                "-> yığın içi sıra YENİDEN ÜRETİLEMEZ",
                            );
                        } else if r.frame > 60 {
                            ui.label("-> bu koşuda tek sıralama görüldü (garanti değil)");
                        }
                        ui.separator();
                        ui.label("yığınlara dağılım deterministik (ekleme sırası eşitlik bozucu),");
                        ui.label("yığın İÇİ rayon ile eşzamanlı ve sırasız.");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kısıtsız yazıcılar. Sekiz tane, aralarında **hiçbir** sıralama kısıtı yok, ve hepsi
/// `Commands` alıyor — yani hepsi yalnız komut kuyruğunun *okunduğunu* bildiriyor, dolayısıyla
/// çizelge hepsini aynı yığına koyabiliyor.
macro_rules! writer {
    ($name:ident, $tag:expr) => {
        fn $name(mut commands: Commands, _input: Res<Input>) {
            order_log().lock().unwrap().push($tag);
            // Komutu gerçekten kuyruğa koy: parametrenin ölü olmadığından emin ol.
            commands.spawn().despawn();
        }
    };
}
writer!(writer_a, b'A');
writer!(writer_b, b'B');
writer!(writer_c, b'C');
writer!(writer_d, b'D');
writer!(writer_e, b'E');
writer!(writer_f, b'F');
writer!(writer_g, b'G');
writer!(writer_h, b'H');

/// Kareyi kapatır ve o karenin sıralamasını deftere işler. Ayrı bir **fazda**, yani yazıcılarla
/// aynı yığına düşemez.
fn tally(mut report: ResMut<ScheduleReport>) {
    let order: String = {
        let mut log = order_log().lock().unwrap();
        let s = String::from_utf8_lossy(&log).to_string();
        log.clear();
        s
    };
    if order.len() != WRITERS {
        return; // yarım kare — sayma
    }

    report.frame += 1;
    if !report.last.is_empty() && order != report.last {
        report.flips += 1;
    }
    report.last = order.clone();
    if let Some(entry) = report.seen.iter_mut().find(|(o, _)| *o == order) {
        entry.1 += 1;
    } else {
        report.seen.push((order, 1));
    }

    if std::env::var("GIZMO_SCHEDULE_SELFTEST").is_ok() && report.frame.is_multiple_of(200) {
        let mut top = report.seen.clone();
        top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        gizmo::gizmo_log!(
            Info,
            "kare {:>4} · zincir {} · farklı sıralama {} · sıra değişimi {} · en sık {:?}",
            report.frame,
            if chain_enabled() { "AÇIK" } else { "kapalı" },
            report.seen.len(),
            report.flips,
            &top[..top.len().min(3)]
        );
    }
}
