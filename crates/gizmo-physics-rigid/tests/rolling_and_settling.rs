//! Whether round things come to rest, and what it costs to make them.
//!
//! # Why this file exists
//!
//! `PhysicsMaterial::rolling_friction` was added because a pile of spheres never settled. Dropped
//! into a heap they would creep and jostle indefinitely: contact friction acts at the contact
//! patch and opposes *sliding*, and a rolling ball is not sliding, so nothing in the solver was
//! ever going to take the last of its spin away. The pile stayed awake, the island stayed active,
//! and the frame cost stayed with it — 9.50 ms for a scene that should have gone quiet.
//!
//! The defect had been there the whole time and **no test caught it**, for a reason worth writing
//! down rather than fixing quietly: every stability scene in this crate is made of boxes. Towers,
//! rafts, lattices, crate piles — a box that stops sliding stops, so box scenes cannot express
//! the failure. The engine had a whole shape class with no settling coverage, and it took a
//! comparison against another engine to notice.
//!
//! So this file is not "tests for the rolling friction feature". It is the sphere-settling
//! coverage the suite never had, with the feature's own guards alongside it.
//!
//! # What is asserted, and why each one is here
//!
//! - **A pile of spheres settles.** The original defect, stated as an outcome rather than as a
//!   mechanism: after ten seconds nothing in the heap is still moving. It is deliberately not an
//!   assertion about spin, or about the friction term, or about how the solver spends its
//!   iterations — any future mechanism that gets the pile to stop passes.
//! - **Without it, they do not.** The companion measurement. A guard whose scene would settle
//!   anyway guards nothing, and this is what says the first assertion is testing the thing it
//!   claims to. It is also the closest thing to a record of the original bug.
//! - **A rolling ball still rolls.** The failure mode of the fix rather than of the bug. A term
//!   that stops a pile by braking everything round would pass the first two and ruin the engine
//!   for every game with a ball in it.
//! - **The default is off.** `rolling_friction` defaults to 0.0 and every preset leaves it there,
//!   so no existing scene changed behaviour when it landed. That is a compatibility claim and it
//!   belongs in a test, because the day someone "sensibly" gives asphalt a default the golden
//!   hashes move and the cause will not be obvious.
//!
//! # What the coefficient is worth, measured
//!
//! The number is not a rolling-resistance coefficient in the textbook sense and should not be
//! chosen as though it were. Swept against a rolling ball (5 m/s, two seconds) and against the
//! pile below:
//!
//! | `rolling_friction` | measured deceleration | textbook `c·g` | pile after 10 s |
//! |---|---|---|---|
//! | 0.005 | 0.18 m/s² | 0.05 | still moving |
//! | 0.01  | 0.25 m/s² | 0.10 | still moving |
//! | 0.02  | 0.39 m/s² | 0.20 | still moving |
//! | 0.05  | 0.80 m/s² | 0.49 | **at rest** |
//! | 0.1   | 1.48 m/s² | 0.98 | at rest |
//! | 0.2   | 2.50 m/s² | 1.96 | at rest |
//!
//! Two things follow, and both are the reason this table is here rather than in a commit message.
//! **A pile needs about 0.05 to settle inside ten seconds** — below that the term is real but too
//! weak to finish the job, so a value picked for physical realism (a hard ball on hard ground is
//! 0.001-0.01) will not stabilise anything. And **the cost is consistently above `c·g`**, about
//! 1.5× at the settling end and worse at the small end, where a floor of roughly 0.15 m/s² that
//! is not rolling friction at all dominates. So it is a stabiliser with a physical flavour, not a
//! model of rolling resistance, and the tests below are calibrated at the 0.05 a pile actually
//! needs rather than at a round number.

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, PhysicsMaterial, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

const DT: f32 = 1.0 / 60.0;
const R: f32 = 0.5;

/// Below this, in m/s, a body is taken to have stopped. Chosen against the sleep threshold rather
/// than against zero: a scene that has gone quiet enough for the engine to sleep it has settled by
/// any definition the engine itself uses.
const AT_REST: f32 = 0.05;

/// The smallest rolling friction measured to settle the pile below inside ten seconds. See the
/// table in the module docs — 0.02 leaves it moving at 1.36 m/s and 0.05 stops it dead.
const SETTLES: f32 = 0.05;

fn ground(world: &mut PhysicsWorld, material: PhysicsMaterial) {
    let mut g = RigidBody::new_static();
    g.wake_up();
    world.add_body(
        BodyHandle::from_id(0),
        g,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(30.0, 1.0, 30.0)).with_material(material),
    );
}

fn sphere(world: &mut PhysicsWorld, id: u32, at: Vec3, v: Velocity, material: PhysicsMaterial) {
    let mut rb = RigidBody::new(1.0, true);
    rb.wake_up();
    let col = Collider::sphere(R).with_material(material);
    rb.update_inertia_from_collider(&col);
    world.add_body(BodyHandle::from_id(id), rb, Transform::new(at), v, col);
}

/// Thirty spheres dropped into a heap, in a material that is otherwise ordinary.
///
/// Laid out on a 5×6 grid at a spacing just under a diameter so they arrive already touching and
/// have to sort themselves out, which is what a pile is. Restitution is 0 — the question is
/// whether they *stop*, and bouncing balls that have not finished bouncing would answer a
/// different one.
fn pile(rolling: f32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    let material = PhysicsMaterial {
        restitution: 0.0,
        rolling_friction: rolling,
        ..Default::default()
    };
    ground(&mut world, material);
    let mut id = 1;
    for i in 0..5 {
        for j in 0..6 {
            let at = Vec3::new(
                (i as f32 - 2.0) * 0.9 * R,
                R + j as f32 * 1.6 * R,
                (j as f32 - 2.5) * 0.9 * R,
            );
            sphere(&mut world, id, at, Velocity::default(), material);
            id += 1;
        }
    }
    world
}

/// The fastest dynamic body, ignoring the static ground.
fn fastest(world: &PhysicsWorld) -> f32 {
    world
        .velocities
        .iter()
        .skip(1)
        .map(|v| v.linear.length())
        .fold(0.0f32, f32::max)
}

fn settle(world: &mut PhysicsWorld, seconds: f32) -> f32 {
    for _ in 0..(seconds / DT) as usize {
        world.step(DT).ok();
    }
    fastest(world)
}

#[test]
fn a_pile_of_spheres_comes_to_rest() {
    let mut world = pile(SETTLES);
    let moving = settle(&mut world, 10.0);
    assert!(
        moving < AT_REST,
        "a heap of spheres should be still after ten seconds, fastest was {moving:.3} m/s"
    );
}

/// The defect, kept as a measurement rather than as a memory.
///
/// This is the state the engine shipped in: the same pile, the same ten seconds, and something in
/// it still moving. If a future change makes spheres settle on their own then this assertion
/// fails, and that failure is *good news* — but it must be read and deleted deliberately, not
/// worked around, because it is also the only thing establishing that the test above has teeth.
#[test]
fn without_rolling_friction_it_does_not() {
    let mut world = pile(0.0);
    let moving = settle(&mut world, 10.0);
    assert!(
        moving >= AT_REST,
        "the pile settled with no rolling friction ({moving:.3} m/s) — the guard above is now \
         testing nothing, so read this test's docs before touching either"
    );
}

/// A ball rolling on the flat keeps rolling.
///
/// The way the fix could be worse than the bug. Rolling friction is a resistance to *spin change*
/// bounded by the normal force, not a brake on the body, and a term that failed to be bounded —
/// or that leaked into the linear channel — would stop a pile beautifully and make every ball in
/// every game feel like it was rolling through sand.
///
/// Five metres per second for two seconds at [`SETTLES`] — the coefficient a pile actually needs,
/// which is the only one worth defending. It keeps about two thirds of its speed there; the
/// assertion is deliberately loose about how much is lost and strict about the ball still being
/// on its way, because the number that matters is the shape of the loss and not its third digit.
#[test]
fn rolling_friction_does_not_brake_a_rolling_ball() {
    let mut world = PhysicsWorld::new();
    let material = PhysicsMaterial {
        restitution: 0.0,
        rolling_friction: SETTLES,
        ..Default::default()
    };
    ground(&mut world, material);
    // Spun to match its own travel, so it is rolling rather than skidding: ω = v / r about the
    // axis across the direction of travel. A skidding ball would lose speed to contact friction
    // and the test would be measuring that instead.
    let v = 5.0;
    sphere(
        &mut world,
        1,
        Vec3::new(-10.0, R, 0.0),
        Velocity {
            linear: Vec3::new(v, 0.0, 0.0),
            angular: Vec3::new(0.0, 0.0, -v / R),
            ..Default::default()
        },
        material,
    );
    for _ in 0..120 {
        world.step(DT).ok();
    }
    let kept = world.velocities[1].linear.length();
    assert!(
        kept > 0.5 * v,
        "a rolling ball kept only {kept:.2} m/s of {v:.1} after two seconds — rolling friction is \
         acting as a brake, not as a resistance"
    );
}

/// Nothing gained rolling friction by default when it was added.
#[test]
fn every_preset_leaves_rolling_friction_off() {
    assert_eq!(PhysicsMaterial::default().rolling_friction, 0.0);
    for (name, m) in [
        ("ASPHALT", PhysicsMaterial::ASPHALT),
        ("ICE", PhysicsMaterial::ICE),
        ("RUBBER", PhysicsMaterial::RUBBER),
        ("WOOD", PhysicsMaterial::WOOD),
        ("METAL", PhysicsMaterial::METAL),
    ] {
        assert_eq!(
            m.rolling_friction, 0.0,
            "{name} gained a rolling friction default — every scene built on it has just changed \
             behaviour, and the golden hashes will say so without saying why"
        );
    }
}
