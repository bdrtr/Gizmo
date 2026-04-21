use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics::character::{physics_character_system, CharacterController};
use gizmo_physics::components::{RigidBody, Transform};
use gizmo_physics::shape::Collider;

fn setup_world() -> World {
    World::new()
}

#[test]
fn test_character_gravity_and_grounding() {
    let mut world = setup_world();

    // Zemin
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Collider::new_aabb(10.0, 1.0, 10.0)); // Yarı yükseklik 1.0 -> Üst yüzey Y=0

    // Karakter
    let char_entity = world.spawn();
    // Kapsül boyutu: Radius=0.5, HalfHeight=1.0. Karakterin tabanı (Y - 1.5).
    // Eğer Y=2.0 ise tabanı Y=0.5. Zemin Y=0
    world.add_component(char_entity, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
    world.add_component(char_entity, CharacterController::new(0.5, 1.0));

    // Step 1: Yerçekimi Serbest Düşüş
    let dt = 0.1; // 0.1 saniye
    physics_character_system(&world, dt);

    let _t = world
        .borrow::<Transform>()
        .get(char_entity.id())
        .unwrap()
        .clone();
    let cc = world
        .borrow::<CharacterController>()
        .get(char_entity.id())
        .unwrap()
        .clone();
    assert!(!cc.is_grounded, "Havada olmalı");
    assert!(
        cc.vertical_velocity < -0.9,
        "Yerçekimi ile düşmeli: {}",
        cc.vertical_velocity
    );

    // Step 2: Zemine çarpma
    // Tam yere çarpması için yeterli zaman geçmeli (yaklaşık 2 saniye)
    for _ in 0..30 {
        physics_character_system(&world, 0.1);
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
    assert_eq!(cc2.ground_normal, Vec3::new(0.0, 1.0, 0.0)); // Zemin düz
                                                             // Karakterin Y pozisyonu = Zemin Üstü (0.0) + Yarıçap (0.5) + HalfHeight (1.0) = 1.5
    assert!(
        (t2.position.y - 1.5).abs() < 0.01 + cc2.skin_width,
        "Yere tam oturmalı: {}",
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
    world.add_component(ground, Collider::new_aabb(10.0, 1.0, 10.0)); // Üst yüzey Y=0

    // Basamak (Karakterin tam önünde, yüksekliği 0.2m)
    // Karakter X = 0. Basamak X = 1.5
    let step = world.spawn();
    // Y=0.1, Yarı uzunluk 0.1 -> Üst yüzey Y=0.2
    world.add_component(step, Transform::new(Vec3::new(1.5, 0.1, 0.0)));
    world.add_component(step, RigidBody::new_static());
    world.add_component(step, Collider::new_aabb(0.5, 0.1, 1.0));

    // Karakter
    let char_entity = world.spawn();
    world.add_component(char_entity, Transform::new(Vec3::new(0.0, 1.52, 0.0)));
    let mut cc = CharacterController::new(0.5, 1.0);
    cc.is_grounded = true; // Yerde varsay
    cc.desired_velocity = Vec3::new(5.0, 0.0, 0.0); // Basamağa doğru sertçe git
    world.add_component(char_entity, cc);

    physics_character_system(&world, 0.1);

    let t = world
        .borrow::<Transform>()
        .get(char_entity.id())
        .unwrap()
        .clone();
    // Basamağa (X=1.0 sınırında) çıkıp Y=0.2'ye tırmanmalı, dolayısıyla pozisyon Y -> 1.7 civarında olmalı
    assert!(
        t.position.y > 1.6,
        "Basamağa tırmanamadı, Y={}",
        t.position.y
    );
}

#[test]
fn test_character_slope_sliding() {
    let mut world = setup_world();
    world.insert_resource(gizmo_physics::components::PhysicsConfig { ground_y: -100.0, ..Default::default() });

    // Zemin olarak devasa bir Küre kullanalım (AABB'ler rotation almaz, bu yüzden eğim oluşturmak için küre şart).
    // Küre yarıçapı 10.0. Merkezi orijinde (0,0,0).
    let slope = world.spawn();
    world.add_component(slope, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(slope, RigidBody::new_static());
    world.add_component(slope, Collider::new_sphere(10.0));

    // Karakteri kürenin 60 derecelik eğimine koyalım.
    // 60 derece: Y = R * cos(60) = 10 * 0.5 = 5.0
    // X = R * sin(60) = 10 * 0.866 = 8.66
    // Yüzey normali A'dan B'ye doğru yani Karakterden Küreye doğru: (-0.866, -0.5, 0.0).
    // Karakterin merkezi: Çapı 0.5 olduğu için +0.5 normal yönünde.
    // X = 8.66 + 0.866 * 1.5 = 9.959
    // Y = 5.0 + 0.5 * 1.5 = 5.75
    let char_entity = world.spawn();
    // Karakterin merkezi (Yarım yükseklik 1.0 + Yarıçap 0.5 = 1.5 birim yüzeyden yukarı)
    let char_pos = Vec3::new(8.66, 5.0, 0.0) + Vec3::new(0.866, 0.5, 0.0) * 1.5;
    world.add_component(char_entity, Transform::new(char_pos));

    let mut cc = CharacterController::new(0.5, 1.0);
    // 60 derece, 45'lik limiti aşıyor.
    cc.slope_limit = 45.0;
    world.add_component(char_entity, cc);

    // Kaymasını izlemek için adımlar at
    for _ in 0..15 {
        physics_character_system(&world, 0.1);
    }

    let cc_out = world
        .borrow::<CharacterController>()
        .get(char_entity.id())
        .unwrap()
        .clone();
    let t_out = world
        .borrow::<Transform>()
        .get(char_entity.id())
        .unwrap()
        .clone();

    // Eğim aşıldığı için karakter dengesizleşmeli (is_grounded false olmalı)
    assert!(
        !cc_out.is_grounded,
        "Eğimi aştığı için yere bağlı sayılamaz! Normal: {:?}, vertical_velocity: {}", cc_out.ground_normal, cc_out.vertical_velocity
    );
    // Yerçekimi ve slope sliding yüzünden daha da sağa/aşağı kaymış olmalı (X > 9.959, Y < 5.75)
    assert!(
        t_out.position.y < 5.7,
        "Rampadan aşağı doğru kayamadı! Y={}",
        t_out.position.y
    );
}
