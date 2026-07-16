//! # 3D PLATFORMER — hareketli platformlar + karakter denetleyicisi
//!
//! Başlangıç platformundan hareketli platforma atla, güvenli bölgeye ulaş; lava'ya düşersen
//! respawn olursun. Sağ-tık + fare ile kamerayı çevir.
//!
//! Bu sürüm motorun modern idiomlarını kullanır — ama SAHNE/KONTROL/DAVRANIŞ birebir korunur:
//!   * **`spawn_bundle` + `RigidBodyBundle`** — her platform TEK `spawn_bundle` çağrısıyla
//!     kurulur (eski `spawn()` + tekrar eden `add_component` zinciri gitti). `kinematic()`/
//!     `static_body()` gövde+collider+velocity üçlüsünü tek yerde toplar. Platformlar tek-tük
//!     ve farklı boyutlu olduğundan Prefab yerine explicit collider'lı bundle uygun.
//!   * **`Camera::forward_from` / `Camera::right_from`** — kamera/hareket yön matematiği elle
//!     yeniden yazılmaz, motorun paylaşılan yardımcısından gelir.
//!   * **Ömür komponenti (`DespawnAfter`) YOK — bilinçli.** Bu demo `PhysicsPlugin` EKLEMEZ;
//!     ömür/fizik schedule'ı çalışmaz ve uçan/geçici nesne yoktur. Kinematik platform+oyuncu
//!     hareketi elle entegre edilir. Bu yüzden lifetime/fizik-bağımlı idiomlar eklenmedi.
//!   * Sahne render = `default_render_pass` doğrudan (SSR/SSGI/volumetric/TAA açık kalsın).
//!
//! ## Kontroller
//!   * **W A S D** — hareket · **SPACE** — zıpla · **Sağ-tık + fare** — kamera

use gizmo::physics::components::CharacterController;
use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use std::f32::consts::PI;

#[derive(Clone)]
struct MovingPlatform {
    pub start_pos: Vec3,
    pub end_pos: Vec3,
    pub speed: f32,
    pub going_to_end: bool,
}
gizmo::core::impl_component!(MovingPlatform);

#[derive(Clone)]
struct DeathZone;
gizmo::core::impl_component!(DeathZone);

struct PlatformerState {
    player: Entity,
    camera_yaw: f32,
    camera_pitch: f32,
    respawn_point: Vec3,
}

fn setup(world: &mut World, renderer: &Renderer) -> PlatformerState {
    println!("🏃 3D PLATFORMER DEMO BAŞLIYOR 🏃");

    let mut asset_manager = AssetManager::new();
    let phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -15.0, 0.0));
    world.insert_resource(phys_world);

    // --- MATERYALLER & MESH'LER ---
    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let sphere_mesh = AssetManager::create_sphere(&renderer.device, 0.5, 16, 16);

    let tex = asset_manager.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    let ground_mat = Material::new(tex.clone()).with_pbr(Vec4::new(0.3, 0.8, 0.3, 1.0), 0.8, 0.0);
    let platform_mat = Material::new(tex.clone()).with_pbr(Vec4::new(0.8, 0.3, 0.8, 1.0), 0.5, 0.0);
    let lava_mat = Material::new(tex.clone()).with_pbr(Vec4::new(1.0, 0.2, 0.0, 1.0), 0.9, 0.0);
    let player_mat = Material::new(tex.clone()).with_pbr(Vec4::new(0.2, 0.4, 1.0, 1.0), 0.2, 0.0);

    // --- BÖLÜM TASARIMI ---
    // Başlangıç platformu (statik)
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::new(5.0, 1.0, 5.0)),
        cube_mesh.clone(),
        ground_mat.clone(),
        MeshRenderer::new(),
        RigidBodyBundle::static_body()
            .with_collider(Collider::box_collider(Vec3::new(2.5, 0.5, 2.5))),
    ));

    // Hareketli platform 1 (kinematik — konumu update'te elle sürülür)
    let move_plat1 = world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 0.0, 10.0)).with_scale(Vec3::new(3.0, 1.0, 3.0)),
        cube_mesh.clone(),
        platform_mat.clone(),
        MeshRenderer::new(),
        RigidBodyBundle::kinematic()
            .with_collider(Collider::box_collider(Vec3::new(1.5, 0.5, 1.5))),
    ));
    world.add_component(
        move_plat1,
        MovingPlatform {
            start_pos: Vec3::new(0.0, 0.0, 10.0),
            end_pos: Vec3::new(15.0, 0.0, 10.0),
            speed: 5.0,
            going_to_end: true,
        },
    );

    // Güvenli bölge (statik)
    world.spawn_bundle((
        Transform::new(Vec3::new(20.0, 2.0, 10.0)).with_scale(Vec3::new(4.0, 1.0, 4.0)),
        cube_mesh.clone(),
        ground_mat.clone(),
        MeshRenderer::new(),
        RigidBodyBundle::static_body()
            .with_collider(Collider::box_collider(Vec3::new(2.0, 0.5, 2.0))),
    ));

    // Lava (ölüm bölgesi). Lav statik kalır; öldürme trigger API'siyle değil, oyuncunun
    // y-yüksekliği eşiğiyle (update'teki -3.0 kontrolü) yapılır.
    let lava = world.spawn_bundle((
        Transform::new(Vec3::new(10.0, -5.0, 5.0)).with_scale(Vec3::new(50.0, 1.0, 50.0)),
        cube_mesh.clone(),
        lava_mat.clone(),
        MeshRenderer::new(),
        RigidBodyBundle::static_body()
            .with_collider(Collider::box_collider(Vec3::new(25.0, 0.5, 25.0))),
    ));
    world.add_component(lava, DeathZone);

    // --- OYUNCU ---
    let respawn = Vec3::new(0.0, 3.0, 0.0);
    let player = world.spawn_bundle((
        Transform::new(respawn),
        sphere_mesh.clone(),
        player_mat,
        MeshRenderer::new(),
        RigidBodyBundle::kinematic().with_collider(Collider::capsule(0.5, 0.5)),
    ));
    world.add_component(
        player,
        CharacterController {
            speed: 8.0,
            jump_speed: 12.0,
            ..Default::default()
        },
    );

    // --- KAMERA ---
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 5.0, -10.0)),
        Camera::new(
            std::f32::consts::FRAC_PI_3,
            0.1,
            1500.0,
            0.0,
            -PI / 8.0,
            true,
        ),
    ));

    // --- GÜNEŞ ---
    world.spawn_bundle((
        Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)),
        DirectionalLight::new(Vec3::new(1.0, 0.9, 0.9), 3.0, LightRole::Sun),
    ));

    PlatformerState {
        player,
        camera_yaw: 0.0,
        camera_pitch: -PI / 8.0,
        respawn_point: respawn,
    }
}

fn update(world: &mut World, state: &mut PlatformerState, dt: f32, input: &Input) {
    // 1. Hareketli platformlar
    if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Velocity>, Mut<MovingPlatform>)>() {
        for (_id, (mut p_trans, mut p_vel, mut plat)) in q.iter_mut() {
            let target = if plat.going_to_end {
                plat.end_pos
            } else {
                plat.start_pos
            };
            let dir = target - p_trans.position;
            let dist = dir.length();

            if dist < 0.1 {
                plat.going_to_end = !plat.going_to_end;
                p_vel.linear = Vec3::ZERO;
            } else {
                p_vel.linear = dir.normalize() * plat.speed;
                // Kinematik gövde motor tarafından entegre edilmez → pozisyonu elle güncelle.
                p_trans.position += p_vel.linear * dt;
            }
        }
    }

    // 2. Kamera kontrolleri
    if input.is_mouse_button_pressed(1) {
        let delta = input.mouse_delta();
        state.camera_yaw -= delta.0 * 0.005;
        state.camera_pitch -= delta.1 * 0.005;
        state.camera_pitch = state.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
    }

    // İleri/sağ yönler motorun paylaşılan yardımcısından (elle trig YOK).
    let forward = Camera::forward_from(state.camera_yaw, state.camera_pitch);
    let right = Camera::right_from(state.camera_yaw);

    let mut move_forward = forward;
    move_forward.y = 0.0;
    move_forward = move_forward.normalize_or_zero();
    let mut move_right = right;
    move_right.y = 0.0;
    move_right = move_right.normalize_or_zero();

    let mut player_pos = Vec3::ZERO;

    // 3. Karakter kontrolü
    if let Some(mut kcc) = world
        .borrow_mut::<CharacterController>()
        .get_mut(state.player.id())
    {
        let mut move_dir = Vec3::ZERO;
        if input.is_key_pressed(KeyCode::KeyW as u32) {
            move_dir += move_forward;
        }
        if input.is_key_pressed(KeyCode::KeyS as u32) {
            move_dir -= move_forward;
        }
        if input.is_key_pressed(KeyCode::KeyD as u32) {
            move_dir += move_right;
        }
        if input.is_key_pressed(KeyCode::KeyA as u32) {
            move_dir -= move_right;
        }

        move_dir = move_dir.normalize_or_zero();
        kcc.target_velocity = move_dir * kcc.speed;

        if input.is_key_pressed(KeyCode::Space as u32) {
            kcc.jump_buffer_timer = kcc.jump_buffer_time;
        }
    }

    // Ölüm kontrolü — oyuncu lava seviyesinin altına düşerse respawn.
    if let Some(mut t) = world.borrow_mut::<Transform>().get_mut(state.player.id()) {
        player_pos = t.position;
        if t.position.y < -3.0 {
            println!("💀 ÖLDÜN! Respawn oluyorsun...");
            t.position = state.respawn_point;
            if let Some(mut v) = world.borrow_mut::<Velocity>().get_mut(state.player.id()) {
                v.linear = Vec3::ZERO;
            }
        }
    }

    // 4. Kamerayı oyuncuya takip ettir
    let cam_pos = player_pos - forward * 8.0 + Vec3::new(0.0, 2.0, 0.0);
    if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Camera>)>() {
        for (_, (mut trans, mut cam)) in q.iter_mut() {
            trans.position = cam_pos;
            trans.rotation = Quat::from_rotation_y(-state.camera_yaw);
            cam.yaw = state.camera_yaw;
            cam.pitch = state.camera_pitch;
        }
    }
}

fn render(
    world: &mut World,
    _state: &PlatformerState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    gizmo::systems::render::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<PlatformerState>::new("Gizmo Engine - 3D Platformer", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
