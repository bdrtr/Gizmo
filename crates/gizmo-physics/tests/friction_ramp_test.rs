/// Sürtünme ve Eğim Testi (Friction & Ramp Integration Test)
///
/// Senaryo: Havadan bir küp rampa üzerine düşer, rampada kayar, en sonunda
/// sürtünme sayesinde yavaşlayıp durur.
///
/// 3 farklı açıda test edilir: 10°, 30°, 45°
///
/// Beklentiler:
///   - Küp asla yerin altına geçmemeli (tünelleme yok)
///   - Küp havaya aşırı fırlamamalı (patlama/enerji yaratma yok)
///   - Belirli bir süre sonra küp duracak veya çok yavaşlayacak
///   - Angular velocity (açısal hız) patlamamalı
use gizmo_math::{Quat, Vec3};
use gizmo_physics::components::{RigidBody, Transform, Velocity};
use gizmo_physics::shape::Collider;

/// Tam fizik simülasyonu çalıştır: movement + collision
/// dt = sabit zaman adımı, steps = kaç kare simüle edilecek
fn run_simulation(world: &mut gizmo_core::World, dt: f32, steps: usize) {
    for _ in 0..steps {
        gizmo_physics::physics_apply_forces_system(world, dt);
        gizmo_physics::physics_movement_system(world, dt);
        gizmo_physics::system::physics_collision_system(world, dt);
    }
}

/// Belirli bir entity'nin Transform pozisyonunu oku
fn get_position(world: &gizmo_core::World, entity_id: u32) -> Vec3 {
    let ts = world.borrow::<gizmo_physics::Transform>();
    ts.get(entity_id).unwrap().position
}

/// Belirli bir entity'nin Velocity'sini oku
fn get_velocity(world: &gizmo_core::World, entity_id: u32) -> gizmo_physics::Velocity {
    let vs = world.borrow::<gizmo_physics::Velocity>();
    *vs.get(entity_id).unwrap()
}

/// Rampa + küp + zemin sahnesi oluştur. Küpü rampa üstünden bırak.
/// Dönen: (world, cube_entity_id)
fn setup_ramp_scene(angle_deg: f32, cube_start_y: f32, friction: f32) -> (gizmo_core::World, u32) {
    let mut world = gizmo_core::World::new();
    let rot_z = angle_deg.to_radians();

    // ==========================================
    // ZEMİN (Büyük, statik, düz AABB platform)
    // ==========================================
    let ground = world.spawn();
    let mut ground_t = Transform::new(Vec3::new(0.0, -10.0, 0.0));
    ground_t.scale = Vec3::new(50.0, 1.0, 50.0);
    world.add_component(ground, ground_t);
    world.add_component(ground, Collider::new_aabb(50.0, 1.0, 50.0));
    world.add_component(ground, RigidBody::new(0.0, 0.0, 1.0, false)); // Statik, yüksek sürtünme

    // ==========================================
    // RAMPA (Statik, ConvexHull — demo ile aynı)
    // ==========================================
    let ramp = world.spawn();
    let mut ramp_t = Transform::new(Vec3::new(0.0, 2.0, 0.0));
    ramp_t.scale = Vec3::new(10.0, 0.5, 5.0);
    ramp_t.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), rot_z);
    world.add_component(ramp, ramp_t);

    let ramp_vertices = vec![
        Vec3::new(-10.0, -0.5, -5.0),
        Vec3::new(10.0, -0.5, -5.0),
        Vec3::new(10.0, 0.5, -5.0),
        Vec3::new(-10.0, 0.5, -5.0),
        Vec3::new(-10.0, -0.5, 5.0),
        Vec3::new(10.0, -0.5, 5.0),
        Vec3::new(10.0, 0.5, 5.0),
        Vec3::new(-10.0, 0.5, 5.0),
    ];
    world.add_component(ramp, Collider::new_convex(ramp_vertices));
    world.add_component(ramp, RigidBody::new(0.0, 0.0, friction, false));

    // ==========================================
    // KÜP (Dinamik, ConvexHull — demo ile aynı)
    // ==========================================
    let cube = world.spawn();
    let mut cube_t = Transform::new(Vec3::new(0.0, cube_start_y, 0.0));
    // Rampa ile paralel açıda bırak (demo senaryosu ile aynı)
    cube_t.rotation = Quat::from_axis_angle(Vec3::new(0.0, 0.0, 1.0), rot_z);
    world.add_component(cube, cube_t);

    let cube_vertices = vec![
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
    ];
    world.add_component(cube, Collider::new_convex(cube_vertices));
    world.add_component(cube, RigidBody::new(10.0, 0.0, friction, true));
    world.add_component(cube, Velocity::new(Vec3::ZERO));

    (world, cube.id())
}

// ================================================================
// TEST 1: Küp rampa üzerine düşüyor ve yerin altına geçmiyor
// ================================================================
#[test]
fn test_cube_does_not_tunnel_through_ramp() {
    // Minimum Y sınırı: zemin -10.0'dır, küp yarı yüksekliği 1.0 → -9.0'un altına asla düşmemeli
    let min_allowed_y = -11.0; // Rampa eğimine ve iç geçmeye biraz tolerans

    for &angle in &[10.0_f32, 30.0, 45.0] {
        let (mut world, cube_id) = setup_ramp_scene(angle, 8.0, 0.5);
        let dt = 1.0 / 60.0;

        for step in 0..600 {
            // 10 saniye simülasyon
            gizmo_physics::physics_apply_forces_system(&world, dt);
            gizmo_physics::physics_movement_system(&world, dt);
            gizmo_physics::system::physics_collision_system(&mut world, dt);

            let pos = get_position(&world, cube_id);
            assert!(
                pos.y > min_allowed_y,
                "[{}° Rampa] Adım {}: Küp yerin altına geçti! Y={:.3} (limit: {:.1})",
                angle,
                step,
                pos.y,
                min_allowed_y
            );
        }

        let final_pos = get_position(&world, cube_id);
        println!("[{}° Tünel Testi] Son Pozisyon: {:?}", angle, final_pos);
    }
}

// ================================================================
// TEST 2: Küp havaya aşırı fırlamamalı (enerji yaratma YOK)
// Yüksekten düşen küp çarpışma sonrası biraz sekebilir ama
// başlangıç yüksekliğinin 2 katını asla aşmamalı
// ================================================================
#[test]
fn test_cube_does_not_fly_away() {
    let start_y = 8.0;
    // Restitution=0 bile olsa eğimli yüzeyden seken küp biraz yükselebilir.
    // Ama ASLA başlangıç yüksekliğinin 2 katına (16m) çıkmamalı — bu enerji üretimi demektir!
    let max_allowed_y = start_y * 2.0;

    for &angle in &[10.0_f32, 30.0, 45.0] {
        let (mut world, cube_id) = setup_ramp_scene(angle, start_y, 0.5);
        let dt = 1.0 / 60.0;

        for step in 0..600 {
            gizmo_physics::physics_apply_forces_system(&world, dt);
            gizmo_physics::physics_movement_system(&world, dt);
            gizmo_physics::system::physics_collision_system(&mut world, dt);

            let pos = get_position(&world, cube_id);
            assert!(
                pos.y < max_allowed_y,
                "[{}° Rampa] Adım {}: Küp havaya aşırı fırladı! Y={:.3} (limit: {:.1})",
                angle,
                step,
                pos.y,
                max_allowed_y
            );
        }

        let final_pos = get_position(&world, cube_id);
        println!("[{}° Uçuş Testi] Son Pozisyon: {:?}", angle, final_pos);
    }
}

// ================================================================
// TEST 3: Küp sonunda yavaşlayıp duruyor (sleeping veya düşük hız)
// 20 saniye fizik simülasyonu sonrası hız çok düşük olmalı
// ================================================================
#[test]
fn test_cube_eventually_settles() {
    let dt = 1.0 / 60.0;

    // 10° rampa: tan(10°)=0.176 < friction(0.5) → küp DURMALI!
    {
        let (mut world, cube_id) = setup_ramp_scene(10.0, 8.0, 0.5);
        run_simulation(&mut world, dt, 1800);
        let vel = get_velocity(&world, cube_id);
        let speed = vel.linear.length();
        println!("[10° Durma Testi] Lineer Hız: {:.4} m/s", speed);
        assert!(speed < 1.0, "[10°] Küp durmalıydı! Hız={:.3}", speed);
    }

    // 30° rampa: tan(30°)=0.577 > friction(0.5) → fiziksel olarak kayar ama hızı SINIRSIZ artmamalı
    {
        let (mut world, cube_id) = setup_ramp_scene(30.0, 8.0, 0.5);
        run_simulation(&mut world, dt, 1800);
        let vel = get_velocity(&world, cube_id);
        let speed = vel.linear.length();
        println!("[30° Sınırlı Kayma Testi] Lineer Hız: {:.4} m/s", speed);
        // Terminal hız: yerçekimi + hava direnci dengesinde sabit hız
        assert!(speed < 15.0, "[30°] Hız sınırsız artıyor! Hız={:.3}", speed);
    }

    // 45° rampa: çok dik ama yine de terminal hızda kalmalı
    {
        let (mut world, cube_id) = setup_ramp_scene(45.0, 8.0, 0.5);
        run_simulation(&mut world, dt, 1800);
        let vel = get_velocity(&world, cube_id);
        let speed = vel.linear.length();
        println!("[45° Sınırlı Kayma Testi] Lineer Hız: {:.4} m/s", speed);
        assert!(speed < 15.0, "[45°] Hız sınırsız artıyor! Hız={:.3}", speed);
    }
}

// ================================================================
// TEST 4: Angular velocity kullanılamayacak kadar büyük olmamali
// ================================================================
#[test]
fn test_angular_velocity_stays_bounded() {
    let max_angular_speed = 100.0; // rad/s — gerçekçi güvenlik sınırı

    for &angle in &[10.0_f32, 30.0, 45.0] {
        let (mut world, cube_id) = setup_ramp_scene(angle, 8.0, 0.5);
        let dt = 1.0 / 60.0;

        for step in 0..600 {
            gizmo_physics::physics_apply_forces_system(&world, dt);
            gizmo_physics::physics_movement_system(&world, dt);
            gizmo_physics::system::physics_collision_system(&mut world, dt);

            let vel = get_velocity(&world, cube_id);
            let angular_speed = vel.angular.length();

            assert!(
                angular_speed < max_angular_speed,
                "[{}° Rampa] Adım {}: Açısal hız patladı! ω={:.3} rad/s",
                angle,
                step,
                angular_speed
            );
        }
    }
}

// ================================================================
// TEST 5: Rampadan düşen küp zeminle de karşılaşmalı (tam senaryo)
// ================================================================
#[test]
fn test_full_drop_slide_stop_scenario() {
    // Senaryo: küp havadan 10° rampaya düşer → kayar → sürtünmeyle DURUR
    // 10° seçtik çünkü tan(10°)=0.176 < friction(0.5) → küp kesinlikle durmalı
    let (mut world, cube_id) = setup_ramp_scene(10.0, 8.0, 0.5);
    let dt = 1.0 / 60.0;

    // 1. İlk 1 saniyede küp düşmeli (Y azalmalı)
    run_simulation(&mut world, dt, 60);
    let pos_after_1s = get_position(&world, cube_id);
    assert!(
        pos_after_1s.y < 8.0,
        "Küp düşmedi! 1 sn sonra hâlâ Y={:.3}",
        pos_after_1s.y
    );
    println!("[Tam Senaryo] 1 sn sonra: {:?}", pos_after_1s);

    // 2. 30 saniye sonra küp durmuş olmalı
    run_simulation(&mut world, dt, 1740);
    let final_pos = get_position(&world, cube_id);
    let final_vel = get_velocity(&world, cube_id);
    let final_speed = final_vel.linear.length();

    println!(
        "[Tam Senaryo] 30 sn sonra: Pos={:?}, Hız={:.4} m/s",
        final_pos, final_speed
    );

    // Tünelleme kontrolü
    assert!(
        final_pos.y > -12.0,
        "Küp yerin altına geçti! Y={:.3}",
        final_pos.y
    );

    // Durma kontrolü — 10° rampada 30 sn sonra kesinlikle durmuş olmalı
    assert!(
        final_speed < 1.0,
        "Küp 30 sn sonra hâlâ hızlı! Hız={:.3} m/s",
        final_speed
    );
}
