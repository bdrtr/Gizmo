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
        if pos_a.x.abs() < 10.0 && pos_b.x.abs() < 10.0 {
        }
        let support = |dir: Vec3| {
            let sa = Self::support_point(shape_a, pos_a, rot_a, dir);
            let sb = Self::support_point(shape_b, pos_b, rot_b, -dir);
            sa - sb
        };

        let res = if let Some(simplex) = Self::gjk_with_simplex(support) {
            if let Some(contact) = Self::epa(simplex, shape_a, pos_a, rot_a, shape_b, pos_b, rot_b) {
                Some(contact)
            } else {
                // EPA failed (likely degenerate simplex), but we KNOW they intersect!
                // Return a basic contact point for triggers and solver fallback
                Some(ContactPoint {
                    point: (pos_a + pos_b) * 0.5,
                    normal: (pos_b - pos_a).try_normalize().unwrap_or(Vec3::Y),
                    penetration: 0.01,
                    ..Default::default()
                })
            }
        } else {
            None
        };
        
        if pos_a.x.abs() < 10.0 && pos_b.x.abs() < 10.0 {
        }
        res
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
        let mut direction = Vec3::X;
        
        let p = support(direction);
        simplex.push(p);
        
        let mut closest_point = p;
        let mut min_dist_sq = p.length_squared();

        for _ in 0..32 {
            if min_dist_sq < 1e-8 {
                return Some((0.0, Vec3::X)); // Intersecting, use fallback normal
            }

            direction = -closest_point;
            let a = support(direction.normalize());
            
            // Convergence check: how much closer can we get in this direction?
            let current_dist = min_dist_sq.sqrt();
            let lower_bound = -a.dot(direction) / current_dist;
            if current_dist - lower_bound < 0.0001 {
                break;
            }

            simplex.push(a);
            closest_point = Self::closest_point_on_simplex(&mut simplex);
            min_dist_sq = closest_point.length_squared();
        }
        
        let dist = min_dist_sq.sqrt();
        let normal = if dist > 1e-6 { closest_point / dist } else { Vec3::X };
        Some((dist, normal))
    }

    fn closest_point_on_simplex(simplex: &mut Vec<Vec3>) -> Vec3 {
        match simplex.len() {
            1 => simplex[0],
            2 => {
                let b = simplex[0];
                let a = simplex[1];
                let ab = b - a;
                let ao = -a;
                let t = ao.dot(ab) / ab.length_squared().max(1e-8);
                if t <= 0.0 {
                    simplex.remove(0); // Keep A
                    a
                } else if t >= 1.0 {
                    simplex.remove(1); // Keep B
                    b
                } else {
                    a + ab * t
                }
            },
            3 => {
                let c = simplex[0];
                let b = simplex[1];
                let a = simplex[2];
                
                let ab = b - a;
                let ac = c - a;
                let ap = -a;

                // Check vertex regions
                let d1 = ab.dot(ap);
                let d2 = ac.dot(ap);
                if d1 <= 0.0 && d2 <= 0.0 {
                    *simplex = vec![a];
                    return a;
                }

                let bp = -b;
                let d3 = ab.dot(bp);
                let d4 = ac.dot(bp);
                if d3 >= 0.0 && d4 <= d3 {
                    *simplex = vec![b];
                    return b;
                }

                let cp = -c;
                let d5 = ab.dot(cp);
                let d6 = ac.dot(cp);
                if d6 >= 0.0 && d5 <= d6 {
                    *simplex = vec![c];
                    return c;
                }

                // Check edge regions
                let vc = d1 * d4 - d3 * d2;
                if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
                    let v = d1 / (d1 - d3);
                    *simplex = vec![b, a];
                    return a + ab * v;
                }

                let vb = d5 * d2 - d1 * d6;
                if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                    let w = d2 / (d2 - d6);
                    *simplex = vec![c, a];
                    return a + ac * w;
                }

                let va = d3 * d6 - d5 * d4;
                if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
                    let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
                    *simplex = vec![c, b];
                    return b + (c - b) * w;
                }

                // Inside face region
                let denom = 1.0 / (va + vb + vc);
                let v = vb * denom;
                let w = vc * denom;
                a + ab * v + ac * w
            },
            4 => {
                // If we have 4 points, we're likely intersecting or about to be.
                // For simplicity in distance calculation, we check if origin is inside.
                // If not, we find the closest face and reduce to triangle case.
                let d = simplex[0];
                let c = simplex[1];
                let b = simplex[2];
                let a = simplex[3];
                
                let abc = (b - a).cross(c - a);
                let acd = (c - a).cross(d - a);
                let adb = (d - a).cross(b - a);
                let bdc = (c - b).cross(d - b);
                
                if abc.dot(-a) > 0.0 { *simplex = vec![c, b, a]; return Self::closest_point_on_simplex(simplex); }
                if acd.dot(-a) > 0.0 { *simplex = vec![d, c, a]; return Self::closest_point_on_simplex(simplex); }
                if adb.dot(-a) > 0.0 { *simplex = vec![b, d, a]; return Self::closest_point_on_simplex(simplex); }
                if bdc.dot(-b) > 0.0 { *simplex = vec![d, c, b]; return Self::closest_point_on_simplex(simplex); }
                
                Vec3::ZERO // Inside
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
    ) -> Option<(f32, Vec3)> {
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
                    return Some((t, normal));
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

    /// Exact CCD Sweep Test using Conservative Advancement
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
        if let Some((t, normal)) = Self::conservative_advancement(
            shape_a, pos_a, rot_a, vel_a,
            shape_b, pos_b, rot_b, vel_b,
            dt,
        ) {
            let rel_vel = vel_b - vel_a;
            let normal_a_to_b = -normal; // CA normal is B to A
            
            // Closing velocity (positive if approaching)
            let closing_speed = rel_vel.dot(normal); // rel_vel is B-A, normal is B to A
            if closing_speed > 0.0 {
                // Gap at t=0
                let gap = closing_speed * t;
                
                let hit_pos_a = pos_a + vel_a * t;
                let hit_pos_b = pos_b + vel_b * t;
                let contact_point = (hit_pos_a + hit_pos_b) * 0.5;
                
                return Some(ContactPoint {
                    point: contact_point,
                    normal: normal_a_to_b,
                    penetration: -gap, // Negative penetration for speculative constraint
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
        
        
        // Penetration should be 0.5 (1.0 + 1.0 - 1.5)
        assert!((contact.penetration - 0.5).abs() < 0.001, "Penetration depth is wrong: {}", contact.penetration);
        assert!((contact.normal.x.abs() - 1.0).abs() < 0.001, "Normal is wrong: {:?}", contact.normal);
    }
}
