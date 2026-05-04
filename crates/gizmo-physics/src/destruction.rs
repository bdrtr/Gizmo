use gizmo_core::entity::Entity;
 
use crate::world::PhysicsWorld;

/// The DestructionSystem handles runtime breaking of objects based on impacts.
pub struct DestructionSystem {
    pub impact_threshold: f32,
    pub crack_propagation_delay: f32,
}

impl Default for DestructionSystem {
    fn default() -> Self {
        Self {
            impact_threshold: 50.0, 
            crack_propagation_delay: 0.1, 
        }
    }
}

impl DestructionSystem {
    pub fn new(impact_threshold: f32) -> Self {
        Self {
            impact_threshold,
            crack_propagation_delay: 0.1,
        }
    }

    /// Evaluates all collisions and marks entities that should break.
    /// In a real ECS, this system would read CollisionEvents, check the RigidBody's fracture_threshold,
    /// and then spawn the ProceduralChunks from fracture.rs.
    pub fn process_impacts(&self, world: &PhysicsWorld) -> Vec<Entity> {
        let mut to_break = Vec::new();
        
        for event in world.collision_events() {
            if event.event_type == crate::collision::CollisionEventType::Started || event.event_type == crate::collision::CollisionEventType::Persisting {
                let impulse = event.contact_points.iter().map(|p| p.normal_impulse).sum::<f32>();
                
                if impulse > self.impact_threshold {
                    // This is a breaking impact.
                    to_break.push(event.entity_a);
                    to_break.push(event.entity_b);
                }
            }
        }
        
        to_break.sort_by_key(|e| e.id());
        to_break.dedup_by_key(|e| e.id());
        to_break
    }
}
