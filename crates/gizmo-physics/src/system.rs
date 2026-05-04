use gizmo_core::world::World;
use gizmo_core::query::{Mut, Query};
use crate::world::PhysicsWorld;
use crate::components::{RigidBody, Transform, Velocity, Collider};
use crate::soft_body::SoftBodyMesh;
use gizmo_core::entity::Entity;

/// Exclusive system that updates the entire physics simulation.
/// It reads all rigid and soft bodies from the ECS, steps the physics world,
/// and writes the transformed positions and velocities back to the ECS.
#[tracing::instrument(skip_all, name = "physics_step_system")]
pub fn physics_step_system(world: &World, dt: f32) {
    // 1. Acquire PhysicsWorld Resource
    let mut physics_world = match world.try_get_resource_mut::<PhysicsWorld>() {
        Ok(res) => res,
        Err(e) => {
            println!("[Physics] FAILED TO GET PhysicsWorld Resource: {:?}", e);
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

        for (id, _rb) in rb_storage.iter() {
            if let Some(transform) = trans_storage.get(id) {
                let mut compound_shapes = Vec::new();
                
                // Check self
                if let Some(c) = col_storage.get(id) {
                    compound_shapes.push((crate::components::Transform::default(), Box::new(c.shape.clone())));
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
                                    let local_pos = inv_rot.mul_vec3(child_trans.position - transform.position);
                                    let local_rot = inv_rot * child_trans.rotation;
                                    
                                    let local_t = crate::components::Transform::new(local_pos).with_rotation(local_rot);
                                    compound_shapes.push((local_t, Box::new(child_col.shape.clone())));
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
                    let mut c = Collider::default();
                    c.shape = *s;
                    c
                } else {
                    let mut c = Collider::default();
                    c.shape = crate::components::ColliderShape::Compound(compound_shapes);
                    c
                };
                
                compound_shapes_map.insert(id, final_collider);
            }
        }
    } // Read locks are dropped here!

    // 3. Query Rigid Bodies (Write Locks)
    let mut rigid_bodies = Vec::new();
    let query_opt = Query::<(Mut<RigidBody>, Mut<Transform>, Mut<Velocity>)>::new(world);
    
    if let Some(query) = &query_opt {
        for (id, (rb, transform, vel)) in query.iter() {
            if let Some(final_collider) = compound_shapes_map.remove(&id) {
                rigid_bodies.push((
                    Entity::new(id, 0),
                    rb.clone(),
                    transform.clone(),
                    vel.clone(),
                    final_collider,
                ));
            }
        }
    } else {
        println!("[Physics] FAILED TO BORROW RigidBody/Transform/Velocity Mutably!");
    }

    // 3. Query Soft Bodies
    let mut soft_bodies = Vec::new();
    let soft_query_opt = Query::<(Mut<SoftBodyMesh>, Mut<Transform>)>::new(world);
    if let Some(soft_query) = &soft_query_opt {
        for (id, (soft_mesh, transform)) in soft_query.iter() {
            soft_bodies.push((
                Entity::new(id, 0),
                soft_mesh.clone(),
                transform.clone(),
            ));
        }
    }

    // 3.5. Update Vehicles
    let all_colliders: Vec<(Entity, Transform, Collider)> = rigid_bodies
        .iter()
        .map(|(ent, _, trans, _, col)| (*ent, trans.clone(), col.clone()))
        .collect();

    let is_paused = physics_world.is_paused && !physics_world.step_once && !physics_world.rewind_requested;

    if !is_paused {
        let vehicle_query_opt = Query::<Mut<crate::vehicle::VehicleController>>::new(world);
        if let Some(vehicle_query) = &vehicle_query_opt {
            for (id, mut vehicle) in vehicle_query.iter() {
                if let Some((ent, rb, trans, vel, _col)) = rigid_bodies.iter_mut().find(|(e, ..)| e.id() == id) {
                    crate::vehicle::update_vehicle(*ent, &mut vehicle, rb, trans, vel, &all_colliders, dt);
                }
            }
        }

        // 3.6. Update Character Controllers
        let kcc_query_opt = Query::<Mut<crate::components::CharacterController>>::new(world);
        if let Some(kcc_query) = &kcc_query_opt {
            for (id, mut kcc) in kcc_query.iter() {
                if let Some((ent, _rb, trans, vel, _col)) = rigid_bodies.iter_mut().find(|(e, ..)| e.id() == id) {
                    crate::character::update_character(*ent, &mut kcc, trans, vel, &all_colliders, dt);
                } else if let Some(mut trans) = world.borrow_mut::<Transform>().get_mut(id) {
                    // KCC can also just be attached to a non-rigidbody entity
                    let mut dummy_vel = Velocity::default();
                    crate::character::update_character(Entity::new(id, 0), &mut kcc, &mut trans, &mut dummy_vel, &all_colliders, dt);
                }
            }
        }
    }

    // 4. Step Simulation
    physics_world.clear_bodies();
    for (entity, rb, trans, vel, col) in &rigid_bodies {
        physics_world.add_body(*entity, rb.clone(), trans.clone(), vel.clone(), col.clone());
    }

    physics_world.step(&mut soft_bodies, dt).expect("Gizmo Physics Engine encountered a critical numerical error (NaN, Infinity, or Overflow) and halted!");

    // Sync back to rigid_bodies so vehicles/ECS writeback works
    for i in 0..physics_world.entities.len() {
        let entity_id = physics_world.entities[i].id();
        if let Some((_, _, trans, vel, _)) = rigid_bodies.iter_mut().find(|(e, ..)| e.id() == entity_id) {
            *trans = physics_world.transforms[i];
            *vel = physics_world.velocities[i];
        }
    }

    // 5. Write back to ECS (Rigid Bodies)
    if let Some(_query) = &query_opt {
        let mut trans_storage = world.borrow_mut::<Transform>();
        let mut vel_storage = world.borrow_mut::<Velocity>();
        for (entity, _rb, transform, vel, _collider) in rigid_bodies {
            if let Some(ecs_trans) = trans_storage.get_mut(entity.id()) {
                *ecs_trans = transform;
            }
            if let Some(ecs_vel) = vel_storage.get_mut(entity.id()) {
                *ecs_vel = vel;
            }
        }
    }

    // 6. Write back to ECS (Soft Bodies)
    if let Some(soft_query) = &soft_query_opt {
        for (entity, soft_mesh, transform) in soft_bodies {
            if let Some((mut sm, mut t)) = soft_query.get(entity.id()) {
                *sm = soft_mesh;
                *t = transform;
            }
        }
    }
    // 7. Dispatch Events
    if let Ok(mut trigger_queue) = world.try_get_resource_mut::<gizmo_core::event::Events<crate::collision::TriggerEvent>>() {
        for event in &physics_world.trigger_events {
            trigger_queue.send(event.clone());
        }
    }
    
    if let Ok(mut collision_queue) = world.try_get_resource_mut::<gizmo_core::event::Events<crate::collision::CollisionEvent>>() {
        for event in &physics_world.collision_events {
            collision_queue.send(event.clone());
        }
    }
}

/// System that processes collision events and breaks objects that exceed their threshold.
pub fn physics_fracture_system(world: &World, dt: f32) {
    use gizmo_core::commands::Commands;
    use crate::components::Breakable;
    use gizmo_core::system::SystemParam;

    let physics_world = match world.try_get_resource::<PhysicsWorld>() {
        Ok(res) => res,
        Err(_) => return,
    };

    let mut commands = match Commands::fetch(world, dt) {
        Ok(c) => c,
        Err(_) => return,
    };

    // Keep track of which entities we shattered so we don't shatter them multiple times per frame
    let mut shattered = std::collections::HashSet::new();

    let query_opt = Query::<(&Breakable, &Transform, &Collider, &Velocity)>::new(world);
    let query = match query_opt {
        Some(q) => q,
        None => return,
    };

    for event in &physics_world.collision_events {
        let mut max_impulse = 0.0;
        for contact in &event.contact_points {
            if contact.normal_impulse > max_impulse {
                max_impulse = contact.normal_impulse;
            }
        }

        // Check Entity A
        if !shattered.contains(&event.entity_a.id()) {
            if let Some((breakable, transform, collider, vel)) = query.get(event.entity_a.id()) {
                if !breakable.is_broken && max_impulse > breakable.threshold {
                    shattered.insert(event.entity_a.id());
                    shatter_entity(&mut commands, event.entity_a, breakable, transform, collider, vel);
                }
            }
        }

        // Check Entity B
        if !shattered.contains(&event.entity_b.id()) {
            if let Some((breakable, transform, collider, vel)) = query.get(event.entity_b.id()) {
                if !breakable.is_broken && max_impulse > breakable.threshold {
                    shattered.insert(event.entity_b.id());
                    shatter_entity(&mut commands, event.entity_b, breakable, transform, collider, vel);
                }
            }
        }
    }
}

fn shatter_entity(
    commands: &mut gizmo_core::commands::Commands,
    entity: Entity,
    breakable: &crate::components::Breakable,
    transform: &Transform,
    collider: &Collider,
    vel: &Velocity,
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
        let radius = (chunk.volume * 0.1).powf(0.33).max(0.1); 
        
        // Offset chunk center by parent's transform
        let world_offset = transform.rotation * chunk.center_of_mass;
        let mut new_transform = *transform;
        new_transform.position += world_offset;

        // Give chunks a slight explosive velocity outwards from the center of mass
        let mut new_vel = *vel;
        new_vel.linear += chunk.center_of_mass.normalize_or_zero() * 5.0; // Explosion effect

        let chunk_collider = Collider::sphere(radius).with_material(collider.material);
        let mut rb = RigidBody::new(chunk.volume * collider.material.density, 0.0, 0.0, true);
        rb.update_inertia_from_collider(&chunk_collider);

        commands.spawn()
            .insert(rb)
            .insert(chunk_collider)
            .insert(new_transform)
            .insert(new_vel);
    }
}

/// System that checks for Explosion components and applies outward forces
/// to all rigid bodies and soft body nodes within the radius.
pub fn physics_explosion_system(world: &World, dt: f32) {
    use crate::components::Explosion;
    use gizmo_core::commands::Commands;
    use gizmo_core::system::SystemParam;
    
    let mut commands = match Commands::fetch(world, dt) {
        Ok(c) => c,
        Err(_) => return,
    };
    
    let explosion_query_opt = Query::<(&Explosion, &Transform)>::new(world);
    let mut active_explosions = Vec::new();
    
    if let Some(exp_query) = &explosion_query_opt {
        for (ent_id, (explosion, transform)) in exp_query.iter() {
            if explosion.is_active {
                active_explosions.push((Entity::new(ent_id, 0), explosion.clone(), transform.position));
            }
        }
    }
    
    if active_explosions.is_empty() {
        return; // Nothing to explode
    }

    let mut shattered = std::collections::HashSet::new();

    // Check for Breakables that should shatter
    let breakable_query_opt = Query::<(&crate::components::Breakable, &Transform, &Collider, &Velocity)>::new(world);
    if let Some(breakable_query) = &breakable_query_opt {
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (id, (breakable, transform, collider, vel)) in breakable_query.iter() {
                if breakable.is_broken || shattered.contains(&id) { continue; }
                
                let diff = transform.position - *exp_pos;
                let dist_sq = diff.length_squared();
                
                if dist_sq < explosion.radius * explosion.radius && dist_sq > 0.001 {
                    let dist = dist_sq.sqrt();
                    let intensity = 1.0 - (dist / explosion.radius);
                    let impulse_mag = explosion.force * intensity;
                    
                    if impulse_mag > breakable.threshold {
                        shattered.insert(id);
                        shatter_entity(&mut commands, Entity::new(id, 0), breakable, transform, collider, vel);
                    }
                }
            }
        }
    }

    // Apply to Rigid Bodies
    let rb_query_opt = Query::<(Mut<RigidBody>, &Transform, Mut<Velocity>)>::new(world);
    if let Some(rb_query) = &rb_query_opt {
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (id, (rb, transform, mut vel)) in rb_query.iter() {
                if !rb.is_dynamic() || shattered.contains(&id) { continue; }
                
                let diff = transform.position - *exp_pos;
                let dist_sq = diff.length_squared();
                
                if dist_sq < explosion.radius * explosion.radius && dist_sq > 0.001 {
                    let dist = dist_sq.sqrt();
                    let dir = diff / dist;
                    
                    // Force drops off linearly with distance
                    let intensity = 1.0 - (dist / explosion.radius);
                    let impulse_mag = explosion.force * intensity;
                    
                    // Apply instantaneous velocity change
                    vel.linear += dir * impulse_mag * rb.inv_mass();
                }
            }
        }
    }

    // Apply to Soft Bodies
    let sb_query_opt = Query::<Mut<crate::soft_body::SoftBodyMesh>>::new(world);
    if let Some(sb_query) = &sb_query_opt {
        for (_exp_entity, explosion, exp_pos) in &active_explosions {
            for (_id, mut sb) in sb_query.iter() {
                for node in sb.nodes.iter_mut() {
                    if node.is_fixed { continue; }
                    
                    let diff = node.position - *exp_pos;
                    let dist_sq = diff.length_squared();
                    
                    if dist_sq < explosion.radius * explosion.radius && dist_sq > 0.001 {
                        let dist = dist_sq.sqrt();
                        let dir = diff / dist;
                        
                        let intensity = 1.0 - (dist / explosion.radius);
                        let impulse_mag = explosion.force * intensity;
                        
                        let inv_m = if node.mass > 0.0 { 1.0 / node.mass } else { 0.0 };
                        node.velocity += dir * impulse_mag * inv_m;
                    }
                }
            }
        }
    }
    
    // Despawn the explosions so they don't trigger again
    for (exp_entity, _, _) in active_explosions {
        commands.entity(exp_entity).despawn();
    }
}
