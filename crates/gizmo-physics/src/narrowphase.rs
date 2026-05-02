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

    /// Compute distance and closest points using GJK (for non-intersecting shapes)
    /// Returns (distance, normal_from_b_to_a)
    pub fn distance<F>(support: F) -> Option<(f32, Vec3)>
    where
        F: Fn(Vec3) -> Vec3,
    {
        let mut simplex = Vec::with_capacity(4);
        let mut direction = Vec3::new(1.0, 0.0, 0.0);
        
        let p = support(direction);
        if p.length_squared() < 1e-6 {
            return Some((0.0, Vec3::ZERO));
        }
        
        simplex.push(p);
        direction = -p;

        let mut min_dist = f32::MAX;
        let mut closest_point = p;

        for _ in 0..EPA_MAX_ITERATIONS {
            let a = support(direction);
            
            // The distance projected along the direction
            // If it doesn't pass the origin, origin is definitely outside
            let d = a.dot(direction) / direction.length();
            if d < 0.0 {
                return Some((closest_point.length(), closest_point.normalize()));
            }

            simplex.push(a);

            if Self::handle_simplex(&mut simplex, &mut direction) {
                return Some((0.0, Vec3::ZERO)); // Intersecting
            }
            
            // Find the closest point on the new simplex to the origin
            // (direction already points to the origin from the new closest feature)
            let current_closest = -direction;
            let current_dist = current_closest.length();
            
            if current_dist < min_dist {
                min_dist = current_dist;
                closest_point = current_closest;
            } else {
                // If we didn't get closer, we converged
                return Some((min_dist, closest_point.normalize()));
            }
        }
        
        Some((min_dist, closest_point.normalize()))
    }

    /// Exact TOI (Time of Impact) using Conservative Advancement
    pub fn conservative_advancement(
        shape_a: &ColliderShape,
        mut pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        vel_a: Vec3,
        shape_b: &ColliderShape,
        mut pos_b: Vec3,
        rot_b: gizmo_math::Quat,
        vel_b: Vec3,
        max_t: f32,
    ) -> Option<f32> {
        let mut t = 0.0;
        let rel_vel = vel_a - vel_b;
        
        // If relative velocity is near zero, they won't collide dynamically in this frame
        if rel_vel.length_squared() < 1e-6 {
            return None;
        }

        for _ in 0..32 { // Max CA iterations
            let support = |dir: Vec3| {
                let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
                let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
                sa - sb
            };

            if let Some((dist, normal)) = Self::distance(support) {
                // If they are intersecting or extremely close, we found the TOI
                if dist < 0.001 {
                    return Some(t);
                }

                // Project relative velocity onto the shortest distance normal
                // Normal points from B to A. We want closing velocity.
                let closing_vel = -rel_vel.dot(normal);

                // If they are moving apart along this axis, no collision will occur
                if closing_vel <= 0.0 {
                    return None;
                }

                // Safely advance time by how long it takes to cover the distance at closing speed
                let delta_t = dist / closing_vel;
                t += delta_t;

                // If advanced past our time step, no collision in this frame
                if t > max_t {
                    return None;
                }

                // Advance positions for the next iteration
                pos_a += vel_a * delta_t;
                pos_b += vel_b * delta_t;
            } else {
                return None; // GJK failed
            }
        }

        Some(t) // Reached max iterations, assume collision at t
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
            let mut cross = ab.cross(ao);
            if cross.length_squared() < 1e-6 {
                cross = if ab.x.abs() > ab.y.abs() {
                    Vec3::new(ab.y, -ab.x, 0.0)
                } else {
                    Vec3::new(0.0, ab.z, -ab.y)
                };
            }
            *direction = cross.cross(ab);
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
                let mut cross = ac.cross(ao);
                if cross.length_squared() < 1e-6 {
                    cross = if ac.x.abs() > ac.y.abs() { Vec3::new(ac.y, -ac.x, 0.0) } else { Vec3::new(0.0, ac.z, -ac.y) };
                }
                *direction = cross.cross(ac);
            } else {
                simplex.remove(0);
                simplex.remove(0);
                let mut cross = ab.cross(ao);
                if cross.length_squared() < 1e-6 {
                    cross = if ab.x.abs() > ab.y.abs() { Vec3::new(ab.y, -ab.x, 0.0) } else { Vec3::new(0.0, ab.z, -ab.y) };
                }
                *direction = cross.cross(ab);
            }
        } else if ab.cross(abc).dot(ao) > 0.0 {
            simplex.remove(0);
            simplex.remove(0);
            let mut cross = ab.cross(ao);
            if cross.length_squared() < 1e-6 {
                cross = if ab.x.abs() > ab.y.abs() { Vec3::new(ab.y, -ab.x, 0.0) } else { Vec3::new(0.0, ab.z, -ab.y) };
            }
            *direction = cross.cross(ab);
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
            let (_closest_face_idx, normal, distance) = Self::find_closest_face(&simplex, &faces)?;

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
            ColliderShape::TriMesh(tm) => {
                let mut best_dot = f32::NEG_INFINITY;
                let mut best_pt = Vec3::ZERO;
                
                if !tm.bvh.nodes.is_empty() {
                    let mut stack = Vec::with_capacity(64);
                    stack.push(0);
                    
                    let abs_dir = gizmo_math::Vec3A::new(local_dir.x.abs(), local_dir.y.abs(), local_dir.z.abs());
                    let dir_a = gizmo_math::Vec3A::new(local_dir.x, local_dir.y, local_dir.z);
                    
                    while let Some(node_idx) = stack.pop() {
                        let node = &tm.bvh.nodes[node_idx];
                        
                        // Calculate maximum possible dot product for this AABB
                        let center = node.aabb.center();
                        let half_extents = node.aabb.half_extents();
                        
                        let max_node_dot = center.dot(dir_a) 
                                         + half_extents.x * abs_dir.x 
                                         + half_extents.y * abs_dir.y 
                                         + half_extents.z * abs_dir.z;
                                         
                        if max_node_dot < best_dot {
                            continue; // Prune this branch
                        }
                        
                        if node.is_leaf() {
                            let start = (node.first_tri_index * 3) as usize;
                            let end = start + (node.tri_count * 3) as usize;
                            for i in start..end {
                                let v = tm.vertices[tm.indices[i] as usize];
                                let d = v.dot(local_dir);
                                if d > best_dot {
                                    best_dot = d;
                                    best_pt = v;
                                }
                            }
                        } else {
                            if node.left_child >= 0 { stack.push(node.left_child as usize); }
                            if node.right_child >= 0 { stack.push(node.right_child as usize); }
                        }
                    }
                } else {
                    for v in &tm.vertices {
                        let d = v.dot(local_dir);
                        if d > best_dot {
                            best_dot = d;
                            best_pt = *v;
                        }
                    }
                }
                best_pt
            }
            ColliderShape::ConvexHull(ch) => {
                let mut best_dot = f32::NEG_INFINITY;
                let mut best_pt = Vec3::ZERO;
                for v in &ch.vertices {
                    let d = v.dot(local_dir);
                    if d > best_dot {
                        best_dot = d;
                        best_pt = *v;
                    }
                }
                best_pt
            }
            crate::components::ColliderShape::Compound(_) => {
                // Approximate fallback since Compound shapes shouldn't be used directly with GJK support
                Vec3::ZERO
            }
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
        assert!((c.normal - Vec3::new(0.0, 1.0, 0.0)).length() < 0.001);
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
