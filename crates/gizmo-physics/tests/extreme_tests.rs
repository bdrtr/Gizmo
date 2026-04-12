use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics::components::{RigidBody, Transform, Velocity};
use gizmo_physics::constraints::{Joint, JointKind, JointWorld};
use gizmo_physics::shape::Collider;
use gizmo_physics::system::physics_collision_system;
use gizmo_physics::{physics_apply_forces_system, physics_movement_system};

fn setup_world() -> World {
    World::new()
}

// ===========================================
// EKSTREM TEST 1: Kuantum Patlaması (Stacking)
// Bir noktaya üst üste sıfır toleransla bindirilmiş 20 obje.
// Çarpışma çözücüsünü (NaN veya sonsuzluğa uçma durumu için) stres testine sokar.
// ===========================================
#[test]
fn test_extreme_quantum_stack_explosion() {
    let mut world = setup_world();
    let num_objects = 20;

    let mut entities = Vec::new();

    // 20 objeyi ufacık bir noktaya (0,0,0) üst üste spawn et, sanki bir kara delik gibi
    for _ in 0..num_objects {
        let e = world.spawn();
        world.add_component(e, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
        world.add_component(e, RigidBody::new(1.0, 0.5, 0.5, false));
        world.add_component(e, Velocity::new(Vec3::ZERO));
        world.add_component(e, Collider::new_sphere(1.0)); // Kocaman küreler iç içe!
        entities.push(e);
    }

    for _ in 0..60 {
        physics_collision_system(&mut world, 0.016);
        physics_apply_forces_system(&world, 0.016);
        physics_movement_system(&world, 0.016);
    }

    // Objelere test: NaN olmamalılar ve şiddetli şekilde dışarı saçılmış olmalılar
    for i in 0..num_objects {
        let t = world
            .borrow::<Transform>()
            .unwrap()
            .get(entities[i].id())
            .unwrap()
            .clone();

        // Asla NaN (Not-a-Number) olmamalı! Fizik motoru patlamamış olmalı.
        assert!(!t.position.x.is_nan());
        assert!(!t.position.y.is_nan());
        assert!(!t.position.z.is_nan());

        // Obje, başlangıç pozisyonu (0,0,0) olan merkezden fırlayıp ayrılmış olmalı
        let dist = t.position.length();
        assert!(
            dist > 0.1,
            "O kadar iç içe obje birbirini dışarı kusmalıydı! Hala merkezde: {}",
            dist
        );
    }
}

// ===========================================
// EKSTREM TEST 2: Göktaşı Çarpışması (Extreme Mass Difference)
// Kütlesi 1 gram olan bir objeye, 1.000.000 Tonluk bir objed çarparsa ne olur?
// ===========================================
#[test]
fn test_extreme_mass_disparity() {
    let mut world = setup_world();

    // Tüy (1 Gram) — çok hafif, fırlayabilir
    let feather = world.spawn();
    world.add_component(feather, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(feather, RigidBody::new(0.001, 0.8, 0.0, true));
    world.add_component(feather, Velocity::new(Vec3::ZERO));
    world.add_component(feather, Collider::new_aabb(0.5, 0.5, 0.5));

    // Ağır top (1000 Kilogram) — hızla geliyor
    let ball = world.spawn();
    // AABB'ler ilk frame'de çakışmalı (SI solver detection bir kere çalışır!)
    world.add_component(ball, Transform::new(Vec3::new(2.0, 0.0, 0.0)));
    world.add_component(ball, RigidBody::new(1000.0, 0.8, 0.0, true));
    world.add_component(ball, Velocity::new(Vec3::new(-10.0, 0.0, 0.0)));
    world.add_component(ball, Collider::new_aabb(1.5, 1.5, 1.5)); // Büyük AABB, tüyle çakışsın

    // Birkaç frame simüle et (çarpışma olsun)
    for _ in 0..20 {
        physics_collision_system(&mut world, 0.016);
        physics_apply_forces_system(&world, 0.016);
        physics_movement_system(&world, 0.016);
    }

    let vel_feather = world
        .borrow::<Velocity>()
        .unwrap()
        .get(feather.id())
        .unwrap()
        .clone();
    let vel_ball = world
        .borrow::<Velocity>()
        .unwrap()
        .get(ball.id())
        .unwrap()
        .clone();

    // Momentum transferi: Tüy fırlamış olmalı (ağır cisim hafif cisme çarptığında büyük hız transferi)
    let feather_speed = vel_feather.linear.length();
    assert!(
        feather_speed > 1.0,
        "Tüy fırlamalıydı! Hız: {:.3} m/s. Momentum transferi çalışmıyor!",
        feather_speed
    );

    // Ağır top neredeyse aynı hızda devam etmeli (tüy çok hafif, enerji transferi minimal)
    assert!(
        vel_ball.linear.x < -5.0,
        "Ağır top yavaşlamamalı! Hız: {:.3} m/s",
        vel_ball.linear.x
    );
}

// ===========================================
// EKSTREM TEST 3: Mikro-Bıçak Tünellemesi (Extreme Aspect Ratio)
// Yüksekliği 1000 metre ama kalınlığı sadece 0.0001 metre olan incecik bir AABB
// CCD algoritmasını sapıtıp sapıtmayacağı testi.
// ===========================================
#[test]
fn test_extreme_needle_wall_ccd() {
    let mut world = setup_world();

    let bullet = world.spawn();
    world.add_component(bullet, Transform::new(Vec3::new(-2.0, 0.0, 0.0)));
    let mut rb = RigidBody::new(1.0, 0.0, 0.0, false);
    rb.ccd_enabled = true;
    world.add_component(bullet, rb);
    world.add_component(bullet, Velocity::new(Vec3::new(500.0, 0.0, 0.0))); // Ciddi hız
    world.add_component(bullet, Collider::new_sphere(0.1));

    let needle = world.spawn();
    world.add_component(needle, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(needle, RigidBody::new_static());
    // X ekseninde ZAR GENİŞLİĞİNDE: 0.0001 (Yarıçapı). Toplam XY kalınlığı : 1 pikselden daha küçük.
    world.add_component(needle, Collider::new_aabb(0.0001, 1000.0, 1000.0));

    // 0.1 sn geçir => 50 m yol alacak
    physics_collision_system(&mut world, 0.1);
    physics_apply_forces_system(&world, 0.1);
    physics_movement_system(&world, 0.1);

    let t = world
        .borrow::<Transform>()
        .unwrap()
        .get(bullet.id())
        .unwrap()
        .clone();
    let v = world
        .borrow::<Velocity>()
        .unwrap()
        .get(bullet.id())
        .unwrap()
        .clone();

    // Merminin hızı ne kadar fazla olursa olsun ve duvar ne kadar atomik incelikte olursa olsun
    // CCD duvarın önünde nesneyi tutmayı başarmalı!
    println!("Bullet Pos: {:?}, Vel: {:?}", t.position, v.linear);
    assert!(t.position.x < 0.0, "Mikro duvarı deldi geçti!");
    assert!(v.linear.x < 1.0, "Hız kesilmedi!");
}

// ===========================================
// EKSTREM TEST 4: Spring Snap (Makaraya dolanan ip kopması simülasyonu)
// Bir objeyi uzaklaştırabildiğimiz kadar uzağa bağlayıp son gücünde serbest bırakırsak?
// ===========================================
#[test]
fn test_extreme_spring_snap() {
    let mut world = setup_world();
    let mut joint_world = JointWorld::new();

    // Dünyanın sonu diyebileceğimiz kadar uzaktaki devasa bir uydudan yay ile dünyaya bağlı.
    let earth = world.spawn();
    world.add_component(earth, Transform::new(Vec3::new(0.0, 0.0, 0.0)));
    world.add_component(earth, RigidBody::new_static());
    world.add_component(earth, Velocity::new(Vec3::ZERO));
    world.add_component(earth, Collider::new_sphere(1.0));

    let satellite = world.spawn();
    world.add_component(satellite, Transform::new(Vec3::new(100000.0, 0.0, 0.0))); // Uçuk mesafe (100km!)
    world.add_component(satellite, RigidBody::new(100.0, 0.5, 0.5, false));
    world.add_component(satellite, Velocity::new(Vec3::ZERO));
    world.add_component(satellite, Collider::new_sphere(1.0));

    // Aralarında sadece 100 m uzunluğunda bir Spring(yay) olsun. (Şu an 99.900m GERİLMİŞ DURUMDA)
    let joint = Joint {
        entity_a: earth.id(),
        entity_b: satellite.id(),
        kind: JointKind::Spring {
            rest_length: 100.0,
            spring_constant: 10000.0,
        }, // Sadece 100m olması genek yay!
        anchor_a: Vec3::ZERO,
        anchor_b: Vec3::ZERO,
        stiffness: 10000.0, // Aşırı sert
        damping: 0.1,
    };
    joint_world.add(joint);

    // Kısıtlayıcıların aşırı mesafeden çekerken fizik kurallarını ihlal (NaN üretmesi) etmemesi gerekir
    // 1 kare (0.016s) serbest bırakalım.
    gizmo_physics::physics_apply_forces_system(&world, 0.016);
    gizmo_physics::physics_movement_system(&world, 0.016);
    world.insert_resource(joint_world);
    gizmo_physics::system::physics_collision_system(&mut world, 0.016);

    let v_sat = world
        .borrow::<Velocity>()
        .unwrap()
        .get(satellite.id())
        .unwrap()
        .clone();

    // Cisim, Dünya tarafına (Negatif X'e) DEHŞET BİR GÜÇLE fırlatılmalı
    assert!(
        v_sat.linear.x < -1000.0,
        "Büyük gerilim çözümsüz fırlatması gerekirdi, gücü yetersiz. Hız: {}",
        v_sat.linear.x
    );
    // Ama NaN olmamalı! Sonsuz olmamalı!
    assert!(
        !v_sat.linear.x.is_nan(),
        "Fizik motoru NaN (Not a Number) patlaması yaşadı!"
    );
    assert!(
        v_sat.linear.x > f32::NEG_INFINITY,
        "Hız sonsuzluğa (Infinity) kaydı!"
    );
}
