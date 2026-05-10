use crate::world::PhysicsWorld;

/// The DestructionSystem handles runtime breaking of objects based on impacts.
pub struct DestructionSystem {
    pub impact_threshold: f32,
}

impl Default for DestructionSystem {
    fn default() -> Self {
        Self {
            impact_threshold: 50.0,
        }
    }
}

impl DestructionSystem {
    pub fn new(impact_threshold: f32) -> Self {
        Self { impact_threshold }
    }

    /// Evaluates all collisions and marks entities that should break.
    /// In a real ECS, this system would read CollisionEvents, check the RigidBody's fracture_threshold,
    /// and then spawn the ProceduralChunks from fracture.rs.
    pub fn process_impacts(&self, world: &PhysicsWorld) -> Vec<crate::collision::FractureEvent> {
        let mut to_break = Vec::new();

        for event in world.collision_events() {
            // Sadece yeni başlayan çarpışmalarda kırma kontrolü yap, Persisting'de sürekli kırmayı engelle.
            if event.event_type == crate::collision::CollisionEventType::Started {
                let mut max_impulse = 0.0;
                let mut impact_point = gizmo_math::Vec3::ZERO;

                for p in &event.contact_points {
                    if p.normal_impulse > max_impulse {
                        max_impulse = p.normal_impulse;
                        impact_point = p.point;
                    }
                }

                // Entity A
                if let Some(&idx_a) = world.entity_index_map.get(&event.entity_a.id()) {
                    let rb_a = &world.rigid_bodies[idx_a];
                    // Eğer per-object threshold yoksa, sistem genel (impact_threshold) değerini minimum olarak kullan
                    let threshold_a = rb_a.fracture_threshold.unwrap_or(self.impact_threshold);

                    if max_impulse > threshold_a {
                        to_break.push(crate::collision::FractureEvent {
                            entity: event.entity_a,
                            impact_point,
                            impact_force: max_impulse,
                        });
                    }
                }

                // Entity B
                if let Some(&idx_b) = world.entity_index_map.get(&event.entity_b.id()) {
                    let rb_b = &world.rigid_bodies[idx_b];
                    let threshold_b = rb_b.fracture_threshold.unwrap_or(self.impact_threshold);

                    if max_impulse > threshold_b {
                        to_break.push(crate::collision::FractureEvent {
                            entity: event.entity_b,
                            impact_point,
                            impact_force: max_impulse,
                        });
                    }
                }
            }
        }

        to_break
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{CollisionEvent, CollisionEventType, ContactPoint};
    use crate::components::RigidBody;
    use gizmo_core::entity::Entity;
    use gizmo_math::Vec3;

    fn setup_world() -> PhysicsWorld {
        let mut world = PhysicsWorld::new();
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);

        let mut rb1 = RigidBody::default();
        rb1.fracture_threshold = Some(10.0);

        let mut rb2 = RigidBody::default();
        rb2.fracture_threshold = Some(100.0);

        use crate::components::{Collider, Transform, Velocity};

        world.add_body(
            e1,
            rb1,
            Transform::default(),
            Velocity::default(),
            Collider::sphere(1.0),
        );
        world.add_body(
            e2,
            rb2,
            Transform::default(),
            Velocity::default(),
            Collider::sphere(1.0),
        );

        world
    }

    #[test]
    fn test_destruction_thresholds() {
        let mut world = setup_world();
        let system = DestructionSystem::new(50.0);

        let mut event = CollisionEvent {
            entity_a: Entity::new(1, 0),
            entity_b: Entity::new(2, 0),
            event_type: CollisionEventType::Started,
            contact_points: arrayvec::ArrayVec::new(),
        };

        event.contact_points.push(ContactPoint {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            penetration: 0.1,
            local_point_a: Vec3::ZERO,
            local_point_b: Vec3::ZERO,
            normal_impulse: 20.0,
            tangent_impulse: Vec3::ZERO,
        });

        world.collision_events.push(event);

        let broken = system.process_impacts(&world);

        // Impulse is 20.0.
        // e1 threshold is 10.0 -> breaks!
        // e2 threshold is 100.0 -> does not break!
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].entity.id(), 1);
    }

    #[test]
    fn test_destruction_ignores_persisting() {
        let mut world = setup_world();
        let system = DestructionSystem::new(50.0);

        let mut event = CollisionEvent {
            entity_a: Entity::new(1, 0),
            entity_b: Entity::new(2, 0),
            event_type: CollisionEventType::Persisting,
            contact_points: arrayvec::ArrayVec::new(),
        };

        event.contact_points.push(ContactPoint {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            penetration: 0.1,
            local_point_a: Vec3::ZERO,
            local_point_b: Vec3::ZERO,
            normal_impulse: 200.0, // High impulse
            tangent_impulse: Vec3::ZERO,
        });

        world.collision_events.push(event);

        let broken = system.process_impacts(&world);

        // Should ignore because it's Persisting
        assert!(broken.is_empty());
    }

    #[test]
    fn test_destruction_fallback_threshold() {
        let mut world = PhysicsWorld::new();
        let e3 = Entity::new(3, 0);
        let rb3 = RigidBody::default(); // fracture_threshold is None
        use crate::components::{Collider, Transform, Velocity};
        world.add_body(
            e3,
            rb3,
            Transform::default(),
            Velocity::default(),
            Collider::sphere(1.0),
        );

        let system = DestructionSystem::new(50.0);

        let mut event = CollisionEvent {
            entity_a: e3,
            entity_b: Entity::new(99, 0), // doesn't exist, will be ignored
            event_type: CollisionEventType::Started,
            contact_points: arrayvec::ArrayVec::new(),
        };

        event.contact_points.push(ContactPoint {
            point: Vec3::ZERO,
            normal: Vec3::Y,
            penetration: 0.1,
            local_point_a: Vec3::ZERO,
            local_point_b: Vec3::ZERO,
            normal_impulse: 60.0, // > 50.0 fallback
            tangent_impulse: Vec3::ZERO,
        });

        world.collision_events.push(event);

        let broken = system.process_impacts(&world);
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].entity.id(), 3);
    }
}
