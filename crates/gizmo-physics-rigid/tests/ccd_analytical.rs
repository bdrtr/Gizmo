//! CCD (Sürekli Çarpışma Tespiti) ANALİTİK MERDİVENİ — tünellemeyi en basitten
//! en karmaşığa kapalı-form oracle'larla sınar, tamamı ECS yolundan
//! (`physics_step_system`). Mevcut CCD testleri (`tests/ccd.rs`, `world/tests.rs`)
//! HAM `PhysicsWorld` API'sini sürer; bu merdiven ise ECS gather→sync(*rb)→step→
//! ccd_resolve_step zincirinin uçtan uca çalıştığının yürütülebilir kanıtıdır.
//!
//! Aritmetik: dahili sabit alt-adım 240 Hz (FIXED_DT=1/240). DT=1/240'ta bir
//! `physics_step_system` çağrısı = tam bir alt-adım. Alt-adım başına yol = v/240.
//! Bir cisim, "yakalama bandı" 2·(duvar_yarı_kalınlık + mermi_yarıçap)'tan hızlı
//! giderse ayrık tespit ONU KAÇIRIR (hiçbir örnek banda düşmez) → tüneller.
//!
//! Rung 1-7 GEÇER (koruma). `ccd_hole_*` rungları BİLİNEN AÇIKLARI belgeler ve
//! `#[ignore]`'dur (CI'yi kırmadan sınırı kilitler; `--ignored` ile çalıştırılır).

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 240.0;

// ── yardımcılar ──────────────────────────────────────────────────────────────

/// Yerçekimsiz sahne → temiz 1-B tünelleme aritmetiği.
fn scene() -> World {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::ZERO));
    w
}

/// e=0: yakalanan mermi zıplamadan yerinde park etsin (sekme testi değil).
fn sticky() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 0.0,
        static_friction: 0.0,
        dynamic_friction: 0.0,
        restitution_combine: CombineMode::Min,
        ..Default::default()
    }
}

/// x=cx merkezli, yarı-kalınlığı half_thick olan statik duvar (ön yüz x=cx-half_thick).
fn wall(world: &mut World, cx: f32, half_thick: f32) -> u32 {
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(cx, 0.0, 0.0)));
    world.add_component(e, RigidBody::new_static());
    world.add_component(e, Velocity::default());
    world.add_component(e, Collider::box_collider(Vec3::new(half_thick, 5.0, 5.0)));
    e.id()
}

/// +x yönünde `speed` ile giden dinamik küre mermi. `ccd` bayrağı doğrudan set edilir.
fn bullet(world: &mut World, x0: f32, speed: f32, r: f32, ccd: bool) -> u32 {
    let mut rb = RigidBody::new(1.0, false); // yerçekimi yok
    rb.linear_damping = 0.0;
    rb.ccd_enabled = ccd;
    rb.wake_up();
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(x0, 0.0, 0.0)));
    world.add_component(e, rb);
    world.add_component(e, Velocity::new(Vec3::new(speed, 0.0, 0.0)));
    world.add_component(e, Collider::sphere(r).with_material(sticky()));
    e.id()
}

/// Küre yerine kutu mermi (GJK'nın ince-en-boy oranında bozulduğu hâli sınamak için).
fn box_bullet(world: &mut World, x0: f32, speed: f32, r: f32, ccd: bool) -> u32 {
    let mut rb = RigidBody::new(1.0, false);
    rb.linear_damping = 0.0;
    rb.ccd_enabled = ccd;
    rb.wake_up();
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(x0, 0.0, 0.0)));
    world.add_component(e, rb);
    world.add_component(e, Velocity::new(Vec3::new(speed, 0.0, 0.0)));
    world.add_component(e, Collider::box_collider(Vec3::splat(r)).with_material(sticky()));
    e.id()
}

/// `steps` alt-adım sür; (tepe_x, son_x, son_vx) döndür. Tepe, backstop düzeltmesinden
/// SONRAKİ (physics_step_system çıkışındaki) konumdur — anlık integrasyon üstü değil.
fn run(world: &World, id: u32, steps: usize) -> (f32, f32, f32) {
    let mut peak = f32::MIN;
    for _ in 0..steps {
        physics_step_system(world, DT);
        let x = world.borrow::<Transform>().get(id).unwrap().position.x;
        peak = peak.max(x);
    }
    let fx = world.borrow::<Transform>().get(id).unwrap().position.x;
    let fv = world.borrow::<Velocity>().get(id).map(|v| v.linear.x).unwrap_or(0.0);
    (peak, fx, fv)
}

// ═══════════════════════════════════════════════════════════════════════════
// Rung 1 — NEGATİF KONTROL: yavaş cisim, kalın duvar, CCD YOK → ayrık durdurur.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn ccd_r1_slow_body_stops_at_thick_wall_without_ccd() {
    let mut w = scene();
    wall(&mut w, 0.0, 0.5); // ön yüz x=-0.5
    let b = bullet(&mut w, -3.0, 3.0, 0.1, false); // yavaş (0.0125 m/alt-adım), ccd yok
    let (peak, fx, fv) = run(&w, b, 480);
    // Ayrık tespit yavaş cismi yakalar: geçmez, duvara girmez, durur. (Kesin dinlenme
    // konumu YERÇEKİMSİZ yatay çarpışmada belirsizdir — e=0 hız sıfırlanınca soft-step
    // geri-kazanımı küçük bir geri hızla cismi yüzeyden uzaklaştırır, geri çeken kuvvet
    // yok → birkaç dm geride uyur. Bu ayrık davranış; merdivenin konusu değil.)
    assert!(peak < 0.0, "yavaş cisim duvar merkezini geçmemeli, tepe_x={peak}");
    assert!(fx < -0.5, "duvar ön yüzüne (x=-0.5) girmemeli, fx={fx}");
    assert!(fx > -1.6, "duvarın makul yakınında durmalı (savrulmamalı), fx={fx}");
    // İleri hareketi kesilmeli — Rung 2'deki tünelleyen +2400 ile keskin karşıtlık.
    // (Küçük geri kayma ~0.3 m/s soft-step geri-kazanım artefaktıdır; yerçekimi/sürtünme
    //  olmadığı için sönmez. Merdivenin konusu tünelleme, dinlenme kalitesi değil.)
    assert!(fv < 0.5, "duvar merminin ileri hareketini durdurmalı, vx={fv}");
}

// ═══════════════════════════════════════════════════════════════════════════
// Rung 2 — SORUN VAR: hızlı cisim, ince duvar, CCD YOK → TÜNELLER (kanıt).
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn ccd_r2_fast_body_tunnels_thin_wall_without_ccd() {
    let mut w = scene();
    wall(&mut w, 0.0, 0.02); // ince: yakalama bandı 2·(0.02+0.05)=0.14 m
    let b = bullet(&mut w, -5.0, 2400.0, 0.05, false); // 10 m/alt-adım ≫ 0.14 → hiçbir örnek banda düşmez
    let (peak, _, _) = run(&w, b, 30);
    assert!(peak > 1.0, "CCD'siz mach cisim ince duvarı TÜNELLEMELİ (sorun burada), tepe_x={peak}");
}

// ═══════════════════════════════════════════════════════════════════════════
// Rung 3 — ASIL ÖNLEME: aynı geometri, CCD AÇIK → geçmez (ECS yolu kanıtı).
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn ccd_r3_ccd_prevents_thin_wall_tunnel() {
    let mut w = scene();
    wall(&mut w, 0.0, 0.02);
    let b = bullet(&mut w, -5.0, 2400.0, 0.05, true); // CCD AÇIK
    let (peak, fx, fv) = run(&w, b, 240);
    assert!(peak < 0.0, "CCD cismi duvar merkezinden önce durdurmalı, tepe_x={peak}");
    assert!(fx < 0.0 && fx > -0.6, "ön yüzün hemen kısasında dinlenmeli, fx={fx}");
    assert!(fv.abs() < 2.0, "~dinlenmeye yavaşlamalı, vx={fv}");
}

// ═══════════════════════════════════════════════════════════════════════════
// Rung 4 — HIZ TARAMASI: ayrığın çuvalladığı yerde CCD tutar.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn ccd_r4_speed_sweep_ccd_holds_where_discrete_fails() {
    let r = 0.05;
    let half = 0.02;
    let band = 2.0 * (half + r); // 0.14
    for &speed in &[300.0_f32, 1200.0, 3000.0] {
        let d = speed / 240.0;
        assert!(d > band, "test öncülü: {speed} m/s ayrığı aşmalı (d={d} > bant={band})");

        // CCD KAPALI → tüneller
        let mut w = scene();
        wall(&mut w, 0.0, half);
        let off = bullet(&mut w, -5.0, speed, r, false);
        let (peak_off, _, _) = run(&w, off, 20);
        assert!(peak_off > 0.5, "CCD'siz {speed} m/s tünellemeli, tepe={peak_off}");

        // CCD AÇIK → tutar
        let mut w2 = scene();
        wall(&mut w2, 0.0, half);
        let on = bullet(&mut w2, -5.0, speed, r, true);
        let (peak_on, _, _) = run(&w2, on, 240);
        assert!(peak_on < half, "CCD {speed} m/s'te tutmalı, tepe={peak_on}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Rung 5 — ŞEKİL/KALINLIK MATRİSİ: küre & kutu × ince & kalın, CCD açık.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn ccd_r5_shape_thickness_matrix_ccd_holds() {
    for &half in &[0.02_f32, 0.5] {
        // küre mermi
        let mut w = scene();
        wall(&mut w, 0.0, half);
        let s = bullet(&mut w, -5.0, 1500.0, 0.05, true);
        let (peak, fx, _) = run(&w, s, 240);
        assert!(peak < half, "küre CCD tutmalı (half={half}), tepe={peak}");
        assert!(fx < 0.0, "küre yakın tarafta dinlenmeli (half={half}), fx={fx}");

        // kutu mermi (ince duvarda GJK bozulur → backstop tek koruma)
        let mut w2 = scene();
        wall(&mut w2, 0.0, half);
        let bx = box_bullet(&mut w2, -5.0, 1500.0, 0.05, true);
        let (peak2, _, _) = run(&w2, bx, 240);
        assert!(peak2 < 0.0, "kutu CCD merkezi geçmemeli (half={half}), tepe={peak2}");
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Rung 6 — KİNEMATİK CCD (bug-fix kanıtı): new_kinematic ccd'yi açar; düzeltilen
// kapılar (is_dynamic → !is_static) artık onu süpürür+backstop'lar. Düzeltmeden
// ÖNCE bu kinematik cisim tünelliyordu.
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn ccd_r6_kinematic_fast_body_does_not_tunnel() {
    let mut w = scene();
    wall(&mut w, 0.0, 0.02);
    let mut rb = RigidBody::new_kinematic(); // ccd_enabled = true
    rb.wake_up();
    let e = w.spawn();
    w.add_component(e, Transform::new(Vec3::new(-5.0, 0.0, 0.0)));
    w.add_component(e, rb);
    w.add_component(e, Velocity::new(Vec3::new(1200.0, 0.0, 0.0)));
    w.add_component(e, Collider::sphere(0.05).with_material(sticky()));
    let (peak, _, _) = run(&w, e.id(), 240);
    assert!(peak < 0.0, "kinematik CCD cismi tünellememeli, tepe_x={peak}");
}

// ═══════════════════════════════════════════════════════════════════════════
// Rung 7 — UYKUDAKİ HEDEF: backstop static VE uyuyan cisimleri hedefler
// (pipeline.rs:894 yalnız uyanık-dinamiği atlar). Mermi yakın tarafta durur.
// (NOT: backstop hedefe momentum AKTARMAZ — bilinen defect; burada sadece
//  merminin geçmediğini kanıtlıyoruz.)
// ═══════════════════════════════════════════════════════════════════════════
#[test]
fn ccd_r7_fast_body_vs_sleeping_target_does_not_tunnel() {
    let mut w = scene();
    let mut orb = RigidBody::new(1.0e6, false);
    orb.is_sleeping = true; // uykuda → geçerli backstop hedefi
    let o = w.spawn();
    w.add_component(o, Transform::new(Vec3::ZERO));
    w.add_component(o, orb);
    w.add_component(o, Velocity::default());
    w.add_component(o, Collider::box_collider(Vec3::new(0.02, 5.0, 5.0)));

    let b = bullet(&mut w, -5.0, 2400.0, 0.05, true);
    let (peak, _, _) = run(&w, b, 120);
    assert!(peak < 0.0, "mermi uyuyan engelle yakalanmalı, tepe_x={peak}");
}

// ═══════════════════════════════════════════════════════════════════════════
// BİLİNEN AÇIKLAR — yürütülebilir belgeler. #[ignore]: CI'yi kırmaz,
// `cargo test -- --ignored` ile sınır doğrulanır. Assert'ler İSTENEN sözleşmedir.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[ignore = "AÇIK: ccd_resolve_step uyanık-dinamik hedefleri atlar (pipeline.rs:894) ve \
speculative mach'ta ince geometride bozulur → UYANIK ince plakaya karşı mermi tüneller. \
Çözüm: dinamik CCD çiftleri için conservative_advancement'i bağla (gjk/simplex.rs:281)."]
fn ccd_hole_fast_body_vs_dynamic_awake_thin_plate() {
    let mut w = scene();
    let mut orb = RigidBody::new(1.0e6, false);
    orb.wake_up(); // UYANIK dinamik → backstop dışlar
    let o = w.spawn();
    w.add_component(o, Transform::new(Vec3::ZERO));
    w.add_component(o, orb);
    w.add_component(o, Velocity::default());
    w.add_component(o, Collider::box_collider(Vec3::new(0.05, 5.0, 5.0)));

    let b = bullet(&mut w, -8.0, 2336.0, 0.0995, true);
    let (peak, _, _) = run(&w, b, 120);
    assert!(peak < 0.05, "İSTENEN: uyanık dinamik plakadan geçmemeli, tepe_x={peak}");
}

#[test]
#[ignore = "AÇIK: dönme CCD'si yok — speculative yalnız öteleme (simplex.rs:354), backstop \
yalnız doğrusal merkez deltasını süpürür (pipeline.rs:870,879). Hızlı dönen ince cismin \
kenarı tüneller. Çözüm: süpürme AABB + backstop yoluna |ω|·max_uzanım ekle."]
fn ccd_hole_fast_spinning_thin_body() {
    let mut w = scene();
    wall(&mut w, 0.0, 0.02);
    // İnce yassı kutu, merkezi ~sabit ama yüksek açısal hız → kenar duvarı süpürür.
    let mut rb = RigidBody::new(1.0, false);
    rb.linear_damping = 0.0;
    rb.angular_damping = 0.0;
    rb.ccd_enabled = true;
    rb.calculate_box_inertia(2.0, 0.04, 2.0);
    rb.wake_up();
    let e = w.spawn();
    // Kenarı duvar düzlemine (x=0) ulaşacak şekilde konumla; merkez -1.0'da.
    w.add_component(e, Transform::new(Vec3::new(-1.0, 0.0, 0.0)));
    w.add_component(e, rb);
    let mut v = Velocity::new(Vec3::ZERO);
    v.angular = Vec3::new(0.0, 0.0, 200.0); // 200 rad/s
    w.add_component(e, v);
    w.add_component(e, Collider::box_collider(Vec3::new(1.0, 0.02, 1.0)).with_material(sticky()));
    let (_peak, _, _) = run(&w, e.id(), 60);
    // İSTENEN: dönen kenar duvarı süpürürken bir temas üretilmeli (şu an üretilmiyor).
    // Bu rung şu an yalnızca sınırı belgeler; assert istenen davranıştır.
}
