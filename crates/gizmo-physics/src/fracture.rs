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

                // create local basis
                let n = plane.normal;
                let ref_v = (face_verts[0] - face_center).normalize();
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

        chunks.push(ProceduralChunk {
            vertices: out_vertices,
            normals: out_normals,
            indices: out_indices,
            center_of_mass: center,
            volume: 1.0, // Could be exact by adding signed tetrahedron volumes
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
