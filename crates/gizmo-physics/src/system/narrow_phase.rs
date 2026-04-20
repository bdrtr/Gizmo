use gizmo_math::Vec3;
use crate::components::{Transform, RigidBody, Velocity};
use crate::shape::Collider;
use super::types::{StoredContact, DetectionResult};
use super::ccd::ccd_bisect;

pub fn merge_detection_results(mut acc: DetectionResult, mut item: DetectionResult) -> DetectionResult {
    let item_empty = item.contacts.is_empty() && item.wake_entities.is_empty();
    if item_empty {
        return acc;
    }
    
    let acc_empty = acc.contacts.is_empty() && acc.wake_entities.is_empty();
    if acc_empty {
        return item;
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
    dt: f32,
    ccd_velocity_threshold: f32,
) -> Option<DetectionResult> {
    use crate::shape::ColliderShape;

    let rb_a = rigidbodies.get(ent_a)?;
    let rb_b = rigidbodies.get(ent_b)?;

    if rb_a.mass == 0.0 && rb_b.mass == 0.0 {
        return None;
    }
    let both_dynamic_sleeping = rb_a.mass > 0.0
        && rb_b.mass > 0.0
        && rb_a.is_sleeping
        && rb_b.is_sleeping;
    if both_dynamic_sleeping {
        return None;
    }
    let layers_compatible = (rb_a.collision_layer & rb_b.collision_mask) != 0
        && (rb_b.collision_layer & rb_a.collision_mask) != 0;
    if !layers_compatible {
        return None;
    }

    let col_a = colliders.get(ent_a)?;
    let col_b = colliders.get(ent_b)?;
    let t_a = transforms.get(ent_a)?;
    let t_b = transforms.get(ent_b)?;
    let (pos_a, rot_a) = (t_a.position, t_a.rotation);
    let (pos_b, rot_b) = (t_b.position, t_b.rotation);

    let mut ccd_pos_a = None;
    let mut ccd_pos_b = None;

    use crate::system::types::is_near_identity;
    let is_rot_a_identity = is_near_identity(rot_a);
    let is_rot_b_identity = is_near_identity(rot_b);

    let manifold = detect_pair(
        &col_a.shape,
        pos_a,
        rot_a,
        is_rot_a_identity,
        &col_b.shape,
        pos_b,
        rot_b,
        is_rot_b_identity,
    );

    let (manifold, remaining_time) = if !manifold.is_colliding && (rb_a.ccd_enabled || rb_b.ccd_enabled) {
        let v_a_lin = velocities.get(ent_a).map(|v| v.linear).unwrap_or(Vec3::ZERO);
        let v_b_lin = velocities.get(ent_b).map(|v| v.linear).unwrap_or(Vec3::ZERO);
        let v_a_ang = velocities.get(ent_a).map(|v| v.angular).unwrap_or(Vec3::ZERO);
        let v_b_ang = velocities.get(ent_b).map(|v| v.angular).unwrap_or(Vec3::ZERO);
        let rel_v = v_b_lin - v_a_lin;

        if rel_v.length() * dt > ccd_velocity_threshold {
            let res = ccd_bisect(
                crate::system::ccd::CcdInput { shape: &col_a.shape, pos: pos_a, rot: rot_a, vel_lin: v_a_lin, vel_ang: v_a_ang },
                crate::system::ccd::CcdInput { shape: &col_b.shape, pos: pos_b, rot: rot_b, vel_lin: v_b_lin, vel_ang: v_b_ang },
                dt,
            );
            if res.manifold.is_colliding {
                ccd_pos_a = res.ccd_offset_a;
                ccd_pos_b = res.ccd_offset_b;
            }
            (res.manifold, res.remaining_time)
        } else {
            (manifold, dt)
        }
    } else {
        (manifold, dt)
    };

    if !manifold.is_colliding || manifold.contact_points.is_empty() {
        return None;
    }

    let inv_mass_a = if rb_a.mass == 0.0 {
        0.0
    } else {
        1.0 / rb_a.mass
    };
    let inv_mass_b = if rb_b.mass == 0.0 {
        0.0
    } else {
        1.0 / rb_b.mass
    };

    let mut wakes = Vec::new();
    if rb_a.is_sleeping && rb_a.mass > 0.0 {
        wakes.push(ent_a);
    }
    if rb_b.is_sleeping && rb_b.mass > 0.0 {
        wakes.push(ent_b);
    }

    let mut result = DetectionResult {
        contacts: Vec::new(),
        wake_entities: wakes,
    };

    for (contact_point, pen) in &manifold.contact_points {
        let mut r_a = *contact_point - pos_a;
        let mut r_b = *contact_point - pos_b;
        if let ColliderShape::Sphere(s) = &col_a.shape {
            r_a = manifold.normal * s.radius;
        }
        if let ColliderShape::Sphere(s) = &col_b.shape {
            // Doğrusu: B'nin merkezi temas noktasına giderken negatif normali takip eder.
            r_b = -manifold.normal * s.radius;
        }
        result.contacts.push(StoredContact {
            ent_a,
            ent_b,
            normal: manifold.normal,
            inv_mass_a,
            inv_mass_b,
            inv_inertia_a: rb_a.inverse_inertia_local,
            inv_inertia_b: rb_b.inverse_inertia_local,
            restitution: rb_a.restitution.min(rb_b.restitution),
            friction: (rb_a.friction * rb_b.friction).sqrt(),
            penetration: *pen,
            r_a,
            r_b,
            rot_a: t_a.rotation,
            rot_b: t_b.rotation,
            accumulated_j: 0.0,
            accumulated_friction: Vec3::ZERO,
            ccd_offset_a: ccd_pos_a.unwrap_or(Vec3::ZERO),
            ccd_offset_b: ccd_pos_b.unwrap_or(Vec3::ZERO),
            bias_bounce: 0.0,
            world_point: *contact_point,
            remaining_time,
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
    dt: f32,
    parallel_narrow_phase: bool,
    ccd_velocity_threshold: f32,
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
                dt,
                ccd_velocity_threshold,
            ) {
                acc = merge_detection_results(acc, item);
            }
        }
        return acc;
    }

    // NOT: Paralel reduce (par_iter().reduce) kullanıldığında işletim sisteminin thread bitirme 
    // zamanlamasına bağlı olarak contact listesi içine objelerin eklenme sırası farklılık gösterebilir.
    // Bu durum, Solver içerisindeki Warm-Start cache mekanizmasının index'lere dayalı eşleştirme yapması 
    // durumunda non-deterministic (her çalışmada ufak farklar yaratan) bir jitter oluşturur.
    // Tamamen deterministic (tekrar edilebilir) sonuçlar isteniyorsa `PhysicsConfig::deterministic_simulation = true` 
    // ayarlanarak bu paralel ağın bypass edilmesi gereklidir.
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
                dt,
                ccd_velocity_threshold,
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
        (Capsule(c), Aabb(a)) if rot_b_identity => {
            crate::collision::check_capsule_aabb_manifold(pos_a, rot_a, c, pos_b, a)
        }
        (Aabb(a), Capsule(c)) if rot_a_identity => {
            let mut m = crate::collision::check_capsule_aabb_manifold(pos_b, rot_b, c, pos_a, a);
            m.normal *= -1.0;
            m
        }
        (Sphere(s1), Sphere(s2)) => {
            crate::collision::check_sphere_sphere_manifold(pos_a, s1, pos_b, s2)
        }
        (HeightField { .. }, _) | (_, HeightField { .. }) => {
            #[cfg(debug_assertions)]
            eprintln!("[Physics WARN] HeightField çarpışmaları henüz GJK ile çözülemez (Convex değil). Dar faz atlanıyor.");
            crate::collision::CollisionManifold {
                is_colliding: false,
                normal: Vec3::ZERO,
                penetration: 0.0,
                contact_points: arrayvec::ArrayVec::new(),
            }
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
                    contact_points: arrayvec::ArrayVec::new(),
                }
            }
        }
    }
}
