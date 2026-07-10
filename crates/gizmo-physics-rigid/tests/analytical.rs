//! ANALİTİK-ORACLE FİZİK DOĞRULAMA MERDİVENİ — en basitten en karmaşığa.
//!
//! Her test KAPALI-FORM (elle çözülebilir) bir senaryoyu ECS yolundan
//! (`physics_step_system`) sürer ve sonucu analitik değere göre sınar. Amaç:
//! bir regresyon "görsel olarak garip" değil, "sayı analitikten kaydı" biçiminde
//! ortaya çıksın. `collider_material` bug'ı (restitution sessizce 0.3'e düşüyordu)
//! aylarca gizlenebildi çünkü tek belirtisi "sekiş zayıf görünüyor"du — Tier 1'deki
//! `restitution_controls_bounce_height` gibi bir test onu ANINDA yakalar.
//!
//! Merdiven:
//!   Tier 0 — integratör (temassız): serbest düşüş, atalet, sönüm, dönme
//!   Tier 1 — tek temas: dinlenme, restitution→sekiş yüksekliği, sürtünme
//!   Tier 2 — iki-cisim çarpışma: elastik (eşit/eşitsiz kütle), esnek-olmayan
//!   Tier 3 — kısıt/eklem: sarkaç periyodu (2π√(L/g)), hinge uzunluk-koruması
//!   Tier 4 — invaryant: sürtünmesiz zincirde momentum korunumu

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;
use gizmo_physics_rigid::Joint;

const DT: f32 = 1.0 / 240.0;
const G: f32 = 9.81;

// ── küçük yardımcılar ────────────────────────────────────────────────────────

fn scene(gravity: Vec3) -> World {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(gravity));
    w
}

fn spawn(world: &mut World, t: Transform, rb: RigidBody, v: Velocity, c: Collider) -> u32 {
    let e = world.spawn();
    world.add_component(e, t);
    world.add_component(e, rb);
    world.add_component(e, v);
    world.add_component(e, c);
    e.id()
}

fn floor(world: &mut World, top_y: f32) {
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(0.0, top_y - 0.5, 0.0)));
    world.add_component(e, RigidBody::new_static());
    world.add_component(e, Velocity::default());
    world.add_component(e, Collider::box_collider(Vec3::new(50.0, 0.5, 50.0)));
}

fn step(world: &World, n: usize) {
    for _ in 0..n {
        physics_step_system(world, DT);
    }
}

fn pos(world: &World, id: u32) -> Vec3 {
    world.borrow::<Transform>().get(id).unwrap().position
}
fn vel(world: &World, id: u32) -> Vec3 {
    world.borrow::<Velocity>().get(id).map(|v| v.linear).unwrap_or(Vec3::ZERO)
}
fn angvel(world: &World, id: u32) -> Vec3 {
    world.borrow::<Velocity>().get(id).map(|v| v.angular).unwrap_or(Vec3::ZERO)
}

/// restitution=e, sürtünmesiz (temas testleri için); combine=Max.
fn bouncy(e: f32) -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: e,
        static_friction: 0.0,
        dynamic_friction: 0.0,
        restitution_combine: CombineMode::Max,
        ..Default::default()
    }
}
/// e=0 (esnek-olmayan); restitution_combine=Min ki birleşik 0 kalsın.
fn sticky() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 0.0,
        static_friction: 0.0,
        dynamic_friction: 0.0,
        restitution_combine: CombineMode::Min,
        ..Default::default()
    }
}
/// yüksek sürtünme, sekişsiz.
fn rough() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 0.0,
        static_friction: 0.9,
        dynamic_friction: 0.8,
        friction_combine: CombineMode::Max,
        restitution_combine: CombineMode::Min,
        ..Default::default()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TIER 0 — İntegratör (temas yok, eklem yok): neredeyse tam olmalı.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn t0_free_fall_matches_kinematics() {
    let mut w = scene(Vec3::new(0.0, -G, 0.0));
    let mut rb = RigidBody::new(1.0, true);
    rb.linear_damping = 0.0; // saf integratörü izole et
    let id = spawn(
        &mut w,
        Transform::new(Vec3::new(0.0, 100.0, 0.0)),
        rb,
        Velocity::default(),
        Collider::sphere(0.5),
    );
    step(&w, 240); // 1.0 s
    let y = pos(&w, id).y;
    let vy = vel(&w, id).y;
    // Sürekli çözüm: v=-g·t=-9.81, y=100-½g·t²=95.095. Yarı-örtük Euler küçük ofset.
    assert!((vy + G).abs() < 0.05, "vy={vy}, beklenen ≈ -9.81");
    assert!((y - 95.095).abs() < 0.2, "y={y}, beklenen ≈ 95.1");
}

#[test]
fn t0_no_force_keeps_velocity() {
    let mut w = scene(Vec3::ZERO);
    let mut rb = RigidBody::new(1.0, false);
    rb.linear_damping = 0.0;
    let id = spawn(
        &mut w,
        Transform::new(Vec3::ZERO),
        rb,
        Velocity::new(Vec3::new(3.0, 0.0, 0.0)),
        Collider::sphere(0.5),
    );
    step(&w, 240);
    let v = vel(&w, id);
    let p = pos(&w, id);
    // Kuvvet yok → hız sabit, enerji sızıntısı olmamalı.
    assert!((v.x - 3.0).abs() < 1e-3, "vx={} sabit kalmalı", v.x);
    assert!((p.x - 3.0).abs() < 0.02, "x={} ≈ v·t = 3.0 olmalı", p.x);
}

#[test]
fn t0_default_damping_decays_velocity() {
    // Varsayılan linear_damping=0.01 HAFİF yavaşlatır — bu motor tasarımı, hata değil.
    // Test bunu BELGELER ki Tier-0'daki tam-korunum testleriyle çelişki sürpriz olmasın.
    let mut w = scene(Vec3::ZERO);
    let rb = RigidBody::new(1.0, false); // varsayılan sönüm
    let id = spawn(
        &mut w,
        Transform::new(Vec3::ZERO),
        rb,
        Velocity::new(Vec3::new(5.0, 0.0, 0.0)),
        Collider::sphere(0.5),
    );
    step(&w, 240);
    let vx = vel(&w, id).x;
    assert!(vx < 5.0 && vx > 4.0, "vx={vx}: hafif sönüm beklenir (0<Δ<%20)");
}

#[test]
fn t0_free_spin_conserves_angular_velocity() {
    let mut w = scene(Vec3::ZERO);
    let mut rb = RigidBody::new(1.0, false);
    rb.angular_damping = 0.0;
    rb.calculate_sphere_inertia(0.5); // izotropik → tumbling yok
    let v = Velocity::new(Vec3::ZERO).with_angular(Vec3::new(0.0, 0.0, 2.0));
    let id = spawn(&mut w, Transform::new(Vec3::ZERO), rb, v, Collider::sphere(0.5));
    step(&w, 240);
    let wz = angvel(&w, id).z;
    assert!((wz - 2.0).abs() < 0.02, "ωz={wz}: tork yok → korunmalı");
}

// ═══════════════════════════════════════════════════════════════════════════
// TIER 1 — Tek temas: dinlenme, restitution, sürtünme.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn t1_sphere_rests_on_floor() {
    let mut w = scene(Vec3::new(0.0, -G, 0.0));
    floor(&mut w, 0.0);
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_sphere_inertia(0.5);
    let id = spawn(
        &mut w,
        Transform::new(Vec3::new(0.0, 0.6, 0.0)),
        rb,
        Velocity::default(),
        Collider::sphere(0.5).with_material(rough()),
    );
    step(&w, 400);
    let y = pos(&w, id).y;
    let vy = vel(&w, id).y;
    // r=0.5 küre, zemin üstü y=0 → merkez y≈0.5'te dinlenmeli (batmamış/fırlamamış).
    assert!((y - 0.5).abs() < 0.06, "y={y}: küre y≈0.5 dinlenmeli");
    assert!(vy.abs() < 0.1, "vy={vy}: dinlenmede ≈0");
}

#[test]
fn t1_restitution_controls_bounce_height() {
    // Aynı yükseklikten iki küre; e=0.9 belirgin biçimde e=0.2'den yükseğe zıplamalı.
    // Bu, malzeme-kaybı bug'ının yüksekliğe yansıyan doğrudan koruması: restitution
    // yok sayılsaydı iki apex de aynı olurdu.
    let apex = |e: f32| -> f32 {
        let mut w = scene(Vec3::new(0.0, -G, 0.0));
        floor(&mut w, 0.0);
        let mut rb = RigidBody::new(1.0, true);
        rb.calculate_sphere_inertia(0.5);
        let id = spawn(
            &mut w,
            Transform::new(Vec3::new(0.0, 5.0, 0.0)),
            rb,
            Velocity::default(),
            Collider::sphere(0.5).with_material(bouncy(e)),
        );
        let mut touched = false;
        let mut top = 0.0_f32;
        for _ in 0..1600 {
            physics_step_system(&w, DT);
            let y = pos(&w, id).y;
            if !touched && y < 0.85 {
                touched = true; // zemine değdi
            }
            if touched {
                top = top.max(y); // ilk sekiş en yüksek — sonrakiler daha alçak
            }
        }
        top
    };
    let high = apex(0.9);
    let low = apex(0.2);
    assert!(
        high > low + 1.0,
        "e=0.9 apex={high:.2} > e=0.2 apex={low:.2} + 1.0 olmalı — restitution solver'a ulaşıyor mu?"
    );
    assert!(high > 2.0, "e=0.9 apex={high:.2}: anlamlı elastik sekiş beklenir");
}

#[test]
fn t1_friction_stops_a_sliding_box() {
    let mut w = scene(Vec3::new(0.0, -G, 0.0));
    floor(&mut w, 0.0);
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_box_inertia(1.0, 1.0, 1.0);
    let id = spawn(
        &mut w,
        Transform::new(Vec3::new(0.0, 0.5, 0.0)),
        rb,
        Velocity::new(Vec3::new(5.0, 0.0, 0.0)),
        Collider::box_collider(Vec3::new(0.5, 0.5, 0.5)).with_material(rough()),
    );
    let x0 = pos(&w, id).x;
    step(&w, 600); // 2.5 s
    let vx = vel(&w, id).x;
    let dx = pos(&w, id).x - x0;
    // Sürtünme kutuyu durdurmalı; yoksa 5 m/s ile sonsuza kayardı.
    assert!(vx.abs() < 0.5, "vx={vx}: sürtünme durdurmalı");
    assert!(dx < 4.0 && dx > 0.0, "dx={dx}: sürtünmeyle sınırlı mesafe (~v²/2μg)");
}

// ═══════════════════════════════════════════════════════════════════════════
// TIER 2 — İki-cisim çarpışma: momentum/enerji kapalı-formu.
// ═══════════════════════════════════════════════════════════════════════════

fn ball_at(world: &mut World, x: f32, vx: f32, mass: f32, mat: PhysicsMaterial) -> u32 {
    let mut rb = RigidBody::new(mass, false);
    rb.calculate_sphere_inertia(0.5);
    spawn(
        world,
        Transform::new(Vec3::new(x, 0.0, 0.0)),
        rb,
        Velocity::new(Vec3::new(vx, 0.0, 0.0)),
        Collider::sphere(0.5).with_material(mat),
    )
}

#[test]
fn t2_elastic_unequal_mass_transfers_momentum() {
    // Yerçekimsiz saf 1-B: hafif A (m=1, +5) ağır B'ye (m=3, dur) çarpar.
    // Efektif e≈0.7 ile A geri teper (va<0), B ileri gider, momentum korunur.
    let mut w = scene(Vec3::ZERO);
    let a = ball_at(&mut w, -1.02, 5.0, 1.0, bouncy(1.0));
    let b = ball_at(&mut w, 0.0, 0.0, 3.0, bouncy(1.0));
    step(&w, 120);
    let va = vel(&w, a).x;
    let vb = vel(&w, b).x;
    assert!(vb > 1.5, "vb={vb}: ağır top ileri itilmeli");
    assert!(va < -0.3, "va={va}: hafif top ağırdan geri sekmeli");
    let p = 1.0 * va + 3.0 * vb;
    assert!((p - 5.0).abs() < 0.6, "Σp={p} ≈ 5.0 korunmalı");
}

#[test]
fn t2_inelastic_bodies_move_together() {
    let mut w = scene(Vec3::ZERO);
    let a = ball_at(&mut w, -1.02, 4.0, 1.0, sticky());
    let b = ball_at(&mut w, 0.0, 0.0, 1.0, sticky());
    step(&w, 120);
    let va = vel(&w, a).x;
    let vb = vel(&w, b).x;
    // e=0 → beraber hareket (~2.0 her ikisi), momentum korunur.
    assert!((va - vb).abs() < 0.7, "va={va} vb={vb}: e=0'da beraber gitmeli");
    assert!((va + vb - 4.0).abs() < 0.6, "Σp={} ≈ 4.0", va + vb);
}

// ═══════════════════════════════════════════════════════════════════════════
// TIER 3 — Kısıt/eklem: sarkaç periyodu, hinge uzunluk-koruması.
// ═══════════════════════════════════════════════════════════════════════════

/// Tek hinge sarkacı kur: (dünya, top_id, pivot). Kiriş sabit çapa, top L uzakta.
fn build_pendulum(l: f32, angle_deg: f32) -> (World, u32, Vec3) {
    let py = 6.0;
    let pivot = Vec3::new(0.0, py, 0.0);
    let mut w = World::new();

    let beam = w.spawn();
    w.add_component(beam, Transform::new(pivot));
    w.add_component(beam, RigidBody::new_static());
    w.add_component(beam, Velocity::default());
    w.add_component(beam, Collider::box_collider(Vec3::new(0.2, 0.05, 0.05)));

    let a0 = angle_deg.to_radians();
    let center = pivot + l * Vec3::new(-a0.sin(), -a0.cos(), 0.0);
    let mut rb = RigidBody::new(1.0, true);
    rb.calculate_sphere_inertia(0.3);
    let ball = w.spawn();
    w.add_component(ball, Transform::new(center).with_rotation(Quat::from_rotation_z(-a0)));
    w.add_component(ball, rb);
    w.add_component(ball, Velocity::default());
    w.add_component(ball, Collider::sphere(0.3));

    let mut phys = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -G, 0.0));
    phys.joints.push(Joint::hinge(
        BodyHandle::from_id(beam.id()),
        BodyHandle::from_id(ball.id()),
        Vec3::ZERO,
        Vec3::new(0.0, l, 0.0),
        Vec3::Z,
    ));
    w.insert_resource(phys);
    (w, ball.id(), pivot)
}

#[test]
fn t3_pendulum_period_matches_small_angle_formula() {
    const L: f32 = 2.0;
    let (w, ball, pivot) = build_pendulum(L, 8.0); // küçük açı
    // x-offset işaret değişimlerinden yarım-periyot ölç.
    let mut prev = pos(&w, ball).x - pivot.x;
    let mut crossings: Vec<f32> = Vec::new();
    for i in 0..2400 {
        physics_step_system(&w, DT);
        let x = pos(&w, ball).x - pivot.x;
        if prev.signum() != x.signum() && prev.abs() > 1e-4 {
            crossings.push(i as f32 * DT);
        }
        prev = x;
    }
    assert!(crossings.len() >= 4, "birkaç salınım beklenir, crossings={}", crossings.len());
    let mut half = 0.0;
    for k in 1..crossings.len() {
        half += crossings[k] - crossings[k - 1];
    }
    half /= (crossings.len() - 1) as f32;
    let period = 2.0 * half;
    let expected = std::f32::consts::TAU * (L / G).sqrt(); // 2π√(L/g) ≈ 2.837 s
    assert!(
        (period - expected).abs() / expected < 0.15,
        "T={period:.3} beklenen 2π√(L/g)={expected:.3} — %15 içinde"
    );
}

#[test]
fn t3_hinge_holds_length() {
    const L: f32 = 2.0;
    let (w, ball, pivot) = build_pendulum(L, 30.0); // daha geniş salınım
    let mut max_dev = 0.0_f32;
    for _ in 0..1200 {
        physics_step_system(&w, DT);
        let d = (pos(&w, ball) - pivot).length();
        max_dev = max_dev.max((d - L).abs());
    }
    assert!(max_dev < 0.1, "hinge uzunluğu L={L}'den max {max_dev:.3} saptı (kısıt sürükleniyor)");
}

// ═══════════════════════════════════════════════════════════════════════════
// TIER 4 — İnvaryant: momentum korunumu (restitution kusurlu olsa bile).
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn t4_frictionless_chain_conserves_momentum() {
    // Yerçekimsiz, eklemsiz 3-top zinciri: soldakine vur. Yalnız iç temas
    // kuvvetleri var → toplam momentum bir INVARYANT (restitution kalitesinden bağımsız).
    let mut w = scene(Vec3::ZERO);
    let r = 0.5;
    let sp = 2.0 * r + 0.01;
    let mut ids = Vec::new();
    for i in 0..3 {
        let vx = if i == 0 { 4.0 } else { 0.0 };
        ids.push(ball_at(&mut w, i as f32 * sp, vx, 1.0, bouncy(1.0)));
    }
    step(&w, 240);
    let pf: f32 = ids.iter().map(|&id| vel(&w, id).x).sum();
    assert!((pf - 4.0).abs() < 0.3, "Σp={pf} ≈ 4.0 (momentum korunmalı)");
}
