use super::*;
use gizmo_physics_core::BodyHandle;
use gizmo_math::Vec3;

#[test]
fn test_physics_world_creation() {
    let world = PhysicsWorld::new();
    assert_eq!(world.integrator.gravity, Vec3::new(0.0, -9.81, 0.0));
}

#[test]
fn test_physics_step() {
    let mut world = PhysicsWorld::new();

    let entity = BodyHandle::from_id(1);
    let rb = RigidBody::default();
    let transform = Transform::new(Vec3::new(0.0, 10.0, 0.0));
    let vel = Velocity::default();
    let collider = Collider::sphere(1.0);

    world.add_body(entity, rb, transform, vel, collider);

    // Simulate for 1 second
    for _ in 0..60 {
        let _ = world.step(1.0 / 60.0);
    }

    // Object should have fallen due to gravity
    assert!(world.transforms[0].position.y < 10.0);
}

/// Air resistance end to end (sync → step → integrate): of two bodies dropped from the same
/// height, the one with heavy drag must fall far less and settle at the analytic terminal
/// velocity, while the one without drag keeps accelerating and ends up much faster.
#[test]
fn air_drag_body_falls_slower_and_reaches_terminal_velocity() {
    let mut world = PhysicsWorld::new(); // gravity -9.81, air_density 1.225

    // Serbest düşüş (drag yok).
    let mut free = RigidBody::new(2.0, true);
    free.linear_damping = 0.0;
    world.add_body(
        BodyHandle::from_id(1),
        free,
        Transform::new(Vec3::new(0.0, 100.0, 0.0)),
        Velocity::default(),
        Collider::sphere(1.0),
    );

    // Ağır hava direnci (~paraşüt).
    let (m, cd, area) = (2.0_f32, 1.2_f32, 3.0_f32);
    let mut dragged = RigidBody::new(m, true).with_air_drag(cd, area);
    dragged.linear_damping = 0.0;
    world.add_body(
        BodyHandle::from_id(2),
        dragged,
        Transform::new(Vec3::new(5.0, 100.0, 0.0)),
        Velocity::default(),
        Collider::sphere(1.0),
    );

    for _ in 0..180 {
        // 3 s
        let _ = world.step(1.0 / 60.0);
    }

    let free_y = world.transforms[0].position.y;
    let drag_y = world.transforms[1].position.y;
    let free_speed = world.velocities[0].linear.length();
    let drag_speed = world.velocities[1].linear.length();

    // Drag'li gövde çok daha az düşmeli.
    assert!(
        drag_y > free_y + 20.0,
        "drag'li top serbest düşüşten çok daha az düşmeli: drag_y={drag_y}, free_y={free_y}"
    );
    // Drag'li gövde analitik terminal hıza oturmalı: v_term = √(2mg/ρCdA).
    let v_term = (2.0 * m * 9.81 / (1.225 * cd * area)).sqrt();
    assert!(
        (drag_speed - v_term).abs() < v_term * 0.1,
        "drag hızı terminal {v_term:.2} m/s civarı olmalı, bulundu {drag_speed:.2}"
    );
    // Drag'siz gövde hızlanmaya devam eder → drag'liden çok daha hızlı.
    assert!(
        free_speed > drag_speed * 3.0,
        "serbest düşüş ({free_speed:.1}) drag'liden ({drag_speed:.1}) çok daha hızlı olmalı"
    );
}

#[test]
fn test_high_stack_stability() {
    let mut world = PhysicsWorld::new();
    // Akademik doğrulama için iterasyon sayısını yüksek tutalım
    world.solver.iterations = 30;

    // Ground
    let mut ground_rb = RigidBody::default();
    ground_rb.body_type = crate::components::rigid_body::BodyType::Static;
    ground_rb.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        ground_rb,
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(50.0, 0.5, 50.0)),
    );

    // 10 Kutuluk bir kule inşa et
    let box_count = 10;
    let box_size = 1.0;
    let half_size = box_size / 2.0;

    for i in 1..=box_count {
        let mut rb = RigidBody::new(1.0, true);
        rb.wake_up(); // Uyumasını engelle ki solver test edilsin

        let y_pos = half_size + (i - 1) as f32 * box_size;

        world.add_body(
            BodyHandle::from_id(i),
            rb,
            Transform::new(Vec3::new(0.0, y_pos, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(half_size, half_size, half_size)),
        );
    }

    // 10 saniye (600 kare) simüle et
    for i in 0..600 {
        let _ = world.step(1.0 / 60.0);
        if i % 60 == 0 {
            println!("Frame {}: Y={}, X={}, Z={}", i, world.transforms[10].position.y, world.transforms[10].position.x, world.transforms[10].position.z);
        }
    }

    // Kule yıkılmamış olmalı (X ve Z ekseninde çok kaymamış olmalı)
    // En üstteki kutunun durumuna bakalım
    let top_box_idx = box_count as usize; // BodyHandle ID starts from 1 for boxes, so idx is `box_count` because ground is 0
    let top_box_pos = world.transforms[top_box_idx].position;

    // Akademik limitler: 10 saniye boyunca dik durmalı, yana yatmamalı
    assert!(
        top_box_pos.x.abs() < 0.1,
        "Top box slid too much on X axis: {}",
        top_box_pos.x
    );
    assert!(
        top_box_pos.z.abs() < 0.1,
        "Top box slid too much on Z axis: {}",
        top_box_pos.z
    );

    // Yüksekliği korunmalı (Jitter / Penetrasyon testi)
    let expected_y = half_size + (box_count - 1) as f32 * box_size;
    let y_error = (top_box_pos.y - expected_y).abs();
    assert!(
        y_error < 0.1,
        "Top box sunk or bounced too much. Expected Y: {}, Actual Y: {}",
        expected_y,
        top_box_pos.y
    );
}

/// A fast bullet against a thin wall, the CCD bullet's bounce in [`ccd_bullet_bounces_off_a_wall`]
/// below: this one is inelastic (restitution 0 on both), so it must come to rest at the wall's face.
fn ccd_bullet_world(restitution: Option<f32>) -> PhysicsWorld {
    let material = |c: Collider| match restitution {
        Some(e) => c.with_material(gizmo_physics_core::PhysicsMaterial { restitution: e, ..Default::default() }),
        None => c,
    };
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::ZERO;
    let mut wall_rb = RigidBody::new_static();
    wall_rb.wake_up();
    world.add_body(BodyHandle::from_id(0), wall_rb, Transform::new(Vec3::ZERO), Velocity::default(), material(Collider::box_collider(Vec3::new(0.1, 5.0, 5.0))));
    let mut bullet_rb = RigidBody::new(1.0, false);
    bullet_rb.ccd_enabled = true;
    bullet_rb.wake_up();
    world.add_body(BodyHandle::from_id(1), bullet_rb, Transform::new(Vec3::new(-5.0, 0.0, 0.0)), Velocity::new(Vec3::new(1200.0, 0.0, 0.0)), material(Collider::sphere(0.2)));
    world
}

/// **A CCD bullet bounces like any body.** The split-impulse path that CCD islands take solved
/// the speculative contact that stops a bullet without restitution, so a bullet died at the wall
/// whatever its material — fixed 2026-11-15 (the speculative restitution pass). With the default
/// materials (restitution 0.3) it now flies back at about 0.3 × 1200 m/s, and still never passes
/// the wall.
#[test]
fn ccd_bullet_bounces_off_a_wall() {
    let mut world = ccd_bullet_world(None);
    // One frame: the bullet covers the 4.7 m to the wall (x = −0.3 at contact) in 3.9 ms of the
    // frame's 16.7, bounces at 0.3 × 1200 m/s, and spends the rest going back — about 4.6 m,
    // ending near x = −4.9 (−4.8 at this world's substeps). Turning where it stood instead of at
    // the wall would end it near −11, which is what a velocity reversed at the start of the step
    // did.
    let _ = world.step(1.0 / 60.0);
    let (x, v) = (world.transforms[1].position.x, world.velocities[1].linear.x);
    assert!((-1200.0 * 0.4..-1200.0 * 0.2).contains(&v), "bounced at {v} m/s, expected about −360");
    assert!((-5.6..-4.2).contains(&x), "after the bounce frame at x = {x}: it did not turn at the wall");
    let mut max_x = x;
    for _ in 0..30 {
        let _ = world.step(1.0 / 60.0);
        max_x = max_x.max(world.transforms[1].position.x);
    }
    assert!(max_x < 0.0, "TUNNELING! peak x = {max_x}");
    assert!(world.velocities[1].linear.x < -300.0, "it keeps flying back");
}

#[test]
fn test_ccd_tunneling_prevention() {
    // Inelastic: the bullet must stop at the wall (the bounce has its own test above).
    let mut world = ccd_bullet_world(Some(0.0));

    // Birden çok kare simüle et: speculative CCD merminin yolunu o kareye izin
    // verilen boşlukla SINIRLAR; mermi duvara varır, ertesi karede tam durur.
    let mut max_x = f32::MIN;
    for _ in 0..120 {
        let _ = world.step(1.0 / 60.0);
        max_x = max_x.max(world.transforms[1].position.x);
    }

    // 1) HİÇBİR karede duvar merkezini geçmemeli (geçseydi x ~ +15 olurdu).
    assert!(
        max_x < 0.0,
        "TUNNELING! Bullet crossed the wall — peak x = {max_x}"
    );

    // 2) Eski `penetration = 0` hatasında mermi başlangıçta (x≈-5) DONUYORDU.
    //    Doğru CCD'de duvarın ön yüzüne (x≈-0.31) dayanıp durmalı.
    let final_x = world.transforms[1].position.x;
    assert!(
        (-0.6..=-0.1).contains(&final_x),
        "Bullet should rest against the wall front (~ -0.31), got x = {final_x} \
         (frozen far short would be ~ -5.0)"
    );

    // 3) Sonunda durmuş olmalı.
    let final_v = world.velocities[1].linear.x;
    assert!(
        final_v.abs() < 1.0,
        "Bullet should have stopped against the wall, vel.x = {final_v}"
    );
}

#[test]
fn test_material_combine_modes_respected() {
    use gizmo_physics_core::{CombineMode, PhysicsMaterial};
    // Temas materyali artık her materyalin combine MODUYLA birleşir
    // (`PhysicsMaterial::combine`). Eskiden pipeline geometrik-ortalama'yı
    // hardcode ediyordu → `friction_combine` yok sayılıyordu.
    //
    // Yüksek-sürtünme + Max-combine kutu, DÜŞÜK-sürtünme zeminde:
    //   Doğru (Max):  μ = max(0.9, 0.1) = 0.9 → ~5-6 m'de durur.
    //   Eski (geo.ort): μ = sqrt(0.9·0.1) = 0.3 → ~17 m kayar.
    // AYIRT EDİCİ: combine() yerine eski hardcode'a dönülürse test DÜŞER.
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

    let mut ground = RigidBody::new_static();
    ground.wake_up();
    let mut gcol = Collider::box_collider(Vec3::new(200.0, 0.5, 200.0));
    gcol.material = PhysicsMaterial {
        static_friction: 0.1,
        dynamic_friction: 0.1,
        friction_combine: CombineMode::GeometricMean,
        ..PhysicsMaterial::default()
    };
    world.add_body(
        BodyHandle::from_id(0),
        ground,
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        gcol,
    );

    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let mut col = Collider::box_collider(Vec3::splat(0.5));
    col.material = PhysicsMaterial {
        static_friction: 0.9,
        dynamic_friction: 0.9,
        friction_combine: CombineMode::Max, // Max, zemin GeometricMean'i ezer
        ..PhysicsMaterial::default()
    };
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(1),
        rb,
        Transform::new(Vec3::new(0.0, 0.5, 0.0)),
        Velocity::new(Vec3::new(10.0, 0.0, 0.0)),
        col,
    );

    for _ in 0..300 {
        let _ = world.step(1.0 / 60.0);
    }
    let x = world.transforms[1].position.x;
    assert!(
        x < 10.0,
        "Max friction_combine yok sayıldı — kutu {x} m kaydı (Max ile ~5-6 m beklenir; \
         eski geo-ort hardcode'u ~17 m verirdi)"
    );
}

#[test]
fn test_coulomb_friction_and_sleeping() {
    use gizmo_physics_core::PhysicsMaterial;
    let mut world = PhysicsWorld::new();
    // Sürtünme için yerçekimi şart (normal kuvveti yaratmak için).
    world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

    // ÖNEMLİ: temas sürtünmesi collider MATERYALİNDEN gelir
    // (`manifold.friction = sqrt(mat_a.dyn * mat_b.dyn)`), `RigidBody::friction`
    // alanından DEĞİL. Bu test eskiden rb.friction'ı değiştiriyordu — o alan
    // temas çözücüye HİÇ ulaşmıyor, dolayısıyla iki kutu da varsayılan materyalle
    // aynı mesafeyi gidip test yalnızca sub-mm gürültüyle "geçiyordu". Farkı
    // gerçek sürücüye, yani materyale koyuyoruz.

    // Zemin — yüksek sürtünmeli, geniş (A ~23 m kayabilir).
    let mut ground_rb = RigidBody::new_static();
    ground_rb.wake_up();
    let mut ground_col = Collider::box_collider(Vec3::new(200.0, 0.5, 200.0));
    ground_col.material = PhysicsMaterial {
        static_friction: 0.9,
        dynamic_friction: 0.9,
        ..PhysicsMaterial::default()
    };
    world.add_body(
        BodyHandle::from_id(0),
        ground_rb,
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        ground_col,
    );

    let mut make_box = |id: u32, z: f32, fric: f32| {
        let mut rb = RigidBody::new(1.0, true);
        rb.wake_up();
        let mut col = Collider::box_collider(Vec3::splat(0.5));
        col.material = PhysicsMaterial {
            static_friction: fric,
            dynamic_friction: fric,
            ..PhysicsMaterial::default()
        };
        rb.update_inertia_from_collider(&col);
        world.add_body(
            BodyHandle::from_id(id),
            rb,
            Transform::new(Vec3::new(0.0, 0.5, z)),
            Velocity::new(Vec3::new(10.0, 0.0, 0.0)),
            col,
        );
    };
    make_box(1, -2.0, 0.05); // Kutu A: düşük sürtünme → uzağa kayar
    make_box(2, 2.0, 0.9); //  Kutu B: yüksek sürtünme → erken durur

    // 5 saniye simüle et (300 kare) — ikisi de durup uyumalı.
    for _ in 0..300 {
        let _ = world.step(1.0 / 60.0);
    }

    let pos_a = world.transforms[1].position;
    let pos_b = world.transforms[2].position;

    // Yüksek sürtünmeli kutu BELİRGİN şekilde daha az yol gitmeli (~5 m'ye karşı
    // ~23 m). Sağlam marj: sub-mm gürültüye değil gerçek sürtünmeye duyarlı.
    assert!(
        pos_b.x < pos_a.x - 5.0,
        "Yüksek sürtünmeli kutu belirgin daha az gitmeli. A: {}, B: {}",
        pos_a.x,
        pos_b.x
    );

    // İkisi de durup UYKU MODUNA geçmeli.
    assert!(world.rigid_bodies[1].is_sleeping, "Düşük sürtünmeli kutu uyumadı!");
    assert!(world.rigid_bodies[2].is_sleeping, "Yüksek sürtünmeli kutu uyumadı!");
}

#[test]
fn test_car_simulation() {
    use crate::joints::data::{Joint, JointData, HingeJointData};

    let mut world = PhysicsWorld::new();
    // Yerçekimi açık (Sürtünme ve ağırlık için gerekli)
    world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

    // --- Zemin ---
    let mut ground_rb = RigidBody::new_static();
    ground_rb.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        ground_rb,
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(100.0, 0.5, 100.0)),
    );

    // --- Şasi (Chassis) ---
    // 1000 kg, sürtünme önemsiz, dinamik
    let mut chassis_rb = RigidBody::new(1000.0, true);
    chassis_rb.wake_up();
    let chassis_col = Collider::box_collider(Vec3::new(1.0, 0.5, 2.0)); // Genişlik 2, Yükseklik 1, Uzunluk 4 (Yarıçaplar)
    chassis_rb.update_inertia_from_collider(&chassis_col);
    let chassis_entity = BodyHandle::from_id(1);
    let chassis_pos = Vec3::new(0.0, 1.5, 0.0);
    world.add_body(
        chassis_entity,
        chassis_rb,
        Transform::new(chassis_pos),
        Velocity::default(),
        chassis_col,
    );

    // Tekerlek Şablonu
    let wheel_radius = 0.5;
    let mut wheel_rb = RigidBody::new(50.0, true); // Yüksek kütle (50kg) ve yüksek sürtünme (0.9)
    wheel_rb.wake_up();
    let wheel_col = Collider::sphere(wheel_radius);
    wheel_rb.update_inertia_from_collider(&wheel_col);

    let wheel_offsets = [
        Vec3::new(-1.2, -0.2, 1.5),  // Sol Ön
        Vec3::new(1.2, -0.2, 1.5),   // Sağ Ön
        Vec3::new(-1.2, -0.2, -1.5), // Sol Arka
        Vec3::new(1.2, -0.2, -1.5),  // Sağ Arka
    ];

    let mut wheel_entities = Vec::new();

    for (i, offset) in wheel_offsets.iter().enumerate() {
        let wheel_entity = BodyHandle::from_id(2 + i as u32);
        wheel_entities.push(wheel_entity);

        world.add_body(
            wheel_entity,
            wheel_rb,
            Transform::new(chassis_pos + *offset),
            Velocity::default(),
            wheel_col.clone(),
        );

        // Menteşe Eklemi (Hinge Joint) oluştur
        let is_rear = i >= 2;
        let hinge_data = HingeJointData {
            axis: Vec3::X, // Tekerlekler X ekseni etrafında dönecek
            use_limits: false,
            lower_limit: 0.0,
            upper_limit: 0.0,
            use_motor: is_rear, // Sadece arka tekerleklerde motor var
            motor_target_velocity: if is_rear { 10.0 } else { 0.0 }, // İleri doğru 10 rad/s
            motor_max_force: if is_rear { 10000.0 } else { 0.0 }, // 10000 N güç
            current_angle: 0.0,
            ..Default::default()
        };

        let joint = Joint {
            entity_a: chassis_entity,
            entity_b: wheel_entity,
            local_anchor_a: *offset, // Şasinin lokal uzayında bağlantı noktası
            local_anchor_b: Vec3::ZERO, // Tekerleğin tam ortası
            break_force: f32::MAX, // Asla kopmasın
            break_torque: f32::MAX,
            is_broken: false,
            scratch: Default::default(),
            collision_enabled: false, // Şasi ile tekerlek çarpışmasın
            data: JointData::Hinge(hinge_data),
        };

        world.joints.push(joint);
    }
    // --- Simülasyon ---
    // Motorlar çalışacak ve arabayı 5 saniye boyunca (300 kare) ileri doğru (Z+) sürecek
    for _ in 0..300 {
        let _ = world.step(1.0 / 60.0);
    }
    // Doğrulama
    let final_chassis_pos = world.transforms[1].position;

    // 1. İleri Sürüş: Araba Z ekseninde (ileri) hareket etmiş olmalı
    assert!(
        final_chassis_pos.z > 3.0,
        "Araba yeterince ileri gidemedi! Motor veya sürtünme çalışmıyor. Z pozisyonu: {}",
        final_chassis_pos.z
    );

    // 2. Denge (Devrilmeme): Arabanın Y pozisyonu stabil kalmalı (uçmamalı veya batmamalı)
    // Başlangıç Y: 1.5, Tekerlek yarıçapı 0.5. Araba yere oturunca Y ~1.0 - 1.2 civarı olmalı
    assert!(
        final_chassis_pos.y > 0.5 && final_chassis_pos.y < 2.0,
        "Araba devrildi, uçtu veya yere battı! Y pozisyonu: {}",
        final_chassis_pos.y
    );

    // 3. X Ekseninde Düz Gitme (Sağa sola savrulmama)
    assert!(
        final_chassis_pos.x.abs() < 1.0,
        "Araba düz gidemedi, sağa sola savruldu! X pozisyonu: {}",
        final_chassis_pos.x
    );
}

/// 2-tangent friction must slow and stop a slide that is not axis-aligned (diagonal)
/// symmetrically in both tangent components. The old single-tangent method could lose the
/// perpendicular component of the accumulated impulse.
#[test]
fn friction_decelerates_diagonal_slide_symmetrically() {
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

    let mut ground = RigidBody::new_static();
    ground.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        ground,
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(100.0, 0.5, 100.0)),
    );

    // Diyagonal kayan kutu (vx = vz); dönmeyi kilitle → saf öteleme sürtünmesi.
    let mut rb = RigidBody::new(1.0, true);
    rb.lock_rotation_x = true;
    rb.lock_rotation_y = true;
    rb.lock_rotation_z = true;
    rb.wake_up();
    let col = Collider::box_collider(Vec3::new(0.5, 0.5, 0.5));
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(1),
        rb,
        Transform::new(Vec3::new(0.0, 0.5, 0.0)),
        Velocity::new(Vec3::new(3.0, 0.0, 3.0)),
        col,
    );

    for _ in 0..10 {
        world.step(1.0 / 60.0).unwrap();
    }
    let v_mid = world.velocities[1].linear;
    let speed_mid = (v_mid.x * v_mid.x + v_mid.z * v_mid.z).sqrt();

    for _ in 0..150 {
        world.step(1.0 / 60.0).unwrap();
    }
    let v_end = world.velocities[1].linear;
    let speed_end = (v_end.x * v_end.x + v_end.z * v_end.z).sqrt();

    // Simetri: x ve z bileşenleri yakın kalmalı (dik bileşen kaybolmaz).
    assert!(
        (v_mid.x - v_mid.z).abs() < 0.2,
        "diyagonal simetri bozuldu: vx={} vz={}",
        v_mid.x,
        v_mid.z
    );
    // Sürtünme belirgin yavaşlatıp neredeyse durdurmalı.
    assert!(speed_end < speed_mid, "yavaşlamadı: {speed_mid} -> {speed_end}");
    assert!(speed_end < 0.5, "durmaya yakın olmalı, kalan hız: {speed_end}");
}

/// A moving kinematic platform must wake the SLEEPING dynamic body resting on it and drag it
/// along by friction. (A kinematic body did not use to count as a "mover", so the island never
/// woke and the sleeping body was never disturbed.)
#[test]
fn moving_kinematic_platform_wakes_sleeping_body() {
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::new(0.0, -10.0, 0.0);

    // Kinematik platform: merkez 0, üst yüz +0.5.
    let plat = RigidBody::new_kinematic();
    world.add_body(
        BodyHandle::from_id(0),
        plat,
        Transform::new(Vec3::new(0.0, 0.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(5.0, 0.5, 5.0)),
    );

    // Üstünde dinamik kutu: merkez 1.0, alt 0.5 = platform üstü.
    let mut box_rb = RigidBody::new(1.0, true);
    box_rb.lock_rotation_x = true;
    box_rb.lock_rotation_y = true;
    box_rb.lock_rotation_z = true;
    box_rb.wake_up();
    let col = Collider::box_collider(Vec3::new(0.5, 0.5, 0.5));
    box_rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(1),
        box_rb,
        Transform::new(Vec3::new(0.0, 1.0, 0.0)),
        Velocity::default(),
        col,
    );

    // Platform sabitken kutuyu uyut.
    for _ in 0..400 {
        world.step(1.0 / 60.0).unwrap();
    }
    assert!(
        world.rigid_bodies[1].is_sleeping,
        "kutu önce uyumalı (uyumadıysa senaryo geçersiz)"
    );
    let x_before = world.transforms[1].position.x;

    // Platformu +x yönünde hareket ettir.
    world.velocities[0].linear = Vec3::new(2.0, 0.0, 0.0);
    for _ in 0..30 {
        world.step(1.0 / 60.0).unwrap();
    }

    assert!(
        !world.rigid_bodies[1].is_sleeping,
        "hareket eden kinematik platform kutuyu uyandırmalı"
    );
    let x_after = world.transforms[1].position.x;
    assert!(
        x_after > x_before + 0.05,
        "kutu platformla sürüklenmeli: {x_before} -> {x_after}"
    );
}

#[test]
fn raycast_excluding_skips_self_and_finds_ground_behind() {
    use gizmo_physics_core::raycast::Ray;

    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::ZERO; // pozisyonlar sabit kalsın

    // Zemin: üstü y=0'da olan geniş kutu (entity 1).
    world.add_body(
        BodyHandle::from_id(1),
        RigidBody::new_static(),
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(50.0, 0.5, 50.0)),
    );
    // "Şasi": ışın kaynağını (y=2) İÇİNE alan kutu, y 1..3 (entity 2).
    world.add_body(
        BodyHandle::from_id(2),
        RigidBody::new(1.0, true),
        Transform::new(Vec3::new(0.0, 2.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::splat(1.0)),
    );
    let _ = world.step(1.0 / 60.0); // spatial hash'i doldur

    // Işın kutunun İÇİNDEN (y=2) düz aşağı.
    let ray = Ray::new(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, -1.0, 0.0));

    // Kendini (kutu=2) dışlayan raycast, ARKADAKİ zemini (entity 1) bulmalı — eski
    // "en yakın hit + hit.entity != self" deseni burada None/self döndürüp aracı
    // düşürüyordu.
    let hit = world
        .raycast_excluding(&ray, 10.0, BodyHandle::from_id(2))
        .expect("kendini dışlayınca zemin bulunmalı");
    assert_eq!(
        hit.entity,
        BodyHandle::from_id(1),
        "self (kutu) atlanıp zemin dönmeli, oldu: {:?}",
        hit.entity
    );
    assert!(
        hit.point.y.abs() < 1e-3,
        "zemin teması y=0'da olmalı, oldu: {}",
        hit.point.y
    );

    // Karşıtlık: bunun yerine zemini dışlarsak kutu (self) dönmeli → dışlama gerçekten çalışıyor.
    let hit2 = world
        .raycast_excluding(&ray, 10.0, BodyHandle::from_id(1))
        .expect("zemin dışlanınca kutu bulunmalı");
    assert_eq!(hit2.entity, BodyHandle::from_id(2));
}

/// **A step of a whole number of substeps runs that many, at every rate.** A falling body stepped
/// by `k` substeps a call must leave nothing in the accumulator after each call and gain exactly
/// `g·k·substep_dt` of speed per call. Without the accumulator's snap, f32 rounding left 0.99999 of
/// a substep behind at 300 and 600 Hz (three of 1/300 s ran two, then four), and residues crept up
/// at 480 Hz until a call ran nine.
#[test]
fn a_step_of_whole_substeps_runs_exactly_that_many_at_every_rate() {
    for (hz, k) in [(60, 1), (120, 2), (240, 4), (300, 3), (300, 5), (480, 8), (600, 10), (1000, 7)] {
        let mut world = PhysicsWorld::new().with_substep_rate(hz).expect("a rate the world accepts");
        let mut rb = RigidBody::new(1.0, true);
        rb.linear_damping = 0.0;
        world.add_body(BodyHandle::from_id(1), rb, Transform::new(Vec3::new(0.0, 1000.0, 0.0)), Velocity::default(), Collider::sphere(0.5));
        let dt = k as f32 * world.substep_dt();
        let mut before = 0.0f32;
        for call in 0..600 {
            world.step(dt).unwrap();
            assert_eq!(world.accumulator, 0.0, "{hz} Hz, {k} substeps a call: call {call} left {} s", world.accumulator);
            let v = world.velocities[0].linear.y;
            let gained = (before - v) / (9.81 * world.substep_dt());
            assert!((gained - k as f32).abs() < 0.01, "{hz} Hz: call {call} ran {gained:.3} substeps, not {k}");
            before = v;
        }
    }
}

/// A pile settling, awake, for two worlds that differ only in what their contacts carry.
fn resting_pile() -> PhysicsWorld {
    use gizmo_physics_core::{Collider, Transform};
    let mut w = PhysicsWorld::new();
    w.add_body(BodyHandle::from_id(0), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -1.0, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)));
    for i in 1..=3u32 {
        let col = Collider::box_collider(Vec3::splat(0.5));
        let mut rb = RigidBody::new(1.0, true);
        rb.update_inertia_from_collider(&col);
        w.add_body(BodyHandle::from_id(i), rb, Transform::new(Vec3::new(0.1 * i as f32, i as f32 - 0.5, 0.0)), Velocity::default(), col);
    }
    w
}

/// The contacts' warm-start impulses are state: the next step starts its solve from them. Two
/// worlds alike in every pose, velocity and body but one with the contacts its pile had and one
/// with none stepped apart, and until 2026-11-26 `state_hash` called them the same.
#[test]
fn the_contacts_a_solve_leaves_are_in_the_state_hash() {
    let mut settled = resting_pile();
    for _ in 0..10 {
        settled.step(1.0 / 60.0).unwrap();
    }
    assert!(settled.rigid_bodies.iter().all(|b| !b.is_dynamic() || !b.is_sleeping), "premise: the pile slept, and a sleeping body is not solved");
    let mut fresh = resting_pile();
    fresh.transforms.clone_from(&settled.transforms);
    fresh.velocities.clone_from(&settled.velocities);
    fresh.rigid_bodies.clone_from(&settled.rigid_bodies);
    fresh.accumulator = settled.accumulator;
    assert!(settled.contact_cache.keys().count() > 0 && fresh.contact_cache.keys().count() == 0, "premise: no contacts");
    assert_ne!(settled.state_hash(), fresh.state_hash(), "two worlds whose contacts differ hashed alike");

    // Why it matters: the two step apart.
    let (mut a, mut b) = (settled, fresh);
    a.step(1.0 / 60.0).unwrap();
    b.step(1.0 / 60.0).unwrap();
    assert_ne!(
        a.transforms.iter().map(|t| t.position.to_array()).collect::<Vec<_>>(),
        b.transforms.iter().map(|t| t.position.to_array()).collect::<Vec<_>>(),
        "premise: the contacts did not change the next step"
    );

    // One impulse is enough, and so is a pair's slot: the same pairs in other slots hash alike.
    let mut w = resting_pile();
    for _ in 0..10 {
        w.step(1.0 / 60.0).unwrap();
    }
    let base = w.state_hash();
    let pair = *w.contact_cache.keys().next().unwrap();
    let entry = w.contact_cache.remove(&pair).unwrap();
    w.contact_cache.insert((BodyHandle::from_id(98), BodyHandle::from_id(99)), CachedPair { is_trigger: false, manifold: None, seen: false, touched: false });
    w.contact_cache.insert(pair, entry);
    w.contact_cache.remove(&(BodyHandle::from_id(98), BodyHandle::from_id(99)));
    assert_eq!(w.state_hash(), base, "the slot a pair is in counted as state");
    w.contact_cache.get_mut(&pair).unwrap().manifold.as_mut().unwrap().contacts[0].normal_impulse += 0.5;
    assert_ne!(w.state_hash(), base, "a warm-start impulse is in no state hash");
}
