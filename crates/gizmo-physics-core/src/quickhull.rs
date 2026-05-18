use gizmo_math::Vec3;
use std::collections::{HashMap, HashSet};

const EPSILON: f32 = 1e-4;

#[derive(Clone, Debug)]
pub struct HullFace {
    pub v: [usize; 3],
    pub normal: Vec3,
    pub outside_set: Vec<usize>,
}

impl HullFace {
    pub fn new(a: usize, b: usize, c: usize, points: &[Vec3]) -> Self {
        let pa = points[a];
        let pb = points[b];
        let pc = points[c];

        let mut normal = (pb - pa).cross(pc - pa);
        let len = normal.length();
        if len > 1e-8 {
            normal /= len;
        } else {
            normal = Vec3::Y;
        }

        Self {
            v: [a, b, c],
            normal,
            outside_set: Vec::new(),
        }
    }

    pub fn distance(&self, p: Vec3, points: &[Vec3]) -> f32 {
        (p - points[self.v[0]]).dot(self.normal)
    }
}

pub struct ConvexHull {
    pub vertices: Vec<Vec3>,
    pub faces: Vec<[u32; 3]>,
}

/// Computes an exact 3D Convex Hull using the Quickhull algorithm.
pub fn compute_convex_hull(points: &[Vec3]) -> ConvexHull {
    if points.len() < 4 {
        // Fallback for very small point sets
        return ConvexHull {
            vertices: points.to_vec(),
            faces: vec![],
        };
    }

    let pts = points;

    // 1. Find 6 extreme points
    let mut min_x = 0;
    let mut max_x = 0;
    let mut min_y = 0;
    let mut max_y = 0;
    let mut min_z = 0;
    let mut max_z = 0;

    for i in 1..pts.len() {
        let p = pts[i];
        if p.x < pts[min_x].x {
            min_x = i;
        }
        if p.x > pts[max_x].x {
            max_x = i;
        }
        if p.y < pts[min_y].y {
            min_y = i;
        }
        if p.y > pts[max_y].y {
            max_y = i;
        }
        if p.z < pts[min_z].z {
            min_z = i;
        }
        if p.z > pts[max_z].z {
            max_z = i;
        }
    }

    let extremes = [min_x, max_x, min_y, max_y, min_z, max_z];

    // Find the pair furthest apart
    let mut max_dist_sq = -1.0;
    let mut p0 = 0;
    let mut p1 = 0;
    for &e1 in &extremes {
        for &e2 in &extremes {
            let dist_sq = (pts[e1] - pts[e2]).length_squared();
            if dist_sq > max_dist_sq {
                max_dist_sq = dist_sq;
                p0 = e1;
                p1 = e2;
            }
        }
    }

    if max_dist_sq < 1e-8 {
        return ConvexHull {
            vertices: vec![pts[0]],
            faces: vec![],
        };
    }

    // Find p2 furthest from line p0-p1
    let mut max_dist_line = -1.0;
    let mut p2 = 0;
    let line_dir = (pts[p1] - pts[p0]).normalize();
    for i in 0..pts.len() {
        let p = pts[i];
        let v = p - pts[p0];
        let proj = v.dot(line_dir);
        let dist_sq = (v - line_dir * proj).length_squared();
        if dist_sq > max_dist_line {
            max_dist_line = dist_sq;
            p2 = i;
        }
    }

    if max_dist_line < 1e-8 {
        return ConvexHull {
            vertices: vec![pts[p0], pts[p1]],
            faces: vec![],
        };
    }

    // Find p3 furthest from plane p0-p1-p2
    let plane_normal = (pts[p1] - pts[p0]).cross(pts[p2] - pts[p0]).normalize();
    let mut max_dist_plane = -1.0;
    let mut p3 = 0;
    for i in 0..pts.len() {
        let dist = (pts[i] - pts[p0]).dot(plane_normal).abs();
        if dist > max_dist_plane {
            max_dist_plane = dist;
            p3 = i;
        }
    }

    if max_dist_plane < 1e-8 {
        // Points are coplanar, return planar points
        let mut dedup = pts.to_vec();
        dedup.dedup_by(|a, b| {
            (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4 && (a.z - b.z).abs() < 1e-4
        });
        return ConvexHull {
            vertices: dedup,
            faces: vec![],
        };
    }

    // Build initial tetrahedron faces
    let mut faces = Vec::new();

    // Ensure normal points OUTWARD.
    // Face 0,1,2: if p3 is in front, flip it to 0,2,1
    let mut f0 = HullFace::new(p0, p1, p2, pts);
    if f0.distance(pts[p3], pts) > 0.0 {
        f0 = HullFace::new(p0, p2, p1, pts);
    }

    let mut f1 = HullFace::new(p0, p3, p1, pts);
    if f1.distance(pts[p2], pts) > 0.0 {
        f1 = HullFace::new(p0, p1, p3, pts);
    }

    let mut f2 = HullFace::new(p1, p3, p2, pts);
    if f2.distance(pts[p0], pts) > 0.0 {
        f2 = HullFace::new(p1, p2, p3, pts);
    }

    let mut f3 = HullFace::new(p2, p3, p0, pts);
    if f3.distance(pts[p1], pts) > 0.0 {
        f3 = HullFace::new(p2, p0, p3, pts);
    }

    faces.push(f0);
    faces.push(f1);
    faces.push(f2);
    faces.push(f3);

    // Assign points to initial faces
    for i in 0..pts.len() {
        if i == p0 || i == p1 || i == p2 || i == p3 {
            continue;
        }
        let p = pts[i];

        let mut best_face = usize::MAX;
        let mut max_d = EPSILON;

        for (f_idx, f) in faces.iter().enumerate() {
            let d = f.distance(p, pts);
            if d > max_d {
                max_d = d;
                best_face = f_idx;
            }
        }

        if best_face != usize::MAX {
            faces[best_face].outside_set.push(i);
        }
    }

    // Main Quickhull Loop
    loop {
        // Find a face with an active outside set
        let mut active_face_idx = usize::MAX;
        for (i, f) in faces.iter().enumerate() {
            if !f.outside_set.is_empty() {
                active_face_idx = i;
                break;
            }
        }

        if active_face_idx == usize::MAX {
            break; // Finished!
        }

        // Pick the furthest point in the outside set
        let mut best_pt = 0;
        let mut max_d = -1.0;
        let active_face = &faces[active_face_idx];

        for &idx in &active_face.outside_set {
            let d = active_face.distance(pts[idx], pts);
            if d > max_d {
                max_d = d;
                best_pt = idx;
            }
        }

        let p = pts[best_pt];

        // Find all visible faces
        let mut visible_faces = Vec::new();
        for (i, f) in faces.iter().enumerate() {
            if f.distance(p, pts) > EPSILON {
                visible_faces.push(i);
            }
        }

        // Extract horizon edges
        // A directed edge (u, v) is added. If a neighboring visible face adds (v, u), they cancel out.
        let mut edge_counts = HashMap::new();
        for &f_idx in &visible_faces {
            let f = &faces[f_idx];
            let edges = [(f.v[0], f.v[1]), (f.v[1], f.v[2]), (f.v[2], f.v[0])];
            for &(u, v) in &edges {
                *edge_counts.entry((u, v)).or_insert(0) += 1;
            }
        }

        let mut horizon_edges = Vec::new();
        for (&(u, v), &count) in &edge_counts {
            if count == 1 && !edge_counts.contains_key(&(v, u)) {
                horizon_edges.push((u, v));
            }
        }

        // Collect all points from the outside sets of visible faces
        let mut orphaned_points = Vec::new();
        for &f_idx in &visible_faces {
            for &idx in &faces[f_idx].outside_set {
                if idx != best_pt {
                    orphaned_points.push(idx);
                }
            }
        }

        // Delete visible faces (mark by removing them later, for now we will build a new face list)
        let mut new_faces = Vec::new();
        for (i, f) in faces.into_iter().enumerate() {
            if !visible_faces.contains(&i) {
                new_faces.push(f);
            }
        }
        faces = new_faces;

        // Create new faces from horizon edges to P
        let mut new_face_indices = Vec::new();
        for &(u, v) in &horizon_edges {
            let new_face = HullFace::new(u, v, best_pt, pts);
            faces.push(new_face);
            new_face_indices.push(faces.len() - 1);
        }

        // Reassign orphaned points
        for idx in orphaned_points {
            let pt = pts[idx];
            let mut best_face = usize::MAX;
            let mut best_d = EPSILON;

            for &f_idx in &new_face_indices {
                let d = faces[f_idx].distance(pt, pts);
                if d > best_d {
                    best_d = d;
                    best_face = f_idx;
                }
            }

            if best_face != usize::MAX {
                faces[best_face].outside_set.push(idx);
            }
        }
    }

    // Extract unique vertices and mapped indices
    let mut vertex_set = HashSet::new();
    for f in &faces {
        vertex_set.insert(f.v[0]);
        vertex_set.insert(f.v[1]);
        vertex_set.insert(f.v[2]);
    }

    let mut out_vertices = Vec::new();
    let mut old_to_new = HashMap::new();

    for old_idx in vertex_set {
        out_vertices.push(pts[old_idx]);
        old_to_new.insert(old_idx, (out_vertices.len() - 1) as u32);
    }

    let mut out_faces = Vec::new();
    for f in &faces {
        out_faces.push([
            old_to_new[&f.v[0]],
            old_to_new[&f.v[1]],
            old_to_new[&f.v[2]],
        ]);
    }

    ConvexHull {
        vertices: out_vertices,
        faces: out_faces,
    }
}
