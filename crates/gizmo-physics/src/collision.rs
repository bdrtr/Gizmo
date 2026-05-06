use gizmo_core::entity::Entity;
use gizmo_math::Vec3;

/// Contact point between two colliding bodies
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct ContactPoint {
    pub point: Vec3,         // World-space contact point
    pub normal: Vec3,        // Contact normal (from A to B)
    pub penetration: f32,    // Penetration depth
    pub local_point_a: Vec3, // Contact point in body A's local space
    pub local_point_b: Vec3, // Contact point in body B's local space
    pub normal_impulse: f32, // Accumulated normal impulse for warm starting
    pub tangent_impulse: Vec3, // Accumulated tangent impulse for warm starting
}

/// Contact manifold - collection of contact points between two bodies
#[derive(Debug, Clone)]
pub struct ContactManifold {
    pub entity_a: Entity,
    pub entity_b: Entity,
    pub contacts: Vec<ContactPoint>,
    pub friction: f32,
    pub static_friction: f32,
    pub restitution: f32,
    pub lifetime: u32, // Frames this manifold has existed
}

impl ContactManifold {
    pub fn new(entity_a: Entity, entity_b: Entity) -> Self {
        let (entity_a, entity_b) = if entity_a.id() < entity_b.id() {
            (entity_a, entity_b)
        } else {
            (entity_b, entity_a)
        };
        Self {
            entity_a,
            entity_b,
            contacts: Vec::new(),
            // Varsayılan değerler; PhysicsWorld step döngüsünde (PhysicsMaterial::combine kullanılarak) ezilir.
            friction: 0.5,
            static_friction: 0.5,
            restitution: 0.5,
            lifetime: 0,
        }
    }

    pub fn add_contact(&mut self, contact: ContactPoint) {
        const CONTACT_DISTANCE_THRESHOLD: f32 = 0.02;
        
        // Try to match with existing contact for warm starting
        for existing in &mut self.contacts {
            let dist = (existing.point - contact.point).length_squared();
            if dist < CONTACT_DISTANCE_THRESHOLD * CONTACT_DISTANCE_THRESHOLD {
                // Update existing contact but preserve impulses
                let prev_normal = existing.normal_impulse;
                let prev_tangent = existing.tangent_impulse;
                *existing = contact;
                existing.normal_impulse = prev_normal;
                existing.tangent_impulse = prev_tangent;
                return;
            }
        }
        
        // Limit to 4 contact points (common in physics engines)
        if self.contacts.len() < 4 {
            self.contacts.push(contact);
        } else {
            // Convex hull / Maximum area heuristic approximation:
            // Find the pair of points closest to each other, and drop the one with the least penetration.
            let mut min_dist_sq = f32::MAX;
            let mut closest_pair = (0, 1);
            let points = [self.contacts[0], self.contacts[1], self.contacts[2], self.contacts[3], contact];
            
            for i in 0..5 {
                for j in (i + 1)..5 {
                    let dist_sq = (points[i].point - points[j].point).length_squared();
                    if dist_sq < min_dist_sq {
                        min_dist_sq = dist_sq;
                        closest_pair = (i, j);
                    }
                }
            }
            
            let (i, j) = closest_pair;
            let drop_idx = if points[i].penetration < points[j].penetration { i } else { j };
            
            self.contacts.clear();
            for k in 0..5 {
                if k != drop_idx {
                    self.contacts.push(points[k]);
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.contacts.clear();
    }
    
    /// Her frame çağrılır
    pub fn tick(&mut self) {
        self.lifetime += 1;
    }
    
    /// Çarpışma devam ediyorsa çağrılır
    pub fn refresh(&mut self) {
        self.lifetime = 0;
    }
    
    pub fn is_stale(&self, max_lifetime: u32) -> bool {
        self.lifetime > max_lifetime
    }
}

/// Collision event types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionEventType {
    Started,
    Persisting,
    Ended,
}

/// Collision event data
#[derive(Debug, Clone)]
pub struct CollisionEvent {
    pub entity_a: Entity,
    pub entity_b: Entity,
    pub event_type: CollisionEventType,
    pub contact_points: arrayvec::ArrayVec<ContactPoint, 4>,
}

/// Trigger event (for non-solid colliders)
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    pub trigger_entity: Entity,
    pub other_entity: Entity,
    pub event_type: CollisionEventType,
}

#[derive(Debug, Clone, Copy)]
pub struct FractureEvent {
    pub entity: gizmo_core::entity::Entity,
    pub impact_point: gizmo_math::Vec3,
    pub impact_force: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::Vec3;

    #[test]
    fn test_manifold_entity_ordering() {
        let e1 = Entity::new(10, 0);
        let e2 = Entity::new(5, 0);
        let manifold = ContactManifold::new(e1, e2);
        assert_eq!(manifold.entity_a.id(), 5);
        assert_eq!(manifold.entity_b.id(), 10);
    }

    #[test]
    fn test_warm_starting_impulse_preservation() {
        let mut manifold = ContactManifold::new(Entity::new(1, 0), Entity::new(2, 0));
        
        // Add first contact
        let pt1 = ContactPoint {
            point: Vec3::new(1.0, 0.0, 0.0),
            normal: Vec3::new(0.0, 1.0, 0.0),
            penetration: 0.1,
            local_point_a: Vec3::ZERO,
            local_point_b: Vec3::ZERO,
            normal_impulse: 5.0, // Existing impulse
            tangent_impulse: Vec3::new(1.0, 0.0, 0.0),
        };
        manifold.add_contact(pt1);

        // Add matching contact (close to pt1) with zero impulse (from new frame)
        let pt2 = ContactPoint {
            point: Vec3::new(1.001, 0.0, 0.0), // Very close, within threshold
            normal: Vec3::new(0.0, 1.0, 0.0),
            penetration: 0.2, // Updated penetration
            local_point_a: Vec3::ZERO,
            local_point_b: Vec3::ZERO,
            normal_impulse: 0.0, // New contact has 0
            tangent_impulse: Vec3::ZERO,
        };
        manifold.add_contact(pt2);

        assert_eq!(manifold.contacts.len(), 1);
        assert_eq!(manifold.contacts[0].normal_impulse, 5.0); // Preserved!
        assert_eq!(manifold.contacts[0].tangent_impulse, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(manifold.contacts[0].penetration, 0.2); // Updated!
    }

    #[test]
    fn test_contact_limit_and_area_maximization() {
        let mut manifold = ContactManifold::new(Entity::new(1, 0), Entity::new(2, 0));
        
        let add_pt = |m: &mut ContactManifold, x: f32, y: f32, pen: f32| {
            m.add_contact(ContactPoint {
                point: Vec3::new(x, y, 0.0),
                normal: Vec3::Y,
                penetration: pen,
                local_point_a: Vec3::ZERO,
                local_point_b: Vec3::ZERO,
                normal_impulse: 0.0,
                tangent_impulse: Vec3::ZERO,
            });
        };

        add_pt(&mut manifold, 0.0, 0.0, 1.0); // 1
        add_pt(&mut manifold, 10.0, 0.0, 1.0); // 2
        add_pt(&mut manifold, 0.0, 10.0, 1.0); // 3
        add_pt(&mut manifold, 10.0, 10.0, 1.0); // 4

        assert_eq!(manifold.contacts.len(), 4);

        // 5th point close to point 1, but with lesser penetration
        add_pt(&mut manifold, 0.1, 0.1, 0.5); // Should drop point 5 since it's the shallower of the closest pair (1 and 5)

        assert_eq!(manifold.contacts.len(), 4);
        
        let has_shallow = manifold.contacts.iter().any(|c| c.penetration == 0.5);
        assert!(!has_shallow, "Shallowest of the closest pair should be dropped");
    }

    #[test]
    fn test_tick_refresh_semantics() {
        let mut manifold = ContactManifold::new(Entity::new(1, 0), Entity::new(2, 0));
        assert_eq!(manifold.lifetime, 0);
        manifold.tick();
        manifold.tick();
        assert_eq!(manifold.lifetime, 2);
        manifold.refresh();
        assert_eq!(manifold.lifetime, 0);
    }
}
