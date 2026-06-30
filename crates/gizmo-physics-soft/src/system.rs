use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use crate::{SoftBodyMesh, cloth::Cloth, rope::Rope};

/// Soft-body integratörleri büyük `dt`'de (kare sıçraması/hitch) patlar (FEM
/// kararlılık sınırı + XPBD aşırı-tahmin). Adımı bu üst sınıra kırp.
const MAX_SOFT_DT: f32 = 1.0 / 30.0;

/// Steps every [`SoftBodyMesh`] in the world by `dt` seconds under `gravity`,
/// resolving collisions against all rigid colliders present in the world.
///
/// The timestep is clamped to a stable upper bound to keep the FEM integrator
/// stable across frame hitches.
#[tracing::instrument(skip_all, name = "soft_body_step_system")]
pub fn soft_body_step_system(world: &World, dt: f32, gravity: Vec3) {
    let dt = dt.min(MAX_SOFT_DT);
    // 1. Collect all rigid colliders for collision resolution
    let mut rigid_colliders = Vec::new();
    if let Some(q) = world.query::<(&Transform, &Collider)>() {
        // SoftBodyMesh expects tuples of (Entity, Transform, Collider)
        for (e, (trans, col)) in q.iter() {
            // Panik koruması: entity sorgu ile get_entity arasında despawn edilmiş
            // olabilir (yarış/tutarsızlık) → unwrap yerine bu collider'ı sessizce atla.
            if let Some(entity) = world.get_entity(e) {
                // Bridge: ECS entity -> opaque physics BodyHandle (id only).
                rigid_colliders.push((BodyHandle::from_id(entity.id()), *trans, col.clone()));
            }
        }
    }

    // 2. Query and step all SoftBodyMesh components
    if let Some(mut q) = unsafe { world.query_unchecked::<gizmo_core::query::Mut<SoftBodyMesh>>() } {
        for (_, mut soft_body) in q.iter_mut() {
            soft_body.step(dt, gravity, &rigid_colliders);
        }
    }
}

/// Steps every [`Cloth`] in the world by `dt` seconds under `gravity` using a fixed
/// number of XPBD sub-steps. The timestep is clamped to a stable upper bound.
#[tracing::instrument(skip_all, name = "cloth_step_system")]
pub fn cloth_step_system(world: &World, dt: f32, gravity: Vec3) {
    let dt = dt.min(MAX_SOFT_DT);
    // Determine a fixed number of XPBD substeps for stability
    let sub_steps = 10;
    
    if let Some(mut q) = unsafe { world.query_unchecked::<gizmo_core::query::Mut<Cloth>>() } {
        for (_, mut cloth) in q.iter_mut() {
            cloth.step(dt, gravity, sub_steps);
        }
    }
}

/// Steps every [`Rope`] in the world by `dt` seconds under `gravity`.
/// The timestep is clamped to a stable upper bound.
#[tracing::instrument(skip_all, name = "rope_step_system")]
pub fn rope_step_system(world: &World, dt: f32, gravity: Vec3) {
    let dt = dt.min(MAX_SOFT_DT);
    if let Some(mut q) = unsafe { world.query_unchecked::<gizmo_core::query::Mut<Rope>>() } {
        for (_, mut rope) in q.iter_mut() {
            rope.step(dt, gravity);
        }
    }
}
