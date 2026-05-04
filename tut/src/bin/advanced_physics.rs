use gizmo::prelude::*;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity, Breakable, Explosion};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::ragdoll::{RagdollBuilder, RagdollBoneDef, RagdollBoneType};
use gizmo::physics::joints::Joint;
use gizmo::physics::rope::Rope;
use gizmo::renderer::asset::AssetManager;
use gizmo::renderer::components::{Camera, Material, MeshRenderer, PointLight};

#[derive(Clone)]
struct GhostTrail {
    history: std::collections::VecDeque<Transform>,
    max_frames: usize,
}

impl gizmo::core::component::Component for GhostTrail {}

struct DemoState {
    camera_speed: f32,
    camera_pitch: f32,
    camera_yaw: f32,
    camera_pos: Vec3,
    rope: Option<Rope>,
    sphere_mesh: gizmo::renderer::components::Mesh,
    chunk_material: gizmo::renderer::components::Material,
}

fn setup(world: &mut World, renderer: &Renderer) -> DemoState {
    let mut asset_manager = AssetManager::new();

    // Camera
    let camera_ent = world.spawn();
    world.add_component(
        camera_ent,
        Transform::new(Vec3::new(0.0, 5.0, 15.0)).with_rotation(Quat::from_rotation_x(-0.2)),
    );
    world.add_component(
        camera_ent,
        Camera::new(
            std::f32::consts::FRAC_PI_3,
            0.1,
            5000.0,
            -std::f32::consts::FRAC_PI_2,
            -0.2,
            true,
        ),
    );
    world.add_component(camera_ent, gizmo::core::EntityName("Main Camera".into()));

    // Textures
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
    let sphere_mesh = AssetManager::create_sphere(&renderer.device, 1.0, 16, 16);

    // Skybox (Procedural)
    let skybox = world.spawn();
    let sky_mesh = AssetManager::create_sphere(&renderer.device, 2000.0, 32, 32);
    let white_tex = asset_manager.create_white_texture(
        &renderer.device,
        &renderer.queue,
        &renderer.scene.texture_bind_group_layout,
    );
    world.add_component(skybox, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(skybox, sky_mesh);
    world.add_component(
        skybox,
        Material::new(white_tex.clone())
            .with_unlit(Vec4::new(1.0, 1.0, 1.0, 1.0))
            .with_skybox(),
    );
    world.add_component(skybox, MeshRenderer::new());

    // Light
    let light = world.spawn();
    world.add_component(light, Transform::new(Vec3::new(0.0, 20.0, 0.0)));
    world.add_component(light, PointLight::new(Vec3::new(1.0, 1.0, 1.0), 500.0, 50.0));

    // Ground
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::ZERO));
    world.add_component(ground, plane_mesh.clone());
    world.add_component(
        ground,
        Material::new(ground_tex).with_pbr(Vec4::new(0.8, 0.8, 0.8, 1.0), 0.8, 0.1),
    );
    world.add_component(ground, MeshRenderer::new());
    world.add_component(ground, Collider::plane(Vec3::Y, 0.0));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());

    let mut phys_world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));

    // --- 1. DESTRUCTION SYSTEM DEMO ---
    // A glass wall that breaks
    for y in 0..5 {
        for x in 0..5 {
            let brick = world.spawn();
            // Ground top is at y=0.1. Brick half-height is 0.5. So start at y=0.6. Add a tiny gap (1.02) to prevent micro-collisions.
            let pos = Vec3::new(-5.0 + (x as f32) * 1.02, 0.6 + (y as f32) * 1.02, -5.0);
            world.add_component(brick, Transform::new(pos).with_scale(Vec3::new(0.5, 0.5, 0.5)));
            world.add_component(brick, cube_mesh.clone());
            world.add_component(brick, Material::new(box_tex.clone()).with_pbr(Vec4::new(0.3, 0.6, 1.0, 0.5), 0.2, 0.0));
            world.add_component(brick, MeshRenderer::new());
            world.add_component(brick, Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)));
            world.add_component(brick, RigidBody::new(10.0, 0.1, 0.5, true));
            world.add_component(brick, Velocity::default());
            // Add GhostTrail for debugging
            world.add_component(brick, GhostTrail {
                history: std::collections::VecDeque::new(),
                max_frames: 10,
            });
            // Make it breakable with high threshold to survive stacking
            world.add_component(brick, Breakable { threshold: 400.0, max_pieces: 4, is_broken: false });
        }
    }

    // --- 2. RAGDOLL DEMO ---
    let ragdoll_root = Vec3::new(5.0, 8.0, -5.0);
    let mut builder = RagdollBuilder::new(ragdoll_root);
    builder.create_humanoid();
    
    // Normally you'd spawn entities dynamically using the builder, but since the builder just creates defs,
    // we'll manually create some connected limbs for demo
    
    let mut prev_ent = None;
    for i in 0..4 {
        let limb = world.spawn();
        let pos = Vec3::new(5.0, 8.0 - (i as f32) * 1.2, 0.0);
        world.add_component(limb, Transform::new(pos).with_scale(Vec3::new(0.2, 0.5, 0.2)));
        world.add_component(limb, cube_mesh.clone());
        world.add_component(limb, Material::new(box_tex.clone()).with_pbr(Vec4::new(0.8, 0.2, 0.2, 1.0), 0.5, 0.5));
        world.add_component(limb, MeshRenderer::new());
        // Put limbs on layer 1 and ignore layer 1 collisions to prevent self-collision explosion
        world.add_component(limb, Collider::box_collider(Vec3::new(0.2, 0.5, 0.2)).with_layer(gizmo::physics::components::CollisionLayer { layer: 1, mask: !(1 << 1) }));
        // Lower restitution to prevent chaotic bouncing and joint solver explosions
        world.add_component(limb, RigidBody::new(5.0, 0.5, 0.1, true));
        world.add_component(limb, Velocity::default());

        if let Some(parent) = prev_ent {
            let fixed = Joint::fixed(
                parent,
                limb,
                Vec3::new(0.0, -0.6, 0.0),
                Vec3::new(0.0, 0.6, 0.0)
            ).with_break_force(f32::MAX, f32::MAX);
            phys_world.joints.push(fixed);
        }
        prev_ent = Some(limb);
    }


    // --- 3. ROPE DEMO ---
    let rope = Rope::new(
        Vec3::new(-5.0, 10.0, 5.0),
        Vec3::new(1.0, -0.2, 0.0),
        20,
        0.5,
        1.0,
        true
    );

    // Create visual spheres for rope nodes
    for _ in 0..rope.nodes.len() {
        let node_ent = world.spawn();
        world.add_component(node_ent, Transform::new(Vec3::ZERO).with_scale(Vec3::splat(0.2)));
        world.add_component(node_ent, sphere_mesh.clone());
        world.add_component(node_ent, Material::new(box_tex.clone()).with_pbr(Vec4::new(1.0, 0.8, 0.1, 1.0), 0.5, 0.5));
        world.add_component(node_ent, MeshRenderer::new());
        world.add_component(node_ent, gizmo::core::EntityName("RopeNode".into()));
    }

    world.insert_resource(phys_world);
    world.insert_resource(asset_manager);

    DemoState {
        camera_speed: 15.0,
        camera_pitch: -0.2,
        camera_yaw: -std::f32::consts::FRAC_PI_2,
        camera_pos: Vec3::new(0.0, 5.0, 15.0),
        rope: Some(rope),
        sphere_mesh: sphere_mesh.clone(),
        chunk_material: Material::new(box_tex.clone()).with_pbr(Vec4::new(0.5, 0.5, 0.5, 1.0), 0.5, 0.5),
    }
}

fn update(world: &mut World, state: &mut DemoState, dt: f32, input: &gizmo::core::input::Input) {
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);
    let mut cam_pos = Vec3::ZERO;

    if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Transform>, gizmo::core::query::Mut<Camera>)>() {
        for (_, (mut transform, mut camera)) in q.iter_mut() {
            let sensitivity = 0.002;
            let (dx, dy) = input.mouse_delta();
            
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

            let yaw_rot = Quat::from_rotation_y(-state.camera_yaw + std::f32::consts::FRAC_PI_2);
            let pitch_rot = Quat::from_rotation_x(state.camera_pitch);
            transform.rotation = yaw_rot * pitch_rot;

            let speed = state.camera_speed * dt * if input.is_key_pressed(KeyCode::ShiftLeft as u32) { 3.0 } else { 1.0 };

            if input.is_key_pressed(KeyCode::KeyW as u32) { state.camera_pos += forward * speed; }
            if input.is_key_pressed(KeyCode::KeyS as u32) { state.camera_pos -= forward * speed; }
            if input.is_key_pressed(KeyCode::KeyA as u32) { state.camera_pos -= right * speed; }
            if input.is_key_pressed(KeyCode::KeyD as u32) { state.camera_pos += right * speed; }
            if input.is_key_pressed(KeyCode::Space as u32) { state.camera_pos += up * speed; }

            transform.position = state.camera_pos;
            transform.update_local_matrix();
            
            camera.yaw = state.camera_yaw;
            camera.pitch = state.camera_pitch;
            
            cam_forward = forward;
            cam_pos = transform.position;
        }
    }

    // Left Click: Explosion (Trigger Destruction)
    if input.is_mouse_button_pressed(gizmo::core::input::mouse::LEFT) {
        let mut hit_pos = None;
        if let Some(phys) = world.get_resource::<PhysicsWorld>() {
            let ray = gizmo::physics::raycast::Ray::new(cam_pos, cam_forward);
            if let Some(hit) = phys.raycast(&ray, 100.0) {
                hit_pos = Some(hit.point);
            }
        }
        
        if let Some(pos) = hit_pos {
            let exp_ent = world.spawn();
            world.add_component(exp_ent, Transform::new(pos));
            world.add_component(exp_ent, Explosion {
                radius: 5.0,
                force: 5000.0,
                is_active: true,
            });
        }
    }
    // Break joints on X key
    if input.is_key_pressed(gizmo::winit::keyboard::KeyCode::KeyX as u32) {
        if let Some(mut phys_world) = world.get_resource_mut::<PhysicsWorld>() {
            for joint in &mut phys_world.joints {
                joint.is_broken = true;
            }
        }
    }

    // 1. Compute and update vectors (v, a, f) and draw them
    if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
        if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Velocity>, &Transform, &RigidBody)>() {
            for (_, (mut vel, trans, rb)) in q.iter_mut() {
                // Calculate acceleration: a = (v_current - v_last) / dt
                let acceleration = (vel.linear - vel.last_linear) / dt.max(0.0001);
                
                // Force = mass * acceleration (Approximation for external forces)
                vel.force = acceleration * rb.mass;
                
                // Draw Velocity (Green)
                gizmos.draw_line(trans.position, trans.position + vel.linear * 0.1, [0.0, 1.0, 0.0, 1.0]);
                
                // Draw Acceleration (Yellow)
                gizmos.draw_line(trans.position, trans.position + acceleration * 0.01, [1.0, 1.0, 0.0, 1.0]);
                
                // Draw Force (Red)
                gizmos.draw_line(trans.position, trans.position + vel.force * 0.005, [1.0, 0.0, 0.0, 1.0]);

                vel.last_linear = vel.linear;
            }
        }
        
        // 2. Draw Ghosting (İz Bırakma)
        if let Some(mut q) = world.query::<(gizmo::core::query::Mut<GhostTrail>, &Transform, &Collider)>() {
            for (_, (mut ghost, trans, col)) in q.iter_mut() {
                // Store current frame
                ghost.history.push_front(*trans);
                if ghost.history.len() > ghost.max_frames {
                    ghost.history.pop_back();
                }
                
                // Draw ghosts
                for (i, g_trans) in ghost.history.iter().enumerate() {
                    let alpha = 1.0 - (i as f32 / ghost.max_frames as f32);
                    let color = [1.0, 1.0, 1.0, alpha * 0.5]; // Semi-transparent white
                    
                    match &col.shape {
                        gizmo::physics::components::ColliderShape::Box(b) => {
                            let h = b.half_extents;
                            let p0 = g_trans.local_matrix.transform_point3(Vec3::new(-h.x, -h.y, -h.z));
                            let p1 = g_trans.local_matrix.transform_point3(Vec3::new( h.x, -h.y, -h.z));
                            let p2 = g_trans.local_matrix.transform_point3(Vec3::new( h.x,  h.y, -h.z));
                            let p3 = g_trans.local_matrix.transform_point3(Vec3::new(-h.x,  h.y, -h.z));
                            let p4 = g_trans.local_matrix.transform_point3(Vec3::new(-h.x, -h.y,  h.z));
                            let p5 = g_trans.local_matrix.transform_point3(Vec3::new( h.x, -h.y,  h.z));
                            let p6 = g_trans.local_matrix.transform_point3(Vec3::new( h.x,  h.y,  h.z));
                            let p7 = g_trans.local_matrix.transform_point3(Vec3::new(-h.x,  h.y,  h.z));
                            
                            gizmos.draw_line(p0, p1, color); gizmos.draw_line(p1, p2, color);
                            gizmos.draw_line(p2, p3, color); gizmos.draw_line(p3, p0, color);
                            gizmos.draw_line(p4, p5, color); gizmos.draw_line(p5, p6, color);
                            gizmos.draw_line(p6, p7, color); gizmos.draw_line(p7, p4, color);
                            gizmos.draw_line(p0, p4, color); gizmos.draw_line(p1, p5, color);
                            gizmos.draw_line(p2, p6, color); gizmos.draw_line(p3, p7, color);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    gizmo::systems::cpu_physics_step_system(world, dt);
    gizmo::physics::physics_fracture_system(world, dt);
    gizmo::physics::physics_explosion_system(world, dt);
    gizmo::systems::physics::physics_debug_system(world);

    // Give visual meshes to chunks created by fracture system
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

    // Rope Update
    if let Some(ref mut rope) = state.rope {
        rope.step(dt, Vec3::new(0.0, -9.81, 0.0));
        
        let mut node_idx = 0;
        if let Some(mut q) = world.query::<(gizmo::core::query::Mut<Transform>, &gizmo::core::EntityName)>() {
            for (_, (mut trans, name)) in q.iter_mut() {
                if name.0 == "RopeNode" {
                    if node_idx < rope.nodes.len() {
                        trans.position = rope.nodes[node_idx].position;
                        node_idx += 1;
                    }
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
    renderer.gpu_physics = None;
    gizmo::systems::default_render_pass(world, encoder, view, renderer);
}

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();
    App::<DemoState>::new("Gizmo Engine - Advanced Physics Demo", 1280, 720)
        .set_setup(setup)
        .set_update(update)
        .set_render(render)
        .run();
}
