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

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Kare başına heap tahsisi sayacı.
///
/// Narrowphase'in çift başına `Vec<ContactPoint>` döndürdüğü görüldü; bunun ölçülen
/// farkın ne kadarını açıkladığı **tahmin edilmemeli**. Aynı süreçte, aynı tahsisatçıyla,
/// iki motor için de sayılır.
struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Backtrace yakalamak kendisi tahsis yapar; bu bayrak olmadan tahsisatçı kendini
    /// çağırır ve yığın taşar.
    static IN_PROBE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Örnekleme aralığı — her N'inci tahsiste bir çağrı yığını alınır. Hepsini almak
/// koşuyu dakikalarca sürdürür ve dağılımı değiştirmez.
static SAMPLE: AtomicUsize = AtomicUsize::new(0);
static PROFILING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SITES: std::sync::Mutex<Option<std::collections::HashMap<String, usize>>> =
    std::sync::Mutex::new(None);

fn record_site() {
    IN_PROBE.with(|g| {
        if g.get() {
            return;
        }
        g.set(true);
        let mut name = String::from("?");
        let mut depth = 0usize;
        backtrace::trace(|frame| {
            depth += 1;
            // İlk kareler tahsisatçının kendisi; motorun içine inen ilk kareyi al.
            if depth < 4 {
                return true;
            }
            let mut found = false;
            backtrace::resolve_frame(frame, |sym| {
                if let Some(n) = sym.name() {
                    let n = format!("{n}");
                    if n.contains("gizmo") || n.contains("rapier") || n.contains("parry") {
                        name = n;
                        found = true;
                    }
                }
            });
            !found && depth < 24
        });
        if let Ok(mut m) = SITES.lock() {
            if let Some(map) = m.as_mut() {
                *map.entry(name).or_insert(0) += 1;
            }
        }
        g.set(false);
    });
}

unsafe impl std::alloc::GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: std::alloc::Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        if PROFILING.load(Ordering::Relaxed)
            && SAMPLE.fetch_add(1, Ordering::Relaxed).is_multiple_of(64)
        {
            record_site();
        }
        unsafe { std::alloc::System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(p, l) }
    }
}

#[global_allocator]
static A: Counting = Counting;

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
fn throughput(n_side: usize, sphere: bool, roll: f32, frames: usize) {
    let n = n_side * n_side * n_side;
    println!(
        "\n── 3. Hız: {n} {}, {frames} kare{} ──",
        if sphere { "küre" } else { "kutu" },
        if roll > 0.0 { format!(" · yuvarlanma direnci {roll}") } else { String::new() }
    );
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
                // Yuvarlanma direnci modeli yok; sorunun sönümlemeyle çözülüp çözülmediğini
                // ölçmeden tam bir model yazmak, bugün beş kez düştüğüm tuzağa altıncı kez
                // düşmek olurdu.
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
                    {
                        let m = PhysicsMaterial { rolling_friction: roll, ..plain() };
                        if sphere {
                            GCollider::sphere(half).with_material(m)
                        } else {
                            GCollider::box_collider(GVec3::new(half, half, half)).with_material(m)
                        }
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
    // Temas sayısı da toplanır: "tahsis ≈ çift sayısı" bir ARİTMETİK çıkarımdı, ölçüm
    // değil. Tahsisler çiftle mi yoksa cisimle mi ölçekleniyor — ikisi bambaşka adres.
    let mut g_contacts = 0u64;
    // **Kaç iterasyon çalıştığı, iterasyonun kaça mal olduğundan önce gelir.** "Substep başına
    // 2,5×" iki zıt şeyle açıklanabilir — aynı sayıda pahalı iterasyon, ya da çok sayıda ucuz
    // iterasyon — ve ikisi zıt düzeltme ister. `solver_sweeps` bunu doğrudan sayar; Rapier'ın
    // karşılığı `num_solver_iterations` (varsayılan 4, alt adım yok).
    let mut g_sweeps = 0u64;
    // Alt-fazlar da kare kare toplanır. Son kareyi okumak kutu sahnesinde sıfır veriyordu:
    // yığın o noktada uyumuş, hiç ada çözülmemiş. Aynı hatayı faz dökümünde bir kez yapmıştım.
    let mut sub = [0.0f64; 6];
    // **Uyanıkken ve uyurken ayrı ayrı.** Her iki motor da bu sahneyi ~75. karede uyutuyor, yani
    // 300 karelik ortalamanın dörtte üçü "uyuyan sahne" maliyetidir. Çözücü hakkında bir şey
    // söyleyen tek pencere, cisimlerin hâlâ hareket ettiği penceredir — ve iki taraf için de aynı
    // şekilde ayrılmazsa kıyas, kimin daha erken uyuduğunu ölçer.
    let (mut g_awake_ms, mut g_awake_frames) = (0.0f64, 0u32);
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let t0 = Instant::now();
    for _ in 0..frames {
        let before = Instant::now();
        let _ = w.step(DT);
        let took = before.elapsed().as_secs_f64() * 1000.0;
        // **Fazlar da yalnız uyanık karelerden.** Kayıtlı faz dökümü 300 kare üzerinden alınmıştı
        // ve dörtte üçü uykuydu; uyuyan bir karede çözücü hiç çalışmaz, dolayısıyla o ortalama
        // çözücünün payını olduğundan küçük, geniş fazınkini büyük gösterir. Aynı hata bir kez
        // "süpürme karenin %86'sı değil" sonucunu üretmişti.
        let awake = (0..n).filter(|&i| !w.rigid_bodies[i].is_sleeping).count() * 100 >= n;
        if awake {
            g_awake_ms += took;
            g_awake_frames += 1;
            g_bp += w.metrics.broadphase_ms as f64;
            g_np += w.metrics.narrowphase_ms as f64;
            g_sv += w.metrics.solver_ms as f64;
            g_it += w.metrics.integration_ms as f64;
        }
        g_contacts += w.metrics.contact_count as u64;
        g_sweeps += w.metrics.solver_sweeps as u64;
        if !awake {
            continue;
        }
        for (acc, v) in sub.iter_mut().zip([
            w.metrics.solver_order_ms,
            w.metrics.solver_prepare_ms,
            w.metrics.solver_sweep_ms,
            w.metrics.solver_relax_ms,
            w.metrics.narrowphase_dispatch_ms,
            w.metrics.narrowphase_manifold_ms,
        ]) {
            *acc += v as f64;
        }
    }
    let g_ms = t0.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    let g_alloc = (ALLOCS.load(Ordering::Relaxed) - a0) / frames;
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
    // **Uyumuyorlar mı, yoksa hâlâ hareket mi ediyorlar?** İkisi bambaşka: eşiğin
    // altındayken uyutulmayan cisim uyku mantığının sorunudur, eşiğin üstünde olan ise
    // fiziğin. Uyku eşikleri 0,05 m/s ve 0,05 rad/s.
    let mut lin: Vec<f32> = (0..n).map(|i| w.velocities[i].linear.length()).collect();
    let mut ang: Vec<f32> = (0..n).map(|i| w.velocities[i].angular.length()).collect();
    lin.sort_by(f32::total_cmp);
    ang.sort_by(f32::total_cmp);
    let g_lin_med = lin[n / 2];
    let g_ang_med = ang[n / 2];
    let g_under = (0..n)
        .filter(|&i| lin[i] < 0.05 && ang[i] < 0.05)
        .count();
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
    let (mut r_awake_ms, mut r_awake_frames) = (0.0f64, 0u32);
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let t0 = Instant::now();
    for _ in 0..frames {
        let before = Instant::now();
        rp.step();
        let took = before.elapsed().as_secs_f64() * 1000.0;
        if rp.bodies.iter().filter(|(_, b)| b.is_dynamic() && !b.is_sleeping()).count() * 100
            >= n
        {
            r_awake_ms += took;
            r_awake_frames += 1;
            r_bp += rp.pipeline.counters.cd.broad_phase_time.time_ms();
            r_np += rp.pipeline.counters.cd.narrow_phase_time.time_ms();
            r_sv += rp.pipeline.counters.stages.solver_time.time_ms();
            r_it += rp.pipeline.counters.stages.update_time.time_ms();
        }
    }
    let r_ms = t0.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    let r_alloc = (ALLOCS.load(Ordering::Relaxed) - a0) / frames;
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

    println!(
        "   gizmo : {g_ms:>7.3} ms/kare (4 alt-adım, uyanıkken {ga:>6.3} × {gf} kare) · uyanık {g_awake:>4}/{n} · ort y {g_mean_y:>5.2} · \
         yayılma {g_spread:>5.1} m · en derin girişim {g_overlap:>5.3}\n            \
         tahsis/kare {g_alloc} · temas/kare {c} · tahsis/temas {r:>5.2}\n            \
         hız: medyan doğrusal {g_lin_med:>6.3} m/s · medyan açısal {g_ang_med:>6.3} rad/s · \
         eşik altında {g_under}/{n}\n            \
         süpürme/kare {sw} (rapier 4) · süpürme başına {per:>6.4} ms",
        ga = g_awake_ms / g_awake_frames.max(1) as f64,
        gf = g_awake_frames,
        c = g_contacts / frames as u64,
        sw = g_sweeps / frames as u64,
        per = g_ms / (g_sweeps as f64 / frames as f64).max(1.0),
        r = g_alloc as f64 / (g_contacts as f64 / frames as f64).max(1.0)
    );
    println!("   rapier: {r_ms:>7.3} ms/kare (1 adım, uyanıkken {ra:>6.3} × {rf} kare)      · uyanık {r_awake:>4}/{n} · ort y {r_mean_y:>5.2} · yayılma {r_spread:>5.1} m · en derin girişim {r_overlap:>5.3} · tahsis/kare {r_alloc}",
        ra = r_awake_ms / r_awake_frames.max(1) as f64, rf = r_awake_frames);
    println!("   oran  : gizmo {:.2}× {}", (g_ms / r_ms).max(r_ms / g_ms), if g_ms > r_ms { "daha yavaş" } else { "daha hızlı" });

    // ── Faz dökümü ───────────────────────────────────────────────────────────
    // "Yavaşız" bir adres değil. Fark tek bir fazdaysa iş bellidir; her faza yayılmışsa
    // konu mimaridir. İki motor da kendi zamanlayıcısını taşıyor, o yüzden bu tahmin
    // değil ölçüm. Gizmo'nunki son KAREnin dört alt-adımının toplamı, Rapier'ınki son
    // adımın; ikisi de "bir 1/60 karesi" demek.
    // Bölenler kare sayısı değil UYANIK kare sayısı, ve iki taraf için ayrı: motorlar farklı
    // karelerde uyuyor (bu sahnede rapier 116, gizmo 75), yani ortak bir bölen ikisinden birini
    // yanlış ölçekler.
    let gf64 = g_awake_frames.max(1) as f64;
    let rf64 = r_awake_frames.max(1) as f64;
    println!("   ── faz dökümü (UYANIK kare başına ortalama, ms) ──");
    println!(
        "   {:<12} {:>9} {:>9}",
        "", "gizmo", "rapier"
    );
    println!(
        "   {:<12} {:>9.3} {:>9.3}",
        "broadphase", g_bp / gf64, r_bp / rf64
    );
    println!(
        "   {:<12} {:>9.3} {:>9.3}",
        "narrowphase", g_np / gf64, r_np / rf64
    );
    println!(
        "   {:<12} {:>9.3} {:>9.3}",
        "çözücü", g_sv / gf64, r_sv / rf64
    );
    println!(
        "   {:<12} {:>9.3} {:>9.3}",
        "entegrasyon", g_it / frames as f64, r_it / frames as f64
    );
    // Alt-faz dökümü: dört faz "hangi çeyrek" der, bunlar "ne" der. Son karenin değerleri
    // — sahne o noktada oturmuş olabilir, o yüzden yanında uyanık sayısı okunmalı.
    // **Adalar paralel çözülüyor**, yani ada başına ölçülen süreler toplanınca duvar
    // saatini aşabilir — bunlar CPU-zamanı payları, kare süresinin dilimleri değil. Oran
    // olarak okunmalı, mutlak olarak değil.
    let f = gf64;
    let solver_cpu: f64 = sub[0] + sub[1] + sub[2] + sub[3];
    println!(
        "   ── alt-faz (kare başına, CPU-ms; adalar paralel olduğundan toplam > duvar saati) ──"
    );
    println!(
        "   çözücü {:>6.3}: sıralama {:>5.3} ({:>2.0}%) · hazırlık {:>5.3} ({:>2.0}%) · \
         süpürme {:>5.3} ({:>2.0}%) · relax {:>5.3} ({:>2.0}%)",
        solver_cpu / f,
        sub[0] / f, 100.0 * sub[0] / solver_cpu.max(1e-9),
        sub[1] / f, 100.0 * sub[1] / solver_cpu.max(1e-9),
        sub[2] / f, 100.0 * sub[2] / solver_cpu.max(1e-9),
        sub[3] / f, 100.0 * sub[3] / solver_cpu.max(1e-9)
    );
    let np_cpu = sub[4] + sub[5];
    println!(
        "   narrowphase {:>6.3}: çarpışma matematiği {:>5.3} ({:>2.0}%) · \
         manifold/önbellek/olay {:>5.3} ({:>2.0}%)",
        np_cpu / f,
        sub[4] / f, 100.0 * sub[4] / np_cpu.max(1e-9),
        sub[5] / f, 100.0 * sub[5] / np_cpu.max(1e-9)
    );
    println!(
        "   toplam: gizmo {:>6.3}  rapier {:>6.3}  (ölçülen kare {:>6.3} / {:>6.3})",
        (g_bp + g_np + g_sv + g_it) / gf64,
        (r_bp + r_np + r_sv + r_it) / rf64,
        g_awake_ms / gf64,
        r_awake_ms / rf64
    );
}

/// **Bir tahsis kaça mal oluyor?**
///
/// Tahsis sayısını düşürmek üç commit boyunca kare süresini kıpırdatmadı. Sayı ile süre
/// arasındaki dönüşümü ölçmeden devam etmek, ölçebildiğimi optimize edip önemli olanı
/// ıskalamak olur. Sahnedeki tahsisler küçük ve kısa ömürlü (temas vektörleri), o yüzden
/// ölçü de öyle.
fn allocation_cost() {
    const N: usize = 200_000;
    let t0 = Instant::now();
    let mut sink = 0usize;
    for i in 0..N {
        let v: Vec<u32> = Vec::with_capacity(4 + (i & 3));
        sink += v.capacity();
        drop(v);
    }
    let ns = t0.elapsed().as_secs_f64() * 1e9 / N as f64;
    println!("\n── Tahsis maliyeti ──");
    println!("   küçük Vec tahsis+bırak: {ns:.1} ns  (sink {sink})");
    for (label, per_frame) in [("gizmo küre", 22833.0f64), ("gizmo kutu", 4700.0), ("rapier", 54.0)] {
        println!("   {label:<12} {per_frame:>7.0} tahsis/kare → {:.3} ms/kare", per_frame * ns / 1e6);
    }
}

/// **Zaman profili, motoru değiştirmeden: ablasyon.**
///
/// `perf` yok ve motorun içine yeni zamanlayıcı gömmek hem semver hem determinizm
/// sorusudur. Ama çözücünün kolları zaten dışarıdan açılıp kapanıyor — her birini kapatıp
/// süreyi ölçmek zamanı özelliklere doğrudan atfeder. Fark, o kolun maliyetidir.
///
/// Not: kapatılan kol yalnız hız değil DAVRANIŞ da değiştirir (yığın kararlılığı bunlara
/// bağlı), yani bu bir "kapatalım gitsin" listesi değil, nereye bakılacağının haritası.
fn ablation() {
    const N: usize = 10;
    let cases: [(&str, fn(&mut PhysicsWorld)); 6] = [
        ("hepsi açık (varsayılan)", |_w| {}),
        ("support_ordering kapalı", |w| w.solver.support_ordering = false),
        ("block_solver kapalı", |w| w.solver.block_solver = false),
        ("adaptive_iterations kapalı", |w| w.solver.adaptive_iterations = false),
        ("iterations 20 → 8", |w| w.solver.iterations = 8),
        ("use_tgs_soft kapalı", |w| w.solver.use_tgs_soft = false),
    ];
    println!("\n── Ablasyon: {} kutu, 300 kare ──", N * N * N);
    for (label, apply) in cases {
        let mut w = PhysicsWorld::new();
        w.integrator.gravity = GVec3::new(0.0, -9.81, 0.0);
        apply(&mut w);
        let (half, gap) = (0.4f32, 1.2f32);
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
        for x in 0..N {
            for y in 0..N {
                for z in 0..N {
                    let mut rb = GRigidBody::new(1.0, true);
                    rb.wake_up();
                    w.add_body(
                        BodyHandle::from_id(id),
                        rb,
                        GTransform::new(GVec3::new(
                            (x as f32 - N as f32 * 0.5) * gap,
                            1.0 + y as f32 * gap,
                            (z as f32 - N as f32 * 0.5) * gap,
                        )),
                        GVelocity::default(),
                        GCollider::box_collider(GVec3::new(half, half, half)).with_material(plain()),
                    );
                    id += 1;
                }
            }
        }
        let t0 = Instant::now();
        for _ in 0..300 {
            let _ = w.step(DT);
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / 300.0;
        let n = N * N * N;
        let mean_y: f32 = (0..n).map(|i| w.transforms[i].position.y).sum::<f32>() / n as f32;
        let awake = (0..n).filter(|&i| !w.rigid_bodies[i].is_sleeping).count();
        println!("   {label:<28} {ms:>6.3} ms/kare · uyanık {awake:>4}/{n} · ort y {mean_y:>5.2}");
    }
}

/// What the iteration budget actually buys.
///
/// The ablation next door says cutting `iterations` 20 → 8 makes the frame *slower*, and reads
/// that as "the count is load-bearing". It is one point, and one point cannot tell a floor from a
/// slope. Rapier settles the same pile with **4 solver iterations per island per frame** while we
/// run 4 substeps × 20 = **80**, so the question is not whether 8 is worse than 20 but where the
/// curve actually turns. (The substep multiplier is the other half of that 80 and cannot be swept
/// from here — `PHYSICS_HZ` is a private constant with no knob on the world.)
///
/// Settling time is reported alongside the frame cost because it is the mechanism the ablation
/// invoked without measuring: a lower count is supposed to cost more by settling later. If a
/// setting settles at the same frame and costs less, that explanation does not hold there.
fn iteration_curve() {
    const N: usize = 10;
    let n = N * N * N;
    println!("\n── İterasyon eğrisi: {n} kutu, 300 kare ──");
    println!("   (rapier: ada başına kare başına 4 iterasyon, alt adım yok)");
    // **İki geçiş, çünkü ilki kendi kendini sabote etti.** `adaptive_iterations` derin adalarda
    // taban sayıyı `max(28, 1.5·D)` ile eziyor: 10 kutu yüksekliğindeki yığın, `iterations` 2'ye
    // çekilse bile 28 süpürme alıyor. İlk geçişte eğri bu yüzden düz göründü, ve düzlüğü ele veren
    // şey ms değil süpürme sütunu oldu — 20 → 2 sayıyı ancak yarıya indirdi.
    for (iters, adaptive) in [32usize, 20, 8, 4, 2, 1]
        .into_iter()
        .flat_map(|i| [(i, true), (i, false)])
    {
        let mut w = PhysicsWorld::new();
        w.integrator.gravity = GVec3::new(0.0, -9.81, 0.0);
        w.solver.iterations = iters;
        w.solver.adaptive_iterations = adaptive;
        let (half, gap) = (0.4f32, 1.2f32);
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
        for x in 0..N {
            for y in 0..N {
                for z in 0..N {
                    let mut rb = GRigidBody::new(1.0, true);
                    rb.wake_up();
                    w.add_body(
                        BodyHandle::from_id(id),
                        rb,
                        GTransform::new(GVec3::new(
                            (x as f32 - N as f32 * 0.5) * gap,
                            1.0 + y as f32 * gap,
                            (z as f32 - N as f32 * 0.5) * gap,
                        )),
                        GVelocity::default(),
                        GCollider::box_collider(GVec3::new(half, half, half)).with_material(plain()),
                    );
                    id += 1;
                }
            }
        }
        let mut settled_at = None;
        let mut sweeps = 0u64;
        // **Uyanık kareleri ayrı tut.** Sahne 75. kare civarında uyuyor, yani 300 karelik bir
        // ortalamanın dörtte üçü uyuyan bir sahnenin maliyetidir ve çözücüyle hiç ilgisi yoktur.
        // Bir kez öyle okundu ve "taban maliyet %86" gibi bir sonuç verdi; gerçekte ölçülen şey
        // uykuydu. İterasyon bütçesi hakkında bir şey söyleyebilecek tek pencere, cisimlerin
        // hâlâ hareket ettiği penceredir.
        let mut awake_ms = 0.0f64;
        let mut awake_frames = 0u32;
        let t0 = Instant::now();
        for f in 0..300 {
            let before = Instant::now();
            let _ = w.step(DT);
            let took = before.elapsed().as_secs_f64() * 1000.0;
            sweeps += w.metrics.solver_sweeps as u64;
            let up = (0..n).filter(|&i| !w.rigid_bodies[i].is_sleeping).count();
            if up * 100 >= n {
                awake_ms += took;
                awake_frames += 1;
            }
            if settled_at.is_none() && up * 100 < n {
                settled_at = Some(f);
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0 / 300.0;
        let ams = awake_ms / awake_frames.max(1) as f64;
        let mean_y: f32 = (0..n).map(|i| w.transforms[i].position.y).sum::<f32>() / n as f32;
        let awake = (0..n).filter(|&i| !w.rigid_bodies[i].is_sleeping).count();
        let deepest = (0..n)
            .map(|i| w.transforms[i].position.y)
            .fold(f32::MAX, f32::min);
        println!(
            "   {iters:>2} iterasyon {a}  {ms:>6.3} ms/kare (uyanıkken {ams:>6.3}) · \
             uyanık {awake:>4}/{n} · ort y {mean_y:>5.2} · en alt {deepest:>5.2} · \
             %99 uyudu {} · süpürme/kare {}",
            settled_at.map_or("hiç".to_string(), |f| format!("{f:>3}. kare")),
            sweeps / 300,
            a = if adaptive { "adaptif" } else { "sabit  " },
        );
    }
}

fn main() {
    if std::env::var("ITERCURVE").is_ok() {
        iteration_curve();
        return;
    }
    if std::env::var("ITER").is_ok() {
        iteration_budget();
        return;
    }
    if std::env::var("ABLATION").is_ok() {
        ablation();
        return;
    }
    if std::env::var("ALLOCCOST").is_ok() {
        allocation_cost();
        return;
    }
    if std::env::var("PROFILE").is_ok() {
        allocation_sites();
        return;
    }
    println!("Gizmo (yerel) ↔ Rapier3D 0.35 — aynı sahneler, aynı dt=1/60");
    accuracy_elastic();
    stability_tower();
    println!("\n── Tahsis: temassız taban çizgisi ──");
    for n in [64usize, 216, 512, 1000] {
        empty_baseline(n);
    }
    // Ölçek testi: tahsis çiftle mi cisimle mi büyüyor?
    for side in [4usize, 6, 8, 10] {
        throughput(side, false, 0.0, 300);
    }
    for r in [0.0f32, 0.05] {
        throughput(10, true, r, 300);
    }
}

/// **Temassız taban çizgisi.** Yığındaki tahsislerin ne kadarı temasla ilgili, ne kadarı
/// her alt-adımda dönen makine? Cisimler yerçekimsiz boşlukta, birbirinden uzak: broadphase
/// yine koşar, narrowphase yine çağrılır, ama tek bir temas üretilmez. Aradaki fark adresi
/// verir — biri narrowphase işi, öteki boru hattı işi.
fn empty_baseline(n: usize) {
    let mut w = PhysicsWorld::new();
    w.integrator.gravity = GVec3::ZERO;
    let side = (n as f32).cbrt().ceil() as usize;
    let mut id = 0u32;
    for x in 0..side {
        for y in 0..side {
            for z in 0..side {
                if id as usize >= n {
                    break;
                }
                let mut rb = GRigidBody::new(1.0, false);
                rb.wake_up();
                w.add_body(
                    BodyHandle::from_id(id),
                    rb,
                    // 20 m aralık: hiçbir çift temas etmez, hatta broadphase hücresini
                    // bile paylaşmaz.
                    GTransform::new(GVec3::new(
                        x as f32 * 20.0,
                        y as f32 * 20.0,
                        z as f32 * 20.0,
                    )),
                    GVelocity::new(GVec3::new(0.001, 0.0, 0.0)),
                    GCollider::box_collider(GVec3::new(0.4, 0.4, 0.4)),
                );
                id += 1;
            }
        }
    }
    let a0 = ALLOCS.load(Ordering::Relaxed);
    let mut contacts = 0u64;
    for _ in 0..120 {
        let _ = w.step(DT);
        contacts += w.metrics.contact_count as u64;
    }
    println!(
        "   temassız {n:>5} cisim → tahsis/kare {:>6} · temas/kare {}",
        (ALLOCS.load(Ordering::Relaxed) - a0) / 120,
        contacts / 120
    );
}

/// Uyanık sahnede tahsislerin ÇAĞRI YERİNE göre dağılımı.
///
/// Sayaç "ne kadar" diyordu, bu "nereden" diyor. Aritmetikle bölerek adres üretmek bu
/// soruşturmada tekrar tekrar yanlış çıktı; burada her 64'üncü tahsisin yığını alınıp
/// motorun içine inen ilk kareye göre gruplanıyor.
fn allocation_sites() {
    const N: usize = 10;
    println!("\n── Tahsis profili: {} küre, uyanık ──", N * N * N);
    let mut w = PhysicsWorld::new();
    w.integrator.gravity = GVec3::new(0.0, -9.81, 0.0);
    let (half, gap) = (0.4f32, 1.2f32);
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
    for x in 0..N {
        for y in 0..N {
            for z in 0..N {
                let mut rb = GRigidBody::new(1.0, true);
                rb.wake_up();
                w.add_body(
                    BodyHandle::from_id(id),
                    rb,
                    GTransform::new(GVec3::new(
                        (x as f32 - N as f32 * 0.5) * gap,
                        1.0 + y as f32 * gap,
                        (z as f32 - N as f32 * 0.5) * gap,
                    )),
                    GVelocity::default(),
                    GCollider::sphere(half).with_material(plain()),
                );
                id += 1;
            }
        }
    }
    // Sahne yerleşsin; profil, yığın hâlâ hareket ederken alınmalı.
    for _ in 0..120 {
        let _ = w.step(DT);
    }
    *SITES.lock().unwrap() = Some(std::collections::HashMap::new());
    PROFILING.store(true, Ordering::Relaxed);
    let a0 = ALLOCS.load(Ordering::Relaxed);
    for _ in 0..60 {
        let _ = w.step(DT);
    }
    let total = ALLOCS.load(Ordering::Relaxed) - a0;
    PROFILING.store(false, Ordering::Relaxed);
    let map = SITES.lock().unwrap().take().unwrap_or_default();
    let mut v: Vec<_> = map.into_iter().collect();
    v.sort_by_key(|(_, c)| std::cmp::Reverse(*c));
    let sampled: usize = v.iter().map(|(_, c)| *c).sum();
    println!("   {} tahsis / 60 kare · örneklenen {sampled}", total);
    for (name, count) in v.into_iter().take(12) {
        let parts: Vec<&str> = name.split("::").collect();
        let short = parts[parts.len().saturating_sub(3)..].join("::");
        println!("   {:>5.1}%  {}", 100.0 * count as f64 / sampled.max(1) as f64, short);
    }
}

// ── ITER=1: how much constraint iteration each engine actually runs ──────────────────────
//
// The recorded conclusion is that the ~2.5× per-substep gap is a convergence-per-iteration
// difference. That is an inference, and it has a rival that wants the opposite fix: the same
// iteration count at 2.5× the cost each. Counting what both engines actually run separates them.
fn iteration_budget() {
    let p = rapier3d::dynamics::IntegrationParameters::default();
    println!("rapier defaults: num_solver_iterations {:?}, internal pgs {:?}, \
              internal stabilization {:?}",
        p.num_solver_iterations, p.num_internal_pgs_iterations,
        p.num_internal_stabilization_iterations);
}
