use crate::{
    broadphase::SpatialHash,
    collision::{CollisionEvent, CollisionEventType, ContactManifold, TriggerEvent},
    components::{Collider, RigidBody, Transform, Velocity},
    integrator::Integrator,
    narrowphase::NarrowPhase,
    raycast::{Ray, Raycast, RaycastHit},
    solver::ConstraintSolver,
    soft_body::SoftBodyMesh,
};
use gizmo_math::Aabb;
use gizmo_core::entity::Entity;
use rayon::prelude::*;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct FluidZone {
    pub bounds_min: gizmo_math::Vec3,
    pub bounds_max: gizmo_math::Vec3,
    pub density: f32,
    pub drag: f32,
}

/// Main physics world that manages all physics simulation
pub struct PhysicsWorld {
    pub integrator: Integrator,
    pub solver: ConstraintSolver,
    pub spatial_hash: SpatialHash,
    pub collision_events: Vec<CollisionEvent>,
    pub trigger_events: Vec<TriggerEvent>,
    pub joints: Vec<crate::joints::Joint>,
    pub joint_solver: crate::joints::JointSolver,
    pub fluid_zones: Vec<FluidZone>,
    pub gpu_compute: Option<crate::gpu_compute::GpuCompute>,
    contact_cache: HashMap<(Entity, Entity), bool>, // Track persistent contacts
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsWorld {
    pub fn new() -> Self {
        Self {
            integrator: Integrator::default(),
            solver: ConstraintSolver::default(),
            spatial_hash: SpatialHash::new(10.0), // 10 meter cells
            collision_events: Vec::new(),
            trigger_events: Vec::new(),
            joints: Vec::new(),
            joint_solver: crate::joints::JointSolver::new(10), // 10 iterations by default
            fluid_zones: Vec::new(),
            gpu_compute: None,
            contact_cache: HashMap::new(),
        }
    }

    pub fn with_gravity(mut self, gravity: gizmo_math::Vec3) -> Self {
        self.integrator.gravity = gravity;
        self
    }

    pub fn enable_gpu_compute(&mut self) {
        self.gpu_compute = pollster::block_on(crate::gpu_compute::GpuCompute::new());
    }

    pub fn with_cell_size(mut self, cell_size: f32) -> Self {
        self.spatial_hash = SpatialHash::new(cell_size);
        self
    }

    /// Main physics step - call this every frame
    pub fn step(
        &mut self,
        bodies: &mut [(Entity, RigidBody, Transform, Velocity, Collider)],
        soft_bodies: &mut [(Entity, SoftBodyMesh, Transform)],
        dt: f32,
    ) {
        // Clear events from last frame
        self.collision_events.clear();
        self.trigger_events.clear();

        // 0. Apply Fluid Buoyancy and Drag (Parallel)
        if !self.fluid_zones.is_empty() {
            bodies.par_iter_mut().for_each(|(_, rb, transform, vel, collider)| {
                for zone in &self.fluid_zones {
                    let pos = transform.position;
                    if pos.x >= zone.bounds_min.x && pos.x <= zone.bounds_max.x &&
                       pos.y >= zone.bounds_min.y && pos.y <= zone.bounds_max.y &&
                       pos.z >= zone.bounds_min.z && pos.z <= zone.bounds_max.z {
                        
                        let extents_y = collider.extents_y();
                        let depth = (zone.bounds_max.y - (pos.y - extents_y)).max(0.0).min(extents_y * 2.0);
                        let submerged_ratio = depth / (extents_y * 2.0);
                        if submerged_ratio > 0.0 {
                            let volume = collider.volume();
                            let buoyancy_force = gizmo_math::Vec3::new(0.0, zone.density * volume * submerged_ratio * 9.81, 0.0);
                            let drag_force = -vel.linear * zone.drag * submerged_ratio;
                            
                            let accel = (buoyancy_force + drag_force) * rb.inv_mass();
                            vel.linear += accel * dt;
                        }
                    }
                }
            });
        }

        // 1. Integrate velocities (apply forces, gravity, damping) (Parallel)
        // Integrator methods take &self, so we can share it
        let integrator = &self.integrator;
        bodies.par_iter_mut().for_each(|(_, rb, _, vel, _)| {
            integrator.integrate_velocities(rb, vel, dt);
        });

        // 1.5 Step Soft Bodies
        let gravity = integrator.gravity;
        if let Some(gpu) = &self.gpu_compute {
            let rigid_colliders: Vec<(gizmo_core::entity::Entity, crate::components::Transform, crate::components::Collider)> = bodies.iter().map(|(e, _, t, _, c)| (*e, *t, c.clone())).collect();
            gpu.step_soft_bodies(soft_bodies, &rigid_colliders, dt, gravity);
        } else {
            let rigid_colliders: Vec<(gizmo_core::entity::Entity, crate::components::Transform, crate::components::Collider)> = bodies.iter().map(|(e, _, t, _, c)| (*e, *t, c.clone())).collect();
            soft_bodies.par_iter_mut().for_each(|(_, sb, _)| {
                sb.step(dt, gravity, &rigid_colliders);
            });
        }

        // 2. Broadphase - build spatial hash and find potential collision pairs
        // CCD: expand AABB of fast-moving bodies by their predicted displacement so
        // the broad phase can still find pairs before interpenetration occurs.
        self.spatial_hash.clear();
        for (entity, rb, transform, vel, collider) in bodies.iter() {
            let aabb = collider.compute_aabb(transform.position, transform.rotation);
            let aabb = if rb.ccd_enabled && rb.is_dynamic() && !rb.is_sleeping {
                let next_pos = transform.position + vel.linear * dt;
                let next_aabb = collider.compute_aabb(next_pos, transform.rotation);
                aabb.merge(next_aabb)
            } else {
                aabb
            };
            self.spatial_hash.insert(*entity, aabb);
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

        // Build Entity -> Index map for fast O(1) lookup
        let mut entity_map = HashMap::new();
        for (i, (entity, _, _, _, _)) in bodies.iter().enumerate() {
            entity_map.insert(*entity, i);
        }

        let mut soft_entity_map = HashMap::new();
        for (i, (entity, _, _)) in soft_bodies.iter().enumerate() {
            soft_entity_map.insert(*entity, i);
        }

        let potential_pairs = self.spatial_hash.query_pairs();

        // 3. Narrowphase - detect actual collisions (Parallel)
        let narrowphase_results: Vec<_> = potential_pairs.par_iter().filter_map(|&(entity_a, entity_b)| {
            let is_a_rigid = entity_map.contains_key(&entity_a);
            let is_b_rigid = entity_map.contains_key(&entity_b);
            
            if is_a_rigid && is_b_rigid {
                let idx_a = *entity_map.get(&entity_a).unwrap();
                let idx_b = *entity_map.get(&entity_b).unwrap();
                let (_, _, transform_a, _, collider_a) = &bodies[idx_a];
                let (_, _, transform_b, _, collider_b) = &bodies[idx_b];

                // Check collision layers
                if !collider_a.collision_layer.can_collide_with(&collider_b.collision_layer) {
                    return None;
                }

                // Perform narrowphase collision detection
                if let Some(contact) = NarrowPhase::test_collision(
                    &collider_a.shape,
                    transform_a.position,
                    transform_a.rotation,
                    &collider_b.shape,
                    transform_b.position,
                    transform_b.rotation,
                ) {
                    Some((
                        entity_a, 
                        entity_b, 
                        Some(contact), 
                        collider_a.is_trigger, 
                        collider_b.is_trigger,
                        collider_a.material,
                        collider_b.material,
                        false // not soft
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
                    None,
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
                    None,
                    false, false, 
                    crate::components::PhysicsMaterial::default(), 
                    crate::components::PhysicsMaterial::default(),
                    true // is soft collision, but we will distinguish it
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
                let is_a_rigid = entity_map.contains_key(&entity_a);
                let is_b_rigid = entity_map.contains_key(&entity_b);
                if is_a_rigid != is_b_rigid {
                    let rigid_ent = if is_a_rigid { entity_a } else { entity_b };
                    let soft_ent = if is_a_rigid { entity_b } else { entity_a };
                    soft_rigid_pairs.push((rigid_ent, soft_ent));
                } else {
                    soft_soft_pairs.push((entity_a, entity_b));
                }
                continue;
            }
            
            let contact = contact_opt.unwrap();
            let pair = (entity_a, entity_b);
            current_contacts.insert(pair, true);

            // Handle triggers
            if is_trigger_a || is_trigger_b {
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
                // Create contact manifold for solid collisions
                let mut manifold = ContactManifold::new(entity_a, entity_b);
                
                // Combine physics materials
                manifold.friction = (mat_a.dynamic_friction * mat_b.dynamic_friction).sqrt();
                manifold.static_friction = (mat_a.static_friction * mat_b.static_friction).sqrt();
                manifold.restitution = mat_a.restitution.max(mat_b.restitution);
                
                manifold.add_contact(contact);
                manifolds.push(manifold);

                // Generate collision event
                let event_type = if self.contact_cache.contains_key(&pair) {
                    CollisionEventType::Persisting
                } else {
                    CollisionEventType::Started
                };

                self.collision_events.push(CollisionEvent {
                    entity_a,
                    entity_b,
                    event_type,
                    contact_points: vec![contact],
                });
            }
        }

        // Detect ended collisions
        for (pair, _) in self.contact_cache.iter() {
            if !current_contacts.contains_key(pair) {
                self.collision_events.push(CollisionEvent {
                    entity_a: pair.0,
                    entity_b: pair.1,
                    event_type: CollisionEventType::Ended,
                    contact_points: Vec::new(),
                });
            }
        }

        self.contact_cache = current_contacts;

        // 3.5 Process Soft vs Rigid collisions
        let node_shape = crate::components::ColliderShape::Sphere(crate::components::SphereShape { radius: 0.1 });
        for (rigid_ent, soft_ent) in soft_rigid_pairs {
            let rigid_idx = *entity_map.get(&rigid_ent).unwrap();
            let soft_idx = *soft_entity_map.get(&soft_ent).unwrap();
            
            // We need to borrow them mutably, but sequentially it's fine since we don't alias the same index
            let (_, rigid_rb, rigid_trans, rigid_vel, rigid_collider) = &mut bodies[rigid_idx];
            let (_, soft_body, _) = &mut soft_bodies[soft_idx];
            
            for node in soft_body.nodes.iter_mut() {
                if let Some(contact) = NarrowPhase::test_collision(
                    &node_shape,
                    node.position,
                    gizmo_math::Quat::IDENTITY,
                    &rigid_collider.shape,
                    rigid_trans.position,
                    rigid_trans.rotation,
                ) {
                    // We have a collision! Normal points from Node(A) to Rigid(B)
                    // Wait, NarrowPhase normal points from A to B. So from Node to Rigid.
                    let normal = contact.normal;
                    let penetration = contact.penetration;
                    
                    let inv_m_node = 1.0 / node.mass;
                    let inv_m_rb = rigid_rb.inv_mass();
                    let total_inv_m = inv_m_node + inv_m_rb;
                    
                    let r_rb = contact.point - rigid_trans.position;
                    let v_node = node.velocity;
                    let v_rb = rigid_vel.linear + rigid_vel.angular.cross(r_rb);
                    
                    let rel_vel = v_rb - v_node; // Rel vel of B relative to A
                    let vel_norm = rel_vel.dot(normal);
                    
                    if vel_norm < 0.0 {
                        // Applying a bouncy penalty
                        let j = -(1.0 + 0.2) * vel_norm / total_inv_m;
                        let impulse = normal * j;
                        
                        node.velocity -= impulse * inv_m_node; // A gets -impulse
                        if rigid_rb.is_dynamic() {
                            rigid_vel.linear += impulse * inv_m_rb; // B gets +impulse
                        }
                    }
                    
                    // Baumgarte position correction
                    let pos_correction = normal * (penetration * 0.5);
                    node.position -= pos_correction * (inv_m_node / total_inv_m);
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
            let idx_a = *soft_entity_map.get(&soft_ent_a).unwrap();
            let idx_b = *soft_entity_map.get(&soft_ent_b).unwrap();
            
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
                            let impulse = normal * (force_mag * dt);
                            let inv_m_a = if node_a.mass > 0.0 && !node_a.is_fixed { 1.0 / node_a.mass } else { 0.0 };
                            let inv_m_b = if node_b.mass > 0.0 && !node_b.is_fixed { 1.0 / node_b.mass } else { 0.0 };
                            let sum_inv_m = inv_m_a + inv_m_b;
                            
                            node_a.velocity += impulse * (inv_m_a / sum_inv_m);
                            node_b.velocity -= impulse * (inv_m_b / sum_inv_m);
                            
                            // Position correction
                            let pos_corr = normal * (penetration * 0.5);
                            node_a.position += pos_corr;
                            node_b.position -= pos_corr;
                        }
                    }
                }
            }
        }

        // 4. Solve constraints (only for non-trigger collisions)
        if !manifolds.is_empty() {
            let mut bodies_a = Vec::new();
            let mut bodies_b = Vec::new();

            // O(1) mapping using entity_map
            for manifold in &manifolds {
                if let Some(&idx_a) = entity_map.get(&manifold.entity_a) {
                    let (_, rb_a, t_a, v_a, _) = bodies[idx_a];
                    bodies_a.push((rb_a, t_a, v_a));
                }
                if let Some(&idx_b) = entity_map.get(&manifold.entity_b) {
                    let (_, rb_b, t_b, v_b, _) = bodies[idx_b];
                    bodies_b.push((rb_b, t_b, v_b));
                }
            }

            self.solver.solve_contacts(&mut manifolds, &mut bodies_a, &mut bodies_b, dt);

            // Write back velocities
            for (i, manifold) in manifolds.iter().enumerate() {
                if let Some(&idx_a) = entity_map.get(&manifold.entity_a) {
                    bodies[idx_a].3 = bodies_a[i].2;
                }
                if let Some(&idx_b) = entity_map.get(&manifold.entity_b) {
                    bodies[idx_b].3 = bodies_b[i].2;
                }
            }
            
            // Update collision events with resolved impulses
            for manifold in &manifolds {
                if let Some(event) = self.collision_events.iter_mut().find(|e| {
                    (e.entity_a == manifold.entity_a && e.entity_b == manifold.entity_b) ||
                    (e.entity_a == manifold.entity_b && e.entity_b == manifold.entity_a)
                }) {
                    event.contact_points = manifold.contacts.clone();
                }
            }
        }

        // 4.5 Solve explicit joints (Hinges, Springs, etc.)
        if !self.joints.is_empty() {
            self.joint_solver.solve_joints(&mut self.joints, bodies, dt);
        }

        // 5. Integrate positions (Parallel)
        bodies.par_iter_mut().for_each(|(_, rb, transform, vel, _)| {
            integrator.integrate_positions(rb, transform, vel, dt);
        });
    }

    /// Get collision events from last step
    pub fn collision_events(&self) -> &[CollisionEvent] {
        &self.collision_events
    }

    /// Get trigger events from last step
    pub fn trigger_events(&self) -> &[TriggerEvent] {
        &self.trigger_events
    }

    /// Apply an impulse to a body at a point
    pub fn apply_impulse(
        &self,
        rb: &RigidBody,
        transform: &Transform,
        vel: &mut Velocity,
        impulse: gizmo_math::Vec3,
        point: gizmo_math::Vec3,
    ) {
        Integrator::apply_impulse_at_point(rb, transform, vel, impulse, point);
    }

    /// Apply a force to a body
    pub fn apply_force(
        &self,
        rb: &RigidBody,
        vel: &mut Velocity,
        force: gizmo_math::Vec3,
        dt: f32,
    ) {
        Integrator::apply_force(rb, vel, force, dt);
    }

    /// Perform a raycast against all bodies
    pub fn raycast(
        &self,
        ray: &Ray,
        bodies: &[(Entity, RigidBody, Transform, Velocity, Collider)],
        max_distance: f32,
    ) -> Option<RaycastHit> {
        let mut closest_hit: Option<RaycastHit> = None;
        let mut closest_distance = max_distance;

        for (entity, _rb, transform, _vel, collider) in bodies {
            // First check AABB for early rejection
            let aabb = collider.compute_aabb(transform.position, transform.rotation);
            if Raycast::ray_aabb(ray, &aabb).is_none() {
                continue;
            }

            // Detailed shape test
            if let Some((distance, normal)) = Raycast::ray_shape(ray, &collider.shape, transform) {
                if distance < closest_distance {
                    closest_distance = distance;
                    closest_hit = Some(RaycastHit {
                        entity: *entity,
                        point: ray.point_at(distance),
                        normal,
                        distance,
                    });
                }
            }
        }

        closest_hit
    }

    /// Perform a raycast and return all hits
    pub fn raycast_all(
        &self,
        ray: &Ray,
        bodies: &[(Entity, RigidBody, Transform, Velocity, Collider)],
        max_distance: f32,
    ) -> Vec<RaycastHit> {
        let mut hits = Vec::new();

        for (entity, _rb, transform, _vel, collider) in bodies {
            // First check AABB
            let aabb = collider.compute_aabb(transform.position, transform.rotation);
            if Raycast::ray_aabb(ray, &aabb).is_none() {
                continue;
            }

            // Detailed shape test
            if let Some((distance, normal)) = Raycast::ray_shape(ray, &collider.shape, transform) {
                if distance <= max_distance {
                    hits.push(RaycastHit {
                        entity: *entity,
                        point: ray.point_at(distance),
                        normal,
                        distance,
                    });
                }
            }
        }

        // Sort by distance
        hits.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    #[test]
    fn test_physics_world_creation() {
        let world = PhysicsWorld::new();
        assert_eq!(world.integrator.gravity, Vec3::new(0.0, -9.81, 0.0));
    }

    #[test]
    fn test_physics_step() {
        let mut world = PhysicsWorld::new();

        let entity = Entity::new(1, 0);
        let rb = RigidBody::default();
        let transform = Transform::new(Vec3::new(0.0, 10.0, 0.0));
        let vel = Velocity::default();
        let collider = Collider::sphere(1.0);

        let mut bodies = vec![(entity, rb, transform, vel, collider)];

        // Simulate for 1 second
        for _ in 0..60 {
            world.step(&mut bodies, &mut [], 1.0 / 60.0);
        }

        // Object should have fallen due to gravity
        assert!(bodies[0].2.position.y < 10.0);
    }
}
