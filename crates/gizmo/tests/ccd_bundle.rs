//! CCD'nin İDİOMATİK cepheden (facade) erişilebilirliğinin uçtan uca kanıtı.
//! `RigidBodyBundle::dynamic(..).with_collider(..).with_velocity(..).with_ccd()`
//! zinciriyle (a) ccd_enabled'ın archetype'a ulaştığını ve (b) mach bir merminin
//! ince duvarı `cpu_physics_step_system` üzerinden GERÇEKTEN tünellemediğini gösterir.
//! Bu, "CCD var ama idiomatik API'den açılamıyor" boşluğunun kapandığının kanıtıdır.

use gizmo_engine::bundles::RigidBodyBundle;
use gizmo_engine::core::world::World;
use gizmo_engine::math::Vec3;
use gizmo_engine::physics::components::{Collider, RigidBody};
use gizmo_engine::physics::world::PhysicsWorld;
use gizmo_engine::physics::Transform;
use gizmo_engine::systems::cpu_physics_step_system;

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
        w.borrow::<RigidBody>().get(bullet.id()).unwrap().ccd_enabled,
        "with_ccd() spawn edilen cismin ccd_enabled'ını set etmeli"
    );

    // (b) Uçtan uca: duvar merkezini asla geçmemeli.
    let mut peak = f32::MIN;
    for _ in 0..240 {
        cpu_physics_step_system(&w, 1.0 / 240.0);
        peak = peak.max(w.borrow::<Transform>().get(bullet.id()).unwrap().position.x);
    }
    assert!(peak < 0.0, "idiomatik .with_ccd() tünellemeyi önlemeli, tepe_x={peak}");
}

#[test]
fn without_ccd_the_same_bullet_tunnels() {
    // Negatif kontrol: aynı mermi .with_ccd() OLMADAN tüneller — bayrağın gerçekten
    // fark yarattığının kanıtı (yoksa Rung yukarıdaki her zaman geçer, anlamsız olurdu).
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

    let mut peak = f32::MIN;
    for _ in 0..30 {
        cpu_physics_step_system(&w, 1.0 / 240.0);
        peak = peak.max(w.borrow::<Transform>().get(bullet.id()).unwrap().position.x);
    }
    assert!(peak > 1.0, "CCD'siz idiomatik mermi tünellemeli (kontrast), tepe_x={peak}");
}
