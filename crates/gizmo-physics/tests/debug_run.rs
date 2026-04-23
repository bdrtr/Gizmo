use gizmo_math::{Quat, Vec3};
use gizmo_physics::components::Transform;
use gizmo_physics::shape::Collider;
use gizmo_physics::shape::{Aabb, ColliderShape, ConvexHull};
use gizmo_physics::system::broad_phase;

#[test]
fn test_broadphase_debug() {
    let mut world = gizmo_core::world::World::new();

    let mut t_ground = Transform::new(Vec3::new(0.0, -10.0, 0.0));
    t_ground.scale = Vec3::new(50.0, 1.0, 50.0);

    let e0 = world.spawn();
    world.add_component(e0, t_ground);
    world.add_component(e0, Collider::new_aabb(50.0, 1.0, 50.0));

    let mut t_cube = Transform::new(Vec3::new(-14.48, -9.14, 0.09));
    t_cube.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 30.0_f32.to_radians());
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
    let e1 = world.spawn();
    world.add_component(e1, t_cube);
    world.add_component(e1, Collider::new_convex(cube_vertices));
    world.add_component(
        e1,
        gizmo_physics::components::RigidBody::new(1.0, 0.5, 0.5, false),
    );

    let t_view = world.borrow::<Transform>();
    let c_view = world.borrow::<Collider>();
    let rb_view = world.borrow::<gizmo_physics::components::RigidBody>();
    let v_view = world.borrow::<gizmo_physics::components::Velocity>();

    let dt = 1.0 / 60.0;
    let parallel_physics = false;
    let pairs = broad_phase(&t_view, &c_view, &rb_view, &v_view, dt, parallel_physics);
    println!("Broadphase pairs: {:?}", pairs);
    assert!(!pairs.is_empty());
}
