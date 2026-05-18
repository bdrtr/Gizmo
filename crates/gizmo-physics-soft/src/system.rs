use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use crate::{SoftBodyMesh, cloth::Cloth, rope::Rope};

#[tracing::instrument(skip_all, name = "soft_body_step_system")]
pub fn soft_body_step_system(world: &World, dt: f32, gravity: Vec3) {
    // 1. Collect all rigid colliders for collision resolution
    let mut rigid_colliders = Vec::new();
    if let Some(q) = world.query::<(&Transform, &Collider)>() {
        // SoftBodyMesh expects tuples of (Entity, Transform, Collider)
        for (e, (trans, col)) in q.iter() {
            rigid_colliders.push((world.get_entity(e).unwrap(), *trans, col.clone()));
        }
    }

    // 2. Query and step all SoftBodyMesh components
    if let Some(mut q) = world.query::<gizmo_core::query::Mut<SoftBodyMesh>>() {
        for (_, mut soft_body) in q.iter_mut() {
            soft_body.step(dt, gravity, &rigid_colliders);
        }
    }
}

#[tracing::instrument(skip_all, name = "cloth_step_system")]
pub fn cloth_step_system(world: &World, dt: f32, gravity: Vec3) {
    // Determine a fixed number of XPBD substeps for stability
    let sub_steps = 10;
    
    if let Some(mut q) = world.query::<gizmo_core::query::Mut<Cloth>>() {
        for (_, mut cloth) in q.iter_mut() {
            cloth.step(dt, gravity, sub_steps);
        }
    }
}

#[tracing::instrument(skip_all, name = "rope_step_system")]
pub fn rope_step_system(world: &World, dt: f32, gravity: Vec3) {
    if let Some(mut q) = world.query::<gizmo_core::query::Mut<Rope>>() {
        for (_, mut rope) in q.iter_mut() {
            rope.step(dt, gravity);
        }
    }
}
