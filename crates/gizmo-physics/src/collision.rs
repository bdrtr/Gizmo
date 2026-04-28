use gizmo_core::entity::Entity;
use gizmo_math::Vec3;

/// Contact point between two colliding bodies
#[derive(Debug, Clone, Copy)]
pub struct ContactPoint {
    pub point: Vec3,         // World-space contact point
    pub normal: Vec3,        // Contact normal (from A to B)
    pub penetration: f32,    // Penetration depth
    pub local_point_a: Vec3, // Contact point in body A's local space
    pub local_point_b: Vec3, // Contact point in body B's local space
}

/// Contact manifold - collection of contact points between two bodies
#[derive(Debug, Clone)]
pub struct ContactManifold {
    pub entity_a: Entity,
    pub entity_b: Entity,
    pub contacts: Vec<ContactPoint>,
    pub friction: f32,
    pub restitution: f32,
}

impl ContactManifold {
    pub fn new(entity_a: Entity, entity_b: Entity) -> Self {
        Self {
            entity_a,
            entity_b,
            contacts: Vec::new(),
            friction: 0.5,
            restitution: 0.5,
        }
    }

    pub fn add_contact(&mut self, contact: ContactPoint) {
        // Limit to 4 contact points (common in physics engines)
        if self.contacts.len() < 4 {
            self.contacts.push(contact);
        } else {
            // Replace the contact with least penetration
            if let Some((idx, _)) = self
                .contacts
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.penetration.partial_cmp(&b.penetration).unwrap())
            {
                if contact.penetration > self.contacts[idx].penetration {
                    self.contacts[idx] = contact;
                }
            }
        }
    }

    pub fn clear(&mut self) {
        self.contacts.clear();
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
    pub contact_points: Vec<ContactPoint>,
}

/// Trigger event (for non-solid colliders)
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    pub trigger_entity: Entity,
    pub other_entity: Entity,
    pub event_type: CollisionEventType,
}
