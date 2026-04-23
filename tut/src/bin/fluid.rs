use gizmo::prelude::*;

struct FluidDemo {
    cam_id: u32,
    cam_yaw: f32,
    cam_pitch: f32,
    cam_pos: Vec3,
    cam_speed: f32,
    fps_timer: f32,
    frames: u32,
    fps: f32,
}

impl FluidDemo {
    fn new() -> Self {
        Self {
            cam_id: 0,
            cam_yaw: -std::f32::consts::FRAC_PI_2,
            cam_pitch: 0.0,
            cam_pos: Vec3::new(0.0, 35.0, 75.0),
            cam_speed: 20.0,
            fps_timer: 0.0,
            frames: 0,
            fps: 0.0,
        }
    }
}

fn main() {
    App::<FluidDemo>::new("Gizmo — SPH Sıvı Simülasyonu", 1600, 900)
        .set_setup(|world, _renderer| {
            println!("══════════════════════════════════════════");
            println!("   💧 SPH Sıvı Simülasyonu Başlıyor...");
            println!("══════════════════════════════════════════");

            let mut state = FluidDemo::new();

            // Kamera
            let cam = world.spawn();
            world.add_component(
                cam,
                Transform::new(state.cam_pos)
                    .with_rotation(pitch_yaw_quat(state.cam_pitch, state.cam_yaw)),
            );
            world.add_component(
                cam,
                Camera::new(
                    std::f32::consts::FRAC_PI_3,
                    0.1,
                    500.0,
                    state.cam_yaw,
                    state.cam_pitch,
                    true,
                ),
            );
            world.add_component(cam, EntityName("Kamera".into()));
            state.cam_id = cam.id();

            // Güneş ışığı
            let sun = world.spawn();
            world.add_component(
                sun,
                Transform::new(Vec3::new(0.0, 100.0, 50.0))
                    .with_rotation(Quat::from_axis_angle(Vec3::X, -0.8)),
            );
            world.add_component(
                sun,
                DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 3.5, true),
            );

            // Gizmo debug hattı ile cam sınırlarını çiz
            world.insert_resource(gizmo::renderer::Gizmos::default());

            // 100K SPH parçacığını ECS varlığı olarak anında kopyala
            let template = world.spawn();
            world.add_component(template, gizmo::renderer::components::FluidParticle);
            world.add_component(template, gizmo::renderer::components::FluidPhase { phase: gizmo::renderer::components::FluidPhaseType::Water });
            world.add_component(template, gizmo::renderer::components::FluidHandle { gpu_index: 0 });

            if let Some(clones) = world.clone_entity(template.id(), 100_000 - 1) {
                let mut handles = world.borrow_mut::<gizmo::renderer::components::FluidHandle>();
                for (i, clone_ent) in clones.into_iter().enumerate() {
                    if let Some(h) = handles.get_mut(clone_ent.id()) {
                        h.gpu_index = (i + 1) as u32;
                    }
                }
            }
            println!("✅ ECS Orchestration: 100.000 FluidParticle anında GPU indeksleriyle doğruldu.");

            // Test ECS Interactor (Sıvı içine düşen veya hareket eden obje)
            let interactor = world.spawn();
            world.add_component(interactor, Transform::new(Vec3::new(0.0, 30.0, 0.0)));
            world.add_component(interactor, gizmo::renderer::components::FluidInteractor {
                collider_gpu_index: 0,
                buoyancy_factor: 1.0,
                radius: 8.0,
                velocity: Vec3::ZERO,
            });
            world.add_component(interactor, EntityName("Su Topu".into()));

            state
        })
        .set_update(|world, state, dt, input| {
            // FPS
            state.fps_timer += dt;
            state.frames += 1;
            if state.fps_timer >= 1.0 {
                state.fps = state.frames as f32 / state.fps_timer;
                state.frames = 0;
                state.fps_timer = 0.0;
            }

            // Kamera hareketi (WASD + QE + Shift)
            let mut speed = state.cam_speed;
            if input.is_key_pressed(KeyCode::ShiftLeft as u32) {
                speed *= 3.0;
            }

            let mut cam_move = Vec3::ZERO;
            if input.is_key_pressed(KeyCode::KeyW as u32) { cam_move.z -= 1.0; }
            if input.is_key_pressed(KeyCode::KeyS as u32) { cam_move.z += 1.0; }
            if input.is_key_pressed(KeyCode::KeyA as u32) { cam_move.x -= 1.0; }
            if input.is_key_pressed(KeyCode::KeyD as u32) { cam_move.x += 1.0; }
            if input.is_key_pressed(KeyCode::KeyQ as u32) { cam_move.y -= 1.0; }
            if input.is_key_pressed(KeyCode::KeyE as u32) { cam_move.y += 1.0; }

            if cam_move.length_squared() > 0.0 {
                cam_move = cam_move.normalize() * speed * dt;
            }

            // Fare ile kamera döndürme
            let mouse_delta = input.mouse_delta();
            if input.is_mouse_button_pressed(1) {
                state.cam_yaw -= mouse_delta.0 * 0.002;
                state.cam_pitch -= mouse_delta.1 * 0.002;
                state.cam_pitch = state.cam_pitch.clamp(-1.5, 1.5);
            }

            if let Some(tr) = world.borrow_mut::<Transform>().get_mut(state.cam_id) {
                let rot = pitch_yaw_quat(state.cam_pitch, state.cam_yaw);
                tr.rotation = rot;

                let forward = rot * Vec3::new(0.0, 0.0, -1.0);
                let right = rot * Vec3::new(1.0, 0.0, 0.0);
                let up = Vec3::Y;

                let movement = right * cam_move.x + up * cam_move.y - forward * cam_move.z;
                tr.position += movement;
                state.cam_pos = tr.position;
            }

            // Test objesini yukarı aşağı hareket ettir
            {
                let mut transforms = world.borrow_mut::<Transform>();
                let mut interactors = world.borrow_mut::<gizmo::renderer::components::FluidInteractor>();
                
                for (ent, interactor) in interactors.iter_mut() {
                    if let Some(trans) = transforms.get_mut(ent) {
                        let old_pos = trans.position;
                        // Sin dalgasıyla suyu dövsün
                        trans.position.y = 15.0 + (state.fps_timer * 2.0).sin() * 10.0;
                        
                        // Hız vektörünü hesapla ki fizik motoru çarpışmayı anlasın
                        let vel = (trans.position - old_pos) / dt;
                        interactor.velocity = vel;

                        // Gizmo ile çiz
                        if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
                            let r = interactor.radius;
                            let min = trans.position - Vec3::splat(r);
                            let max = trans.position + Vec3::splat(r);
                            gizmos.draw_box(min, max, [1.0, 0.2, 0.2, 1.0]);
                        }
                    }
                }
            }

            // Tank sınırlarını Gizmo ile çiz
            if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
                let min = Vec3::new(-25.0, 0.0, -0.26);
                let max = Vec3::new(25.0, 100.0, 0.26);
                gizmos.draw_box(min, max, [0.2, 0.6, 1.0, 0.5]);
            }
        })
        .set_ui(|_world, state, ctx| {
            gizmo::egui::Area::new("fluid_hud")
                .anchor(gizmo::egui::Align2::LEFT_TOP, [10.0, 10.0])
                .show(ctx, |ui| {
                    ui.label(
                        gizmo::egui::RichText::new(format!("FPS: {:.1}", state.fps))
                            .color(gizmo::egui::Color32::YELLOW)
                            .size(24.0)
                            .strong(),
                    );
                    ui.label(
                        gizmo::egui::RichText::new("💧 SPH Fluid — 25K Particles")
                            .color(gizmo::egui::Color32::from_rgb(100, 180, 255))
                            .size(16.0),
                    );
                    ui.label(
                        gizmo::egui::RichText::new("WASD: Hareket | Sağ Tık: Bakış | Shift: Hız")
                            .color(gizmo::egui::Color32::LIGHT_GRAY)
                            .size(13.0),
                    );
                });
        })
        .set_render(|world, _state, encoder, view, renderer, _light_time| {
            fluid_only_render_pass(world, encoder, view, renderer);
        })
        .run();
}

fn pitch_yaw_quat(pitch: f32, yaw: f32) -> Quat {
    let q_yaw = Quat::from_axis_angle(Vec3::Y, yaw);
    let q_pitch = Quat::from_axis_angle(Vec3::X, pitch);
    q_yaw * q_pitch
}

/// Sadece sıvı simülasyonunu çalıştıran özel render pass — fizik küpleri devre dışı.
fn fluid_only_render_pass(
    world: &mut gizmo::core::World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut gizmo::renderer::Renderer,
) {
    use gizmo::renderer::gpu_types::SceneUniforms;

    let aspect = if renderer.size.height > 0 {
        renderer.size.width as f32 / renderer.size.height as f32
    } else { 1.0 };

    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut cam_pos = Vec3::ZERO;
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);

    let cameras = world.borrow::<Camera>(); let transforms = world.borrow::<Transform>();
    {
        if let Some((active_cam, _)) = cameras.iter().next() {
            if let (Some(cam), Some(trans)) = (cameras.get(active_cam), transforms.get(active_cam)) {
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(trans.position);
                cam_pos = trans.position;
                cam_forward = trans.rotation * Vec3::new(0.0, 0.0, -1.0);
            }
        }
    }

    let view_proj = proj * view_mat;
    let id = Mat4::IDENTITY.to_cols_array_2d();
    let scene_uniform_data = SceneUniforms {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
        sun_direction: [0.3, -0.8, 0.2, 1.0],
        sun_color: [1.0, 0.95, 0.9, 1.0],
        lights: [gizmo::renderer::gpu_types::LightData {
            position: [0.0; 4], color: [0.0; 4],
            direction: [0.0, -1.0, 0.0, 0.0], params: [0.0; 4],
        }; 10],
        light_view_proj: [id; 4],
        cascade_splits: [10.0, 50.0, 200.0, 2000.0],
        camera_forward: [cam_forward.x, cam_forward.y, cam_forward.z, 0.0],
        cascade_params: [0.1, 1.0 / gizmo::renderer::SHADOW_MAP_RES as f32, 0.0, 0.0],
        num_lights: 0,
        _pre_align_pad: [0; 3],
        _align_pad: [0; 3],
        _post_align_pad: 0,
        _pad_scene: [0; 3],
        _end_pad: 0,
    };
    renderer.queue.write_buffer(
        &renderer.scene.global_uniform_buffer, 0,
        gizmo::bytemuck::cast_slice(&[scene_uniform_data]),
    );

    // Sadece sıvı compute pass
    if let Some(fluid) = &renderer.gpu_fluid {
        let mut colliders = Vec::new();
        {
            let transforms = world.borrow::<Transform>();
            let interactors = world.borrow::<gizmo::renderer::components::FluidInteractor>();
            
            for (ent, interactor) in interactors.iter() {
                if let Some(trans) = transforms.get(ent) {
                    colliders.push(gizmo::renderer::gpu_fluid::FluidCollider {
                        position: [trans.position.x, trans.position.y, trans.position.z],
                        radius: interactor.radius,
                        velocity: [interactor.velocity.x, interactor.velocity.y, interactor.velocity.z],
                        padding: 0.0,
                    });
                }
            }
        }
        
        fluid.update_parameters(&renderer.queue, [0.0; 3], [0.0; 3], false, &colliders);
        fluid.compute_pass(encoder, &renderer.queue);
    }

    // Render pass
    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Fluid Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.02, g: 0.03, b: 0.08, a: 1.0 }),
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

        // Sadece sıvı parçacıklarını çiz
        if let Some(fluid) = &renderer.gpu_fluid {
            fluid.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
        }

        // Gizmo debug hatları (tank sınırları)
        if let Some(gizmos) = world.get_resource::<gizmo::renderer::Gizmos>() {
            if let Some(debug_renderer) = &mut renderer.debug_renderer {
                debug_renderer.update(&renderer.queue, &gizmos);
                debug_renderer.render(&mut render_pass, &renderer.scene.global_bind_group, gizmos.depth_test);
            }
        }
    }

    // Gizmo temizle
    if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
        gizmos.clear();
    }

    renderer.run_post_processing(encoder, view);
}
