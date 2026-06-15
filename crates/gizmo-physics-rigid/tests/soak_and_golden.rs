//! Soak (uzun-süre kararlılık) + golden (referans senaryo) regresyon testleri.
//!
//! Faz 1 — Property testler RASTGELE girdiyi tarar; bu testler ise SABİT, fiziksel
//! olarak anlamlı iki senaryonun uzun-vadeli davranışını sabitler:
//!   * SOAK   — N-kutu yığını 10 saniye boyunca kararlı kalmalı: enerji patlaması
//!     yok, yanal sürüklenme yok, tünelleme/iç-içe-geçme yok, NaN yok.
//!   * GOLDEN — bilinen bir senaryonun (zeminde dengelenen kutu) yerleşme değerleri
//!     referans aralıkta kalmalı. Toleranslar platformlar-arası f32 sapmasını
//!     soğurur (cross-platform bit-exact GARANTİ EDİLMEZ — bkz. docs/determinism.md),
//!     ama davranış-bozucu bir regresyonu yakalar.

use gizmo_core::entity::Entity;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn add_ground(world: &mut PhysicsWorld) {
    let mut ground = RigidBody::new_static();
    ground.friction = 0.8;
    ground.wake_up();
    world.add_body(
        Entity::new(0, 0),
        ground,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)), // üst yüzey y = 0
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
    );
}

fn add_box(world: &mut PhysicsWorld, id: u32, pos: Vec3, half: f32) {
    let mut rb = RigidBody::new(1.0, 0.0, 0.6, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(half));
    rb.update_inertia_from_collider(&col);
    world.add_body(Entity::new(id, 0), rb, Transform::new(pos), Velocity::default(), col);
}

#[test]
fn soak_box_stack_stays_stable() {
    let mut world = PhysicsWorld::new();
    world.solver.iterations = 16;
    add_ground(&mut world);

    // 3 kutu, yarı-kenar 0.5, başlangıçta TAM TEMASLA (dürtüsüz) dik yığın.
    let n = 3;
    let half = 0.5;
    for i in 0..n {
        let y = half + i as f32 * (2.0 * half); // 0.5, 1.5, 2.5
        add_box(&mut world, i as u32 + 1, Vec3::new(0.0, y, 0.0), half);
    }

    // 10 saniye simüle et.
    for _ in 0..600 {
        world.step(1.0 / 60.0).ok();
    }

    // 1) Hiçbir cisim NaN/Inf değil.
    for i in 0..world.transforms.len() {
        assert!(
            world.transforms[i].position.is_finite() && world.velocities[i].linear.is_finite(),
            "cisim {i} NaN/Inf"
        );
    }

    // 2) Yerleştikten sonra kalıntı hız düşük (patlama/jitter yok). NOT:
    //    calculate_total_energy potansiyel enerjiyi de içerir, bu yüzden yerleşme
    //    ölçütü için doğrudan hızlara bakıyoruz.
    let max_speed = (1..=n)
        .map(|i| world.velocities[i].linear.length() + world.velocities[i].angular.length())
        .fold(0.0f32, f32::max);
    assert!(max_speed.is_finite(), "hız NaN/Inf");
    assert!(max_speed < 0.5, "yığın yerleşmedi / jitter yüksek: max_speed={max_speed}");

    // 3) Kutular yanal sürüklenmedi ve sırası korundu (tünelleme/çökme yok).
    let mut prev_y = -1.0f32;
    for i in 1..=n {
        let p = world.transforms[i].position;
        assert!(
            p.x.abs() < 1.0 && p.z.abs() < 1.0,
            "kutu {i} yanal sürüklendi: {p:?}"
        );
        // En alttaki kutu zemine oturmalı; her kutu altındakinden yukarıda olmalı.
        assert!(
            p.y > prev_y + 0.3,
            "kutu {i} yığın sırasını bozdu (iç-içe geçti?): y={} prev={prev_y}",
            p.y
        );
        assert!(p.y > 0.0, "kutu {i} zeminin altına düştü: y={}", p.y);
        prev_y = p.y;
    }
}

#[test]
fn golden_box_settles_on_ground() {
    let mut world = PhysicsWorld::new();
    world.solver.iterations = 16;
    add_ground(&mut world);

    // Tek kutu (yarı-kenar 0.5) y=5'ten düşer.
    let half = 0.5;
    add_box(&mut world, 1, Vec3::new(0.0, 5.0, 0.0), half);

    for _ in 0..300 {
        world.step(1.0 / 60.0).ok();
    }

    let p = world.transforms[1].position;
    let v = world.velocities[1].linear;

    // GOLDEN referans aralıkları (platformlar-arası f32 sapması için gevşek):
    // Kutu zemine (üst yüzey y=0) oturur → merkez y ≈ yarı-kenar = 0.5.
    assert!(
        (p.y - half).abs() < 0.08,
        "kutu beklenen yükseklikte yerleşmedi: y={} (beklenen ≈ {half})",
        p.y
    );
    // Düşeyde düştü, yana kaymadı.
    assert!(p.x.abs() < 0.05 && p.z.abs() < 0.05, "kutu yana kaydı: {p:?}");
    // Dinlenmede (uyumuş ya da neredeyse durgun).
    assert!(v.length() < 0.1, "kutu dinlenmedi: |v|={}", v.length());
}
