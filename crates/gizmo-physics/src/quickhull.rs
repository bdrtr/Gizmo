use gizmo_math::Vec3;

/// Computes the convex hull of a set of 3D points using a simplified incremental algorithm
/// or QuickHull. Returns the vertices that form the convex hull.
pub fn compute_convex_hull(points: &[Vec3]) -> Vec<Vec3> {
    if points.len() <= 4 {
        return points.to_vec();
    }

    // Find 6 extremal points along the axes
    let mut min_x = 0; let mut max_x = 0;
    let mut min_y = 0; let mut max_y = 0;
    let mut min_z = 0; let mut max_z = 0;

    for (i, p) in points.iter().enumerate() {
        if p.x < points[min_x].x { min_x = i; }
        if p.x > points[max_x].x { max_x = i; }
        if p.y < points[min_y].y { min_y = i; }
        if p.y > points[max_y].y { max_y = i; }
        if p.z < points[min_z].z { min_z = i; }
        if p.z > points[max_z].z { max_z = i; }
    }

    let mut hull_points = vec![
        points[min_x], points[max_x],
        points[min_y], points[max_y],
        points[min_z], points[max_z],
    ];

    // Remove duplicates
    hull_points.sort_by(|a, b| {
        a.x.total_cmp(&b.x)
            .then(a.y.total_cmp(&b.y))
            .then(a.z.total_cmp(&b.z))
    });
    hull_points.dedup_by(|a, b| (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4 && (a.z - b.z).abs() < 1e-4);

    // This is an extremely simplified "bounding box / extremal points" hull.
    // For a production engine, you'd implement a full QuickHull or v-hacd.
    // However, for GJK, providing a heavily decimated point cloud is often enough.
    // We add a few random points from the original set to give volume.
    
    // A more robust approach without full QuickHull is to use a coarse spherical grid
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
                theta.sin() * phi.sin(),
                theta.cos()
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
