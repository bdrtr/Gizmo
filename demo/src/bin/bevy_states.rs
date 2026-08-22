//! # Bevy'nin `state/states` örneğinin Gizmo Engine portu
//!
//! Üst düzey akış: **Menü** ekranında bir "Oyna" düğmesi, basınca **Oyun** durumuna geçiş —
//! ve oyun sistemleri yalnız o durumdayken çalışıyor. Bevy'nin örneği bunu `init_state`,
//! `OnEnter`, `OnExit` ve `run_if(in_state(...))` ile kuruyor.
//!
//! ## Motorda bunun ne kadarı hazır, ne kadarı sizin
//!
//! Gizmo'da [`State<S>`] ve [`in_state`] var, ve ikisi de tam olarak Bevy'dekini yapıyor:
//! durum değişimi **ertelenir** (`set` kuyruğa alır, `apply_transitions` uygular), böylece aynı
//! kare içinde çalışan sistemler yırtık bir görüntü görmez; `in_state` de bir koşula dönüşüp
//! `run_if` ile sisteme takılır.
//!
//! **Ama motorda hiçbir şey bu makineyi SÜRMÜYOR** — `State` bir kaynak, onu ilerleten bir faz
//! yok. Bu, tipin kendi belgesinde yazılı bir karar, ve pratikte iki satır iş demek:
//!   1. `apply_transitions()` her karede bir kez, durumu okuyan sistemlerden **önce**
//!      (burada `Phase::PreUpdate`).
//!   2. Bevy'nin `OnEnter`/`OnExit` çizelgeleri **yok**; giriş/çıkış işini elle yapmak gerekiyor.
//!      Buradaki karşılığı, geçişi uygulayan sistemin dönüş değerine bakıp o kareye özgü
//!      kurulum/temizliği yapması — `spawn_game_scene` / `despawn_game_scene`.
//!
//! Menü de Bevy'dekinden farklı bir yerden geliyor: Bevy `bevy_ui` düğmesi kuruyor, Gizmo'nun
//! kendi arayüz yolu **egui** (`set_ui`). Aynı davranış, motorun kendi aracıyla.
//!
//!
//! **Bir motor tuzağı, ve Bevy'den gelen biri için görünmez:** `add_system` sistemi **sabit
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

/// Bevy'nin `AppState`'i.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppState {
    Menu,
    InGame,
}

/// Oyun sahnesine ait varlıkların işareti — durumdan çıkarken toplu silinirler.
#[derive(Clone, Copy)]
struct GameEntity;
gizmo::core::impl_component!(GameEntity);

/// Oyun sahnesinin kurulmasını/temizlenmesini bir sonraki render'a bildiren kaynak.
///
/// Sahne kurmak mesh ister, mesh `device` ister, `device` de yalnız kurulum ve render
/// kapanışlarında elde vardır — sistemler oradan bakamaz. Bu yüzden durum geçişi sistemde
/// karar veriliyor, sahne ise render kapanışında kuruluyor.
#[derive(Default, Clone, Copy)]
struct SceneRequest {
    spawn: bool,
    despawn: bool,
}
gizmo::core::impl_component!(SceneRequest);

/// Küpün ok tuşlarıyla hareket hızı.
const MOVE_SPEED: f32 = 3.0;

fn main() {
    App::<SimpleSceneState>::new("Gizmo Engine - Bevy States", 1280, 720)
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
                Material::new(white).with_pbr(Vec4::new(0.12, 0.13, 0.15, 1.0), 1.0, 0.0),
                MeshRenderer::new(),
            ));
            scene.world.spawn_bundle(DirectionalLightBundle {
                rotation: Quat::from_rotation_y(0.8) * Quat::from_rotation_x(-0.7),
                intensity: 2.0,
                ..Default::default()
            });

            // Durum makinesi bir KAYNAK. Yerleştirilmediği sürece `in_state` her yerde false
            // döndürür, yani durumla kapılanmış her sistem KAPALI başlar.
            scene.world.insert_resource(State::new(AppState::Menu));
            scene.world.insert_resource(SceneRequest::default());

            scene.spawn_camera(state, Vec3::new(0.0, 6.0, 9.0), Vec3::ZERO);
        })
        // 1) Geçişleri uygula — durumu okuyan her şeyden önce.
        .add_update_system(apply_state_transitions.in_phase(Phase::PreUpdate))
        // 2) Yalnız oyundayken çalışan sistem.
        .add_update_system(
            movement
                .in_phase(Phase::Update)
                .run_if(in_state(AppState::InGame)),
        )
        // 3) Yalnız oyundayken çalışan ikinci sistem: Esc menüye döner.
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
                    ui.heading("Bevy States — Gizmo portu");
                    ui.label("Durum: Menü");
                    if ui.button("Oyna").clicked() {
                        if let Some(mut state) = world.get_resource_mut::<State<AppState>>() {
                            // Geçiş KUYRUĞA girer; bu karede kimse değişmiş görmez.
                            state.set(AppState::InGame);
                        }
                    }
                });
        })
        // Sahne kurma/temizleme: cihaz yalnız burada var.
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_physics = None;
            renderer.gpu_fluid = None;
            renderer.gpu_particles = None;
            renderer.ssr = None;
            renderer.ssgi = None;

            let request = world
                .get_resource::<SceneRequest>()
                .map(|r| *r)
                .unwrap_or_default();
            if request.despawn {
                despawn_game_scene(world);
            }
            if request.spawn {
                spawn_game_scene(world, renderer);
            }
            if request.spawn || request.despawn {
                if let Some(mut r) = world.get_resource_mut::<SceneRequest>() {
                    *r = SceneRequest::default();
                }
            }

            gizmo::systems::default_render_pass(world, encoder, view, renderer);
        })
        .run()
        .expect("uygulama çalıştırılamadı");
}

/// Bevy'nin `StateTransition` çizelgesinin karşılığı — motorda olmayan tek parça.
///
/// Dönüş değeri "bu karede gerçekten geçiş oldu mu"yu söyler, ve `OnEnter`/`OnExit`'in yerini
/// tutan şey de bu: geçiş anında hangi tarafa geçildiğine bakıp kurulum ya da temizlik isteniyor.
fn apply_state_transitions(mut state: ResMut<State<AppState>>, mut request: ResMut<SceneRequest>) {
    if !state.apply_transitions() {
        return;
    }
    match state.get() {
        AppState::InGame => request.spawn = true,
        AppState::Menu => request.despawn = true,
    }
}

/// Yalnız `InGame`'de koşar: küpü ok tuşlarıyla gezdirir.
fn movement(
    mut cubes: Query<(Mut<Transform>, With<GameEntity>)>,
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

/// `OnEnter(InGame)` karşılığı: oyuncunun küpünü kur.
fn spawn_game_scene(world: &mut World, renderer: &mut Renderer) {
    let mut assets = AssetManager::new();
    let white = assets.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.5, 0.0)).with_scale(Vec3::splat(0.5)),
        GlobalTransform::default(),
        AssetManager::create_cube(&renderer.device),
        Material::new(white).with_pbr(Vec4::new(0.35, 0.75, 0.35, 1.0), 0.5, 0.0),
        MeshRenderer::new(),
        GameEntity,
    ));
}

/// `OnExit(InGame)` karşılığı: oyun sahnesini topla.
fn despawn_game_scene(world: &mut World) {
    let doomed: Vec<u32> = world
        .query::<&GameEntity>()
        .map(|q| q.iter().map(|(entity, _)| entity).collect())
        .unwrap_or_default();
    for entity in doomed {
        world.despawn_by_id(entity);
    }
}
