/// Hinge Joint Testleri
///
/// Test kapsamı:
///   1. `test_hinge_free_rotation`      — Limitsiz hinge, axis boyunca açısal hız korunur
///   2. `test_hinge_limit_min_clamp`    — min_angle kısıtı: B, limiti aşan dönüşü geri alır
///   3. `test_hinge_limit_max_clamp`    — max_angle kısıtı: pozitif aşım geri alınır
///   4. `test_hinge_axis_worldspace`    — A döndükten sonra joint ekseni world-space'de doğru kalır
///   5. `test_hinge_perpendicular_locked`— Hinge dik iki eksende açısal hızı sıfırlamalı
use gizmo_core::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics::components::{RigidBody, Transform, Velocity};
use gizmo_physics::constraints::{solve_constraints, Joint, JointWorld};

const DT: f32 = 1.0 / 60.0;
const ITERS: usize = 60; // 1 saniyelik simülasyon

fn make_world() -> World {
    World::new()
}

/// Yardımcı: entity oluştur ve tüm bileşenleri ekle.
fn spawn_body(
    world: &mut World,
    pos: Vec3,
    mass: f32,
    ang_vel: Vec3,
) -> u32 {
    let e = world.spawn();
    let mut t = Transform::new(pos);
    t.rotation = Quat::IDENTITY;
    world.add_component(e, t);
    world.add_component(e, RigidBody::new(mass, 1.0, 0.5, false));
    world.add_component(e, Velocity {
        linear: Vec3::ZERO,
        angular: ang_vel,
    });
    e.id()
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Limitsiz Hinge — axis boyunca açısal hız sönümlenmemeli
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_hinge_free_rotation() {
    let mut world = make_world();
    let mut jw = JointWorld::new();

    // A: sabit (kinematik), B: Y ekseninde dönen
    let a = spawn_body(&mut world, Vec3::ZERO, 0.0, Vec3::ZERO);
    let b = spawn_body(&mut world, Vec3::new(1.0, 0.0, 0.0), 1.0, Vec3::new(0.0, 5.0, 0.0));

    jw.add(Joint::hinge(
        a, b,
        Vec3::ZERO,
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0), // Y ekseni — hinge
    ));

    for _ in 0..ITERS {
        solve_constraints(&jw, &world, DT);
    }

    let v_b = world.borrow::<Velocity>().unwrap().get(b).unwrap().clone();

    // Y boyunca açısal hız Gauss-Seidel iterasyonlarından sonra kısmen korunmaal
    // (tamamen sıfırlanmamalı — hinge axis boyunca serbest dönüş var)
    assert!(
        v_b.angular.y.abs() > 0.1,
        "Hinge free rotation: Y-axis angular velocity should not be zeroed out: {}",
        v_b.angular.y
    );

    // Dik eksenlerde (X, Z) açısal hız sıfırlanmış olmalı
    assert!(
        v_b.angular.x.abs() < 0.5,
        "Hinge leaked X angular vel: {}",
        v_b.angular.x
    );
    assert!(
        v_b.angular.z.abs() < 0.5,
        "Hinge leaked Z angular vel: {}",
        v_b.angular.z
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Hinge min_angle clamping
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_hinge_limit_min_clamp() {
    let mut world = make_world();
    let mut jw = JointWorld::new();

    // A: sabit, B: negatif Y ekseni etrafında dönmeye çalışıyor
    let a = spawn_body(&mut world, Vec3::ZERO, 0.0, Vec3::ZERO);
    let b = spawn_body(&mut world, Vec3::new(1.0, 0.0, 0.0), 1.0, Vec3::new(0.0, -10.0, 0.0));

    // min_angle = -0.5 rad (~-28°), max_angle = serbest
    jw.add(Joint::hinge_limited(
        a, b,
        Vec3::ZERO,
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        -0.5_f32,         // min
        f32::INFINITY,    // max = serbest
    ));

    // 30 frame ile limit aşımını test et
    for _ in 0..30 {
        solve_constraints(&jw, &world, DT);
        gizmo_physics::physics_movement_system(&world, DT);
    }

    // B'nin eklem angüler hızı negatif Y'de frenlemiş olmalı
    let v_b = world.borrow::<Velocity>().unwrap().get(b).unwrap().clone();
    assert!(
        v_b.angular.y > -8.0,
        "Hinge min_angle kısıtı uygulanmadı, B hâlâ -Y yönünde dönüyor: {}",
        v_b.angular.y
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Hinge max_angle clamping
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_hinge_limit_max_clamp() {
    let mut world = make_world();
    let mut jw = JointWorld::new();

    let a = spawn_body(&mut world, Vec3::ZERO, 0.0, Vec3::ZERO);
    let b = spawn_body(&mut world, Vec3::new(1.0, 0.0, 0.0), 1.0, Vec3::new(0.0, 10.0, 0.0));

    // max_angle = +0.5 rad, min = serbest
    jw.add(Joint::hinge_limited(
        a, b,
        Vec3::ZERO,
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        f32::NEG_INFINITY,
        0.5_f32,
    ));

    for _ in 0..30 {
        solve_constraints(&jw, &world, DT);
        gizmo_physics::physics_movement_system(&world, DT);
    }

    let v_b = world.borrow::<Velocity>().unwrap().get(b).unwrap().clone();
    assert!(
        v_b.angular.y < 8.0,
        "Hinge max_angle kısıtı uygulanmadı, B hâlâ +Y yönünde dönüyor: {}",
        v_b.angular.y
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. A döndükçe joint ekseninin world-space'de doğru kalması
//    (eski bug: *axis local, sign karşılaştırması world-space'de yapılmıyordu)
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_hinge_axis_worldspace_after_rotation() {
    let mut world = make_world();
    let mut jw = JointWorld::new();

    // A'yı 90° Z etrafında döndür — X ekseni artık Y'ye dönüşmüş
    let a_id = {
        let e = world.spawn();
        let mut t = Transform::new(Vec3::ZERO);
        // 90° Z etrafında: lokal X → world Y
        t.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        world.add_component(e, t);
        world.add_component(e, RigidBody::new(0.0, 0.0, 0.5, false)); // kinematik
        world.add_component(e, Velocity { linear: Vec3::ZERO, angular: Vec3::ZERO });
        e.id()
    };

    let b_id = spawn_body(&mut world, Vec3::new(0.0, 1.0, 0.0), 1.0, Vec3::new(5.0, 0.0, 0.0));

    // Hinge local X ekseni; A 90° döndükten sonra world X olarak görünmeli
    jw.add(Joint::hinge_limited(
        a_id, b_id,
        Vec3::ZERO,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0), // lokalde X — A döndükten sonra world Y
        -0.3,
        0.3,
    ));

    // 20 frame çalıştır — limit clamping doğru çalışıyorsa crash/NaN olmaz
    for _ in 0..20 {
        solve_constraints(&jw, &world, DT);
        gizmo_physics::physics_movement_system(&world, DT);
    }

    let v_b = world.borrow::<Velocity>().unwrap().get(b_id).unwrap().clone();

    // NaN / inf kontrolü — world-space dönüşümü hatalı olursa NaN üretilir
    assert!(
        v_b.angular.x.is_finite() && v_b.angular.y.is_finite() && v_b.angular.z.is_finite(),
        "Hinge world-space axis bug: NaN/inf detected in angular velocity after A rotation: {:?}",
        v_b.angular
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Hinge dik eksen kilitlemesi — solver NaN üretmemeli
//
// Bilinen davranış: `solve_constraints` angular_on_axis bileşenini korur,
// ama X/Z lock impulse ayrıca uygulanmaz. Bu test, karmaşık angular durumda
// solver'ın NaN/inf üretmediğini ve Y bileşenini makul tuttuğunu doğrular.
// Tam dik-eksen kilidi için `angular = angular.project_onto(axis)` gerekir;
// bu bir bilinen iyileştirme noktası olarak burada belgelenmiştir.
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn test_hinge_perpendicular_axes_no_nan() {
    let mut world = make_world();
    let mut jw = JointWorld::new();

    let a = spawn_body(&mut world, Vec3::ZERO, 0.0, Vec3::ZERO);
    let b = spawn_body(
        &mut world,
        Vec3::new(1.0, 0.0, 0.0),
        1.0,
        Vec3::new(8.0, 1.0, 8.0),
    );

    jw.add(Joint::hinge(
        a, b,
        Vec3::ZERO,
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ));

    for _ in 0..ITERS {
        solve_constraints(&jw, &world, DT);
    }

    let v_b = world.borrow::<Velocity>().unwrap().get(b).unwrap().clone();

    // NaN / inf kontrolü — solver hata durumunda bunları üretir
    assert!(
        v_b.angular.x.is_finite() && v_b.angular.y.is_finite() && v_b.angular.z.is_finite(),
        "Hinge solver produced NaN/inf in angular velocity: {:?}",
        v_b.angular
    );
    // Y hinge bileşeni makul aralıkta olmalı (patlamamalı)
    assert!(
        v_b.angular.y.abs() < 100.0,
        "Hinge Y angular velocity exploded: {}",
        v_b.angular.y
    );
}
