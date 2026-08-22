//! # Bevy'nin `ecs/run_conditions` örneğinin Gizmo Engine portu
//!
//! Bir sistemin **ne zaman** koşacağını, sistemin içine `if` yazmadan söylemek. Gizmo'da bunun
//! adı [`SystemConfig::run_if`]: bir koşul kapanışı alır, `false` döndüğü karelerde sistem hiç
//! çağrılmaz.
//!
//! Sahnede üç küp var, üçü de bir sistemle sürülüyor ve her sistemin kendi kapısı:
//!
//!   * **sol küp** — sayaç arttıkça büyür; kapısı "Space'e basıldı **ya da** sol tık" (VEYA).
//!   * **orta küp** — yalnız sayaç *değiştiği* karelerde döner (VE zinciri: kaynak var **ve**
//!     değeri arttı).
//!   * **sağ küp** — yalnız 2. ile 4. saniye arasında görünür (`time_passed(2)` **ve**
//!     `not(time_passed(4))`).
//!
//! ## Motorun üç kuralı, ve ikisi Bevy'den farklı
//!
//! **1. Tekrarlanan `run_if` DEĞİŞTİRMEZ, İÇ İÇE GEÇER.** Her çağrı bir öncekini sarar, en
//! dıştaki önce değerlendirilir ve hepsinin geçmesi gerekir — yani art arda yazılan koşullar
//! **VE**'lenir. Bevy'de de `.run_if` zincirlemek AND'dir, ama Bevy ayrıca `.or_else()` /
//! `.and_then()` birleştiricileri veriyor.
//!
//! **2. VEYA için birleştirici YOK.** Gizmo'da `or_else` yok; iki koşulun VEYA'sı isteniyorsa
//! **tek bir kapanışın içinde** yazılır. Demo bunu [`either`] yardımcısıyla gösteriyor — üç
//! satır, ve motorun eksiği olarak duruyor.
//!
//! **3. Kapanış biçiminin bir bedeli var, ve motor onu yazıyor.** `run_if(|world| …)` koşulun
//! neye dokunduğunu zamanlayıcıya söyleyemez, bu yüzden sarmalanan sistem **exclusive** olur:
//! kendi grubunda tek başına koşar. Paralel kalması istenen bir koşul için tipli yol var
//! (`SystemExtRunIf::run_if_sys` / `IntoCondition`), o erişimi çıkarabiliyor. Bu demo okunurluk
//! için kapanış biçimini kullanıyor ve bedeli burada söylüyor.
//!
//! ## Kontroller
//!   * **Space** ya da **sol tık** — sayacı artır · **Sağ-tık + fare / WASDQE** — kamera

use gizmo::core::input::Input;
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::core::World;
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Girdi sayacı — Bevy'nin `InputCounter`'ı.
#[derive(Default, Clone, Copy)]
struct InputCounter {
    value: usize,
    /// Sayacın en son arttığı kare (koşullar bunu okuyor).
    changed_frame: u64,
}
gizmo::core::impl_component!(InputCounter);

/// Hangi sistemin bu karede koştuğunu HUD'a taşıyan defter.
#[derive(Default, Clone, Copy)]
struct Ran {
    grow: u64,
    spin: u64,
    window: u64,
}
gizmo::core::impl_component!(Ran);

/// Küplerin işaretleri.
#[derive(Clone, Copy)]
struct GrowCube;
gizmo::core::impl_component!(GrowCube);
#[derive(Clone, Copy)]
struct SpinCube;
gizmo::core::impl_component!(SpinCube);
#[derive(Clone, Copy)]
struct WindowCube;
gizmo::core::impl_component!(WindowCube);

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy Run Conditions", 1280, 720)
        .with_simple_scene(|scene, state| {
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            let cube = AssetManager::create_cube(&scene.renderer.device);

            let mut spawn = |x: f32, albedo: Vec4| {
                scene.world.spawn_bundle((
                    Transform::new(Vec3::new(x, 0.6, 0.0)).with_scale(Vec3::splat(0.35)),
                    GlobalTransform::default(),
                    cube.clone(),
                    Material::new(white.clone()).with_pbr(albedo, 0.5, 0.0),
                    MeshRenderer::new(),
                ))
            };
            let grow = spawn(-1.8, Vec4::new(0.9, 0.5, 0.2, 1.0));
            let spin = spawn(0.0, Vec4::new(0.3, 0.7, 0.4, 1.0));
            let window = spawn(1.8, Vec4::new(0.4, 0.5, 0.95, 1.0));
            scene.world.add_component(grow, GrowCube);
            scene.world.add_component(spin, SpinCube);
            scene.world.add_component(window, WindowCube);

            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 10.0),
                Material::new(white).with_pbr(Vec4::new(0.1, 0.11, 0.13, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.8) * Quat::from_rotation_x(-0.6),
                intensity: 2.5,
                ..Default::default()
            });

            scene.world.insert_resource(InputCounter::default());
            scene.world.insert_resource(Ran::default());
            scene.spawn_camera(state, Vec3::new(0.0, 2.2, 6.0), Vec3::new(0.0, 0.6, 0.0));
        })
        // Sayacı artıran sistemin kendi kapısı yok — girdiyi o okuyor.
        .add_update_system(count_input.in_phase(Phase::Update))
        // VEYA: motorda birleştirici olmadığı için tek kapanışta.
        .add_update_system(
            grow_with_counter
                .in_phase(Phase::Update)
                .run_if(either(space_is_down, left_click_is_down)),
        )
        // VE: iki `run_if` art arda — ikincisi birincinin içine sarılır.
        .add_update_system(
            spin_on_change
                .in_phase(Phase::Update)
                .run_if(counter_exists)
                .run_if(counter_changed_recently),
        )
        // Zaman penceresi: `time_passed(2)` VE `not(time_passed(4))`.
        .add_update_system(
            show_in_window
                .in_phase(Phase::Update)
                .run_if(time_passed(2.0))
                .run_if(not(time_passed(4.0))),
        )
        .set_ui(|world, _state, ctx| {
            let counter = world.get_resource::<InputCounter>().map(|c| *c).unwrap_or_default();
            let ran = world.get_resource::<Ran>().map(|r| *r).unwrap_or_default();
            let frame = world.get_resource::<Time>().map(|t| t.frame()).unwrap_or(0);
            gizmo::egui::Area::new("cond".into())
                .anchor(gizmo::egui::Align2::LEFT_TOP, [12.0, 12.0])
                .show(ctx, |ui| {
                    ui.heading("Koşullu sistemler");
                    ui.label(format!("sayaç: {} · kare: {frame}", counter.value));
                    ui.separator();
                    let mark = |last: u64| if last + 1 >= frame { "KOŞTU" } else { "durdu" };
                    ui.label(format!("sol küp  (Space VEYA sol tık): {}", mark(ran.grow)));
                    ui.label(format!("orta küp (sayaç var VE değişti): {}", mark(ran.spin)));
                    ui.label(format!("sağ küp  (2 sn sonra VE 4 sn öncesi): {}", mark(ran.window)));
                    ui.separator();
                    ui.label("kapanış biçimi sistemi exclusive yapar;");
                    ui.label("paralel kalması için tipli run_if_sys yolu var.");
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

// ── Koşullar ────────────────────────────────────────────────────────────────────────────────

/// Kaynak var mı — Bevy'nin `resource_exists::<T>`'ının karşılığı.
fn counter_exists(world: &World) -> bool {
    world.get_resource::<InputCounter>().is_some()
}

/// Sayaç son birkaç karede arttı mı.
fn counter_changed_recently(world: &World) -> bool {
    let (Some(counter), Some(time)) = (
        world.get_resource::<InputCounter>(),
        world.get_resource::<Time>(),
    ) else {
        return false;
    };
    counter.value > 0 && time.frame().saturating_sub(counter.changed_frame) < 30
}

/// Space basılı mı.
fn space_is_down(world: &World) -> bool {
    world
        .get_resource::<Input>()
        .is_some_and(|input| input.is_key_pressed(gizmo::winit::keyboard::KeyCode::Space as u32))
}

/// Sol tık basılı mı.
fn left_click_is_down(world: &World) -> bool {
    world
        .get_resource::<Input>()
        .is_some_and(|input| input.is_mouse_button_pressed(0))
}

/// **VEYA birleştiricisi — motorda olmadığı için burada.**
///
/// Bevy `cond_a.or_else(cond_b)` yazabiliyor; Gizmo'da iki `run_if` VE'lendiği için VEYA tek bir
/// kapanışta ifade edilmeli. Kısa devre Rust'ın `||`'ından geliyor: ilki doğruysa ikinci hiç
/// çağrılmaz.
fn either(
    mut a: impl FnMut(&World) -> bool + Send + Sync + 'static,
    mut b: impl FnMut(&World) -> bool + Send + Sync + 'static,
) -> impl FnMut(&World) -> bool + Send + Sync + 'static {
    move |world| a(world) || b(world)
}

/// Bevy'nin `not(...)`'unun karşılığı.
fn not(
    mut condition: impl FnMut(&World) -> bool + Send + Sync + 'static,
) -> impl FnMut(&World) -> bool + Send + Sync + 'static {
    move |world| !condition(world)
}

/// Bevy'nin `time_passed(secs)` üreticisi: uygulama başlayalı `secs` saniye geçti mi.
fn time_passed(secs: f64) -> impl FnMut(&World) -> bool + Send + Sync + 'static {
    move |world| {
        world
            .get_resource::<Time>()
            .is_some_and(|time| time.elapsed() >= secs)
    }
}

// ── Sistemler ───────────────────────────────────────────────────────────────────────────────

/// Space ya da sol tık sayacı artırır (kenar okur, bu yüzden kapısı yok — her kare koşmalı).
fn count_input(mut counter: ResMut<InputCounter>, input: Res<Input>, time: Res<Time>) {
    let pressed = input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Space as u32)
        || input.is_mouse_button_just_pressed(0);
    if pressed {
        counter.value += 1;
        counter.changed_frame = time.frame();
    }
}

/// Sol küp: sayaçla büyür. Kapı VEYA olduğu için Space ya da tık basılıyken koşar.
fn grow_with_counter(
    mut cubes: Query<(Mut<Transform>, With<GrowCube>)>,
    counter: Res<InputCounter>,
    mut ran: ResMut<Ran>,
    time: Res<Time>,
) {
    ran.grow = time.frame();
    let scale = 0.35 + (counter.value as f32 * 0.02).min(0.5);
    for (_entity, (mut transform, _)) in cubes.iter_mut() {
        transform.scale = Vec3::splat(scale);
    }
}

/// Orta küp: yalnız sayaç yakın zamanda değiştiyse döner.
fn spin_on_change(
    mut cubes: Query<(Mut<Transform>, With<SpinCube>)>,
    mut ran: ResMut<Ran>,
    time: Res<Time>,
) {
    ran.spin = time.frame();
    for (_entity, (mut transform, _)) in cubes.iter_mut() {
        transform.rotation *= Quat::from_rotation_y(time.dt() * 3.0);
    }
}

/// Sağ küp: yalnız 2-4 saniye penceresinde yukarı süzülür, dışında yere iner.
fn show_in_window(
    mut cubes: Query<(Mut<Transform>, With<WindowCube>)>,
    mut ran: ResMut<Ran>,
    time: Res<Time>,
) {
    ran.window = time.frame();
    for (_entity, (mut transform, _)) in cubes.iter_mut() {
        transform.position.y = 0.6 + ((time.elapsed() as f32 - 2.0) * 1.5).min(1.5);
    }
}
