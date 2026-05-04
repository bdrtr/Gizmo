use gizmo_math::Vec3;

/// Computes an approximated convex hull (extremal points) of a set of 3D points.
/// Returns the vertices that form the boundary of the shape.
pub fn compute_convex_hull_approximation(points: &[Vec3]) -> Vec<Vec3> {
    if points.len() < 4 {
        return points.to_vec();
    }

    // This is an extremely simplified "bounding box / extremal points" hull.
    // For a production engine, you'd implement a full QuickHull or v-hacd to get faces.
    // However, for GJK, providing a decimated point cloud is often enough.
    
    // A robust approach without full QuickHull is to use a coarse spherical grid
    // and take the furthest point in each grid direction.
    let mut extreme_points = Vec::new();
    let num_lat = 8;
    let num_lon = 16;
    
    for lat in 0..num_lat {
        let theta = std::f32::consts::PI * (lat as f32) / ((num_lat - 1) as f32);
        for lon in 0..num_lon {
            let phi = 2.0 * std::f32::consts::PI * (lon as f32) / (num_lon as f32);
            let dir = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin()
            );
            
            let mut best_dot = f32::NEG_INFINITY;
            let mut best_pt = points[0];
            for p in points {
                let d = p.dot(dir);
                if d > best_dot {
                    best_dot = d;
                    best_pt = *p;
                }
            }
            extreme_points.push(best_pt);
        }
    }

    extreme_points.sort_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    });
    extreme_points.dedup_by(|a, b| (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4 && (a.z - b.z).abs() < 1e-4);
    
    extreme_points
}
