//! CCD'nin İDİOMATİK cepheden (facade) erişilebilirliğinin uçtan uca kanıtı.
//! `RigidBodyBundle::dynamic(..).with_collider(..).with_velocity(..).with_ccd()`
//! zinciriyle (a) ccd_enabled'ın archetype'a ulaştığını ve (b) mach bir merminin
//! ince duvarı `cpu_physics_step_system` üzerinden GERÇEKTEN tünellemediğini gösterir.
//! Bu, "CCD var ama idiomatik API'den açılamıyor" boşluğunun kapandığının kanıtıdır.

use gizmo::bundles::RigidBodyBundle;
use gizmo::core::world::World;
use gizmo::math::Vec3;
use gizmo::physics::components::{Collider, RigidBody};
use gizmo::physics::world::PhysicsWorld;
use gizmo::physics::Transform;
use gizmo::systems::cpu_physics_step_system;

#[test]
fn with_ccd_sets_flag_and_prevents_tunnel_end_to_end() {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));

    // İnce statik duvar — idiomatik bundle (ön yüz x=-0.02).
    w.spawn_bundle((
        Transform::new(Vec3::ZERO),
        RigidBodyBundle::static_body()
            .with_collider(Collider::box_collider(Vec3::new(0.02, 5.0, 5.0))),
    ));

    // Hızlı mermi — tamamen idiomatik zincir; .with_ccd() olayın özü.
    let bullet = w.spawn_bundle((
        Transform::new(Vec3::new(-5.0, 0.0, 0.0)),
        RigidBodyBundle::dynamic(1.0)
            .with_collider(Collider::sphere(0.05))
            .with_velocity(Vec3::new(2400.0, 0.0, 0.0)) // 10 m/alt-adım ≫ 0.14 yakalama bandı
            .with_ccd(),
    ));

    // (a) Bayrak archetype'a ulaştı mı? (gather *rb + sync_bodies bunu koruyor.)
    assert!(
        w.query::<&RigidBody>().get(bullet).unwrap().ccd_enabled,
        "with_ccd() spawn edilen cismin ccd_enabled'ını set etmeli"
    );

    // (b) Uçtan uca: duvar merkezini asla geçmemeli.
    let mut peak = f32::MIN;
    for _ in 0..240 {
        cpu_physics_step_system(&w, 1.0 / 240.0);
        peak = peak.max(w.query::<&Transform>().get(bullet).unwrap().position.x);
    }
    assert!(peak < 0.0, "idiomatik .with_ccd() tünellemeyi önlemeli, tepe_x={peak}");
}

#[test]
fn without_ccd_the_flag_stays_off_and_the_bullet_still_stops() {
    // The same bullet without `.with_ccd()`: the flag stays off — so the test above is about the
    // flag reaching the archetype — and the bullet stops at the wall all the same. Until 2026-11-24
    // it went through (this was the contrast); a pair closing faster than its own size allows now
    // gets a speculative contact whether or not either body asked for CCD (`closes_fast`).
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));

    w.spawn_bundle((
        Transform::new(Vec3::ZERO),
        RigidBodyBundle::static_body()
            .with_collider(Collider::box_collider(Vec3::new(0.02, 5.0, 5.0))),
    ));

    let bullet = w.spawn_bundle((
        Transform::new(Vec3::new(-5.0, 0.0, 0.0)),
        RigidBodyBundle::dynamic(1.0)
            .with_collider(Collider::sphere(0.05))
            .with_velocity(Vec3::new(2400.0, 0.0, 0.0)), // .with_ccd() YOK
    ));
    assert!(!w.query::<&RigidBody>().get(bullet).unwrap().ccd_enabled);

    let mut peak = f32::MIN;
    for _ in 0..30 {
        cpu_physics_step_system(&w, 1.0 / 240.0);
        peak = peak.max(w.query::<&Transform>().get(bullet).unwrap().position.x);
    }
    assert!(peak < 0.0, "CCD'siz idiomatik mermi de duvarın önünde durmalı, tepe_x={peak}");
}
