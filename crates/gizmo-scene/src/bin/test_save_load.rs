use gizmo_core::World;
use gizmo_physics::shape::Collider;
use gizmo_scene::registry::SceneRegistry;
use gizmo_scene::scene::SceneData;

fn main() {
    let mut world = World::new();
    // Zemin
    let ground = world.spawn();
    world.add_component(ground, gizmo_core::EntityName("Zemin".to_string()));
    world.add_component(ground, gizmo_physics::components::Transform::new(gizmo_math::Vec3::new(0.0, -1.0, 0.0)).with_scale(gizmo_math::Vec3::new(50.0, 1.0, 50.0)));
    world.add_component(ground, Collider::new_aabb(50.0, 1.0, 50.0));
    world.add_component(ground, gizmo_physics::components::RigidBody::new(0.0, 0.8, 0.2, true));

    // Araba
    let car = world.spawn();
    world.add_component(car, gizmo_core::EntityName("Test_Arabasi".to_string()));
    world.add_component(car, gizmo_physics::components::Transform::new(gizmo_math::Vec3::new(0.0, 2.0, 0.0)));
    world.add_component(car, gizmo_physics::components::Velocity::new(gizmo_math::Vec3::ZERO));
    world.add_component(car, Collider::new_aabb(1.0, 0.5, 2.0));
    
    let mut car_rb = gizmo_physics::components::RigidBody::new(1.0, 0.5, 0.1, false);
    car_rb.calculate_box_inertia(1.0, 0.5, 2.0);
    world.add_component(car, car_rb);

    let vc = gizmo_physics::vehicle::VehicleController::new();
    world.add_component(car, vc);

    let mut children = Vec::new();
    let wheel_positions = [
        (-1.0, -0.5,  1.5, false),
        ( 1.0, -0.5,  1.5, false),
        (-1.0, -0.5, -1.5, true),
        ( 1.0, -0.5, -1.5, true),
    ];

    for (x, y, z, drive) in wheel_positions {
        let w = world.spawn();
        world.add_component(w, gizmo_core::EntityName(format!("Wheel_{}_{}", x, z)));
        world.add_component(w, gizmo_physics::components::Transform::new(gizmo_math::Vec3::new(x, y, z)));
        let mut wc = gizmo_physics::vehicle::WheelComponent::new(1.0, 20.0, 2.0, 0.4);
        if drive { wc = wc.with_drive() }
        world.add_component(w, wc);
        world.add_component(w, gizmo_core::component::Parent(car.id()));
        children.push(w.id());
    }
    world.add_component(car, gizmo_core::component::Children(children));

    // Kamera
    let cam = world.spawn();
    world.add_component(cam, gizmo_core::EntityName("Kamera_Dostum".to_string()));
    world.add_component(cam, gizmo_physics::components::Transform::new(gizmo_math::Vec3::new(0.0, 6.0, 12.0)));
    world.add_component(cam, gizmo_renderer::components::Camera::new(1.047, 0.1, 1000.0, 0.0, -0.34, true));
    
    // Işık (PointLight)
    let light = world.spawn();
    world.add_component(light, gizmo_core::EntityName("Isik_Kaynagi".to_string()));
    world.add_component(light, gizmo_physics::components::Transform::new(gizmo_math::Vec3::new(0.0, 5.0, 0.0)));
    world.add_component(
        light,
        gizmo_renderer::components::PointLight::new(gizmo_math::Vec3::new(1.0, 1.0, 1.0), 300.0),
    );

    // Save
    let mut registry = SceneRegistry::new();
    registry.register::<gizmo_physics::components::Transform>("Transform");
    registry.register::<gizmo_physics::components::Velocity>("Velocity");
    registry.register::<gizmo_physics::components::RigidBody>("RigidBody");
    registry.register::<gizmo_physics::shape::Collider>("Collider");
    registry.register::<gizmo_physics::vehicle::VehicleController>("VehicleController");
    registry.register::<gizmo_physics::vehicle::WheelComponent>("WheelComponent");
    registry.register::<gizmo_renderer::components::Camera>("Camera");
    registry.register::<gizmo_renderer::components::PointLight>("PointLight");
    
    if let Err(e) = SceneData::save(&world, "demo/assets/perfect_car.scene", &registry) {
        println!("Error: {}", e);
    } else {
        println!("perfect_car.scene created successfully!");
    }
}
