//! # Bevy'nin dört ECS genişletme örneğinin Gizmo Engine portu
//!
//! Bu demo **dört** Bevy örneğini birden karşılıyor, çünkü dördünün de cevabı aynı yerden
//! geliyor: `ecs/system_param`, `ecs/custom_query_param`, `ecs/system_piping`,
//! `ecs/one_shot_systems`.
//!
//! Dördü de motorun **genişletme noktalarını** kullanıyor. Gizmo'da o noktalar kapalı.
//!
//! ## Ölçülen: dört trait de mühürlü, iki API de yok
//!
//! | Bevy örneği | Bevy'nin kullandığı | Gizmo |
//! |-------------|---------------------|-------|
//! | `system_param` | `impl SystemParam for MyParam` | `SystemParam: sealed::Sealed` — **uygulanamaz** |
//! | `custom_query_param` | `impl WorldQuery` / `QueryData` | `WorldQuery: sealed::SealedQuery`, `ReadOnlyQuery: sealed::SealedReadOnly`, `FetchComponent: sealed::SealedFetch` — **üçü de mühürlü** |
//! | `system_piping` | `a.pipe(b)` | `pipe` diye bir şey **yok** |
//! | `one_shot_systems` | `commands.run_system(id)` | `run_system` **yok**; sistemler yalnız çizelgeden koşar |
//!
//! Mühür kasıtlı bir tasarım: bir sorgu operandının ya da sistem parametresinin ne eriştiğini
//! çizelge **statik olarak** bilmek zorunda, yoksa paralel çalıştırma güvenli olmaz. Dışarıdan
//! yazılmış bir `SystemParam` bu bilgiyi veremezdi. Ama sonucu şu: Bevy'nin bu dört örneğinin
//! hiçbirinin doğrudan karşılığı yok.
//!
//! ## Karşılıkları ne
//!
//! **`SystemParam` yerine — kaynak demeti.** Bir "parametre" gibi davranan şeyi bir kaynağa
//! koyup `ResMut<T>` almak. Kaybedilen şey kapsülleme: `MyParam`'ın içindeki üç sorgu, üç ayrı
//! parametre olarak sistemin imzasında görünmek zorunda.
//!
//! **`WorldQuery` yerine — demet + filtre.** `(&A, Mut<B>, With<C>, Without<D>)` epeyce yol
//! alıyor; alınamayan şey *türetilmiş* bir operand (örneğin "iki bileşenden hesaplanan tek bir
//! görünüm").
//!
//! **`pipe` yerine — düz fonksiyon.** Boru hattının yaptığı iş "birinin çıktısı ötekinin girdisi";
//! bu, sistemi ikiye bölmeden düz bir Rust fonksiyonu çağırmakla aynı. Kaybedilen şey iki parçanın
//! ayrı ayrı çizelgelenebilmesi.
//!
//! **`run_system` yerine — `run_if` + mandal.** Bir sistemi "bir kez" koşturmak, onu bir bayrağa
//! bağlayıp bayrağı içinden indirmek demek. Demo bunu ölçüyor: mandallı sistem tam **bir kez**
//! koşuyor.
//!
//! ## Ölçüldü
//!
//! Ölçüldü (2026-08-23, 6 küp, kare 120):
//!
//! | vekil | beklenen | ölçülen |
//! |-------|----------|---------|
//! | `With<Spinner> + Without<Frozen>` | 3 | **3** |
//! | `With<Frozen>` | 3 | **3** |
//! | düz fonksiyon (`pipe` yerine) | (−4,0 + −0,8 + 2,4)/3 = −0,800 | **−0,800** |
//! | mandallı sistem, 120 karede koşu sayısı | 1 | **1** |
//! | mandalın son hâli | inik | **inik** |
//!
//! Yani dört vekilin dördü de işi görüyor. Kaybedilen şey davranış değil, **yapı**: kapsülleme
//! (`SystemParam`), türetilmiş operand (`WorldQuery`), ayrı çizelgelenebilirlik (`pipe`) ve
//! "çizelgeye hiç girmemiş bir sistemi çağırabilmek" (`run_system`).
//!
//! ## Kontroller
//!   * **Boşluk** — tek seferlik sistemi yeniden tetikle
//!   * **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Query, With, Without};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// İşaretler — "türetilmiş sorgu operandı" yerine düz demet + filtre kullanmak için.
#[derive(Clone, Copy)]
struct Spinner;
gizmo::core::impl_component!(Spinner);
#[derive(Clone, Copy)]
struct Frozen;
gizmo::core::impl_component!(Frozen);

/// `SystemParam` yerine geçen kaynak demeti: bir arada duran ama parametre olamayan durum.
#[derive(Default, Clone, Copy)]
struct Counters {
    /// Filtreli sorgunun eşlediği varlık sayısı.
    spinning: usize,
    frozen: usize,
    /// Boru hattının yerine geçen düz fonksiyonun sonucu.
    piped: f32,
    /// Tek seferlik sistemin kaç kez koştuğu — beklenen 1.
    once_ran: u32,
    /// Mandal: `true` iken tek seferlik sistem koşar.
    armed: bool,
    frame: u32,
    logged: bool,
}
gizmo::core::impl_component!(Counters);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy ECS Extension Points", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let device = &scene.renderer.device;
            let cube = AssetManager::create_cube(device);

            // Altı küp: üçü dönen, üçü donmuş. Filtreli sorgu ikisini ayıracak.
            for i in 0..6 {
                let x = (i as f32 - 2.5) * 1.6;
                let frozen = i % 2 == 1;
                let entity = scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.0, 0.0)).with_scale(Vec3::splat(0.5)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(
                        if frozen {
                            Vec4::new(0.35, 0.55, 0.9, 1.0)
                        } else {
                            Vec4::new(0.9, 0.6, 0.3, 1.0)
                        },
                        0.5,
                        0.0,
                    ),
                    MeshRenderer::new(),
                    Spinner,
                ));
                if frozen {
                    scene.world.add_component(entity, Frozen);
                }
            }
            scene.world.spawn_bundle((
                Transform::new(Vec3::new(0.0, -1.4, 0.0)),
                GlobalTransform::default(),
                AssetManager::create_plane(device, 20.0),
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.6) * Quat::from_rotation_x(-0.6),
                intensity: 2.4,
                ..Default::default()
            });

            scene.world.insert_resource(Counters {
                armed: true,
                ..Default::default()
            });
            scene.spawn_camera(state, Vec3::new(0.0, 2.5, 9.0), Vec3::ZERO);
        })
        .add_update_system(filtered.in_phase(Phase::Update).label("filtered"))
        // `run_system` yerine: mandala bağlı bir sistem, ve mandalı kendi indiriyor.
        .add_update_system(
            run_once
                .in_phase(Phase::Update)
                .after("filtered")
                .run_if(|world: &gizmo::core::World| {
                    world.get_resource::<Counters>().map(|c| c.armed).unwrap_or(false)
                }),
        )
        .set_ui(|world, _state, ctx| {
            let Some(c) = world.get_resource::<Counters>().map(|c| *c) else {
                return;
            };
            gizmo::egui::Area::new("ext".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    gizmo::egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(470.0);
                        ui.heading("ECS genişletme noktaları");
                        ui.label("dört Bevy örneğinin cevabı aynı yerden geliyor");
                        ui.separator();
                        ui.label("SystemParam · WorldQuery · ReadOnlyQuery · FetchComponent");
                        ui.label("  DÖRDÜ DE MÜHÜRLÜ — dışarıdan uygulanamaz");
                        ui.label("pipe yok · run_system yok");
                        ui.separator();
                        ui.label(format!(
                            "filtreli sorgu: {} dönen · {} donmuş",
                            c.spinning, c.frozen
                        ));
                        ui.label(format!("düz fonksiyon (pipe yerine): {:.3}", c.piped));
                        ui.label(format!("tek seferlik sistem {} kez koştu", c.once_ran));
                        ui.label(format!("mandal: {}", if c.armed { "kurulu" } else { "inik" }));
                        ui.separator();
                        ui.label("boşluk — yeniden tetikle");
                    });
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// `WorldQuery` yerine: demet + `With`/`Without`.
///
/// Alınabilen şey bu; alınamayan şey **türetilmiş** bir operand — iki bileşenden hesaplanan tek
/// bir görünüm yazılamıyor, çünkü `FetchComponent` mühürlü.
fn filtered(
    spinning: Query<(&Transform, With<Spinner>, Without<Frozen>)>,
    frozen: Query<(&Transform, With<Frozen>)>,
    mut counters: ResMut<Counters>,
    input: Res<Input>,
    time: Res<Time>,
) {
    counters.frame += 1;
    counters.spinning = spinning.iter().count();
    counters.frozen = frozen.iter().count();

    // `pipe` yerine: birinin çıktısını ötekinin girdisi yapmak için düz bir fonksiyon çağrısı.
    // Kaybedilen şey iki parçanın ayrı ayrı çizelgelenebilmesi, sonucun kendisi değil.
    let total: f32 = spinning
        .iter()
        .map(|(_, (t, _, _))| t.position.x)
        .sum();
    counters.piped = normalise(total, counters.spinning);

    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32) {
        counters.armed = true;
    }
    let _ = time;

    // Ölçümün ikinci yarısı: tek seferlik sistem koştuktan SONRA da bir daha koşmuyor mu.
    if counters.frame == 120 {
        gizmo::gizmo_log!(
            Info,
            "kare 120 · dönen {} · donmuş {} · pipe-yerine {:.3} · tek-seferlik toplam koşu {} · mandal {}",
            counters.spinning,
            counters.frozen,
            counters.piped,
            counters.once_ran,
            counters.armed
        );
    }
}

/// Boru hattının ikinci ucu — Bevy'de `pipe`'ın sağ tarafı, burada düz bir fonksiyon.
fn normalise(total: f32, count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

/// `run_system` yerine: mandala bağlı bir sistem, mandalı kendi indiriyor.
fn run_once(mut counters: ResMut<Counters>) {
    counters.once_ran += 1;
    counters.armed = false;
    if !counters.logged {
        counters.logged = true;
        gizmo::gizmo_log!(
            Info,
            "tek seferlik sistem {}. karede koştu · toplam koşu {}",
            counters.frame,
            counters.once_ran
        );
    }
}
