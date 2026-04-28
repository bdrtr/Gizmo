use crate::collision::ContactPoint;
use crate::components::{BoxShape, CapsuleShape, ColliderShape, SphereShape};
use gizmo_math::Vec3;

const EPA_TOLERANCE: f32 = 0.001;
const EPA_MAX_ITERATIONS: usize = 32;

/// GJK (Gilbert-Johnson-Keerthi) algorithm for collision detection
pub struct Gjk;

impl Gjk {
    /// Test if two shapes are colliding using GJK
    pub fn test_collision(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> bool {
        let support = |dir: Vec3| {
            let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
            let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
            sa - sb
        };

        Self::gjk_algorithm(support)
    }

    /// Get contact information using GJK + EPA
    pub fn get_contact(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> Option<ContactPoint> {
        let support = |dir: Vec3| {
            let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
            let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
            sa - sb
        };

        if let Some(simplex) = Self::gjk_with_simplex(support) {
            Self::epa(simplex, support, pos_a, pos_b)
        } else {
            None
        }
    }

    /// Core GJK algorithm
    fn gjk_algorithm<F>(support: F) -> bool
    where
        F: Fn(Vec3) -> Vec3,
    {
        Self::gjk_with_simplex(support).is_some()
    }

    /// GJK that returns the final simplex for EPA
    fn gjk_with_simplex<F>(support: F) -> Option<Vec<Vec3>>
    where
        F: Fn(Vec3) -> Vec3,
    {
        let mut simplex = Vec::with_capacity(4);
        let mut direction = Vec3::new(1.0, 0.0, 0.0);

        // First point
        simplex.push(support(direction));
        direction = -simplex[0];

        const MAX_ITERATIONS: usize = 32;
        for _ in 0..MAX_ITERATIONS {
            let a = support(direction);

            if a.dot(direction) < 0.0 {
                return None; // No collision
            }

            simplex.push(a);

            if Self::handle_simplex(&mut simplex, &mut direction) {
                return Some(simplex); // Collision detected
            }
        }

        None
    }

    /// Handle simplex evolution in GJK
    fn handle_simplex(simplex: &mut Vec<Vec3>, direction: &mut Vec3) -> bool {
        match simplex.len() {
            2 => Self::line_case(simplex, direction),
            3 => Self::triangle_case(simplex, direction),
            4 => Self::tetrahedron_case(simplex, direction),
            _ => false,
        }
    }

    fn line_case(simplex: &mut Vec<Vec3>, direction: &mut Vec3) -> bool {
        let a = simplex[1];
        let b = simplex[0];

        let ab = b - a;
        let ao = -a;

        if ab.dot(ao) > 0.0 {
            *direction = ab.cross(ao).cross(ab);
        } else {
            simplex.remove(0);
            *direction = ao;
        }

        false
    }

    fn triangle_case(simplex: &mut Vec<Vec3>, direction: &mut Vec3) -> bool {
        let a = simplex[2];
        let b = simplex[1];
        let c = simplex[0];

        let ab = b - a;
        let ac = c - a;
        let ao = -a;

        let abc = ab.cross(ac);

        if abc.cross(ac).dot(ao) > 0.0 {
            if ac.dot(ao) > 0.0 {
                simplex.remove(1);
                *direction = ac.cross(ao).cross(ac);
            } else {
                simplex.remove(0);
                simplex.remove(0);
                *direction = ab.cross(ao).cross(ab);
            }
        } else if ab.cross(abc).dot(ao) > 0.0 {
            simplex.remove(0);
            simplex.remove(0);
            *direction = ab.cross(ao).cross(ab);
        } else {
            if abc.dot(ao) > 0.0 {
                *direction = abc;
            } else {
                simplex.swap(0, 1);
                *direction = -abc;
            }
        }

        false
    }

    fn tetrahedron_case(simplex: &mut Vec<Vec3>, direction: &mut Vec3) -> bool {
        let a = simplex[3];
        let b = simplex[2];
        let c = simplex[1];
        let d = simplex[0];

        let ab = b - a;
        let ac = c - a;
        let ad = d - a;
        let ao = -a;

        let abc = ab.cross(ac);
        let acd = ac.cross(ad);
        let adb = ad.cross(ab);

        // Check which face the origin is closest to
        if abc.dot(ao) > 0.0 {
            simplex.remove(0);
            return Self::triangle_case(simplex, direction);
        }

        if acd.dot(ao) > 0.0 {
            simplex.remove(2);
            simplex.swap(0, 1);
            return Self::triangle_case(simplex, direction);
        }

        if adb.dot(ao) > 0.0 {
            simplex.remove(1);
            return Self::triangle_case(simplex, direction);
        }

        true // Origin is inside tetrahedron
    }

    /// EPA (Expanding Polytope Algorithm) for contact information
    fn epa<F>(
        mut simplex: Vec<Vec3>,
        support: F,
        pos_a: Vec3,
        pos_b: Vec3,
    ) -> Option<ContactPoint>
    where
        F: Fn(Vec3) -> Vec3,
    {
        let mut faces = Vec::new();
        let mut edges = Vec::new();

        // Initialize with tetrahedron faces
        if simplex.len() == 4 {
            faces.push((0, 1, 2));
            faces.push((0, 3, 1));
            faces.push((0, 2, 3));
            faces.push((1, 3, 2));
        } else {
            return None;
        }

        for _ in 0..EPA_MAX_ITERATIONS {
            // Find closest face
            let (closest_face_idx, normal, distance) = Self::find_closest_face(&simplex, &faces)?;

            if distance < EPA_TOLERANCE {
                break;
            }

            // Get support point in normal direction
            let support_point = support(normal);

            // Check if we can expand further
            let support_distance = support_point.dot(normal);
            if support_distance - distance < EPA_TOLERANCE {
                break;
            }

            // Add new point and rebuild polytope
            let new_point_idx = simplex.len();
            simplex.push(support_point);

            // Remove faces visible from new point and collect edges
            edges.clear();
            let mut i = 0;
            while i < faces.len() {
                let (a, b, c) = faces[i];
                let face_normal = Self::compute_face_normal(&simplex, a, b, c);
                let to_point = simplex[new_point_idx] - simplex[a];

                if face_normal.dot(to_point) > 0.0 {
                    Self::add_edge(&mut edges, a, b);
                    Self::add_edge(&mut edges, b, c);
                    Self::add_edge(&mut edges, c, a);
                    faces.swap_remove(i);
                } else {
                    i += 1;
                }
            }

            // Create new faces from edges to new point
            for (e1, e2) in &edges {
                faces.push((*e1, *e2, new_point_idx));
            }
        }

        // Get final contact information
        let (_, normal, penetration) = Self::find_closest_face(&simplex, &faces)?;

        let contact_point = pos_a + normal * (penetration * 0.5);

        Some(ContactPoint {
            point: contact_point,
            normal,
            penetration,
            local_point_a: contact_point - pos_a,
            local_point_b: contact_point - pos_b,
            normal_impulse: 0.0,
            tangent_impulse: Vec3::ZERO,
        })
    }

    fn find_closest_face(
        simplex: &[Vec3],
        faces: &[(usize, usize, usize)],
    ) -> Option<(usize, Vec3, f32)> {
        let mut min_distance = f32::INFINITY;
        let mut closest_idx = 0;
        let mut closest_normal = Vec3::ZERO;

        for (i, &(a, b, c)) in faces.iter().enumerate() {
            let normal = Self::compute_face_normal(simplex, a, b, c);
            let distance = normal.dot(simplex[a]).abs();

            if distance < min_distance {
                min_distance = distance;
                closest_idx = i;
                closest_normal = normal;
            }
        }

        if min_distance == f32::INFINITY {
            None
        } else {
            Some((closest_idx, closest_normal, min_distance))
        }
    }

    fn compute_face_normal(simplex: &[Vec3], a: usize, b: usize, c: usize) -> Vec3 {
        let ab = simplex[b] - simplex[a];
        let ac = simplex[c] - simplex[a];
        ab.cross(ac).normalize()
    }

    fn add_edge(edges: &mut Vec<(usize, usize)>, a: usize, b: usize) {
        // Remove edge if it already exists (it's shared), otherwise add it
        let reverse = (b, a);
        if let Some(pos) = edges.iter().position(|&e| e == reverse) {
            edges.swap_remove(pos);
        } else {
            edges.push((a, b));
        }
    }

    /// Get support point for a shape in a given direction
    fn support_point(
        shape: &ColliderShape,
        pos: Vec3,
        rot: gizmo_math::Quat,
        dir: Vec3,
    ) -> Vec3 {
        let local_dir = rot.inverse() * dir;

        let local_support = match shape {
            ColliderShape::Sphere(s) => Self::sphere_support(s, local_dir),
            ColliderShape::Box(b) => Self::box_support(b, local_dir),
            ColliderShape::Capsule(c) => Self::capsule_support(c, local_dir),
            ColliderShape::Plane(_) => Vec3::ZERO, // Planes handled separately
        };

        pos + rot * local_support
    }

    fn sphere_support(sphere: &SphereShape, dir: Vec3) -> Vec3 {
        dir.normalize() * sphere.radius
    }

    fn box_support(box_shape: &BoxShape, dir: Vec3) -> Vec3 {
        Vec3::new(
            if dir.x > 0.0 {
                box_shape.half_extents.x
            } else {
                -box_shape.half_extents.x
            },
            if dir.y > 0.0 {
                box_shape.half_extents.y
            } else {
                -box_shape.half_extents.y
            },
            if dir.z > 0.0 {
                box_shape.half_extents.z
            } else {
                -box_shape.half_extents.z
            },
        )
    }

    fn capsule_support(capsule: &CapsuleShape, dir: Vec3) -> Vec3 {
        let dir_normalized = dir.normalize();
        let sphere_center = if dir_normalized.y > 0.0 {
            Vec3::new(0.0, capsule.half_height, 0.0)
        } else {
            Vec3::new(0.0, -capsule.half_height, 0.0)
        };
        sphere_center + dir_normalized * capsule.radius
    }
}

/// Specialized collision tests for common cases (faster than GJK)
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

    /// Main collision detection dispatcher
    pub fn test_collision(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> Option<ContactPoint> {
        // Use specialized tests for common cases
        match (shape_a, shape_b) {
            (ColliderShape::Sphere(sa), ColliderShape::Sphere(sb)) => {
                Self::sphere_sphere(pos_a, sa.radius, pos_b, sb.radius)
            }
            (ColliderShape::Sphere(s), ColliderShape::Plane(p)) => {
                Self::sphere_plane(pos_a, s.radius, p.normal, p.distance)
            }
            (ColliderShape::Plane(p), ColliderShape::Sphere(s)) => {
                Self::sphere_plane(pos_b, s.radius, p.normal, p.distance)
                    .map(|mut contact| {
                        contact.normal = -contact.normal;
                        contact
                    })
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
    fn test_gjk_sphere_collision() {
        let shape_a = ColliderShape::Sphere(SphereShape { radius: 1.0 });
        let shape_b = ColliderShape::Sphere(SphereShape { radius: 1.0 });

        let pos_a = Vec3::new(0.0, 0.0, 0.0);
        let pos_b = Vec3::new(1.5, 0.0, 0.0);
        let rot = gizmo_math::Quat::IDENTITY;

        let colliding = Gjk::test_collision(&shape_a, pos_a, rot, &shape_b, pos_b, rot);
        assert!(colliding);
    }
}
