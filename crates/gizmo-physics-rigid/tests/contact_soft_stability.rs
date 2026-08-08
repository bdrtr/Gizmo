//! Measurement: does the contact solver's soft-constraint update have the same
//! divide-the-penalty-term instability the joint solver was fixed for?
//!
//! `ConstraintSolver::soft_coefficients` (joints) documents that `-impulse_scale·λ` must NOT be
//! divided by the row's `k`, because the λ recursion is then
//! `λ_{n+1} = λ_n·(1 − impulse_scale/k) + …`, which diverges once `impulse_scale > 2k`, i.e.
//! `m_eff > 2/impulse_scale`. It flags `solver/tgs.rs`'s normal update as carrying the divided
//! form and says the contact case "needs its own measurement" rather than a blind change.
//!
//! This file is that measurement. With the shipped contact softness
//! (`contact_hertz = 30`, `contact_damping_ratio = 10`, substep `1/240`):
//!
//! ```text
//! omega         = 2π·30                     = 188.50
//! denom         = 2·10 + (1/240)·188.50     =  20.785
//! c             = (1/240)·188.50·20.785     =  16.325
//! impulse_scale = 1/(1+c)                   =   0.0577
//! predicted divergence at m_eff > 2/impulse_scale = 34.6
//! ```
//!
//! Two things narrow the blast radius, and both are asserted here so the scope claim cannot
//! rot: the block solver — **on by default** — drops `impulse_scale` entirely, and the relax
//! pass runs unbiased where `impulse_scale` is 0 anyway. The divided form is therefore only
//! reachable with `block_solver = false`.
//!
//! Run the measurement with:
//! `cargo test -p gizmo-physics-rigid --test contact_soft_stability -- --ignored --nocapture`

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, PhysicsMaterial, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const DT: f32 = 1.0 / 60.0;

/// One heavy box resting flat on static ground, already in contact.
///
/// `m_eff` at each of the four corner contacts is well below `mass`: the corner lever arms add
/// an angular term to `k_n`, so the effective mass per contact is roughly `mass/4` for a unit
/// cube. The sweep below therefore has to run to the high hundreds of kg to cross a threshold
/// that reads as 34.6 in `m_eff`.
fn heavy_box_on_ground(mass: f32, initial_penetration: f32) -> (PhysicsWorld, usize) {
    let mut world = PhysicsWorld::new();
    // The block solver bypasses the code under measurement; see the module docs.
    world.solver.block_solver = false;

    let mut ground = RigidBody::new_static();
    ground.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        ground,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
    );

    let material = PhysicsMaterial {
        restitution: 0.0,
        ..Default::default()
    };
    let collider = Collider::box_collider(Vec3::splat(0.5)).with_material(material);
    let mut rb = RigidBody::new(mass, true);
    rb.update_inertia_from_collider(&collider);
    rb.wake_up();
    world.add_body(
        BodyHandle::from_id(1),
        rb,
        Transform::new(Vec3::new(0.0, 0.5 - initial_penetration, 0.0)),
        Velocity::default(),
        collider,
    );

    (world, 1)
}

struct Readout {
    /// Deepest the box ever sank below its resting height, in metres.
    max_sink: f32,
    /// Largest speed seen in the last quarter of the run — a settled box reads ~0, a box in a
    /// limit cycle reads a steady non-zero value, a diverging one reads a large one.
    late_speed: f32,
    /// Height at the end. NaN or a large negative value means it fell through.
    final_y: f32,
}

fn run(mass: f32, steps: usize) -> Readout {
    run_penetrating(mass, steps, 0.0)
}

/// `initial_penetration` must exceed `slop` (0.005 m) for the soft/biased branch — the only
/// place `impulse_scale` is non-zero — to engage at all. A box parked exactly at contact never
/// reaches it, which is why the first version of this sweep read identically at every mass.
fn run_penetrating(mass: f32, steps: usize, initial_penetration: f32) -> Readout {
    let (mut world, body) = heavy_box_on_ground(mass, initial_penetration);
    let rest_y = 0.5_f32;
    let mut max_sink = 0.0_f32;
    let mut late_speed = 0.0_f32;

    for step in 0..steps {
        world.step(DT).expect("the step must not fail in this scene");
        let y = world.transforms[body].position.y;
        max_sink = max_sink.max(rest_y - y);
        if step >= steps * 3 / 4 {
            late_speed = late_speed.max(world.velocities[body].linear.length());
        }
    }

    Readout {
        max_sink,
        late_speed,
        final_y: world.transforms[body].position.y,
    }
}

/// The measurement. `#[ignore]`d: it is a table to read, not a gate.
#[test]
#[ignore = "measurement, not a gate — run with --ignored --nocapture"]
fn mass_sweep_on_the_sequential_soft_path() {
    for pen in [0.0, 0.05, 0.2] {
        println!("\n-- initial penetration {pen} m (slop = 0.005) --");
        println!("mass(kg)  max_sink(m)   late_speed(m/s)  final_y(m)");
        for mass in [1.0, 10.0, 35.0, 100.0, 139.0, 300.0, 1000.0, 5000.0] {
            let r = run_penetrating(mass, 900, pen);
            println!(
                "{mass:>8}  {:>11.6}  {:>15.6}  {:>9.6}",
                r.max_sink, r.late_speed, r.final_y
            );
        }
    }
}

/// **The gate the measurement earned.** A soft constraint parameterised by hertz and damping
/// recovers penetration at a rate that does not depend on the body's mass — so the same box in
/// the same penetration must come to rest at the same height whether it weighs 1 kg or 5000.
///
/// Dividing `impulse_scale·λ` by `k_n` broke exactly that. Measured final heights from 0.2 m of
/// penetration: `1 kg 0.4733 · 100 kg 0.4092 · 300 kg 0.4084 · 1000 kg 0.4872` — 0.06 m of
/// spread and not even monotonic, the signature of the λ recursion oscillating and being
/// truncated by its own non-negativity clamp. Undivided it is 0.471068 at every mass.
///
/// Fails on the pre-fix solver at the 100 kg row.
#[test]
fn penetration_recovery_does_not_depend_on_mass() {
    for pen in [0.05, 0.2] {
        let reference = run_penetrating(1.0, 900, pen).final_y;
        for mass in [10.0, 35.0, 100.0, 300.0, 1000.0, 5000.0] {
            let y = run_penetrating(mass, 900, pen).final_y;
            assert!(
                (y - reference).abs() < 1e-3,
                "penetration {pen} m: {mass} kg rests at {y}, 1 kg at {reference} — a soft \
                 contact's recovery must not depend on mass"
            );
        }
    }
}

/// A heavy box must also simply settle, whatever the mass-invariance question.
///
/// Kept separate so a future change that destabilises heavy contacts fails CI instead of
/// merely printing a worse number that nobody reads.
#[test]
fn a_heavy_box_settles_on_the_sequential_soft_path() {
    for mass in [1.0, 100.0, 1000.0, 5000.0] {
        let r = run(mass, 900);
        assert!(
            r.final_y.is_finite() && (r.final_y - 0.5).abs() < 0.1,
            "{mass} kg box must rest near y = 0.5, got {} (sank {} m)",
            r.final_y,
            r.max_sink
        );
        assert!(
            r.late_speed < 0.05,
            "{mass} kg box must come to rest; late speed {} m/s",
            r.late_speed
        );
    }
}

/// Pins the scope claim in the module docs: with the DEFAULT solver config the divided form is
/// not reached at all, because the block solver discards `impulse_scale`.
///
/// If this ever fails, the measurement above stops being a niche-config note and becomes a
/// statement about the shipped path.
#[test]
fn the_default_config_uses_the_block_solver() {
    let world = PhysicsWorld::new();
    assert!(
        world.solver.block_solver,
        "the block solver is the default path, and it does not apply `impulse_scale`"
    );
}
