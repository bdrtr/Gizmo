use gizmo::prelude::*;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity};
use gizmo::physics::world::PhysicsWorld;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, Material, MeshRenderer, PointLight};
use std::f32::consts::PI;

struct DemoState {
    camera_speed: f32,
    camera_pitch: f32,
    camera_yaw: f32,
    camera_pos: Vec3,
    time: f32,
    cube_mesh: gizmo::renderer::components::Mesh,
    cube_mat: gizmo::renderer::components::Material,
    sphere_mesh: gizmo::renderer::components::Mesh,
    metal_mat: gizmo::renderer::components::Material,
    sun_entity: gizmo::core::Entity,
}

// Kırılabilir objeleri etiketleyen bileşen
#[derive(Clone)]
pub struct Destructible;
impl gizmo::prelude::Component for Destructible {}


// Parçalanacak objeleri tutan kuyruk (Resource)
pub struct FractureQueue {
    pub entities: Vec<u32>,
}


fn setup(world: &mut World, renderer: &Renderer) -> DemoState {
    let mut asset_manager = AssetManager::new();

    // Işık (Güneş benzeri bir tepe ışığı)
    let light = world.spawn();
    world.add_component(light, Transform::new(Vec3::new(0.0, 50.0, 0.0)));
    world.add_component(light, PointLight::new(Vec3::new(1.0, 0.9, 0.8), 2000.0, 100.0));

    // Kamera
    let camera_ent = world.spawn();
    world.add_component(
        camera_ent,
        Transform::new(Vec3::new(8.0, 6.0, 12.0)).with_rotation(Quat::from_rotation_x(-0.3)),
    );
    world.add_component(
        camera_ent,
        Camera::new(
            std::f32::consts::FRAC_PI_3,
            0.1,
            5000.0,
            -PI / 4.0,
            -0.3,
            true,
        ),
    );
    world.add_component(camera_ent, gizmo::core::EntityName("Main Camera".into()));

    // --- CPU PHYSICS SETUP ---
    let mut phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    
    // Gökyüzü (Skybox)
    let skybox_mesh = AssetManager::create_inverted_cube(&renderer.device);
    let sky_tex = asset_manager.load_material_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, "assets/sky.jpg").unwrap();
    let sky_mat = Material::new(sky_tex).with_skybox();
    
    let sky_ent = world.spawn();
    world.add_component(sky_ent, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2000.0)));
    world.add_component(sky_ent, skybox_mesh);
    world.add_component(sky_ent, sky_mat);
    world.add_component(sky_ent, MeshRenderer::new());

    // Zemin (Çimen)
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, 0.0, 0.0)).with_scale(Vec3::new(1000.0, 1.0, 1000.0)));
    world.add_component(ground, AssetManager::create_cube(&renderer.device));
    let grass_tex = asset_manager.load_material_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, "assets/grass.jpg").unwrap_or_else(|_| asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout));
    world.add_component(ground, Material::new(grass_tex).with_pbr(Vec4::new(0.5, 0.8, 0.4, 1.0), 0.9, 0.1));
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, Collider::box_collider(Vec3::new(1000.0, 0.5, 1000.0)));
    world.add_component(ground, RigidBody {
        body_type: gizmo::physics::components::BodyType::Static,
        mass: 0.0,
        restitution: 0.3,
        friction: 0.9,
        ..Default::default()
    });



    // Gökyüzünden düşen kutular (Etkileşimi görmek için)
    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let box_tex = asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    for i in 0..20 {
        let box_ent = world.spawn();
        let pos = Vec3::new((i as f32 % 4.0) * 2.0 - 3.0, 10.0 + (i as f32) * 2.0, ((i / 4) as f32) * 2.0 - 3.0);
        world.add_component(box_ent, Transform::new(pos));
        world.add_component(box_ent, cube_mesh.clone());
        world.add_component(box_ent, Material::new(box_tex.clone()).with_pbr(Vec4::splat(1.0), 0.5, 0.0));
        world.add_component(box_ent, MeshRenderer::new());
        world.add_component(box_ent, Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)));
        world.add_component(box_ent, RigidBody::new(500.0, 0.3, 0.8, true)); // Ahşap yoğunluğu
        world.add_component(box_ent, Velocity::default());
    }

    // Güneş Işığı (Gündüz/Gece Döngüsü için)
    let sun_entity = world.spawn();
    world.add_component(sun_entity, Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)));
    world.add_component(sun_entity, gizmo::renderer::components::DirectionalLight::new(
        Vec3::new(1.0, 0.9, 0.8), // Sıcak güneş rengi
        4.0, 
        gizmo::renderer::components::LightRole::Sun
    ));

    let metal_tex = asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    let metal_mat = Material::new(metal_tex.clone()).with_pbr(Vec4::new(0.3, 0.3, 0.3, 1.0), 0.3, 0.9);
    let cube_mat = Material::new(box_tex.clone()).with_pbr(Vec4::splat(1.0), 0.5, 0.0);

    // --- FİZİKSEL ASMA KÖPRÜ (SUSPENSION BRIDGE) ---
    // 1. Köprü direkleri (Statik)
    let p1 = world.spawn();
    world.add_component(p1, Transform::new(Vec3::new(-12.0, 5.0, -10.0)).with_scale(Vec3::new(1.0, 10.0, 1.0)));
    world.add_component(p1, cube_mesh.clone());
    world.add_component(p1, metal_mat.clone());
    world.add_component(p1, MeshRenderer::new());
    world.add_component(p1, Collider::box_collider(Vec3::new(1.0, 10.0, 1.0)));
    world.add_component(p1, RigidBody::new(0.0, 0.1, 0.5, false));

    let p2 = world.spawn();
    world.add_component(p2, Transform::new(Vec3::new(12.0, 5.0, -10.0)).with_scale(Vec3::new(1.0, 10.0, 1.0)));
    world.add_component(p2, cube_mesh.clone());
    world.add_component(p2, metal_mat.clone());
    world.add_component(p2, MeshRenderer::new());
    world.add_component(p2, Collider::box_collider(Vec3::new(1.0, 10.0, 1.0)));
    world.add_component(p2, RigidBody::new(0.0, 0.1, 0.5, false));

    // 2. Köprü Tahtaları (Dinamik - Joints)
    let num_planks = 14;
    let mut planks = Vec::new();
    let plank_width = 0.8;
    let plank_height = 0.1;
    let plank_depth = 1.5;
    
    for i in 0..num_planks {
        let plank = world.spawn();
        let t = (i as f32 + 1.0) / (num_planks as f32 + 1.0);
        let x_pos = -12.0 + (24.0 * t);
        
        world.add_component(plank, Transform::new(Vec3::new(x_pos, 9.0, -10.0)).with_scale(Vec3::new(plank_width, plank_height, plank_depth)));
        world.add_component(plank, cube_mesh.clone());
        world.add_component(plank, cube_mat.clone());
        world.add_component(plank, MeshRenderer::new());
        world.add_component(plank, Collider::box_collider(Vec3::new(plank_width, plank_height, plank_depth)));
        world.add_component(plank, RigidBody::new(10.0, 0.1, 0.8, true));
        world.add_component(plank, Velocity::default());
        world.add_component(plank, Destructible);
        
        planks.push(plank);
    }

    // 3. Mafsalları (Hinge Joints) Bağla
    for i in 0..num_planks+1 {
        let ent_a = if i == 0 { p1 } else { planks[i-1] };
        let ent_b = if i == num_planks { p2 } else { planks[i] };
        
        let anchor_a = if i == 0 { 
            Vec3::new(1.0, 4.0, 0.0) 
        } else { 
            Vec3::new(plank_width, 0.0, 0.0) 
        };
        
        let anchor_b = if i == num_planks { 
            Vec3::new(-1.0, 4.0, 0.0) 
        } else { 
            Vec3::new(-plank_width, 0.0, 0.0) 
        };
        
        let joint = gizmo::physics::joints::Joint::hinge(
            ent_a, ent_b,
            anchor_a, anchor_b,
            Vec3::new(0.0, 0.0, 1.0) // Z ekseninde salınım
        );
        phys_world.joints.push(joint);
    }

    world.insert_resource(phys_world);
    world.insert_resource(FractureQueue { entities: Vec::new() });
    world.insert_resource(asset_manager);

    DemoState {
        camera_speed: 25.0,
        camera_pitch: -0.3,
        camera_yaw: -PI / 4.0,
        camera_pos: Vec3::new(8.0, 6.0, 12.0), // Suya biraz yukarıdan ve çaprazdan bakalım
        time: 0.0,
        cube_mesh,
        cube_mat: Material::new(box_tex).with_pbr(Vec4::splat(1.0), 0.5, 0.0),
        sphere_mesh: AssetManager::create_sphere(&renderer.device, 0.5, 16, 16),
        metal_mat: Material::new(metal_tex)
            .with_pbr(Vec4::new(0.3, 0.3, 0.3, 1.0), 0.1, 1.0), // Parlak krom metal
        sun_entity,
    }
}

fn update(world: &mut World, state: &mut DemoState, dt: f32, input: &gizmo::core::input::Input) {
    state.time += dt;

    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);
    
    if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Transform>, gizmo::core::query::Mut<Camera>)>() {
        for (_, (mut transform, mut camera)) in q.iter_mut() {
            let sensitivity = 0.002;
            let (dx, dy) = input.mouse_delta();
            
            // Sadece sağ tık basılıyken kamerayı döndür
            if input.is_mouse_button_pressed(gizmo::core::input::mouse::RIGHT) {
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

            // Quaternion oluştur
            let yaw_rot = Quat::from_rotation_y(-state.camera_yaw + std::f32::consts::FRAC_PI_2);
            let pitch_rot = Quat::from_rotation_x(state.camera_pitch);
            transform.rotation = yaw_rot * pitch_rot;

            // DİKKAT: Gizmo renderer view matrix'ini Camera objesinden alıyor. Bu yüzden Camera yaw ve pitch değerlerini güncellemeliyiz!
            camera.yaw = state.camera_yaw;
            camera.pitch = state.camera_pitch;

            let speed = state.camera_speed * dt * if input.is_key_pressed(KeyCode::ShiftLeft as u32) { 3.0 } else { 1.0 };

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
            if input.is_key_pressed(KeyCode::KeyE as u32) {
                state.camera_pos += up * speed;
            }
            if input.is_key_pressed(KeyCode::KeyQ as u32) {
                state.camera_pos -= up * speed;
            }

            transform.set_position(state.camera_pos);
            cam_forward = forward;
        }
    }

    if let Some(mut q) = world.query::<gizmo::core::query::Mut<Transform>>() {
        // Güneş rotasyonu (Gündüz/Gece döngüsü)
        if let Some((_, mut trans)) = q.iter_mut().find(|(e, _)| *e == state.sun_entity.id()) {
            let sun_speed = 0.2; // Zamanın hızı
            let sun_angle = -std::f32::consts::PI / 4.0 + state.time * sun_speed;
            trans.rotation = Quat::from_rotation_x(sun_angle);
        }
    }

    // Fırlatma Mekaniği (Sol tık ile Ahşap Kutu, Sağ tık ile Metal Gülle)
    if input.is_mouse_button_just_pressed(gizmo::core::input::mouse::LEFT) {
        let spawn_pos = state.camera_pos + cam_forward * 2.0;
        let throw_velocity = cam_forward * 25.0;

        let box_ent = world.spawn();
        world.add_component(box_ent, Transform::new(spawn_pos));
        world.add_component(box_ent, state.cube_mesh.clone());
        world.add_component(box_ent, state.cube_mat.clone());
        world.add_component(box_ent, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(box_ent, gizmo::physics::components::Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)));
        world.add_component(box_ent, gizmo::physics::components::RigidBody::new(500.0, 0.3, 0.8, true)); // Ahşap (Yoğunluğu sudan az, yüzer)
        world.add_component(box_ent, gizmo::physics::components::Velocity::new(throw_velocity));
    }

    if input.is_mouse_button_just_pressed(gizmo::core::input::mouse::RIGHT) {
        let spawn_pos = state.camera_pos + cam_forward * 2.0;
        let throw_velocity = cam_forward * 35.0; // Metal daha hızlı atılsın

        let sphere_ent = world.spawn();
        world.add_component(sphere_ent, Transform::new(spawn_pos).with_scale(Vec3::splat(0.5)));
        world.add_component(sphere_ent, state.sphere_mesh.clone());
        world.add_component(sphere_ent, state.metal_mat.clone());
        world.add_component(sphere_ent, gizmo::renderer::components::MeshRenderer::new());
        world.add_component(sphere_ent, gizmo::physics::components::Collider::sphere(0.5));
        world.add_component(sphere_ent, gizmo::physics::components::RigidBody::new(5000.0, 0.1, 0.2, true)); // Metal (Çok ağır, hemen batar)
        world.add_component(sphere_ent, gizmo::physics::components::Velocity::new(throw_velocity));
    }

    // Şok Dalgası (Force Push) - E Tuşu
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) {
        let epicenter = state.camera_pos + cam_forward * 5.0; // Patlama noktası kameranın biraz önünde
        let explosion_radius = 25.0;
        let explosion_force = 20000.0; // Güçlü bir şok dalgası

        if let Some(mut rbs) = world.query::<(
            gizmo::core::query::Mut<Transform>, 
            gizmo::core::query::Mut<gizmo::physics::components::RigidBody>, 
            gizmo::core::query::Mut<gizmo::physics::components::Velocity>
        )>() {
            for (_, (trans, rb, mut vel)) in rbs.iter_mut() {
                if rb.body_type != gizmo::physics::components::BodyType::Static {
                    let dir = trans.position - epicenter;
                    let dist = dir.length();
                    if dist < explosion_radius && dist > 0.1 {
                        let force_multiplier = 1.0 - (dist / explosion_radius);
                        let force = force_multiplier * explosion_force;
                        let push = dir.normalize() * force;
                        
                        // İvme (F = m*a => a = F/m)
                        vel.linear += push / rb.mass;
                        // objelerin dönerek uçması için tork etkisi
                        vel.angular += Vec3::new(push.y, push.z, push.x) * 5.0 / rb.mass; 
                    }
                }
            }
        }
    }

    // X Tuşu: Cam Gibi Parçalama (Fracture)
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyX as u32) {
        let mut queue = world.get_resource_mut::<FractureQueue>().unwrap();
        if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Transform>, gizmo::core::query::Mut<Destructible>)>() {
            for (ent_id, _) in q.iter_mut() {
                queue.entities.push(ent_id);
            }
        }
    }

    // CPU Physics Adımı (Gizmo ECS entegrasyonu)
    gizmo::systems::cpu_physics_step_system(world, dt);
}

fn render(
    world: &mut World,
    _state: &DemoState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    // Kırılacak objeleri (Fracture Queue) işle
    let mut to_fracture = Vec::new();
    {
        let mut queue = world.get_resource_mut::<FractureQueue>().unwrap();
        to_fracture.append(&mut queue.entities);
    }
    
    if !to_fracture.is_empty() {
        let mut fracture_data = Vec::new();
        
        if let Some(mut q) = world.query::<(
            gizmo::core::query::Mut<Transform>, 
            gizmo::core::query::Mut<gizmo::renderer::components::Material>, 
            gizmo::core::query::Mut<Velocity>
        )>() {
            for (ent_id, (trans, mat, vel)) in q.iter_mut() {
                if to_fracture.contains(&ent_id) {
                    fracture_data.push((ent_id, *trans, mat.clone(), *vel));
                }
            }
        }
        
        for (ent_id, trans, mat, vel) in fracture_data {
            world.despawn_by_id(ent_id); // Orijinal objeyi yok et
            
            // Voronoi Shatter ile objeyi rastgele parçalara böl
            let chunks = gizmo::physics::fracture::voronoi_shatter(Vec3::splat(0.5), 5, 42 + ent_id as u64);
            
            for chunk in chunks {
                // Parçalar için GPU Vertex'lerini unroll (indices üzerinden)
                let mut gpu_vertices = Vec::new();
                for idx in &chunk.indices {
                    let pos = chunk.vertices[*idx as usize];
                    let normal = chunk.normals[*idx as usize];
                    gpu_vertices.push(gizmo::renderer::gpu_types::Vertex {
                        position: [pos.x, pos.y, pos.z],
                        color: [1.0, 1.0, 1.0],
                        normal: [normal.x, normal.y, normal.z],
                        tex_coords: [0.0, 0.0],
                        joint_indices: [0; 4],
                        joint_weights: [0.0; 4],
                    });
                }
                
                // Dinamik wgpu Buffer oluştur
                use gizmo::wgpu::util::DeviceExt;
                let vbuf = renderer.device.create_buffer_init(&gizmo::wgpu::util::BufferInitDescriptor {
                    label: Some("Chunk VBuf"),
                    contents: gizmo::bytemuck::cast_slice(&gpu_vertices),
                    usage: gizmo::wgpu::BufferUsages::VERTEX,
                });
                
                let mesh = gizmo::renderer::components::Mesh::new(
                    std::sync::Arc::new(vbuf),
                    &gpu_vertices,
                    Vec3::ZERO,
                    format!("chunk_{}", ent_id),
                );
                
                // Hacme göre yaklaşık bir çarpışma küresi hesapla
                let approx_radius = (chunk.volume / 4.188).cbrt().max(0.15);
                
                let chunk_ent = world.spawn();
                let offset = chunk.center_of_mass;
                let new_pos = trans.position + trans.rotation.mul_vec3(offset);
                
                world.add_component(chunk_ent, Transform::new(new_pos).with_rotation(trans.rotation));
                world.add_component(chunk_ent, mesh);
                world.add_component(chunk_ent, mat.clone());
                world.add_component(chunk_ent, gizmo::renderer::components::MeshRenderer::new());
                world.add_component(chunk_ent, Collider::sphere(approx_radius));
                world.add_component(chunk_ent, RigidBody::new(5.0, 0.2, 0.6, true));
                
                // Parçaların merkeze göre dışa doğru patlaması için ufak şok dalgası ekle
                let mut explosion_vel = vel;
                explosion_vel.linear += offset.normalize() * 5.0;
                explosion_vel.angular += Vec3::new(offset.y, offset.z, offset.x) * 10.0;
                world.add_component(chunk_ent, explosion_vel);
            }
        }
    }

    // Sıvı ve Fizik Çarpışmalarını GPU'ya eşitleden çıkar
    renderer.gpu_physics = None;

    // CPU objelerini render et
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<DemoState>::new("Gizmo Engine - Ocean Scene", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run();
}
