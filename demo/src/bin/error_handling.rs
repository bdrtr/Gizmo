//! # Hata yönetimi
//!
//! Bir sistem hata döndürebilir mi, ve hata olunca ne olur? Zengin bir cevap şöyle olurdu: sistem
//! `Result` döndürebilir, `?` kullanılabilir, ve hatayı ne yapacağını genel bir hata işleyici ya
//! da sistem başına konan bir politika belirler — panikle, günlükle, yok say.
//!
//! ## Motorun cevabı tek kelime: **panikle**
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | sistemin `Result` döndürebilmesi | **hayır** — `IntoSystem` yalnız `()` döndüren `FnMut` için |
//! | sistemde `?` operatörü | **hayır** |
//! | genel hata işleyici | **yok** |
//! | sistem başına hata politikası | **yok** |
//! | eksik kaynak | **panik** — uygulama duruyor |
//! | eksik kaynağa dayanıklılık | **`Option<Res<T>>`** — 2026-08-23'te eklendi, aşağıda ölçüldü |
//!
//! Ve bu bir eksiklik değil, **yazılı bir karar**. Motorun panik mesajının kendisi gerekçeyi
//! taşıyor: *"Gizmo Engine, hataların sessizce yok sayılmasını önlemek için sistemi durdurdu."*
//!
//! ### Motorun sistem parametresi listesi kısa — ve bu demo onu bir uzattı
//!
//! `SystemParam` için **beş** uygulama var: `Res<T>`, `ResMut<T>`, `f32` (dt), `Query<Q>` ve
//! `Option<P>`. Sonuncusu ilk yazımda **yoktu**, ve yokluğu bu demoda ölçüldü: en yaygın
//! "hataya dayanıklı parametre" deseni derlenmiyordu. Trait mühürlü olduğu için dışarıdan
//! eklenemiyordu, o yüzden motorun içine kondu (2026-08-23).
//!
//! Mühür duruyor — `Option<P>` onu açmıyor, sadece en çok istenen tek eksiği kapatıyor.
//! Genel uzatılabilirlik hâlâ açık soru; bkz. `docs/API_DEPTH.md`.
//!
//! **Ve `Option<P>` her hatayı yutmuyor.** Yalnız *yokluk* `None` oluyor; ödünç alma
//! çakışması yine panikliyor, çünkü o bir durum değil hata: aynı parametre listesinde iki
//! `ResMut<T>`, ya da yazma kilidini tutan bir sistemin içinden `get_resource`. Onu `None`'a
//! çevirmek, bir çizelgeleme hatasını sıradan yokluk denetimi gibi göstermek olurdu.
//!
//! ## Ölçüldü: politikanın gerçekten bu olduğu
//!
//! Yazıp geçmek yerine deneniyor: kaydı olmayan bir kaynağı `Res<T>` ile isteyen bir sistem
//! çalıştırılıyor, panik yakalanıyor ve mesajı okunuyor.
//!
//! Ölçüldü (2026-08-23):
//!
//! Aynı eksik kaynak, üç farklı biçim, tek kare (kare 120):
//!
//! | biçim | sonuç |
//! |-------|-------|
//! | `Res<T>` | **panikledi** — `❌ FATAL ECS ERROR ❌` |
//! | `run_if` ile korunmuş `Res<T>` | **0** kez koştu — sistem hiç girmedi |
//! | **`Option<Res<T>>`** | **90** kez koştu, **90** kez `None` gördü |
//! | iş mantığı hatası, kaynağa yazılıyor | 2 kayıt, uygulama çalışmaya devam etti |
//!
//! Üç satır üç ayrı davranış, ve ortadaki ikisinin farkı önemli: `run_if` sisteme **hiç
//! girmiyor**, yani yedek dal yazacak yer yok. `Option<Res<T>>` giriyor ve yokluğu kendisi ele
//! alıyor — 90 karenin 90'ında.
//!
//! Politika hâlâ "panikle", ve öyle kalmalı: koruma varsayılan değil, siz koyacaksınız. Değişen
//! şey, koruma koymanın artık iki biçimi olması.
//!
//! ## Karşılığı ne
//!
//! `Result` döndüren tek bir sistemin işi Gizmo'da üç parçaya bölünüyor:
//!
//! 1. **Beklenen yokluk** -> `run_if` ile sistemi hiç koşturmamak. Panik yok, ama sistem de
//!    çalışmıyor — "eksikse şunu yap" yazılamıyor, ancak "eksikse hiç girme" yazılabiliyor.
//! 2. **Beklenmeyen yokluk** -> hiçbir şey yapmayın; motor zaten paniklesin. Sessiz atlamak
//!    hatayı gizler.
//! 3. **İş mantığı hatası** -> bir kaynağa yazıp başka bir sistemde okuyun. Demo bunu yapıyor:
//!    `ErrorLog` kaynağı, hataların toplandığı yer.
//!
//! ## Kontroller
//!   * **H** — iş mantığı hatası üret (kaynağa yazılır, uygulama durmaz)
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Hiçbir zaman dünyaya eklenmeyen kaynak — panik denemesi için.
#[derive(Clone, Copy)]
struct NeverRegistered;
gizmo::core::impl_component!(NeverRegistered);

/// Sistemin döndüremediği `Result`'ın yerine geçen defter.
#[derive(Default, Clone)]
struct ErrorLog {
    /// Yakalanan panik mesajının ilk satırı — politikanın kanıtı.
    fatal_probe: String,
    /// İş mantığı hataları: sistem `Err` döndüremediği için buraya yazılıyor.
    entries: Vec<String>,
    frame: u32,
    /// Korumalı sistemin kaç kez koştuğu — kaynak yok, yani 0 kalmalı.
    guarded_runs: u32,
    /// `Option<Res<..>>` alan sistemin kaç kez koştuğu — kaynak yok ama sistem giriyor.
    optional_runs: u32,
    /// O sistemin kaç kez `None` gördüğü.
    optional_none: u32,
}
gizmo::core::impl_component!(ErrorLog);

/// Sağlığı olan bir varlık — iş mantığı hatasının kaynağı.
#[derive(Clone, Copy)]
struct Health(f32);
gizmo::core::impl_component!(Health);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Error Handling", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            for i in 0..4 {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new((i as f32 - 1.5) * 1.6, 0.0, 0.0))
                        .with_scale(Vec3::splat(0.55)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.85, 0.40, 0.35, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Health(100.0 - i as f32 * 30.0),
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

            // Politikayı ölç: eksik kaynak gerçekten paniklendiriyor mu.
            let fatal_probe = probe_missing_resource(scene.world);
            gizmo::gizmo_log!(Info, "eksik kaynak denemesi -> {}", fatal_probe);

            scene.world.insert_resource(ErrorLog {
                fatal_probe,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 8.0), Vec3::ZERO);
        })
        .add_update_system(count_frames.in_phase(Phase::PreUpdate).label("frames"))
        // `run_if` yolu: sistem hiç koşmuyor. Artık tek yol DEĞİL — `Option<Res<T>>` de var,
        // ve ikisi farklı şeyler: bu, sisteme hiç girmiyor; öteki giriyor ve yedek dal yazabiliyor.
        .add_update_system(
            guarded
                .in_phase(Phase::Update)
                .after("frames")
                .run_if(|world: &World| world.get_resource::<NeverRegistered>().is_some()),
        )
        // Öteki yol: sistem KOŞUYOR ve yokluğu kendisi ele alıyor.
        .add_update_system(tolerant.in_phase(Phase::Update).after("frames"))
        .add_update_system(business_logic.in_phase(Phase::Update))
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<ErrorLog>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("err".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(500.0);
                        ui.heading("Hata yönetimi");
                        ui.label("sistem Result DÖNDÜREMİYOR · ? yok · hata işleyici yok");
                        ui.separator();
                        ui.label("motorun politikası, ölçüldü:");
                        ui.monospace(format!("  {}", r.fatal_probe));
                        ui.separator();
                        ui.label(format!(
                            "kare {} · korumalı sistem {} kez koştu",
                            r.frame, r.guarded_runs
                        ));
                        ui.label("(kaynak hiç eklenmedi -> run_if hep false -> 0)");
                        ui.label(format!(
                            "Option<Res<..>> alan sistem {} kez koştu · {} kez None gördü",
                            r.optional_runs, r.optional_none
                        ));
                        ui.separator();
                        ui.label(format!("iş mantığı hataları ({}):", r.entries.len()));
                        for line in r.entries.iter().rev().take(5) {
                            ui.monospace(format!("  {line}"));
                        }
                        ui.separator();
                        ui.label("H — hata üret (uygulama durmaz)");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Eksik kaynağın gerçekten panikle bittiğini **deneyerek** doğrular.
///
/// Panik motorun kendi koyduğu, belgelenmiş bir koruma — bozulmuş bir durum değil, o yüzden
/// yakalamak meşru. Kanca ölçüm süresince susturuluyor ki günlük kirlenmesin.
fn probe_missing_resource(world: &mut World) -> String {
    use gizmo::core::system::IntoSystem;
    let mut system = (|_missing: Res<NeverRegistered>| {}).into_system();

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        system.run(world, 0.016);
    }));
    std::panic::set_hook(previous);

    match outcome {
        Ok(()) => "PANİKLEMEDİ — sistem eksik kaynakla koştu".to_string(),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            let first = message
                .lines()
                .find(|l| l.contains("FATAL") || l.contains("bulunamadı"))
                .unwrap_or("panikledi")
                .trim()
                .to_string();
            format!("panikledi: {first}")
        }
    }
}

/// `Option<Res<T>>` yolu: kaynak yokken de koşuyor, ve yedek dalını yazabiliyor.
///
/// `run_if`'ten farkı tam burada: orada sistem hiç girmiyor, burada giriyor ve `None`'ı görüyor.
fn tolerant(missing: Option<Res<NeverRegistered>>, mut log: ResMut<ErrorLog>) {
    log.optional_runs += 1;
    if missing.is_none() {
        log.optional_none += 1;
    }
}

/// Kareyi sayar — koşulsuz.
fn count_frames(mut log: ResMut<ErrorLog>) {
    log.frame += 1;
}

/// Eksik kaynağa **dayanıklı** yol: `run_if` kaynağın varlığını soruyor, yoksa sistem hiç
/// çalışmıyor — yedek davranış yazacak bir yer yok. `Option<Res<T>>` tam bunun için var.
///
/// Dikkat: bu, "eksikse şunu yap" demek DEĞİL — sistem hiç girmiyor, yani yedek davranış
/// yazılacak bir yer yok. `Result` döndüren bir sistem orada bir `else` dalı yazabilirdi.
fn guarded(_present: Res<NeverRegistered>, mut log: ResMut<ErrorLog>) {
    log.guarded_runs += 1;
}

/// İş mantığı hatası: sistem `Err` döndüremediği için hatayı bir kaynağa yazıyor.
fn business_logic(
    health: gizmo::core::query::Query<&Health>,
    mut log: ResMut<ErrorLog>,
    input: Res<Input>,
) {
    let want = input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyH as u32)
        || (log.frame == 90 && std::env::var("GIZMO_ERROR_SELFTEST").is_ok());
    if !want {
        return;
    }
    let frame = log.frame;
    for (id, h) in health.iter() {
        if h.0 < 50.0 {
            // `Result` döndürülebilseydi burada `Err(...)` dönerdi ve `?` ile yukarı taşınırdı.
            log.entries
                .push(format!("kare {frame}: varlık {id} sağlığı kritik ({:.0})", h.0));
        }
    }
    gizmo::gizmo_log!(
        Info,
        "iş mantığı hatası üretildi · defterde {} kayıt · korumalı sistem {} kez koştu · Option'lı sistem {} kez koştu ({} kez None) · uygulama ÇALIŞMAYA DEVAM EDİYOR",
        log.entries.len(),
        log.guarded_runs,
        log.optional_runs,
        log.optional_none
    );
}
