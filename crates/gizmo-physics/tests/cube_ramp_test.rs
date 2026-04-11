use gizmo_math::{Quat, Vec3};
use gizmo_physics::components::Transform;
use gizmo_physics::shape::Collider;

#[test]
fn test_cube_ramp_collision() {
    let rot_z = 30.0_f32.to_radians(); // 30 derece

    // Rampa
    let mut ramp_t = Transform::new(Vec3::new(0.0, 0.0, 0.0));
    ramp_t.scale = Vec3::new(10.0, 0.5, 5.0);
    ramp_t.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), rot_z);

    let ramp_vertices = vec![
        Vec3::new(-10.0, -0.5, -5.0),
        Vec3::new(10.0, -0.5, -5.0),
        Vec3::new(10.0, 0.5, -5.0),
        Vec3::new(-10.0, 0.5, -5.0),
        Vec3::new(-10.0, -0.5, 5.0),
        Vec3::new(10.0, -0.5, 5.0),
        Vec3::new(10.0, 0.5, 5.0),
        Vec3::new(-10.0, 0.5, 5.0),
    ];
    let ramp_col = Collider::new_convex(ramp_vertices);

    // Küp (-1, 1 arasında standart)
    let box_x = -(rot_z.cos() * 8.0);
    let box_y = (rot_z.sin() * -8.0).abs() + 2.0;
    let mut cube_t = Transform::new(Vec3::new(box_x, box_y, 0.0));
    cube_t.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), rot_z);

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
    let cube_col = Collider::new_convex(cube_vertices);

    // Küpü rampa üzerine bırak (y ekseninden düşürüyoruz gibi Y=0 olana kadar kontrol edelim)
    cube_t.position.y = 0.5; // Kesin çarpışması gereken bir nokta
    cube_t.position.x = 0.0;

    println!(
        "Ramp Pos: {:?}, Cube Pos: {:?}",
        ramp_t.position, cube_t.position
    );

    let (is_colliding, simplex) = gizmo_physics::gjk::gjk_intersect(
        &cube_col.shape,
        cube_t.position,
        cube_t.rotation,
        &ramp_col.shape,
        ramp_t.position,
        ramp_t.rotation,
    );

    println!("GJK Kesişim Var Mı?: {}", is_colliding);

    if is_colliding {
        let manifold = gizmo_physics::epa::epa_solve(
            simplex,
            &cube_col.shape,
            cube_t.position,
            cube_t.rotation,
            &ramp_col.shape,
            ramp_t.position,
            ramp_t.rotation,
        );
        println!(
            "Manifold Normal: {:?}, Penetration: {}",
            manifold.normal, manifold.penetration
        );
        println!("Contact Points: {:?}", manifold.contact_points);
    } else {
        println!("GJK FAILED TO DETECT COLLISION EVEN WHEN FORCED!");
        assert!(false, "GJK missed the collision!");
    }
}

#[test]
fn test_cube_edge_collision() {
    let rot_z = 30.0_f32.to_radians();

    let mut ramp_t = Transform::new(Vec3::new(0.0, 0.0, 0.0));
    ramp_t.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), rot_z);

    let ramp_vertices = vec![
        Vec3::new(-10.0, -0.5, -5.0),
        Vec3::new(10.0, -0.5, -5.0),
        Vec3::new(10.0, 0.5, -5.0),
        Vec3::new(-10.0, 0.5, -5.0),
        Vec3::new(-10.0, -0.5, 5.0),
        Vec3::new(10.0, -0.5, 5.0),
        Vec3::new(10.0, 0.5, 5.0),
        Vec3::new(-10.0, 0.5, 5.0),
    ];
    let ramp_col = Collider::new_convex(ramp_vertices);

    let mut cube_t = Transform::new(Vec3::new(8.5, 4.0, 0.0)); // Edge of the ramp
    cube_t.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), rot_z);

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
    let cube_col = Collider::new_convex(cube_vertices);

    let (is_colliding, simplex) = gizmo_physics::gjk::gjk_intersect(
        &cube_col.shape,
        cube_t.position,
        cube_t.rotation,
        &ramp_col.shape,
        ramp_t.position,
        ramp_t.rotation,
    );

    // Yüzey değil kenar kesişimlerinde de GJK başarılı olmalı
    println!("GJK Kesişim Var Mı (Edge)?: {}", is_colliding);
    if is_colliding {
        let manifold = gizmo_physics::epa::epa_solve(
            simplex,
            &cube_col.shape,
            cube_t.position,
            cube_t.rotation,
            &ramp_col.shape,
            ramp_t.position,
            ramp_t.rotation,
        );
        println!(
            "Edge Normal: {:?}, Edge Penetration: {}",
            manifold.normal, manifold.penetration
        );
        println!("Edge Contact Points: {:?}", manifold.contact_points);
    }
}

#[test]
fn test_box_drop_straight() {
    let mut ramp_t = Transform::new(Vec3::new(0.0, 0.0, 0.0));
    ramp_t.scale = Vec3::new(10.0, 0.5, 10.0);
    ramp_t.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 0.0);

    let ramp_vertices = vec![
        Vec3::new(-10.0, -0.5, -10.0),
        Vec3::new(10.0, -0.5, -10.0),
        Vec3::new(10.0, 0.5, -10.0),
        Vec3::new(-10.0, 0.5, -10.0),
        Vec3::new(-10.0, -0.5, 10.0),
        Vec3::new(10.0, -0.5, 10.0),
        Vec3::new(10.0, 0.5, 10.0),
        Vec3::new(-10.0, 0.5, 10.0),
    ];
    let ramp_col = Collider::new_convex(ramp_vertices);

    // Box right above it
    let mut cube_t = Transform::new(Vec3::new(0.0, 1.0, 0.0));
    cube_t.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 0.0);

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
    let cube_col = Collider::new_convex(cube_vertices);

    let (is_colliding, simplex) = gizmo_physics::gjk::gjk_intersect(
        &cube_col.shape,
        cube_t.position,
        cube_t.rotation,
        &ramp_col.shape,
        ramp_t.position,
        ramp_t.rotation,
    );

    println!("\n--- STRAIGHT DROP TEST ---");
    println!("GJK Kesişim Var Mı (Edge)?: {}", is_colliding);
    if is_colliding {
        let manifold = gizmo_physics::epa::epa_solve(
            simplex,
            &cube_col.shape,
            cube_t.position,
            cube_t.rotation,
            &ramp_col.shape,
            ramp_t.position,
            ramp_t.rotation,
        );
        println!(
            "Normal: {:?}, Penetration: {}",
            manifold.normal, manifold.penetration
        );
        println!("Contact Points: {:?}", manifold.contact_points);
        println!("--------------------------\n");
    }
}
