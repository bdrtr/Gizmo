use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics::components::{CharacterController, Collider, RigidBody, Transform, Velocity};
use gizmo_physics::system::physics_step_system;
use gizmo_physics::world::PhysicsWorld;

fn setup_world() -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world
}

#[test]
fn test_character_gravity_and_grounding() {
    let mut world = setup_world();

    // Zemin
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());
    world.add_component(ground, Collider::box_collider(Vec3::new(10.0, 1.0, 10.0))); // Yarı yükseklik 1.0 -> Üst yüzey Y=0

    // Karakter
    let char_entity = world.spawn();
    world.add_component(char_entity, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
    let cc = CharacterController::default();
    world.add_component(char_entity, cc);
    world.add_component(char_entity, Collider::capsule(0.5, 1.0));
    world.add_component(char_entity, RigidBody::new_kinematic());
    world.add_component(char_entity, Velocity::default());

    // Step 1: Yerçekimi Serbest Düşüş
    physics_step_system(&world, 0.1);

    let cc = world
        .borrow::<CharacterController>()
        .get(char_entity.id())
        .unwrap()
        .clone();
    let vel = world
        .borrow::<Velocity>()
        .get(char_entity.id())
        .unwrap()
        .clone();
    assert!(!cc.is_grounded, "Havada olmalı");
    assert!(
        vel.linear.y < -0.9,
        "Yerçekimi ile düşmeli: {}",
        vel.linear.y
    );

    // Step 2: Zemine çarpma
    for _ in 0..30 {
        physics_step_system(&world, 0.1);
    }

    let t2 = world
        .borrow::<Transform>()
        .get(char_entity.id())
        .unwrap()
        .clone();
    let cc2 = world
        .borrow::<CharacterController>()
        .get(char_entity.id())
        .unwrap()
        .clone();

    assert!(cc2.is_grounded, "Artık yere değmeli");
    assert!(
        (t2.position.y - 1.5).abs() < 0.1,
        "Yere tam oturmalı (Yarıçap 0.5 + HalfHeight 1.0 = 1.5): {}",
        t2.position.y
    );
}

#[test]
fn test_character_step_climbing() {
    let mut world = setup_world();

    // Zemin
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Velocity::default());
    world.add_component(ground, Collider::box_collider(Vec3::new(10.0, 1.0, 10.0))); // Üst yüzey Y=0

    // Basamak (Karakterin tam önünde, yüksekliği 0.2m)
    // Karakter X = 0. Basamak X = 1.5
    let step = world.spawn();
    world.add_component(step, Transform::new(Vec3::new(1.5, 0.1, 0.0)));
    world.add_component(step, RigidBody::new_static());
    world.add_component(step, Velocity::default());
    world.add_component(step, Collider::box_collider(Vec3::new(0.5, 0.1, 1.0))); // Üst yüzey Y=0.2

    // Karakter
    let char_entity = world.spawn();
    world.add_component(char_entity, Transform::new(Vec3::new(0.0, 1.02, 0.0)));
    let mut cc = CharacterController::default();
    cc.is_grounded = true;
    cc.target_velocity = Vec3::new(5.0, 0.0, 0.0); // Basamağa doğru sertçe git
    world.add_component(char_entity, cc);
    world.add_component(char_entity, Collider::capsule(0.5, 1.0));
    world.add_component(char_entity, RigidBody::new_kinematic());
    world.add_component(char_entity, Velocity::default());

    for _ in 0..10 {
        physics_step_system(&world, 0.1);
    }

    let t = world
        .borrow::<Transform>()
        .get(char_entity.id())
        .unwrap()
        .clone();
    
    // Basamağa (X=1.0 sınırında) çıkıp Y=0.2'ye tırmanmalı, dolayısıyla pozisyon Y -> 1.2 civarında olmalı
    assert!(
        t.position.y > 1.1,
        "Basamağa tırmanamadı, Y={}",
        t.position.y
    );
}

#[test]
fn test_character_slope_sliding() {
    let mut world = setup_world();

    // Küre yerine eğimli bir Box (AABB değil, rotasyonlu)
    let slope = world.spawn();
    // 60 derece eğim
    let rot = gizmo_math::Quat::from_rotation_z(std::f32::consts::PI / 3.0);
    world.add_component(slope, Transform::new(Vec3::new(0.0, 0.0, 0.0)).with_rotation(rot));
    world.add_component(slope, RigidBody::new_static());
    world.add_component(slope, Velocity::default());
    world.add_component(slope, Collider::box_collider(Vec3::new(10.0, 1.0, 10.0)));

    let char_entity = world.spawn();
    // Karakteri eğimli yüzeyin üstüne bırakalım
    world.add_component(char_entity, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
    
    let mut cc = CharacterController::default();
    cc.max_slope_angle = 45.0_f32.to_radians(); // 60 derece eğim var, bu limiti aşıyor
    world.add_component(char_entity, cc);
    world.add_component(char_entity, Collider::capsule(0.5, 1.0));
    world.add_component(char_entity, RigidBody::new_kinematic());
    world.add_component(char_entity, Velocity::default());

    for _ in 0..15 {
        physics_step_system(&world, 0.1);
    }

    let _cc_out = world
        .borrow::<CharacterController>()
        .get(char_entity.id())
        .unwrap()
        .clone();
    let t_out = world
        .borrow::<Transform>()
        .get(char_entity.id())
        .unwrap()
        .clone();

    // Yükseklik 5'ten az olmalı çünkü aşağı kaydı.
    assert!(
        t_out.position.y < 4.5,
        "Rampadan aşağı doğru kayamadı! Y={}",
        t_out.position.y
    );
}
