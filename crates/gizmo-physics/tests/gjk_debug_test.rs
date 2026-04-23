use gizmo_math::{Quat, Vec3};
use gizmo_physics::gjk::gjk_intersect;
use gizmo_physics::shape::{Aabb, ColliderShape, ConvexHull};

#[test]
fn test_gjk_debug() {
    let ground_shape = ColliderShape::Aabb(Aabb {
        half_extents: Vec3::new(50.0, 1.0, 50.0),
    });
    let ground_pos = Vec3::new(0.0, -10.0, 0.0);
    let ground_rot = Quat::IDENTITY;

    let cube_vertices = vec![
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
    ];
    let cube_shape = ColliderShape::ConvexHull(ConvexHull {
        vertices: cube_vertices,
    });
    let cube_pos = Vec3::new(-14.48, -9.33, 0.09);
    let cube_rot = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 30.0_f32.to_radians());

    let (hit, sim) = gjk_intersect(
        &ground_shape,
        ground_pos,
        ground_rot,
        &cube_shape,
        cube_pos,
        cube_rot,
    );
    println!("Hit: {}, Simplex size: {}", hit, sim.size);
    assert!(hit);
}

#[test]
fn test_epa_debug() {
    let ground_shape = ColliderShape::Aabb(Aabb {
        half_extents: Vec3::new(50.0, 1.0, 50.0),
    });
    let ground_pos = Vec3::new(0.0, -10.0, 0.0);
    let ground_rot = Quat::IDENTITY;

    let cube_vertices = vec![
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
    ];
    let cube_shape = ColliderShape::ConvexHull(gizmo_physics::shape::ConvexHull {
        vertices: cube_vertices,
    });
    let cube_pos = Vec3::new(-14.48, -9.33, 0.09);
    let cube_rot = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 30.0_f32.to_radians());

    let (_hit, sim) = gjk_intersect(
        &ground_shape,
        ground_pos,
        ground_rot,
        &cube_shape,
        cube_pos,
        cube_rot,
    );
    let manifold = gizmo_physics::epa::epa_solve(
        sim,
        &ground_shape,
        ground_pos,
        ground_rot,
        &cube_shape,
        cube_pos,
        cube_rot,
    );

    println!(
        "Manifold is_colliding: {}, penetration: {}, normal: {:?}, num_points: {}",
        manifold.is_colliding,
        manifold.penetration,
        manifold.normal,
        manifold.contact_points.len()
    );
}
