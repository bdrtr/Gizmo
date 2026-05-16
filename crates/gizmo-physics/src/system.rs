use crate::components::{Collider, RigidBody, Transform, Velocity};
use crate::soft_body::SoftBodyMesh;
use crate::world::PhysicsWorld;
use gizmo_core::entity::Entity;
use gizmo_core::query::{Mut, Query};
use gizmo_core::world::World;

/// Exclusive system that updates the entire physics simulation.
/// It reads all rigid and soft bodies from the ECS, steps the physics world,
/// and writes the transformed positions and velocities back to the ECS.
#[tracing::instrument(skip_all, name = "physics_step_system")]
pub fn physics_step_system(world: &World, dt: f32) {
    // Record profiler scope (if FrameProfiler resource is available)
    if let Ok(mut profiler) = world.try_get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
        profiler.begin_scope("physics_total");
    }

    // 1. Acquire PhysicsWorld Resource
    let mut physics_world = match world.try_get_resource_mut::<PhysicsWorld>() {
        Ok(res) => res,
        Err(e) => {
            tracing::info!("[Physics] FAILED TO GET PhysicsWorld Resource: {:?}", e);
            return;
        }
    };

    // 2. Gather Compound Shapes (Read Locks Only)
    let mut compound_shapes_map = std::collections::HashMap::new();
    {
        let col_storage = world.borrow::<Collider>();
        let children_storage = world.borrow::<gizmo_core::component::Children>();
        let trans_storage = world.borrow::<Transform>();
        let rb_storage = world.borrow::<RigidBody>();
        let pooled_storage = world.borrow::<gizmo_core::pool::Pooled>();
        let deleted_storage = world.borrow::<gizmo_core::component::IsDeleted>();

        for (id, _rb) in rb_storage.iter() {
            // Pooled veya silinmiş nesneleri simüle etme
            if pooled_storage.get(id).is_some() || deleted_storage.get(id).is_some() {
                continue;
            }
            if let Some(transform) = trans_storage.get(id) {
                let mut compound_shapes = Vec::new();

                // Check self
                if let Some(c) = col_storage.get(id) {
                    compound_shapes.push((
                        crate::components::Transform::default(),
                        Box::new(c.shape.clone()),
                    ));
                }

                // Check children recursively
                let mut stack = vec![id];
                while let Some(curr_id) = stack.pop() {
                    if let Some(children) = children_storage.get(curr_id) {
                        for &child_id in &children.0 {
                            stack.push(child_id);
                            if let Some(child_trans) = trans_storage.get(child_id) {
                                if let Some(child_col) = col_storage.get(child_id) {
                                    // Compute local transform relative to the root
                                    let inv_rot = transform.rotation.inverse();
                                    let local_pos =
                                        inv_rot.mul_vec3(child_trans.position - transform.position);
                                    let local_rot = inv_rot * child_trans.rotation;

                                    let local_t = crate::components::Transform::new(local_pos)
                                        .with_rotation(local_rot);
                                    compound_shapes
                                        .push((local_t, Box::new(child_col.shape.clone())));
                                }
                            }
                        }
                    }
                }

                // Create a single Collider for this RigidBody
                let final_collider = if compound_shapes.is_empty() {
                    Collider::default() // Should technically not be simulated
                } else if compound_shapes.len() == 1 {
                    // Single collider, avoid nesting in Compound
                    let (_t, s) = compound_shapes.remove(0);
                    Collider {
                        shape: *s,
                        ..Default::default()
                    }
                } else {
                    Collider {
                        shape: crate::components::ColliderShape::Compound(compound_shapes),
                        ..Default::default()
                    }
                };

                compound_shapes_map.insert(id, final_collider);
            }
        }
    } // Read locks are dropped here!

    // 3. Query Rigid Bodies (Write Locks)
    let mut rigid_bodies = Vec::new();
    if let Some(query) = Query::<(
        Mut<RigidBody>,
        Mut<Transform>,
        Mut<Velocity>,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
    )>::new(world)
    {
        let deleted_storage = world.borrow::<gizmo_core::component::IsDeleted>();
        for (id, (rb, transform, vel, _)) in query.iter() {
            if deleted_storage.get(id).is_some() {
                continue;
            }
            if let Some(final_collider) = compound_shapes_map.remove(&id) {
                rigid_bodies.push((Entity::new(id, 0), *rb, *transform, *vel, final_collider));
            }
        }
    } else {
        tracing::info!("[Physics] FAILED TO BORROW RigidBody/Transform/Velocity Mutably!");
    }

    // 3.5. Query Soft Bodies
    let mut soft_bodies = Vec::new();
    if let Some(soft_query) = Query::<(
        Mut<SoftBodyMesh>,
        Mut<Transform>,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
    )>::new(world)
    {
        for (id, (soft_mesh, transform, _)) in soft_query.iter() {
            soft_bodies.push((Entity::new(id, 0), soft_mesh.clone(), *transform));
        }
    }

    // 3.5. Update Vehicles
    let all_colliders: Vec<(Entity, Transform, Collider)> = rigid_bodies
        .iter()
        .map(|(ent, _, trans, _, col)| (*ent, *trans, col.clone()))
        .collect();

    let is_paused =
        physics_world.is_paused && !physics_world.step_once && !physics_world.rewind_requested;

    if !is_paused {
        let vehicle_query_opt = Query::<(
            Mut<crate::vehicle::VehicleController>,
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
        )>::new(world);
        if let Some(vehicle_query) = &vehicle_query_opt {
            for (id, (mut vehicle, _)) in vehicle_query.iter() {
                if let Some((ent, rb, trans, vel, _col)) =
                    rigid_bodies.iter_mut().find(|(e, ..)| e.id() == id)
                {
                    crate::vehicle::update_vehicle(
                        *ent,
                        &mut vehicle,
                        rb,
                        trans,
                        vel,
                        &all_colliders,
                        dt,
                    );
                }
            }
        }

        // 3.6. Update Character Controllers
        let kcc_query_opt = Query::<(
            Mut<crate::components::CharacterController>,
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
        )>::new(world);
        if let Some(kcc_query) = &kcc_query_opt {
            let mut vel_storage = world.borrow_mut::<Velocity>();
            let col_storage = world.borrow::<Collider>();

            for (id, (mut kcc, _)) in kcc_query.iter() {
                if let Some((ent, _rb, trans, vel, col)) =
                    rigid_bodies.iter_mut().find(|(e, ..)| e.id() == id)
                {
                    crate::character::update_character(
                        *ent,
                        &mut kcc,
                        trans,
                        vel,
                        col,
                        &all_colliders,
                        dt,
                    );
                } else if let Some(trans) = world.borrow_mut::<Transform>().get_mut(id) {
                    if let (Some(vel), Some(col)) = (vel_storage.get_mut(id), col_storage.get(id)) {
                        crate::character::update_character(
                            Entity::new(id, 0),
                            &mut kcc,
                            trans,
                            vel,
                            col,
                            &all_colliders,
                            dt,
                        );
                    }
                }
            }
        }
    }

    // 3.7. Extract Fluid Simulations
    let mut fluid_sims = Vec::new();
    let fluid_query_opt = Query::<(
        Mut<crate::components::FluidSimulation>,
        Mut<Transform>,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
    )>::new(world);
    if let Some(fluid_query) = &fluid_query_opt {
        for (id, (fluid, transform, _)) in fluid_query.iter() {
            fluid_sims.push((Entity::new(id, 0), fluid.clone(), *transform));
        }
    }

    // 4. Step Simulation
    physics_world.sync_bodies(rigid_bodies.iter());

    physics_world.step(&mut soft_bodies, &mut fluid_sims, dt).expect("Gizmo Physics Engine encountered a critical numerical error (NaN, Infinity, or Overflow) and halted!");

    // Sync back fluids
    if let Some(fluid_query) = &fluid_query_opt {
        for (id, (mut fluid, mut transform, _)) in fluid_query.iter() {
            if let Some((_, f, t)) = fluid_sims.iter().find(|(e, _, _)| e.id() == id) {
                *fluid = f.clone();
                *transform = *t;
            }
        }
    }

    // Sync back to rigid_bodies so vehicles/ECS writeback works
    for i in 0..physics_world.entities.len() {
        let entity_id = physics_world.entities[i].id();
        if let Some((_, rb, trans, vel, _)) =
            rigid_bodies.iter_mut().find(|(e, ..)| e.id() == entity_id)
        {
            *rb = physics_world.rigid_bodies[i];
            *trans = physics_world.transforms[i];
            *vel = physics_world.velocities[i];
        }
    }

    // 5. Write back to ECS (Rigid Bodies)
    if !rigid_bodies.is_empty() {
        if let Some(query) = Query::<(
            Mut<RigidBody>,
            Mut<Transform>,
            Mut<Velocity>,
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
        )>::new(world)
        {
            for (entity, rb, transform, vel, _collider) in rigid_bodies {
                if let Some((mut ecs_rb, mut ecs_trans, mut ecs_vel, _)) = query.get(entity.id()) {
                    *ecs_rb = rb;
                    *ecs_trans = transform;
                    *ecs_vel = vel;
                }
            }
        }
    }

    // 6. Write back to ECS (Soft Bodies)
    if !soft_bodies.is_empty() {
        if let Some(soft_query) = Query::<(
            Mut<SoftBodyMesh>,
            Mut<Transform>,
            gizmo_core::query::Without<gizmo_core::pool::Pooled>,
        )>::new(world)
        {
            for (entity, soft_mesh, transform) in soft_bodies {
                if let Some((mut sm, mut t, _)) = soft_query.get(entity.id()) {
                    sm.nodes.clone_from(&soft_mesh.nodes);
                    *t = transform;
                }
            }
        }
    }

    // 7. Dispatch Events
    if let Ok(mut trigger_queue) =
        world.try_get_resource_mut::<gizmo_core::event::Events<crate::collision::TriggerEvent>>()
    {
        for event in &physics_world.trigger_events {
            trigger_queue.send(event.clone());
        }
    }

    if let Ok(mut collision_queue) =
        world.try_get_resource_mut::<gizmo_core::event::Events<crate::collision::CollisionEvent>>()
    {
        for event in &physics_world.collision_events {
            collision_queue.send(event.clone());
        }
    }

    if physics_world.step_once {
        physics_world.step_once = false;
    }

    // Close profiler scope
    drop(physics_world); // PhysicsWorld lock'unu bırak
    if let Ok(mut profiler) = world.try_get_resource_mut::<gizmo_core::profiler::FrameProfiler>() {
        profiler.end_scope("physics_total");
    }
}

/// System that processes collision events and breaks objects that exceed their threshold.
pub fn physics_fracture_system(world: &World, dt: f32) {
    use crate::components::Breakable;
    use gizmo_core::commands::Commands;
    use gizmo_core::system::SystemParam;

    let physics_world = match world.try_get_resource::<PhysicsWorld>() {
        Ok(res) => res,
        Err(_) => return,
    };

    let mut commands = match Commands::fetch(world, dt) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut shattered = std::collections::HashSet::new();

    let query_opt = Query::<(
        gizmo_core::query::Mut<Breakable>,
        &Transform,
        &Collider,
        &Velocity,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
    )>::new(world);
    let query = match query_opt {
        Some(q) => q,
        None => return,
    };

    for event in &physics_world.collision_events {
        let mut max_impulse = 0.0;
        let mut impact_normal = gizmo_math::Vec3::ZERO;
        let mut impact_point = gizmo_math::Vec3::ZERO;

        for contact in &event.contact_points {
            if contact.normal_impulse > max_impulse {
                max_impulse = contact.normal_impulse;
                impact_normal = contact.normal;
                impact_point = contact.point;
            }
        }

        // Fallback: estimate impact from relative velocity when solver impulse is unavailable
        if max_impulse <= 0.0 && !event.contact_points.is_empty() {
            // Look up velocities of both entities to estimate impact force
            let vel_a = physics_world
                .entity_index_map
                .get(&event.entity_a.id())
                .map(|&idx| physics_world.velocities[idx].linear)
                .unwrap_or(gizmo_math::Vec3::ZERO);
            let vel_b = physics_world
                .entity_index_map
                .get(&event.entity_b.id())
                .map(|&idx| physics_world.velocities[idx].linear)
                .unwrap_or(gizmo_math::Vec3::ZERO);
            let mass_a = physics_world
                .entity_index_map
                .get(&event.entity_a.id())
                .map(|&idx| physics_world.rigid_bodies[idx].mass)
                .unwrap_or(1.0);
            let mass_b = physics_world
                .entity_index_map
                .get(&event.entity_b.id())
                .map(|&idx| physics_world.rigid_bodies[idx].mass)
                .unwrap_or(1.0);

            let rel_speed = (vel_b - vel_a).length();
            let reduced_mass = if mass_a > 0.0 && mass_b > 0.0 {
                (mass_a * mass_b) / (mass_a + mass_b)
            } else {
                mass_a.max(mass_b)
            };
            max_impulse = rel_speed * reduced_mass;
            if let Some(contact) = event.contact_points.first() {
                impact_normal = contact.normal;
                impact_point = contact.point;
            }
        }

        if max_impulse <= 0.0 {
            continue;
        }

        // Check Entity A
        if !shattered.contains(&event.entity_a.id()) {
            if let Some((mut breakable, transform, collider, vel, _)) =
                query.get(event.entity_a.id())
            {
                if !breakable.is_broken && max_impulse > breakable.threshold {
                    breakable.current_health -= max_impulse;
                    if breakable.current_health <= 0.0 {
                        breakable.is_broken = true;
                        shattered.insert(event.entity_a.id());
                        shatter_entity(
                            &mut commands,
                            event.entity_a,
                            &breakable,
                            transform,
                            collider,
                            vel,
                            -impact_normal,
                            impact_point,
                        );
                    }
                }
            }
        }

        // Check Entity B
        if !shattered.contains(&event.entity_b.id()) {
            if let Some((mut breakable, transform, collider, vel, _)) =
                query.get(event.entity_b.id())
            {
                if !breakable.is_broken && max_impulse > breakable.threshold {
                    breakable.current_health -= max_impulse;
                    if breakable.current_health <= 0.0 {
                        breakable.is_broken = true;
                        shattered.insert(event.entity_b.id());
                        shatter_entity(
                            &mut commands,
                            event.entity_b,
                            &breakable,
                            transform,
                            collider,
                            vel,
                            impact_normal,
                            impact_point,
                        );
                    }
                }
            }
        }
    }
    drop(query);
}

fn shatter_entity(
    commands: &mut gizmo_core::commands::Commands,
    entity: Entity,
    breakable: &crate::components::Breakable,
    transform: &Transform,
    collider: &Collider,
    vel: &Velocity,
    impact_direction: gizmo_math::Vec3,
    _impact_point: gizmo_math::Vec3,
) {
    use crate::fracture::voronoi_shatter;

    // We only support shattering boxes for now
    let extents = match &collider.shape {
        crate::components::ColliderShape::Box(b) => b.half_extents,
        _ => return, // Cannot shatter non-boxes easily with our voronoi yet
    };

    // Despawn the original entity
    commands.entity(entity).despawn();

    // Generate chunks
    let chunks = voronoi_shatter(extents, breakable.max_pieces, 42);

    for chunk in chunks {
        // Create new convex hull colliders or approximated boxes for the chunks.
        // For simplicity, we approximate each chunk with a small sphere or box based on its volume.
        // A full implementation would use ConvexHull shapes.
        let radius = (chunk.volume * 0.1).powf(1.0 / 3.0).max(0.1);

        // Offset chunk center by parent's transform
        let world_offset = transform.rotation * chunk.center_of_mass;
        let mut new_transform = *transform;
        new_transform.position += world_offset;

        // Give chunks a slight explosive velocity outwards from the center of mass
        let mut new_vel = *vel;
        let outward = chunk.center_of_mass.normalize_or_zero();
        new_vel.linear += outward * 2.0 + impact_direction * 5.0; // Explosion effect

        let chunk_collider = Collider::sphere(radius).with_material(collider.material);
        let mut rb = RigidBody::new(chunk.volume * collider.material.density, 0.0, 0.0, true);
        rb.update_inertia_from_collider(&chunk_collider);

        commands
            .spawn()
            .insert(rb)
            .insert(chunk_collider)
            .insert(new_transform)
            .insert(new_vel);
    }
}

/// System that checks for Explosion components and applies outward forces
/// to all rigid bodies and soft body nodes within the radius.
pub fn physics_explosion_system(world: &World, dt: f32) {
    use crate::components::{Explosion, ExplosionFalloff};
    use gizmo_core::commands::Commands;
    use gizmo_core::system::SystemParam;

    let mut commands = match Commands::fetch(world, dt) {
        Ok(c) => c,
        Err(_) => return,
    };

    let explosion_query_opt = Query::<(
        &Explosion,
        &Transform,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
    )>::new(world);
    let mut active_explosions = Vec::new();

    if let Some(exp_query) = &explosion_query_opt {
        for (ent_id, (explosion, transform, _)) in exp_query.iter() {
            if explosion.is_active {
                // Apply offset to transform position
                active_explosions.push((
                    Entity::new(ent_id, 0),
                    *explosion,
                    transform.position + explosion.offset,
                ));
            }
        }
    }

    if active_explosions.is_empty() {
        return; // Nothing to explode
    }

    let mut shattered = std::collections::HashSet::new();

    // Helper closure to calculate falloff intensity
    let calculate_intensity = |dist: f32, radius: f32, falloff: ExplosionFalloff| -> f32 {
        if dist >= radius {
            return 0.0;
        }
        match falloff {
            ExplosionFalloff::None => 1.0,
            ExplosionFalloff::Linear => 1.0 - (dist / radius),
            ExplosionFalloff::Quadratic => {
                let ratio = 1.0 - (dist / radius);
                ratio * ratio
            }
        }
    };

    // Check for Breakables that should shatter
    let breakable_query_opt = Query::<(
        gizmo_core::query::Mut<crate::components::Breakable>,
        &Transform,
        &Collider,
        &Velocity,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
    )>::new(world);
    if let Some(breakable_query) = &breakable_query_opt {
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (id, (mut breakable, transform, collider, vel, _)) in breakable_query.iter() {
                if breakable.is_broken || shattered.contains(&id) {
                    continue;
                }

                let diff = transform.position - *exp_pos;
                let dist_sq = diff.length_squared();

                if dist_sq < explosion.force_radius * explosion.force_radius && dist_sq > 0.001 {
                    let dist = dist_sq.sqrt();
                    let intensity =
                        calculate_intensity(dist, explosion.force_radius, explosion.falloff);
                    let impulse_mag = explosion.force * intensity;

                    if impulse_mag > breakable.threshold {
                        breakable.current_health -= explosion.damage * intensity;
                        if breakable.current_health <= 0.0 {
                            breakable.is_broken = true;
                            shattered.insert(id);
                            let dir = diff / dist;
                            let mut exp_vel = *vel;
                            exp_vel.linear += dir * impulse_mag * 0.1; // Estimate mass
                            shatter_entity(
                                &mut commands,
                                Entity::new(id, 0),
                                &breakable,
                                transform,
                                collider,
                                &exp_vel,
                                dir,
                                transform.position,
                            );
                        }
                    }
                }
            }
        }
    }

    // Apply to Rigid Bodies
    let rb_query_opt = Query::<(
        Mut<RigidBody>,
        &Transform,
        Mut<Velocity>,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
    )>::new(world);
    if let Some(rb_query) = &rb_query_opt {
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (id, (rb, transform, mut vel, _)) in rb_query.iter() {
                if !rb.is_dynamic() || shattered.contains(&id) {
                    continue;
                }

                let diff = transform.position - *exp_pos;
                let dist_sq = diff.length_squared();

                if dist_sq < explosion.force_radius * explosion.force_radius && dist_sq > 0.001 {
                    let dist = dist_sq.sqrt();
                    let dir = diff / dist;

                    let intensity =
                        calculate_intensity(dist, explosion.force_radius, explosion.falloff);
                    let impulse_mag = explosion.force * intensity;

                    // Apply instantaneous velocity change
                    vel.linear += dir * impulse_mag * rb.inv_mass();
                }
            }
        }
    }

    // Apply to Soft Bodies
    let sb_query_opt = Query::<(
        Mut<crate::soft_body::SoftBodyMesh>,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>,
    )>::new(world);
    if let Some(sb_query) = &sb_query_opt {
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (_id, (mut sb, _)) in sb_query.iter() {
                for node in sb.nodes.iter_mut() {
                    if node.is_fixed {
                        continue;
                    }

                    let diff = node.position - *exp_pos;
                    let dist_sq = diff.length_squared();

                    if dist_sq < explosion.force_radius * explosion.force_radius && dist_sq > 0.001
                    {
                        let dist = dist_sq.sqrt();
                        let dir = diff / dist;

                        let intensity =
                            calculate_intensity(dist, explosion.force_radius, explosion.falloff);
                        let impulse_mag = explosion.force * intensity;

                        let inv_m = if node.mass > 0.0 {
                            1.0 / node.mass
                        } else {
                            0.0
                        };
                        node.velocity += dir * impulse_mag * inv_m;
                    }
                }
            }
        }
    }

    // Despawn the explosions so they don't trigger again
    // Note: If game logic needs to read explosion damage, it must run BEFORE the physics_explosion_system in the schedule!
    for (exp_entity, _, _) in active_explosions {
        commands.entity(exp_entity).despawn();
    }
}

/// Sistem: Fighter Controller'ları günceller ve Input Buffer'a veri yazar.
pub fn physics_fighter_system(world: &gizmo_core::world::World, input: &gizmo_core::input::Input, action_map: &gizmo_core::input::ActionMap) {
    let mut active_fighters = Vec::new();

    if let Some(query) = gizmo_core::query::Query::<(
        gizmo_core::query::Mut<crate::components::fighter::FighterController>,
        gizmo_core::query::Without<gizmo_core::pool::Pooled>
    )>::new(world) {
        let actions = ["Up", "Down", "Left", "Right", "LightPunch", "HeavyPunch", "LightKick", "HeavyKick"];
        
        for (id, (mut fighter, _)) in query.iter() {
            let was_locked = fighter.is_locked();

            if fighter.hitstop_frames > 0 {
                fighter.hitstop_frames -= 1;
            }

            if fighter.hitstun_frames > 0 {
                fighter.hitstun_frames -= 1;
            }

            fighter.input_buffer.update(input, action_map, &actions);

            if !was_locked {
                // --- Saldırı Tetikleme: Tuşa basıldığında yeni hareket başlat ---
                if fighter.active_move.is_none() {
                    use crate::components::fighter::{CombatMove, FrameData};

                    if action_map.is_action_just_pressed(input, "LightPunch") {
                        fighter.active_move = Some(CombatMove {
                            name: "Jab".to_string(),
                            frame_data: FrameData {
                                startup: 5, active: 3, recovery: 8,
                                damage: 8.0, hitstun: 15, hitstop: 3,
                            },
                        });
                        fighter.current_move_frame = 0;
                    } else if action_map.is_action_just_pressed(input, "HeavyPunch") {
                        fighter.active_move = Some(CombatMove {
                            name: "Straight".to_string(),
                            frame_data: FrameData {
                                startup: 10, active: 4, recovery: 15,
                                damage: 18.0, hitstun: 25, hitstop: 6,
                            },
                        });
                        fighter.current_move_frame = 0;
                    } else if action_map.is_action_just_pressed(input, "LightKick") {
                        fighter.active_move = Some(CombatMove {
                            name: "Low Kick".to_string(),
                            frame_data: FrameData {
                                startup: 6, active: 4, recovery: 10,
                                damage: 10.0, hitstun: 18, hitstop: 3,
                            },
                        });
                        fighter.current_move_frame = 0;
                    } else if action_map.is_action_just_pressed(input, "HeavyKick") {
                        fighter.active_move = Some(CombatMove {
                            name: "Roundhouse".to_string(),
                            frame_data: FrameData {
                                startup: 12, active: 5, recovery: 18,
                                damage: 22.0, hitstun: 30, hitstop: 8,
                            },
                        });
                        fighter.current_move_frame = 0;
                    }
                }

                // --- Aktif hareketin kare ilerlemesi ---
                let mut move_ended = false;
                let mut move_duration = 0;
                
                if let Some(move_data) = &fighter.active_move {
                    move_duration = move_data.frame_data.startup + move_data.frame_data.active + move_data.frame_data.recovery;
                }
                
                if move_duration > 0 {
                    fighter.current_move_frame += 1;
                    if fighter.current_move_frame >= move_duration {
                        move_ended = true;
                    }
                } else {
                    fighter.current_move_frame = 0;
                }
                
                if move_ended {
                    fighter.active_move = None;
                    fighter.current_move_frame = 0;
                }
            }
            
            active_fighters.push((id, fighter.is_in_active_window()));
        }
    }
    
    // Hitbox Active durumu senkronizasyonu
    let children_storage = world.borrow::<gizmo_core::component::Children>();
    let mut hitbox_storage = world.borrow_mut::<crate::components::Hitbox>();
    
    for (fighter_id, is_active_window) in active_fighters {
        let mut stack = vec![fighter_id];
        while let Some(current_id) = stack.pop() {
            if let Some(hitbox) = hitbox_storage.get_mut(current_id) {
                hitbox.active = is_active_window;
            }
            if let Some(children) = children_storage.get(current_id) {
                for &child_id in &children.0 {
                    stack.push(child_id);
                }
            }
        }
    }
}


#[cfg(test)]
mod fighter_tests {
    use super::*;
    use gizmo_core::world::World;
    use gizmo_core::input::{Input, ActionMap};
    use crate::components::fighter::{FighterController, CombatMove, FrameData};
    use crate::components::Hitbox;

    #[test]
    fn test_fighter_frame_data_and_hitbox_sync() {
        let mut world = World::new();
        let input = Input::new();
        let action_map = ActionMap::new();

        let parent_id = world.spawn();
        let child_id = world.spawn();

        let move_data = CombatMove {
            name: "Jab".to_string(),
            frame_data: FrameData {
                startup: 5,
                active: 3,
                recovery: 10,
                ..Default::default()
            },
        };

        let mut fighter = FighterController::default();
        fighter.active_move = Some(move_data);
        world.add_component(parent_id, fighter);

        // Child entity with Hitbox
        let hitbox = Hitbox::new(gizmo_math::Vec3::new(1.0, 1.0, 1.0), 10.0);
        world.add_component(child_id, hitbox);
        world.add_component(child_id, gizmo_core::component::Parent(parent_id.id()));
        world.add_component(parent_id, gizmo_core::component::Children(vec![child_id.id()]));

        // Simüle et ve Hitbox'un durumunu test et
        // Frame 1-4 (Startup) -> Hitbox Inactive
        for _ in 0..4 {
            physics_fighter_system(&world, &input, &action_map);
            let h = world.borrow::<Hitbox>().get(child_id.id()).unwrap().clone();
            assert!(!h.active, "Startup framelerinde Hitbox inaktif olmalidir");
        }

        // Frame 5-7 (Active) -> Hitbox Active
        for _ in 0..3 {
            physics_fighter_system(&world, &input, &action_map);
            let h = world.borrow::<Hitbox>().get(child_id.id()).unwrap().clone();
            assert!(h.active, "Active framelerinde Hitbox aktif olmalidir");
        }

        // Frame 8-17 (Recovery) -> Hitbox Inactive
        for _ in 0..10 {
            physics_fighter_system(&world, &input, &action_map);
            let h = world.borrow::<Hitbox>().get(child_id.id()).unwrap().clone();
            assert!(!h.active, "Recovery framelerinde Hitbox tekrar inaktif olmalidir");
        }

        // Frame 18 (End of Move) -> Hitbox Inactive, active_move = None
        physics_fighter_system(&world, &input, &action_map);
        let f = world.borrow::<FighterController>().get(parent_id.id()).unwrap().clone();
        assert!(f.active_move.is_none(), "Hareket bittiginde active_move temizlenmelidir");
    }

    #[test]
    fn test_fighter_hitstop_freezes_animation() {
        let mut world = World::new();
        let input = Input::new();
        let action_map = ActionMap::new();

        let fighter_id = world.spawn();

        let move_data = CombatMove {
            name: "Heavy".to_string(),
            frame_data: FrameData {
                startup: 5,
                active: 5,
                recovery: 5,
                ..Default::default()
            },
        };

        let mut fighter = FighterController::default();
        fighter.active_move = Some(move_data);
        // Hitstop ekle!
        fighter.apply_hitstop(10);
        world.add_component(fighter_id, fighter);

        // 10 frame boyunca hitstop nedeniyle current_move_frame hiç ilerlememeli
        for _ in 0..10 {
            physics_fighter_system(&world, &input, &action_map);
            let f = world.borrow::<FighterController>().get(fighter_id.id()).unwrap().clone();
            assert_eq!(f.current_move_frame, 0, "Hitstop suresince animasyon kareleri donmalidir");
        }

        // Hitstop bitti, simdi ilerlemeye baslamali
        physics_fighter_system(&world, &input, &action_map);
        let f = world.borrow::<FighterController>().get(fighter_id.id()).unwrap().clone();
        assert_eq!(f.current_move_frame, 1, "Hitstop bittiginde animasyon devam etmelidir");
    }
}


/// Hitbox ↔ Hurtbox AABB çarpışma algılama ve hasar uygulama sistemi.
///
/// Her frame'de:
/// 1. Tüm aktif Hitbox'ları dünya pozisyonlarıyla toplar
/// 2. Tüm Hurtbox'ları dünya pozisyonlarıyla toplar
/// 3. AABB overlap testi yapar
/// 4. Aynı entity'ye ait hitbox ↔ hurtbox çarpışmasını engeller (kendi kendine vuruş yok)
/// 5. Çarpışma varsa: hasar uygular, hitstop/hitstun tetikler
///
/// Döndürülen `HitEvent` listesi, UI ve efekt sistemlerinin kullanabilmesi içindir.
#[derive(Debug, Clone)]
pub struct HitEvent {
    /// Vuran entity (Hitbox sahibi veya parent FighterController)
    pub attacker_id: u32,
    /// Vurulan entity (Hurtbox sahibi veya parent FighterController)
    pub victim_id: u32,
    /// Uygulanan hasar
    pub damage: f32,
    /// Dünya uzayında çarpışma noktası (orta nokta yaklaşımı)
    pub hit_point: gizmo_math::Vec3,
}

pub fn hit_detection_system(world: &gizmo_core::world::World) -> Vec<HitEvent> {
    let mut hit_events = Vec::new();

    let transforms = world.borrow::<Transform>();
    let global_transforms = world.borrow::<crate::components::transform::GlobalTransform>();
    let hitboxes = world.borrow::<crate::components::Hitbox>();
    let hurtboxes = world.borrow::<crate::components::Hurtbox>();
    let parents = world.borrow::<gizmo_core::component::Parent>();

    // --- Faz 1: Aktif Hitbox'ları topla (dünya pozisyonu + sahibi) ---
    struct HitboxInfo {
        owner_id: u32,      // FighterController sahibi (root parent)
        world_min: gizmo_math::Vec3,
        world_max: gizmo_math::Vec3,
        damage: f32,
    }

    struct HurtboxInfo {
        owner_id: u32,      // FighterController sahibi (root parent)
        entity_id: u32,
        world_min: gizmo_math::Vec3,
        world_max: gizmo_math::Vec3,
        multiplier: f32,
    }

    // Entity'nin root parent'ını bul (FighterController aranan yol)
    let find_root = |entity_id: u32| -> u32 {
        let mut current = entity_id;
        loop {
            if let Some(parent) = parents.get(current) {
                current = parent.0;
            } else {
                return current;
            }
        }
    };

    let get_world_pos = |entity_id: u32| -> gizmo_math::Vec3 {
        if let Some(gt) = global_transforms.get(entity_id) {
            // GlobalTransform stores a Mat4 — position is in the 4th column
            let m = &gt.matrix;
            gizmo_math::Vec3::new(m.w_axis.x, m.w_axis.y, m.w_axis.z)
        } else if let Some(t) = transforms.get(entity_id) {
            t.position
        } else {
            gizmo_math::Vec3::ZERO
        }
    };

    let mut active_hitboxes = Vec::new();
    for (id, hitbox) in hitboxes.iter() {
        if !hitbox.active {
            continue;
        }
        let world_pos = get_world_pos(id) + hitbox.offset;
        active_hitboxes.push(HitboxInfo {
            owner_id: find_root(id),
            world_min: world_pos - hitbox.half_extents,
            world_max: world_pos + hitbox.half_extents,
            damage: hitbox.damage,
        });
    }

    if active_hitboxes.is_empty() {
        return hit_events;
    }

    // --- Faz 2: Tüm Hurtbox'ları topla ---
    let mut all_hurtboxes = Vec::new();
    for (id, hurtbox) in hurtboxes.iter() {
        let world_pos = get_world_pos(id) + hurtbox.offset;
        all_hurtboxes.push(HurtboxInfo {
            owner_id: find_root(id),
            entity_id: id,
            world_min: world_pos - hurtbox.half_extents,
            world_max: world_pos + hurtbox.half_extents,
            multiplier: hurtbox.damage_multiplier,
        });
    }

    if all_hurtboxes.is_empty() {
        return hit_events;
    }

    // --- Faz 3: AABB Overlap Testi ---
    // Aynı owner'a ait hitbox/hurtbox çarpışmasını engelle (kendi kendine vuruş yok)
    // Aynı saldırı window'unda aynı hedefi birden fazla vurmayı engelle
    let mut already_hit: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();

    for hitbox in &active_hitboxes {
        for hurtbox in &all_hurtboxes {
            // Kendi kendine vuruş engeli
            if hitbox.owner_id == hurtbox.owner_id {
                continue;
            }

            // Aynı (saldırgan, kurban) çiftini tekrar vurma
            let pair = (hitbox.owner_id, hurtbox.owner_id);
            if already_hit.contains(&pair) {
                continue;
            }

            // AABB Overlap
            let overlap = hitbox.world_min.x <= hurtbox.world_max.x
                && hitbox.world_max.x >= hurtbox.world_min.x
                && hitbox.world_min.y <= hurtbox.world_max.y
                && hitbox.world_max.y >= hurtbox.world_min.y
                && hitbox.world_min.z <= hurtbox.world_max.z
                && hitbox.world_max.z >= hurtbox.world_min.z;

            if overlap {
                let final_damage = hitbox.damage * hurtbox.multiplier;
                let hit_point = (hitbox.world_min + hitbox.world_max + hurtbox.world_min + hurtbox.world_max) * 0.25;

                hit_events.push(HitEvent {
                    attacker_id: hitbox.owner_id,
                    victim_id: hurtbox.owner_id,
                    damage: final_damage,
                    hit_point,
                });

                already_hit.insert(pair);
            }
        }
    }

    // Borrow'ları bırak
    drop(transforms);
    drop(global_transforms);
    drop(hitboxes);
    drop(hurtboxes);
    drop(parents);

    // --- Faz 4: Hasar, Hitstop ve Hitstun Uygula ---
    if !hit_events.is_empty() {
        let mut fighters = world.borrow_mut::<crate::components::fighter::FighterController>();

        for event in &hit_events {
            // Kurbanın frame data'sından hitstun/hitstop değerlerini al
            let (hitstun, hitstop) = {
                if let Some(attacker) = fighters.get(event.attacker_id) {
                    if let Some(active_move) = &attacker.active_move {
                        (active_move.frame_data.hitstun, active_move.frame_data.hitstop)
                    } else {
                        (20, 5) // Varsayılan
                    }
                } else {
                    (20, 5)
                }
            };

            // Kurbana hasar + hitstun uygula
            if let Some(victim) = fighters.get_mut(event.victim_id) {
                if !victim.is_blocking {
                    victim.health = (victim.health - event.damage).max(0.0);
                    victim.apply_hitstun(hitstun);
                } else {
                    // Block durumunda yarı hitstun, hasar yok
                    victim.apply_hitstun(hitstun / 3);
                }
            }

            // Her iki tarafa hitstop uygula (dövüş oyunlarındaki o "donma" hissi)
            if let Some(attacker) = fighters.get_mut(event.attacker_id) {
                attacker.apply_hitstop(hitstop);
            }
            if let Some(victim) = fighters.get_mut(event.victim_id) {
                victim.apply_hitstop(hitstop);
            }
        }
    }

    hit_events
}
