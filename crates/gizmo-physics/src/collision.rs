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
    pub restitution: f32,
    pub lifetime: u32, // Frames this manifold has existed
}

impl ContactManifold {
    pub fn new(entity_a: Entity, entity_b: Entity) -> Self {
        Self {
            entity_a,
            entity_b,
            contacts: Vec::new(),
            friction: 0.5,
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
                // Update existing contact
                *existing = contact;
                return;
            }
        }
        
        // Limit to 4 contact points (common in physics engines)
        if self.contacts.len() < 4 {
            self.contacts.push(contact);
        } else {
            // Replace the contact with least penetration
            if let Some((idx, _)) = self
                .contacts
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.penetration.total_cmp(&b.penetration))
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
    
    pub fn refresh(&mut self) {
        self.lifetime += 1;
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
    pub contact_points: Vec<ContactPoint>,
}

/// Trigger event (for non-solid colliders)
#[derive(Debug, Clone)]
pub struct TriggerEvent {
    pub trigger_entity: Entity,
    pub other_entity: Entity,
    pub event_type: CollisionEventType,
}
