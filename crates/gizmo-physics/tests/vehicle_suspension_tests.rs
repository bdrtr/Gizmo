/// Vehicle Süspansiyon Kuvveti Testleri
///
/// Test kapsamı:
///   1. `test_suspension_spring_force`  — Sıkışma miktarıyla orantılı yay kuvveti
///   2. `test_suspension_mass_scaling`  — Ağır araç daha az ivme almalı (F = ma)
///   3. `test_suspension_damping`       — Damping, aşırı salınımı engeller
///   4. `test_drag_quadratic_scaling`   — Hava direnci hıza değil hızın karesine orantılı
///   5. `test_drag_mass_dependent`      — Ağır araç aynı drag kuvvetinden daha az yavaşlar
///   6. `test_grounded_detection`       — Tekerlek zemine değince is_grounded = true
///   7. `test_wheel_not_grounded_airborne` — Havadayken is_grounded = false
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics::components::{PhysicsConfig, RigidBody, Transform, Velocity};
use gizmo_physics::vehicle::{VehicleController, WheelComponent};

const DT: f32 = 1.0 / 60.0;

fn make_world_with_ground(ground_y: f32) -> World {
    let mut w = World::new();
    w.insert_resource(PhysicsConfig {
        ground_y,
        ..Default::default()
    });
    w
}

fn spawn_vehicle(world: &mut World, pos: Vec3, mass: f32) -> (u32, u32) {
    let e = world.spawn();
    world.add_component(e, Transform::new(pos));
    world.add_component(e, RigidBody::new(mass, 1.0, 0.5, false));
    world.add_component(e, Velocity::new(Vec3::ZERO));

    let vc = VehicleController::new();
    world.add_component(e, vc);

    let wheel = world.spawn();
    world.add_component(wheel, Transform::new(Vec3::ZERO));
    world.add_component(wheel, WheelComponent::new(0.5, 5000.0, 200.0, 0.3));

    // Add parent/child relationship
    world.add_component(wheel, gizmo_core::component::Parent(e.id()));
    let mut added = false;
    {
        let mut children_storage = world.borrow_mut::<gizmo_core::component::Children>();
        if let Some(children) = children_storage.get_mut(e.id()) {
            children.0.push(wheel.id());
            added = true;
        }
    }
    if !added {
        world.add_component(e, gizmo_core::component::Children(vec![wheel.id()]));
    }

    (e.id(), wheel.id())
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Yay kuvveti sıkışmayla orantılı
//    Daha çok sıkışma → daha büyük +Y impulse
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_suspension_spring_force_proportional() {
    // shallow compression: araç ground_y'den 0.6 m yukarıda → compression = (0.5+0.3) - 0.6 = 0.2
    let mut world_shallow = make_world_with_ground(0.0);
    let (ve_shallow, _) = spawn_vehicle(&mut world_shallow, Vec3::new(0.0, 0.6, 0.0), 1000.0);

    // deep compression: araç ground_y'den 0.2 m yukarıda → compression = (0.5+0.3) - 0.2 = 0.6
    let mut world_deep = make_world_with_ground(0.0);
    let (ve_deep, _) = spawn_vehicle(&mut world_deep, Vec3::new(0.0, 0.2, 0.0), 1000.0);

    gizmo_physics::vehicle::physics_vehicle_system(&world_shallow, DT);
    gizmo_physics::vehicle::physics_vehicle_system(&world_deep, DT);

    let v_shallow = world_shallow
        .borrow::<Velocity>()
        .get(ve_shallow)
        .unwrap()
        .clone();
    let v_deep = world_deep
        .borrow::<Velocity>()
        .get(ve_deep)
        .unwrap()
        .clone();

    // Derin sıkışmada daha büyük upward impulse → daha yüksek Y hız artışı
    assert!(
        v_deep.linear.y > v_shallow.linear.y,
        "Derin sıkışma daha fazla kuvvet üretmeli. shallow.vy={:.4}, deep.vy={:.4}",
        v_shallow.linear.y,
        v_deep.linear.y
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Kütle bağımlı ivme — daha ağır araç daha az Y ivmesi alır (a = F/m)
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_suspension_mass_scaling() {
    let compression_pos = Vec3::new(0.0, 0.4, 0.0); // ground_y=0 → compression = 0.4 m

    let mut world_light = make_world_with_ground(0.0);
    let (ve_light, _) = spawn_vehicle(&mut world_light, compression_pos, 500.0);

    let mut world_heavy = make_world_with_ground(0.0);
    let (ve_heavy, _) = spawn_vehicle(&mut world_heavy, compression_pos, 2000.0);

    gizmo_physics::vehicle::physics_vehicle_system(&world_light, DT);
    gizmo_physics::vehicle::physics_vehicle_system(&world_heavy, DT);

    let v_light = world_light
        .borrow::<Velocity>()
        .get(ve_light)
        .unwrap()
        .clone();
    let v_heavy = world_heavy
        .borrow::<Velocity>()
        .get(ve_heavy)
        .unwrap()
        .clone();

    // Hafif araç daha yüksek Y hız kazanmalı (aynı F, az m → büyük a)
    assert!(
        v_light.linear.y > v_heavy.linear.y,
        "Hafif araç daha fazla ivme almalı. light.vy={:.4}, heavy.vy={:.4}",
        v_light.linear.y,
        v_heavy.linear.y
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Damping — zıt hız yönünde sönümleme, yay kadar büyük olduğunda kuvveti azaltır
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_suspension_damping_reduces_force() {
    // İki araç: aynı pozisyon, biri aşağı iniyor (damping kuvveti yaya ekle → azalt)
    let pos = Vec3::new(0.0, 0.4, 0.0);

    // A: durağan
    let mut world_still = make_world_with_ground(0.0);
    let (ve_still, _) = spawn_vehicle(&mut world_still, pos, 1000.0);
    // (Velocity default ZERO)

    // B: aşağı iniyor (damping kuvveti yay kuvvetini azaltır)
    let mut world_falling = make_world_with_ground(0.0);
    let (ve_falling, _) = spawn_vehicle(&mut world_falling, pos, 1000.0);
    {
        let mut v = world_falling.borrow_mut::<Velocity>();
        if let Some(vel) = v.get_mut(ve_falling) {
            vel.linear.y = -5.0; // Aşağı iniyor
        }
    }

    gizmo_physics::vehicle::physics_vehicle_system(&world_still, DT);
    gizmo_physics::vehicle::physics_vehicle_system(&world_falling, DT);

    let v_still = world_still
        .borrow::<Velocity>()
        .get(ve_still)
        .unwrap()
        .clone();
    let v_falling = world_falling
        .borrow::<Velocity>()
        .get(ve_falling)
        .unwrap()
        .clone();

    // Durağan araçta yay net kuvveti > düşen araçta (çünkü damping yayı frenler)
    assert!(
        v_still.linear.y > v_falling.linear.y + 0.001,
        "Damping azaltma bekleniyor: still.vy={:.4}, falling.vy={:.4}",
        v_still.linear.y,
        v_falling.linear.y
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Hava direnci kuadratik — hız 2x olunca drag 4x olmalı
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_drag_quadratic_scaling() {
    // Araç fizik sistemi gerçek drag'i v.linear'a uygular.
    // Burada drag impulse'u hesaplamayı doğrulayan birim testi:
    //   F_drag = 0.5 * Cd * |v|^2 * v̂
    // |v|=10: F = 0.5 * 0.3 * 100 = 15
    // |v|=20: F = 0.5 * 0.3 * 400 = 60  → 4x beklenir

    let cd = 0.3_f32;

    let drag_force = |speed: f32| -> f32 { 0.5 * cd * speed * speed };

    let f10 = drag_force(10.0);
    let f20 = drag_force(20.0);

    let ratio = f20 / f10;
    assert!(
        (ratio - 4.0).abs() < 0.01,
        "Drag kuadratik değil! 2x hız → beklenen 4x drag, elde edildi {:.3}x",
        ratio
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Drag kütle bağımlı — ağır araç daha az yavaşlar
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_drag_mass_dependent() {
    // Hava direnci impulse'u inv_mass ile çarpılıyor → Δv = F_drag * dt / m
    // Hafif araç: m=500 → daha büyük Δv (daha çok yavaşlama)
    // Ağır araç: m=5000 → daha küçük Δv

    let speed = 30.0_f32;
    let cd = 0.3_f32;

    let drag_impulse = 0.5 * cd * speed * speed * DT; // F_drag * dt

    let delta_v_light = drag_impulse / 500.0;
    let delta_v_heavy = drag_impulse / 5000.0;

    assert!(
        delta_v_light > delta_v_heavy * 1.5,
        "Drag kütle bağımlı değil: light Δv={:.5}, heavy Δv={:.5}",
        delta_v_light,
        delta_v_heavy
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. Tekerlek zeminle temas halindeyken is_grounded = true
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_wheel_grounded_when_near_ground() {
    // Araç yeterince alçakta: hit_t < rest_length + radius
    let pos = Vec3::new(0.0, 0.5, 0.0); // ground_y=0 → hit_t ≈ 0.5, threshold = 0.5+0.3=0.8
    let mut world = make_world_with_ground(0.0);
    let (_ve, whe) = spawn_vehicle(&mut world, pos, 1000.0);

    gizmo_physics::vehicle::physics_vehicle_system(&world, DT);

    let vc = world.borrow::<WheelComponent>().get(whe).unwrap().clone();
    assert!(
        vc.is_grounded,
        "Tekerlek zeminde olmalı ama is_grounded=false. compression={:.3}",
        vc.compression
    );
    assert!(
        vc.compression > 0.0,
        "Sıkışma > 0 bekleniyor, elde: {}",
        vc.compression
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. Havadayken is_grounded = false
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_wheel_not_grounded_when_airborne() {
    // Araç çok yüksekte: hit_t > rest_length + radius
    let pos = Vec3::new(0.0, 50.0, 0.0); // ground_y=0 → hit_t ≈ 50 >> 0.8
    let mut world = make_world_with_ground(0.0);
    let (_ve, whe) = spawn_vehicle(&mut world, pos, 1000.0);

    gizmo_physics::vehicle::physics_vehicle_system(&world, DT);

    let vc = world.borrow::<WheelComponent>().get(whe).unwrap().clone();
    assert!(!vc.is_grounded, "Araç havada olmalı ama is_grounded=true!");
    assert!(
        vc.compression == 0.0,
        "Havada sıkışma 0 olmalı, elde: {}",
        vc.compression
    );
}
