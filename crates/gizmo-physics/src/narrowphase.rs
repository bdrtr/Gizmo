use crate::collision::ContactPoint;
use crate::components::ColliderShape;
use gizmo_math::Vec3;
use crate::gjk::Gjk;

pub struct NarrowPhase;

impl NarrowPhase {
    /// Test sphere vs sphere collision
    pub fn sphere_sphere(
        pos_a: Vec3,
        radius_a: f32,
        pos_b: Vec3,
        radius_b: f32,
    ) -> Option<ContactPoint> {
        let delta = pos_b - pos_a;
        let distance_sq = delta.length_squared();
        let radius_sum = radius_a + radius_b;

        if distance_sq < radius_sum * radius_sum && distance_sq > 0.0001 {
            let distance = distance_sq.sqrt();
            let normal = delta / distance;
            let penetration = radius_sum - distance;

            Some(ContactPoint {
                point: pos_a + normal * (radius_a - penetration * 0.5),
                normal,
                penetration,
                local_point_a: normal * radius_a,
                local_point_b: -normal * radius_b,
                normal_impulse: 0.0,
                tangent_impulse: Vec3::ZERO,
            })
        } else {
            None
        }
    }

    /// Test sphere vs plane collision
    pub fn sphere_plane(
        sphere_pos: Vec3,
        sphere_radius: f32,
        plane_normal: Vec3,
        plane_distance: f32,
    ) -> Option<ContactPoint> {
        let distance = sphere_pos.dot(plane_normal) - plane_distance;

        if distance < sphere_radius {
            let penetration = sphere_radius - distance;
            let contact_point = sphere_pos - plane_normal * distance;

            Some(ContactPoint {
                point: contact_point,
                normal: plane_normal,
                penetration,
                local_point_a: -plane_normal * sphere_radius,
                local_point_b: Vec3::ZERO,
                normal_impulse: 0.0,
                tangent_impulse: Vec3::ZERO,
            })
        } else {
            None
        }
    }

    /// Test box vs plane collision
    pub fn box_plane(
        box_pos: Vec3,
        box_rot: gizmo_math::Quat,
        box_extents: Vec3,
        plane_normal: Vec3,
        plane_distance: f32,
    ) -> Option<ContactPoint> {
        let corners = [
            Vec3::new(box_extents.x, box_extents.y, box_extents.z),
            Vec3::new(-box_extents.x, box_extents.y, box_extents.z),
            Vec3::new(box_extents.x, -box_extents.y, box_extents.z),
            Vec3::new(box_extents.x, box_extents.y, -box_extents.z),
            Vec3::new(-box_extents.x, -box_extents.y, box_extents.z),
            Vec3::new(-box_extents.x, box_extents.y, -box_extents.z),
            Vec3::new(box_extents.x, -box_extents.y, -box_extents.z),
            Vec3::new(-box_extents.x, -box_extents.y, -box_extents.z),
        ];

        let mut min_dist = f32::MAX;
        let mut deepest_corner = Vec3::ZERO;

        for corner in &corners {
            let world_corner = box_pos + box_rot * (*corner);
            let dist = world_corner.dot(plane_normal) - plane_distance;
            if dist < min_dist {
                min_dist = dist;
                deepest_corner = world_corner;
            }
        }

        if min_dist < 0.0 {
            Some(ContactPoint {
                point: deepest_corner - plane_normal * min_dist,
                normal: plane_normal,
                penetration: -min_dist,
                local_point_a: box_rot.inverse() * (deepest_corner - box_pos),
                local_point_b: Vec3::ZERO,
                normal_impulse: 0.0,
                tangent_impulse: Vec3::ZERO,
            })
        } else {
            None
        }
    }

    /// Main collision detection dispatcher
    pub fn test_collision(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> Option<ContactPoint> {
        if let ColliderShape::Compound(compound_a) = shape_a {
            let mut deepest_contact = None;
            let mut max_penetration = -f32::MAX;
            for (local_t, sub_shape) in compound_a {
                let world_pos = pos_a + rot_a.mul_vec3(local_t.position);
                let world_rot = rot_a * local_t.rotation;
                if let Some(contact) = Self::test_collision(sub_shape, world_pos, world_rot, shape_b, pos_b, rot_b) {
                    if contact.penetration > max_penetration {
                        max_penetration = contact.penetration;
                        deepest_contact = Some(contact);
                    }
                }
            }
            return deepest_contact;
        }

        if let ColliderShape::Compound(compound_b) = shape_b {
            let mut deepest_contact = None;
            let mut max_penetration = -f32::MAX;
            for (local_t, sub_shape) in compound_b {
                let world_pos = pos_b + rot_b.mul_vec3(local_t.position);
                let world_rot = rot_b * local_t.rotation;
                if let Some(contact) = Self::test_collision(shape_a, pos_a, rot_a, sub_shape, world_pos, world_rot) {
                    if contact.penetration > max_penetration {
                        max_penetration = contact.penetration;
                        deepest_contact = Some(contact);
                    }
                }
            }
            return deepest_contact;
        }

        // Use specialized tests for common cases
        match (shape_a, shape_b) {
            (ColliderShape::Sphere(sa), ColliderShape::Sphere(sb)) => {
                Self::sphere_sphere(pos_a, sa.radius, pos_b, sb.radius)
            }
            (ColliderShape::Sphere(s), ColliderShape::Plane(p)) => {
                Self::sphere_plane(pos_a, s.radius, p.normal, p.distance)
                    .map(|mut contact| {
                        contact.normal = -contact.normal; // Normal must point from A to B
                        contact
                    })
            }
            (ColliderShape::Plane(p), ColliderShape::Sphere(s)) => {
                Self::sphere_plane(pos_b, s.radius, p.normal, p.distance)
            }
            (ColliderShape::Box(b), ColliderShape::Plane(p)) => {
                Self::box_plane(pos_a, rot_a, b.half_extents, p.normal, p.distance)
                    .map(|mut contact| {
                        contact.normal = -contact.normal; // Normal must point from A to B
                        contact
                    })
            }
            (ColliderShape::Plane(p), ColliderShape::Box(b)) => {
                Self::box_plane(pos_b, rot_b, b.half_extents, p.normal, p.distance)
            }
            // Use GJK for other cases
            _ => Gjk::get_contact(shape_a, pos_a, rot_a, shape_b, pos_b, rot_b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sphere_sphere_collision() {
        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let pos_b = Vec3::new(1.5, 0.0, 0.0);

        let contact = NarrowPhase::sphere_sphere(pos_a, 1.0, pos_b, 1.0);
        assert!(contact.is_some());

        let contact = contact.unwrap();
        assert!(contact.penetration > 0.0);
        assert!((contact.normal - Vec3::new(1.0, 0.0, 0.0)).length() < 0.001);
    }

    #[test]
    fn test_sphere_sphere_no_collision() {
        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let pos_b = Vec3::new(3.0, 0.0, 0.0);

        let contact = NarrowPhase::sphere_sphere(pos_a, 1.0, pos_b, 1.0);
        assert!(contact.is_none());
    }

    #[test]
    fn test_box_plane_collision() {
        let box_shape = ColliderShape::Box(BoxShape { half_extents: Vec3::splat(1.0) });
        let plane_shape = ColliderShape::Plane(crate::components::PlaneShape { normal: Vec3::new(0.0, 1.0, 0.0), distance: 0.0 });
        
        let pos_box = Vec3::new(0.0, 0.5, 0.0); // Box center is at 0.5, half extent is 1.0, so lowest point is -0.5
        let rot_box = gizmo_math::Quat::IDENTITY;
        
        let pos_plane = Vec3::ZERO;
        let rot_plane = gizmo_math::Quat::IDENTITY;
        
        let contact = NarrowPhase::test_collision(&box_shape, pos_box, rot_box, &plane_shape, pos_plane, rot_plane);
        
        assert!(contact.is_some());
        let c = contact.unwrap();
        
        // Penetration should be 0.5
        assert!((c.penetration - 0.5).abs() < 0.001);
        assert!((c.normal - Vec3::new(0.0, -1.0, 0.0)).length() < 0.001);
    }

    #[test]
    fn test_capsule_sphere_gjk_collision() {
        let capsule = ColliderShape::Capsule(CapsuleShape { radius: 0.5, half_height: 1.0 });
        let sphere = ColliderShape::Sphere(SphereShape { radius: 0.5 });
        
        let pos_cap = Vec3::new(0.0, 0.0, 0.0);
        let rot_cap = gizmo_math::Quat::IDENTITY;
        
        let pos_sph = Vec3::new(0.8, 1.0, 0.0); // Penetrates capsule top sphere
        let rot_sph = gizmo_math::Quat::IDENTITY;
        
        let colliding = Gjk::test_collision(&capsule, pos_cap, rot_cap, &sphere, pos_sph, rot_sph);
        assert!(colliding);
        
        let contact = Gjk::get_contact(&capsule, pos_cap, rot_cap, &sphere, pos_sph, rot_sph);
        assert!(contact.is_some());
    }

    #[test]
    fn test_gjk_sphere_collision() {
        let shape_a = ColliderShape::Sphere(SphereShape { radius: 1.0 });
        let shape_b = ColliderShape::Sphere(SphereShape { radius: 1.0 });

        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let pos_b = Vec3::new(1.5, 0.0, 0.0);
        let rot = gizmo_math::Quat::IDENTITY;

        let colliding = Gjk::test_collision(&shape_a, pos_a, rot, &shape_b, pos_b, rot);
        assert!(colliding);
    }

    #[test]
    fn test_conservative_advancement_toi() {
        let shape_a = ColliderShape::Sphere(SphereShape { radius: 1.0 });
        let shape_b = ColliderShape::Sphere(SphereShape { radius: 1.0 });

        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let vel_a = Vec3::new(10.0, 0.0, 0.0); // Moving fast to the right
        
        let pos_b = Vec3::new(5.0, 0.0, 0.0);
        let vel_b = Vec3::new(0.0, 0.0, 0.0); // Stationary
        
        let rot = gizmo_math::Quat::IDENTITY;
        
        // Initial distance is 5.0 (centers). Radii sum is 2.0. So distance between surfaces is 3.0.
        // A is moving at 10.0 m/s. It should cover 3.0 meters in 0.3 seconds.
        let toi = Gjk::conservative_advancement(&shape_a, pos_a, rot, vel_a, &shape_b, pos_b, rot, vel_b, 1.0);
        
        assert!(toi.is_some(), "Collision should be detected within 1.0 seconds");
        let t = toi.unwrap();
        
        assert!((t - 0.3).abs() < 0.01, "Expected TOI to be ~0.3s, got {}", t);
    }

    #[test]
    fn test_conservative_advancement_no_collision() {
        let shape_a = ColliderShape::Sphere(SphereShape { radius: 1.0 });
        let shape_b = ColliderShape::Sphere(SphereShape { radius: 1.0 });

        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let vel_a = Vec3::new(0.0, 10.0, 0.0); // Moving UP
        
        let pos_b = Vec3::new(5.0, 0.0, 0.0);
        let vel_b = Vec3::new(0.0, 0.0, 0.0); // Stationary
        
        let rot = gizmo_math::Quat::IDENTITY;
        
        // They will never hit each other
        let toi = Gjk::conservative_advancement(&shape_a, pos_a, rot, vel_a, &shape_b, pos_b, rot, vel_b, 1.0);
        
        assert!(toi.is_none(), "Collision should NOT be detected");
    }
}
