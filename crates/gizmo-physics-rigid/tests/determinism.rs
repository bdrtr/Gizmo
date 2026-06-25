//! Determinizm garantisi (Faz 2 — aynı-platform replay/rollback hedefi).
//!
//! `PhysicsWorld::state_hash()` rollback/replay desync tespiti için sync-hash API'sidir.
//! Bu testler AYNI-PLATFORM determinizmini kilitler:
//!   * İki ÖZDEŞ dünya, aynı adımlarla → AYNI hash (her dünya kendi HashMap'lerini farklı
//!     seed'le kurar; çıktı eşitse engine hash-iterasyon-sırasından BAĞIMSIZ demektir),
//!   * adım sonrası hash değişir (state ilerliyor),
//!   * iki dünyadan biri perturbe edilirse hash AYRIŞIR (desync tespiti çalışır).
//!
//! Cross-platform bit-exact KAPSAM DIŞI (sim f32/glam; bkz docs/determinism.md). Süreçler-
//! arası determinizm `demo/tests/cross_process_determinism.rs` ile ayrıca doğrulanır.

use gizmo_core::entity::Entity;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn build_scene() -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    let mut ground = RigidBody::new_static();
    ground.wake_up();
    world.add_body(
        Entity::new(0, 0),
        ground,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
    );
    // Birkaç düşen+çarpan kutu (temas/island/uyku yollarını uyarır).
    let mut id = 1u32;
    for x in 0..4 {
        for ly in 0..3 {
            let mut rb = RigidBody::new(1.0, true);
            rb.wake_up();
            let col = Collider::box_collider(Vec3::splat(0.5));
            rb.update_inertia_from_collider(&col);
            let px = (x as f32 - 1.5) * 1.05;
            let py = 0.5 + ly as f32 * 1.1 + 0.05;
            world.add_body(
                Entity::new(id, 0),
                rb,
                Transform::new(Vec3::new(px, py, 0.0)),
                Velocity::default(),
                col,
            );
            id += 1;
        }
    }
    world
}

#[test]
fn identical_worlds_produce_identical_hash() {
    // İki ÖZDEŞ dünya, aynı adım sayısı → AYNI state_hash. Her dünya iç HashMap'lerini
    // ayrı seed'le kurar; hash eşitse engine HashMap-iterasyon-sırasından bağımsızdır
    // (island sort + quickhull BTree fix'lerinin garantisi). Aynı-platform determinizmi.
    let mut a = build_scene();
    let mut b = build_scene();
    for _ in 0..120 {
        a.step(1.0 / 60.0).ok();
        b.step(1.0 / 60.0).ok();
    }
    assert_eq!(
        a.state_hash(),
        b.state_hash(),
        "özdeş dünyalar farklı hash üretti → determinizm bozuk (HashMap-sıra bağımlılığı?)"
    );
}

#[test]
fn hash_changes_as_simulation_advances() {
    let mut w = build_scene();
    let h0 = w.state_hash();
    for _ in 0..30 {
        w.step(1.0 / 60.0).ok();
    }
    let h1 = w.state_hash();
    assert_ne!(h0, h1, "simülasyon ilerledi ama hash değişmedi");
}

#[test]
fn hash_detects_divergence() {
    // İki dünya aynı; birine küçük bir perturbasyon → hash AYRIŞMALI (desync tespiti).
    let mut a = build_scene();
    let mut b = build_scene();
    for _ in 0..20 {
        a.step(1.0 / 60.0).ok();
        b.step(1.0 / 60.0).ok();
    }
    // b'de bir cismi hafifçe it (rollback desync simülasyonu).
    b.velocities[1].linear.x += 0.5;
    for _ in 0..20 {
        a.step(1.0 / 60.0).ok();
        b.step(1.0 / 60.0).ok();
    }
    assert_ne!(
        a.state_hash(),
        b.state_hash(),
        "perturbasyona rağmen hash aynı → desync tespiti çalışmıyor"
    );
}
