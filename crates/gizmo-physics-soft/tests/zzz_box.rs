use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_soft::Cloth;

// Deepest penetration of the cloth SURFACE (bilinear per quad) into an axis-aligned box.
fn box_surface_pen(cloth: &Cloth, grid: usize, center: Vec3, he: Vec3, thickness: f32) -> (f32, Vec3) {
    let heb = he + Vec3::splat(thickness);
    let get = |x: usize, y: usize| cloth.nodes[y * grid + x].position;
    let mut pen = 0.0f32;
    let mut worst = Vec3::ZERO;
    for y in 0..grid - 1 {
        for x in 0..grid - 1 {
            let (p00, p10, p01, p11) = (get(x, y), get(x + 1, y), get(x, y + 1), get(x + 1, y + 1));
            let k = 6;
            for iu in 0..=k {
                for iv in 0..=k {
                    let (u, v) = (iu as f32 / k as f32, iv as f32 / k as f32);
                    let p = p00 * ((1.0 - u) * (1.0 - v))
                        + p10 * (u * (1.0 - v))
                        + p01 * ((1.0 - u) * v)
                        + p11 * (u * v);
                    let l = p - center;
                    if l.x.abs() < heb.x && l.y.abs() < heb.y && l.z.abs() < heb.z {
                        let d = (heb.x - l.x.abs()).min(heb.y - l.y.abs()).min(heb.z - l.z.abs());
                        if d > pen {
                            pen = d;
                            worst = p;
                        }
                    }
                }
            }
        }
    }
    (pen, worst)
}

#[test]
fn probe_box_drape() {
    let thickness = 0.02f32;
    let center = Vec3::new(0.0, 1.3, 0.0);
    let he = Vec3::new(1.15, 1.15, 1.15);
    let colliders = vec![
        (BodyHandle::from_id(1), Transform::new(center), Collider::box_collider(he)),
        (
            BodyHandle::from_id(2),
            Transform::new(Vec3::new(0.0, -0.25, 0.0)),
            Collider::box_collider(Vec3::new(20.0, 0.25, 20.0)),
        ),
    ];
    for &segs in &[1usize, 2, 4, 6, 10] {
        let grid = segs + 1;
        let size = 3.4f32;
        let sp = size / segs as f32;
        let mut cloth = Cloth::new(grid, grid, sp, 1.0);
        cloth.thickness = thickness;
        cloth.friction = 0.4;
        let half = size * 0.5;
        for (i, node) in cloth.nodes.iter_mut().enumerate() {
            let x = (i % grid) as f32 * sp - half;
            let z = (i / grid) as f32 * sp - half;
            node.position = Vec3::new(x, 3.6, z);
            node.prev_position = node.position;
        }
        for _ in 0..700 {
            cloth.step(1.0 / 60.0, Vec3::new(0.0, -9.81, 0.0), 10, &colliders);
        }
        let (pen, worst) = box_surface_pen(&cloth, grid, center, he, thickness);
        let nan = cloth.nodes.iter().any(|n| !n.position.is_finite());
        println!(
            "segs={segs} grid={grid} sp={sp:.3}: box_surface_pen={pen:.4} (thick={thickness}) worst=({:.2},{:.2},{:.2}) nan={nan}",
            worst.x, worst.y, worst.z
        );
    }
}
