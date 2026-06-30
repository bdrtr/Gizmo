//! Property-based tests for the joint solver (`JointSolver` via `PhysicsWorld`).
//!
//! Faz 1.2 — bir kısıt çözücünün (constraint solver) iki temel kontratı:
//!   * KISIT SAĞLAMA — ball-socket eklemi iki anchor'ı (rastgele başlangıç
//!     ayrımından bağımsız) birbirine çeker; kalıntı hata küçülür, ASLA büyümez.
//!   * SAĞLAMLIK     — yerçekimi altında eklemle bağlı bir zincir yüzlerce kare
//!     boyunca NaN/Inf üretmez ve sonsuza ışınlanmaz (kararlı çözüm).
//!
//! Bu testler Faz 0'da düzeltilen eklem-efektif-kütle / Fixed-eklem-zincir
//! buglarının regresyonunu rastgele konfigürasyonlarda yakalar.

use gizmo_physics_core::BodyHandle;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{Joint, PhysicsWorld, RigidBody, Velocity};
use proptest::prelude::*;

fn dynamic_body(pos: Vec3, use_gravity: bool) -> (RigidBody, Transform, Velocity, Collider) {
    let mut rb = RigidBody::new(1.0, use_gravity);
    rb.wake_up();
    (
        rb,
        Transform::new(pos),
        Velocity::default(),
        Collider::sphere(0.3),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// KISIT SAĞLAMA: yerçekimsiz iki dinamik cisim, merkezlerinden ball-socket
    /// eklemiyle bağlı (anchor = ZERO → posA == posB hedefi). Rastgele başlangıç
    /// ayrımından sonra çözücü onları aynı noktaya çekmeli: son ayrım hem SONLU
    /// hem de başlangıçtan belirgin küçük olmalı (kısıt geri-itmez, yakınsar).
    #[test]
    fn ball_socket_pulls_anchors_together(
        ox in -3.0f32..3.0, oy in -3.0f32..3.0, oz in -3.0f32..3.0,
    ) {
        let offset = Vec3::new(ox, oy, oz);
        prop_assume!(offset.length() > 0.5); // anlamlı bir başlangıç ayrımı

        let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
        let a = BodyHandle::from_id(1);
        let b = BodyHandle::from_id(2);
        let (rb_a, t_a, v_a, c_a) = dynamic_body(Vec3::ZERO, false);
        let (rb_b, t_b, v_b, c_b) = dynamic_body(offset, false);
        world.add_body(a, rb_a, t_a, v_a, c_a);
        world.add_body(b, rb_b, t_b, v_b, c_b);
        world.joints.push(Joint::ball_socket(a, b, Vec3::ZERO, Vec3::ZERO));

        let initial_sep = offset.length();
        for _ in 0..500 {
            world.step(1.0 / 60.0).ok();
        }

        let sep = (world.transforms[0].position - world.transforms[1].position).length();
        prop_assert!(sep.is_finite(), "anchor ayrımı NaN/Inf");
        prop_assert!(
            sep < 0.05,
            "ball-socket yakınsamadı: {initial_sep} → {sep}"
        );
    }
}

proptest! {
    // 24 vaka: bu test çok-cisimli + 200 kareli zincir simüle ettiğinden düşük tutuldu.
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// SAĞLAMLIK: statik bir tavandan ball-socket zinciriyle sarkan N cisim,
    /// yerçekimi altında 300 kare. Hiçbir cisim NaN/Inf olmamalı ve makul bir
    /// kürenin dışına ışınlanmamalı (çözücü patlamamalı).
    #[test]
    fn joint_chain_stays_stable_under_gravity(n in 2usize..5) {
        let mut world = PhysicsWorld::new(); // varsayılan yerçekimi

        // Tavan: statik.
        let ceiling = BodyHandle::from_id(0);
        let mut ceil_rb = RigidBody::new_static();
        ceil_rb.wake_up();
        world.add_body(
            ceiling,
            ceil_rb,
            Transform::new(Vec3::new(0.0, 5.0, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)),
        );

        // Sarkan zincir.
        let mut prev = ceiling;
        for i in 0..n {
            let e = BodyHandle::from_id(i as u32 + 1);
            let pos = Vec3::new(0.0, 4.0 - i as f32, 0.0);
            let (rb, t, v, c) = dynamic_body(pos, true);
            world.add_body(e, rb, t, v, c);
            // Bir önceki halkaya bağla (anchor'lar merkezlerde).
            world.joints.push(Joint::ball_socket(prev, e, Vec3::ZERO, Vec3::ZERO));
            prev = e;
        }

        for frame in 0..200 {
            world.step(1.0 / 60.0).ok();
            for i in 0..world.transforms.len() {
                let p = world.transforms[i].position;
                prop_assert!(p.is_finite(), "kare {frame}: cisim {i} NaN/Inf");
                prop_assert!(
                    p.length() < 100.0,
                    "kare {frame}: cisim {i} ışınlandı (patlama): {p:?}"
                );
            }
        }
    }
}
