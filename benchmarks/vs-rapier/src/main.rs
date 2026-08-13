//! Gizmo'nun sert-cisim fiziğini Rapier3D ile **aynı sahnelerde** karşılaştırır.
//!
//! Üç eksen, çünkü tek sayı yanıltır: bir motor hızlı olup yığını devirebilir, bir
//! başkası kararlı olup yavaş olabilir, üçüncüsü ikisini de yapıp çarpışmayı yanlış
//! çözebilir.
//!
//! **Adalet notu, sonuçtan önce söylenmeli:** Gizmo `step(1/60)` çağrısını içeride
//! 1/240'lık dört alt-adıma böler; Rapier varsayılanında tek adım atar. Yani "kare
//! başına maliyet" karşılaştırması kullanıcı seviyesinde dürüsttür (bir oyun kare
//! başına ne ödüyor), ama "adım başına iş" olarak okunmamalı — Gizmo dört katı iş
//! yapıyor. Sayılar bu notla birlikte anlamlı.

use std::time::Instant;

use gizmo_math::Vec3 as GVec3;
use gizmo_physics_core::{
    components::{CombineMode, PhysicsMaterial},
    BodyHandle, Collider as GCollider, Transform as GTransform,
};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody as GRigidBody, Velocity as GVelocity};

use rapier3d::prelude::*;

const DT: f32 = 1.0 / 60.0;

/// Hız sahneleri için **iki motorda da aynı** malzeme.
///
/// Varsayılanlara güvenmek kıyası bozuyordu: Gizmo restitution 0,3 ile geliyor, Rapier
/// 0,0 ile. Yani aynı sahnede bizim küreler zıplayıp hiç uyumuyor, onlarınki oturuyordu
/// — ölçülen fark motorların hızı değil, iki farklı sahne oluyordu. Sürtünme 0,5,
/// restitution 0,0: ikisi de açıkça yazılı.
fn plain() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 0.0,
        static_friction: 0.5,
        dynamic_friction: 0.5,
        ..Default::default()
    }
}

fn elastic() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 1.0,
        static_friction: 0.0,
        dynamic_friction: 0.0,
        restitution_combine: CombineMode::Max,
        ..Default::default()
    }
}

/// Rapier'ın adım atması için gereken bütün durum, tek yerde.
struct Rap {
    pipeline: PhysicsPipeline,
    gravity: Vector,
    params: IntegrationParameters,
    islands: IslandManager,
    broad: DefaultBroadPhase,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
}

impl Rap {
    fn new(gravity_y: f32) -> Self {
        let mut me = Self {
            pipeline: PhysicsPipeline::new(),
            gravity: Vector::new(0.0, gravity_y, 0.0),
            params: IntegrationParameters { dt: DT, ..Default::default() },
            islands: IslandManager::new(),
            broad: DefaultBroadPhase::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
        };
        // Faz zamanlayıcıları varsayılanda kapalı; kıyasın tamamı buna bağlı.
        me.pipeline.counters.enable();
        me
    }

    fn step(&mut self) {
        self.pipeline.step(
            self.gravity,
            &self.params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd,
            &(),
            &(),
        );
    }
}

// ─────────────────────────────────────────────────────── 1. doğruluk: elastik çarpışma
//
// Eşit kütle, restitution 1, kafa kafaya. Analitik cevap belli: vuran DURUR, vurulan
// hızın tamamını alır. Bu, motorun "doğru" ya da "yanlış" olduğu ender ölçümlerden
// biri — kıyas değil, sınav.
fn accuracy_elastic() {
    println!("\n── 1. Elastik çarpışma (eşit kütle, e=1, kafa kafaya) ──");
    println!("   analitik cevap: vuran 0.000 m/s, vurulan 5.000 m/s");
    let v = 5.0f32;
    let r = 0.5f32;

    // Gizmo
    let mut w = PhysicsWorld::new();
    w.integrator.gravity = GVec3::ZERO;
    for (id, x, vx) in [(0u32, -2.0f32, v), (1, 0.0, 0.0)] {
        let mut rb = GRigidBody::new(1.0, false);
        rb.wake_up();
        w.add_body(
            BodyHandle::from_id(id),
            rb,
            GTransform::new(GVec3::new(x, 0.0, 0.0)),
            GVelocity::new(GVec3::new(vx, 0.0, 0.0)),
            GCollider::sphere(r).with_material(elastic()),
        );
    }
    for _ in 0..60 {
        let _ = w.step(DT);
    }
    println!(
        "   gizmo : vuran {:>7.3}  vurulan {:>7.3}",
        w.velocities[0].linear.x, w.velocities[1].linear.x
    );

    // Rapier
    let mut rp = Rap::new(0.0);
    let mut hs = Vec::new();
    for (x, vx) in [(-2.0f32, v), (0.0, 0.0)] {
        let h = rp.bodies.insert(
            RigidBodyBuilder::dynamic().translation(Vector::new(x, 0.0, 0.0)).linvel(Vector::new(vx, 0.0, 0.0)).build(),
        );
        let c = ColliderBuilder::ball(r).restitution(1.0).friction(0.0).build();
        rp.colliders.insert_with_parent(c, h, &mut rp.bodies);
        hs.push(h);
    }
    for _ in 0..60 {
        rp.step();
    }
    println!(
        "   rapier: vuran {:>7.3}  vurulan {:>7.3}",
        rp.bodies[hs[0]].linvel().x,
        rp.bodies[hs[1]].linvel().x
    );
}

// ─────────────────────────────────────────────────────── 2. kararlılık: kule
//
// Yirmi kutuluk kule, 600 adım. Ölçülen: en üstteki kutu ne kadar yana kaydı. Devrilen
// kule büyük bir sayı verir, duran kule küçük.
fn stability_tower() {
    println!("\n── 2. Kararlılık: 20 kutuluk kule, 600 adım ──");
    let n = 20;
    let half = 0.5f32;

    // Gizmo
    let mut w = PhysicsWorld::new();
    w.integrator.gravity = GVec3::new(0.0, -9.81, 0.0);
    let mut ground = GRigidBody::new_static();
    ground.wake_up();
    w.add_body(
        BodyHandle::from_id(999),
        ground,
        GTransform::new(GVec3::new(0.0, -1.0, 0.0)),
        GVelocity::default(),
        GCollider::box_collider(GVec3::new(50.0, 1.0, 50.0)),
    );
    for i in 0..n {
        let mut rb = GRigidBody::new(1.0, true);
        rb.wake_up();
        w.add_body(
            BodyHandle::from_id(i as u32),
            rb,
            GTransform::new(GVec3::new(0.0, half + i as f32 * 2.0 * half, 0.0)),
            GVelocity::default(),
            GCollider::box_collider(GVec3::new(half, half, half)),
        );
    }
    let top_start = w.transforms[n].position;
    for _ in 0..600 {
        let _ = w.step(DT);
    }
    let g_drift = {
        let p = w.transforms[n].position;
        ((p.x - top_start.x).powi(2) + (p.z - top_start.z).powi(2)).sqrt()
    };
    let g_fell = w.transforms[n].position.y < top_start.y - half;

    // Rapier
    let mut rp = Rap::new(-9.81);
    let g = rp.bodies.insert(RigidBodyBuilder::fixed().translation(Vector::new(0.0, -1.0, 0.0)).build());
    let gc = ColliderBuilder::cuboid(50.0, 1.0, 50.0).build();
    rp.colliders.insert_with_parent(gc, g, &mut rp.bodies);
    let mut hs = Vec::new();
    for i in 0..n {
        let h = rp.bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(Vector::new(0.0, half + i as f32 * 2.0 * half, 0.0))
                .build(),
        );
        let c = ColliderBuilder::cuboid(half, half, half).build();
        rp.colliders.insert_with_parent(c, h, &mut rp.bodies);
        hs.push(h);
    }
    let r_start = rp.bodies[hs[n - 1]].translation();
    for _ in 0..600 {
        rp.step();
    }
    let r_end = rp.bodies[hs[n - 1]].translation();
    let r_drift = ((r_end.x - r_start.x).powi(2) + (r_end.z - r_start.z).powi(2)).sqrt();
    let r_fell = r_end.y < r_start.y - half;

    println!("   gizmo : en üst kutu yana {g_drift:>6.3} m kaydı{}", if g_fell { ", ve düştü" } else { "" });
    println!("   rapier: en üst kutu yana {r_drift:>6.3} m kaydı{}", if r_fell { ", ve düştü" } else { "" });
}

// ─────────────────────────────────────────────────────── 3. hız: yığın
//
// N kutu ızgaraya dizilip düşürülür. Ölçülen: kare başına milisaniye. Yukarıdaki adalet
// notu burada geçerli — Gizmo çağrı başına dört alt-adım atıyor.
fn throughput(n_side: usize, sphere: bool) {
    let n = n_side * n_side * n_side;
    println!("\n── 3. Hız: {n} {}, 300 kare ──", if sphere { "küre" } else { "kutu" });
    let half = 0.4f32;
    let gap = 1.2f32;

    let mut w = PhysicsWorld::new();
    w.integrator.gravity = GVec3::new(0.0, -9.81, 0.0);
    let mut ground = GRigidBody::new_static();
    ground.wake_up();
    w.add_body(
        BodyHandle::from_id(999_999),
        ground,
        GTransform::new(GVec3::new(0.0, -1.0, 0.0)),
        GVelocity::default(),
        GCollider::box_collider(GVec3::new(200.0, 1.0, 200.0)).with_material(plain()),
    );
    let mut id = 0u32;
    for x in 0..n_side {
        for y in 0..n_side {
            for z in 0..n_side {
                let mut rb = GRigidBody::new(1.0, true);
                rb.wake_up();
                w.add_body(
                    BodyHandle::from_id(id),
                    rb,
                    GTransform::new(GVec3::new(
                        (x as f32 - n_side as f32 * 0.5) * gap,
                        1.0 + y as f32 * gap,
                        (z as f32 - n_side as f32 * 0.5) * gap,
                    )),
                    GVelocity::default(),
                    if sphere {
                        GCollider::sphere(half).with_material(plain())
                    } else {
                        GCollider::box_collider(GVec3::new(half, half, half))
                            .with_material(plain())
                    },
                );
                id += 1;
            }
        }
    }
    // **Faz zamanları KARE KARE toplanır, son kareden okunmaz.** Sahne 300 karede uyuyor;
    // son kareyi ölçmek "her şey sıfır" der ve bu, işin bittiğini değil ölçünün yanlış
    // yerde durduğunu gösterir. Bir kez öyle ölçüldü ve sıfırlar ele verdi.
    let (mut g_bp, mut g_np, mut g_sv, mut g_it) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let t0 = Instant::now();
    for _ in 0..300 {
        let _ = w.step(DT);
        g_bp += w.metrics.broadphase_ms as f64;
        g_np += w.metrics.narrowphase_ms as f64;
        g_sv += w.metrics.solver_ms as f64;
        g_it += w.metrics.integration_ms as f64;
    }
    let g_ms = t0.elapsed().as_secs_f64() * 1000.0 / 300.0;
    // **Uyku, kıyasın en sinsi çarpıtması.** Uyuyan cisim çözücüye uğramaz; bir motor
    // sahneyi erken uyutuyorsa "hızlı" görünür, oysa daha az iş yapmıştır. İkisinin de
    // kaç cismi uyanık bitirdiği yazılmadan ms/kare okunmamalı. Yığının ortalama
    // yüksekliği de yanında: sahneler gerçekten aynı şeye yerleştiyse yakın çıkmalı.
    let g_awake = (0..n).filter(|&i| !w.rigid_bodies[i].is_sleeping).count();
    let g_mean_y: f32 = (0..n).map(|i| w.transforms[i].position.y).sum::<f32>() / n as f32;
    // **Dağılma mı, iç içe geçme mi.** Yığının yayıldığı yarıçap ile en derin girişim
    // birlikte okunmalı: yuvarlanıp dağılan küreler geniş ama temiz bir yığın verir,
    // birbirinin içine giren küreler ise dar ve çakışık. Tek başına "ortalama yükseklik
    // düştü" ikisini ayırmaz.
    let g_spread = (0..n)
        .map(|i| (w.transforms[i].position.x.powi(2) + w.transforms[i].position.z.powi(2)).sqrt())
        .fold(0.0f32, f32::max);
    let mut g_overlap = 0.0f32;
    for i in 0..n.min(300) {
        for j in (i + 1)..n.min(300) {
            let d = (w.transforms[i].position - w.transforms[j].position).length();
            g_overlap = g_overlap.max((2.0 * half - d).max(0.0));
        }
    }

    let mut rp = Rap::new(-9.81);
    let g = rp.bodies.insert(RigidBodyBuilder::fixed().translation(Vector::new(0.0, -1.0, 0.0)).build());
    let gc = ColliderBuilder::cuboid(200.0, 1.0, 200.0).friction(0.5).restitution(0.0).build();
    rp.colliders.insert_with_parent(gc, g, &mut rp.bodies);
    for x in 0..n_side {
        for y in 0..n_side {
            for z in 0..n_side {
                let h = rp.bodies.insert(
                    RigidBodyBuilder::dynamic()
                        .translation(Vector::new(
                            (x as f32 - n_side as f32 * 0.5) * gap,
                            1.0 + y as f32 * gap,
                            (z as f32 - n_side as f32 * 0.5) * gap,
                        ))
                        .build(),
                );
                let c = if sphere {
                    ColliderBuilder::ball(half).friction(0.5).restitution(0.0).build()
                } else {
                    ColliderBuilder::cuboid(half, half, half).friction(0.5).restitution(0.0).build()
                };
                rp.colliders.insert_with_parent(c, h, &mut rp.bodies);
            }
        }
    }
    let (mut r_bp, mut r_np, mut r_sv, mut r_it) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let t0 = Instant::now();
    for _ in 0..300 {
        rp.step();
        r_bp += rp.pipeline.counters.cd.broad_phase_time.time_ms();
        r_np += rp.pipeline.counters.cd.narrow_phase_time.time_ms();
        r_sv += rp.pipeline.counters.stages.solver_time.time_ms();
        r_it += rp.pipeline.counters.stages.update_time.time_ms();
    }
    let r_ms = t0.elapsed().as_secs_f64() * 1000.0 / 300.0;
    let r_awake = rp.bodies.iter().filter(|(_, b)| b.is_dynamic() && !b.is_sleeping()).count();
    let r_mean_y: f32 = rp.bodies.iter().filter(|(_, b)| b.is_dynamic())
        .map(|(_, b)| b.translation().y).sum::<f32>() / n as f32;
    let rpos: Vec<_> = rp.bodies.iter().filter(|(_, b)| b.is_dynamic())
        .map(|(_, b)| b.translation()).collect();
    let r_spread = rpos.iter().map(|p| (p.x * p.x + p.z * p.z).sqrt()).fold(0.0f32, f32::max);
    let mut r_overlap = 0.0f32;
    for i in 0..rpos.len().min(300) {
        for j in (i + 1)..rpos.len().min(300) {
            let d = (rpos[i] - rpos[j]).length();
            r_overlap = r_overlap.max((2.0 * half - d).max(0.0));
        }
    }

    println!("   gizmo : {g_ms:>7.3} ms/kare (4 alt-adım) · uyanık {g_awake:>4}/{n} · ort y {g_mean_y:>5.2} · yayılma {g_spread:>5.1} m · en derin girişim {g_overlap:>5.3}");
    println!("   rapier: {r_ms:>7.3} ms/kare (1 adım)      · uyanık {r_awake:>4}/{n} · ort y {r_mean_y:>5.2} · yayılma {r_spread:>5.1} m · en derin girişim {r_overlap:>5.3}");
    println!("   oran  : gizmo {:.2}× {}", (g_ms / r_ms).max(r_ms / g_ms), if g_ms > r_ms { "daha yavaş" } else { "daha hızlı" });

    // ── Faz dökümü ───────────────────────────────────────────────────────────
    // "Yavaşız" bir adres değil. Fark tek bir fazdaysa iş bellidir; her faza yayılmışsa
    // konu mimaridir. İki motor da kendi zamanlayıcısını taşıyor, o yüzden bu tahmin
    // değil ölçüm. Gizmo'nunki son KAREnin dört alt-adımının toplamı, Rapier'ınki son
    // adımın; ikisi de "bir 1/60 karesi" demek.
    println!("   ── faz dökümü (kare başına ortalama, ms) ──");
    println!(
        "   {:<12} {:>9} {:>9}",
        "", "gizmo", "rapier"
    );
    println!(
        "   {:<12} {:>9.3} {:>9.3}",
        "broadphase", g_bp / 300.0, r_bp / 300.0
    );
    println!(
        "   {:<12} {:>9.3} {:>9.3}",
        "narrowphase", g_np / 300.0, r_np / 300.0
    );
    println!(
        "   {:<12} {:>9.3} {:>9.3}",
        "çözücü", g_sv / 300.0, r_sv / 300.0
    );
    println!(
        "   {:<12} {:>9.3} {:>9.3}",
        "entegrasyon", g_it / 300.0, r_it / 300.0
    );
    println!(
        "   toplam: gizmo {:>6.3}  rapier {:>6.3}  (ölçülen kare {:>6.3} / {:>6.3})",
        (g_bp + g_np + g_sv + g_it) / 300.0,
        (r_bp + r_np + r_sv + r_it) / 300.0,
        g_ms,
        r_ms
    );
}

fn main() {
    println!("Gizmo (yerel) ↔ Rapier3D 0.35 — aynı sahneler, aynı dt=1/60");
    accuracy_elastic();
    stability_tower();
    throughput(10, false);
    throughput(10, true);
}
