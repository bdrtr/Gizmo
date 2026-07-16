//! # Gelişmiş Fizik Demosu — yıkım / ragdoll / halat vitrini
//!
//! Motorun fizik yeteneklerini üç bağımsız sahneyle sergiler:
//!   * **Yıkım** — kırılabilir (`Breakable`) cam duvar; sol-tık ışın-izle + `Explosion`.
//!   * **Ragdoll** — `Joint::fixed` ile zincirlenmiş uzuvlar (X = tüm eklemleri kopar).
//!   * **Halat** — `Rope` düğümleri her kare görsel kürelere kopyalanır.
//!
//! İdiom notları (NEYİN motora, NEYİN demoya ait olduğu konusunda dürüst olalım):
//!   * **`Prefab` + `auto_box_collider`** — cam-duvar blokları TEK blueprint'ten; kutu collider
//!     `Transform.scale`'den OTOMATİK türetilir (boyutu iki kez yazma). Halat düğümleri de
//!     fizik-siz görsel-only `Prefab`.
//!   * **`spawn_bundle`** — kamera/zemin/ragdoll-uzuvları tek çağrıda kurulur (dağınık
//!     `add_component` zinciri yok). Uzuvlarda BİLEREK `RigidBodyBundle` DEĞİL, elle
//!     `RigidBody`+`Velocity`+`Collider` kullanılır: ragdoll+joint çözücü ataletten hassastır,
//!     bundle'ın collider'dan atalet-türetmesi zincirin çözüm dengesini bozabilir → orijinal
//!     varsayılan atalet KORUNUR.
//!   * **Fizik ELLE sürülür** (`cpu_physics_step_system` + fracture/explosion/debug her kare) —
//!     bu demo `PhysicsPlugin` KULLANMAZ; joint/raycast için kendi `PhysicsWorld` kaynağını tutar.
//!     Bu yüzden `DespawnAfter`/`DespawnBelowY` EKLENMEZ: onları işleyecek `LifetimePlugin`
//!     schedule'da yok → sessizce hiçbir şey yapmazlardı.
//!   * **Sahne render = `default_render_pass` DOĞRUDAN** — motor `with_scene_render()` tek-satır
//!     kısayolunu sunar ama onu KULLANMIYORUZ: o kısayol SSR/SSGI/volumetric'i (ve gizmo debug
//!     çizgilerini) kapatır; bu vitrin hız/iz gizmo'larını ve efektleri AÇIK ister.
//!
//! ## Kontroller
//!   * **Sağ-tık + fare / WASD** — kamerayı gez · **Space** — yüksel · **Shift** — hızlan
//!   * **Sol-tık (basılı)** — nişan aldığın yerde patlama · **X** — ragdoll eklemlerini kopar

use gizmo::core::input::mouse;
use gizmo::physics::components::{Breakable, CollisionLayer, Explosion};
use gizmo::physics::joints::Joint;
use gizmo::physics::rope::Rope;
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::BodyHandle;
use gizmo::prelude::*;

/// Hata-ayıklama izi: her karenin Transform'unu tutar, gizmo ile geçmiş kutuları çizilir.
#[derive(Clone)]
struct GhostTrail {
    history: std::collections::VecDeque<Transform>,
    max_frames: usize,
}
gizmo::core::impl_component!(GhostTrail);

struct DemoState {
    camera_speed: f32,
    camera_pitch: f32,
    camera_yaw: f32,
    camera_pos: Vec3,
    rope: Option<Rope>,
    // Fracture ile üretilen chunk'lara görsel mesh/material vermek için (runtime).
    sphere_mesh: Mesh,
    chunk_material: Material,
}

fn setup(world: &mut World, renderer: &Renderer) -> DemoState {
    let mut asset_manager = AssetManager::new();

    // Kamera — Transform + Camera + isim tek spawn_bundle'da.
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 5.0, 15.0)).with_rotation(Quat::from_rotation_x(-0.2)),
        Camera::new(
            std::f32::consts::FRAC_PI_3,
            0.1,
            5000.0,
            -std::f32::consts::FRAC_PI_2,
            -0.2,
            true,
        ),
        EntityName("Main Camera".into()),
    ));

    // Dokular — zemin/blok/uzuv/düğüm hepsi aynı damalı dokuyu paylaşır; skybox beyaz.
    let checker = asset_manager.create_checkerboard_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    let white_tex = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );

    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let plane_mesh = AssetManager::create_plane(&renderer.device, 100.0);
    let sphere_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 16, 16);

    // Skybox (prosedürel, unlit ters küre).
    world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        AssetManager::create_sphere(&renderer.device, 2000.0, 32, 32),
        Material::new(white_tex.clone())
            .with_unlit(Vec4::ONE)
            .with_skybox(),
        MeshRenderer::new(),
    ));

    // Işık.
    world.spawn_bundle((
        Transform::new(Vec3::new(0.0, 20.0, 0.0)),
        PointLight::new(Vec3::ONE, 500.0, 50.0),
    ));

    // Zemin — statik düzlem collider (RigidBodyBundle::static_body → RigidBody+Velocity+Collider).
    world.spawn_bundle((
        Transform::new(Vec3::ZERO),
        plane_mesh.clone(),
        Material::new(checker.clone()).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.8, 0.1),
        MeshRenderer::new(),
        RigidBodyBundle::static_body().with_collider(Collider::plane(Vec3::Y, 0.0)),
    ));

    // Fizik dünyası — joint'ler ve ışın-izleme (raycast) burada tutulur.
    let mut phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));

    // ── 1. YIKIM: kırılabilir cam duvar (5×5 blok) ──
    // TEK blueprint; kutu collider `Transform.scale`'den otomatik (0.5 → box_collider(0.5)).
    // Gülle/mermi gibi per-örnek hız gerekmez → Prefab uygun.
    let glass = Prefab::new(
        cube_mesh.clone(),
        Material::new(checker.clone()).with_pbr(Vec4::new(0.3, 0.6, 1.0, 0.5), 0.2, 0.0),
    )
    .with_body(RigidBodyBundle::dynamic(10.0))
    .auto_box_collider();

    for y in 0..5 {
        for x in 0..5 {
            // Zemin üstü y=0.1; blok yarı-boyu 0.5 → y=0.6'dan başla. 1.02 boşluk mikro-çarpışmayı önler.
            let pos = Vec3::new(-5.0 + (x as f32) * 1.02, 0.6 + (y as f32) * 1.02, -5.0);
            let brick = glass.spawn(world, Transform::new(pos).with_scale(Vec3::splat(0.5)));
            // Hata-ayıklama izi (gizmo geçmiş kutuları).
            world.add_component(
                brick,
                GhostTrail {
                    history: std::collections::VecDeque::new(),
                    max_frames: 10,
                },
            );
            // Yüksek eşik → istifleme sırasında kırılmaz, ancak güçlü darbede (patlama) kırılır.
            world.add_component(
                brick,
                Breakable {
                    threshold: 400.0,
                    max_pieces: 4,
                    ..Default::default()
                },
            );
        }
    }

    // ── 2. RAGDOLL: sabit eklemlerle zincirlenmiş uzuvlar ──
    // Her uzuv spawn_bundle ile tek çağrıda kurulur. Joint'ler ENTITY id'sine göre bağlanır,
    // bu yüzden spawn SIRASI (ebeveyn önce) ve dönen entity KORUNMALIDIR.
    let mut prev_ent: Option<Entity> = None;
    for i in 0..4 {
        let pos = Vec3::new(5.0, 8.0 - (i as f32) * 1.2, 0.0);
        let limb = world.spawn_bundle((
            Transform::new(pos).with_scale(Vec3::new(0.2, 0.5, 0.2)),
            cube_mesh.clone(),
            Material::new(checker.clone()).with_pbr(Vec4::new(0.8, 0.2, 0.2, 1.0), 0.5, 0.5),
            MeshRenderer::new(),
            // Uzuvları layer 1'e koy + layer 1 çarpışmasını yok say → kendi-kendine çarpışma patlamasını önle.
            Collider::box_collider(Vec3::new(0.2, 0.5, 0.2)).with_layer(CollisionLayer {
                layer: 1,
                mask: !(1 << 1),
            }),
            // BİLEREK RigidBodyBundle DEĞİL: joint çözücü ataletten hassas → varsayılan atalet korunur.
            RigidBody::new(5.0, true),
            Velocity::default(),
        ));

        if let Some(parent) = prev_ent {
            let fixed = Joint::fixed(
                BodyHandle::from_id(parent.id()),
                BodyHandle::from_id(limb.id()),
                Vec3::new(0.0, -0.6, 0.0),
                Vec3::new(0.0, 0.6, 0.0),
            )
            .with_break_force(f32::MAX, f32::MAX);
            phys_world.joints.push(fixed);
        }
        prev_ent = Some(limb);
    }

    // ── 3. HALAT: her düğüm için görsel-only küre (fizik `Rope` içinde, CPU'da adımlanır) ──
    let rope = Rope::new(
        Vec3::new(-5.0, 10.0, 5.0),
        Vec3::new(1.0, -0.2, 0.0),
        20,
        0.5,
        1.0,
        true,
        false,
    );

    // Görsel-only Prefab (fizik gövdesi YOK) — konumlar update'te halat düğümlerinden kopyalanır.
    let rope_node = Prefab::new(
        sphere_mesh.clone(),
        Material::new(checker.clone()).with_pbr(Vec4::new(1.0, 0.8, 0.1, 1.0), 0.5, 0.5),
    );
    for _ in 0..rope.nodes.len() {
        let node = rope_node.spawn(
            world,
            Transform::new(Vec3::ZERO).with_scale(Vec3::splat(0.2)),
        );
        world.add_component(node, EntityName("RopeNode".into()));
    }

    world.insert_resource(phys_world);
    world.insert_resource(asset_manager);

    DemoState {
        camera_speed: 15.0,
        camera_pitch: -0.2,
        camera_yaw: -std::f32::consts::FRAC_PI_2,
        camera_pos: Vec3::new(0.0, 5.0, 15.0),
        rope: Some(rope),
        sphere_mesh,
        chunk_material: Material::new(checker).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.5, 0.5),
    }
}

fn update(world: &mut World, state: &mut DemoState, dt: f32, input: &Input) {
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);
    let mut cam_pos = Vec3::ZERO;

    // --- serbest-uçuş kamera (sağ-tık basılıyken fare-look) ---
    if let Some(mut q) = world.query_mut::<(Mut<Transform>, Mut<Camera>)>() {
        for (_, (mut transform, mut camera)) in q.iter_mut() {
            let sensitivity = 0.002;
            let (dx, dy) = input.mouse_delta();

            if input.is_mouse_button_pressed(mouse::RIGHT) {
                state.camera_yaw -= dx * sensitivity;
                state.camera_pitch -= dy * sensitivity;
                state.camera_pitch = state.camera_pitch.clamp(-1.5, 1.5);
            }

            let fx = state.camera_yaw.cos() * state.camera_pitch.cos();
            let fy = state.camera_pitch.sin();
            let fz = state.camera_yaw.sin() * state.camera_pitch.cos();
            let forward = Vec3::new(fx, fy, fz).normalize();
            let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
            let up = Vec3::new(0.0, 1.0, 0.0);

            let yaw_rot = Quat::from_rotation_y(-state.camera_yaw + std::f32::consts::FRAC_PI_2);
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

    // --- Sol-tık: ışın-izle + patlama (yıkımı tetikle) ---
    if input.is_mouse_button_pressed(mouse::LEFT) {
        let mut hit_pos = None;
        if let Some(phys) = world.get_resource::<PhysicsWorld>() {
            let ray = gizmo::physics::raycast::Ray::new(cam_pos, cam_forward);
            if let Some(hit) = phys.raycast(&ray, 100.0) {
                hit_pos = Some(hit.point);
            }
        }

        if let Some(pos) = hit_pos {
            world.spawn_bundle((
                Transform::new(pos),
                Explosion {
                    force_radius: 5.0,
                    force: 5000.0,
                    is_active: true,
                    ..Default::default()
                },
            ));
        }
    }

    // --- X: tüm eklemleri kopar ---
    if input.is_key_pressed(KeyCode::KeyX as u32) {
        if let Some(mut phys_world) = world.get_resource_mut::<PhysicsWorld>() {
            for joint in &mut phys_world.joints {
                joint.is_broken = true;
            }
        }
    }

    // --- gizmo debug: hız vektörleri (yeşil) + iz bırakma (yarı-saydam) ---
    if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
        if let Some(q) = world.query::<(&Velocity, &Transform, &RigidBody)>() {
            for (_, (vel, trans, _rb)) in q.iter() {
                gizmos.draw_line(
                    trans.position,
                    trans.position + vel.linear * 0.1,
                    [0.0, 1.0, 0.0, 1.0],
                );
            }
        }

        // GÜVENLİK: tek-thread demo; GhostTrail bileşeni, tutulan Gizmos kaynak-guard'ından ayrık.
        if let Some(mut q) =
            unsafe { world.query_unchecked::<(Mut<GhostTrail>, &Transform, &Collider)>() }
        {
            for (_, (mut ghost, trans, col)) in q.iter_mut() {
                // Bu kareyi sakla, en eskiyi at.
                ghost.history.push_front(*trans);
                if ghost.history.len() > ghost.max_frames {
                    ghost.history.pop_back();
                }

                // Geçmiş kutuları giderek sönerek çiz.
                for (i, g_trans) in ghost.history.iter().enumerate() {
                    let alpha = 1.0 - (i as f32 / ghost.max_frames as f32);
                    let color = [1.0, 1.0, 1.0, alpha * 0.5];

                    if let ColliderShape::Box(b) = &col.shape {
                        let h = b.half_extents;
                        let p0 = g_trans
                            .local_matrix
                            .transform_point3(Vec3::new(-h.x, -h.y, -h.z));
                        let p1 = g_trans
                            .local_matrix
                            .transform_point3(Vec3::new(h.x, -h.y, -h.z));
                        let p2 = g_trans
                            .local_matrix
                            .transform_point3(Vec3::new(h.x, h.y, -h.z));
                        let p3 = g_trans
                            .local_matrix
                            .transform_point3(Vec3::new(-h.x, h.y, -h.z));
                        let p4 = g_trans
                            .local_matrix
                            .transform_point3(Vec3::new(-h.x, -h.y, h.z));
                        let p5 = g_trans
                            .local_matrix
                            .transform_point3(Vec3::new(h.x, -h.y, h.z));
                        let p6 = g_trans
                            .local_matrix
                            .transform_point3(Vec3::new(h.x, h.y, h.z));
                        let p7 = g_trans
                            .local_matrix
                            .transform_point3(Vec3::new(-h.x, h.y, h.z));

                        gizmos.draw_line(p0, p1, color);
                        gizmos.draw_line(p1, p2, color);
                        gizmos.draw_line(p2, p3, color);
                        gizmos.draw_line(p3, p0, color);
                        gizmos.draw_line(p4, p5, color);
                        gizmos.draw_line(p5, p6, color);
                        gizmos.draw_line(p6, p7, color);
                        gizmos.draw_line(p7, p4, color);
                        gizmos.draw_line(p0, p4, color);
                        gizmos.draw_line(p1, p5, color);
                        gizmos.draw_line(p2, p6, color);
                        gizmos.draw_line(p3, p7, color);
                    }
                }
            }
        }
    }

    // --- fizik ELLE sürülür (bu demo PhysicsPlugin kullanmaz) ---
    gizmo::systems::cpu_physics_step_system(world, dt);
    gizmo::physics::physics_fracture_system(world, dt);
    gizmo::physics::physics_explosion_system(world, dt);
    gizmo::systems::physics::physics_debug_system(world);

    // Fracture sisteminin ürettiği chunk'lara görsel mesh/material ver.
    let mut missing = Vec::new();
    if let Some(q) = world.query::<(&RigidBody, &Collider)>() {
        let meshes = world.borrow::<MeshRenderer>();
        for (e, (rb, _)) in q.iter() {
            if meshes.get(e).is_none() && rb.mass > 0.0 {
                missing.push(e);
            }
        }
    }
    for e in missing {
        if let Some(ent) = world.get_entity(e) {
            world.add_component(ent, state.sphere_mesh.clone());
            world.add_component(ent, state.chunk_material.clone());
            world.add_component(ent, MeshRenderer::new());
        }
    }

    // --- halat adımı + görsel düğümleri güncelle ---
    if let Some(ref mut rope) = state.rope {
        rope.step(dt, Vec3::new(0.0, -9.81, 0.0));

        let mut node_idx = 0;
        if let Some(mut q) = world.query_mut::<(Mut<Transform>, &EntityName)>() {
            for (_, (mut trans, name)) in q.iter_mut() {
                if name.0 == "RopeNode" && node_idx < rope.nodes.len() {
                    trans.position = rope.nodes[node_idx].position;
                    node_idx += 1;
                }
            }
        }
    }
}

fn render(
    world: &mut World,
    _state: &DemoState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    // Tam deferred boru hattı + gizmo çizgileri açık — with_scene_render KULLANMA (efektleri kapatır).
    default_render_pass(world, encoder, view, renderer);
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();
    App::<DemoState>::new("Gizmo Engine - Advanced Physics Demo", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run()
        .expect("uygulama çalıştırılamadı");
}
