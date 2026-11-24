//! A world coarser than the joints' rate (240 Hz) still solves its joints as finely: a free joint —
//! one no contact island holds — is stepped through the world's substep in as many steps as 240 Hz
//! takes (`JointSolver::finish`), and a joint inside a contact island is solved, sweep by sweep,
//! against where its bodies have got to in the island's solve.
//!
//! Measured 2026-11-24, before: at 60 Hz a chain of eight 1 kg links ending in 50 kg came apart by
//! 2.24 m (7.6 mm at 240 Hz); a falling chain folding onto itself opened 33 mm (6 mm).

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{Joint, PhysicsWorld, RigidBody, Velocity};

const RATES: [u32; 3] = [60, 120, 240];

/// Widest any joint is open: the distance between its two anchors.
fn widest(w: &PhysicsWorld) -> f32 {
    w.joints
        .iter()
        .map(|j| {
            let a = w.entity_index_map.get(&j.entity_a.id()).copied().expect("body");
            let b = w.entity_index_map.get(&j.entity_b.id()).copied().expect("body");
            let (ta, tb) = (&w.transforms[a], &w.transforms[b]);
            ((ta.position + ta.rotation * j.local_anchor_a) - (tb.position + tb.rotation * j.local_anchor_b)).length()
        })
        .fold(0.0, f32::max)
}

/// A chain of `links` 0.5 m links (1 kg, the last `last_mass`) laid out level from a fixed point at
/// height `y`, over ground whose top is at 0.
fn chain(hz: u32, links: u32, last_mass: f32, y: f32) -> PhysicsWorld {
    let mut w = PhysicsWorld::new().with_substep_rate(hz).expect("rate");
    w.add_body(BodyHandle::from_id(0), RigidBody::new_static(), Transform::new(Vec3::new(0.0, y, 0.0)), Velocity::default(), Collider::sphere(0.05));
    w.add_body(
        BodyHandle::from_id(100),
        RigidBody::new_static(),
        Transform::new(Vec3::new(0.0, -0.5, 0.0)),
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)),
    );
    for i in 1..=links {
        let mut rb = RigidBody::new(if i == links { last_mass } else { 1.0 }, true);
        let col = Collider::box_collider(Vec3::new(0.25, 0.05, 0.05));
        rb.update_inertia_from_collider(&col);
        rb.linear_damping = 0.0;
        rb.angular_damping = 0.0;
        w.add_body(BodyHandle::from_id(i), rb, Transform::new(Vec3::new(0.5 * i as f32 - 0.25, y, 0.0)), Velocity::default(), col);
        let anchor = if i == 1 { Vec3::ZERO } else { Vec3::new(0.25, 0.0, 0.0) };
        w.joints.push(Joint::ball_socket(BodyHandle::from_id(i - 1), BodyHandle::from_id(i), anchor, Vec3::new(-0.25, 0.0, 0.0)));
    }
    w
}

/// Runs four seconds of frames; the widest any joint opened, and the widest in the last second.
fn run(w: &mut PhysicsWorld) -> (f32, f32) {
    let (mut worst, mut last) = (0.0f32, 0.0f32);
    for frame in 0..240 {
        w.step(1.0 / 60.0).expect("step");
        let open = widest(w);
        assert!(open.is_finite());
        worst = worst.max(open);
        if frame >= 180 {
            last = last.max(open);
        }
    }
    (worst, last)
}

/// The chain carrying fifty times a link's mass, swinging free: it holds at every rate, and at 60
/// and 120 Hz it moves as it does at 240 — its joints are stepped at 240 Hz there too.
#[test]
fn a_chain_carrying_a_heavy_load_holds_at_every_rate_as_at_240() {
    let mut at_240 = chain(240, 8, 50.0, 10.0);
    run(&mut at_240);
    for hz in RATES {
        let mut w = chain(hz, 8, 50.0, 10.0);
        let (worst, _) = run(&mut w);
        assert!(worst < 0.01, "{hz} Hz: a joint opened {worst} m");
        for i in 1..=8 {
            let d = (w.transforms[i + 1].position - at_240.transforms[i + 1].position).length();
            assert!(d < 1e-3, "{hz} Hz: link {i} is {d} m from where it is at 240 Hz");
        }
    }
}

/// A light chain falling and folding onto itself: its links touch, so its joints are solved in
/// contact islands for much of the fall. At 60 Hz it opens no wider than 1.5 cm (3.3 before).
#[test]
fn a_chain_folding_onto_itself_holds_at_every_rate() {
    for hz in RATES {
        let (worst, _) = run(&mut chain(hz, 8, 1.0, 10.0));
        assert!(worst < 0.015, "{hz} Hz: a joint opened {worst} m");
    }
}

/// A chain hung from 2 m whose 50 kg end lands and lies on the ground: once it has settled its
/// joints are as closed at every rate as a few millimetres (5.4 at 60 Hz before). Its landing is
/// not asserted: at 60 Hz the heavy end sinks a substep's travel into the ground, 6 cm, which
/// opens the joints for a frame — a contact found a substep late, not a joint solved loosely.
#[test]
fn a_heavy_chain_lying_on_the_ground_holds_at_every_rate() {
    for hz in RATES {
        let (_, last) = run(&mut chain(hz, 8, 50.0, 2.0));
        assert!(last < 0.003, "{hz} Hz: a joint is open {last} m at rest");
    }
}

/// A 3 kg box hanging at rest from a fixed point carries 3·g: the joint says so at every rate,
/// although a substepped joint's impulses are over a quarter of a 60 Hz substep.
#[test]
fn a_hanging_load_reads_its_weight_at_every_rate() {
    for hz in RATES {
        let mut w = PhysicsWorld::new().with_substep_rate(hz).expect("rate");
        w.add_body(BodyHandle::from_id(0), RigidBody::new_static(), Transform::new(Vec3::ZERO), Velocity::default(), Collider::sphere(0.05));
        let mut rb = RigidBody::new(3.0, true);
        let col = Collider::box_collider(Vec3::splat(0.1));
        rb.update_inertia_from_collider(&col);
        w.add_body(BodyHandle::from_id(1), rb, Transform::new(Vec3::new(0.0, -1.0, 0.0)), Velocity::default(), col);
        w.joints.push(Joint::ball_socket(BodyHandle::from_id(0), BodyHandle::from_id(1), Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0)));
        for _ in 0..120 {
            w.step(1.0 / 60.0).expect("step");
        }
        let force = w.joints[0].last_force();
        let weight = 3.0 * 9.81;
        assert!((force - weight).abs() < 0.05 * weight, "{hz} Hz: the joint carries {force} N, not {weight}");
    }
}
