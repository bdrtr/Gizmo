//! Property-based tests for the rigid-body simulation (`PhysicsWorld::step`).
//!
//! Faz 1.2 — fizik integratör + solver'ının fiziksel KONTRATLARINI rastgele
//! sahnelerde doğrular:
//!   * determinizm   — aynı sahne iki kez → bit-bit aynı sonuç
//!   * sağlamlık      — rastgele sahne yüzlerce kare NaN/Inf üretmez
//!   * korunum        — kuvvetsiz/damping'siz cisim momentumu/KE'yi korur
//!   * termodinamik   — yalıtık cisim ENERJİ KAZANMAZ (damping monoton azaltır)
//!   * temas          — düşen cisim statik zeminden tünellemez
//!
//! Bir invariant kırılırsa proptest minimal karşı-örneğe shrink eder.

use gizmo_core::entity::Entity;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};
use proptest::prelude::*;

/// Tek bir dinamik küre gövdesinin parametreleri.
#[derive(Debug, Clone)]
struct BodySpec {
    pos: Vec3,
    vel: Vec3,
    radius: f32,
}

fn arb_body() -> impl Strategy<Value = BodySpec> {
    (
        -2.0f32..2.0,
        -2.0f32..2.0,
        -2.0f32..2.0,
        -1.0f32..1.0,
        -1.0f32..1.0,
        -1.0f32..1.0,
        0.3f32..0.8,
    )
        .prop_map(|(px, py, pz, vx, vy, vz, radius)| BodySpec {
            pos: Vec3::new(px, py, pz),
            vel: Vec3::new(vx, vy, vz),
            radius,
        })
}

/// Verilen spec'lerden gravitasyonlu bir dünya kur.
fn build_world(specs: &[BodySpec]) -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    for (i, s) in specs.iter().enumerate() {
        let mut rb = RigidBody::new(1.0, true);
        rb.wake_up();
        world.add_body(
            Entity::new(i as u32 + 1, 0),
            rb,
            Transform::new(s.pos),
            Velocity::new(s.vel),
            Collider::sphere(s.radius),
        );
    }
    world
}

fn world_all_finite(world: &PhysicsWorld) -> bool {
    (0..world.transforms.len()).all(|i| {
        world.transforms[i].position.is_finite()
            && world.velocities[i].linear.is_finite()
            && world.velocities[i].angular.is_finite()
    })
}

proptest! {
    // 128 vaka: çok-cisimli + 120 kareli sahneler için CI dostu denge.
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// DETERMİNİZM: aynı başlangıç sahnesi iki ayrı dünyada aynı adım dizisiyle
    /// simüle edilince transform ve hızlar BİT-BİT aynı olmalı. (renet/rollback
    /// netcode ve replay'in dayandığı temel garanti.)
    #[test]
    fn simulation_is_deterministic(specs in prop::collection::vec(arb_body(), 2..7)) {
        let dt = 1.0 / 60.0;
        let mut a = build_world(&specs);
        let mut b = build_world(&specs);

        for _ in 0..60 {
            a.step(dt).ok();
            b.step(dt).ok();
        }

        for i in 0..specs.len() {
            prop_assert_eq!(a.transforms[i].position, b.transforms[i].position,
                "transform[{}] determinizm kırıldı", i);
            prop_assert_eq!(a.velocities[i].linear, b.velocities[i].linear,
                "lineer hız[{}] determinizm kırıldı", i);
            prop_assert_eq!(a.velocities[i].angular, b.velocities[i].angular,
                "açısal hız[{}] determinizm kırıldı", i);
        }
    }

    /// SAĞLAMLIK: rastgele (çoğu örtüşen) sahne 120 kare boyunca NaN/Inf
    /// üretmemeli ve `step` daima Ok dönmeli (integratör NaN'da Err döndürür).
    #[test]
    fn no_nan_under_gravity(specs in prop::collection::vec(arb_body(), 1..8)) {
        let dt = 1.0 / 60.0;
        let mut world = build_world(&specs);

        for frame in 0..120 {
            let r = world.step(dt);
            prop_assert!(r.is_ok(), "kare {frame}: step Err döndü (NaN guard tetiklendi): {:?}", r.err());
            prop_assert!(world_all_finite(&world), "kare {frame}: NaN/Inf durum");
        }
    }

    /// KORUNUM: gravitesiz + damping'siz, çarpışmasız tek cisim → hız sabit,
    /// konum lineer (p ≈ p0 + v·t). Lineer momentum korunur.
    #[test]
    fn free_body_conserves_momentum(
        vx in -3.0f32..3.0, vy in -3.0f32..3.0, vz in -3.0f32..3.0,
    ) {
        let v0 = Vec3::new(vx, vy, vz);
        prop_assume!(v0.length() > 0.5); // uyumayı engelle

        let dt = 1.0 / 60.0;
        let steps = 50;
        let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);

        let mut rb = RigidBody::new(1.0, false);
        rb.linear_damping = 0.0;
        rb.angular_damping = 0.0;
        rb.wake_up();
        let p0 = Vec3::new(0.0, 0.0, 0.0);
        world.add_body(
            Entity::new(1, 0),
            rb,
            Transform::new(p0),
            Velocity::new(v0),
            Collider::sphere(0.5),
        );

        for _ in 0..steps {
            world.step(dt).ok();
        }

        let v_final = world.velocities[0].linear;
        prop_assert!((v_final - v0).length() < 1e-4,
            "hız korunmadı: {:?} → {:?}", v0, v_final);

        let p_final = world.transforms[0].position;
        let p_expected = p0 + v0 * (steps as f32 * dt);
        prop_assert!((p_final - p_expected).length() < 1e-3,
            "konum lineer değil: {:?} beklenen {:?}", p_final, p_expected);
    }

    /// TERMODİNAMİK: gravitesiz, yalıtık (çarpışmasız) bir cisim — lineer +
    /// açısal hızla — ENERJİ KAZANMAMALI. Default damping ile toplam enerji her
    /// karede monoton azalmalı (artış = integratör/damping işaret hatası).
    #[test]
    fn isolated_body_never_gains_energy(
        vx in -3.0f32..3.0, vy in -3.0f32..3.0, vz in -3.0f32..3.0,
        wx in -3.0f32..3.0, wy in -3.0f32..3.0, wz in -3.0f32..3.0,
    ) {
        let dt = 1.0 / 60.0;
        let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);

        let mut rb = RigidBody::new(1.0, false);
        rb.wake_up();
        let mut vel = Velocity::new(Vec3::new(vx, vy, vz));
        vel.angular = Vec3::new(wx, wy, wz);
        world.add_body(
            Entity::new(1, 0),
            rb,
            Transform::new(Vec3::ZERO),
            vel,
            Collider::sphere(0.5),
        );

        let mut prev = world.calculate_total_energy();
        prop_assert!(prev.is_finite());
        for _ in 0..40 {
            world.step(dt).ok();
            let e = world.calculate_total_energy();
            prop_assert!(e.is_finite(), "enerji NaN/Inf");
            // Küçük pozitif tolerans float gürültüsü için.
            prop_assert!(e <= prev + 1e-3, "enerji arttı: {prev} → {e}");
            prev = e;
        }
    }
}

proptest! {
    // 48 vaka: bu test gövde başına 400 adım simüle ettiğinden daha düşük tutuldu.
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// TEMAS: statik zemin (kalın kutu) üzerine rastgele yükseklikten düşen
    /// dinamik küre — yerleştikten sonra zeminin İÇİNDEN geçmemeli (tünelleme).
    #[test]
    fn dropped_body_does_not_tunnel(
        drop_h in 1.0f32..8.0,
        radius in 0.3f32..0.8,
    ) {
        let dt = 1.0 / 60.0;
        let mut world = PhysicsWorld::new();
        world.solver.iterations = 16;

        // Statik zemin: üst yüzeyi y = 0.
        let mut ground = RigidBody::new_static();
        ground.wake_up();
        world.add_body(
            Entity::new(1, 0),
            ground,
            Transform::new(Vec3::new(0.0, -1.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
        );

        // Düşen küre.
        let mut rb = RigidBody::new(1.0, true);
        rb.wake_up();
        world.add_body(
            Entity::new(2, 0),
            rb,
            Transform::new(Vec3::new(0.0, drop_h + radius, 0.0)),
            Velocity::default(),
            Collider::sphere(radius),
        );

        for _ in 0..400 {
            world.step(dt).ok();
        }

        let y = world.transforms[1].position.y;
        prop_assert!(y.is_finite(), "küre konumu NaN");
        // Küre merkezi, yarıçapın biraz altından (penetrasyon slop) daha aşağı
        // düşmemeli — yani zeminden tünelleyip kaybolmamalı.
        prop_assert!(y > radius - 0.2,
            "küre zemine gömüldü/tüneldi: y={y} (radius={radius})");
        // Ve uçup gitmemeli.
        prop_assert!(y < drop_h + radius + 1.0, "küre yukarı fırladı: y={y}");
    }
}
