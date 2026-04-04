use std::sync::Arc;
use winit::{
    event::{Event, WindowEvent, KeyEvent, ElementState},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
    keyboard::{PhysicalKey, KeyCode},
};
use yelbegen::prelude::*;
use yelbegen::editor::EditorContext;
use yelbegen::renderer::{Vertex, EngineUniforms};

// 3 Boyutlu Küp Vertex Üreticisi (36 Köşe = 6 Yüzey * 2 Üçgen)
fn create_cube() -> Vec<Vertex> {
    let mut v = Vec::new();

    // Yardımcı Makro: Tek yüzeyin (2 üçgen = 6 nokta) oluşturulması
    let mut add_face = |p1: [f32;3], p2: [f32;3], p3: [f32;3], p4: [f32;3], norm: [f32;3], col: [f32;3]| {
        v.push(Vertex { position: p1, normal: norm, tex_coords: [0.0, 0.0], color: col });
        v.push(Vertex { position: p2, normal: norm, tex_coords: [1.0, 0.0], color: col });
        v.push(Vertex { position: p3, normal: norm, tex_coords: [1.0, 1.0], color: col });

        v.push(Vertex { position: p1, normal: norm, tex_coords: [0.0, 0.0], color: col });
        v.push(Vertex { position: p3, normal: norm, tex_coords: [1.0, 1.0], color: col });
        v.push(Vertex { position: p4, normal: norm, tex_coords: [0.0, 1.0], color: col });
    };

    let c = [1.0, 1.0, 1.0]; // Objenin Orijinal Rengi Beyaz (Kaplama ışıkla boyanır)

    // Ön
    add_face([-1.0,-1.0, 1.0], [ 1.0,-1.0, 1.0], [ 1.0, 1.0, 1.0], [-1.0, 1.0, 1.0], [ 0.0, 0.0, 1.0], c);
    // Arka
    add_face([ 1.0,-1.0,-1.0], [-1.0,-1.0,-1.0], [-1.0, 1.0,-1.0], [ 1.0, 1.0,-1.0], [ 0.0, 0.0,-1.0], c);
    // Üst
    add_face([-1.0, 1.0, 1.0], [ 1.0, 1.0, 1.0], [ 1.0, 1.0,-1.0], [-1.0, 1.0,-1.0], [ 0.0, 1.0, 0.0], c);
    // Alt
    add_face([-1.0,-1.0,-1.0], [ 1.0,-1.0,-1.0], [ 1.0,-1.0, 1.0], [-1.0,-1.0, 1.0], [ 0.0,-1.0, 0.0], c);
    // Sağ
    add_face([ 1.0,-1.0, 1.0], [ 1.0,-1.0,-1.0], [ 1.0, 1.0,-1.0], [ 1.0, 1.0, 1.0], [ 1.0, 0.0, 0.0], c);
    // Sol
    add_face([-1.0,-1.0,-1.0], [-1.0,-1.0, 1.0], [-1.0, 1.0, 1.0], [-1.0, 1.0,-1.0], [-1.0, 0.0, 0.0], c);

    v
}

// Dışarıdan (.obj) dosya okuyan Asset Parser
fn load_model(path: &str) -> Vec<Vertex> {
    let (models, _) = tobj::load_obj(path, &tobj::GPU_LOAD_OPTIONS).expect("Model dosyasi okunamadi!");
    let mut vertices = Vec::new();

    for m in models.iter() {
        let mesh = &m.mesh;
        for i in 0..mesh.indices.len() {
            let idx = mesh.indices[i] as usize;
            let pos = [mesh.positions[3 * idx], mesh.positions[3 * idx + 1], mesh.positions[3 * idx + 2]];
            
            let norm = if !mesh.normals.is_empty() {
                [mesh.normals[3 * idx], mesh.normals[3 * idx + 1], mesh.normals[3 * idx + 2]]
            } else {
                [0.0, 1.0, 0.0]
            };
            
            let tex = if !mesh.texcoords.is_empty() {
                [mesh.texcoords[2 * idx], mesh.texcoords[2 * idx + 1]]
            } else {
                [0.0, 0.0]
            };
            
            vertices.push(Vertex { position: pos, normal: norm, tex_coords: tex, color: [1.0, 1.0, 1.0] });
        }
    }
    vertices
}

// Gerçek .PNG veya .JPG Dosyası Okuyan Image API Modülü
fn load_image_texture(path: &str) -> (Vec<u8>, u32, u32) {
    let img = image::open(path).expect("Doku (Texture) dosyasi bulunamadi!").to_rgba8();
    let width = img.width();
    let height = img.height();
    let data = img.into_raw();
    (data, width, height)
}

// Kod ile üretilen (Procedural) Satranç Damalı 64x64 Texture
fn create_checkerboard() -> Vec<u8> {
    let mut pixels = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            let is_white = ((x / 8) + (y / 8)) % 2 == 0;
            let color = if is_white { 255 } else { 30 }; // Siyahlar tam karanlık olmasın diye 30
            pixels.extend_from_slice(&[color, color, color, 255]); 
        }
    }
    pixels
}

fn main() {
    println!("Yelbegen Engine: Faz 4 (Işıklandırma ve Texture) Olay Ufku!");
    
    let mut world = World::new();

    // -- VARLIK 1: Test Küpü (Kameranın Tam Önünde) --
    let bouncing_box = world.spawn();
    world.add_component(bouncing_box, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(bouncing_box, Velocity::new(Vec3::ZERO)); 
    world.add_component(bouncing_box, Collider::new_aabb(1.0, 1.0, 1.0));
    world.add_component(bouncing_box, RigidBody::new(1.0, 0.8, 0.2, false)); 

    // -- VARLIK 2: Zemin (Durağan Devasa Kutu) --
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -3.0, 0.0)));
    world.add_component(ground, Velocity::new(Vec3::ZERO));
    world.add_component(ground, Collider::new_aabb(20.0, 1.0, 20.0)); 
    world.add_component(ground, RigidBody::new_static());

    let mut schedule = Schedule::new();
    schedule.add_system(physics_movement_system); 
    schedule.add_system(physics_collision_system);

    let event_loop = EventLoop::new().expect("Event Loop başlatılamadı");
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Yelbegen Faz 4 - Gerçekçi Render (Işık/Doku)")
            .with_inner_size(winit::dpi::LogicalSize::new(1280, 720))
            .build(&event_loop)
            .expect("Pencere oluşturulamadı!"),
    );

    let mut renderer = pollster::block_on(Renderer::new(window.clone()));
    
    // YELBEGEN GPU EKLENTİLERİ BAĞLANIYOR
    let mut editor = EditorContext::new(&renderer.device, renderer.config.format, &window);
    
    // Model testleri için hem Maymunu hem de referans Küpü yüklüyoruz.
    let loaded_vertices = load_model("assets/suzanne.obj");
    println!("Yelbegen: {} köşe başarıyla OBJ modelinden parse edildi!", loaded_vertices.len());
    let mesh_length = loaded_vertices.len() as u32;
    let main_mesh = renderer.create_mesh(&loaded_vertices);
    let cube_mesh = renderer.create_mesh(&create_cube());
    
    // Satranç dokusu yerine Gerçek "Tuğla" resmini Yüklüyoruz!
    let (brick_data, b_width, b_height) = load_image_texture("assets/brick.jpg");
    let brick_bind_group = renderer.create_texture(&brick_data, b_width, b_height);

    event_loop.set_control_flow(ControlFlow::Poll);

    let mut last_update = std::time::Instant::now();
    let mut camera_pos = Vec3::new(0.0, 5.0, 15.0); 
    let mut camera_yaw = -std::f32::consts::FRAC_PI_2;
    let mut camera_pitch = -0.3f32;
    let mut mouse_pressed = false;
    let mut light_time = 0.0f32; // Güneş simülasyonu için

    let _ = event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent { ref event, window_id } if window_id == window.id() => {
                
                let consumes_input = editor.handle_event(&window, event);

                match event {
                    WindowEvent::CloseRequested => elwt.exit(),
                    WindowEvent::Resized(physical_size) => renderer.resize(*physical_size),
                    WindowEvent::MouseInput { state, button: winit::event::MouseButton::Right, .. } => {
                        mouse_pressed = *state == ElementState::Pressed;
                    }
                    WindowEvent::KeyboardInput {
                        event: KeyEvent { physical_key: PhysicalKey::Code(key_code), state: ElementState::Pressed, .. }, ..
                    } => {
                        if !consumes_input {
                            let fx = camera_yaw.cos() * camera_pitch.cos();
                            let fy = camera_pitch.sin();
                            let fz = camera_yaw.sin() * camera_pitch.cos();
                            let camera_front = Vec3::new(fx, fy, fz).normalize();
                            let right = camera_front.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
                            let speed = 0.5;

                            match key_code {
                                KeyCode::KeyW => camera_pos = camera_pos + camera_front * speed,
                                KeyCode::KeyS => camera_pos = camera_pos - camera_front * speed,
                                KeyCode::KeyA => camera_pos = camera_pos - right * speed,
                                KeyCode::KeyD => camera_pos = camera_pos + right * speed,
                                KeyCode::KeyQ => camera_pos.y -= speed,
                                KeyCode::KeyE => camera_pos.y += speed,
                                _ => {}
                            }
                        }
                    }
                    WindowEvent::RedrawRequested => {
                        // 1. ZAMAN BAZLI GÜNEŞ (Işık X düzleminde gidip gelecek ki gölgeyi hissedelim)
                        light_time += 0.05;
                        let g_light_x = light_time.sin() * 2.0;
                        let mut light_vec = [-g_light_x, 1.0, 1.0, 0.0];

                        let Ok(output) = renderer.surface.get_current_texture() else { return };
                        let view_texture = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

                        let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Dunya Cizeri") });

                        // -- 3D RENDER KOMUTU BAŞLAT --
                        {
                            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("Oyun Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &view_texture,
                                    resolve_target: None,
                                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.15, b: 0.20, a: 1.0 }), store: wgpu::StoreOp::Store },
                                })],
                                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                                    view: &renderer.depth_texture_view,
                                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                                    stencil_ops: None,
                                }),
                                timestamp_writes: None, occlusion_query_set: None,
                            });

                            render_pass.set_pipeline(&renderer.render_pipeline);
                            
                            // -- ECS'TEKİ HER OBJEYİ ÇİZ (Sadece Transform'u olanları)
                            let fx = camera_yaw.cos() * camera_pitch.cos();
                            let fy = camera_pitch.sin();
                            let fz = camera_yaw.sin() * camera_pitch.cos();
                            let camera_front = Vec3::new(fx, fy, fz).normalize();

                            let aspect = if renderer.size.height > 0 { renderer.size.width as f32 / renderer.size.height as f32 } else { 1.0 };
                            let proj = Mat4::perspective(std::f32::consts::FRAC_PI_4, aspect, 0.1, 100.0);
                            let view = Mat4::look_at_rh(camera_pos, camera_pos + camera_front, Vec3::new(0.0, 1.0, 0.0));

                                // Ekrandaki spesifik objeleri çizeceğiz
                                let render_entities = [bouncing_box, ground];
                                
                                if let Some(positions) = world.borrow::<Transform>() {
                                    if let Some(colliders) = world.borrow::<Collider>() {
                                        for entity in render_entities.iter() {
                                            if let Some(trans) = positions.get(entity.id()) {
                                                
                                                let mut scale = Vec3::new(1.0, 1.0, 1.0);
                                                if let Some(col) = colliders.get(entity.id()) {
                                                    if let yelbegen::physics::ColliderShape::Aabb(bounding_box) = col.shape {
                                                        scale = bounding_box.half_extents;
                                                    }
                                                }

                                                let mut model = Mat4::translation(trans.position);
                                                model.cols[0].x = scale.x; 
                                                model.cols[1].y = scale.y; 
                                                model.cols[2].z = scale.z;

                                                let mvp = proj * view * model;

                                                let uniform_data = EngineUniforms {
                                                    mvp: mvp.to_cols_array_2d(),
                                                    light_dir: light_vec,
                                                };

                                                renderer.queue.write_buffer(&renderer.uniform_buffer, 0, bytemuck::cast_slice(&[uniform_data]));
                                                
                                                render_pass.set_bind_group(0, &renderer.uniform_bind_group, &[]);
                                                render_pass.set_bind_group(1, &brick_bind_group, &[]); 

                                                // Zemin objesiyse referans Küpü, Maymun objesiyse maymunu çiz!
                                                if entity.id() == ground.id() {
                                                    render_pass.set_vertex_buffer(0, cube_mesh.slice(..));
                                                    render_pass.draw(0..36, 0..1);
                                                } else {
                                                    render_pass.set_vertex_buffer(0, main_mesh.slice(..));
                                                    render_pass.draw(0..mesh_length, 0..1);
                                                }
                                            }
                                        }
                                    }
                                }
                        } // Oyun Pass bitti

                        editor.begin_frame(&window);
                        
                        egui::Window::new("⚙️ Yelbegen Engine Inspector").show(&editor.context, |ui| {
                            ui.heading("Aydınlatma ve Simülasyon");
                            ui.separator();
                            ui.label("Güneş gökyüzünde dinamik olarak dönmektedir, objelerin yüzeyindeki karanlık (Shadow) etkisini gözlemleyin.");
                            
                            ui.separator();
                            if let Some(mut rbs) = world.borrow_mut::<RigidBody>() {
                                if let Some(rb) = rbs.get_mut(bouncing_box.id()) {
                                    ui.horizontal(|ui| {
                                        ui.label("Zıplayan Kutu Kütlesi: ");
                                        ui.add(egui::Slider::new(&mut rb.mass, 0.0..=10.0));
                                    });
                                }
                            }

                            if ui.button("🔄 Başa Sar (Reset)").clicked() {
                                if let Some(mut trans) = world.borrow_mut::<Transform>() {
                                    if let Some(t) = trans.get_mut(bouncing_box.id()) { t.position = Vec3::new(0.0, 5.0, -8.0); }
                                }
                                if let Some(mut vels) = world.borrow_mut::<Velocity>() {
                                    if let Some(v) = vels.get_mut(bouncing_box.id()) { v.linear = Vec3::new(3.0, 0.0, 0.0); }
                                }
                            }
                        });

                        editor.render(&window, &renderer.device, &renderer.queue, &mut encoder, &view_texture);

                        renderer.queue.submit(std::iter::once(encoder.finish()));
                        output.present();
                    }
                    _ => {}
                }
            }
            Event::DeviceEvent { event: winit::event::DeviceEvent::MouseMotion { delta }, .. } => {
                if mouse_pressed {
                    let sensitivity = 0.005;
                    camera_yaw += delta.0 as f32 * sensitivity;
                    camera_pitch -= delta.1 as f32 * sensitivity;
                    
                    if camera_pitch > 1.5 { camera_pitch = 1.5; }
                    if camera_pitch < -1.5 { camera_pitch = -1.5; }
                }
            }
            Event::AboutToWait => {
                let now = std::time::Instant::now();
                if now.duration_since(last_update).as_secs_f32() >= 0.016 {
                    schedule.run(&world);
                    last_update = now;
                    window.request_redraw();
                }
            }
            _ => {}
        }
    });
}
