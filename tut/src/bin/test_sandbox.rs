use gizmo::prelude::*;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity};
use gizmo::physics::world::PhysicsWorld;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, Material, MeshRenderer};
use std::f32::consts::PI;
use gizmo::wgpu::util::DeviceExt;

#[derive(Clone)]
pub struct Destructible;
impl gizmo::prelude::Component for Destructible {}

pub struct FractureQueue {
    pub entities: Vec<u32>,
}

struct TestSandboxState {
    camera_speed: f32,
    camera_pitch: f32,
    camera_yaw: f32,
    camera_pos: Vec3,
    _sun_entity: gizmo::core::Entity,
    
    sphere_mesh: gizmo::renderer::components::Mesh,
    sphere_mat: gizmo::renderer::components::Material,
}

pub struct ObjectBuilder<'a> {
    world: &'a mut World,
    transform: Transform,
    mesh: Option<gizmo::renderer::components::Mesh>,
    material: Option<gizmo::renderer::components::Material>,
    collider: Option<Collider>,
    rigid_body: Option<RigidBody>,
    velocity: Option<Velocity>,
    destructible: bool,
}

impl<'a> ObjectBuilder<'a> {
    pub fn new(world: &'a mut World) -> Self {
        Self {
            world,
            transform: Transform::new(Vec3::ZERO),
            mesh: None,
            material: None,
            collider: None,
            rigid_body: None,
            velocity: None,
            destructible: false,
        }
    }

    pub fn position(mut self, pos: Vec3) -> Self { self.transform.position = pos; self }
    pub fn scale(mut self, scale: Vec3) -> Self { self.transform.scale = scale; self }
    pub fn rotation(mut self, rot: Quat) -> Self { self.transform.rotation = rot; self }
    pub fn mesh(mut self, m: gizmo::renderer::components::Mesh) -> Self { self.mesh = Some(m); self }
    pub fn material(mut self, m: gizmo::renderer::components::Material) -> Self { self.material = Some(m); self }
    pub fn collider(mut self, c: Collider) -> Self { self.collider = Some(c); self }
    pub fn static_body(mut self) -> Self { self.rigid_body = Some(RigidBody::new_static()); self }
    pub fn static_body_with_friction(mut self, friction: f32) -> Self {
        let mut rb = RigidBody::new_static();
        rb.friction = friction;
        rb.restitution = 0.0;
        self.rigid_body = Some(rb);
        self
    }
    pub fn dynamic_body(mut self, mass: f32) -> Self { self.rigid_body = Some(RigidBody::new(mass, 0.1, 0.5, true)); self }
    pub fn dynamic_body_with_props(mut self, mass: f32, friction: f32, restitution: f32) -> Self { 
        self.rigid_body = Some(RigidBody::new(mass, restitution, friction, true)); 
        self 
    }
    pub fn velocity(mut self, v: Vec3) -> Self { self.velocity = Some(Velocity { linear: v, angular: Vec3::ZERO }); self }
    pub fn destructible(mut self) -> Self { self.destructible = true; self }

    pub fn spawn(self) -> gizmo::core::Entity {
        let ent = self.world.spawn();
        self.world.add_component(ent, self.transform);
        if let Some(m) = self.mesh.clone() { self.world.add_component(ent, m); }
        if let Some(m) = self.material.clone() { self.world.add_component(ent, m); }
        if self.mesh.is_some() || self.material.is_some() { self.world.add_component(ent, MeshRenderer::new()); }
        if let Some(c) = self.collider.clone() { self.world.add_component(ent, c); }
        if let Some(r) = self.rigid_body { 
            let mut r_final = r;
            if let Some(c) = &self.collider {
                r_final.update_inertia_from_collider(c);
            }
            self.world.add_component(ent, r_final); 
            if self.velocity.is_none() {
                self.world.add_component(ent, Velocity::default());
            }
        }
        if let Some(v) = self.velocity { self.world.add_component(ent, v); }
        if self.destructible { self.world.add_component(ent, Destructible); }
        ent
    }
}

fn setup(world: &mut World, renderer: &Renderer) -> TestSandboxState {
    let mut asset_manager = AssetManager::new();
    let mut phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    
    // Gökyüzü (Skybox)
    let skybox_mesh = AssetManager::create_inverted_cube(&renderer.device);
    let sky_tex = asset_manager.load_material_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, "tut/assets/sky.jpg").unwrap();
    let sky_mat = Material::new(sky_tex).with_skybox();
    
    let sky_ent = world.spawn();
    world.add_component(sky_ent, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(2000.0)));
    world.add_component(sky_ent, skybox_mesh);
    world.add_component(sky_ent, sky_mat);
    world.add_component(sky_ent, MeshRenderer::new());

    // Zemin (Çimen)
    let mut ground_vertices = Vec::new();
    let r = 500.0;
    let uvs = 300.0;
    let v0 = gizmo::renderer::gpu_types::Vertex { position: [-r, 0.5, r], tex_coords: [0.0, uvs], ..Default::default() };
    let v1 = gizmo::renderer::gpu_types::Vertex { position: [r, 0.5, r], tex_coords: [uvs, uvs], ..Default::default() };
    let v2 = gizmo::renderer::gpu_types::Vertex { position: [r, 0.5, -r], tex_coords: [uvs, 0.0], ..Default::default() };
    let v3 = gizmo::renderer::gpu_types::Vertex { position: [-r, 0.5, -r], tex_coords: [0.0, 0.0], ..Default::default() };

    ground_vertices.push(v0);
    ground_vertices.push(v1);
    ground_vertices.push(v2);
    
    ground_vertices.push(v0);
    ground_vertices.push(v2);
    ground_vertices.push(v3);

    ground_vertices.push(v0);
    ground_vertices.push(v2);
    ground_vertices.push(v1);

    ground_vertices.push(v0);
    ground_vertices.push(v3);
    ground_vertices.push(v2);

    let vbuf = renderer.device.create_buffer_init(&gizmo::wgpu::util::BufferInitDescriptor {
        label: Some("Ground VBuf"),
        contents: bytemuck::cast_slice(&ground_vertices),
        usage: gizmo::wgpu::BufferUsages::VERTEX,
    });
    
    let ground_mesh = gizmo::renderer::components::Mesh::new(
        std::sync::Arc::new(vbuf),
        &ground_vertices,
        Vec3::ZERO,
        "ground_mesh".to_string(),
    );

    let grass_tex = asset_manager.load_material_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout, "assets/grass.jpg").unwrap_or_else(|_| asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout));
    let grass_mat = Material::new(grass_tex).with_pbr(Vec4::new(1.0, 1.0, 1.0, 1.0), 0.9, 0.1);

    ObjectBuilder::new(world)
        .mesh(ground_mesh)
        .material(grass_mat)
        .collider(Collider::plane(Vec3::new(0.0, 1.0, 0.0), 0.5))
        .static_body_with_friction(1.0)
        .spawn();

    let cube_mesh = AssetManager::create_cube(&renderer.device);
    let box_tex = asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
    let _cube_mat = Material::new(box_tex.clone()).with_pbr(Vec4::new(0.8, 0.2, 0.2, 1.0), 0.5, 0.0);

    // Trigger Zone (Görünmez Sensör)
    let trigger_mat = Material::new(asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout))
        .with_pbr(Vec4::new(0.0, 1.0, 0.0, 0.3), 0.5, 0.0); // Yarı saydam yeşil
        
    let trigger_ent = world.spawn();
    world.add_component(trigger_ent, Transform::new(Vec3::new(0.0, 1.0, -5.0)).with_scale(Vec3::new(4.0, 2.0, 4.0)));
    world.add_component(trigger_ent, cube_mesh.clone());
    world.add_component(trigger_ent, trigger_mat);
    world.add_component(trigger_ent, MeshRenderer::new());
    
    let mut trigger_col = Collider::box_collider(Vec3::new(4.0, 2.0, 4.0));
    trigger_col.is_trigger = true; // SADECE SENSÖR! Fiziksel çarpışma olmaz
    world.add_component(trigger_ent, trigger_col);
    
    // Rigidbody ekliyoruz ama statik, çünkü sadece duracak
    world.add_component(trigger_ent, RigidBody::new_static());

    /*
    for y in 0..6 {
        let is_even = y % 2 == 0;
        let count = if is_even { 11 } else { 10 };
        for x in 0..count {
            let offset = if is_even { 0.0 } else { 0.5 };
            let pos_x = (x as f32 - 5.0 + offset) * 1.02;
            let pos = Vec3::new(pos_x, 2.0 + (y as f32) * 1.05, -15.0);

            ObjectBuilder::new(world)
                .position(pos)
                .scale(Vec3::splat(0.5))
                .mesh(cube_mesh.clone())
                .material(cube_mat.clone())
                .collider(Collider::box_collider(Vec3::splat(0.5)))
                .dynamic_body_with_props(20.0, 1.0, 0.0)
                .destructible()
                .spawn();
        }
    }
    */

    // Rampa
    ObjectBuilder::new(world)
        .position(Vec3::new(8.0, 1.0, -10.0))
        .scale(Vec3::new(3.0, 0.5, 5.0))
        .rotation(Quat::from_rotation_x(PI / 6.0))
        .mesh(cube_mesh.clone())
        .material(Material::new(box_tex.clone()).with_pbr(Vec4::new(0.2, 0.2, 0.8, 1.0), 0.5, 0.0))
        .collider(Collider::box_collider(Vec3::new(3.0, 0.5, 5.0)))
        .static_body()
        .spawn();

    // Güneş
    let sun_entity = world.spawn();
    world.add_component(sun_entity, Transform::new(Vec3::ZERO).with_rotation(Quat::from_rotation_x(-PI / 4.0)));
    world.add_component(sun_entity, gizmo::renderer::components::DirectionalLight::new(
        Vec3::new(1.0, 0.95, 0.9), 4.0, gizmo::renderer::components::LightRole::Sun
    ));

    // Kamera
    let camera_ent = world.spawn();
    world.add_component(
        camera_ent,
        Transform::new(Vec3::new(0.0, 2.0, 5.0)),
    );
    world.add_component(
        camera_ent,
        Camera::new(
            std::f32::consts::FRAC_PI_3,
            0.1,
            5000.0,
            0.0,
            0.0,
            true,
        ),
    );


    let sphere_mesh = AssetManager::create_sphere(&renderer.device, 0.5, 16, 16);
    let sphere_mat = Material::new(asset_manager.create_white_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout))
        .with_pbr(Vec4::new(0.1, 0.1, 0.1, 1.0), 0.1, 1.0); // Siyah metalik top

    // --- OYUNCAK ARABA (FİZİKSEL) ---
    let car_pos = Vec3::new(-10.0, 3.0, 0.0);
    let car_mat = Material::new(box_tex.clone()).with_pbr(Vec4::new(0.9, 0.7, 0.1, 1.0), 0.3, 0.8); // Altın sarısı kasa
    
    let chassis = ObjectBuilder::new(world)
        .position(car_pos)
        .scale(Vec3::new(2.0, 0.5, 4.0))
        .mesh(cube_mesh.clone())
        .material(car_mat)
        .collider(Collider::box_collider(Vec3::new(2.0, 0.5, 4.0)))
        .dynamic_body_with_props(100.0, 0.5, 0.1)
        .spawn();

    let wheel_offsets = [
        Vec3::new(-1.2, -0.2, 1.5),  // Sol Ön
        Vec3::new(1.2, -0.2, 1.5),   // Sağ Ön
        Vec3::new(-1.2, -0.2, -1.5), // Sol Arka
        Vec3::new(1.2, -0.2, -1.5),  // Sağ Arka
    ];

    for offset in wheel_offsets.iter() {
        let wheel = ObjectBuilder::new(world)
            .position(car_pos + *offset)
            .scale(Vec3::splat(0.8)) // Base radius is 0.5, scale 0.8 => 0.4
            .mesh(sphere_mesh.clone()) 
            .material(sphere_mat.clone()) // Lastik gibi siyah
            .collider(Collider::sphere(0.4))
            .dynamic_body_with_props(20.0, 1.5, 0.0) // Sürtünme yüksek
            .spawn();

        let mut hinge = gizmo::physics::joints::Joint::hinge(
            chassis,
            wheel,
            *offset,
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0), // X ekseni etrafında dönsün
        );

        if let gizmo::physics::joints::JointData::Hinge(ref mut h) = hinge.data {
            h.use_motor = true;
            h.motor_target_velocity = -15.0; // Tekerlekleri çevirerek ileri gitsin
            h.motor_max_force = 500.0;
        }

        phys_world.joints.push(hinge);
    }

    world.insert_resource(phys_world);
    world.insert_resource(asset_manager);
    world.insert_resource(FractureQueue { entities: Vec::new() });

    TestSandboxState {
        camera_speed: 15.0,
        camera_pitch: 0.0,
        camera_yaw: -PI / 2.0,
        camera_pos: Vec3::new(0.0, 2.0, 5.0),
        _sun_entity: sun_entity,
        sphere_mesh,
        sphere_mat,
    }
}

fn update(world: &mut World, state: &mut TestSandboxState, dt: f32, input: &gizmo::core::input::Input) {
    // --- KAMERA KONTROLÜ (Noclip FPS) ---
    if input.is_mouse_button_pressed(1) { // Sağ Tık
        let delta = input.mouse_delta();
        state.camera_yaw -= delta.0 * 0.005;
        state.camera_pitch -= delta.1 * 0.005;
        state.camera_pitch = state.camera_pitch.clamp(-PI / 2.0 + 0.1, PI / 2.0 - 0.1);
    }
    
    let fx = state.camera_yaw.cos() * state.camera_pitch.cos();
    let fy = state.camera_pitch.sin();
    let fz = state.camera_yaw.sin() * state.camera_pitch.cos();
    let forward = Vec3::new(fx, fy, fz).normalize();
    let right = forward.cross(Vec3::new(0.0, 1.0, 0.0)).normalize();
    let up = Vec3::new(0.0, 1.0, 0.0);

    let speed = if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::ShiftLeft as u32) { state.camera_speed * 3.0 } else { state.camera_speed };

    let mut cam_move = Vec3::ZERO;
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyW as u32) { cam_move += forward; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyS as u32) { cam_move -= forward; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyD as u32) { cam_move += right; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyA as u32) { cam_move -= right; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyE as u32) { cam_move += up; }
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyQ as u32) { cam_move -= up; }

    if cam_move.length_squared() > 0.0 {
        state.camera_pos += cam_move.normalize() * speed * dt;
    }

    if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Transform>, gizmo::core::query::Mut<Camera>)>() {
        let yaw_rot = Quat::from_rotation_y(-state.camera_yaw + std::f32::consts::FRAC_PI_2);
        let pitch_rot = Quat::from_rotation_x(state.camera_pitch);
        let rot = yaw_rot * pitch_rot;

        for (_, (mut trans, mut cam)) in q.iter_mut() {
            trans.position = state.camera_pos;
            trans.rotation = rot;
            cam.yaw = state.camera_yaw;
            cam.pitch = state.camera_pitch;
        }
    }

    // --- FİZİKSEL TOP FIRLATMA (Left Click) ---
    if input.is_mouse_button_just_pressed(0) {
        let ball = world.spawn();
        // Kameranın biraz önünden başlat
        let start_pos = state.camera_pos + forward * 1.5;
        world.add_component(ball, Transform::new(start_pos).with_scale(Vec3::splat(0.5)));
        world.add_component(ball, state.sphere_mesh.clone());
        world.add_component(ball, state.sphere_mat.clone());
        world.add_component(ball, MeshRenderer::new());
        world.add_component(ball, Collider::sphere(0.5));
        world.add_component(ball, RigidBody::new(50.0, 0.4, 0.8, true));
        // Topa ileriye doğru muazzam bir hız ver (Gülle gibi)
        world.add_component(ball, Velocity {
            linear: forward * 60.0,
            angular: Vec3::ZERO,
        });
    }

    // Parçalanma Tetikleyicisi (X tuşu)
    if input.is_key_just_pressed(gizmo::winit::keyboard::KeyCode::KeyX as u32) {
        let mut to_shatter = Vec::new();
        if let Some(q) = world.query::<&Destructible>() {
            for (id, _) in q.iter() {
                to_shatter.push(id);
            }
        }
        if let Some(mut queue) = world.get_resource_mut::<FractureQueue>() {
            queue.entities.extend(to_shatter);
        }
    }

    // CPU Physics
    let mut physics_dt = dt.min(0.1); // Cap maximum physics delta time to avoid tunneling
    while physics_dt > 0.0 {
        let step = physics_dt.min(0.016);
        gizmo::default_systems::cpu_physics_step_system(world, step);
        physics_dt -= step;
    }
    
    // --- Olayları Dinleme (Trigger Event Mimarisi) ---
    if let Some(events) = world.get_resource::<gizmo::core::event::Events<gizmo::physics::collision::TriggerEvent>>() {
        for event in events.iter() {
            use gizmo::physics::collision::CollisionEventType;
            let trigger_ent = event.trigger_entity;
            let other_ent = event.other_entity;
            
            match event.event_type {
                CollisionEventType::Started => {
                    println!("Entity {} GİRDİ -> Sensor {}", other_ent.id(), trigger_ent.id());
                    // Sensörün rengini kırmızı yap
                    if let Some(q) = world.query::<gizmo::core::query::Mut<Material>>() {
                        if let Some(mut mat) = q.get(trigger_ent.id()) {
                            mat.albedo = Vec4::new(1.0, 0.0, 0.0, 0.3); // Kırmızı
                        }
                    }
                }
                CollisionEventType::Persisting => {
                    // İçinde kalmaya devam ediyor
                }
                CollisionEventType::Ended => {
                    println!("Entity {} ÇIKTI -> Sensor {}", other_ent.id(), trigger_ent.id());
                    // Sensörün rengini eski haline (yeşil) çevir
                    if let Some(q) = world.query::<gizmo::core::query::Mut<Material>>() {
                        if let Some(mut mat) = q.get(trigger_ent.id()) {
                            mat.albedo = Vec4::new(0.0, 1.0, 0.0, 0.3); // Yeşil
                        }
                    }
                }
            }
        }
    }
}

fn render(
    world: &mut World,
    _state: &TestSandboxState,
    encoder: &mut gizmo::wgpu::CommandEncoder,
    view: &gizmo::wgpu::TextureView,
    renderer: &mut Renderer,
    _light_time: f32,
) {
    renderer.gpu_physics = None;
    
    // Yıkım kuyruğunu işle (Render pass'ten önce GPU bufferlarını yaratmak için)
    let mut to_shatter = Vec::new();
    if let Some(mut queue) = world.get_resource_mut::<FractureQueue>() {
        to_shatter = std::mem::take(&mut queue.entities);
    }

    let mut info = std::collections::HashMap::new();
    if let Some(q) = world.query::<(&Collider, &Transform, &Velocity)>() {
        for ent in &to_shatter {
            if let Some((c, t, v)) = q.get(*ent) {
                let mut extents = Vec3::splat(1.0);
                if let gizmo::physics::components::ColliderShape::Box(box_shape) = c.shape {
                    extents = box_shape.half_extents * 2.0;
                }
                info.insert(*ent, (extents, t.position, t.rotation, v.linear));
            }
        }
    }

    let mut chunk_mat_opt = None;
    if !to_shatter.is_empty() {
        if let Some(mut asset_manager) = world.get_resource_mut::<AssetManager>() {
            let box_tex = asset_manager.create_checkerboard_texture(&renderer.device, &renderer.queue, &renderer.scene.texture_bind_group_layout);
            chunk_mat_opt = Some(Material::new(box_tex).with_pbr(Vec4::new(0.8, 0.4, 0.1, 1.0), 0.6, 0.0));
        }
    }

    for ent in to_shatter {
        let (extents, original_pos, original_rot, original_vel) = match info.get(&ent) {
            Some(i) => *i,
            None => continue,
        };

        // Objeyi sil
        world.despawn_by_id(ent);

        // Voronoi ile parçala
        let chunks = gizmo::physics::fracture::voronoi_shatter(extents, 8, 42);
        
        let chunk_mat = chunk_mat_opt.clone().unwrap();

        for chunk in chunks {
            let mut vertices = Vec::new();
            for idx in &chunk.indices {
                let p = chunk.vertices[*idx as usize];
                let n = chunk.normals[*idx as usize];
                vertices.push(gizmo::renderer::gpu_types::Vertex {
                    position: [p.x, p.y, p.z],
                    color: [1.0, 1.0, 1.0],
                    normal: [n.x, n.y, n.z],
                    tex_coords: [0.0, 0.0],
                    joint_indices: [0, 0, 0, 0],
                    joint_weights: [0.0, 0.0, 0.0, 0.0],
                });
            }

            let vbuf = renderer.device.create_buffer_init(&gizmo::wgpu::util::BufferInitDescriptor {
                label: Some("Chunk VBuf"),
                contents: bytemuck::cast_slice(&vertices),
                usage: gizmo::wgpu::BufferUsages::VERTEX,
            });
            let chunk_mesh = gizmo::renderer::components::Mesh::new(
                std::sync::Arc::new(vbuf),
                &vertices,
                Vec3::ZERO,
                "fracture_chunk".to_string(),
            );

            let chunk_ent = world.spawn();
            // Parçanın yeni dünya pozisyonu
            let world_offset = original_rot.mul_vec3(chunk.center_of_mass);
            world.add_component(chunk_ent, Transform::new(original_pos + world_offset).with_rotation(original_rot));
            world.add_component(chunk_ent, chunk_mesh);
            world.add_component(chunk_ent, chunk_mat.clone());
            world.add_component(chunk_ent, MeshRenderer::new());
            
            // Ortalama bir kutu collider atıyoruz
            let r = (chunk.volume * 3.0).powf(0.33); // kaba bir yarıçap
            world.add_component(chunk_ent, Collider::box_collider(Vec3::splat(r * 0.5)));
            world.add_component(chunk_ent, RigidBody::new(chunk.volume * 50.0, 0.1, 0.8, true));
            
            // Rastgele bir patlama hızı ekle
            let explosion_dir = (chunk.center_of_mass).normalize_or_zero();
            world.add_component(chunk_ent, Velocity {
                linear: original_vel + explosion_dir * 5.0,
                angular: explosion_dir * 10.0,
            });
        }
    }

    gizmo::default_systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    App::<TestSandboxState>::new("Gizmo Engine - Test Sandbox", 1280, 720)
        .add_event::<gizmo::physics::collision::CollisionEvent>()
        .add_event::<gizmo::physics::collision::TriggerEvent>()
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run();
}
