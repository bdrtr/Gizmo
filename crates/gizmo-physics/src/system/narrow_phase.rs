use gizmo_math::Vec3;
use crate::components::{Transform, RigidBody, Velocity};
use crate::shape::Collider;
use super::types::{StoredContact, DetectionResult};
use super::ccd::ccd_bisect;

pub fn merge_detection_results(mut acc: DetectionResult, mut item: DetectionResult) -> DetectionResult {
    if acc.contacts.is_empty() && acc.wake_entities.is_empty() {
        return item;
    }
    if item.contacts.is_empty() && item.wake_entities.is_empty() {
        return acc;
    }
    acc.contacts.append(&mut item.contacts);
    acc.wake_entities.append(&mut item.wake_entities);
    acc
}

pub fn detect_single_collision_pair(
    ent_a: u32,
    ent_b: u32,
    transforms: &gizmo_core::SparseSet<Transform>,
    colliders: &gizmo_core::SparseSet<Collider>,
    rigidbodies: &gizmo_core::SparseSet<RigidBody>,
    velocities: &gizmo_core::SparseSet<Velocity>,
    vehicle_entities: &std::collections::HashSet<u32>,
    _has_vehicles: bool,
    dt: f32,
) -> Option<DetectionResult> {
    use crate::shape::ColliderShape;

    let t_dense = &transforms.dense;
    let t_sparse = &transforms.sparse;
    let c_dense = &colliders.dense;
    let c_sparse = &colliders.sparse;
    let rb_dense = &rigidbodies.dense;
    let rb_sparse = &rigidbodies.sparse;
    let v_dense = &velocities.dense;
    let v_sparse = &velocities.sparse;
    let _v_set = vehicle_entities;

    let rb_a = rb_sparse.get(&ent_a).map(|&i| &rb_dense[i])?;
    let rb_b = rb_sparse.get(&ent_b).map(|&i| &rb_dense[i])?;

    if rb_a.data.mass == 0.0 && rb_b.data.mass == 0.0 {
        return None;
    }
    let both_dynamic_sleeping = rb_a.data.mass > 0.0
        && rb_b.data.mass > 0.0
        && rb_a.data.is_sleeping
        && rb_b.data.is_sleeping;
    if both_dynamic_sleeping {
        return None;
    }
    let layers_compatible = (rb_a.data.collision_layer & rb_b.data.collision_mask) != 0
        && (rb_b.data.collision_layer & rb_a.data.collision_mask) != 0;
    if !layers_compatible {
        return None;
    }

    let col_a = c_sparse.get(&ent_a).map(|&i| &c_dense[i])?;
    let col_b = c_sparse.get(&ent_b).map(|&i| &c_dense[i])?;
    let t_a = t_sparse.get(&ent_a).map(|&i| &t_dense[i])?;
    let t_b = t_sparse.get(&ent_b).map(|&i| &t_dense[i])?;
    let (pos_a, rot_a) = (t_a.data.position, t_a.data.rotation);
    let (pos_b, rot_b) = (t_b.data.position, t_b.data.rotation);

    let mut ccd_pos_a = None;
    let mut ccd_pos_b = None;

    let is_rot_a_identity =
        rot_a.x.abs() < 0.001 && rot_a.y.abs() < 0.001 && rot_a.z.abs() < 0.001;
    let is_rot_b_identity =
        rot_b.x.abs() < 0.001 && rot_b.y.abs() < 0.001 && rot_b.z.abs() < 0.001;

    let manifold = detect_pair(
        &col_a.data.shape,
        pos_a,
        rot_a,
        is_rot_a_identity,
        &col_b.data.shape,
        pos_b,
        rot_b,
        is_rot_b_identity,
    );

    let manifold = if !manifold.is_colliding && (rb_a.data.ccd_enabled || rb_b.data.ccd_enabled) {
        let v_a_lin = v_sparse
            .get(&ent_a)
            .map(|&i| v_dense[i].data.linear)
            .unwrap_or(Vec3::ZERO);
        let v_b_lin = v_sparse
            .get(&ent_b)
            .map(|&i| v_dense[i].data.linear)
            .unwrap_or(Vec3::ZERO);
        let rel_v = v_b_lin - v_a_lin;

        if rel_v.length() * dt > 0.1 {
            ccd_bisect(
                &col_a.data.shape,
                pos_a,
                rot_a,
                &col_b.data.shape,
                pos_b,
                rot_b,
                v_a_lin,
                v_b_lin,
                dt,
                &mut ccd_pos_a,
                &mut ccd_pos_b,
            )
        } else {
            manifold
        }
    } else {
        manifold
    };

    if !manifold.is_colliding || manifold.contact_points.is_empty() {
        return None;
    }

    let inv_mass_a = if rb_a.data.mass == 0.0 {
        0.0
    } else {
        1.0 / rb_a.data.mass
    };
    let inv_mass_b = if rb_b.data.mass == 0.0 {
        0.0
    } else {
        1.0 / rb_b.data.mass
    };

    let mut wakes = Vec::new();
    if rb_a.data.is_sleeping && rb_a.data.mass > 0.0 {
        wakes.push(ent_a);
    }
    if rb_b.data.is_sleeping && rb_b.data.mass > 0.0 {
        wakes.push(ent_b);
    }

    let mut result = DetectionResult {
        contacts: Vec::new(),
        wake_entities: wakes,
    };

    for (contact_point, pen) in &manifold.contact_points {
        let mut r_a = *contact_point - pos_a;
        let mut r_b = *contact_point - pos_b;
        if let ColliderShape::Sphere(s) = &col_a.data.shape {
            r_a = manifold.normal * s.radius;
        }
        if let ColliderShape::Sphere(s) = &col_b.data.shape {
            r_b = manifold.normal * -s.radius;
        }
        result.contacts.push(StoredContact {
            ent_a,
            ent_b,
            normal: manifold.normal,
            inv_mass_a,
            inv_mass_b,
            inv_inertia_a: rb_a.data.inverse_inertia_local,
            inv_inertia_b: rb_b.data.inverse_inertia_local,
            restitution: rb_a.data.restitution.max(rb_b.data.restitution),
            friction: (rb_a.data.friction * rb_b.data.friction).sqrt(),
            penetration: *pen,
            r_a,
            r_b,
            rot_a: t_a.data.rotation,
            rot_b: t_b.data.rotation,
            accumulated_j: 0.0,
            accumulated_friction: Vec3::ZERO,
            ccd_offset_a: ccd_pos_a.unwrap_or(Vec3::ZERO),
            ccd_offset_b: ccd_pos_b.unwrap_or(Vec3::ZERO),
            bias_bounce: 0.0,
            world_point: *contact_point,
        });
    }

    Some(result)
}

/// FAZ 2 — Narrow-Phase: Her çarpışma çifti için GJK/EPA veya analitik test + CCD bisection.
///
/// `parallel_narrow_phase`: `true` ise Rayon (sıra birleştirmesi platforma göre değişebilir);
/// tekrarlanabilir simülasyon için `PhysicsConfig::deterministic_simulation` ile `false` kullanılır.
pub fn detect_collisions(
    collision_pairs: &[(u32, u32)],
    transforms: &gizmo_core::SparseSet<Transform>,
    colliders: &gizmo_core::SparseSet<Collider>,
    rigidbodies: &gizmo_core::SparseSet<RigidBody>,
    velocities: &gizmo_core::SparseSet<Velocity>,
    vehicle_entities: &std::collections::HashSet<u32>,
    has_vehicles: bool,
    dt: f32,
    parallel_narrow_phase: bool,
) -> DetectionResult {
    if !parallel_narrow_phase {
        let mut acc = DetectionResult {
            contacts: Vec::new(),
            wake_entities: Vec::new(),
        };
        for &(ent_a, ent_b) in collision_pairs {
            if let Some(item) = detect_single_collision_pair(
                ent_a,
                ent_b,
                transforms,
                colliders,
                rigidbodies,
                velocities,
                vehicle_entities,
                has_vehicles,
                dt,
            ) {
                acc = merge_detection_results(acc, item);
            }
        }
        return acc;
    }

    use rayon::prelude::*;

    collision_pairs
        .par_iter()
        .filter_map(|&(ent_a, ent_b)| {
            detect_single_collision_pair(
                ent_a,
                ent_b,
                transforms,
                colliders,
                rigidbodies,
                velocities,
                vehicle_entities,
                has_vehicles,
                dt,
            )
        })
        .reduce(
            || DetectionResult {
                contacts: Vec::new(),
                wake_entities: Vec::new(),
            },
            merge_detection_results,
        )
}

/// Tek bir çarpışma çifti için analitik veya GJK/EPA ile manifold üret.
pub fn detect_pair(
    shape_a: &crate::shape::ColliderShape, pos_a: Vec3, rot_a: gizmo_math::Quat, rot_a_identity: bool,
    shape_b: &crate::shape::ColliderShape, pos_b: Vec3, rot_b: gizmo_math::Quat, rot_b_identity: bool,
) -> crate::collision::CollisionManifold {
    use crate::shape::ColliderShape::*;

    match (shape_a, shape_b) {
        (Aabb(a1), Aabb(a2)) => {
            if rot_a_identity && rot_b_identity {
                crate::collision::check_aabb_aabb_manifold(pos_a, a1, pos_b, a2)
            } else {
                crate::collision::check_obb_obb_manifold(pos_a, rot_a, a1, pos_b, rot_b, a2)
            }
        }
        (Sphere(s), Aabb(a)) => {
            if rot_b_identity {
                crate::collision::check_sphere_aabb_manifold(pos_a, s, pos_b, a)
            } else {
                crate::collision::check_sphere_obb_manifold(pos_a, s, pos_b, rot_b, a)
            }
        }
        (Aabb(a), Sphere(s)) => {
            let mut m = if rot_a_identity {
                crate::collision::check_sphere_aabb_manifold(pos_b, s, pos_a, a)
            } else {
                crate::collision::check_sphere_obb_manifold(pos_b, s, pos_a, rot_a, a)
            };
            m.normal = -m.normal;
            m
        }
        (Capsule(c1), Capsule(c2)) => {
            crate::collision::check_capsule_capsule_manifold(pos_a, rot_a, c1, pos_b, rot_b, c2)
        }
        (Capsule(c), Sphere(s)) => {
            crate::collision::check_capsule_sphere_manifold(pos_a, rot_a, c, pos_b, s)
        }
        (Sphere(s), Capsule(c)) => {
            let mut m = crate::collision::check_capsule_sphere_manifold(pos_b, rot_b, c, pos_a, s);
            m.normal *= -1.0;
            m
        }
        (Capsule(c), Aabb(a)) => {
            crate::collision::check_capsule_aabb_manifold(pos_a, rot_a, c, pos_b, a)
        }
        (Aabb(a), Capsule(c)) => {
            let mut m = crate::collision::check_capsule_aabb_manifold(pos_b, rot_b, c, pos_a, a);
            m.normal *= -1.0;
            m
        }
        (Sphere(s1), Sphere(s2)) => {
            crate::collision::check_sphere_sphere_manifold(pos_a, s1, pos_b, s2)
        }
        _ => {
            // GJK + EPA fallback (ConvexHull ve karışık şekiller için)
            let (is_colliding, simplex) = crate::gjk::gjk_intersect(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b);
            if is_colliding {
                crate::epa::epa_solve(simplex, shape_a, pos_a, rot_a, shape_b, pos_b, rot_b)
            } else {
                crate::collision::CollisionManifold {
                    is_colliding: false,
                    normal: Vec3::ZERO,
                    penetration: 0.0,
                    contact_points: vec![],
                }
            }
        }
    }
}
