//! Determinism CI Testi
//!
//! Aynı fizik sahnesini N frame simüle edip state hash'ini karşılaştırır.
//! Bu test, farklı çalıştırmalarda aynı sonuçların üretildiğini doğrular.
//! CI pipeline'da her commit'te çalıştırılarak multiplayer güvenilirliği sağlanır.

use gizmo_core::entity::Entity;
use gizmo_math::Vec3;
use gizmo_physics::{
    components::{Collider, RigidBody, Transform, Velocity},
    world::PhysicsWorld,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Float değerlerini belirli bir hassasiyete yuvarlayarak hash'lenebilir yapar.
/// Bu, floating-point kararsızlığını önler.
fn quantize(val: f32, precision: i32) -> i64 {
    let scale = 10f32.powi(precision);
    (val * scale).round() as i64
}

/// Vec3 hash'i (quantized)
fn hash_vec3(v: Vec3, precision: i32) -> u64 {
    let mut hasher = DefaultHasher::new();
    quantize(v.x, precision).hash(&mut hasher);
    quantize(v.y, precision).hash(&mut hasher);
    quantize(v.z, precision).hash(&mut hasher);
    hasher.finish()
}

/// Tüm PhysicsWorld state'ini hash'ler
fn hash_physics_state(world: &PhysicsWorld, precision: i32) -> u64 {
    let mut hasher = DefaultHasher::new();

    // Entity sayısı
    world.entities.len().hash(&mut hasher);

    // Her body'nin pozisyon ve velocity'si
    for i in 0..world.entities.len() {
        let t = &world.transforms[i];
        let v = &world.velocities[i];

        hash_vec3(t.position, precision).hash(&mut hasher);
        hash_vec3(v.linear, precision).hash(&mut hasher);
        hash_vec3(v.angular, precision).hash(&mut hasher);
    }

    hasher.finish()
}

/// Basit bir fizik sahnesi oluşturur: farklı yüksekliklerde 10 küre,
/// birbirine çarpar ve yerleşir.
fn create_deterministic_scene() -> PhysicsWorld {
    let mut world = PhysicsWorld::new();

    // Zemin (statik)
    let ground = Entity::new(1000, 0);
    let mut ground_rb = RigidBody::new_static();
    ground_rb.use_gravity = false;
    world.add_body(
        ground,
        ground_rb,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(50.0, 1.0, 50.0)),
    );

    // 10 dinamik küre — farklı yüksekliklerde
    for i in 0..10 {
        let entity = Entity::new(i + 1, 0);
        let mut rb = RigidBody::default();
        rb.mass = 1.0 + i as f32 * 0.5;
        rb.restitution = 0.3;

        let x = (i as f32 - 4.5) * 2.0;
        let y = 5.0 + i as f32 * 1.5;

        world.add_body(
            entity,
            rb,
            Transform::new(Vec3::new(x, y, 0.0)),
            Velocity::default(),
            Collider::sphere(0.5),
        );
    }

    world
}

/// Sahneyi N frame simüle eder ve final state hash'ini döndürür.
fn simulate_and_hash(frames: u32, dt: f32, precision: i32) -> u64 {
    let mut world = create_deterministic_scene();

    for _ in 0..frames {
        let _ = world.step(&mut [], &mut [], dt);
    }

    hash_physics_state(&world, precision)
}

#[test]
fn test_determinism_same_run() {
    // Aynı parametrelerle 2 kez çalıştır — hash aynı olmalı
    let hash1 = simulate_and_hash(120, 1.0 / 60.0, 4);
    let hash2 = simulate_and_hash(120, 1.0 / 60.0, 4);

    assert_eq!(
        hash1, hash2,
        "Determinism hatası! Aynı parametrelerle iki çalıştırma farklı sonuç verdi:\n  hash1: {}\n  hash2: {}",
        hash1, hash2
    );
}

#[test]
fn test_determinism_multiple_runs() {
    // 5 kez çalıştır — tüm hash'ler aynı olmalı
    let hashes: Vec<u64> = (0..5)
        .map(|_| simulate_and_hash(60, 1.0 / 60.0, 4))
        .collect();

    for (i, h) in hashes.iter().enumerate().skip(1) {
        assert_eq!(
            hashes[0], *h,
            "Determinism hatası run #0 vs #{}: {} != {}",
            i, hashes[0], h
        );
    }
}

#[test]
fn test_determinism_longer_simulation() {
    // Uzun simülasyon (5 saniye @ 60fps = 300 frame)
    let hash1 = simulate_and_hash(300, 1.0 / 60.0, 3); // Daha düşük hassasiyet
    let hash2 = simulate_and_hash(300, 1.0 / 60.0, 3);

    assert_eq!(
        hash1, hash2,
        "Uzun simülasyonda determinism hatası:\n  hash1: {}\n  hash2: {}",
        hash1, hash2
    );
}

#[test]
fn test_determinism_state_divergence_detection() {
    // Farklı başlangıç koşulları → farklı hash üretilmeli (sanity check)
    // Motor fixed 240Hz substep kullandığı için farklı dt aynı sonucu verir,
    // bu yüzden farklı sahne konfigürasyonuyla test ediyoruz.
    let hash_normal = simulate_and_hash(120, 1.0 / 60.0, 4);

    // Farklı sahne: ekstra bir cisim ekle
    let mut world = create_deterministic_scene();
    let extra = Entity::new(999, 0);
    let mut rb = RigidBody::default();
    rb.mass = 10.0;
    world.add_body(
        extra,
        rb,
        Transform::new(Vec3::new(0.0, 20.0, 0.0)),
        Velocity::default(),
        Collider::sphere(1.0),
    );
    for _ in 0..120 {
        let _ = world.step(&mut [], &mut [], 1.0 / 60.0);
    }
    let hash_extra = hash_physics_state(&world, 4);

    assert_ne!(
        hash_normal, hash_extra,
        "Sanity check: Farklı sahneler aynı hash üretti — hash fonksiyonu çalışmıyor olabilir"
    );
}

#[test]
fn test_determinism_hash_sensitivity() {
    // 1 frame fark bile hash'i değiştirmeli (sanity check)
    let hash_99 = simulate_and_hash(99, 1.0 / 60.0, 4);
    let hash_100 = simulate_and_hash(100, 1.0 / 60.0, 4);

    assert_ne!(
        hash_99, hash_100,
        "Sanity check: 99 vs 100 frame aynı hash üretti — hash yeterli hassasiyette değil"
    );
}
