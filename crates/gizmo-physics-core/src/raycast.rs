//! Ray casting against collider shapes.
//!
//! These are the exact per-shape tests, nothing more: no broadphase culling, no
//! layer filtering, no notion of which bodies exist. A scene query prunes
//! candidates first and calls [`Raycast::ray_shape`] on the survivors.
//!
//! Every function returns the ray parameter `t` at the hit rather than a point.
//! With the unit direction [`Ray::new`] guarantees, `t` is a distance in metres,
//! and [`Ray::point_at`] turns it back into a position. The ray and the shape must
//! be expressed in the same frame; the local-space conversions that `ray_box`,
//! `ray_capsule` and the mesh/hull arms of `ray_shape` perform internally are not
//! visible to the caller.

use crate::components::{ColliderShape, Transform};
use crate::BodyHandle;
use gizmo_math::Aabb;
use gizmo_math::Vec3;

/// Ray for raycasting
#[derive(Debug, Clone, Copy)]
pub struct Ray {
    /// Where the ray starts. Frame-agnostic: the tests in this module read it in
    /// whatever space the shape they are given lives in, and several of them build
    /// local-space `Ray`s of this same type internally.
    pub origin: Vec3,
    /// Direction of travel — unit length whenever the value came from
    /// [`Ray::new`], which normalises and substitutes `+Z` for a zero or
    /// non-finite input.
    ///
    /// The field is public, so writing to it directly can leave a non-unit
    /// direction, and nothing in this module compensates for that. A `t` is then
    /// not a distance in metres, and the tests do not even agree on how it scales
    /// — [`Raycast::ray_box`] rebuilds a local `Ray` through [`Ray::new`] and so
    /// renormalises, while [`Raycast::ray_aabb`] does not. Worse,
    /// [`Raycast::ray_sphere`] and the cylinder branch of
    /// [`Raycast::ray_capsule`] use quadratics that *assume* unit length and give
    /// wrong roots — not merely rescaled ones — without it.
    pub direction: Vec3, // Should be normalized
}

impl Ray {
    /// Creates a new ray. `direction` is normalised internally.
    ///
    /// If a zero-length (or non-finite) direction vector is given, glam's
    /// `normalize()` call silently produces NaN/Inf, creating a broken Ray, and
    /// every subsequent raycast returns a bogus result. To prevent this,
    /// `try_normalize()` is used and a safe default (`Vec3::Z`) is chosen for
    /// directions that cannot be normalised; the Ray therefore always has a valid,
    /// finite unit direction. For valid (non-zero) directions the behaviour does not change.
    pub fn new(origin: Vec3, direction: Vec3) -> Self {
        Self {
            origin,
            direction: direction.try_normalize().unwrap_or_else(|| {
                // A zero-length / non-finite direction cannot be normalised; glam's bare
                // `normalize` would yield NaN and poison every downstream raycast. We fall
                // back to +Z, but that is almost certainly a caller bug worth surfacing.
                tracing::warn!(
                    ?direction,
                    "Ray::new received a zero/non-finite direction; defaulting to +Z"
                );
                Vec3::Z
            }),
        }
    }

    /// The point at ray parameter `t`: `origin + direction * t`, in the ray's own
    /// frame.
    ///
    /// Nothing is clamped — a negative `t` walks backwards from the origin — and
    /// this is the intended way to turn any `t` returned by this module back into a
    /// position.
    pub fn point_at(&self, t: f32) -> Vec3 {
        self.origin + self.direction * t
    }
}

/// Result of a raycast hit
#[derive(Debug, Clone, Copy)]
pub struct RaycastHit {
    /// The body that was hit. Nothing in this module produces a `RaycastHit` — the
    /// scene query that owns the body list fills this in.
    pub entity: BodyHandle,
    /// World-space hit position, in metres.
    ///
    /// The engine's own queries compute it as `ray.point_at(distance)`, so it lies
    /// exactly on the ray and is a restatement of [`distance`](Self::distance)
    /// rather than an independently measured point on the surface.
    pub point: Vec3,
    /// Unit surface normal at the hit, in world space.
    ///
    /// Its orientation is whatever the shape test returned, and that is not one
    /// rule: the plane, mesh and hull arms of [`Raycast::ray_shape`] flip the
    /// normal to oppose the ray, while the box and sphere tests report the outward
    /// normal of the face actually crossed — which, for a ray that starts inside,
    /// is the exit face and points *along* the ray.
    pub normal: Vec3,
    /// Ray parameter `t` at the hit, measured from [`Ray::origin`] and never
    /// negative; see [`Ray::direction`] for what it is denominated in.
    ///
    /// This is the field to compare when picking the closest of several hits;
    /// [`point`](Self::point) is derived from it.
    pub distance: f32,
}

/// Raycast query system
pub struct Raycast;

impl Raycast {
    /// Test ray against AABB
    ///
    /// Returns the ray parameter of the **entry** face, or — when the origin is
    /// already inside the box — of the **exit** face, so the result is never
    /// negative. `None` when the ray misses the box, or when the whole box lies
    /// behind the origin.
    ///
    /// An axis whose direction component is below 1e-8 is treated as parallel to
    /// that slab: it can only cause a miss (origin outside the slab), never bound
    /// `t`. Empty and inverted boxes are *not* rejected — unlike
    /// [`Aabb::intersects`](gizmo_math::Aabb::intersects) this never consults
    /// `is_empty`, so a box with `min > max` produces a meaningless answer rather
    /// than `None`.
    pub fn ray_aabb(ray: &Ray, aabb: &Aabb) -> Option<f32> {
        // tmin gerçek giriş (negatif olabilir), tmax çıkış. (Eskiden tmin=0'dan
        // başlıyordu → ışın kutunun İÇİNDE başlarsa t=0 dönüp `origin` yüzey üstünde
        // olmadığından çağıran sahte normal üretiyordu.)
        let mut tmin: f32 = f32::NEG_INFINITY;
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

        if tmax < 0.0 {
            return None; // tüm kutu ışının arkasında
        }
        // İçeriden başlama: tmin<0 ise çıkış yüzeyini (tmax) döndür → geçerli yüzey noktası/normal.
        Some(if tmin < 0.0 { tmax } else { tmin })
    }

    /// Test ray against sphere
    ///
    /// Returns the parameter of the nearest intersection strictly in front of the
    /// origin, together with the outward unit normal there (from `center` through
    /// the hit point). `center` and `radius` are in the ray's frame, metres.
    ///
    /// A ray starting inside the sphere gets the exit root, so its outward normal
    /// points *along* the ray rather than against it. Only ONE root has to be strictly
    /// positive: a ray whose origin sits exactly on the surface pointing inward has
    /// `t1 == 0`, so the entry root is skipped and the exit root is returned. A sphere the
    /// ray only reaches behind its origin is rejected.
    ///
    /// The quadratic is the unit-direction form (see [`Ray::direction`]). If the
    /// hit point coincides with `center` the normal falls back to `Vec3::Y`
    /// instead of becoming NaN.
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
    ///
    /// `center` and `rotation` place the box; `half_extents` are its half-sizes in
    /// metres along its **own** local axes, so the full box measures
    /// `2 * half_extents`.
    ///
    /// Returns the ray parameter and the world-space unit face normal. The normal
    /// is recovered by testing the local hit point against each face within 1e-4,
    /// which has two consequences: a hit on an edge or corner reports the
    /// normalised sum of the faces it matched (a diagonal, not a face normal), and
    /// if no face matches at all the fallback is `Vec3::Y`. As with
    /// [`ray_aabb`](Self::ray_aabb), a ray starting inside reports the exit face,
    /// whose outward normal points along the ray.
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

    /// Ray against a solid cylinder about local +Y, centred on `center`.
    ///
    /// Three surfaces, tested together and resolved by nearest hit: the side wall (a quadratic
    /// in the XZ plane, accepted only where the hit's `y` is inside the cylinder) and the two
    /// flat caps (a plane hit accepted only inside the disc). Doing it as one nearest-of-three
    /// is what keeps the normal right at the rim, where the side and a cap meet.
    ///
    /// Returns `(distance, world-space normal)` of the first surface at or after the ray's
    /// origin. A ray that starts *inside* reports the exit surface — the same behaviour as the
    /// other primitives here, and it means a hit with `t == 0` is not special-cased away.
    /// A degenerate direction along the axis with the origin outside the disc misses, rather
    /// than dividing by zero.
    pub fn ray_cylinder(
        ray: &Ray,
        center: Vec3,
        rotation: gizmo_math::Quat,
        radius: f32,
        half_height: f32,
    ) -> Option<(f32, Vec3)> {
        let inv_rot = rotation.inverse();
        let o = inv_rot * (ray.origin - center);
        let d = inv_rot * ray.direction;

        // Nearest accepted hit so far, in local space.
        let mut best: Option<(f32, Vec3)> = None;
        let mut consider = |t: f32, normal: Vec3| {
            if t >= 0.0 && best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, normal));
            }
        };

        // Side wall: |o.xz + t·d.xz|² = r², accepted while the hit stays between the caps.
        let a = d.x * d.x + d.z * d.z;
        if a > 1e-12 {
            let b = 2.0 * (o.x * d.x + o.z * d.z);
            let c = o.x * o.x + o.z * o.z - radius * radius;
            let disc = b * b - 4.0 * a * c;
            if disc >= 0.0 {
                let sqrt_disc = disc.sqrt();
                for t in [(-b - sqrt_disc) / (2.0 * a), (-b + sqrt_disc) / (2.0 * a)] {
                    let y = o.y + t * d.y;
                    if y.abs() <= half_height {
                        let hit = o + d * t;
                        let n = Vec3::new(hit.x, 0.0, hit.z);
                        consider(t, n.try_normalize().unwrap_or(Vec3::X));
                    }
                }
            }
        }

        // Caps: the two planes y = ±half_height, accepted inside the disc.
        if d.y.abs() > 1e-12 {
            for sign in [1.0_f32, -1.0] {
                let t = (sign * half_height - o.y) / d.y;
                let hit = o + d * t;
                if hit.x * hit.x + hit.z * hit.z <= radius * radius {
                    consider(t, Vec3::new(0.0, sign, 0.0));
                }
            }
        }

        best.map(|(t, n)| (t, rotation * n))
    }

    /// Test ray against capsule
    ///
    /// The capsule runs along its **own local Y axis**, and `half_height` is half of
    /// the *cylindrical* section only — the hemispherical caps sit outside it, so the
    /// capsule's total length is `2 * (half_height + radius)` metres.
    ///
    /// Returns the ray parameter and the world-space unit surface normal. The
    /// cylinder is tested first and wins outright when it is struck in front of the
    /// origin within the cylindrical span; otherwise the two caps are tested and the
    /// nearer positive root is taken, which is also what happens for a ray running
    /// (near) parallel to the axis.
    ///
    /// Only the near root of each branch is considered, so a ray that starts
    /// **inside** the capsule is not a case this test handles: it may report no
    /// hit at all, or a spurious hit on the interior of a cap with an
    /// inward-facing normal.
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
                // Check if hit is within cylinder height AND in front of the ray.
                // (`t > 0.0` eksikti: ışının ARKASINDAKİ kapsül negatif t ile sahte
                // isabet döndürüyordu — küre-cap dalı zaten t>0 kontrol ediyor.)
                let y = baoc + t * bard;
                if t > 0.0 && y > 0.0 && y < baba {
                    let hit_point = local_origin + local_dir * t;
                    let normal = (hit_point - (p1 + ba * (y / baba)))
                        .try_normalize()
                        .unwrap_or(Vec3::Y);
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
    ///
    /// Returns the parameter of the nearest hit and a world-space unit normal, or
    /// `None`. Two properties of the placement matter more than the dispatch table:
    ///
    /// - **`transform.scale` is ignored.** Only `position` and `rotation` are read
    ///   (composed with the child's local transform for compound parts). Sizes come
    ///   from the shape itself, so scaling a `Transform` does not scale what this
    ///   test hits.
    /// - **A [`ColliderShape::Plane`] ignores `transform` entirely.** Its `normal`
    ///   and `distance` are used exactly as stored, i.e. they already describe a
    ///   world-space plane, and moving the collider does not move it. The returned
    ///   normal is flipped to oppose the ray, so the plane is solid from both sides.
    ///
    /// Per shape: sphere and box delegate to [`ray_sphere`](Self::ray_sphere) and
    /// [`ray_box`](Self::ray_box) and inherit their inside-start behaviour. Triangle
    /// meshes and convex hulls run Möller–Trumbore over triangles in the shape's own
    /// space, keep the nearest, and flip that triangle's normal to oppose the ray; a
    /// mesh whose BVH is empty falls back to scanning every triangle. A hull whose
    /// `faces` list is empty — what
    /// [`compute_convex_hull`](crate::quickhull::compute_convex_hull) returns for
    /// degenerate input — is approximated by its AABB instead, which reports hits the
    /// real hull would miss. A compound returns its nearest sub-shape hit.
    #[tracing::instrument(skip_all, level = "trace", name = "ray_shape")]
    pub fn ray_shape(
        ray: &Ray,
        shape: &ColliderShape,
        transform: &Transform,
    ) -> Option<(f32, Vec3)> {
        let result = match shape {
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
            ColliderShape::Cylinder(c) => Self::ray_cylinder(
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
                                let v1 = tm.vertices[tm.indices[i + 1] as usize];
                                let v2 = tm.vertices[tm.indices[i + 2] as usize];

                                let e1 = v1 - v0;
                                let e2 = v2 - v0;
                                let h = local_dir.cross(e2);
                                let a = e1.dot(h);
                                if a.abs() < 1e-6 {
                                    continue;
                                }
                                let f = 1.0 / a;
                                let s = local_origin - v0;
                                let u = f * s.dot(h);
                                if !(0.0..=1.0).contains(&u) {
                                    continue;
                                }
                                let q = s.cross(e1);
                                let v = f * local_dir.dot(q);
                                if v < 0.0 || u + v > 1.0 {
                                    continue;
                                }
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
                            if node.left_child >= 0 {
                                stack.push(node.left_child as usize);
                            }
                            if node.right_child >= 0 {
                                stack.push(node.right_child as usize);
                            }
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
                        if a.abs() < 1e-6 {
                            continue;
                        }
                        let f = 1.0 / a;
                        let s = local_origin - v0;
                        let u = f * s.dot(h);
                        if !(0.0..=1.0).contains(&u) {
                            continue;
                        }
                        let q = s.cross(e1);
                        let v = f * local_dir.dot(q);
                        if v < 0.0 || u + v > 1.0 {
                            continue;
                        }
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
                // Yüz yoksa AABB yaklaşımına düş (nadiren; hull genelde yüzleriyle gelir).
                if ch.faces.is_empty() {
                    let mut min = Vec3::splat(f32::MAX);
                    let mut max = Vec3::splat(f32::MIN);
                    for v in ch.vertices.iter() {
                        min = min.min(*v);
                        max = max.max(*v);
                    }
                    let center = (min + max) * 0.5;
                    let half_extents = (max - min) * 0.5;
                    let world_center = transform.position + transform.rotation * center;
                    return Self::ray_box(ray, world_center, transform.rotation, half_extents);
                }

                // Tam ray-hull testi: hull üçgenlerine Möller-Trumbore (eskiden yalnız AABB
                // yaklaşımı vardı → kutu köşelerinde gerçek hull'ı ıskalayan sahte isabet).
                let inv_rot = transform.rotation.inverse();
                let local_origin = inv_rot * (ray.origin - transform.position);
                let local_dir = inv_rot * ray.direction;
                let mut best_t = f32::INFINITY;
                let mut best_normal = Vec3::ZERO;
                for tri in ch.faces.iter() {
                    let v0 = ch.vertices[tri[0] as usize];
                    let v1 = ch.vertices[tri[1] as usize];
                    let v2 = ch.vertices[tri[2] as usize];
                    let e1 = v1 - v0;
                    let e2 = v2 - v0;
                    let h = local_dir.cross(e2);
                    let a = e1.dot(h);
                    if a.abs() < 1e-6 {
                        continue;
                    }
                    let f = 1.0 / a;
                    let s = local_origin - v0;
                    let u = f * s.dot(h);
                    if !(0.0..=1.0).contains(&u) {
                        continue;
                    }
                    let q = s.cross(e1);
                    let v = f * local_dir.dot(q);
                    if v < 0.0 || u + v > 1.0 {
                        continue;
                    }
                    let t = f * e2.dot(q);
                    if t > 0.0 && t < best_t {
                        best_t = t;
                        best_normal = e1.cross(e2).try_normalize().unwrap_or(Vec3::Y);
                        if best_normal.dot(local_dir) > 0.0 {
                            best_normal = -best_normal;
                        }
                    }
                }
                if best_t < f32::INFINITY {
                    Some((best_t, transform.rotation * best_normal))
                } else {
                    None
                }
            }
            ColliderShape::Compound(shapes) => {
                let mut closest_dist = f32::MAX;
                let mut closest_normal = Vec3::ZERO;
                for (local_t, sub_shape) in shapes {
                    let world_pos =
                        transform.position + transform.rotation.mul_vec3(local_t.position);
                    let world_rot = transform.rotation * local_t.rotation;
                    let world_t =
                        crate::components::Transform::new(world_pos).with_rotation(world_rot);
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
        };

        match result {
            Some((distance, _normal)) => tracing::trace!(distance, "raycast hit"),
            None => tracing::trace!("raycast miss"),
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::CylinderShape;
    use gizmo_math::Quat;

    /// The three surfaces of a cylinder, one ray each, plus the miss that a naive
    /// infinite-cylinder test would report as a hit.
    #[test]
    fn a_ray_finds_the_wall_the_cap_and_nothing_past_the_end() {
        let shape = ColliderShape::Cylinder(CylinderShape {
            radius: 0.5,
            half_height: 1.0,
        });
        let at_origin = Transform::new(Vec3::ZERO);

        // Side wall, straight in along -X from 3 m out: 2.5 m to the surface, normal +X.
        let side = Ray::new(Vec3::new(3.0, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0));
        let (t, n) = Raycast::ray_shape(&side, &shape, &at_origin).expect("hits the wall");
        assert!((t - 2.5).abs() < 1e-4, "distance {t}");
        assert!((n - Vec3::X).length() < 1e-4, "normal {n:?}");

        // Top cap, straight down the axis: 2 m, normal +Y. A capsule would answer 1.5 m.
        let cap = Ray::new(Vec3::new(0.0, 3.0, 0.0), Vec3::new(0.0, -1.0, 0.0));
        let (t, n) = Raycast::ray_shape(&cap, &shape, &at_origin).expect("hits the cap");
        assert!((t - 2.0).abs() < 1e-4, "distance {t}");
        assert!((n - Vec3::Y).length() < 1e-4, "normal {n:?}");

        // Level with the wall but past the end: the infinite cylinder is struck, the solid is
        // not. Getting this wrong is a raycast that stops at a wheel's ghost.
        let past_end = Ray::new(Vec3::new(3.0, 2.0, 0.0), Vec3::new(-1.0, 0.0, 0.0));
        assert!(
            Raycast::ray_shape(&past_end, &shape, &at_origin).is_none(),
            "a ray level with nothing must miss"
        );

        // Parallel to the axis but outside the disc: also a miss, and not a divide by zero.
        let beside = Ray::new(Vec3::new(2.0, 3.0, 0.0), Vec3::new(0.0, -1.0, 0.0));
        assert!(Raycast::ray_shape(&beside, &shape, &at_origin).is_none());
    }

    /// The shape turns with the body, so the ray has to be tested in its frame — a cylinder laid
    /// on its side is the wheel case, and the one where a forgotten rotation would still "work"
    /// against an upright test.
    #[test]
    fn a_ray_respects_the_cylinders_rotation_and_position() {
        let shape = ColliderShape::Cylinder(CylinderShape {
            radius: 0.5,
            half_height: 1.0,
        });
        let mut lying = Transform::new(Vec3::new(0.0, 2.0, 0.0));
        lying.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);

        // From above: the axis now points along X, so the top of the shape is a wall at
        // y = 2 + radius.
        let down = Ray::new(Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.0, -1.0, 0.0));
        let (t, n) = Raycast::ray_shape(&down, &shape, &lying).expect("hits the wall");
        assert!((t - 2.5).abs() < 1e-4, "distance {t}");
        assert!((n - Vec3::Y).length() < 1e-4, "normal {n:?}");

        // Along the new axis: now it is the flat end that is struck, at x = half_height.
        let along = Ray::new(Vec3::new(5.0, 2.0, 0.0), Vec3::new(-1.0, 0.0, 0.0));
        let (t, n) = Raycast::ray_shape(&along, &shape, &lying).expect("hits the end");
        assert!((t - 4.0).abs() < 1e-4, "distance {t}");
        assert!((n - Vec3::X).length() < 1e-4, "normal {n:?}");
    }


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
        let plane = crate::components::PlaneShape {
            normal: Vec3::Z,
            distance: 0.0,
        };
        let shape = ColliderShape::Plane(plane);

        // Ray from -5 looking towards +Z
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 1.0));
        let result = Raycast::ray_shape(&ray, &shape, &Transform::new(Vec3::ZERO));
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, -Vec3::Z); // Should be flipped since ray hits the backface
    }

    /// A ray starting from the inside must give a valid exit-surface normal (previously, at
    /// t=0, since `origin` was not on the surface, it returned a bogus +Y).
    #[test]
    fn ray_box_from_inside_returns_valid_exit_normal() {
        use gizmo_math::Quat;
        let ray = Ray::new(Vec3::ZERO, Vec3::X); // kutu merkezinden +X
        let (t, normal) =
            Raycast::ray_box(&ray, Vec3::ZERO, Quat::IDENTITY, Vec3::splat(1.0)).unwrap();
        assert!(t > 0.0, "çıkış mesafesi pozitif olmalı");
        assert!(
            (normal - Vec3::X).length() < 1e-3,
            "çıkış normali +X olmalı (sahte +Y değil), oldu: {normal:?}"
        );
    }

    /// The ConvexHull raycast must be against the REAL hull, not the AABB: a ray passing
    /// through an AABB corner and missing the hull must return None.
    #[test]
    fn convex_hull_raycast_is_exact_not_aabb() {
        use crate::components::collider::ConvexHullShape;
        use crate::quickhull::compute_convex_hull;
        use std::sync::Arc;
        // Tetrahedron (x+y+z ≤ 1 bölgesi); AABB ise [0,1]³.
        let hull = compute_convex_hull(&[Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z]);
        let shape = ColliderShape::ConvexHull(ConvexHullShape {
            vertices: Arc::new(hull.vertices),
            faces: Arc::new(hull.faces),
        });
        let tr = Transform::new(Vec3::ZERO);

        // (0.9,0.9): AABB içinde ama tetrahedron dışında (x+y=1.8>1) → ıskala.
        let miss = Ray::new(Vec3::new(0.9, 0.9, 5.0), Vec3::new(0.0, 0.0, -1.0));
        assert!(
            Raycast::ray_shape(&miss, &shape, &tr).is_none(),
            "AABB köşesinden geçip hull'ı ıskalayan ışın None dönmeli (tam test)"
        );
        // Tetrahedronun içinden geçen ışın isabet etmeli.
        let hit = Ray::new(Vec3::new(0.2, 0.2, 5.0), Vec3::new(0.0, 0.0, -1.0));
        assert!(
            Raycast::ray_shape(&hit, &shape, &tr).is_some(),
            "hull'dan geçen ışın isabet etmeli"
        );
    }

    #[test]
    fn ray_aabb_parallel_outside_slab_misses() {
        // Ray runs along +Z, but its X lies outside the box's X slab → parallel-miss.
        let ray = Ray::new(Vec3::new(5.0, 0.0, -5.0), Vec3::Z);
        let aabb = Aabb::from_center_half_extents(Vec3::ZERO, Vec3::splat(1.0));
        assert!(Raycast::ray_aabb(&ray, &aabb).is_none());
    }

    #[test]
    fn ray_aabb_entirely_behind_misses() {
        // Box is fully behind the origin along the ray direction (tmax < 0).
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::Z);
        let aabb = Aabb::from_center_half_extents(Vec3::ZERO, Vec3::splat(1.0));
        assert!(Raycast::ray_aabb(&ray, &aabb).is_none());
    }

    #[test]
    fn ray_sphere_from_inside_returns_far_exit() {
        // Origin at the centre → the near root is negative, so the exit root (t2) is used.
        let ray = Ray::new(Vec3::ZERO, Vec3::X);
        let (t, normal) = Raycast::ray_sphere(&ray, Vec3::ZERO, 1.0).unwrap();
        assert!((t - 1.0).abs() < 1e-4, "exit distance {t}");
        assert!((normal - Vec3::X).length() < 1e-4, "{normal:?}");
    }

    #[test]
    fn ray_plane_parallel_misses() {
        let plane = ColliderShape::Plane(crate::components::PlaneShape {
            normal: Vec3::Z,
            distance: 0.0,
        });
        // Ray perpendicular to the normal (runs within the plane) → no intersection.
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::X);
        assert!(Raycast::ray_shape(&ray, &plane, &Transform::new(Vec3::ZERO)).is_none());
    }

    #[test]
    fn ray_plane_behind_origin_misses() {
        let plane = ColliderShape::Plane(crate::components::PlaneShape {
            normal: Vec3::Z,
            distance: 0.0,
        });
        // Origin in front of the plane, pointing away → t < 0.
        let ray = Ray::new(Vec3::new(0.0, 0.0, 5.0), Vec3::Z);
        assert!(Raycast::ray_shape(&ray, &plane, &Transform::new(Vec3::ZERO)).is_none());
    }

    #[test]
    fn ray_new_normalises_and_rejects_zero_direction() {
        let r = Ray::new(Vec3::ZERO, Vec3::new(0.0, 0.0, 3.0));
        assert!((r.direction.length() - 1.0).abs() < 1e-6, "direction must be unit");
        // A degenerate zero direction falls back to +Z instead of producing NaNs.
        let z = Ray::new(Vec3::ZERO, Vec3::ZERO);
        assert_eq!(z.direction, Vec3::Z);
        assert!(z.direction.is_finite());
    }

    #[test]
    fn point_at_walks_along_direction() {
        let ray = Ray::new(Vec3::new(1.0, 2.0, 3.0), Vec3::X);
        assert!((ray.point_at(4.0) - Vec3::new(5.0, 2.0, 3.0)).length() < 1e-6);
    }

    #[test]
    fn compound_raycast_returns_nearest_subshape() {
        use crate::components::BoxShape;
        // Two unit boxes along Z (near at z=0, far at z=10). A ray from -Z hits the near one.
        let compound = ColliderShape::Compound(vec![
            (
                Transform::new(Vec3::new(0.0, 0.0, 0.0)),
                Box::new(ColliderShape::Box(BoxShape {
                    half_extents: Vec3::splat(1.0),
                })),
            ),
            (
                Transform::new(Vec3::new(0.0, 0.0, 10.0)),
                Box::new(ColliderShape::Box(BoxShape {
                    half_extents: Vec3::splat(1.0),
                })),
            ),
        ]);
        let ray = Ray::new(Vec3::new(0.0, 0.0, -5.0), Vec3::Z);
        let (t, _n) =
            Raycast::ray_shape(&ray, &compound, &Transform::new(Vec3::ZERO)).unwrap();
        // Near box's front face at z = -1 ⇒ distance 4 (not the far box at ~14).
        assert!((t - 4.0).abs() < 1e-3, "expected nearest hit at t≈4, got {t}");
    }
}
