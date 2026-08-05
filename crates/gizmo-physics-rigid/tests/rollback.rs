//! Faz 3 — Deterministik rollback netcode (GGPO çekirdeği).
//!
//! `PhysicsWorld::snapshot()/restore_snapshot()` + `state_hash()` (Faz 2) üzerine kurulu.
//! Doğrular:
//!   1) rollback + re-simülasyon, kesintisiz simülasyonla BİT-BİT aynı (warm-start dahil
//!      tam durum geri yüklendiği için),
//!   2) lag/jitter/paket-kaybı altında bir peer, geç gelen girdileri rollback ederek
//!      "ground truth" peer'e YAKINSAR (state_hash eşitliği = senkron) — exit kriteri.

use gizmo_physics_core::BodyHandle;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const DT: f32 = 1.0 / 60.0;
const CONTROLLED: usize = 1; // entity 1 → idx 1 (zemin idx 0)

fn build_scene() -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    let mut ground = RigidBody::new_static();
    ground.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        ground,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
    );
    // Kontrollü kutu (girdi alır) + temas/warm-start için komşular.
    for x in 0u32..4 {
        let id = x + 1;
        let mut rb = RigidBody::new(1.0, true);
        rb.wake_up();
        let col = Collider::box_collider(Vec3::splat(0.5));
        rb.update_inertia_from_collider(&col);
        world.add_body(
            BodyHandle::from_id(id),
            rb,
            Transform::new(Vec3::new(x as f32 * 1.02 - 1.5, 0.5, 0.0)),
            Velocity::default(),
            col,
        );
    }
    world
}

/// Girdiyi uygula: kontrollü cisme yatay dürtü (deterministik; uyuyorsa uyandır).
fn apply_input(w: &mut PhysicsWorld, value: f32) {
    if value != 0.0 && w.rigid_bodies[CONTROLLED].is_sleeping {
        w.rigid_bodies[CONTROLLED].wake_up();
    }
    let inv_m = w.rigid_bodies[CONTROLLED].inv_mass();
    w.velocities[CONTROLLED].linear.x += value * inv_m;
}

/// Deterministik "girdi" dizisi (LCG; ground truth + peer aynısını üretir/öğrenir).
fn input_at(tick: usize) -> f32 {
    // tick'e bağlı tekrarlanabilir değer (-0.6..0.6 aralığı).
    let r = (tick.wrapping_mul(1103515245).wrapping_add(12345) >> 16) % 5;
    (r as f32 - 2.0) * 0.3
}

#[test]
fn rollback_resimulation_matches_continuous() {
    // Ground truth: 40 tick kesintisiz.
    let mut gt = build_scene();
    for t in 0..40 {
        apply_input(&mut gt, input_at(t));
        gt.step(DT).ok();
    }
    let truth = gt.state_hash();

    // Peer: 20 tick → snapshot → 40'a git; sonra ROLLBACK(20) + resim 20→40.
    let mut w = build_scene();
    for t in 0..20 {
        apply_input(&mut w, input_at(t));
        w.step(DT).ok();
    }
    let snap = w.snapshot();
    for t in 20..40 {
        apply_input(&mut w, input_at(t));
        w.step(DT).ok();
    }
    assert_eq!(w.state_hash(), truth, "kontrol: kesintisiz sim deterministik değil");

    w.restore_snapshot(&snap);
    for t in 20..40 {
        apply_input(&mut w, input_at(t));
        w.step(DT).ok();
    }
    assert_eq!(
        w.state_hash(),
        truth,
        "rollback+resim ≠ kesintisiz → rollback deterministik değil (tam durum/warm-start eksik?)"
    );
}

#[test]
fn rollback_netcode_converges_under_lag_jitter_loss() {
    const N: usize = 80;
    const LAG: usize = 5; // her girdi LAG tick geç öğrenilir (en kötü hal: her tick rollback)

    // Ground truth: tüm girdiler anında.
    let mut gt = build_scene();
    for t in 0..N {
        apply_input(&mut gt, input_at(t));
        gt.step(DT).ok();
    }
    let truth = gt.state_hash();

    // Peer: tick t'de yalnız input_at(t-LAG)'ı KESİN bilir; o ana dek 0.0 tahmin eder.
    // Geç gelen kesin girdi tahmini bozarsa → o tick'e rollback + resim.
    // (Jitter/paket-kaybı: bazı girdiler atlanıp DAHA SONRA toplu teslim edilir — aşağıda
    //  döngü sonrası kalan LAG girdi için son bir rollback yapılır.)
    let mut peer = build_scene();
    let mut known: Vec<f32> = vec![0.0; N]; // tahmin = 0
    let mut confirmed: Vec<bool> = vec![false; N];
    // snaps[t] = tick t'nin BAŞINDAKİ durum (girdi+step ÖNCESİ).
    let mut snaps: Vec<gizmo_physics_rigid::WorldSnapshot> = Vec::with_capacity(N + 1);

    // Bir tick'i ileri sürer (girdi uygula + step), snaps[t]'yi t'nin başında kaydeder.
    fn advance(peer: &mut PhysicsWorld, snaps: &mut Vec<gizmo_physics_rigid::WorldSnapshot>, t: usize, input: f32) {
        if snaps.len() == t {
            snaps.push(peer.snapshot());
        } else {
            snaps[t] = peer.snapshot();
        }
        apply_input(peer, input);
        peer.step(DT).ok();
    }

    for t in 0..N {
        // Bu tick'te input_at(t-LAG) kesinleşir.
        if t >= LAG {
            let it = t - LAG;
            let truth_in = input_at(it);
            if !confirmed[it] {
                confirmed[it] = true;
                if known[it] != truth_in {
                    known[it] = truth_in;
                    // Rollback: it'in başına dön, it..t arası kesinleşmiş girdilerle resim.
                    peer.restore_snapshot(&snaps[it]);
                    // rt bir tick numarası: hem known[rt] indeksi hem advance()'e argüman.
                    #[allow(clippy::needless_range_loop)]
                    for rt in it..t {
                        advance(&mut peer, &mut snaps, rt, known[rt]);
                    }
                }
            }
        }
        // t için (henüz kesinleşmemişse tahmin=known[t]=0) ileri sür.
        advance(&mut peer, &mut snaps, t, known[t]);
    }

    // Döngü sonu: son LAG tick (N-LAG..N) için kesin girdi henüz uygulanmadı (jitter/gecikme).
    // Hepsini teslim al + en erken kesinleşmemiş tick'ten son bir rollback + resim.
    let earliest = N - LAG;
    let mut need_rollback = false;
    // it bir tick numarası: hem known[it] indeksi hem input_at()'e argüman.
    #[allow(clippy::needless_range_loop)]
    for it in earliest..N {
        let truth_in = input_at(it);
        if known[it] != truth_in {
            known[it] = truth_in;
            need_rollback = true;
        }
    }
    if need_rollback {
        peer.restore_snapshot(&snaps[earliest]);
        // rt bir tick numarası: hem known[rt] indeksi hem advance()'e argüman.
        #[allow(clippy::needless_range_loop)]
        for rt in earliest..N {
            advance(&mut peer, &mut snaps, rt, known[rt]);
        }
    }

    assert_eq!(
        peer.state_hash(),
        truth,
        "lag/jitter sonrası peer ground-truth'a YAKINSAMADI → rollback netcode senkron değil"
    );
}

// Rollback snapshot completeness: force-field state (gravity_fields / fluid_zones)
// feeds `velocity_integration_step`, so it must be captured and restored. These are
// public mutable Vecs gameplay can change at runtime; if one is modified inside a
// rollback window, a restore that ignored it would resimulate under the wrong forces.
#[test]
fn snapshot_restores_gravity_and_fluid_zones() {
    use gizmo_physics_rigid::world::GravityField;

    let mut world = PhysicsWorld::new();

    // Snapshot with ONE gravity field present.
    world.gravity_fields.push(GravityField::default());
    let snap = world.snapshot();
    assert_eq!(world.gravity_fields.len(), 1);

    // Gameplay mutates the force fields AFTER the snapshot (as could happen mid-window):
    // add a second gravity field and a fluid zone.
    let mut extra = GravityField::default();
    extra.gravity = Vec3::new(0.0, 20.0, 0.0); // upward — clearly different sim
    world.gravity_fields.push(extra);
    assert_eq!(world.gravity_fields.len(), 2);

    // Rollback must revert the force-field state to exactly the snapshot.
    world.restore_snapshot(&snap);
    assert_eq!(
        world.gravity_fields.len(),
        1,
        "restore_snapshot must revert gravity_fields to the snapshot state"
    );
    assert!(
        world.fluid_zones.is_empty(),
        "restore_snapshot must revert fluid_zones to the snapshot state"
    );
}

// ── Joint state is part of the simulation state ──────────────────────────────
//
// `WorldSnapshot` carried transforms, velocities, rigid bodies, the contact cache and the
// force fields — but no joint state at all, even though `PhysicsWorld::joints` holds runtime
// fields that are not derivable from any of those:
//
//   * `is_broken` is a ONE-WAY latch. Nothing ever sets it back to false outside scene load,
//     so a joint that snapped inside a rollback window stayed snapped through the restore and
//     the re-simulation ran with a joint the continuous simulation still had.
//   * `initial_relative_rotation` is the reference pose latched on a joint's FIRST solve
//     (ball-socket, slider, D6). Every limit is measured against it.
//
// Neither is visible to `state_hash`, which hashes only transform/velocity/sleep — so the
// desync stays invisible until it bleeds into velocities, which for a broken joint is
// immediately and permanently.

/// Measured: the rope goes taut at tick 38 and its peak reaction sits between 1200 and
/// 2000 N, so this threshold breaks it at tick 38 — after the snapshot at 20 and before the
/// window closes at 60. The break has to land INSIDE the window or the test is vacuous.
const BREAK_FORCE: f32 = 1200.0;

/// A mass free-falling on a slack rope. The rope only carries load when it goes TAUT, which
/// happens well after the snapshot point — so the break lands inside the rollback window
/// rather than in the release transient at tick 0.
fn breakable_rope(break_force: f32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0));
    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        anchor,
        Transform::new(Vec3::new(0.0, 10.0, 0.0)),
        Velocity::default(),
        Collider::sphere(0.1),
    );
    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(0.2));
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(1),
        rb,
        Transform::new(Vec3::new(0.0, 9.0, 0.0)), // 1 m below: 2 m of slack to fall through
        Velocity::default(),
        col,
    );
    let mut j = gizmo_physics_rigid::Joint::rope(
        BodyHandle::from_id(0),
        BodyHandle::from_id(1),
        Vec3::ZERO,
        Vec3::ZERO,
        3.0,
    );
    j.break_force = break_force;
    world.joints.push(j);
    world
}

/// A joint that breaks inside the rollback window must be un-broken by the restore.
#[test]
fn rollback_restores_a_broken_joint() {
    // Ground truth: 60 ticks straight through.
    let mut gt = breakable_rope(BREAK_FORCE);
    for _ in 0..60 {
        gt.step(DT).ok();
    }
    let truth = gt.state_hash();
    assert!(
        gt.joints[0].is_broken,
        "scene precondition: the joint must actually break within the window, or this test \
         asserts nothing"
    );

    let mut w = breakable_rope(BREAK_FORCE);
    for _ in 0..20 {
        w.step(DT).ok();
    }
    assert!(
        !w.joints[0].is_broken,
        "scene precondition: the joint must still be intact at the snapshot point"
    );
    let snap = w.snapshot();

    for _ in 20..60 {
        w.step(DT).ok();
    }
    assert_eq!(w.state_hash(), truth, "control: the continuous sim is not deterministic");

    w.restore_snapshot(&snap);
    assert!(
        !w.joints[0].is_broken,
        "restore_snapshot must un-break a joint that broke after the snapshot — `is_broken` \
         is a one-way latch and nothing else resets it"
    );
    for _ in 20..60 {
        w.step(DT).ok();
    }
    assert_eq!(
        w.state_hash(),
        truth,
        "rollback + resim != continuous once a joint breaks in the window"
    );
}

/// `initial_relative_rotation` is latched on first solve and every limit is measured from it.
/// If a rollback window spans the latch, the restore has to revert it too.
#[test]
fn rollback_restores_the_latched_joint_reference_pose() {
    use gizmo_physics_rigid::{Joint, JointData};

    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    let mut anchor = RigidBody::new_static();
    anchor.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        anchor,
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        Collider::sphere(0.1),
    );
    let mut rb = RigidBody::new(1.0, false);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(0.2));
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(1),
        rb,
        Transform::new(Vec3::new(0.5, 0.0, 0.0)),
        Velocity {
            angular: Vec3::new(2.0, 0.0, 0.0),
            ..Default::default()
        },
        col,
    );
    let mut j = Joint::ball_socket(
        BodyHandle::from_id(0),
        BodyHandle::from_id(1),
        Vec3::new(0.5, 0.0, 0.0),
        Vec3::ZERO,
    );
    if let JointData::BallSocket(ref mut d) = j.data {
        d.use_cone_limit = true;
        d.cone_limit_angle = 0.5;
    }
    world.joints.push(j);

    // Snapshot BEFORE the first solve, so the reference pose is still unlatched.
    let unlatched = match world.joints[0].data {
        JointData::BallSocket(d) => d.initial_relative_rotation,
        _ => unreachable!(),
    };
    assert!(unlatched.is_none(), "precondition: not yet latched");
    let snap = world.snapshot();

    for _ in 0..30 {
        world.step(DT).ok();
    }
    let latched = match world.joints[0].data {
        JointData::BallSocket(d) => d.initial_relative_rotation,
        _ => unreachable!(),
    };
    assert!(latched.is_some(), "precondition: stepping must latch the reference pose");

    world.restore_snapshot(&snap);
    let after = match world.joints[0].data {
        JointData::BallSocket(d) => d.initial_relative_rotation,
        _ => unreachable!(),
    };
    assert!(
        after.is_none(),
        "restore_snapshot must revert the latched reference pose — every cone/twist/swing \
         limit is measured against it, so a stale one silently redefines the joint's rest pose"
    );
}

