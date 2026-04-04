use winit::{
    event::{Event, WindowEvent, DeviceEvent, ElementState},
    keyboard::{PhysicalKey, KeyCode},
};

use yelbegen::prelude::*;
use yelbegen::renderer::EngineUniforms;
use std::collections::HashSet;

use yelbegen::app::App;
use std::sync::Arc;
use yelbegen::renderer::components::{Mesh, Material, MeshRenderer, Camera};
use yelbegen::renderer::asset::AssetManager;

// ======================== FİZİK SİSTEMLERİ ========================

/// Yerçekimi uygular ve pozisyonları günceller.
/// Velocity bileşeni olan entity'ler üzerinde iterasyon yapar.
pub fn physics_movement_system(world: &mut World, dt: f32) {
    if let (Some(mut trans), Some(mut vel), Some(rbs)) =
        (world.borrow_mut::<Transform>(), world.borrow_mut::<Velocity>(), world.borrow::<RigidBody>())
    {
        const GRAVITY: f32 = -9.81;
        for i in 0..vel.dense.len() {
            let entity_id = vel.entity_dense[i];

            if let Some(rb) = rbs.get(entity_id) {
                if !rb.use_gravity {
                    continue;
                }
            }

            let mut linear = vel.dense[i].linear;
            linear.y += GRAVITY * dt;
            vel.dense[i].linear = linear;

            if let Some(t) = trans.get_mut(entity_id) {
                t.position += linear * dt;
            }
        }
    }
}

/// AABB çarpışma tespiti ve çözümleme.
/// Entity ID tabanlı güvenli erişim kullanır.
pub fn physics_collision_system(world: &mut World, _dt: f32) {
    let mut collision_resolutions: Vec<(u32, Vec3, f32)> = Vec::new();

    if let (Some(trans), Some(colliders), Some(rbs)) =
        (world.borrow::<Transform>(), world.borrow::<Collider>(), world.borrow::<RigidBody>())
    {
        // Collider bileşeni olan entity'ler üzerinde çift döngü
        let entities: Vec<u32> = colliders.entity_dense.clone();

        for i in 0..entities.len() {
            let e1 = entities[i];
            for j in (i + 1)..entities.len() {
                let e2 = entities[j];

                let (pos1, pos2) = match (trans.get(e1), trans.get(e2)) {
                    (Some(t1), Some(t2)) => (t1.position, t2.position),
                    _ => continue,
                };
                let (col1, col2) = match (colliders.get(e1), colliders.get(e2)) {
                    (Some(c1), Some(c2)) => (c1, c2),
                    _ => continue,
                };
                let (_rb1, rb2) = match (rbs.get(e1), rbs.get(e2)) {
                    (Some(r1), Some(r2)) => (r1, r2),
                    _ => continue,
                };

                if let (
                    yelbegen::physics::ColliderShape::Aabb(aabb1),
                    yelbegen::physics::ColliderShape::Aabb(aabb2),
                ) = (&col1.shape, &col2.shape)
                {
                    // AABB kesişim testi
                    let a_min = pos1 - aabb1.half_extents;
                    let a_max = pos1 + aabb1.half_extents;
                    let b_min = pos2 - aabb2.half_extents;
                    let b_max = pos2 + aabb2.half_extents;

                    let intersects = a_min.x <= b_max.x && a_max.x >= b_min.x
                        && a_min.y <= b_max.y && a_max.y >= b_min.y
                        && a_min.z <= b_max.z && a_max.z >= b_min.z;

                    if intersects {
                        // Dinamik objeyi (e1) statik (e2) üzerine it
                        let resolved_y = (pos2.y + aabb2.half_extents.y) + aabb1.half_extents.y;
                        let resolved_pos = Vec3::new(pos1.x, resolved_y, pos1.z);
                        collision_resolutions.push((e1, resolved_pos, rb2.restitution));
                    }
                }
            }
        }
    }

    // Çözümleme: pozisyon düzelt + hız yansıt
    for (entity_id, new_pos, restitution) in &collision_resolutions {
        if let Some(mut t) = world.borrow_mut::<Transform>() {
            if let Some(trans) = t.get_mut(*entity_id) {
                trans.position = *new_pos;
            }
        }
        if let Some(mut v) = world.borrow_mut::<Velocity>() {
            if let Some(vel) = v.get_mut(*entity_id) {
                vel.linear.y = -vel.linear.y * restitution;
            }
        }
    }
}

// ======================== OYUN DURUMU ========================

struct GameState {
    mouse_pressed: bool,
    keys: HashSet<KeyCode>,
    bouncing_box_id: u32,
    player_id: u32,
}

// ======================== ANA FONKSİYON ========================

fn main() {
    let mut app = App::new("Yelbegen Engine (Faz 12)", 1280, 720);

    // 1. SETUP
    app = app.set_setup(|world, renderer| {
        println!("Yelbegen Engine: Faz 12 — Temiz Mimari");

        // Kaplamalar
        let tex = AssetManager::load_texture(&renderer.device, &renderer.queue, "assets/brick.jpg");
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = renderer.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let tbind = Arc::new(renderer.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Mesh TBIND"),
            layout: &renderer.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        }));

        // Uniform buffer üretici
        let create_renderer = || -> MeshRenderer {
            let ubuf = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Mesh UBUF"),
                size: std::mem::size_of::<EngineUniforms>() as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let ubind = renderer.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Mesh UBIND"),
                layout: &renderer.uniform_bind_group_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }],
            });
            MeshRenderer::new(ubuf, ubind)
        };

        // --- Zıplayan Kutu (Suzanne) ---
        let bouncing_box = world.spawn();
        world.add_component(bouncing_box, Transform::new(Vec3::new(0.0, 5.0, -8.0)));
        world.add_component(bouncing_box, Velocity::new(Vec3::new(3.0, 0.0, 0.0)));
        world.add_component(bouncing_box, Collider::new_aabb(0.5, 0.5, 0.5));
        world.add_component(bouncing_box, RigidBody::new(1.0, 0.8, 0.2, true));
        world.add_component(bouncing_box, AssetManager::load_obj(&renderer.device, "demo/assets/suzanne.obj"));
        world.add_component(bouncing_box, Material::new(tbind.clone()));
        world.add_component(bouncing_box, create_renderer());

        // --- Zemin (Suzanne geçici) ---
        let ground = world.spawn();
        world.add_component(ground, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
        world.add_component(ground, Velocity::new(Vec3::ZERO));
        world.add_component(ground, Collider::new_aabb(10.0, 1.0, 10.0));
        world.add_component(ground, RigidBody::new_static());
        world.add_component(ground, AssetManager::load_obj(&renderer.device, "demo/assets/suzanne.obj"));
        world.add_component(ground, Material::new(tbind.clone()));
        world.add_component(ground, create_renderer());

        // --- Player (Kamera) ---
        let player = world.spawn();
        world.add_component(player, Transform::new(Vec3::new(0.0, 5.0, 15.0)));
        world.add_component(player, Camera::new(
            std::f32::consts::FRAC_PI_4, 0.1, 100.0,
            -std::f32::consts::FRAC_PI_2, -0.3, true,
        ));

        GameState {
            mouse_pressed: false,
            keys: HashSet::new(),
            bouncing_box_id: bouncing_box.id(),
            player_id: player.id(),
        }
    });

    // 2. INPUT HOOK
    app = app.set_input(|world, state, event| {
        let mut handled = false;
        match event {
            Event::WindowEvent { event: WindowEvent::KeyboardInput { event: kb_event, .. }, .. } => {
                if let PhysicalKey::Code(keycode) = kb_event.physical_key {
                    if kb_event.state == ElementState::Pressed {
                        state.keys.insert(keycode);
                    } else {
                        state.keys.remove(&keycode);
                    }
                }
                handled = true;
            }
            Event::WindowEvent { event: WindowEvent::MouseInput { state: m_state, button: winit::event::MouseButton::Right, .. }, .. } => {
                state.mouse_pressed = *m_state == ElementState::Pressed;
                handled = true;
            }
            Event::DeviceEvent { event: DeviceEvent::MouseMotion { delta }, .. } => {
                if state.mouse_pressed {
                    if let Some(mut cameras) = world.borrow_mut::<Camera>() {
                        if let Some(cam) = cameras.get_mut(state.player_id) {
                            cam.yaw += delta.0 as f32 * 0.005;
                            cam.pitch -= delta.1 as f32 * 0.005;
                            cam.pitch = cam.pitch.clamp(-1.5, 1.5);
                        }
                    }
                    handled = true;
                }
            }
            _ => {}
        }
        handled
    });

    // 3. UPDATE HOOK
    app = app.set_update(|world, state, dt| {
        let speed = 10.0 * dt;

        let mut f = Vec3::ZERO;
        let mut r = Vec3::ZERO;

        if let Some(cameras) = world.borrow::<Camera>() {
            if let Some(cam) = cameras.get(state.player_id) {
                f = cam.get_front();
                r = cam.get_right();
            }
        }

        if let Some(mut trans) = world.borrow_mut::<Transform>() {
            // Kamera Hareketi
            if let Some(t) = trans.get_mut(state.player_id) {
                if state.keys.contains(&KeyCode::KeyW) { t.position += f * speed; }
                if state.keys.contains(&KeyCode::KeyS) { t.position -= f * speed; }
                if state.keys.contains(&KeyCode::KeyA) { t.position -= r * speed; }
                if state.keys.contains(&KeyCode::KeyD) { t.position += r * speed; }
                if state.keys.contains(&KeyCode::KeyQ) { t.position.y -= speed; }
                if state.keys.contains(&KeyCode::KeyE) { t.position.y += speed; }
            }

            // Suzanne'ı Y ekseni etrafında döndür
            if let Some(t) = trans.get_mut(state.bouncing_box_id) {
                // Zamanı (veya benzeri bir delta_time bazlı değeri) kullanarak Quaternion üretiyoruz
                // Şimdilik sürekli dönüş sağlamak için yeni bir quaternionla mevcut olanı çarpıyoruz
                let rot_delta = Quat::from_axis_angle(Vec3::new(0.0, 1.0, 0.0), dt * 2.0); // Saniyede 2 radyan
                t.rotation = (t.rotation * rot_delta).normalize();
            }
        }
    });

    // 4. ECS SİSTEMLERİ
    app = app.add_system(physics_movement_system);
    app = app.add_system(physics_collision_system);

    // 5. EGUI ARAYÜZ
    app = app.set_ui(|world, state, ctx| {
        egui::Window::new("⚙️ Yelbegen Engine Inspector").show(ctx, |ui| {
            ui.heading("Aydınlatma ve Simülasyon");
            ui.separator();

            if let Some(mut rbs) = world.borrow_mut::<RigidBody>() {
                if let Some(rb) = rbs.get_mut(state.bouncing_box_id) {
                    ui.horizontal(|ui| {
                        ui.label("Kütle: ");
                        ui.add(egui::Slider::new(&mut rb.mass, 0.0..=10.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Sekme: ");
                        ui.add(egui::Slider::new(&mut rb.restitution, 0.0..=1.0));
                    });
                }
            }

            if ui.button("🔄 Başa Sar").clicked() {
                if let Some(mut trans) = world.borrow_mut::<Transform>() {
                    if let Some(t) = trans.get_mut(state.bouncing_box_id) {
                        t.position = Vec3::new(0.0, 5.0, -8.0);
                    }
                }
                if let Some(mut vels) = world.borrow_mut::<Velocity>() {
                    if let Some(v) = vels.get_mut(state.bouncing_box_id) {
                        v.linear = Vec3::new(3.0, 0.0, 0.0);
                    }
                }
            }

            ui.add_space(10.0);
            if let Some(trans) = world.borrow::<Transform>() {
                if let Some(t) = trans.get(state.player_id) {
                    ui.label(format!("Kamera: {:.1}, {:.1}, {:.1}", t.position.x, t.position.y, t.position.z));
                }
            }
            ui.label("WASD: hareket | Q/E: dikey | Sağ Tık: bakış");
        });
    });

    // 6. RENDER HOOK
    app = app.set_render(|world, state, encoder, view, renderer, light_time| {
        let aspect = if renderer.size.height > 0 {
            renderer.size.width as f32 / renderer.size.height as f32
        } else {
            1.0
        };

        let mut proj = Mat4::perspective(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);
        let mut view_mat = Mat4::translation(Vec3::ZERO);

        if let (Some(cameras), Some(transforms)) = (world.borrow::<Camera>(), world.borrow::<Transform>()) {
            if let (Some(cam), Some(trans)) = (cameras.get(state.player_id), transforms.get(state.player_id)) {
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(trans.position);
            }
        }

        let g_light_x = (light_time * 2.0).sin() * 2.0;

        // Uniform buffer güncelleme
        if let (Some(meshes), Some(renderers), Some(positions), Some(colliders)) =
            (world.borrow::<Mesh>(), world.borrow::<MeshRenderer>(), world.borrow::<Transform>(), world.borrow::<Collider>())
        {
            for entity_id in &renderers.entity_dense {
                let e = *entity_id;
                if let (Some(mesh), Some(mesh_ren), Some(trans)) = (meshes.get(e), renderers.get(e), positions.get(e)) {
                    let mut col_scale = Vec3::new(1.0, 1.0, 1.0);
                    if let Some(col) = colliders.get(e) {
                        if let yelbegen::physics::ColliderShape::Aabb(aabb) = &col.shape {
                            if e == state.bouncing_box_id {
                                col_scale = aabb.half_extents;
                            }
                        }
                    }

                    // T * R * S
                    let trans_mat = trans.model_matrix();
                    
                    // Box collision box boyutları kadar büyütmek istiyorsak özel bir scale matrisi ile çarpabiliriz
                    // Ama motor prensibi olarak scale Transform'da olmalı
                    let extra_scale = Mat4::scale(col_scale);

                    let center_mat = Mat4::translation(mesh.center_offset);
                    
                    // Önce kendi merkezine git, ekstra boyut ekle, sonra asıl Transform matrisini uygula
                    let model = trans_mat * extra_scale * center_mat;

                    let mvp = proj * view_mat * model;

                    let uniform_data = EngineUniforms {
                        mvp: mvp.to_cols_array_2d(),
                        light_dir: [-g_light_x, 1.0, 1.0, 0.0],
                    };
                    renderer.queue.write_buffer(&mesh_ren.ubuf, 0, bytemuck::cast_slice(&[uniform_data]));
                }
            }
        }

        // Render pass: ECS üzerindeki tüm Mesh+Material bileşenlerini çiz
        let meshes_ref = world.borrow::<Mesh>();
        let materials_ref = world.borrow::<Material>();
        let renderers_ref = world.borrow::<MeshRenderer>();

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Main Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.15, b: 0.20, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.depth_texture_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        render_pass.set_pipeline(&renderer.render_pipeline);

        if let (Some(meshes), Some(materials), Some(renderers)) = (&meshes_ref, &materials_ref, &renderers_ref) {
            for entity_id in &renderers.entity_dense {
                let e = *entity_id;
                if let (Some(mesh), Some(mat), Some(mesh_ren)) = (meshes.get(e), materials.get(e), renderers.get(e)) {
                    render_pass.set_bind_group(0, &mesh_ren.ubind, &[]);
                    render_pass.set_bind_group(1, &mat.bind_group, &[]);
                    render_pass.set_vertex_buffer(0, mesh.vbuf.slice(..));
                    render_pass.draw(0..mesh.vertex_count, 0..1);
                }
            }
        }
    });

    app.run();
}
