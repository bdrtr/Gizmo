//! `physics_explosion_system` regression tests — the blast/sleep interaction.
//!
//! The bug these lock down: the system wrote the blast impulse into `Velocity` but left
//! `RigidBody::is_sleeping` alone. A sleeping body is skipped by BOTH integration stages
//! (`Integrator::integrate_velocities` / `integrate_positions` return early on the flag, and
//! `pipeline.rs` skips it again before calling either), and an island whose members are all
//! asleep is not even solved — so the write sat in the component and moved nothing. A settled
//! stack next to a blast simply did not react: it looked like the explosion had not happened.
//!
//! Note the failure mode is *positional*, not velocity-level: the pre-fix code does leave the
//! blast velocity in the component, so asserting on `Velocity` alone would pass either way and
//! prove nothing. These tests assert on `is_sleeping` and on the transform.
//!
//! They also pin the SCOPE of the wake, which is the obvious way to over-correct this: waking
//! bodies the blast never reached would be its own bug (a whole level twitching awake because
//! something exploded in the next room). The wake sits inside the radius test and fires only
//! when the applied Δv is non-zero.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{Explosion, ExplosionFalloff, RigidBody, Velocity};
use gizmo_physics_rigid::system::{physics_explosion_system, physics_step_system};
use gizmo_physics_rigid::world::PhysicsWorld;

/// The blast used by both tests: centred at `BLAST_POS`, reach 5 m, linear falloff.
/// A unit-mass body 3 m away therefore takes `12.5 · (1 − 3/5) / 1 kg = 5 m/s`.
const BLAST_POS: Vec3 = Vec3::new(-3.0, 0.5, 0.0);
const BLAST_FORCE: f32 = 12.5;
const BLAST_RADIUS: f32 = 5.0;

fn blast() -> Explosion {
    Explosion {
        force_radius: BLAST_RADIUS,
        force: BLAST_FORCE,
        damage: 0.0,
        damage_radius: BLAST_RADIUS,
        falloff: ExplosionFalloff::Linear,
        offset: Vec3::ZERO,
        is_active: true,
    }
}

fn spawn_blast(world: &mut World) {
    let e = world.spawn();
    world.add_component(e, blast());
    world.add_component(e, Transform::new(BLAST_POS));
}

/// A dynamic box parked as if it had already settled and fallen asleep.
fn sleeping_box(mass: f32) -> RigidBody {
    let mut rb = RigidBody::new(mass, true);
    rb.update_inertia_from_collider(&Collider::box_collider(Vec3::splat(0.5)));
    // What `update_sleep_state` would have left after a quiet run — see
    // `RigidBody::SLEEP_FRAMES_REQUIRED`.
    rb.is_sleeping = true;
    rb.sleep_counter = RigidBody::SLEEP_FRAMES_REQUIRED + 40;
    rb
}

fn spawn_body(world: &mut World, rb: RigidBody, pos: Vec3) -> gizmo_core::entity::Entity {
    let e = world.spawn();
    world.add_component(e, rb);
    world.add_component(e, Collider::box_collider(Vec3::splat(0.5)));
    world.add_component(e, Transform::new(pos));
    world.add_component(e, Velocity::default());
    e
}

/// The wake reaches exactly the bodies the blast MOVES — no fewer, no more.
///
/// One pass of the explosion system over three hand-parked sleeping bodies, with no stepping
/// at all, so nothing but the system under test can touch the flags:
///
/// * **in range, finite mass** — must wake. This is the assertion that FAILS before the fix:
///   the pre-fix system gave this body its 5 m/s and left `is_sleeping` set, so the integrator
///   would have thrown the impulse away on the next step.
/// * **out of range** — must stay asleep, and must keep a zero velocity. Guards against
///   hoisting the wake above the radius test.
/// * **in range but `mass == 0`** (dynamic, so `inv_mass() == 0`) — must stay asleep: the
///   blast applies a zero Δv to it, i.e. it does not move it, so waking it would only spend
///   simulation on a body nothing happened to. Guards against an unconditional wake inside
///   the radius test.
#[test]
fn explosion_wakes_exactly_the_bodies_it_moves() {
    let mut world = World::new();

    // 3 m from the blast, unit mass → Δv = 5 m/s along +x.
    let hit = spawn_body(&mut world, sleeping_box(1.0), Vec3::new(0.0, 0.5, 0.0));
    // 23 m away — far outside the 5 m reach.
    let far = spawn_body(&mut world, sleeping_box(1.0), Vec3::new(20.0, 0.5, 0.0));
    // sqrt(3² + 2²) ≈ 3.6 m → inside the reach, but infinite mass → Δv = 0.
    let massless = spawn_body(&mut world, sleeping_box(0.0), Vec3::new(0.0, 0.5, 2.0));

    spawn_blast(&mut world);
    physics_explosion_system(&world, 1.0 / 60.0);
    world.apply_commands(); // the system despawns the (one-shot) explosion entity

    let bodies = world.borrow::<RigidBody>();
    let vels = world.borrow::<Velocity>();

    // ── The fix ──────────────────────────────────────────────────────────────
    let hit_rb = bodies.get(hit.id()).expect("hit body");
    let hit_v = vels.get(hit.id()).expect("hit velocity");
    assert!(
        !hit_rb.is_sleeping,
        "a blast inside the radius must WAKE the body it pushes — otherwise the integrator \
         skips it and the impulse ({:?}) is silently swallowed",
        hit_v.linear
    );
    assert_eq!(hit_rb.sleep_counter, 0, "waking must reset the sleep counter");
    // The impulse itself is unchanged by the fix: 12.5 · (1 − 3/5) / 1 kg = 5 m/s, +x.
    assert!(
        (hit_v.linear - Vec3::new(5.0, 0.0, 0.0)).length() < 1e-3,
        "blast impulse changed: {:?}",
        hit_v.linear
    );

    // ── Scope: nothing else is disturbed ─────────────────────────────────────
    let far_rb = bodies.get(far.id()).expect("far body");
    assert!(
        far_rb.is_sleeping,
        "a body 23 m outside a 5 m blast must stay asleep — the radius test has to gate the wake"
    );
    assert_eq!(
        vels.get(far.id()).expect("far velocity").linear,
        Vec3::ZERO,
        "a body outside the radius must not be pushed either"
    );

    let massless_rb = bodies.get(massless.id()).expect("massless body");
    assert!(
        massless_rb.is_sleeping,
        "the blast applies zero Δv to an infinite-mass body, so it must not be woken by it"
    );
    assert_eq!(
        vels.get(massless.id()).expect("massless velocity").linear,
        Vec3::ZERO,
        "infinite mass — the blast cannot accelerate it"
    );
}

/// End-to-end: a box that has genuinely settled and fallen asleep on the ground is MOVED by a
/// blast beside it, driven through the real ECS bridge (`physics_step_system`).
///
/// This is the user-visible symptom, and it is why the assertion has to be positional. Before
/// the fix the box's `Velocity` still received the 5 m/s — `is_sleeping` was the only thing
/// missing — but `pipeline.rs` skips a sleeping body in gravity/velocity integration, its
/// contact pair with the (static) ground is filtered out as dormant, its island is never
/// solved, and position integration returns early. The box therefore stays exactly where it
/// was, forever, carrying an unspent velocity. Asserting on `Velocity` here would pass either
/// way; asserting on `Transform` is what makes the test bite.
#[test]
fn explosion_moves_a_body_that_has_fallen_asleep() {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new());

    // Static ground, top face at y = 0.
    let ground = world.spawn();
    world.add_component(ground, RigidBody::new_static());
    world.add_component(ground, Collider::box_collider(Vec3::new(50.0, 0.5, 50.0)));
    world.add_component(ground, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(ground, Velocity::default());

    // Two awake unit boxes resting on it. `far` is the control: it settles and sleeps like
    // `target`, but sits well outside the blast, so it must still be asleep and unmoved
    // afterwards. Static bodies do not merge islands (`island.rs` unions only through dynamic
    // bodies), so the two never share one and cannot wake each other through the ground.
    let mut awake = RigidBody::new(1.0, true);
    awake.update_inertia_from_collider(&Collider::box_collider(Vec3::splat(0.5)));
    let target = spawn_body(&mut world, awake, Vec3::new(0.0, 0.5, 0.0));
    let far = spawn_body(&mut world, awake, Vec3::new(20.0, 0.5, 0.0));

    // Let both settle and fall asleep (240 Hz internally → 400 frames ≈ 1600 substeps,
    // against the 60-substep sleep requirement; same cadence as `world/tests.rs`).
    for _ in 0..400 {
        physics_step_system(&world, 1.0 / 60.0);
    }

    let (target_x0, far_x0) = {
        let bodies = world.borrow::<RigidBody>();
        assert!(
            bodies.get(target.id()).expect("target").is_sleeping,
            "scenario invalid: the target box never fell asleep, so there is nothing to wake"
        );
        assert!(
            bodies.get(far.id()).expect("far").is_sleeping,
            "scenario invalid: the control box never fell asleep"
        );
        let ts = world.borrow::<Transform>();
        (
            ts.get(target.id()).expect("target").position.x,
            ts.get(far.id()).expect("far").position.x,
        )
    };

    // Detonate, then keep stepping.
    spawn_blast(&mut world);
    physics_explosion_system(&world, 1.0 / 60.0);
    world.apply_commands();
    physics_step_system(&world, 1.0 / 60.0);

    {
        let bodies = world.borrow::<RigidBody>();
        assert!(
            !bodies.get(target.id()).expect("target").is_sleeping,
            "the blasted box must be awake after the first step"
        );
        assert!(
            bodies.get(far.id()).expect("far").is_sleeping,
            "the control box is 23 m from a 5 m blast — it must still be asleep"
        );
    }

    // One second of simulation. At 5 m/s into a μ≈0.5 contact the box coasts ≈2.5 m before
    // friction stops it; the 0.5 m bar is far below that, so the test keys on "the blast
    // landed", not on a tuned distance.
    for _ in 0..60 {
        physics_step_system(&world, 1.0 / 60.0);
    }

    let ts = world.borrow::<Transform>();
    let target_x1 = ts.get(target.id()).expect("target").position.x;
    let far_x1 = ts.get(far.id()).expect("far").position.x;

    assert!(
        target_x1 > target_x0 + 0.5,
        "the blast must actually MOVE the sleeping box: x {target_x0} -> {target_x1} \
         (unchanged means the impulse was written into a body the integrator skips)"
    );
    assert!(
        (far_x1 - far_x0).abs() < 1e-3,
        "the control box was never in range and must not have moved: x {far_x0} -> {far_x1}"
    );
}
