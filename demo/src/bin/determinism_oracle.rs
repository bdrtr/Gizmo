//! Determinizm oracle (Faz 2): kanonik küçük sahneyi koşar ve `state_hash`'i basar.
//! Cross-process determinizm testi (demo/tests/cross_process_determinism.rs) bunu İKİ
//! AYRI SÜREÇTE çalıştırıp hash'leri karşılaştırır — farklı süreçlerin farklı HashMap
//! taban-seed'i aldığı halde çıktının eşit kalması, aynı-platform determinizmini kanıtlar.
//!
//! Sahne KÜÇÜK tutulur (debug'da hızlı): temas/island/uyku yollarını uyaracak kadar kutu.

use gizmo::core::entity::Entity;
use gizmo::math::Vec3;
use gizmo::physics::components::{Collider, RigidBody, Transform, Velocity};
use gizmo::physics::world::PhysicsWorld;

fn main() {
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

    // 6×6 sütun × 2 kat = 72 kutu, küçük boşlukla düşüp çarpar/yerleşir.
    let mut id = 1u32;
    for x in 0..6 {
        for z in 0..6 {
            for ly in 0..2 {
                let mut rb = RigidBody::new(1.0, 0.2, 0.6, true);
                rb.wake_up();
                let col = Collider::box_collider(Vec3::splat(0.5));
                rb.update_inertia_from_collider(&col);
                let px = (x as f32 - 2.5) * 1.1;
                let pz = (z as f32 - 2.5) * 1.1;
                let py = 0.5 + ly as f32 * 1.1 + 0.05;
                world.add_body(
                    Entity::new(id, 0),
                    rb,
                    Transform::new(Vec3::new(px, py, pz)),
                    Velocity::default(),
                    col,
                );
                id += 1;
            }
        }
    }

    for _ in 0..120 {
        world.step(1.0 / 60.0).ok();
    }

    // Test'in ayrıştırdığı tek satır.
    println!("STATE_HASH={:016X}", world.state_hash());
}
