use crate::collision::ContactPoint;
use crate::components::{ColliderShape, SphereShape, BoxShape, CapsuleShape};
use gizmo_math::Vec3;

const EPA_TOLERANCE: f32 = 0.001;
const EPA_MAX_ITERATIONS: usize = 32;

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

        Self::gjk_with_simplex(support).is_some()
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
            Self::epa(simplex, shape_a, pos_a, rot_a, shape_b, pos_b, rot_b)
        } else {
            None
        }
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
            direction = direction.try_normalize().unwrap_or(Vec3::X);
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
            direction = direction.try_normalize().unwrap_or(Vec3::X);
            let a = support(direction);
            
            // The distance projected along the direction
            // If it doesn't pass the origin, origin is definitely outside
            let d = a.dot(direction);
            if d < 0.0 {
                return Some((closest_point.length(), closest_point.normalize()));
            }

            simplex.push(a);

            if Self::handle_simplex(&mut simplex, &mut direction) {
                return Some((0.0, Vec3::ZERO)); // Intersecting
            }
            
            // Find the closest point on the new simplex to the origin
            let current_closest = Self::closest_point_on_simplex(&simplex);
            let current_dist = current_closest.length();
            
            if current_dist < min_dist {
                min_dist = current_dist;
                closest_point = current_closest;
                // Update direction to point to origin from closest point
                if current_dist > 1e-6 {
                    direction = -closest_point;
                }
            } else {
                // If we didn't get closer, we converged
                let normal = if closest_point.length_squared() > 1e-8 { closest_point.normalize() } else { Vec3::X };
                return Some((min_dist, normal));
            }
        }
        
        let normal = if closest_point.length_squared() > 1e-8 { closest_point.normalize() } else { Vec3::X };
        Some((min_dist, normal))
    }

    fn closest_point_on_simplex(simplex: &[Vec3]) -> Vec3 {
        match simplex.len() {
            1 => simplex[0],
            2 => {
                let a = simplex[1];
                let b = simplex[0];
                let ab = b - a;
                let t = (-a).dot(ab) / ab.length_squared().max(1e-8);
                let t = t.clamp(0.0, 1.0);
                a + ab * t
            },
            3 => {
                let a = simplex[2];
                let b = simplex[1];
                let c = simplex[0];
                let ab = b - a;
                let ac = c - a;
                let normal = ab.cross(ac);
                
                // Project origin onto plane
                let t = (-a).dot(normal) / normal.length_squared().max(1e-8);
                let proj = normal * t;
                
                // For simplicity, return the projection. Technically we should check if it's inside the triangle.
                // Since handle_simplex already filters out non-voronoi regions, this is an acceptable approximation for distance.
                a + proj
            },
            4 => {
                // If it's a tetrahedron and we haven't exited, origin is inside
                Vec3::ZERO
            },
            _ => Vec3::ZERO,
        }
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

        None // Converge edemedik, CCD miss kabul et
    }

    /// Fast Speculative Contact for CCD
    pub fn speculative_contact(
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        vel_a: Vec3,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
        vel_b: Vec3,
        dt: f32,
    ) -> Option<ContactPoint> {
        let support = |dir: Vec3| {
            let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
            let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
            sa - sb
        };

        if let Some((dist, normal)) = Self::distance(support) {
            if dist <= 0.0 { return None; } // Already intersecting

            let rel_vel = vel_b - vel_a;
            let normal_a_to_b = -normal; // Gjk::distance returns vector from B to A. We need A to B.
            
            // Closing velocity: how fast B is moving towards A.
            let closing_vel = rel_vel.dot(normal); // Same as -rel_vel.dot(normal_a_to_b)
            
            if closing_vel > 0.0 && dist < closing_vel * dt {
                let contact_point = pos_a + normal_a_to_b * (dist * 0.5); 
                return Some(ContactPoint {
                    point: contact_point,
                    normal: normal_a_to_b,
                    penetration: -dist,
                    local_point_a: contact_point - pos_a,
                    local_point_b: contact_point - pos_b,
                    normal_impulse: 0.0,
                    tangent_impulse: Vec3::ZERO,
                });
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
                *simplex = vec![c, a];
                let mut cross = ac.cross(ao);
                if cross.length_squared() < 1e-6 {
                    cross = if ac.x.abs() > ac.y.abs() { Vec3::new(ac.y, -ac.x, 0.0) } else { Vec3::new(0.0, ac.z, -ac.y) };
                }
                *direction = cross.cross(ac);
            } else {
                *simplex = vec![b, a];
                let mut cross = ab.cross(ao);
                if cross.length_squared() < 1e-6 {
                    cross = if ab.x.abs() > ab.y.abs() { Vec3::new(ab.y, -ab.x, 0.0) } else { Vec3::new(0.0, ab.z, -ab.y) };
                }
                *direction = cross.cross(ab);
            }
        } else if ab.cross(abc).dot(ao) > 0.0 {
            *simplex = vec![b, a];
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
    fn epa(
        mut simplex: Vec<Vec3>,
        shape_a: &ColliderShape,
        pos_a: Vec3,
        rot_a: gizmo_math::Quat,
        shape_b: &ColliderShape,
        pos_b: Vec3,
        rot_b: gizmo_math::Quat,
    ) -> Option<ContactPoint>
    {
        let support = |dir: Vec3| {
            let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
            let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
            sa - sb
        };
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
        
        // dbg!(penetration, normal);

        let pt_a = Self::support_point(shape_a, pos_a, rot_a, -normal);
        let pt_b = Self::support_point(shape_b, pos_b, rot_b, -normal);
        let contact_point = (pt_a + pt_b) * 0.5;

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
            let distance = normal.dot(simplex[a]);

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
        let normal_raw = ab.cross(ac);
        if normal_raw.dot(simplex[a]) < 0.0 {
            -normal_raw.try_normalize().unwrap_or(Vec3::X)
        } else {
            normal_raw.try_normalize().unwrap_or(Vec3::X)
        }
    }

    fn add_edge(edges: &mut Vec<(usize, usize)>, a: usize, b: usize) {
        let reverse = (b, a);
        let forward = (a, b);
        
        // Remove edge if it already exists in reverse orientation (shared edge)
        if let Some(pos) = edges.iter().position(|&e| e == reverse) {
            edges.swap_remove(pos);
        } 
        // Also check if it exists in forward orientation (invalid geometry, but prevents explosions)
        else if let Some(pos) = edges.iter().position(|&e| e == forward) {
            edges.swap_remove(pos);
        } 
        else {
            edges.push((a, b));
        }
    }

    /// Get support point for a shape in a given direction
    pub fn support_point(
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
            ColliderShape::Plane(_) => {
                debug_assert!(false, "Plane shapes must use separate collision detection");
                Vec3::ZERO // Planes handled separately
            }
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
                    for v in tm.vertices.iter() {
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
                for v in ch.vertices.iter() {
                    let d = v.dot(local_dir);
                    if d > best_dot {
                        best_dot = d;
                        best_pt = *v;
                    }
                }
                best_pt
            }
            crate::components::ColliderShape::Compound(_) => {
                debug_assert!(false, "Compound shapes must use separate collision detection");
                Vec3::ZERO
            }
        };

        pos + rot * local_support
    }

    fn sphere_support(sphere: &SphereShape, dir: Vec3) -> Vec3 {
        dir.try_normalize().unwrap_or(Vec3::X) * sphere.radius
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
        let dir_normalized = dir.try_normalize().unwrap_or(Vec3::X);
        let sphere_center = if dir_normalized.y > 0.0 {
            Vec3::new(0.0, capsule.half_height, 0.0)
        } else {
            Vec3::new(0.0, -capsule.half_height, 0.0)
        };
        sphere_center + dir_normalized * capsule.radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::{Vec3, Quat};

    #[test]
    fn test_sphere_vs_sphere_collision() {
        let shape = ColliderShape::Sphere(SphereShape { radius: 1.0 });
        
        // Intersecting
        assert!(Gjk::test_collision(&shape, Vec3::ZERO, Quat::IDENTITY, &shape, Vec3::new(1.5, 0.0, 0.0), Quat::IDENTITY));
        
        // Not Intersecting
        assert!(!Gjk::test_collision(&shape, Vec3::ZERO, Quat::IDENTITY, &shape, Vec3::new(2.5, 0.0, 0.0), Quat::IDENTITY));
    }

    #[test]
    fn test_box_vs_box_collision() {
        let shape = ColliderShape::Box(BoxShape { half_extents: Vec3::new(1.0, 1.0, 1.0) });
        
        // Intersecting
        assert!(Gjk::test_collision(&shape, Vec3::ZERO, Quat::IDENTITY, &shape, Vec3::new(1.5, 0.0, 0.0), Quat::IDENTITY));
        
        // Not Intersecting
        assert!(!Gjk::test_collision(&shape, Vec3::ZERO, Quat::IDENTITY, &shape, Vec3::new(2.5, 0.0, 0.0), Quat::IDENTITY));
    }

    #[test]
    fn test_epa_contact_generation() {
        let shape_a = ColliderShape::Box(BoxShape { half_extents: Vec3::new(1.0, 1.0, 1.0) });
        let shape_b = ColliderShape::Box(BoxShape { half_extents: Vec3::new(1.0, 1.0, 1.0) });
        
        // Penetrating by 0.5
        let contact = Gjk::get_contact(&shape_a, Vec3::ZERO, Quat::IDENTITY, &shape_b, Vec3::new(1.5, 0.0, 0.0), Quat::IDENTITY);
        
        assert!(contact.is_some(), "EPA failed to generate contact");
        let contact = contact.unwrap();
        
        println!("Test got contact: {:?}", contact);
        
        // Penetration should be 0.5 (1.0 + 1.0 - 1.5)
        assert!((contact.penetration - 0.5).abs() < 0.001, "Penetration depth is wrong: {}", contact.penetration);
        assert!((contact.normal.x.abs() - 1.0).abs() < 0.001, "Normal is wrong: {:?}", contact.normal);
    }
}
