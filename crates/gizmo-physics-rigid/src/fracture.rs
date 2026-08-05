//! Voronoi shattering: cutting a box-shaped body into convex debris.
//!
//! Only boxes can be shattered — the input is a set of half-extents, not a mesh — and the
//! geometry stays in the shattered body's local frame throughout. [`voronoi_shatter`] does
//! the cutting and stops at bare meshes; [`generate_fracture_chunks`] is the level above it,
//! turning those meshes into ready-to-spawn
//! `(body, transform, collider, velocity, chunk)` tuples — the chunk mesh comes back with
//! them — with the parent's mass divided between them by volume and an outward kick applied.
//!
//! Debris is simulation state, so every random draw in this module is seeded by the caller —
//! same-platform reproducibility, in line with the rest of the engine.

use gizmo_math::Vec3;
use gizmo_physics_core::BodyHandle;
use rand::{rngs::StdRng, RngExt, SeedableRng};

/// Patlama hız değişimi (force/mass) üst sınırı. İnce kıymık parçalar çok küçük kütleye
/// sahip olduğundan `force/mass` patlayıp parçaları absürt hızlarda fırlatıyordu (tünelleme
/// / kararsızlık). Hız değişimini bu makul sınıra kırp.
const MAX_EXPLOSION_DV: f32 = 50.0;

/// One convex piece of a shattered body: a triangle mesh plus the mass properties derived
/// from it.
///
/// Every coordinate is in the **local frame of the body that was shattered** — the same
/// frame its half-extents were given in, origin at the box centre. The mesh is deliberately
/// not re-centred on [`center_of_mass`](Self::center_of_mass), which is what lets all the
/// debris of one break share the original body's transform and differ only in their mass
/// properties.
///
/// The mesh is a render/collision source, not a simulated object: it carries no material,
/// no colour and no UVs.
#[derive(Clone, Debug)]
pub struct ProceduralChunk {
    /// Vertex positions in metres, in the parent body's local frame, all inside the source
    /// box up to a roughly millimetre plane tolerance.
    ///
    /// Faces do not share vertices — each face pushes its own copy so its flat normal stays
    /// hard — so this is not a minimal vertex set and geometrically identical positions
    /// appear several times.
    pub vertices: Vec<Vec3>,
    /// One outward unit normal per entry of [`vertices`](Self::vertices) (the two lists
    /// always have equal length), constant across each face: every copy of a face's vertices
    /// carries that face's plane normal, which is what gives the debris hard edges rather
    /// than a smoothed look.
    pub normals: Vec<Vec3>,
    /// Triangle list into [`vertices`](Self::vertices) / [`normals`](Self::normals): three
    /// consecutive entries per triangle, so the length is always a multiple of three. Each
    /// face is fan-triangulated, so all of a face's triangles share its first sorted vertex.
    ///
    /// Take facing from [`normals`](Self::normals), not from the winding — the fan is
    /// ordered the opposite way round from the stored normal, and sliver faces are not
    /// reliably consistent even about that. Mass properties are unaffected either way: they
    /// only need the winding to be self-consistent and take the absolute value.
    pub indices: Vec<u32>,
    /// The chunk's volume centroid — the centre of mass of a uniform-density solid — in the
    /// parent's local frame.
    ///
    /// Deliberately **not** the average of [`vertices`](Self::vertices): for an asymmetric
    /// cell the two differ (a square pyramid's mass sits at h/4, its vertex mean at h/5), and
    /// feeding the vertex mean to the solver biased the chunk's rotational dynamics. Falls
    /// back to the vertex average only when the mesh's signed volume is degenerate.
    pub center_of_mass: Vec3,
    /// Chunk volume in cubic metres, from the same signed-tetrahedron decomposition as
    /// [`center_of_mass`](Self::center_of_mass) — exact for a mesh that closed up, and off
    /// by the missing contribution for one that lost a degenerate face to the build
    /// tolerances.
    ///
    /// Floored at `0.001`, so that splitting the parent's mass by volume fraction can never
    /// divide by zero; a sliver below that floor therefore reports, and is given, more mass
    /// than its geometry deserves.
    pub volume: f32, // approximated
}

#[derive(Clone, Copy)]
struct MathPlane {
    normal: Vec3,
    d: f32, // dot(N, P) - d = 0 => dot(N, P) = d
}

impl MathPlane {
    // Normal points OUTSIDE
    fn distance(&self, pt: Vec3) -> f32 {
        self.normal.dot(pt) - self.d
    }

    fn from_point_normal(pt: Vec3, normal: Vec3) -> Self {
        Self {
            normal: normal.normalize(),
            d: normal.normalize().dot(pt),
        }
    }
}

/// Volume **and** center-of-mass of a convex polyhedron (closed triangle mesh),
/// via signed-tetrahedron decomposition relative to the vertex centroid.
///
/// The COM returned is the true **volume centroid** (center of mass of a uniform-
/// density solid), NOT the vertex average. For asymmetric voronoi chunks the two
/// differ (e.g. a pyramid's mass sits at h/4, not the h/5 vertex mean), and the
/// vertex average biased the chunk's rotational dynamics. Each triangle `(a,b,c)`
/// plus the reference point forms a tetrahedron with signed volume `v = a·(b×c)/6`
/// (relative coords) and centroid `(a+b+c)/4`; the mesh COM is the volume-weighted
/// mean of the tetra centroids, `Σ v·g / Σ v`. Sign cancels between numerator and
/// denominator, so winding handedness (as long as it is *consistent*) does not
/// matter. Volume is `|Σ v|` — unchanged from the previous volume-only routine, so
/// per-chunk mass distribution is preserved.
fn compute_convex_mass_props(vertices: &[Vec3], indices: &[u32]) -> (Vec3, f32) {
    let vertex_centroid =
        vertices.iter().copied().fold(Vec3::ZERO, |a, b| a + b) / vertices.len().max(1) as f32;
    if indices.len() < 3 {
        return (vertex_centroid, 0.001);
    }
    let mut vol6 = 0.0f32; // 6 × signed volume (Σ of triangle a·(b×c))
    let mut com_acc = Vec3::ZERO; // Σ (a+b+c)·v6_i, relative to vertex_centroid
    for tri in indices.chunks_exact(3) {
        let a = vertices[tri[0] as usize] - vertex_centroid;
        let b = vertices[tri[1] as usize] - vertex_centroid;
        let c = vertices[tri[2] as usize] - vertex_centroid;
        let v6 = a.dot(b.cross(c));
        vol6 += v6;
        com_acc += (a + b + c) * v6;
    }
    let volume = (vol6 / 6.0).abs().max(0.001);
    // C_rel = Σ(v_i·g_i)/Σ(v_i) = Σ(v6·(a+b+c))/(4·Σv6). Degenerate (near-zero
    // signed volume, e.g. sliver/coplanar) → fall back to the vertex centroid.
    let com = if vol6.abs() > 1e-8 {
        vertex_centroid + com_acc / (4.0 * vol6)
    } else {
        vertex_centroid
    };
    (com, volume)
}

/// Cut an axis-aligned box into convex Voronoi cells.
///
/// `extents` are **half**-extents in metres: the box spans `-extents ..= extents` about the
/// origin, and every returned [`ProceduralChunk`] stays in that same local frame. Degenerate
/// axes are made safe rather than rejected — each component is taken absolute and floored to
/// `1e-3`, so a zero-thickness panel shatters as a millimetre-thick one instead of panicking
/// inside the RNG's range sampling.
///
/// `num_pieces` is the number of Voronoi seed points, i.e. an **upper bound** on the result
/// length, not a promise: a cell whose half-space intersection yields fewer than four
/// distinct corners, or no triangles at all, is dropped. The result can be shorter than
/// asked for and, for `num_pieces == 0`, empty. The cells partition the box between them, so
/// pieces are not sized independently of each other; the reported
/// [`ProceduralChunk::volume`] is floored, though, so do not read the returned volumes as an
/// exact share of the original — see that field.
///
/// # Cost
///
/// Per cell this intersects every *triple* of that cell's bounding planes — six box planes
/// plus one bisector for every other seed — and tests each candidate corner against all of
/// them. The work therefore climbs steeply with `num_pieces`; treat it as a load-time or
/// precompute routine (see [`PreFracturedCache`]) rather than something a gameplay system
/// calls per frame.
///
/// # Determinism
///
/// `seed` is the only source of randomness, so the same `(extents, num_pieces, seed)` yields
/// the same chunks — that is what lets a fractured scene take part in replay and rollback.
/// Same-platform only, as everywhere in this engine; do not treat the layout as stable
/// across platforms, or across versions of this crate or of `rand`.
pub fn voronoi_shatter(extents: Vec3, num_pieces: u32, seed: u64) -> Vec<ProceduralChunk> {
    // A degenerate half-extent (0 or negative — e.g. a thin panel/floor tile authored as a
    // zero-thickness box) would make `rng.random_range(-e..e)` sample an empty range, which
    // is an unconditional panic in rand 0.10 (fires in release too), crashing the physics
    // step mid-shatter. Floor each axis to a small positive extent — same `.max(1e-3)` idiom
    // used for volume/radius elsewhere in this module.
    let extents = extents.abs().max(Vec3::splat(1e-3));
    let mut rng = StdRng::seed_from_u64(seed);

    // 1. Generate seeds
    let mut seeds = Vec::with_capacity(num_pieces as usize);
    for _ in 0..num_pieces {
        seeds.push(Vec3::new(
            rng.random_range(-extents.x..extents.x),
            rng.random_range(-extents.y..extents.y),
            rng.random_range(-extents.z..extents.z),
        ));
    }

    let mut chunks = Vec::with_capacity(num_pieces as usize);

    let box_planes = vec![
        MathPlane::from_point_normal(Vec3::new(extents.x, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0)),
        MathPlane::from_point_normal(Vec3::new(-extents.x, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0)),
        MathPlane::from_point_normal(Vec3::new(0.0, extents.y, 0.0), Vec3::new(0.0, 1.0, 0.0)),
        MathPlane::from_point_normal(Vec3::new(0.0, -extents.y, 0.0), Vec3::new(0.0, -1.0, 0.0)),
        MathPlane::from_point_normal(Vec3::new(0.0, 0.0, extents.z), Vec3::new(0.0, 0.0, 1.0)),
        MathPlane::from_point_normal(Vec3::new(0.0, 0.0, -extents.z), Vec3::new(0.0, 0.0, -1.0)),
    ];

    // Reusable buffers to avoid memory allocation jitter
    let mut planes = Vec::with_capacity(box_planes.len() + num_pieces as usize);
    let mut raw_vertices = Vec::with_capacity(256);
    let mut out_vertices = Vec::with_capacity(256);
    let mut out_normals = Vec::with_capacity(256);
    let mut out_indices = Vec::with_capacity(512);
    let mut face_verts = Vec::with_capacity(64);

    for i in 0..num_pieces as usize {
        let p_i = seeds[i];

        planes.clear();
        planes.extend_from_slice(&box_planes);

        for (j, &p_j) in seeds[..num_pieces as usize].iter().enumerate() {
            if i == j {
                continue;
            }
            let dir = p_j - p_i;
            let length = dir.length();
            if length < 0.001 {
                continue;
            }
            let normal = dir / length;
            let mid = (p_i + p_j) * 0.5;
            planes.push(MathPlane::from_point_normal(mid, normal));
        }

        // Find vertices via plane intersections
        raw_vertices.clear();
        let num_planes = planes.len();

        for p1 in 0..num_planes {
            for p2 in (p1 + 1)..num_planes {
                for p3 in (p2 + 1)..num_planes {
                    if let Some(intersection) =
                        intersect_planes(&planes[p1], &planes[p2], &planes[p3])
                    {
                        // Check if it's inside all other planes
                        let mut is_inside = true;
                        for (k, plane) in planes.iter().enumerate() {
                            if k == p1 || k == p2 || k == p3 {
                                continue;
                            }
                            if plane.distance(intersection) > 0.001 {
                                // Slight epsilon
                                is_inside = false;
                                break;
                            }
                        }
                        if is_inside {
                            // Don't add duplicates
                            let mut dup = false;
                            for &v in &raw_vertices {
                                let diff: Vec3 = v - intersection;
                                if diff.length_squared() < 0.0001 {
                                    dup = true;
                                    break;
                                }
                            }
                            if !dup {
                                raw_vertices.push(intersection);
                            }
                        }
                    }
                }
            }
        }

        // If something went wrong and we couldn't form a 3D boundary, skip
        if raw_vertices.len() < 4 {
            continue;
        }

        out_vertices.clear();
        out_normals.clear();
        out_indices.clear();

        // Accumulate face triangles
        // A face is formed by a subset of raw_vertices that lie on one of the `planes`.
        for plane in &planes {
            face_verts.clear();
            for &v in &raw_vertices {
                if plane.distance(v).abs() < 0.005 {
                    face_verts.push(v);
                }
            }
            if face_verts.len() >= 3 {
                // Sort vertices around the plane normal, projecting onto a 2D coordinate system
                let face_center = face_verts.iter().copied().fold(Vec3::ZERO, |a, b| a + b)
                    / face_verts.len() as f32;

                // create local basis — guard against degenerate ref_v
                let n = plane.normal;
                let mut ref_v = Vec3::ZERO;
                for fv in &face_verts {
                    let candidate = *fv - face_center;
                    if candidate.length_squared() > 1e-8 {
                        ref_v = candidate.normalize();
                        break;
                    }
                }
                // If all vertices coincide with face_center (degenerate), skip face
                if ref_v.length_squared() < 0.5 {
                    continue;
                }
                // Ensure ref_v is not parallel to normal
                let cross_test = n.cross(ref_v);
                if cross_test.length_squared() < 1e-8 {
                    // Pick an arbitrary perpendicular
                    ref_v = if n.x.abs() > 0.9 {
                        Vec3::new(0.0, 1.0, 0.0)
                    } else {
                        Vec3::new(1.0, 0.0, 0.0)
                    };
                }
                let tangent = n.cross(ref_v).normalize();
                let bitangent = n.cross(tangent).normalize();

                face_verts.sort_by(|a, b| {
                    let dir_a = *a - face_center;
                    let dir_b = *b - face_center;
                    let angle_a = f32::atan2(dir_a.dot(tangent), dir_a.dot(bitangent));
                    let angle_b = f32::atan2(dir_b.dot(tangent), dir_b.dot(bitangent));
                    angle_a
                        .partial_cmp(&angle_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                // Fan triangulation
                let base_idx = out_vertices.len() as u32;

                // To keep hard edges, duplicate the vertices for this face and calculate proper normals
                let norm = plane.normal;
                for v in &face_verts {
                    out_vertices.push(*v);
                    out_normals.push(norm);
                }

                for k in 1..(face_verts.len() - 1) {
                    out_indices.push(base_idx);
                    out_indices.push(base_idx + k as u32);
                    out_indices.push(base_idx + k as u32 + 1);
                }
            }
        }

        if out_indices.is_empty() {
            continue;
        }

        // True volume centroid (mass center of a uniform solid), not the vertex
        // average — the mesh is a closed convex polyhedron here.
        let (center_of_mass, volume) = compute_convex_mass_props(&out_vertices, &out_indices);
        chunks.push(ProceduralChunk {
            vertices: out_vertices.clone(),
            normals: out_normals.clone(),
            indices: out_indices.clone(),
            center_of_mass,
            volume,
        });
    }

    chunks
}

// Intersects three planes and finds the intersection point
fn intersect_planes(p1: &MathPlane, p2: &MathPlane, p3: &MathPlane) -> Option<Vec3> {
    let cross = p2.normal.cross(p3.normal);
    let det = p1.normal.dot(cross);
    if det.abs() < 0.0001 {
        return None; // Planes do not intersect at a single point (parallel)
    }

    let inv_det = 1.0 / det;
    let res =
        (cross * p1.d) + (p3.normal.cross(p1.normal) * p2.d) + (p1.normal.cross(p2.normal) * p3.d);

    Some(res * inv_det)
}

/// Helper function to create physics chunks from a fracturing event.
/// Returns a list of (RigidBody, Transform, Collider, ProceduralChunk) for the ECS to spawn.
///
/// # Determinism
///
/// `seed` drives **everything** random about the result: the Voronoi cell layout (and hence
/// every chunk's geometry, volume, mass and inertia) and the debris spin. The same inputs
/// always produce bit-identical output, which is what lets fractured scenes take part in
/// replay and rollback.
///
/// Pick a seed that is itself reproducible — an entity id, a frame counter, a hash of the two
/// — **not** `rand::random()`. Passing entropy here silently opts the whole simulation out of
/// the crate's same-platform bit-exactness guarantee: a replay would shatter the object
/// differently, and a rollback netcode peer would desync on the first fracture.
///
/// Until 0.9 this function seeded itself from the thread-local entropy pool, which made every
/// caller non-deterministic with no way to opt out. Passing an explicit seed is the fix; use
/// a fixed constant if you genuinely do not care about variation.
///
/// ```
/// # use gizmo_physics_rigid::fracture::generate_fracture_chunks;
/// # use gizmo_physics_rigid::components::{RigidBody, Velocity};
/// # use gizmo_physics_core::Transform;
/// # use gizmo_math::Vec3;
/// let t = Transform::new(Vec3::ZERO);
/// let rb = RigidBody::new(10.0, true);
/// let v = Velocity::default();
/// let args = (Vec3::splat(0.5), 8u32, Vec3::new(0.0, 1.0, 0.0), 50.0);
///
/// let a = generate_fracture_chunks(&t, &rb, &v, args.0, args.1, args.2, args.3, 1234);
/// let b = generate_fracture_chunks(&t, &rb, &v, args.0, args.1, args.2, args.3, 1234);
/// assert_eq!(a.len(), b.len());
/// assert_eq!(a[0].1.position, b[0].1.position); // same seed → same debris
/// ```
pub fn generate_fracture_chunks(
    original_transform: &gizmo_physics_core::Transform,
    original_body: &crate::components::RigidBody,
    original_velocity: &crate::components::Velocity,
    extents: Vec3,
    num_pieces: u32,
    impact_point: Vec3,
    impact_force: f32,
    seed: u64,
) -> Vec<(
    crate::components::RigidBody,
    gizmo_physics_core::Transform,
    gizmo_physics_core::Collider,
    crate::components::Velocity,
    ProceduralChunk,
)> {
    let chunks = voronoi_shatter(extents, num_pieces, seed);

    // Separate stream for the debris spin so that adding/removing a jitter draw can never
    // shift the Voronoi cell layout (and vice versa). The constant is the golden-ratio odd
    // integer used by SplitMix64 — any fixed odd constant works; the point is only that the
    // two streams are decorrelated and reproducible.
    let mut spin_rng = StdRng::seed_from_u64(seed ^ 0x9E37_79B9_7F4A_7C15);

    let mut results = Vec::with_capacity(chunks.len());
    let total_volume: f32 = chunks.iter().map(|c| c.volume).sum();
    let original_mass = original_body.mass;

    for chunk in chunks {
        // Calculate fraction of mass
        let mass = if total_volume > 0.0 {
            original_mass * (chunk.volume / total_volume)
        } else {
            0.1
        };

        // Create new rigid body. Friction/restitution live on the collider
        // material, not the body, so fragments inherit them via their colliders.
        let mut rb = crate::components::RigidBody::new(mass, original_body.use_gravity);
        rb.center_of_mass = chunk.center_of_mass;

        // Inherit exact same velocity + explosion force away from impact point
        let mut vel = *original_velocity;

        // Calculate explosion force direction
        let world_chunk_center =
            original_transform.position + original_transform.rotation * chunk.center_of_mass;
        let dir = world_chunk_center - impact_point;
        if dir.length_squared() > 0.001 {
            let explosion_dir = dir.normalize();
            // Force drops off with distance (simplified)
            let force = impact_force * 0.1 / (dir.length() + 1.0);
            let dv = (force / mass).min(MAX_EXPLOSION_DV);
            vel.linear += explosion_dir * dv;

            // Add some random spin — drawn from the seeded `spin_rng`, never from the
            // thread-local entropy pool: this function is part of the crate's public API and
            // the crate's contract is same-platform bit-exact replay.
            vel.angular += Vec3::new(
                spin_rng.random_range(-0.5f32..0.5),
                spin_rng.random_range(-0.5f32..0.5),
                spin_rng.random_range(-0.5f32..0.5),
            ) * dv
                * 0.5;
        }

        // Create convex hull collider
        let hull = gizmo_physics_core::quickhull::compute_convex_hull(&chunk.vertices);
        let collider = gizmo_physics_core::Collider::from_shape(
            gizmo_physics_core::ColliderShape::ConvexHull(gizmo_physics_core::ConvexHullShape {
                vertices: std::sync::Arc::new(hull.vertices),
                faces: std::sync::Arc::new(hull.faces),
            }),
        );

        rb.update_inertia_from_collider(&collider);

        let transform = gizmo_physics_core::Transform {
            position: original_transform.position, // The vertices in the chunk are local to the original center
            rotation: original_transform.rotation,
            scale: original_transform.scale,
            ..*original_transform
        };

        results.push((rb, transform, collider, vel, chunk));
    }

    results
}

/// Stores pre-fractured chunks to avoid expensive runtime calculations (Pre-fracture Caching).
/// Ideal for AAA games where destruction must not drop frames.
#[derive(Default)]
pub struct PreFracturedCache {
    /// Maps an BodyHandle ID to its pre-calculated fracture data
    pub cache: std::collections::HashMap<BodyHandle, Vec<ProceduralChunk>>,
}

impl PreFracturedCache {
    /// An empty cache, identical to [`Default::default`].
    ///
    /// Until [`pre_fracture`](Self::pre_fracture) has run for a given handle,
    /// [`get_fracture_chunks`](Self::get_fracture_chunks) returns `None` for it — the cache
    /// has no runtime fallback of its own, so the caller decides whether a miss means
    /// shattering on the spot with [`generate_fracture_chunks`] or not breaking at all.
    pub fn new() -> Self {
        Self {
            cache: std::collections::HashMap::new(),
        }
    }

    /// Pre-calculates fracture chunks for an entity and stores them in the cache.
    /// This should be called during a loading screen.
    pub fn pre_fracture(
        &mut self,
        entity: BodyHandle,
        extents: Vec3,
        num_pieces: u32,
        seed: u64,
    ) {
        let chunks = voronoi_shatter(extents, num_pieces, seed);
        self.cache.insert(entity, chunks);
    }

    /// Spawns the chunks from the cache if available, taking only O(N) time to clone instead of O(N^3).
    /// If not in cache, optionally falls back to runtime calculation.
    pub fn get_fracture_chunks(
        &self,
        entity: BodyHandle,
        original_transform: &gizmo_physics_core::Transform,
        original_body: &crate::components::RigidBody,
        original_velocity: &crate::components::Velocity,
        impact_point: Vec3,
        impact_force: f32,
    ) -> Option<
        Vec<(
            crate::components::RigidBody,
            gizmo_physics_core::Transform,
            gizmo_physics_core::Collider,
            crate::components::Velocity,
            ProceduralChunk,
        )>,
    > {
        let chunks = self.cache.get(&entity)?;

        let mut results = Vec::with_capacity(chunks.len());
        let total_volume: f32 = chunks.iter().map(|c| c.volume).sum();
        let original_mass = original_body.mass;

        for chunk in chunks {
            let mass = if total_volume > 0.0 {
                original_mass * (chunk.volume / total_volume)
            } else {
                0.1
            };

            let mut rb = crate::components::RigidBody::new(mass, original_body.use_gravity);
            rb.center_of_mass = chunk.center_of_mass;

            let mut vel = *original_velocity;
            let world_chunk_center =
                original_transform.position + original_transform.rotation * chunk.center_of_mass;
            let dir = world_chunk_center - impact_point;
            if dir.length_squared() > 0.001 {
                let explosion_dir = dir.normalize();
                let force = impact_force * 0.1 / (dir.length() + 1.0);
                let dv = (force / mass).min(MAX_EXPLOSION_DV);
                vel.linear += explosion_dir * dv;

                // Deterministic spin based on chunk properties (since cache is pre-calculated)
                vel.angular += Vec3::new(
                    (chunk.center_of_mass.x * 12.345).fract() - 0.5,
                    (chunk.center_of_mass.y * 67.890).fract() - 0.5,
                    (chunk.center_of_mass.z * 42.123).fract() - 0.5,
                ) * dv
                    * 0.5;
            }

            let hull = gizmo_physics_core::quickhull::compute_convex_hull(&chunk.vertices);
            let collider = gizmo_physics_core::Collider::from_shape(
                gizmo_physics_core::ColliderShape::ConvexHull(gizmo_physics_core::ConvexHullShape {
                    vertices: std::sync::Arc::new(hull.vertices),
                    faces: std::sync::Arc::new(hull.faces),
                }),
            );

            rb.update_inertia_from_collider(&collider);

            let transform = gizmo_physics_core::Transform {
                position: original_transform.position,
                rotation: original_transform.rotation,
                scale: original_transform.scale,
                ..*original_transform
            };

            results.push((rb, transform, collider, vel, chunk.clone()));
        }

        Some(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{RigidBody, Velocity};
    use gizmo_physics_core::BodyHandle;
    use gizmo_physics_core::Transform;

    #[test]
    fn voronoi_shatter_does_not_panic_on_degenerate_extents() {
        // Regression: a thin/flat breakable (a zero or negative half-extent) made
        // `rng.random_range(-e..e)` sample an empty range, an unconditional panic in rand
        // 0.10 that crashed the physics step. Degenerate extents must be floored, not panic.
        let flat = voronoi_shatter(Vec3::new(1.0, 0.0, 1.0), 6, 42);
        assert!(!flat.is_empty(), "flat box should still produce chunks");
        let neg = voronoi_shatter(Vec3::new(-1.0, 0.5, 0.0), 4, 7);
        assert!(!neg.is_empty(), "negative/zero extents must be handled, not panic");
        // Sanity: a normal box still shatters.
        assert!(!voronoi_shatter(Vec3::splat(1.0), 8, 1).is_empty());
    }

    /// COM = gerçek HACİM merkezi (düzgün-yoğunluklu katının kütle merkezi), vertex
    /// ortalaması DEĞİL. Ayırt edici şekil: kare piramit — kütle merkezi tabandan h/4,
    /// vertex ortalaması ise h/5. Eski kod (vertex-centroid) bu testte FAIL eder.
    #[test]
    fn convex_mass_props_returns_volume_centroid_not_vertex_average() {
        // Taban z=0'da 2×2 kare (alan 4), tepe (0,0,3) → hacim = ⅓·4·3 = 4.
        let verts = vec![
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 3.0),
        ];
        // Vertex ortalaması = (0,0,0.6); hacim merkezi = (0,0,0.75).
        let mut tris: Vec<[u32; 3]> = vec![
            [0, 1, 2],
            [0, 2, 3], // taban
            [0, 1, 4],
            [1, 2, 4],
            [2, 3, 4],
            [3, 0, 4], // yan yüzler
        ];
        // Fonksiyon TUTARLI sarım ister; her üçgeni dışa-normal olacak şekilde düzelt.
        let centroid = verts.iter().copied().fold(Vec3::ZERO, |a, b| a + b) / verts.len() as f32;
        let mut indices = Vec::new();
        for t in &mut tris {
            let (a, b, c) = (
                verts[t[0] as usize],
                verts[t[1] as usize],
                verts[t[2] as usize],
            );
            let n = (b - a).cross(c - a);
            let face_center = (a + b + c) / 3.0;
            if n.dot(face_center - centroid) < 0.0 {
                t.swap(1, 2);
            }
            indices.extend_from_slice(t);
        }

        let (com, vol) = compute_convex_mass_props(&verts, &indices);
        assert!((vol - 4.0).abs() < 1e-3, "hacim ~4 olmalı, bulundu {vol}");
        assert!(
            com.x.abs() < 1e-4 && com.y.abs() < 1e-4,
            "simetri: com.xy ≈ 0, bulundu {com:?}"
        );
        assert!(
            (com.z - 0.75).abs() < 1e-3,
            "hacim merkezi z ≈ 0.75 olmalı (vertex ortalaması 0.6 DEĞİL), bulundu {}",
            com.z
        );
        assert!(
            (com.z - 0.6).abs() > 0.1,
            "COM vertex ortalamasından (0.6) belirgin farklı olmalı"
        );
    }

    /// İnce kıymık (çok küçük hacim/kütle) parçalar, büyük çarpma kuvvetinde bile
    /// makul hızda fırlatılmalı — `force/mass` patlaması MAX_EXPLOSION_DV ile kırpılır.
    #[test]
    fn explosion_velocity_clamped_for_tiny_chunks() {
        let tetra = || {
            vec![
                Vec3::ZERO,
                Vec3::X * 0.1,
                Vec3::Y * 0.1,
                Vec3::Z * 0.1,
            ]
        };
        let big = ProceduralChunk {
            vertices: tetra(),
            normals: vec![],
            indices: vec![],
            center_of_mass: Vec3::new(1.0, 0.0, 0.0),
            volume: 10.0,
        };
        let tiny = ProceduralChunk {
            vertices: tetra(),
            normals: vec![],
            indices: vec![],
            center_of_mass: Vec3::new(-1.0, 0.0, 0.0),
            volume: 0.001, // minik → çok küçük kütle
        };

        let mut cache = PreFracturedCache::new();
        let e = BodyHandle::from_id(1);
        cache.cache.insert(e, vec![big, tiny]);

        let tr = Transform::new(Vec3::ZERO);
        let body = RigidBody::new(10.0, true);
        let out = cache
            .get_fracture_chunks(e, &tr, &body, &Velocity::default(), Vec3::new(0.0, 5.0, 0.0), 1.0e6)
            .unwrap();

        assert_eq!(out.len(), 2);
        for (_rb, _t, _c, v, _chunk) in &out {
            assert!(v.linear.is_finite() && v.angular.is_finite(), "hız sonlu olmalı");
            assert!(
                v.linear.length() < 100.0,
                "patlama hızı makul sınırda olmalı (clamp), oldu: {}",
                v.linear.length()
            );
        }
    }
}

#[cfg(test)]
mod determinism_tests {
    use super::*;
    use crate::components::{RigidBody, Velocity};
    use gizmo_physics_core::Transform;

    fn scenario() -> (Transform, RigidBody, Velocity, Vec3, u32, Vec3, f32) {
        (
            Transform::new(Vec3::new(1.0, 2.0, 3.0)),
            RigidBody::new(10.0, true),
            Velocity::default(),
            Vec3::new(0.5, 0.4, 0.3),
            12,
            Vec3::new(0.0, 1.5, 0.0),
            80.0,
        )
    }

    fn run(seed: u64) -> Vec<(Vec3, Vec3, Vec3, f32)> {
        let (t, rb, v, extents, pieces, impact, force) = scenario();
        generate_fracture_chunks(&t, &rb, &v, extents, pieces, impact, force, seed)
            .into_iter()
            .map(|(body, transform, _collider, vel, chunk)| {
                (transform.position, vel.linear, vel.angular, body.mass + chunk.volume)
            })
            .collect()
    }

    /// The contract this whole crate is sold on: same inputs → bit-identical output.
    ///
    /// Before 0.9 this function called `rand::random::<u64>()` for the Voronoi seed and
    /// `rand::random::<f32>()` three times for the debris spin, so two calls with identical
    /// arguments produced different chunk geometry, masses, inertias and angular velocities.
    /// Any scene that fractured therefore diverged between replays and desynced under
    /// rollback — the exact failure the `state_hash` + cross-process oracle exist to prevent.
    #[test]
    fn same_seed_reproduces_debris_bit_for_bit() {
        let a = run(0xC0FFEE);
        let b = run(0xC0FFEE);
        assert!(!a.is_empty(), "scenario must actually produce chunks");
        assert_eq!(a.len(), b.len(), "chunk count must be reproducible");
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(x.0.to_array(), y.0.to_array(), "chunk {i} position drifted");
            assert_eq!(x.1.to_array(), y.1.to_array(), "chunk {i} linear velocity drifted");
            assert_eq!(x.2.to_array(), y.2.to_array(), "chunk {i} angular velocity drifted");
            assert_eq!(x.3.to_bits(), y.3.to_bits(), "chunk {i} mass/volume drifted");
        }
    }

    /// The seed must actually be load-bearing — a function that ignores it would pass the
    /// test above trivially.
    #[test]
    fn different_seeds_produce_different_debris() {
        let a = run(1);
        let b = run(2);
        assert!(!a.is_empty() && !b.is_empty());
        let identical = a.len() == b.len()
            && a.iter()
                .zip(b.iter())
                .all(|(x, y)| x.0.to_array() == y.0.to_array() && x.3.to_bits() == y.3.to_bits());
        assert!(!identical, "seed had no observable effect on the debris");
    }

    /// The debris spin must come from the seeded stream too, not just the cell layout.
    /// Guards against a partial fix that seeds `voronoi_shatter` but leaves the jitter on
    /// `rand::random()` — chunk geometry would match while angular velocity drifted.
    #[test]
    fn angular_jitter_is_seeded_not_entropy() {
        let a = run(7);
        let b = run(7);
        let spun: Vec<_> = a.iter().filter(|c| c.2.length_squared() > 0.0).collect();
        assert!(
            !spun.is_empty(),
            "scenario must impart spin, otherwise this test proves nothing"
        );
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(
                x.2.to_array(),
                y.2.to_array(),
                "chunk {i} angular velocity is not reproducible — jitter still uses entropy"
            );
        }
    }
}
