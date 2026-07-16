//! # CPU FİZİK — eklem/su/soft-body vitrin (temiz sürüm)
//!
//! Motorun CPU fizik yolunu (opt-in `PhysicsWorld` kaynağı + elle `cpu_physics_step_system`)
//! sergiler: menteşeli sarkaç, ball-socket zincir, motorlu kayan platform, suda yüzen kutular
//! ve GPU soft-body (jello). `PhysicsPlugin` KULLANMAZ — fizik adımı bilinçli olarak `update`
//! içinde elle çağrılır (bu demonun konusu tam olarak bu manuel CPU adımı).
//!
//! Neyin motora ait olduğu konusunda dürüst olalım:
//!   * **`spawn_bundle((...))`** — her gövde tek çağrıda kurulur (Transform + mesh + material +
//!     collider + rigid body + velocity); eski sürümdeki düzinelerce `add_component` satırı gitti.
//!     Ortak kurulum [`spawn_box`] yardımcısında toplanır ve joint bağlamak için entity döndürür.
//!   * **Prefab/`RigidBodyBundle` KULLANILMADI** — ikisi de collider'dan ataleti türetir
//!     (`update_inertia_from_collider`); ham `RigidBody::new` yolu türetmez (atalet = `splat(1.0)`).
//!     Bu demonun sarkaç/zincir dinamiği o ham ataletle ayarlı → davranışı KORUMAK için gövdeler
//!     ham bileşenlerle spawn'lanır (yalnız tekrar `spawn_bundle`'a sarılır).
//!   * **`Camera::forward_from`** — nişan/hareket yönü paylaşılan kamera yardımcısından (elle trig yok).
//!   * **Sahne render = `default_render_pass`** — motorun tam deferred boru hattı (debug gizmo
//!     çizgileri + efektler açık). Bu demo GPU-fizik kullanmaz; `gpu_physics` zaten motor-varsayılanı
//!     `None` olduğundan render'da state-mutasyonu GEREKMEZ.
//!
//! ## Kontroller
//!   * **Sağ tık + fare** — kamerayı döndür · **WASD / Space** — hareket · **Shift** — hızlı
//!   * **Sol tık** — bakılan cisme ileri kuvvet uygula (raycast force-push)

use gizmo::core::input::mouse;
use gizmo::core::EntityName;
use gizmo::physics::joints::{Joint, JointData};
use gizmo::physics::soft_body::SoftBodyMesh;
use gizmo::physics::world::{FluidZone, PhysicsWorld, ZoneShape};
use gizmo::physics::BodyHandle;
use gizmo::prelude::*;
use std::f32::consts::{FRAC_PI_2, FRAC_PI_3, PI};

struct DemoState {
    camera_speed: f32,
    camera_pitch: f32,
    camera_yaw: f32,
    camera_pos: Vec3,
    time: f32,
}

/// Kutu-collider'lı bir fizik gövdesi (statik ya da dinamik) TEK `spawn_bundle` çağrısıyla
/// kurup entity'yi döndürür (joint bağlamak için). `half_extents` collider yarı-boyu;
/// `transform.scale` görsel ölçek — ikisi bu demoda bağımsızdır, bu yüzden ayrı verilir.
fn spawn_box(
    world: &mut World,
    mesh: &Mesh,
    material: Material,
    transform: Transform,
    half_extents: Vec3,
    body: RigidBody,
) -> Entity {
    world.spawn_bundle((
        transform,
        mesh.clone(),
        material,
        MeshRenderer::new(),
        Collider::box_collider(half_extents),
        body,
        Velocity::default(),
    ))
}

fn setup(world: &mut World, renderer: &Renderer) -> DemoState {
    let mut asset_manager = AssetManager::new();

    // --- Dokular & mesh'ler ---
    let ground_tex = asset_manager.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let box_tex = asset_manager.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let plane_mesh = AssetManager::create_plane(&renderer.device, 100.0);

    // Tüm kutular aynı taban materyali paylaşır (beyaz, checker); renk per-gövde ezilir.
    let box_mat = Material::new(box_tex).with_pbr(Vec4::splat(1.0), 0.5, 0.0);

    // --- Kamera ---
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 10.0, 20.0)).with_rotation(Quat::from_rotation_x(-0.2)),
        Camera::new(FRAC_PI_3, 0.1, 5000.0, -FRAC_PI_2, -0.2, true),
        EntityName("Main Camera".into()),
    ));

    // --- Işık ---
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 20.0, 0.0)),
        PointLight::new(Vec3::new(1.0, 1.0, 1.0), 500.0, 50.0),
    ));

    // --- Yer düzlemi (statik) ---
    world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        plane_mesh,
        Material::new(ground_tex).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.8, 0.1),
        MeshRenderer::new(),
        Collider::box_collider(Vec3::new(50.0, 0.1, 50.0)),
        RigidBody::new_static(),
        Velocity::default(),
    ));

    // --- CPU FİZİK DÜNYASI (kaynak) ---
    let mut phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));

    // Su havuzu — bir kısmı içine düşen kutular ahşap yoğunluğuyla yüzer.
    phys_world.fluid_zones.push(FluidZone {
        shape: ZoneShape::Box {
            min: Vec3::new(-2.0, 0.0, -2.0),
            max: Vec3::new(2.0, 2.0, 2.0),
        },
        density: 1200.0,
        viscosity: 1.0,
        linear_drag: 5.0,
        quadratic_drag: 1.0,
        ..Default::default()
    });
    phys_world.enable_gpu_compute();

    // --- MENTEŞELİ SARKAÇ (HINGE) ---
    let ceiling = spawn_box(
        world,
        &cube_mesh,
        box_mat.clone(),
        Transform::new(Vec3::new(0.0, 15.0, -10.0)).with_scale(Vec3::new(2.0, 0.5, 2.0)),
        Vec3::new(2.0, 0.5, 2.0),
        RigidBody::new_static(),
    );
    // Tam dengede başlasın: Tavan(15.0) - TavanAltı(0.25) - SarkaçÜstü(2.5) = 12.25
    let pendulum = spawn_box(
        world,
        &cube_mesh,
        box_mat
            .clone()
            .with_pbr(Vec4::new(1.0, 0.0, 0.0, 1.0), 0.5, 0.5),
        Transform::new(Vec3::new(0.0, 12.25, -10.0)).with_scale(Vec3::new(1.0, 5.0, 1.0)),
        Vec3::new(1.0, 5.0, 1.0),
        RigidBody::new(10.0, true),
    );

    let mut hinge = Joint::hinge(
        BodyHandle::from_id(ceiling.id()),
        BodyHandle::from_id(pendulum.id()),
        Vec3::new(0.0, -0.25, 0.0), // tavan yerel çıpası (alt yüzey)
        Vec3::new(0.0, 2.5, 0.0),   // sarkaç yerel çıpası (üst yüzey)
        Vec3::new(0.0, 0.0, 1.0),   // Z ekseni etrafında dönsün
    );
    if let JointData::Hinge(ref mut data) = hinge.data {
        data.use_motor = false;
        data.use_limits = true;
        data.lower_limit = -PI / 2.0;
        data.upper_limit = PI / 2.0;
    }
    phys_world.joints.push(hinge);

    // --- BALL-SOCKET ZİNCİR ---
    let chain_start = Vec3::new(-5.0, 15.0, -10.0);
    let mut prev_ent = spawn_box(
        world,
        &cube_mesh,
        box_mat.clone(),
        Transform::new(chain_start),
        Vec3::new(1.0, 1.0, 1.0),
        RigidBody::new_static(),
    );
    let link_mat = box_mat
        .clone()
        .with_pbr(Vec4::new(0.0, 1.0, 0.0, 1.0), 0.5, 0.5);
    for i in 1..=6 {
        // Dümdüz aşağı sırala, kopmasınlar.
        let link_pos = chain_start - Vec3::new(0.0, i as f32, 0.0);
        let link_ent = spawn_box(
            world,
            &cube_mesh,
            link_mat.clone(),
            Transform::new(link_pos),
            Vec3::new(1.0, 1.0, 1.0),
            RigidBody::new(2.0, true),
        );
        let mut ball_joint = Joint::ball_socket(
            BodyHandle::from_id(prev_ent.id()),
            BodyHandle::from_id(link_ent.id()),
            Vec3::new(0.0, -0.5, 0.0),
            Vec3::new(0.0, 0.5, 0.0),
        );
        if let JointData::BallSocket(ref mut data) = ball_joint.data {
            data.use_cone_limit = true;
            data.cone_limit_angle = PI / 4.0; // 45 derece limit
        }
        phys_world.joints.push(ball_joint);
        prev_ent = link_ent;
    }

    // --- SLIDER (MOTORLU KAYAN PLATFORM) ---
    let slider_base = spawn_box(
        world,
        &cube_mesh,
        box_mat.clone(),
        Transform::new(Vec3::new(5.0, 5.0, -10.0)).with_scale(Vec3::new(0.5, 0.5, 0.5)),
        Vec3::new(0.25, 0.25, 0.25),
        RigidBody::new_static(),
    );
    let slider_plat = spawn_box(
        world,
        &cube_mesh,
        box_mat
            .clone()
            .with_pbr(Vec4::new(0.0, 0.0, 1.0, 1.0), 0.5, 0.5),
        Transform::new(Vec3::new(5.0, 5.0, -10.0)).with_scale(Vec3::new(4.0, 0.5, 2.0)),
        Vec3::new(2.0, 0.25, 1.0),
        RigidBody::new(20.0, true),
    );

    let mut slider_joint = Joint::slider(
        BodyHandle::from_id(slider_base.id()),
        BodyHandle::from_id(slider_plat.id()),
        Vec3::ZERO,
        Vec3::ZERO,
        Vec3::new(1.0, 0.0, 0.0), // X ekseninde kayar
    );
    if let JointData::Slider(ref mut data) = slider_joint.data {
        data.use_limits = true;
        data.lower_limit = -5.0;
        data.upper_limit = 5.0;
        data.use_motor = true;
        data.motor_target_velocity = 3.0; // kendiliğinden kaysın
        data.motor_max_force = 500.0;
    }
    phys_world.joints.push(slider_joint);

    // --- Serbest düşen kutular (bir kısmı su havuzuna -2..2 içine düşecek şekilde hizalı) ---
    for i in 0..10 {
        let pos = Vec3::new(
            (i as f32 % 3.0) - 1.0,
            5.0 + (i as f32) * 1.5, // yukarıdan başlasın ki hemen görünsün
            (i as f32 % 2.0) - 0.5,
        );
        // Yoğunluk suya göre ayarlı: mass = 500 (ahşap yoğunluğu) → suda gerçekçi yüzer.
        spawn_box(
            world,
            &cube_mesh,
            box_mat.clone(),
            Transform::new(pos),
            Vec3::new(1.0, 1.0, 1.0),
            RigidBody::new(500.0, true),
        );
    }

    // --- GPU SOFT BODY (JELLO) — 3×3×3 tetrahedron ızgarası ---
    let mut soft_body = SoftBodyMesh::new(1000.0, 0.3)
        .expect("geçerli Young modülü ve Poisson oranı ile SoftBodyMesh oluşturulmalı"); // v=0.5 tekilliğini önle
    soft_body.damping = 5.0; // kararlılık için yüksek sönüm
    let grid_size = 3;
    let spacing = 0.8;
    let offset = (grid_size as f32 - 1.0) * spacing / 2.0;

    // Düğümleri ekle (node başına 2 kg)
    for x in 0..grid_size {
        for y in 0..grid_size {
            for z in 0..grid_size {
                let pos = Vec3::new(
                    x as f32 * spacing - offset,
                    y as f32 * spacing - offset + 15.0, // yukarıdan başlasın
                    z as f32 * spacing - offset,
                );
                soft_body.add_node(pos, 2.0);
            }
        }
    }

    // 1B indeks yardımcısı
    let idx = |x, y, z| -> u32 { (x * grid_size * grid_size + y * grid_size + z) as u32 };

    // Elemanlar (tetrahedronlar) — komşu düğümleri voksellere bağla, her vokseli 5 tetrahedrona böl.
    for x in 0..grid_size - 1 {
        for y in 0..grid_size - 1 {
            for z in 0..grid_size - 1 {
                let i0 = idx(x, y, z);
                let i1 = idx(x + 1, y, z);
                let i2 = idx(x, y + 1, z);
                let i3 = idx(x, y, z + 1);
                let i4 = idx(x + 1, y + 1, z);
                let i5 = idx(x + 1, y, z + 1);
                let i6 = idx(x, y + 1, z + 1);
                let i7 = idx(x + 1, y + 1, z + 1);

                let _ = soft_body.add_element(i0, i1, i2, i3);
                let _ = soft_body.add_element(i1, i4, i2, i7);
                let _ = soft_body.add_element(i1, i3, i5, i7);
                let _ = soft_body.add_element(i2, i3, i6, i7);
                let _ = soft_body.add_element(i1, i2, i3, i7);
            }
        }
    }

    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 15.0, 0.0)),
        EntityName("Jello Cube".into()),
        soft_body,
    ));

    world.insert_resource(phys_world);
    world.insert_resource(asset_manager);
    world.insert_resource(gizmo::renderer::Gizmos::default());

    DemoState {
        camera_speed: 15.0,
        camera_pitch: -0.2,
        camera_yaw: -FRAC_PI_2,
        camera_pos: Vec3::new(0.0, 10.0, 20.0),
        time: 0.0,
    }
}

fn update(world: &mut World, state: &mut DemoState, dt: f32, input: &Input) {
    state.time += dt;

    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);
    let mut cam_pos = Vec3::ZERO;

    if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Camera>)>() {
        for (_, (mut transform, mut camera)) in q.iter_mut() {
            let (dx, dy) = input.mouse_delta();
            let sensitivity = 0.002;

            // Yalnız sağ tık basılıyken kamerayı döndür (serbest fare kaymasını önler).
            if input.is_mouse_button_pressed(mouse::RIGHT) {
                state.camera_yaw -= dx * sensitivity;
                state.camera_pitch -= dy * sensitivity;
                state.camera_pitch = state.camera_pitch.clamp(-1.5, 1.5);
            }

            // Bakış/hareket yönü — paylaşılan kamera yardımcısından (elle trig yok).
            let forward = Camera::forward_from(state.camera_yaw, state.camera_pitch);
            let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
            let up = Vec3::new(0.0, 1.0, 0.0);

            // Yaw/pitch → oryantasyon quaternion'u.
            let yaw_rot = Quat::from_rotation_y(-state.camera_yaw + FRAC_PI_2);
            let pitch_rot = Quat::from_rotation_x(state.camera_pitch);
            transform.rotation = yaw_rot * pitch_rot;

            let speed = state.camera_speed
                * dt
                * if input.is_key_pressed(KeyCode::ShiftLeft as u32) {
                    3.0
                } else {
                    1.0
                };

            if input.is_key_pressed(KeyCode::KeyW as u32) {
                state.camera_pos += forward * speed;
            }
            if input.is_key_pressed(KeyCode::KeyS as u32) {
                state.camera_pos -= forward * speed;
            }
            if input.is_key_pressed(KeyCode::KeyA as u32) {
                state.camera_pos -= right * speed;
            }
            if input.is_key_pressed(KeyCode::KeyD as u32) {
                state.camera_pos += right * speed;
            }
            if input.is_key_pressed(KeyCode::Space as u32) {
                state.camera_pos += up * speed;
            }

            transform.position = state.camera_pos;
            transform.update_local_matrix();

            camera.yaw = state.camera_yaw;
            camera.pitch = state.camera_pitch;

            cam_forward = forward;
            cam_pos = transform.position;
        }
    }

    // Force Push (sol tık) — bakılan dinamik cisme ileri kuvvet uygula.
    if input.is_mouse_button_pressed(mouse::LEFT) {
        if let Some(phys) = world.get_resource::<PhysicsWorld>() {
            let ray = gizmo::physics::raycast::Ray::new(cam_pos, cam_forward);
            if let Some(hit) = phys.raycast(&ray, 50.0) {
                // SAFETY: tek-thread'li demo; Velocity bileşeni tutulan PhysicsWorld kaynak
                // guard'ından ayrık (disjoint).
                if let Some(mut q) =
                    unsafe { world.query_unchecked::<(Mut<Velocity>, &RigidBody)>() }
                {
                    if let Some((mut vel, rb)) = q.get_mut(hit.entity.id()) {
                        if rb.is_dynamic() {
                            vel.linear += cam_forward * 20.0; // ileri fırlat
                        }
                    }
                }
            }
        }
    }

    // CPU fizik adımı (Gizmo ECS entegrasyonu) + collider debug çizimi.
    gizmo::systems::cpu_physics_step_system(world, dt);
    gizmo::systems::physics_debug_system(world);
}

fn render(
    world: &mut World,
    _state: &DemoState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<DemoState>::new("Gizmo Engine - CPU Physics", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
