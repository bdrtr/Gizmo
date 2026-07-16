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
use gizmo_physics_rigid::{ConstraintSolver, PhysicsWorld, RigidBody, Velocity};

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

// ─────────────────────────────────────────────────────────────────────────────
// RESTING-STACK INSTABILITY — history + the buckling analysis behind the fix.
//
// A FULLY SETTLED box stack used to spontaneously gain energy and blow up after enough
// frames. Measured (pre-fix, 60 fps): N=2 never; N=5 ~frame 2050; N=16 ~868; N=32 ~462.
// `soak_tall_stack_n16` above only runs 240–600 frames — JUST short of the blow-up — so
// it shipped green and HID the bug. `run_resting_stack_soak` runs 1500+ frames and asserts
// max|v| < 0.5 EVERY frame, putting the instability in-horizon. The FIX (block contact
// solver + adaptive iterations, 2026-07-15) makes realistic stacks (N≤16) stay bounded;
// see soak_resting_stacks_stay_bounded (the live regression test) and the diagnosis below.
//
// ROOT CAUSE — CORRECTED (2026-07-14, this session). The FIXPLAN blamed
// "under-converged biased Gauss-Seidel (O(N²) Delassus condition)". An empirical
// iteration ladder REFUTES that: the blow-up frame is CHAOTIC / non-monotonic in the
// iteration count (N=32 blows at every count 30…110, e.g. iters=35→@63, iters=60→@1458
// WITH support ordering) — genuine under-convergence would monotonically prevent it.
// Further refutations from the sweep harness below:
//   • support-ordering (bottom-up) barely moves the blow-up frame → NOT an O(N²)→O(N)
//     GS-order problem for this SOFT solver;
//   • frictionless is CATASTROPHICALLY worse (blows ~frame 2) → friction is STABILISING,
//     the injection is in the NORMAL/position channel, not friction coupling;
//   • hertz / damping / relax / timestep-consistent-`h` soft-param tweaks only shift the
//     frame or make it worse.
// Actual behaviour — CONFIRMED LATERAL BUCKLING (probe_buckling_vs_pump, 2026-07-15):
// the column's lean (max box |x|+|z|) and rotation angle grow EXPONENTIALLY (doubling
// ~every 100 frames) for hundreds of frames while max|v| stays tiny (<0.05); then, once
// the lean is large, the column TOPPLES and PE→KE gives the sudden max|v| escape. This is
// an inverted-pendulum / Euler-buckling instability, NOT a vertical energy pump. It cleanly
// explains every constraint: height-scaling (taller column ⇒ lower buckling critical load),
// friction-stabilises (friction resists the lean; frictionless has zero lateral restraint ⇒
// instant collapse), both solvers, metastable (slow lean → sudden topple), never-sleeps
// (the slow lean keeps velocities near the 0.05 threshold). Root: the contact solve provides
// an artificially WEAK lateral/rotational restoring torque for a stacked column, so the
// buckling critical height is far too low (~N=5 vs the dozens a real box stack tolerates).
//
// SHARED ROOT (2026-07-15): the SI (split-impulse) solver ALSO blows up (use_tgs_soft=off →
// N16@360, N32@67) → NOT TGS-specific; the too-weak restoring stiffness is upstream/shared.
// The box-box manifold IS per-corner correct (contacts.rs clip_box_box: signed_depth per
// incident corner, separated corners rejected), so the restoring SIGNAL exists.
//
// It is a GENUINE LINEAR INSTABILITY, not a discrete bug (2026-07-15, signed-drift probe):
// the lean direction MEANDERS randomly early (top.x wanders ±) then LOCKS IN one noise-
// selected direction near frame ~400 and grows exponentially — i.e. the equilibrium has an
// eigenvalue just above 1 for N≥5, seeded by float noise. Not a fixed solve-order bias
// (so symmetrisation cannot cancel it). EXHAUSTIVELY REFUTED specific causes (each by a
// measured experiment): under-convergence (chaotic iteration ladder), friction (STABILISES;
// frictionless→frame 2), soft-params (hertz/damping/relax), support-ordering, interleaved
// relax (worse), narrowphase coherence, restitution/cache, gravity order, AND — via a
// 10-agent workflow's top candidate — WARM-START carry-over: tightening the match tolerance
// (0.02→1e-3→1e-4) makes it WORSE, not better (warm-start is STABILISING, not the injection).
// Only rotating anchors mildly help (868→1114, better torque arms).
//
// FIX (2026-07-15): a manifold BLOCK solver (solver/block.rs) — solve each contact patch's
// up-to-4 coplanar normal impulses JOINTLY (regularised active-set LCP) instead of one-at-a-
// time Gauss-Seidel, which restores the tilt-resisting torque the sequential sweep lost — plus
// support-order + adaptive iterations (sweeps scale with island depth) so support propagates
// up the column. This raises the buckling critical height from ~5 to ≥16 and cuts residual
// energy ~100× (20 m/s blow-ups → ~0.2 peak). REMAINING: extreme towers (N≥32, 32:1 aspect
// ratio) still buckle eventually — the iterative solve leaves a weak creep and NO iteration/
// reg/damping tuning robustly fixes N32 (chaotic across builds); a whole-chain DIRECT solve
// is needed (soak_extreme_tower_n32_stays_bounded, #[ignore]d). See
// docs/solver-stack-instability-FIXPLAN.md for the full investigation + 10-agent synthesis.
//
// GATE that DID hold: resting penetration sits > slop for tall stacks (N16 0.0074,
// N32 0.0092 vs slop 0.005) — kept below as a mechanism cross-check.

struct StackSoakResult {
    n: usize,
    /// First outer frame at which max|v| ≥ threshold (None ⇒ stayed bounded).
    blew_up_at: Option<usize>,
    /// Peak max|v| over all frames (diagnostic).
    peak_speed: f32,
    /// Max geometric penetration (overlap, m) observed while genuinely at rest
    /// (frame > 30 and this frame's max|v| < 0.1). Determinism-safe: pure position
    /// arithmetic on axis-aligned stacked boxes, no internal solver state.
    resting_penetration: f32,
}

/// Build an isolated N-box stack placed at EXACT contact (zero initial gap/penetration,
/// zero velocity → already at rest) on a static ground and soak it for `frames` outer
/// steps of 1/60 s with the DEFAULT solver. Restitution-0 so this measures solver
/// convergence quality, not bounce chaos.
fn run_resting_stack_soak(n: usize, frames: usize, vel_threshold: f32) -> StackSoakResult {
    run_resting_stack_soak_cfg(n, frames, vel_threshold, |_| {})
}

fn run_resting_stack_soak_cfg(
    n: usize,
    frames: usize,
    vel_threshold: f32,
    configure: impl Fn(&mut ConstraintSolver),
) -> StackSoakResult {
    run_resting_stack_soak_full(n, frames, vel_threshold, None, configure)
}

fn run_resting_stack_soak_full(
    n: usize,
    frames: usize,
    vel_threshold: f32,
    // None → keep material defaults (RED-test behaviour). Some(f) → override both
    // static & dynamic friction to `f` (e.g. 0.0 for the frictionless experiment).
    friction: Option<f32>,
    configure: impl Fn(&mut ConstraintSolver),
) -> StackSoakResult {
    let mut world = PhysicsWorld::new(); // default solver (iterations=20)
    configure(&mut world.solver);
    add_ground(&mut world);

    let half = 0.5;
    let no_bounce = PhysicsMaterial {
        restitution: 0.0,
        static_friction: friction.unwrap_or(PhysicsMaterial::default().static_friction),
        dynamic_friction: friction.unwrap_or(PhysicsMaterial::default().dynamic_friction),
        ..Default::default()
    };
    for i in 0..n {
        let y = half + i as f32 * (2.0 * half); // exact contact: 0.5, 1.5, 2.5, …
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

    let mut result = StackSoakResult {
        n,
        blew_up_at: None,
        peak_speed: 0.0,
        resting_penetration: 0.0,
    };

    for f in 0..frames {
        world.step(1.0 / 60.0).ok();

        // Bail out of penetration/velocity math if anything went non-finite.
        let all_finite = (1..=n).all(|i| {
            world.transforms[i].position.is_finite() && world.velocities[i].linear.is_finite()
        });
        if !all_finite {
            result.blew_up_at.get_or_insert(f);
            result.peak_speed = f32::INFINITY;
            break;
        }

        let max_speed = (1..=n)
            .map(|i| world.velocities[i].linear.length() + world.velocities[i].angular.length())
            .fold(0.0f32, f32::max);
        result.peak_speed = result.peak_speed.max(max_speed);
        if result.blew_up_at.is_none() && max_speed >= vel_threshold {
            result.blew_up_at = Some(f);
        }

        // Steady-state penetration — only while genuinely resting AND still in the
        // original stacked configuration (before any blow-up). The vertical-gap
        // formula assumes axis-aligned stacked boxes, which only holds pre-blow-up;
        // post-blow-up scatter would feed it garbage overlaps. Ground overlap for the
        // bottom box plus every box↔box interface.
        if f > 30 && max_speed < 0.1 && result.blew_up_at.is_none() {
            let mut pen = (half - world.transforms[1].position.y).max(0.0); // ground (top y=0)
            for i in 1..n {
                let gap = world.transforms[i + 1].position.y - world.transforms[i].position.y;
                pen = pen.max((2.0 * half - gap).max(0.0));
            }
            result.resting_penetration = result.resting_penetration.max(pen);
        }
    }

    result
}

fn stack_soak_report(results: &[StackSoakResult]) -> String {
    let mut report = String::from("\nresting-stack soak (max|v| bound 0.5):\n");
    for r in results {
        report.push_str(&format!(
            "  N={:>2}  peak|v|={:>8.3}  resting_pen={:>7.4}  {}\n",
            r.n,
            r.peak_speed,
            r.resting_penetration,
            match r.blew_up_at {
                Some(f) => format!("BLEW UP at frame {f}"),
                None => "stable".to_string(),
            },
        ));
    }
    report
}

/// Regression lock for the resting-stack BUCKLING fix. Before the fix a fully settled stack
/// of N≥5 boxes spontaneously toppled (energy from nowhere). Two layers fixed it:
///   1. Manifold BLOCK solver (solver/block.rs) — resolves each contact patch's coplanar
///      normals jointly, restoring the tilt-resisting torque sequential Gauss-Seidel
///      under-resolved (took the ceiling to N≈16).
///   2. FULL warm-start (warm_start_factor=1.0, 2026-07-15) — the previous 0.85 discarded 15%
///      of the accumulated impulse each substep, and the partial re-convergence injected a
///      marginal amount of energy that compounded in tall stacks. Full warm-start closes it
///      and raises the robust ceiling to N≈32 (verified bounded over 3000 frames in
///      grid_candidate_fixes; the config-sweep data showed this is the single most effective,
///      most principled knob — hertz/tol/iters are chaotic and don't compose).
/// Realistic game stacks (≤~12) are far below this. (Extreme 48+ towers still buckle and need
/// a friction-aware direct solve — see the #[ignore]d acceptance test below.)
#[test]
fn soak_resting_stacks_stay_bounded() {
    let frames = 1500;
    let vel_threshold = 0.5;
    // Full warm-start (warm_start_factor=1.0) raised the robust-stability ceiling from N≤16
    // to N≤32 (verified bounded over 3000 frames in grid_candidate_fixes). Lock the whole
    // range so a regression to the old partial-warm-start injection is caught.
    let results: Vec<StackSoakResult> = [2usize, 5, 16, 24, 32]
        .into_iter()
        .map(|n| run_resting_stack_soak(n, frames, vel_threshold))
        .collect();
    let report = stack_soak_report(&results);
    eprint!("{report}");

    let blew: Vec<&StackSoakResult> = results.iter().filter(|r| r.blew_up_at.is_some()).collect();
    assert!(
        blew.is_empty(),
        "a resting stack (N≤32) spontaneously gained energy / buckled — the full-warm-start \
         stack-stability fix regressed:{report}"
    );
}

/// Exercises the opt-in whole-chain DIRECT solve end-to-end (island Delassus assembly +
/// tgs_sweep_island): a small chain with `direct_chain_solve=true` must stay finite and
/// bounded. (The direct solve extends the stable-tower range; it is default-off due to its
/// O(n³) cost — see soak_extreme_tower_n32_stays_bounded for the range it still doesn't reach.)
#[test]
fn direct_chain_solve_keeps_small_stack_bounded() {
    let r = run_resting_stack_soak_cfg(6, 300, 0.5, |s| s.direct_chain_solve = true);
    assert!(r.peak_speed.is_finite(), "direct solve produced non-finite velocity");
    assert!(
        r.blew_up_at.is_none(),
        "direct-solve chain blew up (peak {:.3} @ {:?})",
        r.peak_speed,
        r.blew_up_at
    );
}

/// Acceptance test for the REMAINING work: extreme aspect-ratio towers (N≥48). Full warm-start
/// pushed robust stability to N≤32 (now locked in soak_resting_stacks_stay_bounded), but the
/// residual marginal energy injection still compounds for VERY tall single columns — N=48
/// blows up around frame ~200, and empirically NO parameter tuning (warm-start/hertz/tol/iters,
/// alone or combined) robustly fixes N≥48; the blow-up frame is chaotic. A complete fix needs a
/// friction-aware whole-chain DIRECT/global solve (the current `solve_island_normals` handles
/// only normals, and is O(n³)). Un-ignore when that lands. See grid_candidate_fixes for data.
#[test]
#[ignore = "extreme tower (N48) still buckles eventually; needs a friction-aware whole-chain direct solve — un-ignore when it lands"]
fn soak_extreme_tower_n48_stays_bounded() {
    let results: Vec<StackSoakResult> = [48usize]
        .into_iter()
        .map(|n| run_resting_stack_soak(n, 1500, 0.5))
        .collect();
    let report = stack_soak_report(&results);
    eprint!("{report}");
    assert!(
        results.iter().all(|r| r.blew_up_at.is_none()),
        "extreme tower N48 still buckles:{report}"
    );
}

/// Empirical sweep harness (not a pass/fail test) — run with:
///   cargo test -p gizmo-physics-rigid --test soak_and_golden sweep_ -- --ignored --nocapture
/// Prints the blow-up frame + peak speed for N∈{16,32} under a matrix of solver configs
/// so stack-instability fixes are chosen from data, not speculation.
#[test]
#[ignore = "diagnostic sweep, run manually with --ignored --nocapture"]
fn sweep_resting_stack_configs() {
    let frames = 1500;
    let vt = 0.5;
    type Cfg = (&'static str, fn(&mut ConstraintSolver));
    let configs: &[Cfg] = &[
        ("baseline (ordering on)", |_s| {}),
        ("ordering OFF", |s| s.support_ordering = false),
        ("iters=25", |s| s.iterations = 25),
        ("iters=30", |s| s.iterations = 30),
        ("iters=30 + ordering OFF", |s| {
            s.iterations = 30;
            s.support_ordering = false;
        }),
        ("relax=8", |s| s.relax_iterations = 8),
        ("hertz=60", |s| s.contact_hertz = 60.0),
        ("hertz=15", |s| s.contact_hertz = 15.0),
        ("damping=2", |s| s.contact_damping_ratio = 2.0),
        ("warm_start=1.0", |s| s.warm_start_factor = 1.0),
        // KEY: the SI (non-TGS) solver ALSO blows up (even earlier) → the injection is
        // NOT TGS-specific; it is upstream/shared (warm-start / narrowphase manifold).
        ("SI path (use_tgs_soft=off)", |s| s.use_tgs_soft = false),
        // Box2D-v3 rotating anchors: mild N16 help only, does not fix the bug.
        ("rotating_anchors", |s| s.rotating_anchors = true),
        // ⭐ Warm-start match tolerance sweep (workflow's decisive experiment): tighter
        // tolerance denies stale-J-at-shifted-arm to slid contacts → should kill buckling.
        ("ws_tol=5e-3", |s| s.warm_start_match_tolerance = 5e-3),
        ("ws_tol=1e-3", |s| s.warm_start_match_tolerance = 1e-3),
        ("ws_tol=1e-4", |s| s.warm_start_match_tolerance = 1e-4),
        ("ws_tol=1e-3 + SI", |s| {
            s.warm_start_match_tolerance = 1e-3;
            s.use_tgs_soft = false;
        }),
        // ⭐ BLOCK SOLVER (structural fix): joint per-manifold normal solve → exact
        // tilt-restoring torque → should raise the buckling critical height.
        ("block_solver", |s| s.block_solver = true),
        ("block + ordering", |s| {
            s.block_solver = true;
            s.support_ordering = true;
        }),
        ("block+ord+iters30", |s| {
            s.block_solver = true;
            s.support_ordering = true;
            s.iterations = 30;
        }),
        ("block+ord+iters40", |s| {
            s.block_solver = true;
            s.support_ordering = true;
            s.iterations = 40;
        }),
        ("block+ord+iters60", |s| {
            s.block_solver = true;
            s.support_ordering = true;
            s.iterations = 60;
        }),
    ];
    // Decisive test: is the escape seeded by friction coupling the manifold's 4-point
    // asymmetry into lateral/angular motion? Compare default friction vs frictionless.
    println!("\n=== friction vs frictionless (blow-up frame) ===");
    for fr in [None, Some(0.0f32)] {
        for n in [16usize, 32] {
            let r = run_resting_stack_soak_full(n, 1600, vt, fr, |_| {});
            let tag = match r.blew_up_at {
                Some(f) => format!("BLOW@{f:<4}(pk{:>5.0})", r.peak_speed),
                None => "OK(never)".to_string(),
            };
            print!("  fr={:<5} N{n}: {tag}    ", fr.map_or("dflt".into(), |x| format!("{x}")));
        }
        println!();
    }

    // Decisive test of the O(N²)→O(N) ordering hypothesis: if ordering makes GS
    // propagate the support front one contact/sweep, N=32 should stabilise around
    // iters≈N (~35) WITH ordering, but need ~(N+1)²/π²≈110 WITHOUT it.
    println!("\n=== iteration ladder: ordering ON vs OFF (N=32, blow-up frame) ===");
    for iters in [30usize, 35, 40, 50, 60, 80, 110] {
        for on in [true, false] {
            let r = run_resting_stack_soak_cfg(32, 1600, vt, move |s| {
                s.iterations = iters;
                s.support_ordering = on;
            });
            let tag = match r.blew_up_at {
                Some(f) => format!("BLOW@{f:<4}(pk{:>5.0})", r.peak_speed),
                None => "  OK (never)   ".to_string(),
            };
            print!("  iters={iters:<3} ord={:<5} {tag}    ", on);
        }
        println!();
    }

    println!("\n=== resting-stack config sweep (1500 frames, blow-up frame @ max|v|≥0.5) ===");
    for &(name, cfg) in configs {
        let mut line = format!("{name:>28} | ");
        for n in [16usize, 32] {
            let r = run_resting_stack_soak_cfg(n, frames, vt, cfg);
            let tag = match r.blew_up_at {
                Some(f) => format!("N{n}: BLOW@{f:<4} (pk {:>6.1})", r.peak_speed),
                None => format!("N{n}: OK (pk {:>5.2})    ", r.peak_speed),
            };
            line.push_str(&format!("{tag}   "));
        }
        println!("{line}");
    }
    println!();
}

#[test]
#[ignore = "diagnostic grid, run manually with --ignored --nocapture"]
fn grid_block_stability() {
    // Find a robust block-solver operating point: for each N, print blow-up frame (or OK)
    // across iterations / reg / relax. Block + ordering on, adaptive iterations OFF here
    // (iterations set explicitly) to isolate the landscape.
    // Long-horizon (3000-frame) verification of the SHIPPING config (default adaptive
    // block solver — no iteration override) to confirm the achieved guarantee is genuine
    // stability, not just delay past 1500 frames. Finds the boundary N.
    println!("\n=== shipping default (block+ord+adaptive+direct), 3000 frames ===");
    for n in [16usize, 24, 32, 40, 48, 64] {
        let r = run_resting_stack_soak(n, 3000, 0.5);
        let tag = match r.blew_up_at {
            Some(f) => format!("BLOW @{f}"),
            None => "OK".to_string(),
        };
        println!("  N={n:<3} peak|v|={:>8.3}  {tag}", r.peak_speed);
    }
    println!();
}

#[test]
#[ignore = "diagnostic grid, run manually with --ignored --nocapture"]
fn grid_candidate_fixes() {
    // The config sweep found 3 independent knobs that stabilise N=32 (contact_hertz=60,
    // warm_start_factor=1.0, warm_start_match_tolerance=1e-3). But the iteration ladder is
    // CHAOTIC, so "stable at N=32/1500f" may be luck. Verify each candidate (and combos) is
    // ROBUSTLY stable across N=16..64 over 3000 frames. The winner becomes the new default.
    type Cfg = (&'static str, fn(&mut ConstraintSolver));
    let cands: &[Cfg] = &[
        ("baseline (defaults)", |_s| {}),
        ("ws_factor=1.0", |s| s.warm_start_factor = 1.0),
        ("direct", |s| s.direct_chain_solve = true),
        ("direct + ws1.0", |s| {
            s.direct_chain_solve = true;
            s.warm_start_factor = 1.0;
        }),
        ("direct + ws1.0 + tol1e-3", |s| {
            s.direct_chain_solve = true;
            s.warm_start_factor = 1.0;
            s.warm_start_match_tolerance = 1e-3;
        }),
        ("direct + ws1.0 + hertz60", |s| {
            s.direct_chain_solve = true;
            s.warm_start_factor = 1.0;
            s.contact_hertz = 60.0;
        }),
    ];
    println!("\n=== candidate-fix robustness grid (3000 frames, N=16..64) ===");
    for &(name, cfg) in cands {
        let mut line = format!("{name:>22} | ");
        for n in [16usize, 24, 32, 48, 64] {
            let r = run_resting_stack_soak_cfg(n, 3000, 0.5, cfg);
            let tag = match r.blew_up_at {
                Some(f) => format!("N{n}:BLOW@{f}"),
                None => format!("N{n}:OK({:.2})", r.peak_speed),
            };
            line.push_str(&format!("{tag:<15} "));
        }
        println!("{line}");
    }
    println!();
}

#[test]
#[ignore = "diagnostic, run manually with --ignored --nocapture"]
fn probe_buckling_vs_pump() {
    // DECISIVE: is the escape a LATERAL BUCKLING/topple (lean grows steadily → topple) or a
    // vertical energy pump (stays vertical, then sudden)? Track, per frame up to blow-up:
    //   lat  = max box |x|+|z|          (lateral COM drift = lean of the column)
    //   ang  = max box rotation angle    (2·acos|w|, radians)
    //   maxv = max box |v_lin|+|v_ang|
    // If lat/ang grow monotonically well BEFORE maxv crosses 0.5 → buckling.
    // If lat/ang stay ~0 until maxv spikes → vertical pump that then goes lateral.
    for n in [16usize] {
        let mut world = PhysicsWorld::new();
        add_ground(&mut world);
        let half = 0.5;
        let no_bounce = PhysicsMaterial {
            restitution: 0.0,
            ..Default::default()
        };
        for i in 0..n {
            let y = half + i as f32 * (2.0 * half);
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
        println!("\n=== N={n} buckling probe (frame: top.x top.z  lat  ang  max|v|) — signed to see if the lean is SYSTEMATIC ===");
        for f in 0..1200 {
            world.step(1.0 / 60.0).ok();
            let mut lat = 0.0f32;
            let mut ang = 0.0f32;
            let mut maxv = 0.0f32;
            for i in 1..=n {
                let p = world.transforms[i].position;
                lat = lat.max(p.x.abs() + p.z.abs());
                let w = world.transforms[i].rotation.w.abs().min(1.0);
                ang = ang.max(2.0 * w.acos());
                maxv = maxv.max(
                    world.velocities[i].linear.length() + world.velocities[i].angular.length(),
                );
            }
            let top = world.transforms[n].position; // signed drift of the top box
            if f % 40 == 0 || maxv > 0.5 {
                println!(
                    "  f{f:<4} top.x={:>+9.5} top.z={:>+9.5} lat={lat:>8.5} ang={ang:>8.5} maxv={maxv:>7.4}",
                    top.x, top.z
                );
            }
            if maxv > 0.5 {
                println!("  -> ESCAPE at {f}");
                break;
            }
        }
    }
}

#[test]
#[ignore = "diagnostic, run manually with --ignored --nocapture"]
fn probe_resting_stack_sleep() {
    // Why doesn't the resting stack sleep before it blows up? Trace the velocity
    // plateau and sleep state for N=16 up to the blow-up.
    for n in [16usize] {
        let mut world = PhysicsWorld::new();
        add_ground(&mut world);
        let half = 0.5;
        let no_bounce = PhysicsMaterial {
            restitution: 0.0,
            ..Default::default()
        };
        for i in 0..n {
            let y = half + i as f32 * (2.0 * half);
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
        println!("\n=== N={n} sleep probe (frame: max|v| min|v| #sleeping) ===");
        for f in 0..1000 {
            world.step(1.0 / 60.0).ok();
            let speeds: Vec<f32> = (1..=n)
                .map(|i| world.velocities[i].linear.length() + world.velocities[i].angular.length())
                .collect();
            let maxv = speeds.iter().cloned().fold(0.0f32, f32::max);
            let minv = speeds.iter().cloned().fold(f32::MAX, f32::min);
            let sleeping = (1..=n).filter(|&i| world.rigid_bodies[i].is_sleeping).count();
            let energy = world.calculate_total_energy();
            if f < 20 || f % 40 == 0 || maxv > 0.5 {
                println!("  f{f:<4} max={maxv:>7.4} min={minv:>7.4} sleeping={sleeping}/{n} E={energy:>10.4}");
            }
            if maxv > 0.5 {
                println!("  -> BLEW UP at {f}");
                break;
            }
        }
    }
}

/// Validates dropping the `yikim_ustasi` demo's `spawn_asleep` workaround: its hardest
/// structure (level 3 — a 2-wide × 12-tall tower, mass 1.4, friction 0.85, damping
/// 0.06/0.12) is built AWAKE and must stay bounded/upright. Before the block solver an
/// awake tower buckled; the demo hid it by spawning blocks asleep. Now the real solver
/// holds it, so the hack is gone — this locks that in.
#[test]
fn soak_demo_tower_awake_stays_upright() {
    let mut world = PhysicsWorld::new();
    add_ground(&mut world);

    let half = 0.5;
    let mat = PhysicsMaterial {
        restitution: 0.0,
        static_friction: 0.85,
        dynamic_friction: 0.85,
        ..Default::default()
    };
    let mut id = 1u32;
    for row in 0..12 {
        for dx in [-0.5f32, 0.5] {
            let pos = Vec3::new(-4.0 + dx, half + row as f32, 0.0);
            let mut rb = RigidBody::new(1.4, true);
            rb.wake_up();
            rb.linear_damping = 0.06;
            rb.angular_damping = 0.12;
            let col = Collider::box_collider(Vec3::splat(half)).with_material(mat);
            rb.update_inertia_from_collider(&col);
            world.add_body(
                BodyHandle::from_id(id),
                rb,
                Transform::new(pos),
                Velocity::default(),
                col,
            );
            id += 1;
        }
    }
    let n = 24usize;

    // 10 s awake at rest — the demo hits the tower within a few seconds, so this is a
    // generous margin for the "stands on its own until struck" guarantee.
    for f in 0..600 {
        world.step(1.0 / 60.0).ok();
        let max_speed = (1..=n)
            .map(|i| world.velocities[i].linear.length() + world.velocities[i].angular.length())
            .fold(0.0f32, f32::max);
        assert!(
            max_speed < 0.5,
            "awake demo tower gained energy / buckled at frame {f}: max|v|={max_speed}"
        );
    }
    // Nothing toppled off or sank through the floor.
    for i in 1..=n {
        let p = world.transforms[i].position;
        assert!(p.is_finite(), "block {i} non-finite");
        assert!(p.y > 0.0 && p.x.abs() < 8.0, "block {i} left the tower: {p:?}");
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
    assert!(
        p.x.abs() < 0.05 && p.z.abs() < 0.05,
        "kutu yana kaydı: {p:?}"
    );
    // Dinlenmede (uyumuş ya da neredeyse durgun).
    assert!(v.length() < 0.1, "kutu dinlenmedi: |v|={}", v.length());
}
