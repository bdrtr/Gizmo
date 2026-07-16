//! # KCC Sandbox — Karakter Kontrolcüsü (CharacterController) test sahnesi
//!
//! Basamak (`step_height`), yürünebilir rampa (30°) ve dik rampa (60° → kayar,
//! `max_slope_angle`) üstünde kinematik bir karakteri sür. FPS kamerası karakterin
//! baş hizasına oturur; SAĞ-tık + fare ile bak, WASD ile yürü, SPACE ile zıpla.
//!
//! Bu demo motorun modern idiomlarını kullanır ama NEYİN motora ait olduğu konusunda dürüst:
//!   * **`PhysicsPlugin` YOK** — sahne fiziği BİLEREK elle sürülür: `PhysicsWorld` kaynağı
//!     kurulur ve `update` her kare `physics_step_system`'i 16 ms alt-adımlara bölerek çağırır.
//!     Bu yüzden `DespawnAfter`/`DespawnBelowY` gibi ömür-programı bileşenleri EKLENMEZ (bu
//!     demoda geçici/uçan nesne de yok); onları eklemek sessizce çalışmazdı.
//!   * **`Prefab` + `auto_box_collider`** — zemin/basamaklar/rampalar tek statik blueprint'ten;
//!     kutu collider `Transform.scale`'den OTOMATİK türetilir (boyut iki kez yazılmaz).
//!   * **`spawn_bundle` + explicit `Collider`** — karakter tek-tük kapsül gövde: kinematik
//!     `RigidBodyBundle` + `Collider::capsule`, mesh/materyal/`CharacterController` tek çağrıda.
//!   * **`Camera::forward_from`** — bakış yönü paylaşılan yardımcıdan (elle trig yok).
//!   * **Render = `default_render_pass` DOĞRUDAN** — motorun tüm deferred efektlerini (gölge,
//!     GI) açık tutan çıplak kurulum; `with_scene_render()` kısayolu bilerek kullanılmaz.
//!
//! ## Kontroller
//!   * **SAĞ-tık + Fare** — etrafa bak · **W A S D** — kameraya göre yürü · **SPACE** — zıpla

use gizmo::physics::components::CharacterController;
use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use std::f32::consts::{FRAC_PI_3, PI};

struct KccState {
    character: Entity,
    yaw: f32,
    pitch: f32,
}

fn setup(world: &mut World, renderer: &Renderer) -> KccState {
    let mut assets = AssetManager::new();

    // Manuel fizik dünyası — bu demo PhysicsPlugin KULLANMAZ; adımları update'te elle atar.
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));

    let cube = AssetManager::create_cube(&renderer.device);
    let checker = assets.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let ground_mat =
        Material::new(checker.clone()).with_pbr(Vec4::new(0.6, 0.6, 0.6, 1.0), 0.8, 0.1);

    // --- Gökyüzü kubbesi (skybox dokusu; diskten) ---
    let sky_path = if std::path::Path::new("tut/assets/sky.jpg").exists() {
        "tut/assets/sky.jpg"
    } else {
        "assets/sky.jpg"
    };
    let sky_tex = assets
        .load_material_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
            sky_path,
        )
        .expect("skybox dokusu yüklenemedi");
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2000.0)),
        AssetManager::create_inverted_cube(&renderer.device),
        Material::new(sky_tex).with_skybox(),
        MeshRenderer::new(),
    ));

    // --- Statik zemin/basamak/rampalar: TEK blueprint (kutu collider Transform.scale'den) ---
    let block = Prefab::new(cube, ground_mat)
        .with_body(RigidBodyBundle::static_body())
        .auto_box_collider();

    // Zemin (üstü y=0)
    block.spawn(
        world,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)).with_scale(Vec3::new(100.0, 1.0, 100.0)),
    );

    // Basamaklar — step_height testi (5 kademe, giderek yükselen)
    for i in 0..5 {
        let step_h = 0.2 * (i as f32 + 1.0);
        block.spawn(
            world,
            Transform::new(Vec3::new(5.0 + i as f32, step_h - 1.0, 0.0))
                .with_scale(Vec3::new(1.0, step_h, 4.0)),
        );
    }

    // Yürünebilir rampa (30°) — çıkılabilmeli
    block.spawn(
        world,
        Transform::new(Vec3::new(0.0, 1.0, 10.0))
            .with_scale(Vec3::new(5.0, 0.5, 10.0))
            .with_rotation(Quat::from_rotation_x(PI / 6.0)),
    );
    // Dik rampa (60°) — max_slope_angle testi: karakter kaymalı
    block.spawn(
        world,
        Transform::new(Vec3::new(10.0, 2.0, 10.0))
            .with_scale(Vec3::new(5.0, 0.5, 10.0))
            .with_rotation(Quat::from_rotation_x(PI / 3.0)),
    );

    // --- Karakter (kinematik kapsül; KCC sistemi onu sürer) ---
    let char_mat = Material::new(checker).with_pbr(Vec4::new(0.1, 0.8, 0.2, 1.0), 0.5, 0.5);
    let character = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 2.0, 0.0)),
        AssetManager::create_sphere(&renderer.device, 0.5, 16, 16),
        char_mat,
        MeshRenderer::new(),
        CharacterController {
            speed: 8.0,
            jump_speed: 6.0,
            step_height: 0.3,
            ..Default::default()
        },
        // Kinematik: fizik kuvvetleri doğrudan etkilemez, hareketi KCC sistemi verir.
        RigidBodyBundle::kinematic().with_collider(Collider::capsule(0.5, 0.5)),
    ));

    // --- Kamera (update'te her kare karakterin başına taşınır; bu yalnız ilk kare) ---
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 5.0, 10.0)),
        Camera::new(FRAC_PI_3, 0.1, 1000.0, 0.0, -PI / 8.0, true),
    ));

    // --- Güneş ---
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)),
        DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 4.0, LightRole::Sun),
    ));

    println!("🏃 KCC Sandbox — SAĞ-tık+fare: bak · WASD: yürü · SPACE: zıpla");
    KccState {
        character,
        yaw: 0.0,
        pitch: -PI / 8.0,
    }
}

fn update(world: &mut World, state: &mut KccState, dt: f32, input: &Input) {
    // --- Kamera bakışı: SAĞ-tık basılıyken fare ile (pointer-lock yok) ---
    if input.is_mouse_button_pressed(1) {
        let (dx, dy) = input.mouse_delta();
        state.yaw -= dx * 0.005;
        state.pitch = (state.pitch - dy * 0.005).clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
    }

    // Hareket yönleri kameradan: ileri (yatay düzleme yassıtılmış) + sağ.
    // Motorun Camera'sı yaw=0'da +X'e bakar; karakter mesh'i yaw ile hizalanır.
    let forward = Camera::forward_from(state.yaw, state.pitch);
    let move_forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let move_right = Vec3::new(-state.yaw.sin(), 0.0, state.yaw.cos()).normalize_or_zero();
    let cam_rot = Quat::from_rotation_y(-state.yaw);

    // --- Karakter kontrolü (WASD → hedef hız, SPACE → zıplama tamponu) ---
    if let Some(mut kcc) = world
        .borrow_mut::<CharacterController>()
        .get_mut(state.character.id())
    {
        let mut dir = Vec3::ZERO;
        if input.is_key_pressed(KeyCode::KeyW as u32) {
            dir += move_forward;
        }
        if input.is_key_pressed(KeyCode::KeyS as u32) {
            dir -= move_forward;
        }
        if input.is_key_pressed(KeyCode::KeyD as u32) {
            dir += move_right;
        }
        if input.is_key_pressed(KeyCode::KeyA as u32) {
            dir -= move_right;
        }
        kcc.target_velocity = dir.normalize_or_zero() * kcc.speed;

        if input.is_key_pressed(KeyCode::Space as u32) {
            kcc.jump_buffer_timer = kcc.jump_buffer_time;
        }
    }

    // --- Fizik adımı (manuel; 16 ms alt-adımlara böl) ---
    let mut remaining = dt.min(0.1);
    while remaining > 0.0 {
        let step = remaining.min(0.016);
        gizmo::physics::system::physics_step_system(world, step);
        remaining -= step;
    }

    // --- Karakteri kamera yaw'ına döndür + konumunu oku ---
    let mut char_pos = Vec3::ZERO;
    if let Some(mut tr) = world
        .borrow_mut::<Transform>()
        .get_mut(state.character.id())
    {
        char_pos = tr.position;
        tr.rotation = Quat::from_rotation_y(state.yaw);
    }

    // --- FPS kamerası: karakterin baş hizasında ---
    let cam_pos = char_pos + Vec3::new(0.0, 0.8, 0.0);
    if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Camera>)>() {
        for (_, (mut tr, mut cam)) in q.iter_mut() {
            tr.position = cam_pos;
            tr.rotation = cam_rot;
            cam.yaw = state.yaw;
            cam.pitch = state.pitch;
        }
    }
}

fn render(
    world: &mut World,
    _state: &KccState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    // Render'ı motorun tam deferred boru hattına devret (efektler açık kalır).
    default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<KccState>::new("Gizmo Engine - KCC Sandbox", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
