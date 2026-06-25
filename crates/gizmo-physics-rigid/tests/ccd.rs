//! CCD (continuous collision detection) robustness suite — Faz 4.
//!
//! Speculative-contact CCD must satisfy two contracts that a single-frame check
//! cannot capture, so every test here drives the full public `PhysicsWorld::step`
//! pipeline over many frames:
//!
//!   * **NO TUNNELLING** — a fast body never crosses to the far side of thin
//!     geometry, regardless of speed / wall thickness / body size.
//!   * **NO GHOST WALL** — CCD must not perturb a body that has a clear path or is
//!     separating, and must not freeze an impacting body far short of the surface
//!     (the old `penetration = 0` speculative bug stopped it at its *start-of-frame*
//!     position; the fix lets it advance up to the wall and stop there).
//!
//! Together these are *discriminating*: removing CCD makes the bullet tunnel
//! (peak x ≫ 0); the old freeze-bug makes it rest ~5 m short of the wall.

use gizmo_core::entity::Entity;
use gizmo_math::Vec3;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};
use proptest::prelude::*;

const DT: f32 = 1.0 / 60.0;

/// Gravity-free world with a thin static wall centred at the origin, `half_thick`
/// along X (front face at `-half_thick`, far face at `+half_thick`). Wall is index 0.
fn wall_world(half_thick: f32) -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::ZERO;

    let mut wall = RigidBody::new_static();
    wall.wake_up();
    world.add_body(
        Entity::new(0, 0),
        wall,
        Transform::new(Vec3::ZERO),
        Velocity::default(),
        Collider::box_collider(Vec3::new(half_thick, 10.0, 10.0)),
    );
    world
}

/// Append a CCD sphere "bullet" with the given id, position, velocity and radius.
fn add_bullet(world: &mut PhysicsWorld, id: u32, pos: Vec3, vel: Vec3, radius: f32) {
    let mut rb = RigidBody::new(1.0, false);
    rb.ccd_enabled = true;
    rb.wake_up();
    world.add_body(
        Entity::new(id, 0),
        rb,
        Transform::new(pos),
        Velocity::new(vel),
        Collider::sphere(radius),
    );
}

/// Head-on impacts across a wide speed range never tunnel and never freeze short.
#[test]
fn fast_bullet_never_tunnels_thin_wall() {
    let half = 0.05; // 0.1 m thick wall — far thinner than one frame of travel
    let radius = 0.2;
    let front_face = -half; // x of the near surface
    let rest_x = front_face - radius; // where the bullet centre should come to rest

    for &speed in &[200.0_f32, 600.0, 1200.0, 3000.0] {
        let mut world = wall_world(half);
        add_bullet(&mut world, 1, Vec3::new(-5.0, 0.0, 0.0), Vec3::new(speed, 0.0, 0.0), radius);

        let mut max_x = f32::MIN;
        for _ in 0..240 {
            let _ = world.step(DT);
            max_x = max_x.max(world.transforms[1].position.x);
        }

        // Never crossed the wall centre (a tunnelling bullet would reach x ≈ +15).
        assert!(max_x < 0.0, "speed {speed}: tunnelled, peak x = {max_x}");

        let fx = world.transforms[1].position.x;
        // Rests against the front face (not frozen ~5 m short — the old ghost bug).
        assert!(
            (fx - rest_x).abs() < 0.1,
            "speed {speed}: bullet should rest at x≈{rest_x}, got {fx} \
             (ghost-freeze bug would leave it near -5.0)"
        );
        let v = world.velocities[1].linear.x;
        assert!(v.abs() < 1.0, "speed {speed}: bullet did not stop, vel.x = {v}");
    }
}

/// A CCD body with nothing in its path must travel its full ballistic distance —
/// CCD must not invent a phantom obstacle.
#[test]
fn ccd_body_with_clear_path_is_not_stopped() {
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::ZERO;
    add_bullet(&mut world, 1, Vec3::new(-5.0, 0.0, 0.0), Vec3::new(600.0, 0.0, 0.0), 0.2);

    for _ in 0..60 {
        let _ = world.step(DT); // 1 s
    }
    let x = world.transforms[0].position.x; // only body ⇒ index 0
    // 600 m/s · 1 s = 600 m of travel from x = -5.
    assert!(x > 500.0, "CCD body was wrongly slowed with a clear path: x = {x}");
}

/// A CCD body moving *away* from nearby geometry must not be held by a speculative
/// contact (separating pairs are gated out by the closing-velocity check).
#[test]
fn ccd_does_not_hold_separating_body() {
    let mut world = wall_world(0.1);
    // Just in front of the wall, flying away from it (−X).
    add_bullet(&mut world, 1, Vec3::new(-0.5, 0.0, 0.0), Vec3::new(-600.0, 0.0, 0.0), 0.2);

    for _ in 0..30 {
        let _ = world.step(DT);
    }
    let x = world.transforms[1].position.x;
    assert!(x < -100.0, "separating CCD body was wrongly held near the wall: x = {x}");
}

/// Two CCD bodies closing head-on at high speed meet but never pass through.
#[test]
fn two_fast_ccd_bodies_do_not_pass_through() {
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::ZERO;
    add_bullet(&mut world, 1, Vec3::new(-5.0, 0.0, 0.0), Vec3::new(400.0, 0.0, 0.0), 0.2); // index 0
    add_bullet(&mut world, 2, Vec3::new(5.0, 0.0, 0.0), Vec3::new(-400.0, 0.0, 0.0), 0.2); // index 1

    let mut min_gap = f32::MAX;
    for _ in 0..240 {
        let _ = world.step(DT);
        let xa = world.transforms[0].position.x;
        let xb = world.transforms[1].position.x;
        // A starts left of B and must never swap sides.
        assert!(xa <= xb + 0.05, "bodies passed through each other: xa={xa} xb={xb}");
        min_gap = min_gap.min(xb - xa);
    }
    // They must actually have met (centres within ~2·r + skin), not stopped early.
    assert!(min_gap < 0.6, "bodies never met — min centre gap {min_gap}");
}

/// An off-centre, angled high-speed impact still cannot tunnel (the normal comes
/// from the GJK closest feature, not the centre line, so the Y offset/slide is fine).
#[test]
fn offset_angled_impact_does_not_tunnel() {
    let mut world = wall_world(0.1);
    add_bullet(&mut world, 1, Vec3::new(-5.0, 3.0, 0.0), Vec3::new(1500.0, -200.0, 0.0), 0.2);

    let mut max_x = f32::MIN;
    for _ in 0..240 {
        let _ = world.step(DT);
        max_x = max_x.max(world.transforms[1].position.x);
    }
    assert!(max_x < 0.0, "offset impact tunnelled: peak x = {max_x}");
}

/// Regression: turning CCD on must not change ordinary resting behaviour. A dropped
/// CCD sphere settles on the ground exactly like a discrete body (the speculative
/// path lands it cleanly instead of freezing it above the floor).
#[test]
fn ccd_body_settles_on_ground_like_discrete() {
    let mut world = PhysicsWorld::new();
    world.integrator.gravity = Vec3::new(0.0, -9.81, 0.0);

    let mut ground = RigidBody::new_static();
    ground.wake_up();
    world.add_body(
        Entity::new(0, 0),
        ground,
        Transform::new(Vec3::new(0.0, -1.0, 0.0)), // top surface at y = 0
        Velocity::default(),
        Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)),
    );

    let mut rb = RigidBody::new(1.0, true);
    rb.ccd_enabled = true;
    rb.wake_up();
    world.add_body(
        Entity::new(1, 0),
        rb,
        Transform::new(Vec3::new(0.0, 3.0, 0.0)),
        Velocity::default(),
        Collider::sphere(0.5),
    );

    for _ in 0..300 {
        let _ = world.step(DT); // 5 s
    }
    let y = world.transforms[1].position.y;
    assert!((y - 0.5).abs() < 0.1, "CCD sphere did not settle on the ground: y = {y}");
    let v = world.velocities[1].linear.length();
    assert!(v < 0.2, "CCD sphere never came to rest: |v| = {v}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// For any high speed, any thin wall and any body size, a head-on CCD bullet
    /// never tunnels: its centre never crosses the wall's far face, and it comes
    /// to rest on the near side. (Extreme geometry — a body far larger than the
    /// wall — may briefly poke a surface through during the multi-sub-step settle,
    /// but the centre stays on the near side and the discrete solver recovers it;
    /// the genuine no-tunnel invariant is about the centre, not transient overlap.)
    #[test]
    fn prop_ccd_never_tunnels(
        speed in 150.0f32..3000.0,
        half_thick in 0.02f32..0.4,
        radius in 0.05f32..0.4,
    ) {
        let mut world = wall_world(half_thick);
        add_bullet(&mut world, 1, Vec3::new(-8.0, 0.0, 0.0), Vec3::new(speed, 0.0, 0.0), radius);

        // A tunnelling bullet shoots to large +x — its centre would cross the far
        // face (+half_thick). Require the centre to stay strictly before it.
        for _ in 0..480 {
            let _ = world.step(DT);
            let x = world.transforms[1].position.x;
            prop_assert!(
                x < half_thick,
                "tunnelled: speed={speed} half={half_thick} r={radius} x={x}"
            );
        }
        // And it must settle on the near side (rest ≈ -half_thick - radius < 0):
        // stopped by the wall, not lodged through or drifting on.
        let final_x = world.transforms[1].position.x;
        prop_assert!(
            final_x < 0.0,
            "did not settle on the near side: speed={speed} half={half_thick} r={radius} final_x={final_x}"
        );
    }
}
