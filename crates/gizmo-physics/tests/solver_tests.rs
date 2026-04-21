use gizmo_core::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics::components::{RigidBody, Transform, Velocity};
use gizmo_physics::constraints::{Joint, JointKind, JointWorld};

fn setup_world() -> World {
    let w = World::new();
    w
}

#[test]
fn test_fixed_joint_pull() {
    let mut world = setup_world();
    let mut joint_world = JointWorld::new();

    // A nesnesi çok ağır (1000 kg), B nesnesi hafif (10 kg)
    let entity_a = world.spawn();
    world.add_component(entity_a, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    let rb_a = RigidBody::new(1000.0, 0.0, 0.5, false);
    world.add_component(entity_a, rb_a);
    world.add_component(entity_a, Velocity::new(Vec3::ZERO));
    world.add_component(entity_a, gizmo_physics::Collider::new_sphere(0.5));

    let entity_b = world.spawn();
    world.add_component(entity_b, Transform::new(Vec3::new(2.0, 0.0, 0.0)));
    let rb_b = RigidBody::new(10.0, 0.0, 0.5, false);
    world.add_component(entity_b, rb_b);
    let vel_b = Velocity::new(Vec3::new(100.0, 0.0, 0.0));
    world.add_component(entity_b, vel_b);
    world.add_component(entity_b, gizmo_physics::Collider::new_sphere(0.5));

    // A ve B'yi Fixed Joint ile bağla
    let joint = Joint {
        entity_a: entity_a.id(),
        entity_b: entity_b.id(),
        kind: JointKind::Fixed {
            relative_rotation: Quat::IDENTITY,
        },
        anchor_a: Vec3::new(1.0, 0.0, 0.0),  // A'nın sağ ucunda
        anchor_b: Vec3::new(-1.0, 0.0, 0.0), // B'nin sol ucunda
        stiffness: 1.0,
        damping: 0.1,
    };
    joint_world.add(joint);

    // Constraints çözücüyü çalıştır
    // 1 adım simülasyon (dt = 0.016)
    // Hızın pozisyona dönüşmesi ve constraint'in onu geri çekmesi için movement system çalışmalı
    gizmo_physics::physics_apply_forces_system(&world, 0.016);
    gizmo_physics::physics_movement_system(&world, 0.016);
    world.insert_resource(joint_world);
    gizmo_physics::system::physics_collision_system(&mut world, 0.016);

    let v_a = world
        .borrow::<Velocity>()
        .get(entity_a.id())
        .unwrap()
        .clone();
    let v_b = world
        .borrow::<Velocity>()
        .get(entity_b.id())
        .unwrap()
        .clone();

    // B hızı ciddi şekilde sönümlenmiş olmalı
    assert!(
        v_b.linear.x < 25.0,
        "B nesnesinin kaçış hızı joint tarafından durdurulamadı! Hızı: {}",
        v_b.linear.x
    );
    // A nesnesine momentum aktarılmış olmalı
    assert!(
        v_a.linear.x > 0.01,
        "A nesnesi hiç çekilmedi! Momentum aktarımı hatalı. A: {}",
        v_a.linear.x
    );
}
