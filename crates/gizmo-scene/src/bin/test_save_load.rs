use gizmo_core::World;
use gizmo_physics::shape::Collider;
use gizmo_scene::registry::SceneRegistry;
use gizmo_scene::scene::SceneData;

fn main() {
    let mut world = World::new();
    // Zemin
    let ground = world.spawn();
    world.add_component(ground, gizmo_core::EntityName("Zemin".to_string()));
    world.add_component(
        ground,
        gizmo_physics::components::Transform::new(gizmo_math::Vec3::new(0.0, -1.0, 0.0))
            .with_scale(gizmo_math::Vec3::new(50.0, 1.0, 50.0)),
    );
    world.add_component(
        ground,
        Collider::aabb(gizmo_math::Vec3::new(50.0, 1.0, 50.0)),
    );
    world.add_component(ground, gizmo_physics::components::RigidBody::new_static());

    // Araba
    let car = world.spawn();
    world.add_component(car, gizmo_core::EntityName("Test_Arabasi".to_string()));
    world.add_component(
        car,
        gizmo_physics::components::Transform::new(gizmo_math::Vec3::new(0.0, 2.0, 0.0)),
    );
    world.add_component(
        car,
        gizmo_physics::components::Velocity::new(gizmo_math::Vec3::ZERO),
    );
    world.add_component(car, Collider::aabb(gizmo_math::Vec3::new(1.0, 0.5, 2.0)));

    let car_rb = gizmo_physics::components::RigidBody::new(1500.0, 0.5, 0.1, true);
    world.add_component(car, car_rb);

    // Kamera
    let cam = world.spawn();
    world.add_component(cam, gizmo_core::EntityName("Kamera_Dostum".to_string()));
    world.add_component(
        cam,
        gizmo_physics::components::Transform::new(gizmo_math::Vec3::new(0.0, 6.0, 12.0)),
    );
    world.add_component(
        cam,
        gizmo_renderer::components::Camera::new(1.047, 0.1, 1000.0, 0.0, -0.34, true),
    );

    // Işık (PointLight)
    let light = world.spawn();
    world.add_component(light, gizmo_core::EntityName("Isik_Kaynagi".to_string()));
    world.add_component(
        light,
        gizmo_physics::components::Transform::new(gizmo_math::Vec3::new(0.0, 5.0, 0.0)),
    );
    world.add_component(
        light,
        gizmo_renderer::components::PointLight::new(
            gizmo_math::Vec3::new(1.0, 1.0, 1.0),
            300.0,
            10.0,
        ),
    );

    // Save
    let mut registry = SceneRegistry::new();
    registry.register::<gizmo_physics::components::Transform>("Transform");
    registry.register::<gizmo_physics::components::Velocity>("Velocity");
    registry.register::<gizmo_physics::components::RigidBody>("RigidBody");
    registry.register::<gizmo_physics::shape::Collider>("Collider");

    registry.register::<gizmo_renderer::components::Camera>("Camera");
    registry.register::<gizmo_renderer::components::PointLight>("PointLight");

    if let Err(e) = SceneData::save(&world, "demo/assets/perfect_car.scene", &registry) {
        tracing::info!("Error: {}", e);
    } else {
        tracing::info!("perfect_car.scene created successfully!");
    }
}
