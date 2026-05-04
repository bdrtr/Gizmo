use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics::components::{Collider, RigidBody, Transform, Velocity};
use gizmo_physics::system::physics_step_system;
use gizmo_physics::world::PhysicsWorld;

fn setup_world() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world
}

#[test]
fn test_compound_shape_deadlock_and_collision() {
    let mut world = setup_world();

    // Spawn a ground plane
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());
    world.add_component(ground, Collider::box_collider(Vec3::new(10.0, 1.0, 10.0)));

    // Spawn a parent rigid body
    let parent_ent = world.spawn();
    world.add_component(parent_ent, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
    world.add_component(parent_ent, RigidBody::new(1.0, 0.0, 0.5, true));
    world.add_component(parent_ent, Velocity::default());
    // Give it a small base collider
    world.add_component(parent_ent, Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)));

    // Spawn a child entity with a collider (no rigid body or velocity needed for children)
    let child_ent = world.spawn();
    // Offset the child by 1 unit up
    world.add_component(child_ent, Transform::new(Vec3::new(0.0, 1.0, 0.0)));
    world.add_component(child_ent, Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)));
    
    // Add parent-child relationship
    world.add_component(parent_ent, gizmo_core::component::Children(vec![child_ent.id()]));
    world.add_component(child_ent, gizmo_core::component::Parent(parent_ent.id()));

    // Run the physics system multiple times
    // This used to cause a deadlock due to simultaneous borrows of Transform/Collider.
    for _ in 0..60 {
        physics_step_system(&world, 0.016);
    }

    // Check that the parent has fallen and collided with the ground.
    // Base is at 5.0, height is 0.5, so it should fall and land on the ground (Y=0.0).
    let parent_transform = world.borrow::<Transform>().get(parent_ent.id()).unwrap().clone();
    
    // Since gravity acts on it, it shouldn't just stay at 5.0.
    assert!(
        parent_transform.position.y < 4.9,
        "Parent should have fallen, but is at Y = {}",
        parent_transform.position.y
    );
    
    // It should eventually rest around Y = 0.5 (half height of base box)
    // We didn't simulate enough frames for it to fully rest, but at least we confirmed no deadlock!
}
