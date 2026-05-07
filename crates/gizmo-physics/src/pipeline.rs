//! Physics Pipeline — `step_internal` alt fonksiyonları
//! `world.rs`'deki monolitik 700+ satırlık fonksiyon buraya bölündü.

use crate::{
    collision::{CollisionEvent, CollisionEventType, ContactManifold, TriggerEvent},
    components::{Transform, Velocity},
    narrowphase::NarrowPhase,
    soft_body::SoftBodyMesh,
    world::{PhysicsWorld, ZoneShape},
};
use gizmo_math::Aabb;
use gizmo_core::entity::Entity;
use rayon::prelude::*;
use std::collections::HashMap;

impl PhysicsWorld {
    /// 0. Yerçekimi alanları, sıvı bölgeleri ve hız entegrasyonu (Paralel)
    #[tracing::instrument(skip_all, name = "velocity_integration")]
    pub(crate) fn velocity_integration_step(&mut self, dt: f32) -> Result<(), crate::error::GizmoError> {
            // 0. Compute per-body gravity and apply fluid buoyancy/drag (Parallel)
            let default_gravity = self.integrator.gravity;
            let gravity_fields = &self.gravity_fields;
            let fluid_zones = &self.fluid_zones;
            let has_error = std::sync::atomic::AtomicBool::new(false);
            let watchlist = &self.watchlist;
            
            self.entities.par_iter()
                .zip(self.rigid_bodies.par_iter_mut())
                .zip(self.transforms.par_iter())
                .zip(self.velocities.par_iter_mut())
                .zip(self.colliders.par_iter())
                .try_for_each(|((((&entity, rb), transform), vel), collider)| -> Result<(), crate::error::GizmoError> {
                if rb.is_sleeping {
                    return Ok(());
                }
                
                let pos = transform.position;
                
                // Resolve gravity
                let mut active_gravity = default_gravity;
                let mut max_priority = i32::MIN;
                
                for field in gravity_fields {
                    if field.shape.contains(pos) {
                        if field.priority > max_priority {
                            active_gravity = field.gravity;
                            max_priority = field.priority;
                        }
                    }
                }
                
                // Apply Fluid Zones
                for zone in fluid_zones {
                    if zone.shape.contains(pos) {
                        let extents_y = collider.extents_y();
                        let surface_y = match zone.shape {
                            ZoneShape::Box { max, .. } => max.y,
                            ZoneShape::Sphere { center, radius } => center.y + radius,
                        };
                        
                        let depth = (surface_y - (pos.y - extents_y)).max(0.0).min(extents_y * 2.0);
                        let submerged_ratio = depth / (extents_y * 2.0).max(0.001);
                        
                        if submerged_ratio > 0.0 {
                            let volume = collider.volume();
                            let submerged_volume = volume * submerged_ratio;
                            
                            let buoyancy_force = -active_gravity * (submerged_volume * zone.density);
                            let speed = vel.linear.length();
                            let dir = if speed > 1e-4 { vel.linear / speed } else { gizmo_math::Vec3::ZERO };
                            let drag_mag = zone.linear_drag * speed + zone.quadratic_drag * speed * speed;
                            let drag_force = -dir * drag_mag * submerged_ratio;
                            
                            let accel = (buoyancy_force + drag_force) * rb.inv_mass();
                            vel.linear += accel * dt;
                        }
                    }
                }
    
                // 1. Integrate velocities (apply forces, gravity, damping)
                let local_integrator = crate::integrator::Integrator { gravity: active_gravity };
                if let Err(e) = local_integrator.integrate_velocities(entity, rb, vel, dt) {
                    tracing::error!("Physics integration error: {:?}. Throwing error upwards.", e);
                    has_error.store(true, std::sync::atomic::Ordering::Relaxed);
                    return Err(e);
                }
                
                // Log Filtering & Watchlist
                if !watchlist.is_empty() && watchlist.contains(&entity) {
                    tracing::debug!("WATCHLIST [{:?}]: Pos: {:.2?}, Vel: {:.2?}", entity, transform.position, vel.linear);
                }
    
                // Critical Value Watcher
                let speed_sq = vel.linear.length_squared();
                if speed_sq > 1_000_000.0 { // Speed > 1000 m/s
                    tracing::warn!("CRITICAL VELOCITY: Entity {:?} is moving at {:.2} m/s! This may cause tunneling or explosion.", entity, speed_sq.sqrt());
                }
    
                Ok(())
            })?;
    
            if has_error.load(std::sync::atomic::Ordering::Relaxed) {
                self.trigger_snapshot("Velocity Integration Error (NaN/Overflow)");
            }
    
        Ok(())
    }

    /// 1.5-1.6 Yumuşak cisim ve sıvı simülasyonu adımı
    #[tracing::instrument(skip_all, name = "soft_body_fluid")]
    pub(crate) fn soft_body_and_fluid_step(
        &mut self,
        soft_bodies: &mut [(Entity, SoftBodyMesh, Transform)],
        _fluid_sims: &mut [(Entity, crate::components::FluidSimulation, Transform)],
        dt: f32,
    ) {
            // 1.5 Step Soft Bodies
            let gravity = self.integrator.gravity;
            let mut rigid_colliders = Vec::with_capacity(self.entities.len());
            for i in 0..self.entities.len() {
                rigid_colliders.push((self.entities[i], self.transforms[i], self.colliders[i].clone()));
            }
            
            #[cfg(feature = "gpu_physics")]
            {
                if let Some(gpu) = &mut self.gpu_compute {
                    gpu.step_soft_bodies(soft_bodies, &rigid_colliders, dt, gravity);
                } else {
                    soft_bodies.par_iter_mut().for_each(|(_, sb, _)| {
                        sb.step(dt, gravity, &rigid_colliders);
                    });
                }
            }
            #[cfg(not(feature = "gpu_physics"))]
            {
                soft_bodies.par_iter_mut().for_each(|(_, sb, _)| {
                    sb.step(dt, gravity, &rigid_colliders);
                });
            }
    
            // 1.6 Step Fluid Simulations
    
    }

    /// 2. Broadphase — uzamsal hash güncelleme ve potansiyel çarpışma çiftleri
    #[tracing::instrument(skip_all, name = "broadphase")]
    pub(crate) fn broadphase_step(
        &mut self,
        soft_bodies: &[(Entity, SoftBodyMesh, Transform)],
        dt: f32,
    ) {
            // 2. Broadphase - update spatial hash and find potential collision pairs
            // Clear the spatial hash for the current sub-step
            self.spatial_hash.clear_mut();
            
            // BVH insert: sequential (&mut self gerektirir — BVH query'si O(log N) olduğu için toplamda hâlâ çok hızlı)
            for i in 0..self.entities.len() {
                let entity = self.entities[i];
                let rb = &self.rigid_bodies[i];
                let transform = &self.transforms[i];
                let vel = &self.velocities[i];
                let collider = &self.colliders[i];
                
                let aabb = collider.compute_aabb(transform.position, transform.rotation);
                let aabb = if rb.ccd_enabled && rb.is_dynamic() && !rb.is_sleeping {
                    // For CCD bodies, sweep the AABB across the full potential travel distance.
                    // Using just vel*dt only covers one sub-step (~4m at 1000m/s).
                    // We use the remaining accumulator time to cover the entire frame's travel.
                    let speed = vel.linear.length();
                    let sweep_dt = if speed > 100.0 {
                        // High-speed: cover at least 1 full frame worth of travel
                        dt.max(1.0 / 60.0)
                    } else {
                        dt
                    };
                    let next_pos  = transform.position + vel.linear * sweep_dt;
                    let next_aabb = collider.compute_aabb(next_pos, transform.rotation);
                    aabb.merge(next_aabb)
                } else {
                    aabb
                };
                self.spatial_hash.insert(entity, aabb);
            }
    
            for (entity, soft_body, _) in soft_bodies.iter() {
                let mut min = gizmo_math::Vec3::splat(f32::MAX);
                let mut max = gizmo_math::Vec3::splat(f32::MIN);
                for node in &soft_body.nodes {
                    min = min.min(node.position);
                    max = max.max(node.position);
                }
                self.spatial_hash.insert(*entity, Aabb { min: min.into(), max: max.into() });
            }
    }

    /// 3. Narrowphase — gerçek çarpışma tespiti, tetikleyiciler, yumuşak cisim çarpışmaları
    #[tracing::instrument(skip_all, name = "narrowphase_collision")]
    pub(crate) fn narrowphase_and_collision_step(
        &mut self,
        soft_bodies: &mut [(Entity, SoftBodyMesh, Transform)],
        dt: f32,
    ) -> Vec<ContactManifold> {
        let entity_map = &self.entity_index_map;
        let mut soft_entity_map = HashMap::new();
        for (i, (entity, _, _)) in soft_bodies.iter().enumerate() {
            soft_entity_map.insert(entity.id(), i);
        }
        let potential_pairs = self.spatial_hash.query_pairs();

            // 3. Narrowphase - detect actual collisions (Parallel)
            let narrowphase_results: Vec<_> = potential_pairs.par_iter().filter_map(|&(entity_a, entity_b)| {
                let is_a_rigid = entity_map.contains_key(&entity_a.id());
                let is_b_rigid = entity_map.contains_key(&entity_b.id());
                
                if is_a_rigid && is_b_rigid {
                    let idx_a = *entity_map.get(&entity_a.id())?;
                    let idx_b = *entity_map.get(&entity_b.id())?;
                    let transform_a = &self.transforms[idx_a];
                    let collider_a = &self.colliders[idx_a];
                    let transform_b = &self.transforms[idx_b];
                    let collider_b = &self.colliders[idx_b];
    
                    // Check collision layers
                    if !collider_a.collision_layer.can_collide_with(&collider_b.collision_layer) {
                        return None;
                    }
    
                    // Narrowphase: 4'e kadar temas noktası üret (Box-Box SAT)
                    let mut contacts = NarrowPhase::test_collision_manifold(
                        &collider_a.shape,
                        transform_a.position,
                        transform_a.rotation,
                        &collider_b.shape,
                        transform_b.position,
                        transform_b.rotation,
                    );
    
                    // CCD - Speculative Contacts
                    if contacts.is_empty() {
                        let rb_a = &self.rigid_bodies[idx_a];
                        let rb_b = &self.rigid_bodies[idx_b];
                        
                        if rb_a.ccd_enabled || rb_b.ccd_enabled {
                            let vel_a = &self.velocities[idx_a];
                            let vel_b = &self.velocities[idx_b];
                            
                            if let Some(speculative_contact) = crate::gjk::Gjk::speculative_contact(
                                &collider_a.shape, transform_a.position, transform_a.rotation, vel_a.linear,
                                &collider_b.shape, transform_b.position, transform_b.rotation, vel_b.linear,
                                dt
                            ) {
                                contacts.push(speculative_contact);
                            }
                        }
                    }
    
                    if !contacts.is_empty() {
                        Some((
                            entity_a,
                            entity_b,
                            contacts,
                            collider_a.is_trigger,
                            collider_b.is_trigger,
                            collider_a.material,
                            collider_b.material,
                            false
                        ))
                    } else {
                        None
                    }
                } else if is_a_rigid != is_b_rigid {
                    // One rigid, one soft
                    let rigid_ent = if is_a_rigid { entity_a } else { entity_b };
                    let soft_ent = if is_a_rigid { entity_b } else { entity_a };
                    
                    // Return a marker so we know to process Soft vs Rigid sequentially
                    Some((
                        rigid_ent,
                        soft_ent,
                        vec![],
                        false, false, // not triggers
                        crate::components::PhysicsMaterial::default(),
                        crate::components::PhysicsMaterial::default(),
                        true // is soft collision
                    ))
                } else {
                    // Soft vs Soft
                    Some((
                        entity_a,
                        entity_b,
                        vec![],
                        false, false,
                        crate::components::PhysicsMaterial::default(),
                        crate::components::PhysicsMaterial::default(),
                        true
                    ))
                }
            }).collect();
    
            let mut manifolds = Vec::new();
            let mut current_contacts = HashMap::new();
            let mut soft_rigid_pairs = Vec::new();
            let mut soft_soft_pairs = Vec::new();
    
            // Sequentially process results to preserve determinism and state
            for (entity_a, entity_b, contact_opt, is_trigger_a, is_trigger_b, mat_a, mat_b, is_soft) in narrowphase_results {
                if is_soft {
                    let is_a_rigid = entity_map.contains_key(&entity_a.id());
                    let is_b_rigid = entity_map.contains_key(&entity_b.id());
                    if is_a_rigid != is_b_rigid {
                        let rigid_ent = if is_a_rigid { entity_a } else { entity_b };
                        let soft_ent = if is_a_rigid { entity_b } else { entity_a };
                        soft_rigid_pairs.push((rigid_ent, soft_ent));
                    } else {
                        soft_soft_pairs.push((entity_a, entity_b));
                    }
                    continue;
                }
                
                let contacts = contact_opt;
                let pair = (entity_a, entity_b);
    
                // Handle triggers
                if is_trigger_a || is_trigger_b {
                    current_contacts.insert(pair, (true, None));
                    
                    let event_type = if self.contact_cache.contains_key(&pair) {
                        CollisionEventType::Persisting
                    } else {
                        CollisionEventType::Started
                    };
                    self.trigger_events.push(TriggerEvent {
                        trigger_entity: if is_trigger_a { entity_a } else { entity_b },
                        other_entity:   if is_trigger_a { entity_b } else { entity_a },
                        event_type,
                    });
                } else {
                    let mut manifold = ContactManifold::new(entity_a, entity_b);
                    manifold.friction        = (mat_a.dynamic_friction * mat_b.dynamic_friction).sqrt();
                    manifold.static_friction = (mat_a.static_friction  * mat_b.static_friction).sqrt();
                    manifold.restitution     = mat_a.restitution.max(mat_b.restitution);
    
                    if let Some(Some((_, Some(old_manifold)))) = self.contact_cache.get(&pair).map(|o| Some(o)) {
                        manifold.lifetime = old_manifold.lifetime + 1;
                        for mut contact in contacts.iter().copied() {
                            for old in &old_manifold.contacts {
                                if (old.point - contact.point).length_squared() < 0.02 * 0.02 {
                                    contact.normal_impulse = old.normal_impulse;
                                    contact.tangent_impulse = old.tangent_impulse;
                                    break;
                                }
                            }
                            manifold.contacts.push(contact);
                        }
                    } else {
                        for contact in contacts.iter().copied() { manifold.contacts.push(contact); }
                    }
    
                    current_contacts.insert(pair, (false, Some(manifold.clone())));
                    manifolds.push(manifold);
    
                    let event_type = if self.contact_cache.contains_key(&pair) {
                        CollisionEventType::Persisting
                    } else {
                        CollisionEventType::Started
                    };
                    self.collision_events.push(CollisionEvent {
                        entity_a,
                        entity_b,
                        event_type,
                        contact_points: contacts.into_iter().take(4).collect(),
                    });
                }
            }
    
            // Detect ended collisions
            for (pair, (is_trigger, _)) in self.contact_cache.iter() {
                if !current_contacts.contains_key(pair) {
                    // Herhangi bir temas bittiğinde, destek (support) kaybolmuş olabilir.
                    // Uçan/havada kalan kutular olmaması için her iki nesneyi de ZORLA uyandır.
                    if let Some(&idx_a) = entity_map.get(&pair.0.id()) {
                        if self.rigid_bodies[idx_a].is_dynamic() {
                            self.rigid_bodies[idx_a].wake_up();
                        }
                    }
                    if let Some(&idx_b) = entity_map.get(&pair.1.id()) {
                        if self.rigid_bodies[idx_b].is_dynamic() {
                            self.rigid_bodies[idx_b].wake_up();
                        }
                    }
    
                    if *is_trigger {
                        self.trigger_events.push(TriggerEvent {
                            trigger_entity: pair.0,
                            other_entity: pair.1,
                            event_type: CollisionEventType::Ended,
                        });
                    } else {
                        self.collision_events.push(CollisionEvent {
                            entity_a: pair.0,
                            entity_b: pair.1,
                            event_type: CollisionEventType::Ended,
                            contact_points: arrayvec::ArrayVec::new(),
                        });
                    }
                }
            }
    
            self.contact_cache = current_contacts;
    
            // 3.5 Process Soft vs Rigid collisions
            let node_shape = crate::components::ColliderShape::Sphere(crate::components::SphereShape { radius: 0.1 });
            for (rigid_ent, soft_ent) in soft_rigid_pairs {
                if let (Some(&rigid_idx), Some(&soft_idx)) = (
                    entity_map.get(&rigid_ent.id()),
                    soft_entity_map.get(&soft_ent.id())
                ) {
                    // Cannot borrow mutably from self multiple times easily, but we can do it because fields are separate.
                    let rigid_rb = &self.rigid_bodies[rigid_idx];
                    let rigid_trans = &self.transforms[rigid_idx];
                    let rigid_collider = &self.colliders[rigid_idx];
                    let mut rigid_vel = self.velocities[rigid_idx]; // Copy out
                
                let (_, soft_body, _) = &mut soft_bodies[soft_idx];
                let mut changed = false;
                
                for node in soft_body.nodes.iter_mut() {
                    if let Some(contact) = NarrowPhase::test_collision(
                        &node_shape,
                        node.position,
                        gizmo_math::Quat::IDENTITY,
                        &rigid_collider.shape,
                        rigid_trans.position,
                        rigid_trans.rotation,
                    ) {
                        // NarrowPhase::test_collision returns normal pointing from shape_a (node)
                        // to shape_b (rigid). For correct impulse, we need the normal pointing
                        // from rigid toward node (separating direction for node).
                        let normal = -contact.normal;
                        let penetration = contact.penetration;
                        
                        let inv_m_node = 1.0 / node.mass;
                        let inv_m_rb = rigid_rb.inv_mass();
                        let total_inv_m = inv_m_node + inv_m_rb;
                        
                        let r_rb = contact.point - rigid_trans.position;
                        let v_node = node.velocity;
                        let v_rb = rigid_vel.linear + rigid_vel.angular.cross(r_rb);
                        
                        // rel_vel: velocity of node relative to rigid body
                        let rel_vel = v_node - v_rb;
                        let vel_norm = rel_vel.dot(normal);
                        
                        // vel_norm < 0 means node is approaching rigid along the normal
                        if vel_norm < 0.0 {
                            let j = -(1.0 + 0.2) * vel_norm / total_inv_m;
                            let impulse = normal * j;
                            
                            // Push node away from rigid
                            node.velocity += impulse * inv_m_node;
                            if rigid_rb.is_dynamic() {
                                // Push rigid away from node
                                rigid_vel.linear -= impulse * inv_m_rb;
                                changed = true;
                            }
                        }
                        
                        // Positional correction: push node out along normal
                        let pos_correction = normal * (penetration * 0.5);
                        node.position += pos_correction * (inv_m_node / total_inv_m);
                    }
                }
                if changed {
                    self.velocities[rigid_idx] = rigid_vel; // Write back
                }
            }
            }
    
            // 3.6 Process Soft vs Soft collisions
            // Treating each node as a small sphere (radius 0.1) for node-to-node penalty forces.
            let node_radius = 0.1;
            let node_diameter_sq = (node_radius * 2.0) * (node_radius * 2.0);
            let penalty_stiffness = 5000.0;
            let penalty_damping = 50.0;
    
            for (soft_ent_a, soft_ent_b) in soft_soft_pairs {
                if let (Some(&idx_a), Some(&idx_b)) = (
                    soft_entity_map.get(&soft_ent_a.id()),
                    soft_entity_map.get(&soft_ent_b.id())
                ) {
                    // To mutate two different soft bodies in the same array, we must use split_at_mut
                    // to bypass the borrow checker safely, since we know idx_a != idx_b.
                if idx_a == idx_b { continue; }
                let (sb_a, sb_b) = if idx_a < idx_b {
                    let (left, right) = soft_bodies.split_at_mut(idx_b);
                    (&mut left[idx_a].1, &mut right[0].1)
                } else {
                    let (left, right) = soft_bodies.split_at_mut(idx_a);
                    (&mut right[0].1, &mut left[idx_b].1)
                };
    
                for node_a in sb_a.nodes.iter_mut() {
                    for node_b in sb_b.nodes.iter_mut() {
                        let diff = node_a.position - node_b.position;
                        let dist_sq = diff.length_squared();
                        if dist_sq < node_diameter_sq && dist_sq > 1e-6 {
                            let dist = dist_sq.sqrt();
                            let normal = diff / dist; // points from B to A
                            let penetration = (node_radius * 2.0) - dist;
                            
                            let rel_vel = node_a.velocity - node_b.velocity;
                            let vel_along_normal = rel_vel.dot(normal);
                            
                            // Spring-damper penalty force
                            let force_mag = penetration * penalty_stiffness - vel_along_normal * penalty_damping;
                            if force_mag > 0.0 {
                                let inv_m_a = if node_a.mass > 0.0 && !node_a.is_fixed { 1.0 / node_a.mass } else { 0.0 };
                                let inv_m_b = if node_b.mass > 0.0 && !node_b.is_fixed { 1.0 / node_b.mass } else { 0.0 };
                                let total_inv_m = inv_m_a + inv_m_b;
                                if total_inv_m > 1e-8 {
                                    // Standard impulse: equal and opposite, scaled by each body's inv_mass
                                    let impulse = normal * (force_mag * dt);
                                    node_a.velocity += impulse * inv_m_a;
                                    node_b.velocity -= impulse * inv_m_b;
                                    
                                    // Position correction weighted by mass ratio
                                    let pos_corr = normal * (penetration * 0.5);
                                    node_a.position += pos_corr * (inv_m_a / total_inv_m);
                                    node_b.position -= pos_corr * (inv_m_b / total_inv_m);
                                }
                            }
                        }
                    }
                }
                }
            }
    
        manifolds
    }

    /// 4-4.5 Kısıt çözücü (çarpışma + eklem)
    #[tracing::instrument(skip_all, name = "constraint_solve")]
    pub(crate) fn constraint_solve_step(
        &mut self,
        manifolds: Vec<ContactManifold>,
        dt: f32,
    ) {
            let entity_map = &self.entity_index_map;

            // 4. Solve constraints (only for non-trigger collisions)
            if !manifolds.is_empty() {
                let is_dynamic = |entity: Entity| -> bool {
                    if let Some(&idx) = entity_map.get(&entity.id()) {
                        self.rigid_bodies[idx].is_dynamic()
                    } else {
                        false
                    }
                };
                
                let islands = crate::island::IslandManager::build_islands(&manifolds, &is_dynamic);
                let island_manifold_groups = crate::island::IslandManager::split_manifolds(manifolds, &islands);
    
                let rigid_bodies = &self.rigid_bodies;
                let transforms = &self.transforms;
                let velocities = &self.velocities;
                let solver = &self.solver;
                let entities_arr = &self.entities;
    
                let results: Vec<(Vec<(Entity, crate::components::Velocity)>, Vec<ContactManifold>, Vec<Entity>, Vec<crate::collision::FractureEvent>)> = island_manifold_groups.into_par_iter().map(|mut island_manifolds| {
                    let mut island_is_sleeping = true;
                    for manifold in island_manifolds.iter() {
                        if let (Some(&idx_a), Some(&idx_b)) = (
                            entity_map.get(&manifold.entity_a.id()),
                            entity_map.get(&manifold.entity_b.id())
                        ) {
                            if rigid_bodies[idx_a].is_dynamic() && !rigid_bodies[idx_a].is_sleeping {
                                island_is_sleeping = false;
                            }
                            if rigid_bodies[idx_b].is_dynamic() && !rigid_bodies[idx_b].is_sleeping {
                                island_is_sleeping = false;
                            }
                        }
                    }
    
                    let mut wake_updates = Vec::new();
    
                    if !island_is_sleeping {
                        let mut island_indices = std::collections::HashSet::new();
    
                        for manifold in island_manifolds.iter() {
                            if let (Some(&idx_a), Some(&idx_b)) = (
                                entity_map.get(&manifold.entity_a.id()),
                                entity_map.get(&manifold.entity_b.id())
                            ) {
                                island_indices.insert(idx_a);
                                island_indices.insert(idx_b);
                                
                                // If island is awake, wake up any sleeping bodies in it
                                if rigid_bodies[idx_a].is_dynamic() && rigid_bodies[idx_a].is_sleeping {
                                    wake_updates.push(manifold.entity_a);
                                }
                                if rigid_bodies[idx_b].is_dynamic() && rigid_bodies[idx_b].is_sleeping {
                                    wake_updates.push(manifold.entity_b);
                                }
                            }
                        }
    
                        // Thread-local velocity cache to prevent extreme allocations per island
                        thread_local! {
                            static VEL_CACHE: std::cell::RefCell<Vec<Velocity>> = std::cell::RefCell::new(Vec::new());
                        }
                        
                        let mut velocity_updates = Vec::with_capacity(island_indices.len());
                        
                        VEL_CACHE.with(|cache| {
                            let mut local_velocities = cache.borrow_mut();
                            if local_velocities.len() < velocities.len() {
                                local_velocities.resize(velocities.len(), Velocity::default());
                            }
                            
                            // Copy only active indices
                            for &idx in &island_indices {
                                local_velocities[idx] = velocities[idx];
                            }
                            
                            solver.solve_contacts(&mut island_manifolds, rigid_bodies, transforms, &mut local_velocities, entity_map, dt);
                            
                            for &idx in &island_indices {
                                if rigid_bodies[idx].is_dynamic() {
                                    velocity_updates.push((entities_arr[idx], local_velocities[idx]));
                                }
                            }
                        });
    
                        let mut local_fractures = Vec::new();
                        for manifold in island_manifolds.iter() {
                            if let (Some(&idx_a), Some(&idx_b)) = (
                                entity_map.get(&manifold.entity_a.id()),
                                entity_map.get(&manifold.entity_b.id())
                            ) {
                                // Check for fractures
                                let mut max_impulse = 0.0;
                                let mut impact_point = gizmo_math::Vec3::ZERO;
                                for contact in &manifold.contacts {
                                    if contact.normal_impulse > max_impulse {
                                        max_impulse = contact.normal_impulse;
                                        impact_point = contact.point;
                                    }
                                }
    
                                if let Some(threshold_a) = rigid_bodies[idx_a].fracture_threshold {
                                    if max_impulse > threshold_a {
                                        local_fractures.push(crate::collision::FractureEvent {
                                            entity: manifold.entity_a,
                                            impact_point,
                                            impact_force: max_impulse,
                                        });
                                    }
                                }
                                if let Some(threshold_b) = rigid_bodies[idx_b].fracture_threshold {
                                    if max_impulse > threshold_b {
                                        local_fractures.push(crate::collision::FractureEvent {
                                            entity: manifold.entity_b,
                                            impact_point,
                                            impact_force: max_impulse,
                                        });
                                    }
                                }
                            }
                        }
                        
                        (velocity_updates, island_manifolds, wake_updates, local_fractures)
                    } else {
                        // Island is completely asleep. Skip the solver!
                        (Vec::new(), island_manifolds, wake_updates, Vec::new())
                    }
                }).collect();
    
                // Write back velocities and wake ups
                for (island_vels, _, wake_ups, local_fractures) in &results {
                    for &(entity, ref vel) in island_vels {
                        if let Some(&idx) = entity_map.get(&entity.id()) {
                            self.velocities[idx] = *vel;
                        }
                    }
                    for &entity in wake_ups {
                        if let Some(&idx) = entity_map.get(&entity.id()) {
                            self.rigid_bodies[idx].wake_up();
                        }
                    }
                    self.fracture_events.extend_from_slice(local_fractures);
                }
                
                // Update collision events with resolved impulses
                for (_, island_manifolds, _, _) in results {
                    for manifold in island_manifolds {
                        let solved_contacts: arrayvec::ArrayVec<_, 4> = manifold.contacts.into_iter().take(4).collect();
                        // Update ALL matching events (sub-stepping creates multiple events per pair)
                        for event in self.collision_events.iter_mut() {
                            if (event.entity_a == manifold.entity_a && event.entity_b == manifold.entity_b) ||
                               (event.entity_a == manifold.entity_b && event.entity_b == manifold.entity_a)
                            {
                                event.contact_points = solved_contacts.clone();
                            }
                        }
                    }
                }
            }
    
            // 4.5 Solve explicit joints (Hinges, Springs, etc.)
            if !self.joints.is_empty() {
                self.joint_solver.solve_joints(
                    &mut self.joints, 
                    &self.entity_index_map, 
                    &self.rigid_bodies, 
                    &self.transforms, 
                    &mut self.velocities, 
                    dt
                );
            }
    
    }

    /// 5-6 Pozisyon entegrasyonu ve uyku durumu güncellemesi
    #[tracing::instrument(skip_all, name = "position_integration")]
    pub(crate) fn position_integration_step(&mut self, dt: f32) -> Result<(), crate::error::GizmoError> {
            // 5. Integrate positions (Parallel)
            let pos_error = std::sync::atomic::AtomicBool::new(false);
            let pos_integrator = &self.integrator;
            self.entities.par_iter()
                .zip(self.rigid_bodies.par_iter_mut())
                .zip(self.transforms.par_iter_mut())
                .zip(self.velocities.par_iter_mut())
                .try_for_each(|(((&entity, rb), transform), vel)| -> Result<(), crate::error::GizmoError> {
                    if rb.is_sleeping {
                        return Ok(());
                    }
                    
                    rb.enforce_locks(vel);
                    if let Err(e) = pos_integrator.integrate_positions(entity, rb, transform, vel, dt) {
                        tracing::error!("Physics position integration error: {:?}. Throwing error upwards.", e);
                        pos_error.store(true, std::sync::atomic::Ordering::Relaxed);
                        return Err(e);
                    }
                    Ok(())
                })?;
    
            if pos_error.load(std::sync::atomic::Ordering::Relaxed) {
                self.trigger_snapshot("Position Integration Error (NaN/Overflow)");
            }
    

        Ok(())
    }
}
