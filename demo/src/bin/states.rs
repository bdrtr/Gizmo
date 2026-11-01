//! # Uygulama durumları
//!
//! Üst düzey akış: **Menü** ekranında bir "Oyna" düğmesi, basınca **Oyun** durumuna geçiş —
//! ve oyun sistemleri yalnız o durumdayken çalışıyor.
//!
//! ## Motor durum makinesini sürüyor (2026-11-02'den beri)
//!
//! [`State<S>`] ve [`in_state`] baştan beri vardı; eksik olan, makineyi **süren** şeydi —
//! `apply_transitions`'ı kimse çağırmıyordu, giriş/çıkış çizelgesi yoktu ve başlangıç durumuna
//! giriş hiç bildirilmiyordu. Bu demo o boşluğu iki sistemle ve render kapanışında bir istek
//! kaynağıyla elle dolduruyordu. Şimdi:
//!
//!   * `App::add_state(AppState::Menu)` — makineyi kurar; uygulama döngüsü her karede, **her
//!     çizelgeden önce** sıradaki geçişi uygular.
//!   * `App::on_enter(AppState::InGame, |world| …)` — o duruma girişte `&mut World` ile çalışır.
//!     Başlangıç durumuna giriş de bildirilir.
//!   * [`DespawnOnExit`]`(AppState::InGame)` — o durumdan çıkışta varlık (çocuklarıyla) silinir.
//!
//! Sahne kurmak mesh ister, mesh de `device`; hook'ta cihaz yok. Bu yüzden küpün mesh'i ve
//! malzemesi kurulumda **bir kez** oluşturulup kaynak olarak saklanıyor, hook onları klonluyor.
//!
//! Menü, motorun kendi arayüz yolundan geliyor: **egui** (`set_ui`).
//!
//! **Bir motor tuzağı, ve dışarıdan bakınca görünmez:** `add_system` sistemi **sabit
//! adımlı** çizelgeye kaydeder — kare başına sıfır ya da birkaç kez koşar. Girdi kenarları
//! (`is_key_just_pressed`) ise kare başına bir kez yakalanıp temizlenir, yani o çizelgede
//! okunursa basışlar kaçar ya da iki kez işlenir. Girdiye bakan her sistem bu yüzden
//! `add_update_system` ile kaydediliyor (kare başına tam bir kez, gerçek `dt` ile).
//! ## Kontroller
//!   * **Menüde**: "Oyna" düğmesi · **Oyunda**: ok tuşlarıyla küpü gezdir, **Esc** menüye dön
//!   * **Sağ-tık + fare** — bak · **WASDQE** — kamera

use gizmo::core::input::{Input, MoveKeys};
use gizmo::core::query::{Mut, Query, With};
use gizmo::core::system::{IntoSystemConfig, Phase, Res, ResMut};
use gizmo::prelude::*;
use gizmo::simple::{SimpleAppExt, SimpleSceneState};

/// Uygulamanın iki durumu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppState {
    Menu,
    InGame,
}

/// Oyuncunun küpünün işareti — hareket sistemi bunu arar.
#[derive(Clone, Copy)]
struct Player;
gizmo::core::impl_component!(Player);

/// Oyuna girişte kurulan küpün mesh'i ve malzemesi, kurulumda bir kez oluşturulmuş:
/// `on_enter` hook'unda GPU cihazı yok, klonlamak ise ucuz (ikisi de `Arc`).
#[derive(Clone)]
struct GameAssets {
    cube: Mesh,
    material: Material,
}

/// Küpün ok tuşlarıyla hareket hızı.
const MOVE_SPEED: f32 = 3.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - States", 1280, 720)
        // Durum makinesi: kurulur ve uygulama döngüsü tarafından sürülür.
        .add_state(AppState::Menu)
        // Oyuna girişte küpü kur. `DespawnOnExit` çıkışta onu kendisi toplar.
        .on_enter(AppState::InGame, |world| {
            let Some(assets) = world.get_resource::<GameAssets>().map(|a| a.clone()) else { return };
            world.spawn_bundle((
                Transform::new(Vec3::new(0.0, 0.5, 0.0)).with_scale(Vec3::splat(0.5)),
                GlobalTransform::default(),
                assets.cube,
                assets.material,
                MeshRenderer::new(),
                Player,
                DespawnOnExit(AppState::InGame),
            ));
        })
        .on_enter(AppState::Menu, |_world| {
            gizmo::gizmo_log!(Info, "menüye girildi (ilk karede de)");
        })
        .with_simple_scene(|scene, state| {
            // Menüde de görünen sabit sahne: zemin ve güneş.
            let white = scene.asset_manager.create_white_texture(
                &scene.renderer.device,
                &scene.renderer.queue,
                &scene.renderer.scene.texture_bind_group_layout,
            );
            scene.world.spawn_bundle((
                Transform::new(Vec3::ZERO),
                GlobalTransform::default(),
                AssetManager::create_plane(&scene.renderer.device, 12.0),
                Material::new(white.clone()).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.8) * Quat::from_rotation_x(-0.7),
                intensity: 2.0,
                ..Default::default()
            });
            scene.world.insert_resource(GameAssets {
                cube: AssetManager::create_cube(&scene.renderer.device),
                material: Material::new(white).with_pbr(Vec4::new(0.35, 0.75, 0.35, 1.0), 0.5, 0.0),
            });
            scene.spawn_camera(state, Vec3::new(0.0, 6.0, 9.0), Vec3::ZERO);
        })
        // Yalnız oyundayken çalışan iki sistem.
        .add_update_system(
            movement
                .in_phase(Phase::Update)
                .run_if(in_state(AppState::InGame)),
        )
        .add_update_system(
            leave_to_menu
                .in_phase(Phase::Update)
                .run_if(in_state(AppState::InGame)),
        )
        // Menü: motorun kendi arayüz yolu.
        .set_ui(|world, _state, ctx| {
            let in_menu = world
                .get_resource::<State<AppState>>()
                .is_some_and(|s| *s.get() == AppState::Menu);
            if !in_menu {
                return;
            }
            gizmo::egui::Area::new("menu".into())
                .anchor(gizmo::egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.heading("Durumlar");
                    ui.label("Durum: Menü");
                    if ui.button("Oyna").clicked() {
                        if let Some(mut state) = world.get_resource_mut::<State<AppState>>() {
                            // Geçiş KUYRUĞA girer; bir sonraki karenin başında uygulanır.
                            state.set(AppState::InGame);
                        }
                    }
                });
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Yalnız `InGame`'de koşar: küpü ok tuşlarıyla gezdirir.
fn movement(
    mut cubes: Query<(Mut<Transform>, With<Player>)>,
    input: Res<Input>,
    time: Res<Time>,
) {
    let (x, z) = input.move_axis_with(MoveKeys::ARROWS);
    if x == 0.0 && z == 0.0 {
        return;
    }
    // Ok tuşlarının "ileri"si sahnede −Z.
    let step = Vec3::new(x, 0.0, -z) * MOVE_SPEED * time.dt();
    for (_entity, (mut transform, _)) in cubes.iter_mut() {
        transform.position += step;
    }
}

/// Yalnız `InGame`'de koşar: Esc menüye döner.
fn leave_to_menu(mut state: ResMut<State<AppState>>, input: Res<Input>) {
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::Escape as u32) {
        state.set(AppState::Menu);
    }
}
