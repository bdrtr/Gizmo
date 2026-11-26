//! Coins stacked stand: a coin lies on another along a patch of its face (`round_face` in the
//! narrow phase), not on one point in the middle of their overlap, which it tipped on. Of twenty
//! stacks of five 3 mm coins at 240 Hz, none stood before (2026-11-26).

use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

fn rnd(s: &mut u32) -> f32 {
    *s ^= *s << 13;
    *s ^= *s >> 17;
    *s ^= *s << 5;
    (*s >> 8) as f32 / (1u32 << 24) as f32
}

#[test]
fn stacks_of_coins_stand() {
    const STACKS: usize = 20;
    const COINS: usize = 5;
    let (radius, half) = (0.012f32, 0.0015f32);
    let coin = Collider::cylinder(radius, half);
    let tall = half + 2.0 * half * (COINS - 1) as f32;
    let mut seed = 5u32;
    let mut standing = 0;
    for _ in 0..STACKS {
        let mut w = PhysicsWorld::new();
        w.add_body(BodyHandle::from_id(1), RigidBody::new_static(), Transform::new(Vec3::new(0.0, -0.05, 0.0)), Velocity::default(), Collider::box_collider(Vec3::new(0.2, 0.05, 0.2)));
        for k in 0..COINS {
            let mut rb = RigidBody::new(0.005, true);
            rb.update_inertia_from_collider(&coin);
            // Each a little off the one under it, turned anyhow about its axis.
            let mut t = Transform::new(Vec3::new(0.004 * (rnd(&mut seed) - 0.5), half + (2.0 * half + 2e-4) * k as f32, 0.004 * (rnd(&mut seed) - 0.5)));
            t.rotation = Quat::from_rotation_y(6.3 * rnd(&mut seed));
            w.add_body(BodyHandle::from_id(10 + k as u32), rb, t, Velocity::default(), coin.clone());
        }
        for _ in 0..300 {
            w.step(1.0 / 60.0).unwrap();
        }
        let top = w.entity_index_map.get(&(10 + COINS as u32 - 1)).copied().unwrap();
        if w.transforms[top].position.y > 0.85 * tall {
            standing += 1;
        }
    }
    // 19 of 20 now; none before.
    assert!(standing >= 18, "{standing} of {STACKS} stacks of {COINS} coins stand");
}
