//! Physics pipeline — internal sub-steps of `PhysicsWorld::step`.
//!
//! The original monolithic 700-line `step_internal` has been split into
//! focused, individually-traceable functions.  Each function is responsible
//! for exactly one pipeline stage and returns a typed result so the caller
//! can handle errors at the right boundary.

use crate::{
    collision::{CollisionEvent, CollisionEventType, ContactManifold, TriggerEvent},
    components::{Transform, Velocity},
    narrowphase::NarrowPhase,
    soft_body::SoftBodyMesh,
    world::{PhysicsWorld, ZoneShape},
};
use gizmo_core::entity::Entity;
use gizmo_math::Aabb;
use rayon::prelude::*;
use std::collections::HashMap;

impl PhysicsWorld {
    // ================================================================== //
    //  Stage 0 — Velocity integration                                     //
    // ================================================================== //

    /// Apply gravity fields, fluid buoyancy/drag, and integrate velocities
    /// for every non-sleeping rigid body (parallel).
    #[tracing::instrument(skip_all, name = "velocity_integration")]
    pub(crate) fn velocity_integration_step(
        &mut self,
        dt: f32,
    ) -> Result<(), crate::error::GizmoError> {
        let default_gravity = self.integrator.gravity;
        let gravity_fields = &self.gravity_fields;
        let fluid_zones = &self.fluid_zones;
        let watchlist = &self.watchlist;

        // Parallel iteration over all entities.
        // `try_for_each` short-circuits on the first `Err` and propagates it.
        self.entities
            .par_iter()
            .zip(self.rigid_bodies.par_iter_mut())
            .zip(self.transforms.par_iter())
            .zip(self.velocities.par_iter_mut())
            .zip(self.colliders.par_iter())
            .try_for_each(
                |((((&entity, rb), transform), vel), collider)| -> Result<(), crate::error::GizmoError> {
                    if rb.is_sleeping {
                        return Ok(());
                    }

                    let pos = transform.position;

                    // ── Gravity field resolution (highest priority wins) ──
                    let active_gravity = gravity_fields
                        .iter()
                        .filter(|f| f.shape.contains(pos))
                        .max_by_key(|f| f.priority)
                        .map_or(default_gravity, |f| f.gravity);

                    // ── Fluid buoyancy & drag ─────────────────────────────
                    for zone in fluid_zones {
                        if !zone.shape.contains(pos) {
                            continue;
                        }

                        let extents_y = collider.extents_y();
                        let surface_y = match zone.shape {
                            ZoneShape::Box { max, .. }         => max.y,
                            ZoneShape::Sphere { center, radius } => center.y + radius,
                        };

                        let submerged_depth =
                            (surface_y - (pos.y - extents_y)).clamp(0.0, extents_y * 2.0);
                        let denom = (extents_y * 2.0).max(f32::EPSILON);
                        let submerged_ratio = submerged_depth / denom;

                        if submerged_ratio > 0.0 {
                            let submerged_volume = collider.volume() * submerged_ratio;
                            let buoyancy_force   = -active_gravity * (submerged_volume * zone.density);

                            let speed = vel.linear.length();
                            let drag_dir = if speed > 1e-4 {
                                vel.linear / speed
                            } else {
                                gizmo_math::Vec3::ZERO
                            };
                            let drag_mag = zone.linear_drag    * speed
                                         + zone.quadratic_drag * speed * speed;
                            let drag_force = -drag_dir * drag_mag * submerged_ratio;

                            vel.linear +=
                                (buoyancy_force + drag_force) * rb.inv_mass() * dt;
                        }
                    }

                    // ── Velocity integration ──────────────────────────────
                    let local_integrator =
                        crate::integrator::Integrator { gravity: active_gravity };

                    local_integrator.integrate_velocities(entity, rb, vel, dt)?;

                    // ── Watchlist debug logging ───────────────────────────
                    if !watchlist.is_empty() && watchlist.contains(&entity) {
                        tracing::debug!(
                            "WATCHLIST [{:?}]: pos={:.3?}  lin_vel={:.3?}",
                            entity, transform.position, vel.linear,
                        );
                    }

                    Ok(())
                },
            )
            .inspect_err(|e| {
                tracing::error!("Velocity integration failed: {e:?}");
                self.trigger_snapshot("Velocity Integration Error (NaN/Overflow)");
            })
    }

    // ================================================================== //
    //  Stage 1.5–1.6 — Soft-body & fluid simulation                      //
    // ================================================================== //

    #[tracing::instrument(skip_all, name = "soft_body_fluid")]
    pub(crate) fn soft_body_and_fluid_step(
        &mut self,
        soft_bodies: &mut [(Entity, SoftBodyMesh, Transform)],
        _fluid_sims: &mut [(Entity, crate::components::FluidSimulation, Transform)],
        dt: f32,
    ) {
        let gravity = self.integrator.gravity;

        // Build a lightweight snapshot of rigid colliders for soft-body queries.
        let rigid_colliders: Vec<_> = self
            .entities
            .iter()
            .zip(&self.transforms)
            .zip(&self.colliders)
            .map(|((&entity, transform), collider)| (entity, *transform, collider.clone()))
            .collect();

        // ── Soft bodies ───────────────────────────────────────────────────
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

        // ── Fluid simulations (placeholder) ──────────────────────────────
        // TODO: step fluid sims here.
    }

    // ================================================================== //
    //  Stage 2 — Broadphase                                               //
    // ================================================================== //

    /// Rebuild the spatial hash with swept AABBs and insert soft-body bounds.
    #[tracing::instrument(skip_all, name = "broadphase")]
    pub(crate) fn broadphase_step(
        &mut self,
        soft_bodies: &[(Entity, SoftBodyMesh, Transform)],
        dt: f32,
    ) {
        self.spatial_hash.clear();

        // Rigid bodies — sequential because `spatial_hash` needs `&mut self`.
        for i in 0..self.entities.len() {
            let entity = self.entities[i];
            let rb = &self.rigid_bodies[i];
            let transform = &self.transforms[i];
            let vel = &self.velocities[i];
            let collider = &self.colliders[i];

            let base_aabb = collider.compute_aabb(transform.position, transform.rotation);

            // For CCD bodies sweep the AABB over the full expected travel this frame.
            let aabb = if rb.ccd_enabled && rb.is_dynamic() && !rb.is_sleeping {
                let sweep_dt = if vel.linear.length() > 100.0 {
                    // High-speed: guarantee at least one full 60 Hz frame is covered.
                    dt.max(1.0 / 60.0)
                } else {
                    dt
                };
                let swept_pos = transform.position + vel.linear * sweep_dt;
                let swept_aabb = collider.compute_aabb(swept_pos, transform.rotation);
                base_aabb.merge(swept_aabb)
            } else {
                base_aabb
            };

            self.spatial_hash.insert(entity, aabb);
        }

        // Soft bodies — insert bounding AABB over all nodes.
        for (entity, soft_body, _) in soft_bodies {
            let (min, max) = soft_body.nodes.iter().fold(
                (
                    gizmo_math::Vec3::splat(f32::MAX),
                    gizmo_math::Vec3::splat(f32::MIN),
                ),
                |(mn, mx), node| (mn.min(node.position), mx.max(node.position)),
            );
            self.spatial_hash.insert(
                *entity,
                Aabb {
                    min: min.into(),
                    max: max.into(),
                },
            );
        }
    }

    // ================================================================== //
    //  Stage 3 — Narrowphase & collision events                           //
    // ================================================================== //

    /// Detect actual collisions (parallel narrowphase), emit collision /
    /// trigger events, and handle soft-body contacts (sequential).
    ///
    /// Returns the set of [`ContactManifold`]s to be fed into the solver.
    #[tracing::instrument(skip_all, name = "narrowphase_collision")]
    pub(crate) fn narrowphase_and_collision_step(
        &mut self,
        soft_bodies: &mut [(Entity, SoftBodyMesh, Transform)],
        dt: f32,
    ) -> Vec<ContactManifold> {
        let entity_map = &self.entity_index_map;
        let soft_entity_map: HashMap<u32, usize> = soft_bodies
            .iter()
            .enumerate()
            .map(|(i, (e, _, _))| (e.id(), i))
            .collect();

        let potential_pairs = self.spatial_hash.query_pairs();

        // ── Parallel narrowphase ──────────────────────────────────────────
        // Each entry: (entity_a, entity_b, contacts, is_trigger_a, is_trigger_b,
        //              mat_a, mat_b, is_soft_pair)
        type NpResult = (
            Entity,
            Entity,
            Vec<crate::collision::ContactPoint>,
            bool,
            bool,
            crate::components::PhysicsMaterial,
            crate::components::PhysicsMaterial,
            bool, // is_soft_pair
        );

        let default_material = crate::components::PhysicsMaterial::default();

        let narrowphase_results: Vec<NpResult> = potential_pairs
            .par_iter()
            .filter_map(|&(entity_a, entity_b)| {
                let is_a_rigid = entity_map.contains_key(&entity_a.id());
                let is_b_rigid = entity_map.contains_key(&entity_b.id());

                match (is_a_rigid, is_b_rigid) {
                    // ── Rigid vs Rigid ────────────────────────────────────
                    (true, true) => {
                        let idx_a = *entity_map.get(&entity_a.id())?;
                        let idx_b = *entity_map.get(&entity_b.id())?;

                        let collider_a = &self.colliders[idx_a];
                        let collider_b = &self.colliders[idx_b];

                        if !collider_a
                            .collision_layer
                            .can_collide_with(&collider_b.collision_layer)
                        {
                            return None;
                        }

                        let transform_a = &self.transforms[idx_a];
                        let transform_b = &self.transforms[idx_b];

                        let mut contacts = NarrowPhase::test_collision_manifold(
                            &collider_a.shape,
                            transform_a.position,
                            transform_a.rotation,
                            &collider_b.shape,
                            transform_b.position,
                            transform_b.rotation,
                        );

                        // Speculative contacts for CCD bodies.
                        if contacts.is_empty() {
                            let rb_a = &self.rigid_bodies[idx_a];
                            let rb_b = &self.rigid_bodies[idx_b];

                            if rb_a.ccd_enabled || rb_b.ccd_enabled {
                                let vel_a = &self.velocities[idx_a];
                                let vel_b = &self.velocities[idx_b];

                                if let Some(sc) = crate::gjk::Gjk::speculative_contact(
                                    &collider_a.shape,
                                    transform_a.position,
                                    transform_a.rotation,
                                    vel_a.linear,
                                    &collider_b.shape,
                                    transform_b.position,
                                    transform_b.rotation,
                                    vel_b.linear,
                                    dt,
                                ) {
                                    contacts.push(sc);
                                }
                            }
                        }

                        if contacts.is_empty() {
                            return None;
                        }

                        Some((
                            entity_a,
                            entity_b,
                            contacts,
                            collider_a.is_trigger,
                            collider_b.is_trigger,
                            collider_a.material,
                            collider_b.material,
                            false,
                        ))
                    }

                    // ── Mixed rigid / soft ─────────────────────────────────
                    // Return a marker; the actual work happens sequentially below.
                    (true, false) | (false, true) => Some((
                        entity_a,
                        entity_b,
                        Vec::new(),
                        false,
                        false,
                        default_material,
                        default_material,
                        true,
                    )),

                    // ── Soft vs Soft ──────────────────────────────────────
                    (false, false) => Some((
                        entity_a,
                        entity_b,
                        Vec::new(),
                        false,
                        false,
                        default_material,
                        default_material,
                        true,
                    )),
                }
            })
            .collect();

        // ── Sequential post-processing ────────────────────────────────────
        let mut manifolds = Vec::new();
        let mut current_cache = HashMap::new();
        let mut soft_rigid_pairs = Vec::new();
        let mut soft_soft_pairs = Vec::new();

        for (entity_a, entity_b, contacts, is_trigger_a, is_trigger_b, mat_a, mat_b, is_soft) in
            narrowphase_results
        {
            if is_soft {
                let is_a_rigid = entity_map.contains_key(&entity_a.id());
                let is_b_rigid = entity_map.contains_key(&entity_b.id());

                match (is_a_rigid, is_b_rigid) {
                    (true, false) => soft_rigid_pairs.push((entity_a, entity_b)),
                    (false, true) => soft_rigid_pairs.push((entity_b, entity_a)),
                    _ => soft_soft_pairs.push((entity_a, entity_b)),
                }
                continue;
            }

            let pair = (entity_a, entity_b);

            if is_trigger_a || is_trigger_b {
                // ── Trigger ───────────────────────────────────────────────
                current_cache.insert(pair, (true, None));

                let event_type = if self.contact_cache.contains_key(&pair) {
                    CollisionEventType::Persisting
                } else {
                    CollisionEventType::Started
                };
                self.trigger_events.push(TriggerEvent {
                    trigger_entity: if is_trigger_a { entity_a } else { entity_b },
                    other_entity: if is_trigger_a { entity_b } else { entity_a },
                    event_type,
                });
            } else {
                // ── Solid contact ─────────────────────────────────────────
                let mut manifold = ContactManifold::new(entity_a, entity_b);
                manifold.friction = (mat_a.dynamic_friction * mat_b.dynamic_friction).sqrt();
                manifold.static_friction = (mat_a.static_friction * mat_b.static_friction).sqrt();
                manifold.restitution = mat_a.restitution.max(mat_b.restitution);

                // Warm-start: reuse impulses from the previous frame's manifold.
                if let Some((_, Some(old_manifold))) = self.contact_cache.get(&pair) {
                    manifold.lifetime = old_manifold.lifetime + 1;
                    for mut contact in contacts.iter().copied() {
                        if let Some(old) = old_manifold
                            .contacts
                            .iter()
                            .find(|o| (o.point - contact.point).length_squared() < 0.02 * 0.02)
                        {
                            contact.normal_impulse = old.normal_impulse;
                            contact.tangent_impulse = old.tangent_impulse;
                        }
                        manifold.contacts.push(contact);
                    }
                } else {
                    manifold.contacts.extend(contacts.iter().copied());
                }

                current_cache.insert(pair, (false, Some(manifold.clone())));

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

                manifolds.push(manifold);
            }
        }

        // ── Ended collisions ──────────────────────────────────────────────
        for (pair, (is_trigger, _)) in &self.contact_cache {
            if current_cache.contains_key(pair) {
                continue;
            }

            // Wake both bodies so they aren't frozen mid-air after losing support.
            for entity in [pair.0, pair.1] {
                if let Some(&idx) = entity_map.get(&entity.id()) {
                    if self.rigid_bodies[idx].is_dynamic() {
                        self.rigid_bodies[idx].wake_up();
                    }
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

        self.contact_cache = current_cache;

        // ── Stage 3.5: Soft vs Rigid contacts (sequential) ───────────────
        let node_shape = crate::components::ColliderShape::Sphere(crate::components::SphereShape {
            radius: 0.1,
        });

        for (rigid_ent, soft_ent) in soft_rigid_pairs {
            let (Some(&rigid_idx), Some(&soft_idx)) = (
                entity_map.get(&rigid_ent.id()),
                soft_entity_map.get(&soft_ent.id()),
            ) else {
                continue;
            };

            // Copy out the mutable data we need from self before the loop.
            let rigid_transform = self.transforms[rigid_idx];
            let rigid_collider = self.colliders[rigid_idx].clone();
            let rigid_rb = &self.rigid_bodies[rigid_idx];
            let is_rigid_dynamic = rigid_rb.is_dynamic();
            let inv_m_rb = rigid_rb.inv_mass();

            let mut rigid_vel = self.velocities[rigid_idx];
            let mut vel_changed = false;

            let (_, soft_body, _) = &mut soft_bodies[soft_idx];

            for node in &mut soft_body.nodes {
                let Some(contact) = NarrowPhase::test_collision(
                    &node_shape,
                    node.position,
                    gizmo_math::Quat::IDENTITY,
                    &rigid_collider.shape,
                    rigid_transform.position,
                    rigid_transform.rotation,
                ) else {
                    continue;
                };

                // Normal points from the rigid body toward the node (separating direction).
                let normal = -contact.normal;
                let penetration = contact.penetration;

                let inv_m_node = 1.0 / node.mass;
                let total_inv_m = inv_m_node + inv_m_rb;

                let r_rb = contact.point - rigid_transform.position;
                let v_rb = rigid_vel.linear + rigid_vel.angular.cross(r_rb);
                let rel_vel = node.velocity - v_rb;
                let vel_along_normal = rel_vel.dot(normal);

                // Only resolve if approaching.
                if vel_along_normal < 0.0 {
                    let restitution = 0.2;
                    let j = -(1.0 + restitution) * vel_along_normal / total_inv_m;
                    let impulse = normal * j;

                    node.velocity += impulse * inv_m_node;

                    if is_rigid_dynamic {
                        rigid_vel.linear -= impulse * inv_m_rb;
                        vel_changed = true;
                    }
                }

                // Positional correction — split proportionally by inverse mass.
                let correction = normal * (penetration * 0.5);
                node.position += correction * (inv_m_node / total_inv_m);
            }

            if vel_changed {
                self.velocities[rigid_idx] = rigid_vel;
            }
        }

        // ── Stage 3.6: Soft vs Soft contacts (sequential) ────────────────
        const NODE_RADIUS: f32 = 0.1;
        const PENALTY_STIFFNESS: f32 = 5_000.0;
        const PENALTY_DAMPING: f32 = 50.0;

        let node_diam_sq = (NODE_RADIUS * 2.0) * (NODE_RADIUS * 2.0);

        for (soft_ent_a, soft_ent_b) in soft_soft_pairs {
            let (Some(&idx_a), Some(&idx_b)) = (
                soft_entity_map.get(&soft_ent_a.id()),
                soft_entity_map.get(&soft_ent_b.id()),
            ) else {
                continue;
            };

            if idx_a == idx_b {
                continue;
            }

            // Safe simultaneous mutable access to two different entries.
            let (sb_a, sb_b) = if idx_a < idx_b {
                let (left, right) = soft_bodies.split_at_mut(idx_b);
                (&mut left[idx_a].1, &mut right[0].1)
            } else {
                let (left, right) = soft_bodies.split_at_mut(idx_a);
                (&mut right[0].1, &mut left[idx_b].1)
            };

            for node_a in &mut sb_a.nodes {
                for node_b in &mut sb_b.nodes {
                    let diff = node_a.position - node_b.position;
                    let dist_sq = diff.length_squared();

                    if dist_sq >= node_diam_sq || dist_sq <= 1e-6 {
                        continue;
                    }

                    let dist = dist_sq.sqrt();
                    let normal = diff / dist; // B → A
                    let penetration = NODE_RADIUS * 2.0 - dist;

                    let rel_vel = node_a.velocity - node_b.velocity;
                    let vel_along_normal = rel_vel.dot(normal);

                    let force_mag =
                        penetration * PENALTY_STIFFNESS - vel_along_normal * PENALTY_DAMPING;

                    if force_mag <= 0.0 {
                        continue;
                    }

                    let inv_m_a = if node_a.mass > 0.0 && !node_a.is_fixed {
                        1.0 / node_a.mass
                    } else {
                        0.0
                    };
                    let inv_m_b = if node_b.mass > 0.0 && !node_b.is_fixed {
                        1.0 / node_b.mass
                    } else {
                        0.0
                    };
                    let total_inv_m = inv_m_a + inv_m_b;

                    if total_inv_m <= 1e-8 {
                        continue;
                    }

                    let impulse = normal * (force_mag * dt);
                    node_a.velocity += impulse * inv_m_a;
                    node_b.velocity -= impulse * inv_m_b;

                    let correction = normal * (penetration * 0.5);
                    node_a.position += correction * (inv_m_a / total_inv_m);
                    node_b.position -= correction * (inv_m_b / total_inv_m);
                }
            }
        }

        manifolds
    }

    // ================================================================== //
    //  Stage 4–4.5 — Constraint solver                                    //
    // ================================================================== //

    /// Solve collision constraints (via island-parallel PGS) and explicit
    /// joints (hinges, springs, …).
    #[tracing::instrument(skip_all, name = "constraint_solve")]
    pub(crate) fn constraint_solve_step(&mut self, manifolds: Vec<ContactManifold>, dt: f32) {
        let entity_map = &self.entity_index_map;

        // ── Collision constraints ─────────────────────────────────────────
        if !manifolds.is_empty() {
            let is_dynamic = |entity: Entity| -> bool {
                entity_map
                    .get(&entity.id())
                    .is_some_and(|&idx| self.rigid_bodies[idx].is_dynamic())
            };

            let islands = crate::island::IslandManager::build_islands(&manifolds, &is_dynamic);
            let island_groups = crate::island::IslandManager::split_manifolds(manifolds, &islands);

            let rigid_bodies = &self.rigid_bodies;
            let transforms = &self.transforms;
            let velocities = &self.velocities;
            let solver = &self.solver;
            let entities_arr = &self.entities;

            type IslandResult = (
                Vec<(Entity, Velocity)>, // velocity updates
                Vec<ContactManifold>,    // solved manifolds (warm-start data)
                Vec<Entity>,             // entities to wake up
                Vec<crate::collision::FractureEvent>,
            );

            let results: Vec<IslandResult> = island_groups
                .into_par_iter()
                .map(|mut island_manifolds| -> IslandResult {
                    // Skip entirely if the whole island is asleep.
                    let island_awake = island_manifolds.iter().any(|m| {
                        [m.entity_a, m.entity_b].iter().any(|&e| {
                            entity_map.get(&e.id()).is_some_and(|&i| {
                                rigid_bodies[i].is_dynamic() && !rigid_bodies[i].is_sleeping
                            })
                        })
                    });

                    if !island_awake {
                        return (Vec::new(), island_manifolds, Vec::new(), Vec::new());
                    }

                    // Collect island indices and bodies that need waking.
                    let mut island_indices = std::collections::HashSet::new();
                    let mut wake_updates = Vec::new();

                    for m in &island_manifolds {
                        for &e in &[m.entity_a, m.entity_b] {
                            if let Some(&idx) = entity_map.get(&e.id()) {
                                island_indices.insert(idx);
                                if rigid_bodies[idx].is_dynamic() && rigid_bodies[idx].is_sleeping {
                                    wake_updates.push(e);
                                }
                            }
                        }
                    }

                    // Thread-local velocity buffer to avoid per-island allocations.
                    thread_local! {
                        static VEL_CACHE: std::cell::RefCell<Vec<Velocity>> =
                            const { std::cell::RefCell::new(Vec::new()) };
                    }

                    let mut velocity_updates = Vec::with_capacity(island_indices.len());

                    VEL_CACHE.with(|cache| {
                        let mut buf = cache.borrow_mut();

                        // Grow to fit the full velocity array if needed.
                        if buf.len() < velocities.len() {
                            buf.resize(velocities.len(), Velocity::default());
                        }

                        // Copy only this island's velocities in.
                        for &idx in &island_indices {
                            buf[idx] = velocities[idx];
                        }

                        solver.solve_contacts(
                            &mut island_manifolds,
                            rigid_bodies,
                            transforms,
                            &mut buf,
                            entity_map,
                            dt,
                        );

                        // Collect results.
                        for &idx in &island_indices {
                            if rigid_bodies[idx].is_dynamic() {
                                velocity_updates.push((entities_arr[idx], buf[idx]));
                            }
                        }
                    });

                    // Fracture detection.
                    let mut fractures = Vec::new();
                    for m in &island_manifolds {
                        let max_impulse_contact = m.contacts.iter().max_by(|a, b| {
                            a.normal_impulse.partial_cmp(&b.normal_impulse).unwrap()
                        });

                        if let Some(contact) = max_impulse_contact {
                            let impulse = contact.normal_impulse;
                            let point = contact.point;

                            for &(entity, manifold_ent) in
                                &[(m.entity_a, m.entity_a), (m.entity_b, m.entity_b)]
                            {
                                let _ = manifold_ent; // suppress unused warning
                                if let Some(&idx) = entity_map.get(&entity.id()) {
                                    if let Some(threshold) = rigid_bodies[idx].fracture_threshold {
                                        if impulse > threshold {
                                            fractures.push(crate::collision::FractureEvent {
                                                entity,
                                                impact_point: point,
                                                impact_force: impulse,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }

                    (velocity_updates, island_manifolds, wake_updates, fractures)
                })
                .collect();

            // Write-back: velocities, wake-ups, fractures, warm-start data.
            // Build a lookup from entity pair → event index for O(1) updates.
            let event_index: HashMap<(Entity, Entity), usize> = self
                .collision_events
                .iter()
                .enumerate()
                .map(|(i, e)| ((e.entity_a, e.entity_b), i))
                .collect();

            for (island_vels, island_manifolds, wake_ups, local_fractures) in results {
                for (entity, vel) in island_vels {
                    if let Some(&idx) = entity_map.get(&entity.id()) {
                        self.velocities[idx] = vel;
                    }
                }
                for entity in wake_ups {
                    if let Some(&idx) = entity_map.get(&entity.id()) {
                        self.rigid_bodies[idx].wake_up();
                    }
                }
                self.fracture_events.extend(local_fractures);

                // Update solved contact points on the matching collision event.
                for manifold in island_manifolds {
                    let solved: arrayvec::ArrayVec<_, 4> =
                        manifold.contacts.iter().copied().take(4).collect();

                    // Try both orderings (broadphase may report either).
                    let key_ab = (manifold.entity_a, manifold.entity_b);
                    let key_ba = (manifold.entity_b, manifold.entity_a);

                    if let Some(&idx) = event_index
                        .get(&key_ab)
                        .or_else(|| event_index.get(&key_ba))
                    {
                        self.collision_events[idx].contact_points = solved;
                    }
                }
            }
        }

        // ── Joints ────────────────────────────────────────────────────────
        if !self.joints.is_empty() {
            self.joint_solver.solve_joints(
                &mut self.joints,
                &self.entity_index_map,
                &self.rigid_bodies,
                &self.transforms,
                &mut self.velocities,
                dt,
            );
        }
    }

    // ================================================================== //
    //  Stage 5-6 — Position integration & sleep update                    //
    // ================================================================== //

    /// Integrate positions from velocities (parallel) and update sleep state.
    #[tracing::instrument(skip_all, name = "position_integration")]
    pub(crate) fn position_integration_step(
        &mut self,
        dt: f32,
    ) -> Result<(), crate::error::GizmoError> {
        let integrator = &self.integrator;

        self.entities
            .par_iter()
            .zip(self.rigid_bodies.par_iter_mut())
            .zip(self.transforms.par_iter_mut())
            .zip(self.velocities.par_iter_mut())
            .try_for_each(
                |(((&entity, rb), transform), vel)| -> Result<(), crate::error::GizmoError> {
                    if rb.is_sleeping {
                        return Ok(());
                    }

                    // Apply axis locks before position integration.
                    rb.enforce_locks(vel);

                    integrator.integrate_positions(entity, rb, transform, vel, dt)?;

                    Ok(())
                },
            )
            .inspect_err(|e| {
                tracing::error!("Position integration failed: {e:?}");
                self.trigger_snapshot("Position Integration Error (NaN/Overflow)");
            })
    }
}
