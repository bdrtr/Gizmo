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

use gizmo_physics_core::BodyHandle;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, PhysicsMaterial, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn add_ground(world: &mut PhysicsWorld) {
    let mut ground = RigidBody::new_static();
    ground.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        ground,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)), // üst yüzey y = 0
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
    );
}

fn add_box(world: &mut PhysicsWorld, id: u32, pos: Vec3, half: f32) {
    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(half));
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(id),
        rb,
        Transform::new(pos),
        Velocity::default(),
        col,
    );
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
    assert!(
        max_speed < 0.5,
        "yığın yerleşmedi / jitter yüksek: max_speed={max_speed}"
    );

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
fn soak_falling_stack_survives_impact() {
    // Faz 4 (solver kalite turu) regresyonu: 8 kutu KÜÇÜK BOŞLUKLARLA bırakılıp
    // birbirine ÇARPARAK düşer. Mükemmel hizalı bir yığın metastable'dır; ileri-tek-
    // yönlü PGS, manifoldun 4 temas noktasını sabit sırada işleyip her çarpmada küçük
    // bir merkez-dışı (dönme) yanlılığı bırakır → yığın devrilip yanlara saçılırdı
    // (eski davranış: bu senaryoda max|xz| ~3-5). Simetrik Gauss-Seidel (solver,
    // iterasyonda yön değiştirir) bu yanlılığı iptal eder; yığın dik kalır.
    //
    // AYIRT EDİCİ: solver'da `reverse` sabit `false` yapılınca bu test DÜŞER.
    let mut world = PhysicsWorld::new();
    world.solver.iterations = 20; // varsayılan; 16'da bu yükseklik henüz yakınsamaz
    add_ground(&mut world);

    let n = 8;
    let half = 0.5;
    let gap = 0.1; // her kutu mükemmel temasın 0.1 m üstünde → düşüp çarpar
    for i in 0..n {
        let y = half + i as f32 * (2.0 * half + gap);
        add_box(&mut world, i as u32 + 1, Vec3::new(0.0, y, 0.0), half);
    }

    for _ in 0..600 {
        world.step(1.0 / 60.0).ok();
    }

    // 1) NaN/Inf yok.
    for i in 1..=n {
        assert!(
            world.transforms[i].position.is_finite() && world.velocities[i].linear.is_finite(),
            "kutu {i} NaN/Inf"
        );
    }

    // 2) Çarpma sonrası yerleşmiş (jitter/patlama yok).
    let max_speed = (1..=n)
        .map(|i| world.velocities[i].linear.length() + world.velocities[i].angular.length())
        .fold(0.0f32, f32::max);
    assert!(max_speed < 0.5, "yığın yerleşmedi: max_speed={max_speed}");

    // 3) Yığın dik kaldı: yanal sürüklenme yok ve sıra korundu (çökme/saçılma yok).
    let mut prev_y = -1.0f32;
    for i in 1..=n {
        let p = world.transforms[i].position;
        assert!(
            p.x.abs() < 0.3 && p.z.abs() < 0.3,
            "kutu {i} yanal kaydı (yığın çöktü): {p:?}"
        );
        assert!(
            p.y > prev_y + 0.3,
            "kutu {i} yığın sırasını bozdu: y={} prev={prev_y}",
            p.y
        );
        prev_y = p.y;
    }

    // 4) Tepe kutu yaklaşık beklenen yükseklikte (yığın boyu korundu).
    let expected_top = half + (n - 1) as f32 * (2.0 * half);
    assert!(
        (world.transforms[n].position.y - expected_top).abs() < 0.4,
        "tepe kutu beklenen yükseklikte değil: y={} (beklenen ≈ {expected_top})",
        world.transforms[n].position.y
    );
}

#[test]
fn soak_tall_stack_n16_stays_upright() {
    // Faz 4 KALAN regresyonu (TGS Soft hedefi): YÜKSEK (n=16) yığın KÜÇÜK BOŞLUKLARLA
    // bırakılıp birbirine ÇARPARAK düşer (yüksek-enerji çarpma). SI çözücü
    // (warm-start + simetrik GS + split-impulse, 20 iter) bu yükseklikte çarpma
    // dürtüsünü 16 cisim boyunca yayamaz; metastable yığın kaotik devrilip saçılır.
    // TGS Soft (soft constraint + relax) bunu çözer.
    //
    // AYIRT EDİCİ: mevcut SI'de bu test DÜŞER (yığın çöker / saçılır).
    let mut world = PhysicsWorld::new();
    add_ground(&mut world);

    // Restitution-0 materyal: bu test ÇÖZÜCÜNÜN dürtü-yayma kalitesini (TGS'in katkısı)
    // ölçer, sekme kaosunu değil. Sekmeyen kutular kullanmak standart yığın-testi
    // yöntemidir (yüksek-restitution 16-katlı slam her motorda kaotiktir; ayrı konu).
    let no_bounce = PhysicsMaterial {
        restitution: 0.0,
        ..Default::default()
    };
    let n = 16;
    let half = 0.5;
    let gap = 0.1; // her kutu mükemmel temasın 0.1 m üstünde → düşüp çarpar
    for i in 0..n {
        let y = half + i as f32 * (2.0 * half + gap);
        let mut rb = RigidBody::new(1.0, true);
        rb.wake_up();
        let col = Collider::box_collider(Vec3::splat(half)).with_material(no_bounce);
        rb.update_inertia_from_collider(&col);
        world.add_body(
            BodyHandle::from_id(i as u32 + 1),
            rb,
            Transform::new(Vec3::new(0.0, y, 0.0)),
            Velocity::default(),
            col,
        );
    }

    // 4 sn simüle et (yerleşmesi için).
    for _ in 0..240 {
        world.step(1.0 / 60.0).ok();
    }

    // 1) NaN/Inf yok.
    for i in 1..=n {
        assert!(
            world.transforms[i].position.is_finite() && world.velocities[i].linear.is_finite(),
            "kutu {i} NaN/Inf"
        );
    }

    // 2) Yığın DİK kaldı: yanal sürüklenme küçük + sıra korundu (çökme/saçılma yok).
    let mut prev_y = -1.0f32;
    let mut max_xz = 0.0f32;
    for i in 1..=n {
        let p = world.transforms[i].position;
        max_xz = max_xz.max(p.x.abs()).max(p.z.abs());
        assert!(
            p.y > prev_y + 0.3,
            "kutu {i} yığın sırasını bozdu (çöktü): y={} prev={prev_y}",
            p.y
        );
        prev_y = p.y;
    }
    assert!(
        max_xz < 0.5,
        "yığın çöktü / yanlara saçıldı: max|xz|={max_xz}"
    );

    // 3) Tepe kutu yaklaşık beklenen yükseklikte (yığın boyu korundu).
    let expected_top = half + (n - 1) as f32 * (2.0 * half);
    assert!(
        (world.transforms[n].position.y - expected_top).abs() < 0.6,
        "tepe kutu beklenen yükseklikte değil: y={} (beklenen ≈ {expected_top})",
        world.transforms[n].position.y
    );
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
    assert!(
        p.x.abs() < 0.05 && p.z.abs() < 0.05,
        "kutu yana kaydı: {p:?}"
    );
    // Dinlenmede (uyumuş ya da neredeyse durgun).
    assert!(v.length() < 0.1, "kutu dinlenmedi: |v|={}", v.length());
}
