//! A body thinner than the solver's slop rests on its face, not in what it rests on.
//!
//! The slop (5 mm) is how deep a contact may rest without being pushed out, and a coin is 2 mm thick:
//! spun onto the floor it met it 2 mm deep within one substep, was held there as resting within the
//! slop, and fell asleep wholly inside the floor (2026-11-26). A thin body's contacts take a slop of a
//! quarter of its half thickness (`PhysicsWorld::body_slop`).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn rnd(s: &mut u32) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    (*s >> 8) as f32 / (1u32 << 24) as f32
}

/// Coins of radius 12.5 mm and `2·half_height` thick, thrown spinning onto a floor at `hz`: where
/// each comes to rest — its height and whether its face is up — and whether it fell asleep.
fn throw_coins(hz: u32, half_height: f32, throws: usize) -> Vec<(f32, f32, bool)> {
    let mut seed = 99u32;
    (0..throws)
        .map(|_| {
            let mut w = PhysicsWorld::new().with_substep_rate(hz).unwrap();
            w.add_body(BodyHandle::from_id(0), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -0.5, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(20.0, 0.5, 20.0)));
            let c = Collider::cylinder(0.0125, half_height);
            let mut rb = RigidBody::new(0.005, true);
            rb.update_inertia_from_collider(&c);
            let mut t = Transform::new(Vec3::new(0.0, 0.3 + 0.0375 * rnd(&mut seed), 0.0));
            t.rotation = Quat::from_euler(gizmo_math::EulerRot::XYZ, 6.3 * rnd(&mut seed), 6.3 * rnd(&mut seed), 6.3 * rnd(&mut seed));
            let mut v = Velocity::new(Vec3::new(rnd(&mut seed) - 0.5, rnd(&mut seed), rnd(&mut seed) - 0.5));
            v.angular = Vec3::new(rnd(&mut seed) - 0.5, rnd(&mut seed) - 0.5, rnd(&mut seed) - 0.5) * 60.0;
            w.add_body(BodyHandle::from_id(1), rb, t, v, c);
            for _ in 0..600 {
                w.step(1.0 / 60.0).unwrap();
            }
            let t = w.transforms[1];
            (t.position.y, (t.rotation * Vec3::Y).y.abs(), w.rigid_bodies[1].is_sleeping)
        })
        .collect()
}

#[test]
fn a_coin_spun_onto_the_floor_rests_on_it_not_in_it() {
    for (hz, half_height) in [(240u32, 0.001f32), (240, 0.0006), (60, 0.001)] {
        for (k, (y, up, asleep)) in throw_coins(hz, half_height, 12).into_iter().enumerate() {
            assert!(asleep, "{hz} Hz, {half_height} m: coin {k} never came to rest");
            assert!(up > 0.99, "{hz} Hz, {half_height} m: coin {k} rests off its face ({up})");
            // At most a quarter of its half thickness in (its slop) and a little for the soft contact
            // at 60 Hz: the coin's centre stays above the floor by most of its half thickness.
            assert!(y > 0.6 * half_height, "{hz} Hz, {half_height} m: coin {k} rests {:.2} mm in", (half_height - y) * 1e3);
        }
    }
}
