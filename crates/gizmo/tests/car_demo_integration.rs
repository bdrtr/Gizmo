use gizmo_core::{world::World, system::Schedule};
use gizmo_physics_core::{Transform, Collider, components::GlobalTransform};
use gizmo_physics_rigid::{components::{RigidBody, Velocity}, physics_step_system, PhysicsWorld};
use gizmo_engine::systems::transform::TransformPropagateSystem;
use gizmo_math::Vec3;

#[test]
fn car_demo_full_frame() {
    let mut world = World::new();
    // Tüm component'leri kaydet
    world.register_component_type::<Transform>();
    world.register_component_type::<GlobalTransform>();
    world.register_component_type::<RigidBody>();
    world.register_component_type::<Velocity>();
    world.register_component_type::<Collider>();

    
    world.insert_resource(PhysicsWorld::new());
    world.insert_resource(gizmo_core::time::Time::new());
    world.insert_resource(gizmo_core::time::PhysicsTime::new(60));

    // Şasi spawn et
    let chassis = world.spawn();
    world.add_component(chassis, Transform { position: Vec3::new(0.0, 1.0, 0.0), ..Default::default() });
    world.add_component(chassis, GlobalTransform::default());
    world.add_component(chassis, RigidBody::new(200.0, true));
    world.add_component(chassis, Velocity::default());
    world.add_component(chassis, Collider::box_collider(Vec3::new(1.0, 1.0, 1.0)));

    
    // Bir frame simüle et
    let mut scheduler = Schedule::new();
    scheduler.add_system(TransformPropagateSystem);
    
    physics_step_system(&world, 0.016);
    scheduler.run(&mut world, 0.016);
    
    // Yerçekimi etkisiyle Y pozisyonu değişmeli
    let t_comps = world.borrow::<Transform>();
    let t = t_comps.get(chassis.id()).unwrap();
    assert!(t.position.y < 1.0); // düşüyor
}
