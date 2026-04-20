use gizmo_math::{Vec3, Quat};
use gizmo_physics::shape::{ColliderShape, Aabb, ConvexHull};
use gizmo_physics::shape::Collider;
use gizmo_physics::components::Transform;
use gizmo_physics::system::broad_phase;

#[test]
fn test_broadphase_debug() {
    let mut transforms = gizmo_core::SparseSet::new();
    let mut colliders = gizmo_core::SparseSet::new();

    let mut t_ground = Transform::new(Vec3::new(0.0, -10.0, 0.0));
    t_ground.scale = Vec3::new(50.0, 1.0, 50.0);
    transforms.insert(0, t_ground);
    colliders.insert(0, Collider::new_aabb(50.0, 1.0, 50.0));

    let mut t_cube = Transform::new(Vec3::new(-14.48, -9.14, 0.09));
    t_cube.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), 30.0_f32.to_radians());
    let cube_vertices = vec![
        Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0), Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0), Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0), Vec3::new(-1.0, 1.0, 1.0),
    ];
    transforms.insert(1, t_cube);
    colliders.insert(1, Collider::new_convex(cube_vertices));

    let rigidbodies = gizmo_core::SparseSet::new();
    let velocities = gizmo_core::SparseSet::new();
    let dt = 1.0/60.0;
    let parallel_physics = false;
    let pairs = broad_phase(&transforms, &colliders, &rigidbodies, &velocities, dt, parallel_physics);
    println!("Broadphase pairs: {:?}", pairs);
    assert!(!pairs.is_empty());
}
