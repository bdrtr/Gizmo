//! # Bevy'nin `ecs/fallible_params` örneğinin Gizmo Engine portu
//!
//! Bir sistem, istediği şey henüz **ortada yokken** ne yapmalı? Bevy'nin cevabı sessiz atlamak:
//! `Single<…>` ya da `Res<T>` çözülemezse sistem o kare hiç koşmuyor ve uygulama sürüyor.
//!
//! ## Gizmo'nun cevabı farklı: **panik**
//!
//! Motorun sistem çağrı yolu parametreyi çözemezse programı durduruyor —
//! `into_system.rs`'deki mesaj birebir şöyle: *"❌ FATAL ECS ERROR ❌ … Gizmo Engine, hataların
//! sessizce yok sayılmasını önlemek için sistemi durdurdu."*
//!
//! Bu bir kaza değil, **tercih**: eksik bir kaynak sessizce atlanırsa sistem hiç koşmaz ve kimse
//! fark etmez. Ama Bevy'nin kipini isteyen bir oyun için karşılığı ne?
//!
//! `SystemParam::fetch` aslında bir `Result` döndürüyor; paniğe çeviren yer çağrı sarmalayıcısı.
//! Yani hata yolu var, yalnız yumuşak bir varyantı (`Option<Res<T>>` gibi) yok.
//!
//! ## Motorun verdiği kapı: `run_if`
//!
//! [`SystemConfig::run_if`]'in belgesi bunu açıkça söylüyor: *"Koşul `false` iken sistemin gövdesi
//! **ve parametre çekimi** atlanır — yani bu şekilde korunan bir sistem, koruma `false` kaldığı
//! sürece eksik bir `Res<T>` yüzünden paniklemez."* Kapanış `&World` aldığı için "kaynak var mı"
//! doğrudan sorulabiliyor.
//!
//! Bedeli de belgede: kapanış biçimi çizelgeye opak olduğu için sistem **dışlayıcı** işaretleniyor
//! (kendi partisinde yalnız koşuyor). Bevy'nin fallible parametresinin böyle bir bedeli yok.
//!
//! ## Ölçülen
//!
//! Demo iki sistem çalıştırıyor, ikisi de var olmayan bir kaynağı isteyen:
//!
//!   * **korumalı** — `run_if(kaynak var mı)` ile sarılmış. Kaynak 2. saniyede doğuyor.
//!   * **korumasız** — `GIZMO_FALLIBLE_PANIC=1` verilmedikçe hiç kaydedilmiyor, çünkü kaydedilirse
//!     ilk karede programı öldürüyor. Bu, demonun kanıtladığı şeyin ta kendisi.
//!
//! Ölçüldü (2026-08-23):
//!
//! | koşu | çıkış kodu | sonuç |
//! |------|------------|-------|
//! | korumalı (varsayılan) | **0** | kaynak 2,00 s'de doğdu, korumalı sistem ilk kez **2,01 s**'de koştu |
//! | `GIZMO_FALLIBLE_PANIC=1` | **101** | ilk karede öldü |
//!
//! İki şey birden görünüyor. Koruma **çalışıyor**: kaynak yokken sistem hiç çağrılmıyor ve
//! panik yok; kaynak doğduktan **bir kare sonra** koşmaya başlıyor (2,00 → 2,01 s). Korumasız
//! aynı sistem ise programı ilk karede bitiriyor.
//!
//! **Ve bir yan bulgu:** panik mesajı `stderr`'e gitmiyor. Motorun panik kancası onu
//! `tracing::error!` ile yazıp bir hata diyaloğu açıyor; bu koşuda yakalanan günlük **boştu** ve
//! elde yalnız çıkış kodu kaldı. Başsız bir CI koşusunda bir Gizmo çökmesi, sebebini söylemeden
//! yalnız `101` olarak görünebilir.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// 2. saniyede doğan kaynak — korumalı sistemin beklediği şey.
#[derive(Clone, Copy)]
struct LateArrival {
    born_at: f32,
}
gizmo::core::impl_component!(LateArrival);

/// Demonun defteri.
#[derive(Default, Clone, Copy)]
struct Ledger {
    elapsed: f32,
    /// Kaynak doğdu mu, ve ne zaman.
    spawned_at: Option<f32>,
    /// Korumalı sistem kaç kez koştu.
    guarded_runs: u32,
    /// İlk koşusunun zamanı.
    first_run_at: Option<f32>,
}
gizmo::core::impl_component!(Ledger);

/// Kaynağın doğacağı an.
const BIRTH: f32 = 2.0;

fn main() {
    let suicidal = std::env::var("GIZMO_FALLIBLE_PANIC").is_ok();

    let mut app = App::<SimpleSceneState>::new("Gizmo Engine - Bevy Fallible Params", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_sphere(device, 1.0, 20, 32),
                Material::new(white.clone()).with_pbr(Vec4::new(0.3, 0.6, 0.9, 1.0), 0.5, 0.0),
                MeshRenderer::new(),
                Spin::new(Vec3::Y, 0.8),
            ));
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.6, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 14.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.7),
                intensity: 2.4,
                ..Default::default()
            });
            scene.world.insert_resource(Ledger::default());
            scene.spawn_camera(state, Vec3::new(0.0, 2.0, 6.0), Vec3::ZERO);
        })
        .add_plugin(SpinPlugin)
        .add_update_system(tick.in_phase(Phase::Update).label("tick"))
        // Korumalı sistem: kaynak yokken parametre ÇEKİMİ de atlanıyor, o yüzden paniklemiyor.
        .add_update_system(
            guarded
                .in_phase(Phase::Update)
                .after("tick")
                .run_if(|world: &gizmo::core::World| world.get_resource::<LateArrival>().is_some()),
        );

    // Korumasız sistem yalnız açıkça istenirse kaydediliyor: kaydedilirse ilk karede öldürüyor.
    if suicidal {
        app = app.add_update_system(unguarded.in_phase(Phase::Update).after("tick"));
    }

    app.set_render(|world, _state, encoder, view, renderer, _light_time| {
        renderer.gpu_physics = None;
        renderer.gpu_fluid = None;
        renderer.gpu_particles = None;
        renderer.ssr = None;
        renderer.ssgi = None;

        // Kaynağı doğurmak yapısal bir değişiklik.
        let born = world
            .get_resource::<Ledger>()
            .map(|l| (l.elapsed, l.spawned_at.is_none()))
            .unwrap_or((0.0, false));
        if born.1 && born.0 >= BIRTH {
            world.insert_resource(LateArrival { born_at: born.0 });
            if let Some(mut ledger) = world.get_resource_mut::<Ledger>() {
                ledger.spawned_at = Some(born.0);
            }
            gizmo::gizmo_log!(Info, "kaynak {:.2} s'de doğdu", born.0);
        }

        gizmo::systems::default_render_pass(world, encoder, view, renderer);
    })
    .set_ui(move |world, _state, ctx| {
        let Some(l) = world.get_resource::<Ledger>().map(|l| *l) else {
            return;
        };
        gizmo::egui::Area::new("fallible".into())
            .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
            .show(ctx, |ui| {
                gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(460.0);
                    ui.heading("Çözülemeyen parametre");
                    ui.label(format!("geçen süre: {:.2} s", l.elapsed));
                    ui.label(match l.spawned_at {
                        Some(t) => format!("kaynak {t:.2} s'de doğdu"),
                        None => format!("kaynak henüz yok ({BIRTH:.1} s'de doğacak)"),
                    });
                    ui.separator();
                    ui.label(format!("korumalı sistem {} kez koştu", l.guarded_runs));
                    ui.label(match l.first_run_at {
                        Some(t) => format!("  ilk koşusu: {t:.2} s"),
                        None => "  henüz hiç koşmadı".to_string(),
                    });
                    ui.separator();
                    ui.label("motor eksik parametrede PANİKLER — Bevy sistemi atlar.");
                    ui.label("run_if koşulu false iken parametre ÇEKİMİ de atlanıyor,");
                    ui.label("motorun verdiği kapı bu. Bedeli: sistem dışlayıcı olur.");
                    ui.separator();
                    ui.label(if suicidal {
                        "GIZMO_FALLIBLE_PANIC=1 — korumasız sistem kayıtlı"
                    } else {
                        "GIZMO_FALLIBLE_PANIC=1 korumasız sistemi de kaydeder (panikler)"
                    });
                });
            });
    })
    .run()
    .expect("uygulama çalıştırılamadı");
}

/// Süreyi biriktirir.
fn tick(mut ledger: ResMut<Ledger>, time: Res<Time>) {
    ledger.elapsed += time.dt();
}

/// Korumalı sistem: `run_if` sayesinde kaynak doğana kadar hiç çağrılmıyor.
fn guarded(arrival: Res<LateArrival>, mut ledger: ResMut<Ledger>) {
    ledger.guarded_runs += 1;
    if ledger.first_run_at.is_none() {
        let at = ledger.elapsed;
        ledger.first_run_at = Some(at);
        gizmo::gizmo_log!(
            Info,
            "korumalı sistem ilk kez {:.2} s'de koştu (kaynak {:.2} s'de doğmuştu)",
            at,
            arrival.born_at
        );
    }
}

/// Korumasız sistem: aynı kaynağı korumasız istiyor, yani ilk karede programı öldürüyor.
fn unguarded(_arrival: Res<LateArrival>) {
    gizmo::gizmo_log!(Info, "buraya asla ulaşılmaz");
}
