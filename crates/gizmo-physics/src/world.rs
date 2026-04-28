use crate::{
    broadphase::{Aabb, SpatialHash},
    collision::{CollisionEvent, CollisionEventType, ContactManifold, TriggerEvent},
    components::{Collider, RigidBody, Transform, Velocity},
    integrator::Integrator,
    narrowphase::NarrowPhase,
    solver::ConstraintSolver,
};
use gizmo_core::entity::Entity;
use std::collections::HashMap;

/// Main physics world that manages all physics simulation
pub struct PhysicsWorld {
    pub integrator: Integrator,
    pub solver: ConstraintSolver,
    pub spatial_hash: SpatialHash,
    pub collision_events: Vec<CollisionEvent>,
    pub trigger_events: Vec<TriggerEvent>,
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
            contact_cache: HashMap::new(),
        }
    }

    pub fn with_gravity(mut self, gravity: gizmo_math::Vec3) -> Self {
        self.integrator.gravity = gravity;
        self
    }

    pub fn with_cell_size(mut self, cell_size: f32) -> Self {
        self.spatial_hash = SpatialHash::new(cell_size);
        self
    }

    /// Main physics step - call this every frame
    pub fn step(
        &mut self,
        bodies: &mut [(Entity, RigidBody, Transform, Velocity, Collider)],
        dt: f32,
    ) {
        // Clear events from last frame
        self.collision_events.clear();
        self.trigger_events.clear();

        // 1. Integrate velocities (apply forces, gravity, damping)
        for (_, rb, _, vel, _) in bodies.iter_mut() {
            self.integrator.integrate_velocities(rb, vel, dt);
        }

        // 2. Broadphase - build spatial hash and find potential collision pairs
        self.spatial_hash.clear();
        for (entity, _, transform, _, collider) in bodies.iter() {
            let aabb = collider.compute_aabb(transform.position, transform.rotation);
            self.spatial_hash.insert(*entity, &aabb);
        }

        let potential_pairs = self.spatial_hash.query_pairs();

        // 3. Narrowphase - detect actual collisions
        let mut manifolds = Vec::new();
        let mut current_contacts = HashMap::new();

        for (entity_a, entity_b) in potential_pairs {
            // Find the bodies
            let body_a = bodies.iter().find(|(e, _, _, _, _)| *e == entity_a);
            let body_b = bodies.iter().find(|(e, _, _, _, _)| *e == entity_b);

            if let (Some((_, rb_a, transform_a, _, collider_a)), Some((_, rb_b, transform_b, _, collider_b))) =
                (body_a, body_b)
            {
                // Check collision layers
                if !collider_a.collision_layer.can_collide_with(&collider_b.collision_layer) {
                    continue;
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
                    let pair = (entity_a, entity_b);
                    current_contacts.insert(pair, true);

                    // Handle triggers
                    if collider_a.is_trigger || collider_b.is_trigger {
                        let event_type = if self.contact_cache.contains_key(&pair) {
                            CollisionEventType::Persisting
                        } else {
                            CollisionEventType::Started
                        };

                        self.trigger_events.push(TriggerEvent {
                            trigger_entity: if collider_a.is_trigger { entity_a } else { entity_b },
                            other_entity: if collider_a.is_trigger { entity_b } else { entity_a },
                            event_type,
                        });
                    } else {
                        // Create contact manifold for solid collisions
                        let mut manifold = ContactManifold::new(entity_a, entity_b);
                        manifold.friction = (collider_a.material.dynamic_friction
                            + collider_b.material.dynamic_friction)
                            * 0.5;
                        manifold.restitution =
                            (collider_a.material.restitution + collider_b.material.restitution) * 0.5;
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

        // 4. Solve constraints (only for non-trigger collisions)
        if !manifolds.is_empty() {
            let mut bodies_a = Vec::new();
            let mut bodies_b = Vec::new();

            for manifold in &manifolds {
                if let Some((_, rb_a, t_a, v_a, _)) =
                    bodies.iter_mut().find(|(e, _, _, _, _)| *e == manifold.entity_a)
                {
                    bodies_a.push((*rb_a, *t_a, *v_a));
                }
                if let Some((_, rb_b, t_b, v_b, _)) =
                    bodies.iter_mut().find(|(e, _, _, _, _)| *e == manifold.entity_b)
                {
                    bodies_b.push((*rb_b, *t_b, *v_b));
                }
            }

            self.solver.solve_contacts(&manifolds, &mut bodies_a, &mut bodies_b, dt);

            // Write back velocities
            for (i, manifold) in manifolds.iter().enumerate() {
                if let Some((_, _, _, v_a, _)) =
                    bodies.iter_mut().find(|(e, _, _, _, _)| *e == manifold.entity_a)
                {
                    *v_a = bodies_a[i].2;
                }
                if let Some((_, _, _, v_b, _)) =
                    bodies.iter_mut().find(|(e, _, _, _, _)| *e == manifold.entity_b)
                {
                    *v_b = bodies_b[i].2;
                }
            }
        }

        // 5. Integrate positions
        for (_, rb, transform, vel, _) in bodies.iter_mut() {
            self.integrator.integrate_positions(rb, transform, vel, dt);
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::{Quat, Vec3};

    #[test]
    fn test_physics_world_creation() {
        let world = PhysicsWorld::new();
        assert_eq!(world.integrator.gravity, Vec3::new(0.0, -9.81, 0.0));
    }

    #[test]
    fn test_physics_step() {
        let mut world = PhysicsWorld::new();

        let entity = Entity::from_raw(1);
        let rb = RigidBody::default();
        let mut transform = Transform::new(Vec3::new(0.0, 10.0, 0.0));
        let mut vel = Velocity::default();
        let collider = Collider::sphere(1.0);

        let mut bodies = vec![(entity, rb, transform, vel, collider)];

        // Simulate for 1 second
        for _ in 0..60 {
            world.step(&mut bodies, 1.0 / 60.0);
        }

        // Object should have fallen due to gravity
        assert!(bodies[0].2.position.y < 10.0);
    }
}
