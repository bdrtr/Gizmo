//! Newton's-cradle contact characterisation.
//!
//! Written to chase a report that the cradle's balls pass through each other when swung
//! fast. **They do not**, in anything this harness can build: two elastic spheres head-on
//! up to 320 m/s, five touching spheres up to 320 m/s, a kinematic "hand" driven into
//! rope-hung neighbours, and the whole cradle — static beam, rope joints, gravity — at
//! every swing angle it can reach. The frame-hitch theory is dead too: `PhysicsWorld::step`
//! never integrates a bigger step when it runs out of substeps, it drops time instead.
//!
//! What the chase did find is here as `ccd_makes_a_bounce_a_thud`: **switching CCD on turns
//! a perfectly elastic collision into a perfectly inelastic one.** That is the first thing
//! anyone reaches for against tunnelling, and in a Newton's cradle it destroys the only
//! behaviour the scene exists to show.

use gizmo_math::Vec3;
use gizmo_physics_core::{
    components::{CombineMode, PhysicsMaterial},
    BodyHandle, Collider, Transform,
};
use gizmo_physics_rigid::{joints::Joint, PhysicsWorld, RigidBody, Velocity};

const DT: f32 = 1.0 / 60.0;
const R: f32 = 0.5;

fn elastic() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 1.0,
        static_friction: 0.0,
        dynamic_friction: 0.0,
        restitution_combine: CombineMode::Max,
        ..Default::default()
    }
}

fn ball(world: &mut PhysicsWorld, id: u32, x: f32, vx: f32, ccd: bool) {
    let mut rb = RigidBody::new(1.0, false);
    rb.ccd_enabled = ccd;
    rb.wake_up();
    world.add_body(
        BodyHandle::from_id(id),
        rb,
        Transform::new(Vec3::new(x, 0.0, 0.0)),
        Velocity::new(Vec3::new(vx, 0.0, 0.0)),
        Collider::sphere(R).with_material(elastic()),
    );
}

/// A row of touching elastic spheres never lets one through another, however fast the
/// first one arrives.
///
/// The speeds go two orders of magnitude past anything the demo can produce — its drag
/// servo clamps at 15 m/s and a 4 m pendulum tops out near 10 — precisely so that a
/// failure here would be about the contact and not about the scene.
#[test]
fn a_touching_row_never_lets_one_through() {
    const N: usize = 5;
    const GAP: f32 = 0.01;
    for &v in &[5.0f32, 20.0, 80.0, 320.0] {
        let mut world = PhysicsWorld::new();
        world.integrator.gravity = Vec3::ZERO;
        let spacing = 2.0 * R + GAP;
        ball(&mut world, 0, -spacing * 2.0, v, false);
        for i in 1..N {
            ball(&mut world, i as u32, (i as f32 - 1.0) * spacing, 0.0, false);
        }
        for _ in 0..240 {
            let _ = world.step(DT);
            for i in 1..N {
                let gap = world.transforms[i].position.x - world.transforms[i - 1].position.x;
                assert!(gap > 0.0, "balls swapped order at v={v}: gap {gap}");
            }
        }
    }
}

/// The cradle as the demo builds it — beam, ropes, gravity — keeps its balls out of each
/// other at every swing the scene can reach.
///
/// **Overlap is measured between centres, not along X.** A ball with enough speed loops
/// over its own pivot, and while it is up there its X passes its neighbours' with no
/// contact involved at all; an X-order test calls that a pass-through and is wrong. That
/// mistake was made once already while chasing this report.
#[test]
fn the_cradle_keeps_its_balls_apart() {
    const N: usize = 5;
    const GAP: f32 = 0.01;
    const L: f32 = 4.0;
    const PIVOT_Y: f32 = 6.0;
    for &angle_deg in &[55.0f32, 80.0, 110.0] {
        let mut world = PhysicsWorld::new();
        world.integrator.gravity = Vec3::new(0.0, -9.81, 0.0);
        let spacing = 2.0 * R + GAP;
        let start_x = -((N as f32 - 1.0) / 2.0) * spacing;

        let mut beam = RigidBody::new_static();
        beam.wake_up();
        world.add_body(
            BodyHandle::from_id(100),
            beam,
            Transform::new(Vec3::new(0.0, PIVOT_Y, 0.0)),
            Velocity::default(),
            Collider::box_collider(Vec3::new(N as f32 * spacing, 0.075, 0.075)),
        );

        for i in 0..N {
            let pivot = Vec3::new(start_x + i as f32 * spacing, PIVOT_Y, 0.0);
            let centre = if i == 0 {
                let a = angle_deg.to_radians();
                pivot + L * Vec3::new(-a.sin(), -a.cos(), 0.0)
            } else {
                pivot - Vec3::new(0.0, L, 0.0)
            };
            // `true` is `use_gravity`. Passing `false` here once left the whole cradle
            // standing still, and the run looked clean for entirely the wrong reason.
            let mut rb = RigidBody::new(1.0, true);
            rb.wake_up();
            world.add_body(
                BodyHandle::from_id(i as u32),
                rb,
                Transform::new(centre),
                Velocity::default(),
                Collider::sphere(R).with_material(elastic()),
            );
            world.joints.push(Joint::rope(
                BodyHandle::from_id(100),
                BodyHandle::from_id(i as u32),
                pivot - Vec3::new(0.0, PIVOT_Y, 0.0),
                Vec3::ZERO,
                L,
            ));
        }

        // Bodies are stored in insertion order: the beam is 0, the balls are 1..=N.
        let mut deepest = 0.0f32;
        for _ in 0..600 {
            let _ = world.step(DT);
            for i in 1..=N {
                for j in (i + 1)..=N {
                    let d = (world.transforms[i].position - world.transforms[j].position).length();
                    deepest = deepest.max((2.0 * R - d).max(0.0));
                }
            }
        }
        // A tenth of the radius: past the solver's ordinary recovery slack, and far short
        // of the half-diameter a real pass-through would need.
        assert!(
            deepest < 0.05,
            "cradle at {angle_deg}° let its balls overlap by {deepest:.3} m of 2R={:.2}",
            2.0 * R
        );
    }
}

/// **A known limitation, pinned as a test rather than left as folklore.** Equal masses,
/// restitution 1, head-on: the mover must stop dead and the target must leave with its
/// speed. It does — until CCD is switched on, and then the two share the momentum
/// equally, which is a perfectly *inelastic* collision.
///
/// Measured: 0.000 / 4.950 m/s with CCD off against **2.475 / 2.475** with it on, each
/// taking exactly half, and the same halving at 12 m/s.
///
/// **The cause is two steps, and the second one is the surprising half.** Enabling CCD on
/// a body does not only add the geometric backstop: at `solver/mod.rs` the island is
/// solved by `if self.use_tgs_soft && !has_ccd`, so **one CCD body drops its whole island
/// off the modern TGS-soft solver onto the older split-impulse path**. That path then
/// refuses restitution on a speculative contact — deliberately, with the reason written
/// beside it: a speculative contact is a gap-closing *limit*, and bouncing off it makes
/// the body decelerate inconsistently between substeps and overshoot the surface on the
/// last one. But CCD is exactly what creates speculative contacts, so the two rules meet
/// and the impact loses its bounce: the speculative constraint eats the approach velocity,
/// and by the time the contact is real there is nothing left to reflect.
///
/// Neither consequence is written down at [`RigidBody::with_ccd`], whose documentation
/// promises only that fast bodies will not tunnel. It matters most exactly here: reaching
/// for CCD to stop a Newton's cradle tunnelling silently removes the bounce the scene
/// exists to show.
///
/// Fixing it is not a one-line gate — moving the gate in `tgs.rs` changes nothing, because
/// a CCD island never reaches that file. It is either restitution on speculative contacts
/// in the split-impulse path (against a documented objection) or letting a CCD island keep
/// the TGS-soft solver (an architectural call with a determinism contract attached).
#[test]
#[ignore = "documents the CCD-loses-restitution limitation; un-ignore when it is fixed"]
fn ccd_makes_a_bounce_a_thud() {
    let v = 5.0f32;
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::ZERO;
    ball(&mut world, 0, -2.0, v, true);
    ball(&mut world, 1, 0.0, 0.0, true);
    for _ in 0..60 {
        let _ = world.step(DT);
    }
    let (mover, target) = (world.velocities[0].linear.x, world.velocities[1].linear.x);
    assert!(mover.abs() < 0.1, "an elastic hit should stop the mover, left it at {mover}");
    assert!(target > v * 0.9, "an elastic hit should pass the speed on, target got {target}");
}

/// **The reproduction, from a live run's own numbers.** Dragging — not speed, not a
/// frame hitch — is what buries the balls in each other. Measured in the demo: eight
/// overlap events, the deepest **0.272 m of a 1.00 m diameter**, at frame times of
/// 3.8-5.4 ms (200 fps, nothing stalling) and a top speed of exactly **15.0 m/s**, which
/// is the drag servo's own `clamp_length_max(15.0)`. No fling key was ever pressed.
///
/// The mechanism is the drag's design meeting its limit. While held, the ball is made
/// **kinematic** so that it pushes its neighbours — and a kinematic body is not pushed
/// back by anything. The servo sets `v = (target - here) * gain` every frame, so while
/// the ball is blocked the error stays large and it keeps being driven into the
/// obstruction at the clamp speed, indefinitely. The neighbour it is pushing hangs from a
/// rope and can only escape along its own arc.
///
/// An earlier attempt at this test drove the kinematic body in a straight line at a
/// constant velocity and found nothing, because that body stops caring once it has
/// arrived. What reproduces it is a *servo whose target is on the far side of the chain*.
#[test]
fn dragging_a_ball_into_the_others_buries_it() {
    const N: usize = 5;
    const GAP: f32 = 0.01;
    const L: f32 = 4.0;
    const PIVOT_Y: f32 = 6.0;
    const DRAG_GAIN: f32 = 18.0;
    const DRAG_CLAMP: f32 = 15.0;

    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::new(0.0, -9.81, 0.0);
    let spacing = 2.0 * R + GAP;
    let start_x = -((N as f32 - 1.0) / 2.0) * spacing;

    let mut beam = RigidBody::new_static();
    beam.wake_up();
    world.add_body(
        BodyHandle::from_id(100),
        beam,
        Transform::new(Vec3::new(0.0, PIVOT_Y, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(N as f32 * spacing, 0.075, 0.075)),
    );

    for i in 0..N {
        let pivot = Vec3::new(start_x + i as f32 * spacing, PIVOT_Y, 0.0);
        let mut rb = if i == 0 { RigidBody::new_kinematic() } else { RigidBody::new(1.0, true) };
        rb.use_gravity = i != 0;
        rb.wake_up();
        world.add_body(
            BodyHandle::from_id(i as u32),
            rb,
            Transform::new(pivot - Vec3::new(0.0, L, 0.0)),
            Velocity::default(),
            Collider::sphere(R).with_material(elastic()),
        );
        world.joints.push(Joint::rope(
            BodyHandle::from_id(100),
            BodyHandle::from_id(i as u32),
            pivot - Vec3::new(0.0, PIVOT_Y, 0.0),
            Vec3::ZERO,
            L,
        ));
    }

    // **The hand does not hold still — it swings.** A fixed target only buries the ball by
    // 0.061 m, a quarter of what the live run showed; what the report described was
    // swinging the ball about, which drags it back and forth *through* the row rather than
    // pressing it against one side. Modelled as the mouse sweeping across the cradle at a
    // couple of hertz, which is roughly a hand's pace.
    let left = Vec3::new(start_x - spacing, PIVOT_Y - L, 0.0);
    let right = Vec3::new(start_x + 3.0 * spacing, PIVOT_Y - L, 0.0);
    let mut deepest = 0.0f32;
    for frame in 0..300 {
        let phase = (frame as f32 * DT * 2.0 * std::f32::consts::PI * 1.5).sin();
        let target = left.lerp(right, phase.mul_add(0.5, 0.5));
        let here = world.transforms[1].position;
        world.velocities[1].linear = ((target - here) * DRAG_GAIN).clamp_length_max(DRAG_CLAMP);
        world.velocities[1].angular = Vec3::ZERO;
        let _ = world.step(DT);
        for i in 1..=N {
            for j in (i + 1)..=N {
                let d = (world.transforms[i].position - world.transforms[j].position).length();
                deepest = deepest.max((2.0 * R - d).max(0.0));
            }
        }
    }
    println!("held-and-shoved: deepest overlap {deepest:.3} m of 2R={:.2}", 2.0 * R);
    assert!(deepest > 0.1, "expected the live run's burial, got only {deepest:.3} m");
}
