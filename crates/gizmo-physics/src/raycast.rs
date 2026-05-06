use crate::components::{ColliderShape, Transform};
use gizmo_math::Aabb;
use gizmo_core::entity::Entity;
use gizmo_math::Vec3;

/// Ray for raycasting
#[derive(Debug, Clone, Copy)]
pub struct Ray {
    pub origin: Vec3,
    pub direction: Vec3, // Should be normalized
}

impl Ray {
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self {
            origin,
            direction: direction.normalize(),
        }
    }

    pub fn point_at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }
}

/// Result of a raycast hit
#[derive(Debug, Clone, Copy)]
pub struct RaycastHit {
    pub entity: Entity,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

/// Raycast query system
pub struct Raycast;

impl Raycast {
    /// Test ray against AABB
    pub fn ray_aabb(ray: &Ray, aabb: &Aabb) -> Option<f32> {
        let mut tmin: f32 = 0.0;
        let mut tmax = f32::INFINITY;

        for i in 0..3 {
            let origin = match i {
                0 => ray.origin.x,
                1 => ray.origin.y,
                _ => ray.origin.z,
            };
            let dir = match i {
                0 => ray.direction.x,
                1 => ray.direction.y,
                _ => ray.direction.z,
            };
            let min = match i {
                0 => aabb.min.x,
                1 => aabb.min.y,
                _ => aabb.min.z,
            };
            let max = match i {
                0 => aabb.max.x,
                1 => aabb.max.y,
                _ => aabb.max.z,
            };

            if dir.abs() < 1e-8 {
                // Ray is parallel to slab
                if origin < min || origin > max {
                    return None;
                }
            } else {
                let inv_d = 1.0 / dir;
                let mut t1 = (min - origin) * inv_d;
                let mut t2 = (max - origin) * inv_d;

                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }

                tmin = tmin.max(t1);
                tmax = tmax.min(t2);

                if tmin > tmax {
                    return None;
                }
            }
        }

        Some(tmin)
    }

    /// Test ray against sphere
    pub fn ray_sphere(ray: &Ray, center: Vec3, radius: f32) -> Option<(f32, Vec3)> {
        let oc = ray.origin - center;
        let b = oc.dot(ray.direction);
        let c = oc.dot(oc) - radius * radius;
        let discriminant = b * b - c;

        if discriminant < 0.0 {
            return None;
        }

        let sqrt_d = discriminant.sqrt();
        let t1 = -b - sqrt_d;
        let t2 = -b + sqrt_d;

        let t = if t1 > 0.0 {
            t1
        } else if t2 > 0.0 {
            t2
        } else {
            return None;
        };

        let hit_point = ray.point_at(t);
        let normal = (hit_point - center).try_normalize().unwrap_or(Vec3::Y);

        Some((t, normal))
    }

    /// Test ray against box (OBB)
    pub fn ray_box(
        ray: &Ray,
        center: Vec3,
        rotation: gizmo_math::Quat,
        half_extents: Vec3,
    ) -> Option<(f32, Vec3)> {
        // Transform ray to box's local space
        let inv_rot = rotation.inverse();
        let local_origin = inv_rot * (ray.origin - center);
        let local_dir = inv_rot * ray.direction;

        let local_ray = Ray::new(local_origin, local_dir);

        // Create AABB in local space
        let local_aabb = Aabb::from_center_half_extents(Vec3::ZERO, half_extents);

        if let Some(t) = Self::ray_aabb(&local_ray, &local_aabb) {
            let local_hit = local_ray.point_at(t);

            // Calculate normal in local space
            let mut normal = Vec3::ZERO;

            let epsilon = 1e-4;
            for i in 0..3 {
                if (local_hit[i] - half_extents[i]).abs() < epsilon { 
                    normal[i] = 1.0; 
                }
                if (local_hit[i] + half_extents[i]).abs() < epsilon { 
                    normal[i] = -1.0; 
                }
            }
            normal = normal.try_normalize().unwrap_or(Vec3::Y);

            // Transform normal back to world space
            let world_normal = rotation * normal;

            Some((t, world_normal))
        } else {
            None
        }
    }

    /// Test ray against capsule
    pub fn ray_capsule(
        ray: &Ray,
        center: Vec3,
        rotation: gizmo_math::Quat,
        radius: f32,
        half_height: f32,
    ) -> Option<(f32, Vec3)> {
        // Transform to local space
        let inv_rot = rotation.inverse();
        let local_origin = inv_rot * (ray.origin - center);
        let local_dir = inv_rot * ray.direction;

        // Capsule is aligned along Y axis in local space
        let p1 = Vec3::new(0.0, half_height, 0.0);
        let p2 = Vec3::new(0.0, -half_height, 0.0);

        // Ray-cylinder intersection
        let ba = p2 - p1;
        let oc = local_origin - p1;

        let baba = ba.dot(ba);
        let bard = ba.dot(local_dir);
        let baoc = ba.dot(oc);

        let k2 = baba - bard * bard;
        let k1 = baba * oc.dot(local_dir) - baoc * bard;
        let k0 = baba * oc.dot(oc) - baoc * baoc - radius * radius * baba;

        if k2.abs() >= 1e-8 {
            let h = k1 * k1 - k2 * k0;
            if h >= 0.0 {
                let t = (-k1 - h.sqrt()) / k2;
                // Check if hit is within cylinder height
                let y = baoc + t * bard;
                if y > 0.0 && y < baba {
                    let hit_point = local_origin + local_dir * t;
                    let normal = (hit_point - (p1 + ba * (y / baba))).try_normalize().unwrap_or(Vec3::Y);
                    let world_normal = rotation * normal;
                    return Some((t, world_normal));
                }
            }
        }

        // Check sphere caps
        let mut best_t = f32::INFINITY;
        let mut best_normal = Vec3::ZERO;

        for &cap_center in &[p1, p2] {
            let oc = local_origin - cap_center;
            let a = local_dir.dot(local_dir);
            let b = 2.0 * oc.dot(local_dir);
            let c = oc.dot(oc) - radius * radius;
            let discriminant = b * b - 4.0 * a * c;

            if discriminant >= 0.0 {
                let t = (-b - discriminant.sqrt()) / (2.0 * a);
                if t > 0.0 && t < best_t {
                    best_t = t;
                    let hit = local_origin + local_dir * t;
                    best_normal = (hit - cap_center).try_normalize().unwrap_or(Vec3::Y);
                }
            }
        }

        if best_t < f32::INFINITY {
            let world_normal = rotation * best_normal;
            Some((best_t, world_normal))
        } else {
            None
        }
    }

    /// Test ray against collider shape
    pub fn ray_shape(
        ray: &Ray,
        shape: &ColliderShape,
        transform: &Transform,
    ) -> Option<(f32, Vec3)> {
        match shape {
            ColliderShape::Sphere(s) => Self::ray_sphere(ray, transform.position, s.radius),
            ColliderShape::Box(b) => {
                Self::ray_box(ray, transform.position, transform.rotation, b.half_extents)
            }
            ColliderShape::Capsule(c) => Self::ray_capsule(
                ray,
                transform.position,
                transform.rotation,
                c.radius,
                c.half_height,
            ),
            ColliderShape::Plane(p) => {
                // Ray-plane intersection
                let denom = ray.direction.dot(p.normal);
                if denom.abs() > 1e-6 {
                    let t = (p.distance - ray.origin.dot(p.normal)) / denom;
                    if t >= 0.0 {
                        let normal = if denom < 0.0 { p.normal } else { -p.normal };
                        Some((t, normal))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            ColliderShape::TriMesh(tm) => {
                let mut best_t = f32::INFINITY;
                let mut best_normal = Vec3::ZERO;
                let inv_rot = transform.rotation.inverse();
                let local_origin = inv_rot * (ray.origin - transform.position);
                let local_dir = inv_rot * ray.direction;
                let local_ray = Ray::new(local_origin, local_dir);
                
                if !tm.bvh.nodes.is_empty() {
                    let mut stack = Vec::with_capacity(64);
                    stack.push(0); // root node
                    
                    while let Some(node_idx) = stack.pop() {
                        let node = &tm.bvh.nodes[node_idx];
                        
                        // Check AABB
                        if Self::ray_aabb(&local_ray, &node.aabb).is_none() {
                            continue;
                        }
                        
                        if node.is_leaf() {
                            let start = (node.first_tri_index * 3) as usize;
                            let end = start + (node.tri_count * 3) as usize;
                            for i in (start..end).step_by(3) {
                                let v0 = tm.vertices[tm.indices[i] as usize];
                                let v1 = tm.vertices[tm.indices[i+1] as usize];
                                let v2 = tm.vertices[tm.indices[i+2] as usize];
                                
                                let e1 = v1 - v0;
                                let e2 = v2 - v0;
                                let h = local_dir.cross(e2);
                                let a = e1.dot(h);
                                if a.abs() < 1e-6 { continue; }
                                let f = 1.0 / a;
                                let s = local_origin - v0;
                                let u = f * s.dot(h);
                                if u < 0.0 || u > 1.0 { continue; }
                                let q = s.cross(e1);
                                let v = f * local_dir.dot(q);
                                if v < 0.0 || u + v > 1.0 { continue; }
                                let t = f * e2.dot(q);
                                if t > 0.0 && t < best_t {
                                    best_t = t;
                                    best_normal = e1.cross(e2).try_normalize().unwrap_or(Vec3::Y);
                                    if best_normal.dot(local_dir) > 0.0 {
                                        best_normal = -best_normal;
                                    }
                                }
                            }
                        } else {
                            if node.left_child >= 0 { stack.push(node.left_child as usize); }
                            if node.right_child >= 0 { stack.push(node.right_child as usize); }
                        }
                    }
                } else {
                    // Fallback to naive loop if BVH is missing
                    for chunk in tm.indices.chunks_exact(3) {
                        let v0 = tm.vertices[chunk[0] as usize];
                        let v1 = tm.vertices[chunk[1] as usize];
                        let v2 = tm.vertices[chunk[2] as usize];
                        let e1 = v1 - v0;
                        let e2 = v2 - v0;
                        let h = local_dir.cross(e2);
                        let a = e1.dot(h);
                        if a.abs() < 1e-6 { continue; }
                        let f = 1.0 / a;
                        let s = local_origin - v0;
                        let u = f * s.dot(h);
                        if u < 0.0 || u > 1.0 { continue; }
                        let q = s.cross(e1);
                        let v = f * local_dir.dot(q);
                        if v < 0.0 || u + v > 1.0 { continue; }
                        let t = f * e2.dot(q);
                        if t > 0.0 && t < best_t {
                            best_t = t;
                            best_normal = e1.cross(e2).try_normalize().unwrap_or(Vec3::Y);
                            if best_normal.dot(local_dir) > 0.0 {
                                best_normal = -best_normal;
                            }
                        }
                    }
                }
                
                if best_t < f32::INFINITY {
                    Some((best_t, transform.rotation * best_normal))
                } else {
                    None
                }
            }
            ColliderShape::ConvexHull(ch) => {
                let mut min = Vec3::splat(f32::MAX);
                let mut max = Vec3::splat(f32::MIN);
                for v in ch.vertices.iter() {
                    min.x = min.x.min(v.x); min.y = min.y.min(v.y); min.z = min.z.min(v.z);
                    max.x = max.x.max(v.x); max.y = max.y.max(v.y); max.z = max.z.max(v.z);
                }
                let center = (min + max) * 0.5;
                let half_extents = (max - min) * 0.5;
                
                // Adjust transform to local space of the original transform
                let world_center = transform.position + transform.rotation * center;
                Self::ray_box(ray, world_center, transform.rotation, half_extents)
            }
            ColliderShape::Compound(shapes) => {
                let mut closest_dist = f32::MAX;
                let mut closest_normal = Vec3::ZERO;
                for (local_t, sub_shape) in shapes {
                    let world_pos = transform.position + transform.rotation.mul_vec3(local_t.position);
                    let world_rot = transform.rotation * local_t.rotation;
                    let world_t = crate::components::Transform::new(world_pos).with_rotation(world_rot);
                    if let Some((d, n)) = Self::ray_shape(ray, sub_shape, &world_t) {
                        if d < closest_dist {
                            closest_dist = d;
                            closest_normal = n;
                        }
                    }
                }
                if closest_dist < f32::MAX {
                    Some((closest_dist, closest_normal))
                } else {
                    None
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ray_sphere() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let center = Vec3::ZERO;
        let radius = 1.0;

        let result = Raycast::ray_sphere(&ray, center, radius);
        assert!(result.is_some());

        let (t, _normal) = result.unwrap();
        assert!((t - 4.0).abs() < 0.01);
    }

    #[test]
    fn test_ray_aabb() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let aabb = Aabb::from_center_half_extents(Vec3::ZERO, Vec3::splat(1.0));

        let result = Raycast::ray_aabb(&ray, &aabb);
        assert!(result.is_some());

        let t = result.unwrap();
        assert!((t - 4.0).abs() < 0.01);
    }

    #[test]
    fn test_ray_miss() {
        let ray = Ray::new(Vec3::new(5.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0));
        let center = Vec3::ZERO;
        let radius = 1.0;

        let result = Raycast::ray_sphere(&ray, center, radius);
        assert!(result.is_none());
    }

    #[test]
    fn test_ray_box() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let center = Vec3::ZERO;
        let result = Raycast::ray_box(&ray, center, gizmo_math::Quat::IDENTITY, Vec3::splat(1.0));
        assert!(result.is_some());
        let (t, normal) = result.unwrap();
        assert!((t - 4.0).abs() < 0.01);
        assert!((normal.z - -1.0).abs() < 0.01);
    }

    #[test]
    fn test_ray_capsule() {
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let center = Vec3::ZERO;
        let result = Raycast::ray_capsule(&ray, center, gizmo_math::Quat::IDENTITY, 1.0, 1.0);
        assert!(result.is_some());
        let (t, normal) = result.unwrap();
        assert!((t - 4.0).abs() < 0.01);
        assert!((normal.z - -1.0).abs() < 0.01);
    }

    #[test]
    fn test_ray_capsule_parallel() {
        let ray = Ray::new(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, -1.0, 0.0));
        let center = Vec3::ZERO;
        // The ray is parallel to the Y axis (the capsule's internal axis).
        // It hits the top sphere cap. The height is half_height = 1.0. 
        // The top sphere cap is centered at Y=1.0 with radius 1.0. Hit should be at Y=2.0.
        let result = Raycast::ray_capsule(&ray, center, gizmo_math::Quat::IDENTITY, 1.0, 1.0);
        assert!(result.is_some());
        let (t, normal) = result.unwrap();
        assert!((t - 8.0).abs() < 0.01); // 10.0 - 2.0 = 8.0
        assert!((normal.y - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_ray_plane_backface() {
        // Plane is at Z=0, pointing towards +Z.
        let plane = crate::components::PlaneShape { normal: Vec3::Z, distance: 0.0 };
        let shape = ColliderShape::Plane(plane);
        
        // Ray from -5 looking towards +Z
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let result = Raycast::ray_shape(&ray, &shape, &Transform::new(Vec3::ZERO));
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, -Vec3::Z); // Should be flipped since ray hits the backface
    }
}
