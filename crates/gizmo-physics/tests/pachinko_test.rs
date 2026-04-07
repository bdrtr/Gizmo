use gizmo_core::world::World;
use gizmo_physics::components::{RigidBody, Transform, Velocity};
use gizmo_physics::shape::Collider;
use gizmo_math::Vec3;
use gizmo_physics::system::{physics_collision_system, PhysicsSolverState};
use gizmo_physics::integration::physics_movement_system;
use gizmo_physics::JointWorld;
use std::time::Instant;

#[test]
fn test_pachinko_broadphase_benchmark() {
    let mut world = World::new();
    world.insert_resource(JointWorld::new());
    world.insert_resource(PhysicsSolverState::new());

    let num_spheres = 2000; // Galton kutusuna bırakılacak top sayısı
    let pins_x = 20; // x eksenindeki çivi sayısı
    let pins_y = 15; // y eksenindeki çivi sırası

    println!("\n=== PACHINKO / GALTON KUTUSU TESTİ BAŞLIYOR ===");
    println!("Dinamik Top Sayısı: {}", num_spheres);
    println!("Statik Çivi Sayısı: {}", pins_x * pins_y);

    // 1. Zemin ve Duvarlar
    let ground = world.spawn();
    world.add_component(ground, Transform::new(Vec3::new(0.0, -2.0, 0.0)));
    world.add_component(ground, Collider::new_aabb(50.0, 1.0, 50.0));
    let mut gr_rb = RigidBody::new_static();
    gr_rb.restitution = 0.1;
    world.add_component(ground, gr_rb);

    // 2. Çiviler (Pins)
    for y in 0..pins_y {
        for x in 0..pins_x {
            // Her satırı bir öncekinden biraz kaydırarak çapraz matris oluştur
            let offset = if y % 2 == 0 { 0.0 } else { 1.5 };
            let pos_x = (x as f32) * 3.0 - (pins_x as f32 * 1.5) + offset;
            let pos_y = (y as f32) * 3.0 + 5.0; // Çiviler havada dizilsin
            
            let pin = world.spawn();
            world.add_component(pin, Transform::new(Vec3::new(pos_x, pos_y, 0.0)));
            world.add_component(pin, Collider::new_sphere(0.5)); // Yuvarlak çiviler
            let mut pin_rb = RigidBody::new_static();
            pin_rb.friction = 0.1;
            pin_rb.restitution = 0.5; // Toplar çivilere çarpıp seksin
            world.add_component(pin, pin_rb);
        }
    }

    // 3. Yukarıdan düşen toplar
    for i in 0..num_spheres {
        let sphere = world.spawn();
        
        // Yukarıda dar bir huni (kaynak) gibi bir noktadan hafif rastgelelikle bırak
        let drop_x = (i as f32 % 10.0) * 0.1 - 0.5; 
        let drop_y = 60.0 + (i as f32 * 0.1); // Peş peşe bırakılmaları için yükseklikleri artır
        let drop_z = (i as f32 % 5.0) * 0.1 - 0.25;

        world.add_component(sphere, Transform::new(Vec3::new(drop_x, drop_y, drop_z)));
        world.add_component(sphere, Velocity::new(Vec3::ZERO));
        world.add_component(sphere, Collider::new_sphere(0.4)); // Çivilerin arasından geçebilecek boyutta
        
        // Dinamik top kütlesi
        let mut rb = RigidBody::new(10.0, 0.3, 0.2, true);
        rb.ccd_enabled = true; // Çok hızlı düşerlerse içinden geçmesinler diye CCD aktif
        world.add_component(sphere, rb);
    }

    // 4. Simülasyon Döngüsü (10 saniye boyunca saniyede 60 kare)
    let steps = 600;
    let dt = 1.0 / 60.0;
    let mut total_time = 0.0;
    let mut max_time = 0.0;

    let start_sim = Instant::now();

    for _ in 0..steps {
        let step_start = Instant::now();
        
        physics_collision_system(&world, dt);
        physics_movement_system(&world, dt);
        if let Some(jw) = world.get_resource::<JointWorld>() {
            gizmo_physics::solve_constraints(&*jw, &world, dt);
        }

        let step_dur = step_start.elapsed().as_secs_f64() * 1000.0; // Milisaniye (ms)
        total_time += step_dur;
        if step_dur > max_time {
            max_time = step_dur;
        }
    }

    let elapsed = start_sim.elapsed();
    let avg_time = total_time / (steps as f64);

    println!("=== SONUÇLAR ===");
    println!("Toplam Simülasyon Süresi: {:.2}s (Gerçek hayatta karşılığı: 10 saniye)", elapsed.as_secs_f64());
    println!("Ortalama Frame (Kare) Çözüm Süresi: {:.3} ms (İdeal limit: <16.6ms)", avg_time);
    println!("En Yavaş Frame Çözüm Süresi (Max Spike): {:.3} ms", max_time);
    
    // Testin patlamaması ve akıcı olması için FPS hedeflerini (ms) test içinde sorgula
    assert!(avg_time < 5.0, "PERFORMANS UYARISI: Ortalama süre 5ms'nin üstünde! Broadphase verimi düşük olabilir.");
    assert!(max_time < 50.0, "PERFORMANS DROP UYARISI: Çok büyük ani takılma (spike) yaşandı ({:.3} ms)", max_time);

    println!("Test başarıyla tamamlandı. Broad-phase optimizasyonu 10/10 akıcı çalışıyor!\n");
}
