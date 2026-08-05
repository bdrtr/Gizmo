//! Simulation-quality measurement for the contact solver.
//!
//! # Why this file exists
//!
//! The solver scales its per-island sweep count with island support depth
//! (`solver/mod.rs`: `n_iterations = min(96, max(cfg, max(28, 1.5·depth)))` when the block
//! solver is on and `depth >= 5`). `benches/step_bench.rs` measured that this policy, not the
//! per-contact cost, is ~93% of the solver's super-linear growth on a dense scene — and an
//! attempt to throttle it made that scene 1.9× faster while buckling the N=24 and N=32 towers
//! in `soak_resting_stacks_stay_bounded`.
//!
//! That attempt was reverted for a reason worth stating plainly: it measured only SPEED. Nobody
//! measured whether the throttled scene's simulation got WORSE. This file is the missing
//! measurement — the instrument a sweep-throttling change has to pass before it can ship.
//!
//! # The one thing not to forget
//!
//! **The failure mode that matters is invisible to per-step residuals.** The root-cause note in
//! `soak_and_golden.rs` establishes that tall-stack collapse is lateral (Euler) buckling: the
//! column's lean grows exponentially for *hundreds of frames* while `max|v|` stays under 0.05
//! and the stack is, at every individual step, essentially at rest and essentially
//! non-penetrating. A convergence-residual metric therefore reads ~zero through the entire
//! ramp and only notices once the tower is already falling.
//!
//! So a quality suite built only from residuals would have passed the reverted fix. Any metric
//! here has to carry a term that grows *during* the ramp.
//!
//! # What is measured, and how, without new instrumentation
//!
//! Everything below is computed from `PhysicsWorld`'s public state after `step`:
//!
//! - **Residual penetration** — `collision_events()` carries the solved contact points, and the
//!   solver never writes `ContactPoint::penetration` back (it reads it once into `pen0`,
//!   `solver/tgs.rs:302`). So the depth on an event is the narrowphase depth *at the top of that
//!   substep*, i.e. exactly the overlap the previous substep's solve failed to remove. Events
//!   accumulate over the frame's four substeps, so the max over a frame is a residual reading
//!   that needs no solver-internal access and cannot perturb the simulation. This works for any
//!   geometry, unlike the stacked-box vertical-gap formula in `soak_and_golden.rs`.
//! - **Lean and tilt** — position and orientation only.
//! - **Energy** — `calculate_total_energy()`.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, PhysicsMaterial, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const DT: f32 = 1.0 / 60.0;

// ─────────────────────────────────────────────────────────────────────────────
// Per-frame observables
// ─────────────────────────────────────────────────────────────────────────────

/// One frame's quality reading. Every field is derived from public post-step state.
#[derive(Debug, Clone, Copy, Default)]
struct Frame {
    /// `max(|v_lin| + |v_ang|)` over the dynamic bodies — the existing soak's blow-up signal.
    max_speed: f32,
    /// The same, but skipping sleeping bodies.
    ///
    /// These can differ, and if they do then every blow-up frame in this file is suspect. The
    /// solver writes velocities back for every dynamic member of an active island without
    /// checking the sleep flag (`pipeline.rs`), while the integrator skips sleeping bodies
    /// entirely — so a sleeping body can be left holding a velocity it will never act on and
    /// never decay. A blow-up detector reading `max_speed` would report that frozen number as
    /// motion.
    max_speed_awake: f32,
    /// Largest positive contact penetration reported this frame. See the module docs: this is
    /// the depth the previous substep's solve left behind, read back through `collision_events`.
    max_penetration: f32,
    /// Mean positive penetration over all contacts this frame — max alone is a single bad
    /// contact, the mean says whether the whole island is sunk.
    mean_penetration: f32,
    /// Contacts contributing to the two figures above.
    contact_count: usize,
    /// Largest horizontal distance of any body from its own starting `(x, z)`. For a column
    /// this is the buckling amplitude; for a raft it is how far the lattice has spread.
    lean: f32,
    /// Largest angle (radians) between a body's local +Y and world +Y. Rotation-channel twin of
    /// `lean`: a column can buckle by tilting boxes without translating their centres much.
    tilt: f32,
    /// `calculate_total_energy()` — kinetic plus gravitational potential.
    energy: f32,
    /// Dynamic bodies asleep at the end of the frame. A settled scene that never sleeps is a
    /// quality defect in its own right (and a cost defect).
    asleep: usize,
}

/// Read the whole observable set off a stepped world. `origins` is the initial position per
/// dynamic body, in the same order as `bodies`.
fn observe(world: &PhysicsWorld, bodies: &[usize], origins: &[Vec3]) -> Frame {
    let mut f = Frame::default();

    for (&i, &origin) in bodies.iter().zip(origins) {
        let t = &world.transforms[i];
        let v = &world.velocities[i];
        if !t.position.is_finite() || !v.linear.is_finite() || !v.angular.is_finite() {
            return Frame {
                max_speed: f32::INFINITY,
                ..f
            };
        }
        let speed = v.linear.length() + v.angular.length();
        f.max_speed = f.max_speed.max(speed);
        if !world.rigid_bodies[i].is_sleeping {
            f.max_speed_awake = f.max_speed_awake.max(speed);
        }

        let d = t.position - origin;
        f.lean = f.lean.max(Vec3::new(d.x, 0.0, d.z).length());

        let up = t.rotation.mul_vec3(Vec3::Y);
        f.tilt = f.tilt.max(up.dot(Vec3::Y).clamp(-1.0, 1.0).acos());

        if world.rigid_bodies[i].is_sleeping {
            f.asleep += 1;
        }
    }

    let mut sum = 0.0f32;
    for ev in world.collision_events() {
        for c in &ev.contact_points {
            // Negative depth marks a speculative CCD contact, not an overlap — skip those.
            if c.penetration > 0.0 {
                f.max_penetration = f.max_penetration.max(c.penetration);
                sum += c.penetration;
                f.contact_count += 1;
            }
        }
    }
    if f.contact_count > 0 {
        f.mean_penetration = sum / f.contact_count as f32;
    }

    f.energy = world.calculate_total_energy();
    f
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenes
// ─────────────────────────────────────────────────────────────────────────────

/// Ground half-extent used by every scene here. Matches `soak_and_golden.rs`.
///
/// This is not a free choice, and that is a finding in its own right: with 200.0 instead (the
/// value `benches/step_bench.rs` uses) the N=24 tower buckles at frame ~1140 while the soak's
/// identical-by-intent tower stays bounded for 1500. Nothing about the solver changes — the
/// ground is static and its top face is at y=0 either way. Only the float noise of the contact
/// clip against a 10× larger face changes, and the documented root cause of the instability is
/// an eigenvalue just above 1 seeded by exactly that kind of noise. See
/// `ground_extent_flips_the_blow_up` below.
const GROUND_HALF: f32 = 20.0;

fn add_ground_sized(world: &mut PhysicsWorld, half: f32) {
    let mut g = RigidBody::new_static();
    g.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        g,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(half, 1.0, half)),
    );
}

fn add_ground(world: &mut PhysicsWorld) {
    add_ground_sized(world, GROUND_HALF);
}

fn add_box(world: &mut PhysicsWorld, id: u32, pos: Vec3, half: f32, material: PhysicsMaterial) {
    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let col = Collider::box_collider(Vec3::splat(half)).with_material(material);
    rb.update_inertia_from_collider(&col);
    world.add_body(
        BodyHandle::from_id(id),
        rb,
        Transform::new(pos),
        Velocity::default(),
        col,
    );
}

/// The `soak_and_golden.rs` tower: `n` restitution-0 boxes stacked at exact contact on the
/// ground, already at rest. This is the scene whose sweeps demonstrably cannot be cut.
fn scene_tower(n: usize) -> (PhysicsWorld, Vec<usize>, Vec<Vec3>) {
    scene_tower_on_ground(n, GROUND_HALF)
}

fn scene_tower_on_ground(n: usize, ground_half: f32) -> (PhysicsWorld, Vec<usize>, Vec<Vec3>) {
    let mut world = PhysicsWorld::new();
    add_ground_sized(&mut world, ground_half);
    let half = 0.5;
    let no_bounce = PhysicsMaterial {
        restitution: 0.0,
        ..Default::default()
    };
    for i in 0..n {
        let y = half + i as f32 * (2.0 * half);
        add_box(
            &mut world,
            i as u32 + 1,
            Vec3::new(0.0, y, 0.0),
            half,
            no_bounce,
        );
    }
    let bodies: Vec<usize> = (1..=n).collect();
    let origins = bodies.iter().map(|&i| world.transforms[i].position).collect();
    (world, bodies, origins)
}

/// The `step_bench.rs` `dense_contacts` scene: `n` boxes of half-extent 0.5 on a 0.9-spaced
/// square lattice at y=5, zero gravity, never touching the ground — so the island is
/// anchor-free and its support depth is the lattice diameter, √n−1. This is the scene whose
/// sweeps look wasted.
fn scene_raft(n: u32) -> (PhysicsWorld, Vec<usize>, Vec<Vec3>) {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    add_ground(&mut world);
    let side = (n as f32).sqrt().ceil() as u32;
    for i in 0..n {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        add_box(
            &mut world,
            i + 1,
            Vec3::new(x * 0.9, 5.0, z * 0.9),
            0.5,
            PhysicsMaterial::default(),
        );
    }
    let bodies: Vec<usize> = (1..=n as usize).collect();
    let origins = bodies.iter().map(|&i| world.transforms[i].position).collect();
    (world, bodies, origins)
}

/// The raft's honest counterpart: the same anchor-free mid-air lattice, but spaced at EXACT
/// contact instead of 0.1 m overlapped, in zero gravity, at rest.
///
/// This scene exists because the benchmark raft cannot answer the quality question. Placing 256
/// rigid boxes 0.1 m inside each other is not a physical state, the lattice has no
/// non-overlapping configuration reachable without expanding, and every solver invents
/// something to escape it — so there is no correct answer to compare against, and
/// `sweep_ladder_raft` duly shows more sweeps buying *more* invented energy with no way to say
/// whether that is better or worse.
///
/// At exact contact the correct answer is not merely definable, it is trivial: no gravity, no
/// velocity, no overlap, nothing to solve. **The lattice must not move, at all, forever.** Any
/// motion is invented by the solver, so the quality scalar is absolute — its ideal value is
/// zero, not whatever the current build happens to produce.
///
/// And it keeps the property the whole question is about: no body touches the ground, so the
/// island is anchor-free and its support depth is the lattice diameter √n−1, which is exactly
/// the input that drives the adaptive sweep count on the benchmark scene.
///
/// **Measured result, and it is the point of the scene rather than a disappointment:** this
/// reads exactly 0.0000 on every channel at every sweep count from 1 to 96, and every body is
/// asleep. In zero gravity with nothing overlapping there is no load for the contacts to carry,
/// so an anchor-free lattice has literally nothing for the extra sweeps to converge. That is a
/// real answer to "are the raft's sweeps earned" — for an unloaded anchor-free cluster they are
/// not — but it is also why this scene cannot be the whole test: it exercises the sleep system
/// as much as the solver. The loaded anchor-free case is [`scene_compressed_free_chain`].
fn scene_floating_lattice(n: u32) -> (PhysicsWorld, Vec<usize>, Vec<Vec3>) {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    add_ground(&mut world);
    let side = (n as f32).sqrt().ceil() as u32;
    for i in 0..n {
        let (x, z) = ((i % side) as f32, (i / side) as f32);
        add_box(
            &mut world,
            i + 1,
            Vec3::new(x, 5.0, z), // spacing 1.0 == 2·half: faces touch, nothing overlaps
            0.5,
            PhysicsMaterial::default(),
        );
    }
    let bodies: Vec<usize> = (1..=n as usize).collect();
    let origins = bodies.iter().map(|&i| world.transforms[i].position).collect();
    (world, bodies, origins)
}

/// A `side × height × side` block of crates at exact contact on the ground, under real gravity.
///
/// This is the realistic dense scene the benchmark raft is standing in for, and the reason it
/// is here is to check whether a realistic dense scene reaches the pathological depth at all.
/// Its bottom layer rests on the ground, so the BFS has anchors and the support depth is the
/// stack HEIGHT — the quantity the adaptive policy was written for — rather than a lattice
/// diameter. Ideal answer, again exactly zero: a block at exact contact must stay put.
fn scene_crate_pile(side: u32, height: u32) -> (PhysicsWorld, Vec<usize>, Vec<Vec3>) {
    scene_crate_pile_spaced(side, height, 1.0)
}

/// `lateral_spacing` is the horizontal pitch; at 1.0 (== 2·half) neighbouring columns touch
/// exactly, above it they start apart. Vertical pitch is always exact contact.
fn scene_crate_pile_spaced(
    side: u32,
    height: u32,
    lateral_spacing: f32,
) -> (PhysicsWorld, Vec<usize>, Vec<Vec3>) {
    let mut world = PhysicsWorld::new();
    add_ground(&mut world);
    let no_bounce = PhysicsMaterial {
        restitution: 0.0,
        ..Default::default()
    };
    let mut id = 1u32;
    for y in 0..height {
        for x in 0..side {
            for z in 0..side {
                add_box(
                    &mut world,
                    id,
                    Vec3::new(x as f32 * lateral_spacing, 0.5 + y as f32, z as f32 * lateral_spacing),
                    0.5,
                    no_bounce,
                );
                id += 1;
            }
        }
    }
    let bodies: Vec<usize> = (1..id as usize).collect();
    let origins = bodies.iter().map(|&i| world.transforms[i].position).collect();
    (world, bodies, origins)
}

/// A row of `n` boxes at exact contact in zero gravity, with the two end boxes given equal and
/// opposite inward velocities — an anchor-free island that is nevertheless deep AND loaded.
///
/// This is the scene the reverted fix would have got wrong, isolated. That fix suppressed the
/// sweep scaling whenever an island had no static or kinematic anchor, on the reasoning that
/// without an anchor there is no support chain. This chain has no anchor and is nothing but
/// support chain: the impulse has to propagate along all `n` bodies, which is precisely the
/// work the sweeps do.
///
/// It reaches support depth `n−1` — with no anchor the BFS roots at the lowest-indexed body
/// (`solver/mod.rs`), so a linked chain's eccentricity is its length, the same as a tower of
/// that height. It does not *start* there: only the two ends are in contact at first, so the
/// island is fragmented and coalesces as the compression wave travels inward. Measured at n=32
/// by `audit_effective_sweeps_per_scene` — depth 3 → 7 → 11 → 15 → 31 over frames 0–4, with the
/// sweep count following at 20 → 26 → 28 → 28 → 46. Read the scene past frame 4 for the
/// deep-island behaviour.
///
/// Two things are exactly known here rather than blessed from a previous run, which is what
/// makes it a gate and not a baseline:
///   - **Momentum.** The initial linear momentum is exactly zero by symmetry and there are no
///     external forces, so it must stay zero. A contact solver that leaks momentum is wrong,
///     full stop.
///   - **Energy.** There is no source of energy, so the kinetic energy must never exceed what
///     the two end boxes started with. Restitution is 0, so it should fall well below it.
fn scene_compressed_free_chain(n: usize) -> (PhysicsWorld, Vec<usize>, Vec<Vec3>) {
    let mut world = PhysicsWorld::new().with_gravity(Vec3::ZERO);
    add_ground(&mut world); // present but far below; nothing ever reaches it
    let no_bounce = PhysicsMaterial {
        restitution: 0.0,
        ..Default::default()
    };
    for i in 0..n {
        add_box(
            &mut world,
            i as u32 + 1,
            Vec3::new(i as f32, 50.0, 0.0),
            0.5,
            no_bounce,
        );
    }
    let bodies: Vec<usize> = (1..=n).collect();
    world.velocities[bodies[0]].linear = Vec3::new(1.0, 0.0, 0.0);
    world.velocities[bodies[n - 1]].linear = Vec3::new(-1.0, 0.0, 0.0);
    let origins = bodies.iter().map(|&i| world.transforms[i].position).collect();
    (world, bodies, origins)
}

/// What the adaptive policy actually did on the last step: `(max island support depth, sweeps
/// summed over islands and substeps, islands)`.
///
/// A step at 1/60 s runs four 1/240 s substeps, and both counters accumulate across all of them,
/// so `sweeps / islands` is the average per-island sweep count for the frame.
fn policy_readout(world: &PhysicsWorld) -> (u32, usize, usize) {
    (
        world.metrics.max_island_depth,
        world.metrics.solver_sweeps,
        world.metrics.island_count,
    )
}

/// Linear momentum and kinetic energy of the listed bodies.
fn momentum_and_ke(world: &PhysicsWorld, bodies: &[usize]) -> (Vec3, f32) {
    let mut p = Vec3::ZERO;
    let mut ke = 0.0f32;
    for &i in bodies {
        let v = &world.velocities[i];
        let rb = &world.rigid_bodies[i];
        p += rb.mass * v.linear;
        ke += 0.5 * rb.mass * v.linear.length_squared()
            + 0.5 * rb.local_inertia.dot(v.angular * v.angular);
    }
    (p, ke)
}

// ─────────────────────────────────────────────────────────────────────────────
// EXPLORATORY MEASUREMENTS (ignored by default — they print, they do not assert)
// ─────────────────────────────────────────────────────────────────────────────

fn trace(label: &str, world: &mut PhysicsWorld, bodies: &[usize], origins: &[Vec3], frames: usize) {
    eprintln!("\n=== {label} ===");
    eprintln!(
        "{:>6}  {:>10}  {:>10}  {:>10}  {:>8}  {:>10}  {:>9}  {:>12}  {:>6}",
        "frame", "max|v|", "lean", "tilt(deg)", "contacts", "pen_max", "pen_mean", "energy", "asleep"
    );
    for f in 0..frames {
        world.step(DT).ok();
        let o = observe(world, bodies, origins);
        let report = f < 5 || (f + 1) % (frames / 25).max(1) == 0 || f == frames - 1;
        if report {
            eprintln!(
                "{:>6}  {:>10.5}  {:>10.6}  {:>10.4}  {:>8}  {:>10.6}  {:>9.6}  {:>12.4}  {:>6}",
                f,
                o.max_speed,
                o.lean,
                o.tilt.to_degrees(),
                o.contact_count,
                o.max_penetration,
                o.mean_penetration,
                o.energy,
                o.asleep,
            );
        }
        if !o.max_speed.is_finite() {
            eprintln!("  non-finite at frame {f} — stopping");
            break;
        }
    }
}

/// Summary of a whole run — the shape a gate would compare.
#[derive(Debug, Clone, Copy)]
struct Run {
    blew_up_at: Option<usize>,
    peak_speed: f32,
    peak_lean: f32,
    peak_tilt_deg: f32,
    final_pen_max: f32,
    /// Largest contact penetration seen while the scene was genuinely settled (past frame 60,
    /// this frame's `max|v|` under 0.1).
    ///
    /// This is the channel nothing else covers. Every other number here is about whether the
    /// scene stays *stable*; a build that simply lets everything sink 30% deeper and then sits
    /// there is perfectly stable, perfectly quiet, converges fine, and is wrong. Unlike the
    /// stacked-box vertical-gap formula in `soak_and_golden.rs` this reads the engine's own
    /// contact depths, so it works on a block, a pile or a chain rather than only on a column.
    resting_pen: f32,
    /// The blow-up frame recomputed while ignoring sleeping bodies. If this ever differs from
    /// [`blew_up_at`](Self::blew_up_at) then the detector is reading the sleep system — see
    /// [`Frame::max_speed_awake`].
    blew_up_at_awake: Option<usize>,
    /// Lean and tilt at the frame the detector tripped, so a topple can be told from a twitch.
    trip_lean: f32,
    trip_tilt_deg: f32,
}

fn run(
    world: &mut PhysicsWorld,
    bodies: &[usize],
    origins: &[Vec3],
    frames: usize,
    vel_threshold: f32,
) -> Run {
    let mut r = Run {
        blew_up_at: None,
        peak_speed: 0.0,
        peak_lean: 0.0,
        peak_tilt_deg: 0.0,
        final_pen_max: 0.0,
        resting_pen: 0.0,
        blew_up_at_awake: None,
        trip_lean: 0.0,
        trip_tilt_deg: 0.0,
    };
    for f in 0..frames {
        world.step(DT).ok();
        let o = observe(world, bodies, origins);
        if !o.max_speed.is_finite() {
            r.blew_up_at.get_or_insert(f);
            r.peak_speed = f32::INFINITY;
            break;
        }
        r.peak_speed = r.peak_speed.max(o.max_speed);
        r.peak_lean = r.peak_lean.max(o.lean);
        r.peak_tilt_deg = r.peak_tilt_deg.max(o.tilt.to_degrees());
        r.final_pen_max = o.max_penetration;
        // Settled only, and only before any collapse: a toppling scene's overlaps are the
        // collapse, not the resting depth.
        if f > 60 && o.max_speed < 0.1 && r.blew_up_at.is_none() {
            r.resting_pen = r.resting_pen.max(o.max_penetration);
        }
        if r.blew_up_at.is_none() && o.max_speed >= vel_threshold {
            r.blew_up_at = Some(f);
            r.trip_lean = o.lean;
            r.trip_tilt_deg = o.tilt.to_degrees();
        }
        if r.blew_up_at_awake.is_none() && o.max_speed_awake >= vel_threshold {
            r.blew_up_at_awake = Some(f);
        }
    }
    r
}

/// How much of `soak_resting_stacks_stay_bounded`'s green is the solver, and how much is one
/// lucky float realisation?
///
/// The only difference between the tower this file first measured and the soak's tower was the
/// static ground's half-extent — 200 m (copied from `benches/step_bench.rs`) versus the soak's
/// 20 m. The ground is static and its top face sits at y=0 in both cases, so nothing about the
/// physics being asked for changes; only the floating-point detail of clipping a contact against
/// a bigger face does. If that alone moves a tower from "bounded for 1500 frames" to "buckled at
/// ~1140", then a pass/fail-on-blow-up gate is measuring noise as much as quality — which is a
/// constraint on how the sweep-throttling gate may be built.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn ground_extent_flips_the_blow_up() {
    eprintln!("\n=== blow-up frame vs static ground half-extent (1500 frames, |v| bound 0.5) ===");
    eprintln!(
        "{:>3}  {:>10}  {:>12}  {:>10}  {:>10}  {:>12}",
        "N", "ground", "blew_up_at", "peak|v|", "peak_lean", "peak_tilt_deg"
    );
    for n in [16usize, 24, 32] {
        for ground_half in [20.0f32, 50.0, 100.0, 200.0] {
            let (mut world, bodies, origins) = scene_tower_on_ground(n, ground_half);
            let r = run(&mut world, &bodies, &origins, 1500, 0.5);
            eprintln!(
                "{:>3}  {:>10.0}  {:>12}  {:>10.3}  {:>10.4}  {:>12.3}",
                n,
                ground_half,
                match r.blew_up_at {
                    Some(f) => f.to_string(),
                    None => "-".to_string(),
                },
                r.peak_speed,
                r.peak_lean,
                r.peak_tilt_deg,
            );
        }
    }
}

/// Does the buckling ramp show up in anything a per-step residual can see?
///
/// Run with:
/// `cargo test -p gizmo-physics-rigid --release --test solver_quality measure_ -- --ignored --nocapture`
#[test]
#[ignore = "measurement, not a gate — prints a trace"]
fn measure_tower_buckling_channels() {
    for n in [16usize, 24, 32] {
        let (mut world, bodies, origins) = scene_tower(n);
        trace(&format!("tower N={n}"), &mut world, &bodies, &origins, 1500);
    }
}

/// What does the floating raft actually do over time, and does anything settle?
#[test]
#[ignore = "measurement, not a gate — prints a trace"]
fn measure_raft_behaviour() {
    for n in [64u32, 256] {
        let (mut world, bodies, origins) = scene_raft(n);
        trace(&format!("raft N={n}"), &mut world, &bodies, &origins, 300);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The sweep ladder
// ─────────────────────────────────────────────────────────────────────────────
//
// `adaptive_iterations = false` is what makes this possible at all: with it on, a deep island's
// sweep count is `max(iterations, max(28, 1.5·depth))`, so `iterations` can only raise it and
// the interesting half of the ladder — below the floor — is unreachable.

/// Sweep counts to walk. 20 is the configured default, 28 the adaptive floor, 46 what the
/// N=1024 raft actually gets, 96 the cap.
const LADDER: [usize; 8] = [1, 4, 8, 16, 20, 28, 46, 96];

fn solver_with_exact_sweeps(world: &mut PhysicsWorld, sweeps: usize) {
    world.solver.adaptive_iterations = false;
    world.solver.iterations = sweeps;
}

/// Are the tower's sweeps earned? Walk the sweep count down and watch the stack.
///
/// Each (N, sweeps) cell is run over three static ground sizes rather than one. That is not
/// belt-and-braces: `ground_extent_flips_the_blow_up` shows a single trajectory's pass/fail is
/// flippable by a float perturbation with no physical content, so one run cannot distinguish
/// "this sweep count is unsafe" from "this seed was unlucky". Three is not a real ensemble
/// either — it is the smallest number that stops a single flip from reading as a trend.
#[test]
#[ignore = "measurement, not a gate — prints a table (~3 min)"]
fn sweep_ladder_tower() {
    const GROUNDS: [f32; 3] = [20.0, 100.0, 200.0];
    eprintln!("\n=== tower: quality vs sweep count (1500 frames, |v| bound 0.5, 3 grounds) ===");
    eprintln!(
        "{:>3}  {:>7}  {:>9}  {:>12}  {:>11}  {:>11}  {:>11}",
        "N", "sweeps", "blew/3", "earliest", "worst|v|", "worst_lean", "worst_pen"
    );
    for n in [16usize, 24, 32] {
        for sweeps in LADDER {
            let mut blew = 0;
            let mut earliest: Option<usize> = None;
            let (mut worst_v, mut worst_lean, mut worst_pen) = (0.0f32, 0.0f32, 0.0f32);
            for g in GROUNDS {
                let (mut world, bodies, origins) = scene_tower_on_ground(n, g);
                solver_with_exact_sweeps(&mut world, sweeps);
                let r = run(&mut world, &bodies, &origins, 1500, 0.5);
                if let Some(f) = r.blew_up_at {
                    blew += 1;
                    earliest = Some(earliest.map_or(f, |e: usize| e.min(f)));
                }
                worst_v = worst_v.max(r.peak_speed);
                worst_lean = worst_lean.max(r.peak_lean);
                worst_pen = worst_pen.max(r.final_pen_max);
            }
            eprintln!(
                "{:>3}  {:>7}  {:>9}  {:>12}  {:>11.3}  {:>11.4}  {:>11.6}",
                n,
                sweeps,
                blew,
                match earliest {
                    Some(f) => f.to_string(),
                    None => "-".to_string(),
                },
                worst_v,
                worst_lean,
                worst_pen,
            );
        }
    }
}

/// Are the raft's sweeps earned?
///
/// The raft is not a settling scene — `measure_raft_behaviour` shows it is an explosion: the
/// lattice starts 0.1 m overlapped on every axis, the solver depenetrates it by launching
/// everything outward, and by frame ~215 there are no contacts left. So "does it stay stable"
/// is not the question. The question a depenetration transient can be asked is how much
/// KINETIC ENERGY the solver invents to resolve the overlap, and how fast the overlap goes
/// away. Energy from nothing is the defect; the TGS bias injects it deliberately and the relax
/// pass is supposed to take it back out, so this reads how well that cycle closes.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn sweep_ladder_raft() {
    eprintln!("\n=== raft: depenetration quality vs sweep count (zero gravity, KE from nothing) ===");
    eprintln!(
        "{:>4}  {:>7}  {:>12}  {:>12}  {:>11}  {:>11}  {:>10}",
        "N", "sweeps", "KE@1", "KE@30", "pen@1", "pen@30", "peak|v|"
    );
    for n in [64u32, 256] {
        for sweeps in LADDER {
            let (mut world, bodies, origins) = scene_raft(n);
            solver_with_exact_sweeps(&mut world, sweeps);

            world.step(DT).ok();
            let first = observe(&world, &bodies, &origins);

            let mut peak_v = first.max_speed;
            let mut last = first;
            for _ in 1..30 {
                world.step(DT).ok();
                last = observe(&world, &bodies, &origins);
                peak_v = peak_v.max(last.max_speed);
            }
            eprintln!(
                "{:>4}  {:>7}  {:>12.2}  {:>12.2}  {:>11.6}  {:>11.6}  {:>10.3}",
                n, sweeps, first.energy, last.energy, first.max_penetration, last.max_penetration, peak_v,
            );
        }
    }
}

/// The measurement the whole question turns on: on the two scenes whose correct answer is
/// exactly "nothing moves", how few sweeps still deliver it?
///
/// Everything reported here is invented by the solver — the ideal column reads 0.000 all the
/// way down — so a row is judged against zero, not against the current build.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn sweep_ladder_at_rest() {
    eprintln!("\n=== scenes whose correct answer is EXACTLY zero motion (600 frames) ===");
    eprintln!(
        "{:>26}  {:>7}  {:>10}  {:>10}  {:>11}  {:>10}  {:>8}",
        "scene", "sweeps", "peak|v|", "drift", "tilt(deg)", "KE_end", "asleep"
    );
    for (label, n) in [
        ("floating lattice n=64", 64u32),
        ("floating lattice n=256", 256),
    ] {
        for sweeps in LADDER {
            let (mut world, bodies, origins) = scene_floating_lattice(n);
            solver_with_exact_sweeps(&mut world, sweeps);
            report_at_rest(label, sweeps, &mut world, &bodies, &origins, 600);
        }
    }
    for (label, side, height) in [("crate pile 4x6x4", 4u32, 6u32), ("crate pile 6x6x6", 6, 6)] {
        for sweeps in LADDER {
            let (mut world, bodies, origins) = scene_crate_pile(side, height);
            solver_with_exact_sweeps(&mut world, sweeps);
            report_at_rest(label, sweeps, &mut world, &bodies, &origins, 600);
        }
    }
}

fn report_at_rest(
    label: &str,
    sweeps: usize,
    world: &mut PhysicsWorld,
    bodies: &[usize],
    origins: &[Vec3],
    frames: usize,
) {
    let mut peak_v = 0.0f32;
    let mut peak_tilt = 0.0f32;
    let mut drift = 0.0f32;
    let mut last = Frame::default();
    for _ in 0..frames {
        world.step(DT).ok();
        last = observe(world, bodies, origins);
        if !last.max_speed.is_finite() {
            break;
        }
        peak_v = peak_v.max(last.max_speed);
        peak_tilt = peak_tilt.max(last.tilt.to_degrees());
    }
    // Total displacement from the starting pose, not just the horizontal lean: a lattice that
    // slowly inflates is as wrong as one that leans.
    for (&i, &origin) in bodies.iter().zip(origins) {
        let d = world.transforms[i].position - origin;
        if d.is_finite() {
            drift = drift.max(d.length());
        }
    }
    // KE alone, with gravitational potential removed, so the number is the energy the solver
    // invented rather than the constant a pile's height contributes.
    let ke: f32 = bodies
        .iter()
        .map(|&i| {
            let v = &world.velocities[i];
            let rb = &world.rigid_bodies[i];
            0.5 * rb.mass * v.linear.length_squared()
                + 0.5 * rb.local_inertia.dot(v.angular * v.angular)
        })
        .sum();
    eprintln!(
        "{:>26}  {:>7}  {:>10.4}  {:>10.4}  {:>11.4}  {:>10.4}  {:>8}",
        label, sweeps, peak_v, drift, peak_tilt, ke, last.asleep,
    );
}

/// The loaded anchor-free chain: two conservation laws that hold exactly, walked down the
/// sweep ladder. `|p|` must stay 0 and `KE` must never exceed the 1.0 J the two end boxes
/// started with.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn sweep_ladder_free_chain() {
    eprintln!("\n=== compressed free chain: conservation vs sweep count (1800 frames) ===");
    eprintln!(
        "{:>4}  {:>7}  {:>12}  {:>10}  {:>10}  {:>14}",
        "n", "sweeps", "max|p|", "max_KE", "KE_end", "frames_to_rest"
    );
    for n in [8usize, 24, 32] {
        for sweeps in LADDER {
            let (mut world, bodies, _) = scene_compressed_free_chain(n);
            solver_with_exact_sweeps(&mut world, sweeps);
            let c = chain_conservation(&mut world, &bodies, 1800);
            eprintln!(
                "{:>4}  {:>7}  {:>12.6}  {:>10.4}  {:>10.4}  {:>14}",
                n,
                sweeps,
                c.max_momentum,
                c.max_ke,
                c.ke_end,
                match c.frames_to_rest {
                    Some(f) => f.to_string(),
                    None => "never".to_string(),
                }
            );
        }
    }
}

/// What the free chain conserves, and how long it takes to stop.
#[derive(Debug, Clone, Copy)]
struct Conservation {
    /// Largest `|Σ m·v|` over the run. Starts at exactly zero and has no external force acting
    /// on it, so every unit of this is solver-invented momentum.
    max_momentum: f32,
    /// Largest kinetic energy over the run. There is no energy source, so this may never exceed
    /// what the two end boxes were given.
    max_ke: f32,
    /// Kinetic energy at the end of the run.
    ke_end: f32,
    /// First frame after which the chain stays below the engine's own resting threshold for the
    /// remainder of the run. `None` means it never settled.
    ///
    /// This is the scalar worth gating on. Its ideal is not a number read off the current build:
    /// restitution is 0 and nothing feeds the chain, so it *must* come to rest, and how long
    /// that takes is exactly what a caller who stacks crates cares about. It is also monotone in
    /// the sweep count, unlike the tower's blow-up frame.
    frames_to_rest: Option<usize>,
    /// Whether any body was already asleep at the frame rest was declared.
    ///
    /// Without this the metric above is gameable, and not hypothetically: a body sleeps after 60
    /// consecutive qualifying substeps — 0.25 s, or 15 outer frames — below 0.05 m/s, and a
    /// sleeping body's velocity reads as zero whether or not the solver earned it. So *anything*
    /// that makes bodies sleep sooner (lowering the frame requirement, raising the speed
    /// threshold) would show up as the chain settling faster, with the solver unchanged.
    /// Settling by dissipation and settling by freezing have to be distinguishable, or the gate
    /// measures the sleep system.
    slept_before_rest: bool,
}

/// The engine's own sleep velocity threshold; a body below it is a candidate for sleeping.
const REST_SPEED: f32 = 0.05;

fn chain_conservation(world: &mut PhysicsWorld, bodies: &[usize], frames: usize) -> Conservation {
    let mut c = Conservation {
        max_momentum: 0.0,
        max_ke: 0.0,
        ke_end: 0.0,
        frames_to_rest: None,
        slept_before_rest: false,
    };
    // Tracks the last frame the chain was still moving; the settle frame is one past it.
    let mut last_moving: Option<usize> = None;
    let mut first_sleep: Option<usize> = None;
    for f in 0..frames {
        world.step(DT).ok();
        let (p, ke) = momentum_and_ke(world, bodies);
        if !p.is_finite() || !ke.is_finite() {
            c.max_momentum = f32::INFINITY;
            c.max_ke = f32::INFINITY;
            return c;
        }
        c.max_momentum = c.max_momentum.max(p.length());
        c.max_ke = c.max_ke.max(ke);
        c.ke_end = ke;

        if first_sleep.is_none() && bodies.iter().any(|&i| world.rigid_bodies[i].is_sleeping) {
            first_sleep = Some(f);
        }
        let moving = bodies.iter().any(|&i| {
            let v = &world.velocities[i];
            v.linear.length() + v.angular.length() >= REST_SPEED
        });
        if moving {
            last_moving = Some(f);
        }
    }
    c.frames_to_rest = match last_moving {
        Some(f) if f + 1 < frames => Some(f + 1),
        Some(_) => None,
        None => Some(0),
    };
    c.slept_before_rest = match (first_sleep, c.frames_to_rest) {
        (Some(s), Some(rest)) => s < rest,
        _ => false,
    };
    c
}

/// Is the flat floor of 28 sweeps earned by a *shallow* island, or only by a tall one?
///
/// `sweep_ladder_tower` shows the floor is calibrated almost exactly right for a 24-high column:
/// 20 sweeps buckles 2 of 3 grounds, 28 buckles none. But the policy applies that same 28 to
/// every island of depth ≥ 5, and a realistic game pile is 6 crates high, not 24. If depth 6
/// converges at 8 sweeps and holds there over a long horizon, the floor is over-provisioned by
/// 3.5× on by far the most common shape — a much better-founded target than the benchmark raft,
/// whose quality question turned out to be unanswerable.
///
/// 3000 frames, not 600: this repository has already shipped a soak green that was hiding an
/// explosion at frame ~853, and `sweep_ladder_tower` finds first blow-ups as late as frame 147.
#[test]
#[ignore = "measurement, not a gate — long (~5 min)"]
fn is_the_28_floor_earned_by_a_shallow_pile() {
    const GROUNDS: [f32; 3] = [20.0, 100.0, 200.0];
    eprintln!("\n=== does a depth-6 pile need the 28-sweep floor? (3000 frames, 3 grounds) ===");
    eprintln!(
        "{:>18}  {:>7}  {:>9}  {:>12}  {:>11}  {:>11}",
        "scene", "sweeps", "blew/3", "earliest", "worst|v|", "worst_drift"
    );
    for sweeps in [4usize, 8, 16, 28] {
        for (label, side, height) in [("pile 4x6x4", 4u32, 6u32), ("pile 4x12x4", 4, 12)] {
            let mut blew = 0;
            let mut earliest: Option<usize> = None;
            let (mut worst_v, mut worst_lean) = (0.0f32, 0.0f32);
            for g in GROUNDS {
                let (mut world, bodies, origins) = scene_crate_pile(side, height);
                // Rebuild on the requested ground: scene_crate_pile uses the default.
                world.colliders[0] = Collider::box_collider(Vec3::new(g, 1.0, g));
                solver_with_exact_sweeps(&mut world, sweeps);
                let r = run(&mut world, &bodies, &origins, 3000, 0.5);
                if let Some(f) = r.blew_up_at {
                    blew += 1;
                    earliest = Some(earliest.map_or(f, |e: usize| e.min(f)));
                }
                worst_v = worst_v.max(r.peak_speed);
                worst_lean = worst_lean.max(r.peak_lean);
            }
            eprintln!(
                "{:>18}  {:>7}  {:>9}  {:>12}  {:>11.3}  {:>11.4}",
                label,
                sweeps,
                blew,
                match earliest {
                    Some(f) => f.to_string(),
                    None => "-".to_string(),
                },
                worst_v,
                worst_lean,
            );
        }
    }
}

/// `is_the_28_floor_earned_by_a_shallow_pile` reported that a 4×12×4 crate block collapses on
/// all three grounds at every sweep count tried, including the 28 the default configuration
/// actually gives it. That is a claim about the shipped engine, not about the sweep policy, so
/// it gets verified separately and from more than one angle before it is believed:
///
///  - with the genuine default solver rather than a forced sweep count, in case forcing the
///    count is itself the difference;
///  - with a 2 cm lateral gap between the columns, so the block's side-by-side boxes are not
///    all in marginal exactly-touching contact — if the gap fixes it, the defect is in
///    degenerate face contacts, not in stack stability;
///  - as a trace, so a settling transient tripping the 0.5 threshold cannot be mistaken for a
///    collapse.
#[test]
#[ignore = "measurement, not a gate — long (~4 min)"]
fn verify_wide_pile_collapse() {
    for (label, spacing) in [("exact contact", 1.0f32), ("2cm lateral gap", 1.02)] {
        let (mut world, bodies, origins) = scene_crate_pile_spaced(4, 12, spacing);
        eprintln!(
            "\n--- 4x12x4 crate block, {label}, DEFAULT solver (adaptive on) — sweeps it \
             actually gets: depth-driven ---"
        );
        trace(label, &mut world, &bodies, &origins, 3000);
    }
}

/// Is the wide block's collapse a sweep-count problem at all?
///
/// If it survives at 46 or 96 sweeps, the adaptive policy is UNDER-provisioning a shape it was
/// never calibrated for, and the sweep question has a second half nobody has looked at. If it
/// collapses at 96 too, sweeps are not the lever and this is the known buckling instability
/// showing up at a height the documentation calls safe.
///
/// **Do not read this run on its own — it uses ONE ground and that is not enough.** It reports
/// 96 sweeps saving the block, which is true for this shape and false as a general conclusion:
/// `wide_block_collapse_per_ground` runs two grounds across widths 2, 3 and 4 and finds 96
/// sweeps rescuing only the 4-wide one. See `height_12_stacks_stay_standing` for what the full
/// data supports.
#[test]
#[ignore = "measurement, not a gate"]
fn can_sweeps_save_the_wide_pile() {
    eprintln!("\n=== 4x12x4 crate block vs sweep count (3000 frames, ground 20) ===");
    eprintln!("{:>7}  {:>12}  {:>11}  {:>11}", "sweeps", "blew_up_at", "peak|v|", "peak_lean");
    for sweeps in [28usize, 46, 96] {
        let (mut world, bodies, origins) = scene_crate_pile(4, 12);
        solver_with_exact_sweeps(&mut world, sweeps);
        let r = run(&mut world, &bodies, &origins, 3000, 0.5);
        eprintln!(
            "{:>7}  {:>12}  {:>11.3}  {:>11.4}",
            sweeps,
            match r.blew_up_at {
                Some(f) => f.to_string(),
                None => "-".to_string(),
            },
            r.peak_speed,
            r.peak_lean
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// THE GATES
// ─────────────────────────────────────────────────────────────────────────────
//
// These four run in the ordinary test suite. Between them they are what a change to the sweep
// policy has to pass. Each threshold below is anchored to a physical statement, not to a number
// read off the current build — the project's own history with blessed thresholds is the reason.

/// The chain's initial kinetic energy: two 1 kg boxes at 1 m/s.
const CHAIN_INITIAL_KE: f32 = 1.0;

/// Two laws that hold exactly on the compressed free chain, so the bar is physics rather than a
/// previous measurement.
///
/// The chain starts with zero net momentum and has no external force on it, so its momentum
/// must stay zero; it has no energy source, so its kinetic energy may never exceed what the two
/// end boxes were given; and its collisions are restitution-0 with friction, so it must come to
/// rest rather than ringing forever.
///
/// Anchoring the bounds: the momentum bound is 1% of the 1 kg·m/s each end box carries — a
/// solver leaking more than that is losing a hundredth of the scene's momentum to nothing.
/// The rest deadline is one second of simulated time, against a chain that in practice stops in
/// 3–4 frames. Both are far looser than what the engine delivers, deliberately: they are here
/// to catch a change that breaks the physics, and `sweep_throttling_is_visible_to_this_file`
/// below is what keeps them from being loose enough to catch nothing.
#[test]
fn free_chain_conserves_momentum_and_comes_to_rest() {
    for n in [8usize, 24, 32] {
        let (mut world, bodies, _) = scene_compressed_free_chain(n);
        let c = chain_conservation(&mut world, &bodies, 600);
        assert!(
            c.max_momentum < 1e-2,
            "n={n}: solver invented momentum in a closed system \
             (|p| reached {:.6}, must stay 0): {c:?}",
            c.max_momentum
        );
        assert!(
            c.max_ke <= CHAIN_INITIAL_KE * 1.05,
            "n={n}: solver created energy from nothing \
             (KE reached {:.4} against {CHAIN_INITIAL_KE:.4} available): {c:?}",
            c.max_ke
        );
        assert!(
            matches!(c.frames_to_rest, Some(f) if f <= 60),
            "n={n}: a restitution-0 chain with friction did not come to rest within a second \
             of simulated time: {c:?}"
        );
        assert!(
            !c.slept_before_rest,
            "n={n}: the chain was recorded as at rest while bodies were already asleep, so the \
             reading is the sleep system rather than the solver — see Conservation::\
             slept_before_rest: {c:?}"
        );
    }
}

/// The gate on the gates: this file must be able to SEE a sweep-count cut.
///
/// Every other assertion here is only worth its runtime if throttling the solver actually moves
/// the numbers. It would be easy for that to stop being true without anyone noticing — raise the
/// sleep threshold, change what counts as at rest, alter the chain scene until it settles
/// regardless — and then the suite would wave through exactly the change it exists to catch.
///
/// So: starve the chain to 4 sweeps and require the damage to be visible. If this test ever
/// fails, the correct reading is not "throttling became safe", it is "this file went blind and
/// must not be trusted until it is fixed".
#[test]
fn sweep_throttling_is_visible_to_this_file() {
    let n = 32;

    let (mut default_world, bodies, _) = scene_compressed_free_chain(n);
    let good = chain_conservation(&mut default_world, &bodies, 600);

    let (mut starved_world, bodies, _) = scene_compressed_free_chain(n);
    solver_with_exact_sweeps(&mut starved_world, 4);
    let bad = chain_conservation(&mut starved_world, &bodies, 600);

    let good_rest = good.frames_to_rest.expect("default config must settle the chain");
    match bad.frames_to_rest {
        // Either it never settles inside the window, or it takes far longer. Both are visible;
        // what would be invisible is settling just as fast, and that is the failure.
        None => {}
        Some(bad_rest) => assert!(
            bad_rest > good_rest * 10,
            "starving the solver from the default sweep count to 4 barely changed how long the \
             chain took to settle ({good_rest} -> {bad_rest} frames). This file can no longer \
             see a sweep cut, so its other assertions are not protecting anything.\n  \
             default: {good:?}\n  starved: {bad:?}"
        ),
    }
    assert!(
        bad.max_momentum > good.max_momentum * 10.0,
        "starving the solver barely changed the momentum leak ({:.6} -> {:.6}); this file has \
         gone blind to sweep count.\n  default: {good:?}\n  starved: {bad:?}",
        good.max_momentum,
        bad.max_momentum
    );
}

/// A realistic crate stack — 4×4 wide, 6 high, at exact contact — must still be standing after
/// 25 seconds of simulated time.
///
/// The height is not arbitrary: `docs/ENGINE.md` records ≤~12 as the range game structures
/// actually use, and this sits inside it with margin. The horizon is not arbitrary either —
/// `is_the_28_floor_earned_by_a_shallow_pile` finds this same block collapsing at frame 1150
/// when starved to 4 sweeps, so a 600-frame version of this test would pass a solver that had
/// been throttled into instability.
#[test]
fn realistic_crate_stack_stays_standing() {
    let (mut world, bodies, origins) = scene_crate_pile(4, 6);
    let r = run(&mut world, &bodies, &origins, 1500, 0.5);
    assert!(
        r.blew_up_at.is_none(),
        "a 4x6x4 crate block at rest collapsed on its own: {r:?}"
    );

    // Twelve high, on the 200 m ground — the exact cell that collapsed at frame 1979 before
    // sleep became island-collective, and the cheapest single guard against that returning.
    // The full six-cell version is `height_12_stacks_stay_standing`.
    let (mut tall, tall_bodies, tall_origins) = scene_crate_pile(2, 12);
    tall.colliders[0] = Collider::box_collider(Vec3::new(200.0, 1.0, 200.0));
    let t = run(&mut tall, &tall_bodies, &tall_origins, 3000, 0.5);
    assert!(
        t.blew_up_at.is_none(),
        "a 2x12x2 crate block at rest collapsed on its own: {t:?}"
    );
    assert!(
        r.peak_lean < 0.5,
        "a 4x6x4 crate block at rest leaned {:.3} m — it is on its way over even if max|v| \
         never tripped: {r:?}",
        r.peak_lean
    );
    // The one channel that catches a build which is perfectly stable and quietly wrong. The bar
    // is the solver's own declared tolerance with room to spare: `ConstraintSolver::slop` is
    // 0.005 m, contacts are expected to sit at roughly that depth, and 3× it is 1.5 cm of a
    // 1 m crate — a sixtieth of the box, past which the stack is visibly interpenetrating.
    assert!(
        r.resting_pen < 0.015,
        "a settled 4x6x4 crate block is sinking {:.4} m into itself (3x the 0.005 m solver \
         slop) — stable, quiet, and wrong: {r:?}",
        r.resting_pen
    );
}

/// The gate that says whether the other gates mean anything: do the scenes in this file
/// actually reach the adaptive sweep policy?
///
/// Everything else here asserts a property of a scene. None of them can tell you the scene ever
/// engaged the code under test. That matters because a sweep ladder only sees a throttle keyed
/// on the *sweep count*; a throttle keyed on something else — the sleep counter, island size,
/// how `island_depth` itself is computed — could leave every assertion green while quietly
/// dropping the scenes out of the adaptive branch entirely. This reads
/// `PhysicsMetrics::max_island_depth` and `solver_sweeps` and requires the branch to have fired.
///
/// If it fails, no other result in this file is evidence of anything.
#[test]
fn the_gated_scenes_reach_the_adaptive_policy() {
    // (label, world, the depth the policy needs to see)
    let mut cases: Vec<(&str, PhysicsWorld)> = Vec::new();
    let (mut pile, _, _) = scene_crate_pile(4, 6);
    for _ in 0..90 {
        pile.step(DT).ok(); // let it settle so the island is the resting block, not the drop
    }
    cases.push(("crate pile 4x6x4", pile));

    let (mut chain, _, _) = scene_compressed_free_chain(32);
    for _ in 0..5 {
        // The chain starts fragmented and links up as the compression wave reaches the middle;
        // before that it is a pair of shallow islands and would not test the policy.
        chain.step(DT).ok();
    }
    cases.push(("free chain n=32", chain));

    for (label, mut world) in cases {
        let configured = world.solver.iterations;
        // Woken for the probe. A settled scene sleeps as a whole island, and a fully dormant
        // island is not solved at all — so reading the policy off a settled pile reports depth 0
        // and says nothing about whether the scene reaches the adaptive branch when it IS solved,
        // which is the question. (Before islands slept collectively this did not arise, because a
        // pile never managed to sleep as a unit.)
        for i in 1..world.entities.len() {
            world.rigid_bodies[i].wake_up();
        }
        world.step(DT).ok();
        let (depth, sweeps, islands) = policy_readout(&world);
        assert!(
            depth >= 5,
            "{label}: support depth is {depth}, below the >= 5 the adaptive sweep policy needs \
             to engage — this scene is not testing the policy at all"
        );
        assert!(
            islands > 0,
            "{label}: no islands were solved, so the scene exercised nothing"
        );
        let per_island = sweeps as f32 / islands as f32;
        assert!(
            per_island > configured as f32,
            "{label}: averaged {per_island:.1} sweeps per island against a configured \
             {configured} — the adaptive branch did not raise the count, so this scene is \
             running the ordinary path and the gates built on it prove nothing about the policy"
        );
    }
}

/// What sweep count does each scene in this file actually receive? The number the whole of
/// Phase C is about, read directly instead of inferred from a `state_hash` ladder.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn audit_effective_sweeps_per_scene() {
    eprintln!("\n=== what the adaptive policy hands each scene (one frame = 4 substeps) ===");
    eprintln!(
        "{:>26}  {:>6}  {:>8}  {:>8}  {:>12}",
        "scene", "depth", "islands", "sweeps", "per island"
    );
    let row = |label: &str, world: &PhysicsWorld| {
        let (depth, sweeps, islands) = policy_readout(world);
        eprintln!(
            "{:>26}  {:>6}  {:>8}  {:>8}  {:>12.1}",
            label,
            depth,
            islands,
            sweeps,
            if islands > 0 { sweeps as f32 / islands as f32 } else { 0.0 }
        );
    };

    for (label, n) in [("tower N=16", 16usize), ("tower N=24", 24), ("tower N=32", 32)] {
        let (mut w, _, _) = scene_tower(n);
        for _ in 0..90 {
            w.step(DT).ok();
        }
        w.step(DT).ok();
        row(label, &w);
    }
    for (label, side, height) in [("pile 4x6x4", 4u32, 6u32), ("pile 4x12x4", 4, 12)] {
        let (mut w, _, _) = scene_crate_pile(side, height);
        for _ in 0..90 {
            w.step(DT).ok();
        }
        w.step(DT).ok();
        row(label, &w);
    }
    for n in [64u32, 256] {
        let (mut w, _, _) = scene_raft(n);
        w.step(DT).ok();
        row(&format!("bench raft N={n} (frame 1)"), &w);
    }
    for n in [64u32, 256] {
        let (mut w, _, _) = scene_floating_lattice(n);
        w.step(DT).ok();
        row(&format!("exact-contact lattice N={n}"), &w);
    }
    for n in [8usize, 24, 32] {
        let (mut w, _, _) = scene_compressed_free_chain(n);
        for f in 0..6 {
            w.step(DT).ok();
            row(&format!("free chain n={n} @frame {f}"), &w);
        }
    }
}

/// The decisive experiment for the policy: at a FIXED height, does the required sweep count
/// depend on the block's WIDTH?
///
/// Support depth is the policy's only input, and depth is the same 12 for a 1-wide column and a
/// 4-wide block of the same height — `audit_effective_sweeps_per_scene` confirms both are handed
/// 28 sweeps. `can_sweeps_save_the_wide_pile` shows the 4-wide block needs 96. If the requirement
/// climbs with width while depth holds still, then depth cannot be the input, and no rule
/// rewritten in terms of depth alone — anchoring, chain-versus-lattice, eccentricity ratios —
/// can be correct. That would settle what the previous session was hunting for, in the negative.
///
/// Reported as the smallest sweep count on the ladder that survived both grounds.
///
/// **Measured, and it does not decide what it was built to decide.** Widths 2 and 3 survive at
/// no count on the ladder while width 4 survives at 96 — non-monotone in width, which means the
/// shapes are not stable enough at height 12 for a "required sweep count" to exist. The
/// follow-ups are `wide_block_collapse_per_ground` and `height_12_stacks_stay_standing`. The
/// question this was meant to answer — whether support depth can be the policy's input — is
/// still open, and needs a scene class that stands reliably in the first place.
#[test]
#[ignore = "measurement, not a gate — long (~10 min)"]
fn does_required_sweep_count_depend_on_width() {
    const GROUNDS: [f32; 2] = [20.0, 200.0];
    const SWEEPS: [usize; 5] = [8, 16, 28, 46, 96];
    eprintln!("\n=== required sweeps vs block width at fixed height (3000 frames, 2 grounds) ===");
    eprintln!(
        "{:>16}  {:>6}  {:>7}  {:>8}  {:>28}",
        "block", "depth", "bodies", "policy", "survives from"
    );
    for (side, height) in [(1u32, 12u32), (2, 12), (3, 12), (4, 12)] {
        // What the policy hands this shape, read rather than derived.
        let (mut probe, _, _) = scene_crate_pile(side, height);
        for _ in 0..90 {
            probe.step(DT).ok();
        }
        probe.step(DT).ok();
        let (depth, sweeps, islands) = policy_readout(&probe);
        let policy = if islands > 0 { sweeps as f32 / islands as f32 } else { 0.0 };

        let mut survives_from: Option<usize> = None;
        for s in SWEEPS {
            let all_ok = GROUNDS.iter().all(|&g| {
                let (mut world, bodies, origins) = scene_crate_pile(side, height);
                world.colliders[0] = Collider::box_collider(Vec3::new(g, 1.0, g));
                solver_with_exact_sweeps(&mut world, s);
                run(&mut world, &bodies, &origins, 3000, 0.5).blew_up_at.is_none()
            });
            if all_ok {
                survives_from = Some(s);
                break;
            }
        }
        eprintln!(
            "{:>16}  {:>6}  {:>7}  {:>8.0}  {:>28}",
            format!("{side}x{height}x{side}"),
            depth,
            side * side * height,
            policy,
            match survives_from {
                Some(s) => format!("{s} sweeps"),
                None => "never (not even 96)".to_string(),
            }
        );
    }
}

/// Follow-up to `does_required_sweep_count_depend_on_width`, which reported that 2- and 3-wide
/// 12-high blocks survive at NO sweep count on the ladder while the 4-wide one survives at 96.
/// Non-monotonic in width, which given `ground_extent_flips_the_blow_up` is exactly what one
/// unlucky ground would look like. So: report each ground separately, with its blow-up frame,
/// instead of the all-grounds-survived summary.
///
/// What this decides: whether the wide-block collapse is the sweep policy under-spending (fixable
/// by spending more) or a stability gap sweeps do not close (not a policy question at all).
#[test]
#[ignore = "measurement, not a gate — long (~6 min)"]
fn wide_block_collapse_per_ground() {
    eprintln!("\n=== wide blocks, each ground reported separately (3000 frames) ===");
    eprintln!(
        "{:>16}  {:>7}  {:>8}  {:>12}  {:>11}",
        "block", "sweeps", "ground", "blew_up_at", "peak|v|"
    );
    for (side, height) in [(2u32, 6u32), (3, 6), (2, 12), (3, 12), (4, 12)] {
        for sweeps in [28usize, 96] {
            for g in [20.0f32, 200.0] {
                let (mut world, bodies, origins) = scene_crate_pile(side, height);
                world.colliders[0] = Collider::box_collider(Vec3::new(g, 1.0, g));
                solver_with_exact_sweeps(&mut world, sweeps);
                let r = run(&mut world, &bodies, &origins, 3000, 0.5);
                eprintln!(
                    "{:>16}  {:>7}  {:>8.0}  {:>12}  {:>11.3}",
                    format!("{side}x{height}x{side}"),
                    sweeps,
                    g,
                    match r.blew_up_at {
                        Some(f) => f.to_string(),
                        None => "-".to_string(),
                    },
                    r.peak_speed
                );
            }
        }
    }
}

/// The sharpest form of the wide-block finding: at height 12, a single column stands and two
/// columns do not.
///
/// `does_required_sweep_count_depend_on_width` has 1×12×1 surviving from 8 sweeps while 2×12×2
/// collapses at 28 and at 96 on both grounds. Widening the base is supposed to make a stack more
/// stable, not less, so this either is a real defect or is an artefact of the columns being at
/// exact lateral contact. The lateral-gap arm settles that.
#[test]
#[ignore = "measurement, not a gate"]
fn one_column_stands_and_two_do_not() {
    eprintln!("\n=== height 12, default solver, one column vs two (3000 frames) ===");
    eprintln!(
        "{:>10}  {:>16}  {:>8}  {:>12}  {:>12}  {:>11}  {:>10}  {:>10}",
        "block", "lateral", "ground", "blew_up_at", "awake-only", "peak|v|", "trip_lean", "trip_tilt"
    );
    for (side, spacing, label) in [
        (1u32, 1.0f32, "n/a"),
        (2, 1.0, "exact contact"),
        (2, 1.02, "2cm gap"),
        (2, 1.2, "20cm gap"),
    ] {
        for g in [20.0f32, 200.0] {
            let (mut world, bodies, origins) = scene_crate_pile_spaced(side, 12, spacing);
            world.colliders[0] = Collider::box_collider(Vec3::new(g, 1.0, g));
            let r = run(&mut world, &bodies, &origins, 3000, 0.5);
            eprintln!(
                "{:>10}  {:>16}  {:>8.0}  {:>12}  {:>12}  {:>11.3}  {:>10.4}  {:>10.3}",
                format!("{side}x12x{side}"),
                label,
                g,
                match r.blew_up_at {
                    Some(f) => f.to_string(),
                    None => "-".to_string(),
                },
                match r.blew_up_at_awake {
                    Some(f) => f.to_string(),
                    None => "-".to_string(),
                },
                r.peak_speed,
                r.trip_lean,
                r.trip_tilt_deg,
            );
        }
    }
}

/// Does a bigger static ground produce a WORSE contact, or does it only change which way the
/// dice fall?
///
/// `ground_extent_flips_the_blow_up` and `one_column_stands_and_two_do_not` both show the static
/// ground's half-extent deciding whether a stack survives, with everything physical held fixed.
/// The obvious mechanism is precision: contact generation clips the box's face against the
/// ground's, and a 200 m reference face means quantities of order 0.5 get computed as differences
/// of quantities of order 200, which costs roughly `log2(200/0.5) ≈ 9` bits of an f32's 24.
///
/// This tests that WITHOUT simulating: place one box at rest on grounds of different half-extent,
/// advance a single substep, and compare the contact geometry each one produces. The scene is
/// numerically identical apart from the ground's size, and the ground is static, so every
/// difference from the smallest-ground reference is error rather than physics.
///
/// The prediction, if precision is the mechanism: the deviation grows roughly linearly with the
/// half-extent, and it appears in the contact points' LATERAL coordinates — which is what feeds a
/// torque error, which is what seeds buckling. A deviation that stays at zero, or one that appears
/// only in the normal direction, refutes it.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn does_a_bigger_ground_degrade_the_contact() {
    /// One substep's worth of contact geometry for a box placed at `pose`, sorted so two runs
    /// are comparable.
    fn contacts_after_one_substep(
        ground_half: f32,
        pose: (Vec3, gizmo_math::Quat),
    ) -> Vec<(Vec3, Vec3, f32)> {
        let mut world = PhysicsWorld::new();
        add_ground_sized(&mut world, ground_half);
        add_box(
            &mut world,
            1,
            pose.0,
            0.5,
            PhysicsMaterial {
                restitution: 0.0,
                ..Default::default()
            },
        );
        world.transforms[1].rotation = pose.1;
        world.transforms[1].update_local_matrix();
        // Exactly one 1/240 s substep, so the box's own state is still bit-identical across runs
        // and the only thing that differs is the clip against a bigger face.
        world.step_once = true;
        world.step(DT).ok();

        let mut out: Vec<(Vec3, Vec3, f32)> = world
            .collision_events()
            .iter()
            .flat_map(|e| e.contact_points.iter())
            .map(|c| (c.point, c.normal, c.penetration))
            .collect();
        // Sort by position so point order cannot masquerade as a difference.
        out.sort_by(|a, b| {
            (a.0.x, a.0.y, a.0.z)
                .partial_cmp(&(b.0.x, b.0.y, b.0.z))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    // Two poses. The first is exact contact, which is how every scene in this file starts. The
    // second is the SETTLED pose — the box sits a few millimetres into the ground once gravity
    // and the solver have reached steady state, and that is the state a resting stack actually
    // lives in. They turn out to behave completely differently, which is the finding.
    let settled = {
        let mut w = PhysicsWorld::new();
        add_ground_sized(&mut w, 1.0);
        add_box(
            &mut w,
            1,
            Vec3::new(0.0, 0.5, 0.0),
            0.5,
            PhysicsMaterial {
                restitution: 0.0,
                ..Default::default()
            },
        );
        for _ in 0..240 {
            w.step(DT).ok();
        }
        (w.transforms[1].position, w.transforms[1].rotation)
    };

    for (pose_label, pose) in [
        ("exact contact", (Vec3::new(0.0, 0.5, 0.0), gizmo_math::Quat::IDENTITY)),
        ("settled", settled),
    ] {
        let reference = contacts_after_one_substep(1.0, pose);
        eprintln!(
            "\n=== contact geometry vs static ground half-extent — box {pose_label} \
             (1 substep; the scene is identical apart from the ground's size, so every \
             difference is error) ==="
        );
        eprintln!(
            "reference half-extent 1.0: {} point(s) {:?}",
            reference.len(),
            reference.iter().map(|c| c.0).collect::<Vec<_>>()
        );
        eprintln!(
            "{:>12}  {:>8}  {:>14}  {:>14}  {:>14}  {:>34}",
            "ground", "points", "max Δlateral", "max Δnormal", "max Δpen", "points"
        );
        for h in [1.0f32, 5.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0] {
            let got = contacts_after_one_substep(h, pose);
            let pts = format!("{:?}", got.iter().map(|c| c.0).collect::<Vec<_>>());
            let pts = if pts.len() > 34 { format!("{}…", &pts[..33]) } else { pts };
            if got.len() != reference.len() {
                eprintln!(
                    "{:>12.0}  {:>8}  {:>14}  {:>14}  {:>14}  {:>34}",
                    h, got.len(), "(count differs)", "-", "-", pts
                );
                continue;
            }
            let (mut d_lat, mut d_nrm, mut d_pen) = (0.0f32, 0.0f32, 0.0f32);
            for (a, b) in reference.iter().zip(&got) {
                let dp = b.0 - a.0;
                // Lateral = perpendicular to the reference contact normal; that is the component
                // that becomes a lever-arm error and therefore a torque error.
                let along = dp.dot(a.1) * a.1;
                d_lat = d_lat.max((dp - along).length());
                d_nrm = d_nrm.max((b.1 - a.1).length());
                d_pen = d_pen.max((b.2 - a.2).abs());
            }
            eprintln!(
                "{:>12.0}  {:>8}  {:>14.3e}  {:>14.3e}  {:>14.3e}  {:>34}",
                h, got.len(), d_lat, d_nrm, d_pen, pts
            );
        }
    }
}

/// Is the frame-70 collapse localised at a 2 cm gap, or is it a broad band?
///
/// `one_column_stands_and_two_do_not` found a 2×12×2 block collapsing at frame 70 with a 2 cm
/// lateral gap while exact contact and a 20 cm gap both stood — two orders of magnitude faster
/// than every other collapse measured, which suggests a different mechanism from the slow
/// buckling and therefore an easier one to find.
///
/// 2 cm is also exactly `ConstraintSolver::warm_start_match_tolerance`. If the collapse band is
/// narrow and sits on that value, that coincidence is the lead. If the band is broad, or centred
/// somewhere else, it is not.
#[test]
#[ignore = "measurement, not a gate — long (~5 min)"]
fn where_is_the_fast_collapse_band() {
    eprintln!("\n=== 2x12x2, default solver, collapse frame vs lateral gap (3000 frames) ===");
    eprintln!(
        "{:>10}  {:>12}  {:>12}  {:>11}",
        "gap (m)", "ground 20", "ground 200", "note"
    );
    for gap in [0.0f32, 0.005, 0.01, 0.015, 0.02, 0.025, 0.03, 0.05, 0.1, 0.2] {
        let mut cells = Vec::new();
        for g in [20.0f32, 200.0] {
            let (mut world, bodies, origins) = scene_crate_pile_spaced(2, 12, 1.0 + gap);
            world.colliders[0] = Collider::box_collider(Vec3::new(g, 1.0, g));
            let r = run(&mut world, &bodies, &origins, 3000, 0.5);
            cells.push(match r.blew_up_at {
                Some(f) => f.to_string(),
                None => "-".to_string(),
            });
        }
        eprintln!(
            "{:>10.3}  {:>12}  {:>12}  {:>11}",
            gap,
            cells[0],
            cells[1],
            if (gap - 0.02).abs() < 1e-6 { "= warm-start tol" } else { "" }
        );
    }
}

/// How many contact points does a settled interface actually carry, and where are they?
///
/// The block solver's whole premise is that a manifold has up to four coplanar points and that
/// solving them jointly restores the tilt-resisting torque sequential Gauss-Seidel loses. That
/// premise is worth checking rather than assuming: a one-point manifold provides no tilt
/// stiffness at all, however good the block solve is.
///
/// Reported for a settled 2-box stack, so both the ground interface and a box-box interface are
/// visible, across ground sizes.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn how_many_points_does_a_settled_interface_carry() {
    eprintln!("\n=== settled 2-box stack: contact points per interface, last frame ===");
    for h in [1.0f32, 20.0, 200.0] {
        let mut world = PhysicsWorld::new();
        add_ground_sized(&mut world, h);
        let no_bounce = PhysicsMaterial {
            restitution: 0.0,
            ..Default::default()
        };
        add_box(&mut world, 1, Vec3::new(0.0, 0.5, 0.0), 0.5, no_bounce);
        add_box(&mut world, 2, Vec3::new(0.0, 1.5, 0.0), 0.5, no_bounce);
        // Kept awake throughout: a body sleeps after 15 frames below the rest threshold, and a
        // pair with both bodies dormant skips narrowphase entirely, so a settled stack reports
        // no contacts at all. Waking is not a physical change — it only decides whether the
        // island is solved — but it is the difference between measuring the interface and
        // measuring nothing.
        for _ in 0..240 {
            for i in 1..world.entities.len() {
                world.rigid_bodies[i].wake_up();
            }
            world.step(DT).ok();
        }
        for i in 1..world.entities.len() {
            world.rigid_bodies[i].wake_up();
        }
        // One more frame, then report that frame's events. Four substeps run, so each live pair
        // contributes four events; the per-event point count is what matters here.
        world.step(DT).ok();
        eprintln!("\n  ground half-extent {h}:");
        let mut seen: Vec<(u32, u32, usize)> = Vec::new();
        for ev in world.collision_events() {
            let key = (ev.entity_a.id(), ev.entity_b.id(), ev.contact_points.len());
            if !seen.contains(&key) {
                seen.push(key);
                let pts: Vec<String> = ev
                    .contact_points
                    .iter()
                    .map(|c| {
                        format!(
                            "({:+.3},{:+.3},{:+.3} d={:+.4})",
                            c.point.x, c.point.y, c.point.z, c.penetration
                        )
                    })
                    .collect();
                eprintln!(
                    "    pair {}-{}: {} point(s)  {}",
                    key.0,
                    key.1,
                    key.2,
                    pts.join(" ")
                );
            }
        }
    }
}

/// What does a contact manifold look like on the substep it is BORN, as opposed to once it has
/// persisted?
///
/// `how_many_points_does_a_settled_interface_carry` shows a settled interface carrying four
/// corner points, identically at ground half-extent 1, 20 and 200 — so the block solver's premise
/// holds in steady state and the ground size does not degrade it there.
///
/// `does_a_bigger_ground_degrade_the_contact` found something different on a *fresh* pair's first
/// substep: a single point, whose lateral position depends on the other collider's size and
/// slides to the resting box's own edge as the ground grows (z = 0 at half-extent 1, −0.4988 at
/// 200). A one-point manifold carries no tilt-resisting torque, and one placed half a box off
/// centre applies a torque impulse that should not be there.
///
/// This separates the two: it prints every event of the first frame in order, so the birth
/// substep and the three that follow it are visible side by side.
///
/// # Measured, and the mechanism is confirmed in the source
///
/// ```text
///   ground half-extent   birth -> steady   birth point
///   0.60 … 1.50          1 -> 1            ( 0.0000, 0, +0.0000)   <- never recovers
///   2.00                 1 -> 4            (-0.4000, 0,  0.0000)
///   3.00                 1 -> 4            ( 0.0000, 0, -0.4286)
///   5.00                 1 -> 4            ( 0.0000, 0, -0.4545)
///  20.00                 1 -> 4            ( 0.0000, 0, -0.4878)
/// 200.00                 1 -> 4            ( 0.0000, 0, -0.4988)
/// ```
///
/// **Every manifold is born as a single point, at every ground size.** The cause is one line:
/// `gizmo-physics-core/src/narrowphase/contacts.rs`, in `clip_box_box`, rejects a corner with
/// `if signed_depth <= 0.0 { return None }`. A box placed at EXACT contact has all four corners
/// at `signed_depth == 0.0`, so all four are rejected, Sutherland–Hodgman returns empty, the
/// swapped-reference retry returns empty for the same reason, and the pair falls through to the
/// GJK/EPA fallback — which returns ONE contact. Note the asymmetry that gives it away: the
/// lateral slab test right below carries a deliberate `SLAB_TOLERANCE = 1e-3` "to avoid
/// floating-point edge-case rejections", while the depth test has no tolerance at all.
///
/// **And the single point is not at the centre.** Its offset grows with the ground's half-extent
/// and converges on the resting box's own edge — 0.4 of a box at half-extent 2, 0.4988 at 200
/// (the values fit `H/(2H+1)`). A one-point manifold carries no tilt-resisting torque whatever the
/// block solver does with it, and one placed at the edge applies a torque impulse that the
/// geometry does not call for.
///
/// **What this does NOT explain — corrected.** I first wrote that this was the mechanism behind
/// the ground-size sensitivity: bigger ground, bigger off-centre kick, earlier collapse. The
/// offset does grow with the ground; the KICK does not.
/// `does_a_bigger_ground_deliver_a_bigger_birth_kick` measures it. For a unit cube the angular
/// velocity a single normal contact at lateral offset `r` imparts is `6r·Δv/(1 + 6r²)`, which is
/// *maximised* at `r = 1/√6 = 0.408` — exactly where these offsets begin — so pushing the point
/// further out barely changes it. Measured `|Δω|`: 0.028351 at half-extent 2, 0.030458 at 20,
/// 0.030636 at 200, 0.030651 at 1000. Between the two ground sizes that decide a stack's fate
/// that is a **0.58% difference in seed**, and at this repository's measured growth rate (lean
/// doubling about every 100 frames, λ ≈ 0.0069 per frame) a 0.58% seed difference predicts a
/// collapse shifted by `ln(1.0058)/λ ≈ 0.8` frames. The observed shift is at least 672. Off by
/// roughly three orders of magnitude, so this is not the ground-size mechanism.
///
/// What survives is the defect on its own terms, and it is worth fixing: every interface in the
/// engine is born with no tilt stiffness at all and a `Δω ≈ 0.03 rad/s` kick it should not
/// receive, in a scene whose correct answer is that nothing moves. **The ground-size sensitivity
/// remains unexplained.**
///
/// **Below half-extent ~1.5 the interface never recovers**, staying one point forever. Consistent
/// with the same mechanism: at those sizes the GJK point lands at the centre, so it holds the box
/// up with no torque, so the box never sinks, so `signed_depth` never becomes positive and the
/// clip path is never reached again. A crate resting on a small platform therefore has no tilt
/// stiffness at all.
///
/// Every scene in this file and in `soak_and_golden.rs` places its boxes at exact contact, so all
/// of them are born through this path.
#[test]
#[ignore = "measurement, not a gate — prints a trace"]
fn what_does_a_manifold_look_like_when_it_is_born() {
    eprintln!("\n=== points per event, box resting on grounds of increasing half-extent ===");
    eprintln!(
        "{:>10}  {:>12}  {:>34}  {:>26}",
        "ground", "birth / then", "birth point", "steady point count"
    );
    for h in [0.6f32, 0.75, 1.0, 1.5, 2.0, 3.0, 5.0, 20.0, 200.0] {
        let mut world = PhysicsWorld::new();
        add_ground_sized(&mut world, h);
        add_box(
            &mut world,
            1,
            Vec3::new(0.0, 0.5, 0.0),
            0.5,
            PhysicsMaterial {
                restitution: 0.0,
                ..Default::default()
            },
        );
        let mut birth: Option<(usize, Vec3)> = None;
        let mut steady = 0usize;
        for _ in 0..30 {
            world.step(DT).ok();
            for ev in world.collision_events() {
                if birth.is_none() {
                    birth = Some((
                        ev.contact_points.len(),
                        ev.contact_points.iter().next().map(|c| c.point).unwrap_or(Vec3::ZERO),
                    ));
                }
                steady = ev.contact_points.len();
            }
        }
        let (bn, bp) = birth.unwrap_or((0, Vec3::ZERO));
        eprintln!(
            "{:>10.2}  {:>12}  {:>34}  {:>26}",
            h,
            format!("{bn} -> {steady}"),
            format!("({:+.4},{:+.4},{:+.4})", bp.x, bp.y, bp.z),
            steady
        );
    }

    for h in [1.0f32, 200.0] {
        eprintln!("\n=== box dropped onto a ground of half-extent {h}: first 3 frames, every event ===");
        let mut world = PhysicsWorld::new();
        add_ground_sized(&mut world, h);
        add_box(
            &mut world,
            1,
            Vec3::new(0.0, 0.5, 0.0),
            0.5,
            PhysicsMaterial {
                restitution: 0.0,
                ..Default::default()
            },
        );
        for f in 0..3 {
            world.step(DT).ok();
            for (i, ev) in world.collision_events().iter().enumerate() {
                let pts: Vec<String> = ev
                    .contact_points
                    .iter()
                    .map(|c| format!("({:+.4},{:+.4},{:+.4})", c.point.x, c.point.y, c.point.z))
                    .collect();
                eprintln!(
                    "  frame {f} event {i} pair {}-{} {:?}: {} point(s)  {}",
                    ev.entity_a.id(),
                    ev.entity_b.id(),
                    ev.event_type,
                    ev.contact_points.len(),
                    pts.join(" ")
                );
            }
        }
    }
}

/// Does the frame-70 collapse MOVE when `warm_start_match_tolerance` moves?
///
/// `where_is_the_fast_collapse_band` puts the fast collapse in a single cell — gap 0.020 on
/// ground 20, frame 70, with 0.015 and 0.025 both standing 3000 frames — and 0.020 is exactly
/// the default `warm_start_match_tolerance`. That is suggestive and it is also exactly the shape
/// of coincidence this repository has a documented history of believing.
///
/// So this is the falsifier rather than more of the same evidence. Raise the tolerance to 0.05
/// and re-scan the gap.
///
///   - If the fast collapse follows the tolerance to gap ≈ 0.05 and gap 0.020 goes quiet, the
///     warm-start match is the mechanism.
///   - If it stays at 0.020, or vanishes, or appears somewhere unrelated, it is not — and the
///     frame-70 cell is one more draw from the chaotic distribution that
///     `ground_extent_flips_the_blow_up` already showed this scene class has.
///
/// **Measured: REFUTED.** The collapse stays at gap 0.020 and at frame 70 for
/// `warm_start_match_tolerance` of 0.002, 0.02 and 0.05 alike — identical frame, three values
/// spanning 25×. The warm-start match is not in the causal path here, and the fact that 0.02 is
/// both the default tolerance and the interesting gap is a coincidence.
///
/// `max_linear_correction` is also 0.02 and can be ruled out without a run: it is read only at
/// `solver/mod.rs:631` and `:787`, both inside the split-impulse path, which the default
/// configuration does not take (`use_tgs_soft` is on).
///
/// What IS at gap 0.020 remains unexplained. See
/// `is_the_frame_70_event_really_a_collapse` for what the failure actually looks like.
#[test]
#[ignore = "measurement, not a gate — long (~4 min)"]
fn does_the_fast_collapse_follow_the_warm_start_tolerance() {
    eprintln!("\n=== 2x12x2 on ground 20: collapse frame vs (lateral gap, warm-start tolerance) ===");
    eprintln!("{:>10}  {:>14}  {:>14}  {:>14}", "gap (m)", "tol 0.02 (def)", "tol 0.05", "tol 0.002");
    for gap in [0.01f32, 0.015, 0.02, 0.025, 0.03, 0.04, 0.05, 0.06] {
        let mut cells = Vec::new();
        for tol in [0.02f32, 0.05, 0.002] {
            let (mut world, bodies, origins) = scene_crate_pile_spaced(2, 12, 1.0 + gap);
            world.solver.warm_start_match_tolerance = tol;
            let r = run(&mut world, &bodies, &origins, 3000, 0.5);
            cells.push(match r.blew_up_at {
                Some(f) => f.to_string(),
                None => "-".to_string(),
            });
        }
        eprintln!(
            "{:>10.3}  {:>14}  {:>14}  {:>14}",
            gap, cells[0], cells[1], cells[2]
        );
    }
}

/// Is the frame-70 event a collapse at all, or a transient that grazes the 0.5 threshold?
///
/// `run()` records `blew_up_at` at the first frame `max|v|` reaches 0.5, which cannot tell a
/// topple from a settling twitch. Before any more of this session's budget goes into explaining
/// the frame-70 cell, it is worth knowing whether there is anything there to explain.
///
/// # Measured: both, and the distinction matters
///
/// There is a genuine collapse, but it is not at frame 70 — the tower is still standing there and
/// only topples around frame 200-250. Frame 70 is a single-frame velocity transient that trips a
/// first-crossing detector. So "two orders of magnitude faster than every other collapse" was an
/// artefact of the detector; one order is the honest figure.
///
/// What is genuinely different is the SIGNATURE, and this is the useful part:
///
/// ```text
///   frame     max|v|      lean   tilt°   pen_max   pen_mean
///       4    0.05856  0.005243   0.066  0.000457   0.000153
///      31    0.08401  0.012192   1.068  0.009799   0.000391
///      95    0.12302  0.021434   3.179  0.016274   0.000764
///     143    0.45800  0.045185   5.867  0.017525   0.001094
///     207    2.56732  0.437135  26.112  0.147483   0.006310
/// ```
///
/// The tilt climbs from the first frames and `pen_max` reaches three times the 0.005 slop by
/// frame 95. Compare the slow buckling of a 4×12×4 block at exact contact, where `pen_max` sits
/// flat at 0.0055 and everything looks healthy for 1559 frames before a sudden topple. This stack
/// is progressively sinking and leaning from the start, which is a sustained failure rather than
/// a noise-seeded exponential — a different mechanism, and one the observables see clearly from
/// about frame 30 rather than only in hindsight.
#[test]
#[ignore = "measurement, not a gate — prints a trace"]
fn is_the_frame_70_event_really_a_collapse() {
    let (mut world, bodies, origins) = scene_crate_pile_spaced(2, 12, 1.02);
    trace("2x12x2, 2cm gap, ground 20", &mut world, &bodies, &origins, 400);
}

/// Does the off-centre birth contact actually deliver a BIGGER kick on a bigger ground?
///
/// `what_does_a_manifold_look_like_when_it_is_born` shows the birth point's lateral offset
/// growing with the ground's half-extent — 0.400 at 2, 0.4878 at 20, 0.4988 at 200 — and I
/// concluded from that (commit `b47b3b0`) that a bigger ground delivers a bigger off-centre
/// torque and therefore an earlier collapse. That step does not follow, and the algebra says so.
///
/// For a unit cube (`m = 1`, so `I = 2/12` and `inv_I = 6`) taking one substep of gravity
/// (`Δv = 9.81/240 = 0.0409 m/s`), a single normal contact at lateral offset `r` needs impulse
/// `λ = Δv / (1 + 6r²)` and imparts `Δω = 6rλ = 6r·Δv / (1 + 6r²)`. That is **maximised at
/// `r = 1/√6 = 0.408`** — which is where the measured offsets start, and they only move outward
/// from there:
///
/// ```text
///   ground H    birth r    predicted Δω
///        2       0.4000        0.0500
///       20       0.4878        0.0493
///      200       0.4988        0.0491
/// ```
///
/// So the kick is essentially FLAT across the whole measured range, and very slightly *smaller*
/// on the bigger ground. Against a lean that doubles about every 100 frames (λ ≈ 0.0069/frame),
/// a 0.4% difference in seed moves the collapse by `ln(1.004)/λ ≈ 0.6` frames. The measured gap
/// between ground 20 and ground 200 for a 1×12×1 column is at least 672 frames.
///
/// This measures it rather than trusting the algebra: read the box's angular velocity right after
/// the birth substep. If Δω is flat in the ground size, the causal claim in `b47b3b0` is refuted
/// and only the defect itself survives.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn does_a_bigger_ground_deliver_a_bigger_birth_kick() {
    eprintln!("\n=== angular velocity imparted on the substep the contact is born ===");
    eprintln!(
        "{:>10}  {:>14}  {:>14}  {:>14}",
        "ground", "birth |r|", "|Δω| measured", "|Δv| measured"
    );
    for h in [1.0f32, 1.5, 2.0, 3.0, 5.0, 20.0, 100.0, 200.0, 1000.0] {
        let mut world = PhysicsWorld::new();
        add_ground_sized(&mut world, h);
        add_box(
            &mut world,
            1,
            Vec3::new(0.0, 0.5, 0.0),
            0.5,
            PhysicsMaterial {
                restitution: 0.0,
                ..Default::default()
            },
        );
        world.step_once = true;
        world.step(DT).ok();

        let r = world
            .collision_events()
            .iter()
            .flat_map(|e| e.contact_points.iter())
            .map(|c| {
                let d = c.point - world.transforms[1].position;
                Vec3::new(d.x, 0.0, d.z).length()
            })
            .fold(0.0f32, f32::max);
        eprintln!(
            "{:>10.1}  {:>14.4}  {:>14.6}  {:>14.6}",
            h,
            r,
            world.velocities[1].angular.length(),
            world.velocities[1].linear.length()
        );
    }
}

/// Is the lean accumulated SLIDING at the interfaces, or accumulated ROTATION of the boxes?
///
/// This decides the leading hypothesis for the growth rate. `solver/tgs.rs` gives the normal
/// channel a position-level term — `Prepared::pen0`, driven into the bias by all three sweeps —
/// and gives the tangential channel nothing: every friction solve is plain `acc_t - rel·t/k_t`,
/// velocity-level only. Numerical slip at a resting contact therefore never gets corrected, so
/// it should creep and ratchet. If that is the mechanism, the column's lean is the sum of its
/// interface slips and the boxes stay nearly level.
///
/// Against that, the root-cause note in `soak_and_golden.rs` describes the mode as a chain of
/// relative rotations. If the lean is rotation-dominated instead, the missing tangential position
/// term is not what is driving it, however real the omission is.
///
/// Both cannot be true, so this measures which.
///
/// - `slip` = Σ over interfaces of the horizontal offset between consecutive boxes. This is the
///   lean a pure sliding mode would produce.
/// - `path` = Σ over interfaces and frames of |change in that offset|. If sliding ratchets one
///   way, `path ≈ |slip|`; if it oscillates around zero, `path ≫ |slip|`. This is what tells
///   creep from jitter, and no single-frame reading can.
/// - `tilt` = Σ of per-box tilt angles × box height. This is the lean a pure rotation mode would
///   produce.
/// - `lean` = the top box's actual horizontal displacement, for comparison against the two.
///
/// # Measured: the ratchet is refuted, and the decomposition was the wrong question
///
/// At 2200 frames on a 1×12×1 column, ground 200: `slip` (net) 0.0216, `path` 0.4622 — a ratio of
/// 21, so the interfaces micro-slide continuously but back and forth, not one way. `slip` and
/// `tilt` also track each other at a near-constant ratio (0.0216 / 0.0191), so they are two
/// measurements of one coupled rocking mode rather than two competing modes.
///
/// And `path` does not discriminate: at frame 2200 the SURVIVING realisations have accumulated
/// *more* of it (0.521 on ground 20, 0.480 for 2×12×2 on ground 20) than the collapsing ones
/// (0.462, 0.400). What separates them is the sustained amplitude — net lean and tilt are 3–18×
/// larger on the 200 m ground, already at frame 200, and stay that way for two thousand frames.
///
/// What survives of the hypothesis: the contact never LOCKS. Real static friction would hold a
/// resting interface still; here it micro-slides forever, which is what a tangential channel with
/// no position-level term does. That is a real defect. It is just not, on its own, what decides
/// whether the stack falls over.
#[test]
#[ignore = "measurement, not a gate — prints a trace"]
fn is_the_lean_slip_or_rotation() {
    // The 200 m ground, because that is the realisation that actually collapses (frame 2328) and
    // the growth is what is being decomposed.
    // Both grounds for the single column, because the discriminating question is whether the
    // micro-sliding differs between the realisation that survives (20) and the one that
    // collapses (200). If the slip path accumulates at the same rate in both, the sliding is a
    // property of every resting stack rather than the thing that decides its fate.
    for (label, side, ground) in [
        ("1x12x1", 1u32, 20.0f32),
        ("1x12x1", 1, 200.0),
        ("2x12x2", 2, 20.0),
        ("2x12x2", 2, 200.0),
    ] {
        let (mut world, bodies, origins) = scene_crate_pile(side, 12);
        world.colliders[0] = Collider::box_collider(Vec3::new(ground, 1.0, ground));

        // One column's worth of body indices, bottom to top: the x=0,z=0 column. scene_crate_pile
        // lays out y outermost, then x, then z, so the column stride is side*side.
        let column: Vec<usize> = (0..12).map(|y| bodies[(y * side * side) as usize]).collect();

        eprintln!("\n=== {label} on ground {ground}: lean decomposition ===");
        eprintln!(
            "{:>6}  {:>12}  {:>12}  {:>12}  {:>12}  {:>10}",
            "frame", "lean(top)", "slip(sum)", "slip path", "tilt(sum)", "max|v|"
        );

        let mut prev_offsets = vec![Vec3::ZERO; column.len() - 1];
        let mut path = 0.0f32;
        for f in 0..2400 {
            world.step(DT).ok();

            let mut slip_sum = 0.0f32;
            for (k, w) in column.windows(2).enumerate() {
                let d = world.transforms[w[1]].position - world.transforms[w[0]].position;
                let off = Vec3::new(d.x, 0.0, d.z);
                if !off.is_finite() {
                    break;
                }
                slip_sum += off.length();
                path += (off - prev_offsets[k]).length();
                prev_offsets[k] = off;
            }

            // Rotation contribution: each box's tilt tips everything above it by height × angle.
            let tilt_sum: f32 = column
                .iter()
                .map(|&i| {
                    let up = world.transforms[i].rotation.mul_vec3(Vec3::Y);
                    up.dot(Vec3::Y).clamp(-1.0, 1.0).acos() // box height is 1.0, so angle == offset
                })
                .sum();

            if f % 200 == 0 || f == 2399 {
                let o = observe(&world, &bodies, &origins);
                let top = *column.last().unwrap();
                let d = world.transforms[top].position - origins[(11 * side * side) as usize];
                eprintln!(
                    "{:>6}  {:>12.6}  {:>12.6}  {:>12.6}  {:>12.6}  {:>10.4}",
                    f,
                    Vec3::new(d.x, 0.0, d.z).length(),
                    slip_sum,
                    path,
                    tilt_sum,
                    o.max_speed
                );
                if !o.max_speed.is_finite() || o.max_speed > 1.0 {
                    eprintln!("  collapsed — stopping");
                    break;
                }
            }
        }
    }
}

/// Does the SETTLED contact jitter more on a bigger ground?
///
/// `is_the_lean_slip_or_rotation` refutes two stories and points at a third. It refutes the
/// ratchet (accumulated slip path oscillates: 21× more total motion than net displacement) and
/// it refutes slip path as the discriminator (the SURVIVING realisations accumulate *more* of it
/// — 0.521 on ground 20 against 0.462 on ground 200 at frame 2200). What separates survival from
/// collapse is the sustained AMPLITUDE: on the 200 m ground the column's net lean and tilt are
/// 3–18× larger, and they are already that much larger at frame 200 and stay there for two
/// thousand frames before it goes over.
///
/// A one-time kick cannot set a sustained amplitude, and
/// `does_a_bigger_ground_deliver_a_bigger_birth_kick` measured the kick as flat anyway. A
/// sustained amplitude needs a sustained noise source that scales with the ground's size — which
/// is what the precision hypothesis actually predicts, and which I dropped too early when the
/// birth-manifold defect turned up. Contact points on a 200 m ground are computed from world
/// coordinates ten times larger than on a 20 m one, so their absolute rounding error is ten times
/// larger, every frame, forever.
///
/// This measures it where `how_many_points_does_a_settled_interface_carry` could not: not whether
/// the settled manifold has the right shape, but how much it MOVES frame to frame when nothing
/// physical is moving. The stack is held awake so the interface keeps being regenerated.
///
///   - jitter scaling with the ground's half-extent ⇒ the sustained noise floor is precision, and
///     that is the ground-size mechanism
///   - jitter flat in the ground's half-extent ⇒ precision is refuted for good and the sustained
///     amplitude difference has some other source
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn does_a_settled_contact_jitter_more_on_a_bigger_ground() {
    eprintln!("\n=== settled 2-box stack, per-frame jitter of the GROUND interface (600 frames) ===");
    eprintln!(
        "{:>10}  {:>16}  {:>16}  {:>16}",
        "ground", "Σ|Δpoint|", "Σ|Δpenetration|", "max|Δpoint|"
    );
    for h in [5.0f32, 20.0, 50.0, 100.0, 200.0, 500.0] {
        let mut world = PhysicsWorld::new();
        add_ground_sized(&mut world, h);
        let no_bounce = PhysicsMaterial {
            restitution: 0.0,
            ..Default::default()
        };
        add_box(&mut world, 1, Vec3::new(0.0, 0.5, 0.0), 0.5, no_bounce);
        add_box(&mut world, 2, Vec3::new(0.0, 1.5, 0.0), 0.5, no_bounce);

        // Settle first, awake throughout — a dormant pair skips narrowphase and reports nothing.
        for _ in 0..240 {
            for i in 1..world.entities.len() {
                world.rigid_bodies[i].wake_up();
            }
            world.step(DT).ok();
        }

        let ground_points = |w: &PhysicsWorld| -> Vec<(Vec3, f32)> {
            let mut v: Vec<(Vec3, f32)> = w
                .collision_events()
                .iter()
                .filter(|e| e.entity_a.id() == 0 || e.entity_b.id() == 0)
                .flat_map(|e| e.contact_points.iter())
                .map(|c| (c.point, c.penetration))
                .collect();
            v.sort_by(|a, b| {
                (a.0.x, a.0.z)
                    .partial_cmp(&(b.0.x, b.0.z))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            v
        };

        let mut prev = ground_points(&world);
        let (mut sum_p, mut sum_d, mut max_p) = (0.0f32, 0.0f32, 0.0f32);
        for _ in 0..600 {
            for i in 1..world.entities.len() {
                world.rigid_bodies[i].wake_up();
            }
            world.step(DT).ok();
            let now = ground_points(&world);
            if now.len() == prev.len() {
                for (a, b) in prev.iter().zip(&now) {
                    let d = (b.0 - a.0).length();
                    sum_p += d;
                    max_p = max_p.max(d);
                    sum_d += (b.1 - a.1).abs();
                }
            }
            prev = now;
        }
        eprintln!(
            "{:>10.0}  {:>16.6e}  {:>16.6e}  {:>16.6e}",
            h, sum_p, sum_d, max_p
        );
    }
}

/// Is the ground's half-extent a mechanism at all, or just a way of drawing a different sample?
///
/// Four candidate mechanisms for the ground-size sensitivity have now been measured and refuted:
/// contact-generation precision (a settled interface is bit-stable at every ground size,
/// `does_a_settled_contact_jitter_more_on_a_bigger_ground`); the birth kick's magnitude (flat to
/// 0.58% across 20 → 200, `does_a_bigger_ground_deliver_a_bigger_birth_kick`); a tangential
/// ratchet (the slip oscillates 21:1, `is_the_lean_slip_or_rotation`); and accumulated slip as
/// the discriminator (the surviving runs accumulate *more* of it).
///
/// What is left is that there is no mechanism: half-extent 20 and 200 are two draws from a
/// distribution, and the instability is chaotic enough that any perturbation decorrelates them.
///
/// This is the test that settles it, and the design matters. Instead of comparing 20 against 200,
/// compare 20.0 against 20.001 — a one-millimetre change to a 40-metre static box, which no
/// mechanism proportional to size could possibly notice.
///
///   - **If tiny perturbations scatter the outcome as widely as 20 → 200 does**, ground size is
///     not a mechanism, N1 is a sampling artefact, and the correct way to report anything about
///     this scene class is a distribution over an ensemble rather than any single run's verdict.
///   - **If tiny perturbations give consistent outcomes** and only large size changes move them,
///     there is a mechanism and it is still unidentified.
///
/// # Measured: it is BOTH, and neither answer alone was right
///
/// ```text
///  half-extent   blew_up_at   peak_lean        half-extent   blew_up_at   peak_lean
///       20.000       -          0.0104             128.000       -          0.0182
///       20.001       -          0.0140             130.000       -          0.0186
///       20.010       -          0.0098             140.000     2312         10.1681
///       20.100       -          0.0110             150.000     2875          4.2182
///       21.000       -          0.0104             160.000       -          0.0178
///       50.000       -          0.0186             175.000       -          0.0186
///      100.000       -          0.0142             190.000       -          0.0178
///      110.000       -          0.0233             200.000     2328         10.0384
///      120.000       -          0.0239
/// ```
///
/// **A millimetre does not decorrelate anything.** 20.000 through 21.000 give the same outcome and
/// peak leans within 0.004 of each other. So the instability is not so chaotic that any
/// perturbation reshuffles it, and H-C — "ground size has no mechanism, it just draws a different
/// sample" — is refuted.
///
/// **But there is no threshold either.** 140 and 150 collapse, 160, 175 and 190 stand, 200
/// collapses. My own intermediate reading of this table said the mechanism "switches on between
/// 100 and 200"; bisecting shows a scattered failure region, not a switch.
///
/// **What the size actually does is raise the resting amplitude**, and that part is orderly: peak
/// lean sits near 0.011 up to half-extent 21, and near 0.018–0.024 from 50 upward, where it
/// saturates. Collapses appear only once the amplitude is in the upper band, and *which* sizes in
/// that band collapse is chaotic.
///
/// So N1 is a real size-dependent effect on the amplitude, with a chaotic outcome layered on top
/// of it. What raises the amplitude is still unidentified: contact-generation precision is
/// refuted for a settled interface (`does_a_settled_contact_jitter_more_on_a_bigger_ground`
/// measures zero jitter at every size), though only on a two-box stack that reaches an exact
/// fixed point — which is a weaker test than the tall column this is about, and is the loose end
/// worth pulling next.
#[test]
#[ignore = "measurement, not a gate — long (~6 min)"]
fn is_ground_size_a_mechanism_or_just_a_seed() {
    eprintln!("\n=== 1x12x1, default solver, 3000 frames: outcome vs tiny ground perturbations ===");
    eprintln!(
        "{:>12}  {:>12}  {:>11}  {:>11}",
        "ground half", "blew_up_at", "peak|v|", "peak_lean"
    );
    // The first group is physically indistinguishable — a millimetre on a 40 m box. The second
    // spans the range that produced the original observation.
    for h in [
        20.0f32, 20.001, 20.01, 20.1, 21.0, // "the same" ground
        50.0, 100.0, // still quiet
        // Bisecting what first looked like a threshold between 100 and 200. It is not one — see
        // the table in the doc comment. 128 is in the list because a boundary landing on a power
        // of two would have pointed at a float exponent; it does not.
        110.0, 120.0, 128.0, 130.0, 140.0, 150.0, 160.0, 175.0, 190.0, 200.0,
    ] {
        let (mut world, bodies, origins) = scene_tower_on_ground(12, h);
        let r = run(&mut world, &bodies, &origins, 3000, 0.5);
        eprintln!(
            "{:>12.3}  {:>12}  {:>11.3}  {:>11.4}",
            h,
            match r.blew_up_at {
                Some(f) => f.to_string(),
                None => "-".to_string(),
            },
            r.peak_speed,
            r.peak_lean
        );
    }
}

/// The loose end from `does_a_settled_contact_jitter_more_on_a_bigger_ground`: repeat the
/// precision test on the scene it is actually about.
///
/// That measurement found a settled interface bit-stable at every ground size and so refuted
/// contact-generation precision as the source of the sustained amplitude — but it used a two-box
/// stack, which reaches an exact fixed point and therefore never exercises the arithmetic under
/// load. The twelve-high column does not sit still: it rocks at an amplitude that
/// `is_ground_size_a_mechanism_or_just_a_seed` shows roughly doubling with the ground's size.
///
/// The measurement problem is separating numerical jitter from real motion — on a leaning column
/// the contact points move because the bodies move. `ContactPoint` solves it directly:
/// `local_point_a` and `local_point_b` are the contact expressed in each body's own frame
/// (`gizmo-physics-core/src/collision.rs`), so a geometrically stable contact has a constant local
/// point however much its body travels. Movement there is the contact being re-derived
/// differently, which is exactly the quantity in question.
///
/// The box-to-box interfaces are the built-in control: both bodies are half-extent 0.5 whatever
/// the ground is, so no size-dependent mechanism can touch them. Only the ground interface sees
/// the big collider.
///
///   - ground-interface jitter scaling with half-extent while box-box jitter stays flat ⇒
///     precision is back, and it is the amplitude mechanism
///   - both flat ⇒ the fifth candidate falls too, and the mechanism is in the solver's own state
///     rather than in the geometry it is fed
///
/// # Measured: flat. Precision is refuted on the tall column too.
///
/// ```text
///   ground half-extent    Σ|Δlocal| ground interface
///                   20                    2.700e-5
///                   50                    2.700e-5
///                  100                    2.551e-5
///                  200                    2.599e-5
///                  500                    2.301e-5
/// ```
///
/// Twenty-five-fold in the collider's size, no change in how much the contact moves in the body's
/// own frame — slightly *less*, if anything. The fifth candidate falls with the other four.
///
/// The box-to-box control column is NOT reported, because the way it was gathered is invalid:
/// eleven interfaces contribute four points each, all with local coordinates near ±0.5, and
/// sorting them into one list mixes points from different interfaces so that any reordering
/// between frames reads as a huge jump. It produced ~6000 per run, which is six orders of
/// magnitude above the ground interface's figure and is measurement noise, not physics. Matching
/// per interface would fix it; the ground pair needs no such fix because it is one interface.
///
/// **The incidental finding is the bigger one.** Holding every body awake keeps this column
/// almost perfectly straight — peak lean 4e-5, against 0.0104 for the same scene left to sleep
/// naturally (`is_ground_size_a_mechanism_or_just_a_seed`). A factor of 250. Sleeping is supposed
/// to be an optimisation with no effect on the answer. See
/// `does_the_column_only_lean_when_it_is_allowed_to_sleep`.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn does_the_tall_column_contact_jitter_with_ground_size() {
    eprintln!("\n=== 1x12x1 held awake, 600 frames: contact drift in BODY-LOCAL coordinates ===");
    eprintln!(
        "{:>10}  {:>18}  {:>18}  {:>12}",
        "ground", "Σ|Δlocal| ground", "Σ|Δlocal| box-box", "peak lean"
    );
    for h in [20.0f32, 50.0, 100.0, 200.0, 500.0] {
        let (mut world, bodies, origins) = scene_tower_on_ground(12, h);
        let wake = |w: &mut PhysicsWorld| {
            for i in 1..w.entities.len() {
                w.rigid_bodies[i].wake_up();
            }
        };
        for _ in 0..240 {
            wake(&mut world);
            world.step(DT).ok();
        }

        // Local-space contact points, split into the ground pair and everything else. Sorted so a
        // reordering between frames cannot read as drift.
        let sample = |w: &PhysicsWorld, ground_pair: bool| -> Vec<Vec3> {
            let mut v: Vec<Vec3> = w
                .collision_events()
                .iter()
                .filter(|e| (e.entity_a.id() == 0 || e.entity_b.id() == 0) == ground_pair)
                .flat_map(|e| e.contact_points.iter())
                .map(|c| c.local_point_b)
                .collect();
            v.sort_by(|a, b| {
                (a.x, a.y, a.z)
                    .partial_cmp(&(b.x, b.y, b.z))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            v
        };

        let (mut prev_g, mut prev_b) = (sample(&world, true), sample(&world, false));
        let (mut sum_g, mut sum_b) = (0.0f32, 0.0f32);
        let mut peak_lean = 0.0f32;
        for _ in 0..600 {
            wake(&mut world);
            world.step(DT).ok();
            peak_lean = peak_lean.max(observe(&world, &bodies, &origins).lean);
            for (prev, sum, ground_pair) in [
                (&mut prev_g, &mut sum_g, true),
                (&mut prev_b, &mut sum_b, false),
            ] {
                let now = sample(&world, ground_pair);
                if now.len() == prev.len() {
                    for (a, b) in prev.iter().zip(&now) {
                        *sum += (*b - *a).length();
                    }
                }
                *prev = now;
            }
        }
        eprintln!(
            "{:>10.0}  {:>18.6e}  {:>18.6e}  {:>12.5}",
            h, sum_g, sum_b, peak_lean
        );
    }
}

/// Does the column only lean when it is allowed to sleep?
///
/// `does_the_tall_column_contact_jitter_with_ground_size` had to hold every body awake to keep the
/// contacts being regenerated, and the column it measured stayed 250× straighter than the same
/// scene left alone. Sleeping is meant to be an optimisation — an island nobody is touching stops
/// being solved — and an optimisation that changes the answer by 250× is not an optimisation.
///
/// There is a concrete mechanism available for it, found while auditing the write-back:
/// `pipeline.rs` applies the solver's results to **every dynamic member of an active island
/// without checking the sleep flag** — both `self.velocities[idx] = vel` and the split-impulse
/// `t.position += dlin` / rotation delta. The integrator then skips sleeping bodies entirely. So a
/// body that has fallen asleep inside a still-active island keeps being *teleported* by position
/// corrections while never integrating, never damping, and never re-evaluating whether it should
/// still be asleep. A partially-sleeping column is being shoved with no velocity feedback.
///
/// Partial sleep is not hypothetical here: the traces in `measure_tower_buckling_channels` and
/// `verify_wide_pile_collapse` show `asleep` counts flickering between 0 and 6 while the stack is
/// nominally at rest.
///
///   - forced-awake stands at every ground size while natural sleep collapses at 140, 150 and 200
///     ⇒ the sleep path is the mechanism, and the ground size only shuffles *when* things sleep
///   - both collapse similarly ⇒ the 250× was an artefact of the forced-wake run being a different
///     trajectory, and this is a dead end
#[test]
#[ignore = "measurement, not a gate — long (~4 min)"]
fn does_the_column_only_lean_when_it_is_allowed_to_sleep() {
    eprintln!("\n=== 1x12x1, 3000 frames: natural sleep vs every body held awake ===");
    eprintln!(
        "{:>10}  {:>26}  {:>26}",
        "ground", "natural (blew@ / lean)", "forced awake (blew@ / lean)"
    );
    for h in [20.0f32, 100.0, 140.0, 150.0, 200.0] {
        let mut cells = Vec::new();
        for force_awake in [false, true] {
            let (mut world, bodies, origins) = scene_tower_on_ground(12, h);
            let mut r = Run {
                blew_up_at: None,
                peak_speed: 0.0,
                peak_lean: 0.0,
                peak_tilt_deg: 0.0,
                final_pen_max: 0.0,
                resting_pen: 0.0,
                blew_up_at_awake: None,
                trip_lean: 0.0,
                trip_tilt_deg: 0.0,
            };
            for f in 0..3000 {
                if force_awake {
                    for i in 1..world.entities.len() {
                        world.rigid_bodies[i].wake_up();
                    }
                }
                world.step(DT).ok();
                let o = observe(&world, &bodies, &origins);
                if !o.max_speed.is_finite() {
                    r.blew_up_at.get_or_insert(f);
                    break;
                }
                r.peak_speed = r.peak_speed.max(o.max_speed);
                r.peak_lean = r.peak_lean.max(o.lean);
                if r.blew_up_at.is_none() && o.max_speed >= 0.5 {
                    r.blew_up_at = Some(f);
                }
            }
            cells.push(format!(
                "{:>10} / {:<12.6}",
                match r.blew_up_at {
                    Some(f) => f.to_string(),
                    None => "-".to_string(),
                },
                r.peak_lean
            ));
        }
        eprintln!("{:>10.0}  {:>26}  {:>26}", h, cells[0], cells[1]);
    }
}

/// Does a sleeping stack keep re-deriving DEGENERATE manifolds?
///
/// Two findings want to be the same finding. `what_does_a_manifold_look_like_when_it_is_born`
/// shows a manifold born at exact contact carrying a single off-centre point and a `Δω ≈ 0.03`
/// kick, because `clip_box_box` rejects every corner at `signed_depth == 0.0`.
/// `does_the_column_only_lean_when_it_is_allowed_to_sleep` shows a column held awake leaning
/// 0.000106 and never falling, while the same column left to sleep leans up to 10 and collapses.
/// If sleeping drops a pair out of narrowphase and waking makes it re-derive its contacts from a
/// near-zero-penetration state, the two are one mechanism: sleep cycling re-triggers the
/// degenerate birth over and over.
///
/// The prediction is specific and easy to falsify: a naturally-sleeping stack should show
/// repeated one-point manifolds, and a held-awake one should show none after the first substep.
/// A first attempt at a fix — skipping the solver write-back for bodies that stay asleep — is
/// already known NOT to reproduce the held-awake behaviour (it left the lean at 0.0099 against
/// 0.000106, removed three collapses out of five while adding one to a previously-green test, and
/// was reverted), so the mechanism is something about sleep other than the stale write-back.
///
/// Counted here: events carrying exactly one contact point, and `Started` events, which is what
/// a genuinely re-born pair emits.
#[test]
#[ignore = "measurement, not a gate — prints a table"]
fn does_sleep_cycling_re_derive_degenerate_manifolds() {
    eprintln!("\n=== 1x12x1, 1200 frames: degenerate (1-point) manifolds and pair re-births ===");
    eprintln!(
        "{:>10}  {:>14}  {:>14}  {:>14}  {:>12}",
        "ground", "mode", "1-point events", "Started events", "peak lean"
    );
    for h in [20.0f32, 200.0] {
        for force_awake in [false, true] {
            let (mut world, bodies, origins) = scene_tower_on_ground(12, h);
            let (mut degenerate, mut started) = (0usize, 0usize);
            let mut peak_lean = 0.0f32;
            for _ in 0..1200 {
                if force_awake {
                    for i in 1..world.entities.len() {
                        world.rigid_bodies[i].wake_up();
                    }
                }
                world.step(DT).ok();
                for ev in world.collision_events() {
                    if ev.contact_points.len() == 1 {
                        degenerate += 1;
                    }
                    if matches!(ev.event_type, gizmo_physics_core::CollisionEventType::Started) {
                        started += 1;
                    }
                }
                peak_lean = peak_lean.max(observe(&world, &bodies, &origins).lean);
            }
            eprintln!(
                "{:>10.0}  {:>14}  {:>14}  {:>14}  {:>12.6}",
                h,
                if force_awake { "held awake" } else { "natural" },
                degenerate,
                started,
                peak_lean
            );
        }
    }
}

/// The height-6 class, on the DEFAULT configuration, over a ground ensemble.
///
/// Height 6 has been the solid ground of every table in this file — `wide_block_collapse_per_ground`
/// found 2×6×2 standing in all four of its cells and 3×6×3 in three of four. That makes it the
/// right place to check a solver change for damage, because a regression here is a regression
/// inside the envelope `docs/ENGINE.md` calls safe, not out at the marginal edge.
///
/// Forced sweep counts are deliberately NOT used: the question is what ships.
///
/// # This is the measurement that killed the sleeping-as-static fix
///
/// Treating a sleeping body as infinite-mass inside the solve — the standard remedy for the
/// non-conserving interface documented in `solver/mod.rs` — looked good on height 12: the
/// `height_12_stacks_stay_standing` ensemble went from 5 of 6 cells collapsing to 2, the two
/// survivors collapsed about twice as late, every existing soak stayed green (including
/// `soak_demo_tower_awake_stays_upright`, which a previous attempt had broken), and determinism
/// held at a new hash.
///
/// Then this ensemble:
///
/// ```text
///   block    ground   without the fix     with the fix
///   2x6x2       20          -                1249
///   2x6x2      100          -                   -
///   2x6x2      200          -                   -
///   3x6x3    20/100/200      -                   -
///   4x6x4       20          -                   -
///   4x6x4      100          -                2724
///   4x6x4      200          -                1851
///                        0 of 9            3 of 9
/// ```
///
/// Height 6 is the class the engine currently carries reliably and is well inside the ≤~12
/// envelope. Trading it for a partial improvement at height 12 is not a fix, so the change was
/// reverted.
///
/// Why it probably backfires, as a hypothesis for whoever picks this up: an infinitely massive
/// sleeper in the middle of a column is a discontinuity in the mass distribution, and the stack's
/// dynamics change abruptly the moment any body falls asleep. Both treatments are wrong — finite
/// mass with no integration is non-conserving, infinite mass is a step change — which points at
/// the real condition being that a stack is allowed to sleep PARTIALLY at all. Bodies fall asleep
/// individually in `integrator.rs`; only waking is island-collective (`pipeline.rs`). Holding
/// everything awake, which is exactly the state where no partial sleep exists, is the one
/// configuration measured that behaves perfectly.
#[test]
#[ignore = "measurement, not a gate — long (~5 min)"]
fn height_6_blocks_on_the_default_config() {
    eprintln!("\n=== height-6 blocks, DEFAULT solver, 3000 frames, ground ensemble ===");
    eprintln!(
        "{:>10}  {:>8}  {:>12}  {:>11}  {:>11}",
        "block", "ground", "blew_up_at", "peak|v|", "peak_lean"
    );
    let mut collapses = 0;
    let mut cells = 0;
    for side in [2u32, 3, 4] {
        for g in [20.0f32, 100.0, 200.0] {
            let (mut world, bodies, origins) = scene_crate_pile(side, 6);
            world.colliders[0] = Collider::box_collider(Vec3::new(g, 1.0, g));
            let r = run(&mut world, &bodies, &origins, 3000, 0.5);
            cells += 1;
            if r.blew_up_at.is_some() {
                collapses += 1;
            }
            eprintln!(
                "{:>10}  {:>8.0}  {:>12}  {:>11.3}  {:>11.4}",
                format!("{side}x6x{side}"),
                g,
                match r.blew_up_at {
                    Some(f) => f.to_string(),
                    None => "-".to_string(),
                },
                r.peak_speed,
                r.peak_lean
            );
        }
    }
    eprintln!("  collapsed {collapses} of {cells}");
}

/// Does a starved solver's pile survive, or does it merely fall asleep before the damage shows?
///
/// This is the question `negative_control_starved_pile_must_fail_the_gate` cannot answer on its
/// own, and it is the attack a reviewer raised against this whole file: a stability gate can pass
/// trivially by the scene going quiet. It matters most right after a change to when things sleep.
///
/// Three arms on the same starved (4-sweep) scene:
///   - natural, 1500 frames — what the gate sees
///   - natural, 6000 frames — is the collapse merely later?
///   - held awake, 1500 frames — what the solver does when sleep cannot hide anything
///
/// If the held-awake arm collapses while the natural one sleeps through it, sleeping is masking a
/// real defect and the gate must be rebuilt on the awake arm. If neither collapses, the starved
/// solver genuinely is not damaging this scene any more and the negative control has simply lost
/// its subject.
#[test]
#[ignore = "measurement, not a gate — long (~4 min)"]
fn is_the_starved_pile_surviving_or_just_sleeping() {
    eprintln!("\n=== 4x6x4 at 4 forced sweeps: survival vs sleep ===");
    eprintln!(
        "{:>26}  {:>12}  {:>11}  {:>11}  {:>12}",
        "arm", "blew_up_at", "peak|v|", "peak_lean", "asleep@end"
    );
    for (label, frames, force_awake) in [
        ("natural, 1500", 1500usize, false),
        ("natural, 6000", 6000, false),
        ("held awake, 1500", 1500, true),
    ] {
        let (mut world, bodies, origins) = scene_crate_pile(4, 6);
        solver_with_exact_sweeps(&mut world, 1);
        let mut r_blew: Option<usize> = None;
        let (mut peak_v, mut peak_lean) = (0.0f32, 0.0f32);
        let mut asleep = 0usize;
        for f in 0..frames {
            if force_awake {
                for i in 1..world.entities.len() {
                    world.rigid_bodies[i].wake_up();
                }
            }
            world.step(DT).ok();
            let o = observe(&world, &bodies, &origins);
            if !o.max_speed.is_finite() {
                r_blew.get_or_insert(f);
                break;
            }
            peak_v = peak_v.max(o.max_speed);
            peak_lean = peak_lean.max(o.lean);
            asleep = o.asleep;
            if r_blew.is_none() && o.max_speed >= 0.5 {
                r_blew = Some(f);
            }
        }
        eprintln!(
            "{:>26}  {:>12}  {:>11.3}  {:>11.4}  {:>12}",
            label,
            match r_blew {
                Some(f) => f.to_string(),
                None => "-".to_string(),
            },
            peak_v,
            peak_lean,
            format!("{}/{}", asleep, bodies.len())
        );
    }
}

/// Retired negative control, kept because its premise turned out to be measuring the bug.
///
/// It used to starve `realistic_crate_stack_stays_standing`'s own scene to 4 sweeps and require
/// the gate to fail, on the reasoning that a gate nobody has watched fail is a gate of unknown
/// strength. It did fail, so the gate looked well-founded.
///
/// Then island-collective sleep landed and this stopped tripping at any sweep count, including 1.
/// `is_the_starved_pile_surviving_or_just_sleeping` settles what that means, and it is not that
/// the scene now passes by going quiet:
///
/// ```text
///   4x6x4 at ONE sweep      blew_up_at   peak|v|   peak_lean   asleep@end
///   before, natural            193        15.426     8.4074      25/96
///   before, held awake          -          0.159     0.0021       0/96
///   after,  natural             -          0.159     0.0021      96/96
///   after,  held awake          -          0.159     0.0021       0/96
/// ```
///
/// Held awake, the pile was ALWAYS fine at one sweep. What destroyed it was the 25 of 96 bodies
/// that had fallen asleep inside a still-active island. So sweep starvation never damaged this
/// scene — it only jittered bodies into the partial-sleep regime where the real defect lived.
/// This control was measuring the bug, not the sweep count.
///
/// Sweep sensitivity is still guarded, by `sweep_throttling_is_visible_to_this_file` on the free
/// chain, which does not depend on sleep. The crate-stack gate is now a stability gate, and the
/// evidence that its class is real is `height_12_stacks_stay_standing`, which failed before this
/// change and passes after it.
#[test]
#[ignore = "retired — its premise was refuted; see the doc comment"]
fn negative_control_starved_pile_must_fail_the_gate() {
    let (mut world, bodies, origins) = scene_crate_pile(4, 6);
    solver_with_exact_sweeps(&mut world, 1);
    let r = run(&mut world, &bodies, &origins, 1500, 0.5);
    assert!(
        r.blew_up_at.is_some() || r.peak_lean >= 0.5 || r.resting_pen >= 0.015,
        "starving the crate-stack gate's own scene did not trip any of its three assertions \
         inside 1500 frames. The gate is not protecting the thing it claims to: either the \
         horizon is too short or the criteria are too loose. {r:?}"
    );
}

/// **Currently fails. Records a live defect this file found, and is `#[ignore]`d in the same
/// way `soak_extreme_tower_n48_stays_bounded` is.**
///
/// A crate stack twelve boxes high does not reliably stand for 3000 frames on the default
/// configuration — at any width from 1 to 4, with or without a lateral gap, at every sweep count
/// tried. Twelve is the top of the ≤~12 envelope `docs/ENGINE.md` calls safe, and it is far below
/// the 32-high column `soak_resting_stacks_stay_bounded` keeps green.
///
/// **What is NOT true, corrected from an earlier reading of this same data.** The first pass ran
/// only the 4-wide block and only on one ground, saw it survive at 96 sweeps and collapse at 28,
/// and concluded the adaptive policy was under-spending sweeps by ~4×. Running each ground
/// separately refutes that:
///
/// ```text
///   block     28 sweeps          96 sweeps          (blow-up frame, ground 20 / ground 200)
///   1x12x1    -    / 2328*       stands at 8 sweeps forced
///   2x12x2    2451 / 1979        2782 / 2037     <- 96 sweeps does NOT save it
///   3x12x3    2379 / 1373        2687 / -
///   4x12x4    1267 / 1447        -    / -        <- the one shape 96 sweeps does save
///   2x6x2     -    / -           -    / -        <- height 6 is solid
///   3x6x3     -    / 2670        -    / -
///   * default config (adaptive, 28 sweeps)
/// ```
///
/// Raising the sweep count rescues the 4-wide block and nothing else. So this is not a sweep
/// *budget* problem, and the sweep policy is not the fix — which also means it is not evidence
/// that support depth is the wrong policy input, however plausible that remains on other grounds.
///
/// **What the data does support** is narrower and worse: at height 12 the outcome is decided by
/// perturbations with no physical content. Every knob varied flips it non-monotonically — the
/// static ground's half-extent (20 stands, 200 collapses), the lateral gap between columns
/// (exact contact stands, 2 cm collapses at frame 70, 20 cm stands), the sweep count (8 forced
/// sweeps stands where 28 adaptive sweeps collapses), and the width (1 and 2 differ). That is
/// the signature the root-cause note in `soak_and_golden.rs` already describes: an eigenvalue
/// just above 1, seeded by float noise. At height 12 it is marginal rather than settled.
///
/// **Why the suite never saw it.** `soak_resting_stacks_stay_bounded` tests 1-wide columns, on
/// one ground, for 1500 frames. Every collapse above except one lands between frames 1979 and
/// 2782 — past its horizon — and the ones inside it are on a ground size it never builds.
///
/// The honest conclusion for the sweep work: the sweep policy cannot be tuned against a scene
/// class whose stability is this marginal, because any measured improvement is inside the noise.
/// Height 12 needs the stability gap closed first.
#[test]
#[ignore = "acceptance test — passes since island-collective sleep; run before shipping a solver change (~10 s)"]
fn height_12_stacks_stay_standing() {
    let mut failures = Vec::new();
    for side in [1u32, 2, 4] {
        for g in [20.0f32, 200.0] {
            let (mut world, bodies, origins) = scene_crate_pile(side, 12);
            world.colliders[0] = Collider::box_collider(Vec3::new(g, 1.0, g));
            let r = run(&mut world, &bodies, &origins, 3000, 0.5);
            if r.blew_up_at.is_some() {
                failures.push(format!("{side}x12x{side} on ground {g}: {r:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "12-high crate stacks collapsed on their own:\n  {}",
        failures.join("\n  ")
    );
}

/// Sanity: the observables read something sane on a scene whose answer is known by hand, so a
/// broken reader cannot silently make every other measurement look clean. A single box resting
/// on the ground must end up still, level, barely penetrating, and asleep.
#[test]
fn observables_read_a_resting_box_correctly() {
    let (mut world, bodies, origins) = scene_tower(1);
    let mut last = Frame::default();
    for _ in 0..240 {
        world.step(DT).ok();
        last = observe(&world, &bodies, &origins);
    }
    assert!(last.max_speed < 0.05, "a lone resting box moved: {last:?}");
    assert!(last.lean < 1e-3, "a lone resting box drifted sideways: {last:?}");
    assert!(
        last.tilt.to_degrees() < 1.0,
        "a lone resting box tilted: {last:?}"
    );
    assert!(
        last.max_penetration < 0.05,
        "a lone resting box sank into the ground: {last:?}"
    );
    assert!(
        last.energy.is_finite(),
        "energy readout went non-finite: {last:?}"
    );
    // The reader must actually be reading contacts — if `collision_events` were empty the
    // penetration assertions above would pass vacuously, which is exactly the failure this
    // sanity test exists to catch.
    assert!(
        last.contact_count > 0 || last.asleep == 1,
        "no contacts observed and the box is not asleep — the reader is blind: {last:?}"
    );
}
