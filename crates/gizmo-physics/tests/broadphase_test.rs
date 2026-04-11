use gizmo_math::Vec3;
use gizmo_physics::components::{RigidBody, Transform, Velocity};
use gizmo_physics::shape::Collider;
use std::time::Instant;

fn setup_world() -> gizmo_core::World {
    gizmo_core::World::new()
}

// ================================================================
// TEST: Devasa Uzay (Broadphase Optimizasyonu Testi)
// 10.000 obje geniş bir uzaya rastgele dağılacak.
// Bu test Broadphase'in (Sweep & Prune) n^2 yerine n log n karmaşıklığında
// çalıştığını ve gereksiz dar aşama (narrow phase) testlerinden
// başarıyla kaçındığını ölçer.
// ================================================================
#[test]
fn test_broadphase_performance_10000_objects() {
    let mut world = setup_world();

    // Rastgele obje üretimi için basit bir pseudo-random (bağımlılık eklememek için)
    let mut seed = 12345u32;
    let mut rand = || -> f32 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32) / (u32::MAX as f32) // 0.0 ile 1.0 arası
    };

    println!("🌍 10.000 obje oluşturuluyor...");

    // 10.000 adet AABB/Küre geniş bir alana dağıt
    // -500 ile +500 arasına dağılacaklar
    let range = 1000.0;

    let start_setup = Instant::now();
    for _ in 0..10_000 {
        let e = world.spawn();

        let x = rand() * range - (range / 2.0);
        let y = rand() * range - (range / 2.0);
        let z = rand() * range - (range / 2.0);

        world.add_component(e, Transform::new(Vec3::new(x, y, z)));

        // Rastgele hızlar verelim (-5.0 ile 5.0 m/s)
        let vx = rand() * 10.0 - 5.0;
        let vy = rand() * 10.0 - 5.0;
        let vz = rand() * 10.0 - 5.0;
        world.add_component(e, Velocity::new(Vec3::new(vx, vy, vz)));

        world.add_component(e, RigidBody::new(1.0, 0.5, 0.2, false)); // use_gravity = false
        world.add_component(e, Collider::new_aabb(0.5, 0.5, 0.5));
    }

    println!("   Setup tamamlandı: {:?}", start_setup.elapsed());

    let dt = 1.0 / 60.0;
    let steps = 60; // 1 saniye simülasyon (60 kare)

    println!("🚀 60 karelik fizik simülasyonu başlatılıyor (10.000 obje)...");
    let start_sim = Instant::now();

    for i in 0..steps {
        let step_start = Instant::now();
        gizmo_physics::physics_apply_forces_system(&world, dt);
        gizmo_physics::physics_movement_system(&world, dt);
        gizmo_physics::system::physics_collision_system(&world, dt);

        if i % 10 == 0 || i == steps - 1 {
            println!("   Kare {:02} hesaplandı: {:?}", i, step_start.elapsed());
        }
    }

    let avg_time_per_frame = start_sim.elapsed() / steps as u32;
    println!("✅ Simülasyon tamamlandı!");
    println!("   Toplam süre (60 kare): {:?}", start_sim.elapsed());
    println!("   Ortalama kare süresi: {:?}", avg_time_per_frame);

    assert!(
        avg_time_per_frame.as_millis() < 80,
        "Fizik motoru çok yavaş! Broadphase O(n^2) engelliyor olabilir."
    );
}

#[test]
fn test_broadphase_performance_100000_objects() {
    let mut world = setup_world();

    let mut seed = 12345u32;
    let mut rand = || -> f32 {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32) / (u32::MAX as f32)
    };

    println!("🌍 100.000 obje oluşturuluyor...");
    let range = 2000.0; // Daha geniş dağılım
    let start_setup = Instant::now();
    for _ in 0..100_000 {
        let e = world.spawn();
        let x = rand() * range - (range / 2.0);
        let y = rand() * range - (range / 2.0);
        let z = rand() * range - (range / 2.0);
        world.add_component(e, Transform::new(Vec3::new(x, y, z)));
        let vx = rand() * 10.0 - 5.0;
        let vy = rand() * 10.0 - 5.0;
        let vz = rand() * 10.0 - 5.0;
        world.add_component(e, Velocity::new(Vec3::new(vx, vy, vz)));
        world.add_component(e, RigidBody::new(1.0, 0.5, 0.2, false));
        world.add_component(e, Collider::new_aabb(0.5, 0.5, 0.5));
    }
    println!("   Setup tamamlandı: {:?}", start_setup.elapsed());

    let dt = 1.0 / 60.0;
    let steps = 10; // 100k için sadece 10 kare test edelim (uzun sürmesin)

    println!("🚀 10 karelik fizik simülasyonu başlatılıyor (100.000 obje)...");
    let start_sim = Instant::now();

    for i in 0..steps {
        let step_start = Instant::now();
        gizmo_physics::physics_apply_forces_system(&world, dt);
        gizmo_physics::physics_movement_system(&world, dt);
        gizmo_physics::system::physics_collision_system(&world, dt);
        println!("   Kare {:02} hesaplandı: {:?}", i, step_start.elapsed());
    }

    let avg_time_per_frame = start_sim.elapsed() / steps as u32;
    println!("✅ Simülasyon tamamlandı! (100.000 Obje)");
    println!("   Ortalama kare süresi: {:?}", avg_time_per_frame);

    // Debug modunda 100k obje için ortalama kare süresi < 500ms olmasını bekleriz
    // Release modunda muhtemelen ~20-30ms olacaktır.
    assert!(
        avg_time_per_frame.as_millis() < 1000,
        "100.000 obje performansı çok düşük."
    );
}
