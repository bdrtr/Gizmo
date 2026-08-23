//! # Sistem biçimleri
//!
//! Bu demo tek bir soruyu üç yandan soruyor — *bir sistem ne olabilir ve ne zaman koşar*:
//! başlangıç sistemi, kapatma (closure) sistemi, jenerik sistem.
//!
//! ## Motorun cevabı
//!
//! | yetenek | Gizmo |
//! |---------|-------|
//! | bir kez, her şeyden önce koşan sistem | **`Startup` fazı yok** — ama `Phase::User(500)` artık `PreUpdate`'ten öncesini adlandırabiliyor |
//! | kapatmanın sisteme dönüşmesi | **aynen çalışıyor** — `IntoSystem` her `FnMut` için uygulanmış |
//! | aynı sistemin tür başına çizelgeye eklenmesi | **aynen çalışıyor** — düz Rust jenerikleri |
//! | birçok parametreyi tek yapıda toplamak | **var** (2026-08-23) — `system_param!` |
//!
//! Dördünden üçü karşılanıyor. `Startup` hâlâ yok: `Phase::User(500)` "her şeyden önce" diyebiliyor
//! ama "yalnız bir kez" diyemiyor — o mandalın işi, ve aşağıda ölçülü.
//!
//! ## Kompozit parametre: mühür ve neden orada olduğu
//!
//! Altı kaynak isteyen bir sistem altı parametre alıyordu, ve aynı altısını isteyen her sistem
//! onları tekrar yazıyordu. Bir yapıda toplamak mümkün değildi çünkü `SystemParam` **mühürlü**.
//!
//! Mührün sebebi `fetch` değil, [`get_access_info`]. Eksik bildiren bir erişim beyanı hata
//! vermiyor — çizelge, aynı kaynağa yazan iki sistemi yan yana koşturuyor, ve bozulmanın hangisinde
//! olacağı rayon'un o karede hangisine önce ulaştığına bağlı.
//!
//! `system_param!` o beyanı **alan alan iletiyor**, yani elle yazılmadığı için sapamıyor. Motorun
//! `the_scheduler_separates_a_composite_from_a_conflicting_writer` testi bunu çizelge seviyesinde
//! kilitliyor: kompozitten `ResMut<Score>` yazan bir sistem ile düz `ResMut<Score>` yazan bir sistem
//! **iki ayrı yığına** düşmeli. İletim kaldırılınca test kırmızıya dönüyor — "aynı kaynağa yazan iki
//! sistem tek yığına kondu".
//!
//! Mühür duruyor ve anlamı değişmedi: `Sealed`'i elle uygulamak hâlâ derleniyor, ve hâlâ
//! `get_access_info`'yu elle yazmak demek.
//!
//! ## Başlangıç sistemi: iki vekil, ve ikisi aynı şey değil
//!
//! Motorda "bir kez, her şeyden önce" bir faz yok. İki karşılık var ve **farklı zamanlarda**
//! koşuyorlar:
//!
//! 1. **Kurulum kapatması** (`with_simple_scene`) — çizelge hiç dönmeden, ilk kareden **önce**.
//!    Buna en yakın olan bu, ve sistem değil: `&mut Scene` alıyor, sorgu değil.
//! 2. **Mandallı sistem** (`run_if` + bir bayrak) — ilk karede, ama çizelgenin **içinde**, yani
//!    o karenin öteki sistemleriyle aynı fazda.
//!
//! Fark ölçülebilir ve ölçülüyor: kurulum kapatması kare 0'da, mandallı sistem kare 1'de.
//!
//! ## Ölçüldü
//!
//! Ölçüldü (2026-08-23, kare 90):
//!
//! | biçim | ölçülen | beklenen |
//! |-------|---------|----------|
//! | kurulum kapatması | kare **0** (çizelge dönmeden) | — |
//! | mandallı "başlangıç" sistemi | kare **1**, **1** kez | 1 kez |
//! | kapatma-sistem, koşu sayısı | **89** | kare 90'da 89 |
//! | kapatmanın taşıdığı değer | **96** karakter | `"merhaba"` (7) + 89 = 96 |
//! | jenerik `report_pool::<Health>` | 3 varlık, toplam **240** | 100+80+60 |
//! | kompozit parametre, koşu sayısı | **89** | kapatmayla aynı |
//! | kompozitin tek imzadan okuduğu | **health 240 · mana 120** | jeneriklerin sayılarıyla birebir |
//! | jenerik `report_pool::<Mana>` | 2 varlık, toplam **120** | 50+70 |
//!
//! Üç şey okunuyor:
//!
//! **Kapatma gerçekten sistem oluyor**, ve `move` ile taşıdığı değeri çağrılar arasında
//! saklıyor. 7 karakterle başlayıp 89 koşuda 96'ya çıkması bunun kanıtı; durum saklanmasaydı
//! her karede 8'de kalırdı.
//!
//! **Jenerik sistem hiçbir motor desteği istemiyor.** `report_pool::<Health>` ve
//! `report_pool::<Mana>` iki ayrı sistem olarak kaydediliyor ve ikisi de doğru sayıyor. Kural
//! şu: her tür için çizelgeye ayrı bir kopya koymak gerekiyor.
//!
//! **Ama "başlangıç" iki farklı zaman.** Kurulum kapatması kare 0'da (çizelge hiç dönmeden),
//! mandallı sistem kare 1'de. İkisinin de yapamadığı bir şey var: *sorgu alan, ama çizelgenin ilk
//! turundan önce koşan* bir sistem. Gizmo'da böyle bir şey yazılamıyor.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::query::Query;
use gizmo::core::system::{IntoSystemConfig, Phase, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Jenerik sistemin üstünde çalışacağı iki ayrı bileşen — ikisi ayrı tür, çünkü mesele tam
/// olarak "aynı mantık, farklı tür".
#[derive(Clone, Copy)]
struct Health(f32);
gizmo::core::impl_component!(Health);
#[derive(Clone, Copy)]
struct Mana(f32);
gizmo::core::impl_component!(Mana);

/// Jenerik sistemin okuyabilmesi için ortak arayüz.
trait Pool: gizmo::core::Component {
    const NAME: &'static str;
    fn value(&self) -> f32;
}
impl Pool for Health {
    const NAME: &'static str = "can";
    fn value(&self) -> f32 {
        self.0
    }
}
impl Pool for Mana {
    const NAME: &'static str = "mana";
    fn value(&self) -> f32 {
        self.0
    }
}

gizmo::core::system_param! {
    /// Dört parametre yerine bir tane.
    ///
    /// `SystemParam` mühürlü, ve mührün sebebi `get_access_info`: eksik bildiren bir erişim
    /// beyanı hata vermiyor, yalnız çizelgenin çakışan iki sistemi yan yana koşturmasına izin
    /// veriyor. Makro o beyanı alan alan iletiyor, yani elle yazılmadığı için sapamıyor.
    struct Bundle<'w> {
        report: ResMut<FormReport>,
        health: Query<&'static Health>,
        mana: Query<&'static Mana>,
    }
}

/// Kompozit parametreyi tek başına alan sistem. Üç şeyi tek imzadan okuyor.
fn composite_reader(mut b: Bundle) {
    let h: f32 = b.health.iter().map(|(_, x)| x.0).sum();
    let m: f32 = b.mana.iter().map(|(_, x)| x.0).sum();
    b.report.composite_runs += 1;
    b.report.composite_seen = format!("health {h:.0} · mana {m:.0}");
}

/// Ölçüm defteri.
#[derive(Default, Clone)]
struct FormReport {
    frame: u32,
    /// Kurulum kapatmasının koştuğu kare (çizelge dönmeden önce, yani 0).
    setup_frame: i64,
    /// Mandallı "başlangıç" sisteminin koştuğu kare.
    latched_frame: i64,
    /// Mandallı sistemin toplam koşu sayısı — 1 olmalı.
    latched_runs: u32,
    armed: bool,
    /// Kapatma-sistemin koşu sayısı.
    closure_runs: u32,
    /// Kapatmanın kareler arasında sakladığı değer.
    closure_state: String,
    /// Jenerik sistemin tür başına raporu.
    generic: Vec<String>,
    /// Kompozit parametreli sistemin koşu sayısı.
    composite_runs: u32,
    /// O sistemin kompozit üzerinden okuduğu üç değer — dördü de tek parametreden.
    composite_seen: String,
}
gizmo::core::impl_component!(FormReport);

fn main() {
    // Durumlu kapatma: dışarıdaki bir değeri taşıyor ve çağrılar arasında güncelliyor.
    // `move` sayesinde değer kapatmanın içinde yaşıyor.
    let mut carried = String::from("merhaba");
    let stateful_closure = move |mut report: ResMut<FormReport>| {
        carried = format!("{carried}+");
        report.closure_runs += 1;
        report.closure_state = carried.clone();
    };

    App::<SimpleSceneState>::new("Gizmo Engine - System Forms", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            for i in 0..3 {
                let x = (i as f32 - 1.0) * 2.0;
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.6)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        Vec4::new(0.85, 0.45, 0.35, 1.0),
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Health(100.0 - i as f32 * 20.0),
                ));
                if i != 1 {
                    scene.world.add_component(entity, Mana(50.0 + i as f32 * 10.0));
                }
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

            // BU, "her şeyden önce, bir kez"e en yakın şey — ve çizelge hiç dönmeden koşuyor.
            gizmo::gizmo_log!(Info, "kurulum kapatması koştu (çizelgeden ÖNCE)");
            scene.world.insert_resource(FormReport {
                setup_frame: 0,
                latched_frame: -1,
                armed: true,
                closure_state: "-".into(),
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 8.0), Vec3::ZERO);
        })
        .add_update_system(count_frames.in_phase(Phase::PreUpdate).label("frames"))
        // "Başlangıç sistemi"nin vekili: mandala bağlı, mandalı kendi indiriyor.
        .add_update_system(
            latched_startup
                .in_phase(Phase::Update)
                .after("frames")
                .run_if(|world: &gizmo::core::World| {
                    world.get_resource::<FormReport>().map(|r| r.armed).unwrap_or(false)
                }),
        )
        // Kapatma, doğrudan sistem olarak. Motorun `IntoSystem`'i her `FnMut` için uygulanmış.
        .add_update_system(stateful_closure.in_phase(Phase::Update).after("frames"))
        // Aynı jenerik fonksiyon, iki farklı tür için iki ayrı sistem.
        .add_update_system(report_pool::<Health>.in_phase(Phase::PostUpdate))
        .add_update_system(report_pool::<Mana>.in_phase(Phase::PostUpdate))
        // Kompozit parametre: üç şeyi tek imzadan alıyor, ve `ResMut<FormReport>` yazdığı için
        // çizelge onu öteki yazıcılardan ayırıyor — beyan iletildiği için.
        .add_update_system(composite_reader.in_phase(Phase::Render))
        .set_ui(|world, _state, ctx| {
            let Some(r) = world.get_resource::<FormReport>().map(|r| r.clone()) else {
                return;
            };
            gizmo::egui::Area::new("forms".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("Bir sistem ne olabilir, ne zaman koşar");
                        ui.label(format!("kare: {}", r.frame));
                        ui.separator();
                        ui.label("BAŞLANGIÇ — motorda Startup fazı YOK");
                        ui.monospace(format!("  kurulum kapatması : kare {}", r.setup_frame));
                        ui.monospace(format!(
                            "  mandallı sistem   : kare {} · {} kez",
                            r.latched_frame, r.latched_runs
                        ));
                        ui.separator();
                        ui.label("KAPATMA — sistem olarak çalışıyor");
                        ui.monospace(format!("  koşu {} · taşıdığı değer: {}", r.closure_runs, r.closure_state));
                        ui.separator();
                        ui.label("JENERİK — tür başına bir sistem");
                        for line in &r.generic {
                            ui.monospace(format!("  {line}"));
                        }
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Kareyi sayar, ve 90. karede her üç biçimin sonucunu tek satırda basar.
fn count_frames(mut report: ResMut<FormReport>) {
    report.frame += 1;
    if report.frame == 90 {
        gizmo::gizmo_log!(
            Info,
            "kare 90 · kapatma {} kez koştu, taşıdığı değer {} karakter · jenerik: {:?}",
            report.closure_runs,
            report.closure_state.len(),
            report.generic
        );
        gizmo::gizmo_log!(
            Info,
            "kare 90 · kompozit parametre {} kez koştu · okuduğu: {}",
            report.composite_runs,
            report.composite_seen
        );
    }
}

/// "Başlangıç sistemi"nin vekili.
fn latched_startup(mut report: ResMut<FormReport>) {
    report.latched_runs += 1;
    report.latched_frame = report.frame as i64;
    report.armed = false;
    gizmo::gizmo_log!(
        Info,
        "mandallı başlangıç sistemi kare {}'de koştu (çizelgenin İÇİNDE)",
        report.latched_frame
    );
}

/// Jenerik sistem: aynı mantık, tür başına bir örnek. Motorda özel bir destek gerektirmiyor —
/// düz Rust jenerikleri yetiyor.
fn report_pool<T: Pool>(pool: Query<&T>, mut report: ResMut<FormReport>) {
    let mut count = 0usize;
    let mut total = 0.0f32;
    for (_entity, value) in pool.iter() {
        count += 1;
        total += value.value();
    }
    let line = format!("{:<5}: {} varlık · toplam {:.0}", T::NAME, count, total);
    if let Some(slot) = report.generic.iter_mut().find(|l| l.starts_with(T::NAME)) {
        *slot = line;
    } else {
        report.generic.push(line);
        report.generic.sort();
    }
}
