use gizmo::physics::world::PhysicsWorld;
use gizmo::prelude::*;
use super::{DemoState, PendingDecal};

pub(super) fn update(world: &mut World, state: &mut DemoState, dt: f32, input: &gizmo::core::input::Input) {
    let mut is_console_open = false;
    if let Some(console_state) = world.get_resource::<gizmo::core::cvar::DevConsoleState>() {
        is_console_open = console_state.is_open;
    }

    let mut throttle = 0.0;
    let mut brake = 0.0;
    let mut air_torque = 0.0;

    if !is_console_open {
        if input.is_key_pressed(KeyCode::KeyW as u32)
            || input.is_key_pressed(KeyCode::KeyD as u32)
            || input.is_key_pressed(KeyCode::ArrowUp as u32)
            || input.is_key_pressed(KeyCode::ArrowRight as u32)
        {
            throttle = 1.0;
            air_torque = 1500.0; // Lean back (+Z rotation is nose up when facing right)
        } else if input.is_key_pressed(KeyCode::KeyS as u32)
            || input.is_key_pressed(KeyCode::KeyA as u32)
            || input.is_key_pressed(KeyCode::ArrowDown as u32)
            || input.is_key_pressed(KeyCode::ArrowLeft as u32)
        {
            brake = 1.0;
            air_torque = -1500.0; // Lean forward (-Z rotation is nose down when facing right)
        }
    }

    // --- CVAR (Developer Console) UYGULAMASI ---
    if let Some(registry) = world.get_resource::<gizmo::core::cvar::CVarRegistry>() {
        // Yerçekimini canlı güncelle
        if let Some(gizmo::core::cvar::CVarValue::Float(g)) = registry.get("physics_gravity_y") {
            if let Ok(mut phys) =
                world.try_get_resource_mut::<gizmo::physics::world::PhysicsWorld>()
            {
                phys.integrator.gravity = Vec3::new(0.0, *g, 0.0);
            }
        }

        // Tork gücünü canlı güncelle
        if let Some(gizmo::core::cvar::CVarValue::Float(t)) = registry.get("car_torque") {
            if throttle > 0.0 {
                air_torque = *t;
            } else if brake > 0.0 {
                air_torque = -*t;
            }
        }
    }

    // --- PHYSICS DEBUG CONTROLS ---
    if !is_console_open {
        if let Ok(mut phys_world) =
            world.try_get_resource_mut::<gizmo::physics::world::PhysicsWorld>()
        {
            // P: Pause toggle
            if input.is_key_just_pressed(KeyCode::KeyP as u32) {
                phys_world.is_paused = !phys_world.is_paused;
                tracing::warn!("Physics Paused: {}", phys_world.is_paused);
            }

            // O: Step once (when paused)
            if input.is_key_pressed(KeyCode::KeyO as u32) {
                phys_world.step_once = true;
                tracing::warn!("Physics Step Triggered");
            }

            // R: Rewind time! (Hold to continuously rewind)
            if input.is_key_pressed(KeyCode::KeyR as u32) {
                phys_world.rewind_requested = true;
            }
        }
    }

    // Air Control and Inputs
    // SAFETY: single-threaded demo; VehicleController, RigidBody, Velocity are distinct component types and never alias.
    if let Some(mut q_v) = unsafe {
        world.query_unchecked::<gizmo::core::query::Mut<gizmo::physics::vehicle::VehicleController>>()
    } {
        if let Some(mut vehicle) = q_v.get_mut(state.car_entity.id()) {
            let is_reverse = brake > 0.0 && vehicle.current_speed_kmh < 1.0;
            vehicle.set_reverse(is_reverse);

            if vehicle.reverse_input {
                // In reverse, the S key (brake) acts as the gas pedal
                vehicle.throttle_input = brake;
                vehicle.brake_input = throttle; // W key becomes the brake when reversing
            } else {
                vehicle.throttle_input = throttle;
                vehicle.brake_input = brake;
            }

            if throttle > 0.0 || brake > 0.0 {
                if let Some(mut q_rb) = unsafe { world.query_unchecked::<gizmo::core::query::Mut<RigidBody>>() } {
                    if let Some(mut rb) = q_rb.get_mut(state.car_entity.id()) {
                        rb.wake_up();
                    }
                }
            }

            // Check if grounded to apply air control
            let is_grounded = vehicle.wheels.iter().any(|w| w.is_grounded);
            if !is_grounded && air_torque != 0.0 {
                if let Some(mut q_rb) = unsafe { world.query_unchecked::<gizmo::core::query::Mut<Velocity>>() } {
                    if let Some(mut vel) = q_rb.get_mut(state.car_entity.id()) {
                        // Directly applying torque scaled by dt
                        vel.angular.z += air_torque * dt * 0.005;
                    }
                }
            }
        }
    }

    // Step physics
    gizmo::systems::cpu_physics_step_system(world, dt);

    if state.show_physics_debug {
        gizmo::systems::physics::physics_debug_system(world);
    }

    // Sync Visual Wheels and Spawns Particles
    let mut car_pos = Vec3::ZERO;
    let mut car_rot = Quat::IDENTITY;
    if let Some(q) = world.query::<&Transform>() {
        if let Some(t) = q.get(state.car_entity.id()) {
            car_pos = t.position;
            car_rot = t.rotation;
        }
    }

    if let Some(q_v) = world.query::<&gizmo::physics::vehicle::VehicleController>() {
        if let Some(vehicle) = q_v.get(state.car_entity.id()) {
            let speed = vehicle.current_speed_kmh;
            let mut pending = state.pending_particles.borrow_mut();

            if let Some(mut q_t) = unsafe { world.query_unchecked::<gizmo::core::query::Mut<Transform>>() } {
                for i in 0..4 {
                    let wheel = &vehicle.wheels[i];
                    let anchor_world = car_pos + car_rot.mul_vec3(wheel.attachment_local_pos);
                    let up = car_rot.mul_vec3(Vec3::new(0.0, 1.0, 0.0));
                    let wheel_world_pos = anchor_world - up * wheel.suspension_length;

                    if let Some(mut wt) = q_t.get_mut(state.wheel_entities[i].id()) {
                        wt.set_position(wheel_world_pos);

                        let align_rot = Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2);
                        let spin_rot = Quat::from_rotation_x(wheel.rotation_angle);
                        wt.set_rotation(car_rot * spin_rot * align_rot);

                        if let Some(new_radius) = state.update_wheel_radius {
                            wt.set_scale(Vec3::splat(new_radius));
                        }
                    }

                    if let Some(mut st) = q_t.get_mut(state.suspension_entities[i].id()) {
                        st.set_position((anchor_world + wheel_world_pos) * 0.5);
                        let mut scale = st.scale;
                        scale.y = wheel.suspension_length * 0.5;
                        st.set_scale(scale);
                        st.set_rotation(car_rot); // Keep strut upright
                    }

                    // Dust particles ONLY for grounded wheels
                    if wheel.is_grounded
                        && (speed > 5.0 || (speed < 5.0 && (throttle > 0.0 || brake > 0.0)))
                    {
                        let pos_bottom = wheel_world_pos - Vec3::new(0.0, wheel.radius * 0.9, 0.0);
                        for _ in 0..2 {
                            let vx = (rand::random::<f32>() - 0.5) * 1.5; // less horizontal spread
                            let vy = rand::random::<f32>() * 1.5 + 0.5; // gentle lift
                            let vz = (rand::random::<f32>() - 0.5) * 1.5;

                            pending.push(gizmo::renderer::gpu_particles::GpuParticle {
                                position: [pos_bottom.x, pos_bottom.y, pos_bottom.z],
                                life: 0.4 + rand::random::<f32>() * 0.3, // Shorter life
                                velocity: [vx, vy, vz],
                                max_life: 0.8,
                                color: [0.55, 0.45, 0.35, 0.5], // Brownish dust
                                size_start: 0.2 + rand::random::<f32>() * 0.2, // Very small
                                size_end: 0.8 + rand::random::<f32>() * 0.4, // Small dissipation
                                _padding: [0.0; 2],
                            });
                        }

                        // If slipping heavily or braking hard, spawn a tire track
                        if brake > 0.0 || (throttle > 0.0 && speed < 10.0) {
                            state.pending_decals.borrow_mut().push(PendingDecal {
                                position: pos_bottom,
                                rotation: Quat::from_rotation_y(car_rot.to_euler(EulerRot::YXZ).0),
                            });
                        }
                    }
                }
            }
        }
    }

    let mut car_vel = Vec3::ZERO;
    if let Some(q) = world.query::<&Velocity>() {
        if let Some(v) = q.get(state.car_entity.id()) {
            car_vel = v.linear;
        }
    }

    // Camera follow
    if let Some(mut q) = world.query_mut::<(gizmo::core::query::Mut<Transform>, &Camera)>() {
        for (_, (mut cam_trans, _)) in q.iter_mut() {
            let speed = car_vel.length();
            let look_ahead = if speed > 1.0 {
                (car_vel / speed) * (speed * 0.3).min(15.0)
            } else {
                Vec3::ZERO
            };
            let zoom_out = (speed * 0.2).min(15.0);

            let mut dynamic_offset = state.camera_offset;
            dynamic_offset.z += zoom_out;
            dynamic_offset.y += zoom_out * 0.2;

            let desired_pos = car_pos + look_ahead + dynamic_offset;
            cam_trans.position = cam_trans.position.lerp(desired_pos, dt * 3.0);
            cam_trans.update_local_matrix();
        }
    }

    let pending_decals = state.pending_decals.replace(Vec::new());
    if !pending_decals.is_empty() {
        for pd in pending_decals {
            if state.decals.len() < 400 {
                let decal = world.spawn();
                world.add_component(
                    decal,
                    Transform::new(pd.position)
                        .with_scale(Vec3::new(1.0, 2.0, 1.0)) // Height must be enough to hit the ground
                        .with_rotation(pd.rotation),
                );
                world.add_component(
                    decal,
                    gizmo::renderer::components::Decal::new(
                        state.tire_track_bg.clone(),
                        Vec4::new(0.05, 0.05, 0.05, 0.9),
                    ),
                );
                state.decals.push(decal);
            } else {
                let decal = state.decals[state.decal_index];
                if let Some(mut q_t) = world.query_mut::<gizmo::core::query::Mut<Transform>>() {
                    if let Some(mut dt) = q_t.get_mut(decal.id()) {
                        dt.position = pd.position;
                        dt.rotation = pd.rotation;
                    }
                }
                state.decal_index = (state.decal_index + 1) % 400;
            }
        }
    }

    // Audio Update
    if let Some(audio_manager) = &mut state.audio_manager {
        audio_manager.update();

        // Engine Pitch
        if let Some(id) = state.engine_audio_id {
            let speed = car_vel.length();
            let base_pitch = 0.5; // Idle
            let target_pitch = base_pitch + (speed * 0.05_f32) + (throttle * 0.4_f32).max(brake * 0.4_f32);
            audio_manager.set_pitch(id, target_pitch);

            let volume = 0.1_f32 + (throttle * 0.3_f32).max(brake * 0.3_f32) + (speed * 0.01_f32).min(0.2_f32);
            audio_manager.set_volume(id, volume);
        }

        // Crash sounds
        if let Ok(phys_world) = world.try_get_resource::<PhysicsWorld>() {
            for event in phys_world.collision_events() {
                // If collision is strong enough
                let max_impulse = event
                    .contact_points
                    .iter()
                    .map(|c| c.normal_impulse)
                    .fold(0.0_f32, f32::max);
                if max_impulse > 1000.0 {
                    // arbitrary threshold
                    if let Ok(id) = audio_manager.play("crash") {
                        let vol = (max_impulse / 10000.0).clamp(0.1, 1.0);
                        audio_manager.set_volume(id, vol);
                    }
                }
            }
        }
    }

    state.update_wheel_radius = None;
}
