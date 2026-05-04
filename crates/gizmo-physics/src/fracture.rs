use gizmo_math::Vec3;
use rand::{rngs::StdRng, RngExt, SeedableRng};

#[derive(Clone, Debug)]
pub struct ProceduralChunk {
    pub vertices: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
    pub center_of_mass: Vec3,
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

/// Compute the approximate volume of a convex polyhedron defined by its vertices
/// using signed tetrahedron decomposition relative to the centroid.
fn compute_convex_volume(vertices: &[Vec3], indices: &[u32]) -> f32 {
    if indices.len() < 3 {
        return 0.001;
    }
    // Use the centroid as the reference point
    let centroid = vertices.iter().copied().fold(Vec3::ZERO, |a, b| a + b)
        / vertices.len().max(1) as f32;
    let mut vol = 0.0f32;
    // Sum signed tetrahedron volumes for each triangle face
    for tri in indices.chunks_exact(3) {
        let a = vertices[tri[0] as usize] - centroid;
        let b = vertices[tri[1] as usize] - centroid;
        let c = vertices[tri[2] as usize] - centroid;
        vol += a.dot(b.cross(c));
    }
    (vol / 6.0).abs().max(0.001)
}

pub fn voronoi_shatter(extents: Vec3, num_pieces: u32, seed: u64) -> Vec<ProceduralChunk> {
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

    for i in 0..num_pieces as usize {
        let p_i = seeds[i];

        let mut planes = box_planes.clone();

        for j in 0..num_pieces as usize {
            if i == j {
                continue;
            }
            let p_j = seeds[j];
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
        let mut raw_vertices = Vec::new();
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

        let mut center = Vec3::ZERO;
        for &v in &raw_vertices {
            center += v;
        }
        center /= raw_vertices.len() as f32;

        let mut out_vertices = Vec::new();
        let mut out_normals = Vec::new();
        let mut out_indices = Vec::new();

        // Accumulate face triangles
        // A face is formed by a subset of raw_vertices that lie on one of the `planes`.
        for plane in &planes {
            let mut face_verts = Vec::new();
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
                if ref_v.length_squared() < 0.5 { continue; }
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

        let volume = compute_convex_volume(&out_vertices, &out_indices);
        chunks.push(ProceduralChunk {
            vertices: out_vertices,
            normals: out_normals,
            indices: out_indices,
            center_of_mass: center,
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
pub fn generate_fracture_chunks(
    original_transform: &crate::components::Transform,
    original_body: &crate::components::RigidBody,
    original_velocity: &crate::components::Velocity,
    extents: Vec3,
    num_pieces: u32,
    impact_point: Vec3,
    impact_force: f32,
) -> Vec<(crate::components::RigidBody, crate::components::Transform, crate::components::Collider, crate::components::Velocity, ProceduralChunk)> {
    let chunks = voronoi_shatter(extents, num_pieces, rand::random::<u64>());
    
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

        // Create new rigid body
        let mut rb = crate::components::RigidBody::new(
            mass,
            original_body.restitution,
            original_body.friction,
            original_body.use_gravity
        );
        rb.center_of_mass = chunk.center_of_mass;
        
        // Inherit exact same velocity + explosion force away from impact point
        let mut vel = *original_velocity;
        
        // Calculate explosion force direction
        let world_chunk_center = original_transform.position + original_transform.rotation * chunk.center_of_mass;
        let dir = world_chunk_center - impact_point;
        if dir.length_squared() > 0.001 {
            let explosion_dir = dir.normalize();
            // Force drops off with distance (simplified)
            let force = impact_force * 0.1 / (dir.length() + 1.0);
            vel.linear += explosion_dir * (force / mass);
            
            // Add some random spin
            vel.angular += Vec3::new(
                rand::random::<f32>() - 0.5,
                rand::random::<f32>() - 0.5,
                rand::random::<f32>() - 0.5
            ) * (force / mass) * 0.5;
        }

        // Create convex hull collider
        let collider = crate::components::Collider {
            shape: crate::components::ColliderShape::ConvexHull(crate::components::ConvexHullShape {
                vertices: std::sync::Arc::new(chunk.vertices.clone()),
            }),
            is_trigger: false,
            material: crate::components::PhysicsMaterial::default(),
            collision_layer: crate::components::CollisionLayer::default(),
        };
        
        rb.update_inertia_from_collider(&collider);

        let transform = crate::components::Transform {
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
    /// Maps an Entity ID to its pre-calculated fracture data
    pub cache: std::collections::HashMap<gizmo_core::entity::Entity, Vec<ProceduralChunk>>,
}

impl PreFracturedCache {
    pub fn new() -> Self {
        Self {
            cache: std::collections::HashMap::new(),
        }
    }

    /// Pre-calculates fracture chunks for an entity and stores them in the cache.
    /// This should be called during a loading screen.
    pub fn pre_fracture(
        &mut self,
        entity: gizmo_core::entity::Entity,
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
        entity: gizmo_core::entity::Entity,
        original_transform: &crate::components::Transform,
        original_body: &crate::components::RigidBody,
        original_velocity: &crate::components::Velocity,
        impact_point: Vec3,
        impact_force: f32,
    ) -> Option<Vec<(crate::components::RigidBody, crate::components::Transform, crate::components::Collider, crate::components::Velocity, ProceduralChunk)>> {
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

            let mut rb = crate::components::RigidBody::new(
                mass,
                original_body.restitution,
                original_body.friction,
                original_body.use_gravity
            );
            rb.center_of_mass = chunk.center_of_mass;
            
            let mut vel = *original_velocity;
            let world_chunk_center = original_transform.position + original_transform.rotation * chunk.center_of_mass;
            let dir = world_chunk_center - impact_point;
            if dir.length_squared() > 0.001 {
                let explosion_dir = dir.normalize();
                let force = impact_force * 0.1 / (dir.length() + 1.0);
                vel.linear += explosion_dir * (force / mass);
                
                // Deterministic spin based on chunk properties (since cache is pre-calculated)
                vel.angular += Vec3::new(
                    (chunk.center_of_mass.x * 12.345).fract() - 0.5,
                    (chunk.center_of_mass.y * 67.890).fract() - 0.5,
                    (chunk.center_of_mass.z * 42.123).fract() - 0.5
                ) * (force / mass) * 0.5;
            }

            let collider = crate::components::Collider {
                shape: crate::components::ColliderShape::ConvexHull(crate::components::ConvexHullShape {
                    vertices: std::sync::Arc::new(chunk.vertices.clone()),
                }),
                is_trigger: false,
                material: crate::components::PhysicsMaterial::default(),
                collision_layer: crate::components::CollisionLayer::default(),
            };
            
            rb.update_inertia_from_collider(&collider);

            let transform = crate::components::Transform {
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
