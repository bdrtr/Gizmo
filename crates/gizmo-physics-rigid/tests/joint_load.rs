//! **What a joint carries is readable.** `Joint::last_force` / `last_torque` / `load` report the
//! reaction the solver measures against the break thresholds — it was `pub(crate)`, so a game
//! could not show how close a beam was to giving way.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::world::PhysicsWorld;

/// A 10 kg weight hung 2 m below a fixed point on a ball socket; returns the world after
/// `steps` (long enough for it to settle and fall asleep).
fn hanging(break_force: f32, steps: usize) -> World {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let anchor = world.spawn();
    world.add_component(anchor, Transform::new(Vec3::new(0.0, 10.0, 0.0)));
    world.add_component(anchor, RigidBody::new_static());
    world.add_component(anchor, Velocity::default());
    world.add_component(anchor, Collider::sphere(0.1));
    let weight = world.spawn();
    world.add_component(weight, Transform::new(Vec3::new(0.0, 8.0, 0.0)));
    world.add_component(weight, RigidBody::new(10.0, true));
    world.add_component(weight, Velocity::default());
    world.add_component(weight, Collider::sphere(0.2));
    let mut joint = Joint::ball_socket(BodyHandle::from_id(anchor.id()), BodyHandle::from_id(weight.id()), Vec3::new(0.0, -2.0, 0.0), Vec3::ZERO);
    joint.break_force = break_force;
    world.get_resource_mut::<PhysicsWorld>().unwrap().joints.push(joint);
    for _ in 0..steps {
        gizmo_physics_rigid::system::physics_step_system(&world, 1.0 / 120.0);
        world.apply_commands();
    }
    world
}

#[test]
fn a_hanging_weight_loads_its_joint_with_its_weight_even_asleep() {
    let world = hanging(200.0, 240);
    let physics = world.get_resource::<PhysicsWorld>().unwrap();
    let rope = &physics.joints[0];
    let force = rope.last_force();
    eprintln!("force {force} load {} broken {}", rope.load(), rope.is_broken);
    assert!(!rope.is_broken, "98 N on a 200 N joint — it used to read 196 N and break at 100");
    assert!((force - 98.1).abs() < 0.05 * 98.1, "10 kg hangs with ~98 N: {force}");
    assert!((rope.load() - force / 200.0).abs() < 1e-6, "load is force over the threshold");
    assert!(!rope.is_broken);
}

#[test]
fn a_joint_too_weak_for_the_weight_breaks_at_a_load_above_one() {
    let world = hanging(50.0, 240);
    let physics = world.get_resource::<PhysicsWorld>().unwrap();
    let rope = &physics.joints[0];
    eprintln!("broken {} force {} load {}", rope.is_broken, rope.last_force(), rope.load());
    assert!(rope.is_broken, "98 N on a 50 N joint");
    assert!(rope.load() > 1.0, "the step that broke it is the last one it reports: {}", rope.load());
}

/// **A determinate truss carries what statics says.** A wall bracket of two 5 kg bars pinned to
/// the wall — the upper one level, the lower one at 45° — meeting at a tip that holds a 50 kg
/// weight. With `P` the tip load (the weight and half of each bar), the upper bar is in tension
/// `P` and the lower in compression `√2·P`; the joints at the tip and the wall carry those
/// (the wall ones plus a bar's own half-weight, which the tolerance covers). Averaged over the
/// last half second, each is within 8 %.
///
/// Measured while separating what `kopru`'s truss showed: at this 10:1 mass ratio the readings sit
/// within ~3–6 % of statics; with 0.1 kg bars (500:1) they read ~30 % low and never settle — the
/// sequential-impulse mass-ratio limit, not asserted here. So a truss reading far above statics
/// is not the measurement or a determinate solve; `kopru`'s four pinned supports make it
/// indeterminate.
#[test]
fn a_two_bar_bracket_carries_the_member_forces_statics_gives() {
    use gizmo_math::Quat;
    use gizmo_physics_core::CollisionLayer;
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let wall = |w: &mut World, at: Vec3| {
        let e = w.spawn();
        w.add_component(e, Transform::new(at));
        w.add_component(e, RigidBody::new_static());
        w.add_component(e, Velocity::default());
        w.add_component(e, Collider::sphere(0.05).with_layer(CollisionLayer::new(2).with_mask(0)));
        e.id()
    };
    let bar = |w: &mut World, a: Vec3, b: Vec3| {
        let d = b - a;
        let half = d.length() * 0.5;
        let col = Collider::box_collider(Vec3::new(half, 0.05, 0.05)).with_layer(CollisionLayer::new(1).with_mask(0));
        let mut rb = RigidBody::new(5.0, true);
        rb.update_inertia_from_collider(&col);
        rb.lock_translation_z = true;
        rb.lock_rotation_x = true;
        rb.lock_rotation_y = true;
        let e = w.spawn();
        w.add_component(e, Transform::new(a + d * 0.5).with_rotation(Quat::from_rotation_z(d.y.atan2(d.x))));
        w.add_component(e, rb);
        w.add_component(e, Velocity::default());
        w.add_component(e, col);
        (e.id(), half)
    };
    let top = wall(&mut world, Vec3::ZERO);
    let bottom = wall(&mut world, Vec3::new(0.0, -2.0, 0.0));
    let (upper, hu) = bar(&mut world, Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0));
    let (lower, hl) = bar(&mut world, Vec3::new(0.0, -2.0, 0.0), Vec3::new(2.0, 0.0, 0.0));
    let weight = world.spawn();
    world.add_component(weight, Transform::new(Vec3::new(2.0, -1.0, 0.0)));
    let mut rb = RigidBody::new(50.0, true);
    rb.lock_translation_z = true;
    world.add_component(weight, rb);
    world.add_component(weight, Velocity::default());
    world.add_component(weight, Collider::sphere(0.2).with_layer(CollisionLayer::new(3).with_mask(0)));
    let h = BodyHandle::from_id;
    world.get_resource_mut::<PhysicsWorld>().unwrap().joints = vec![
        Joint::ball_socket(h(top), h(upper), Vec3::ZERO, Vec3::new(-hu, 0.0, 0.0)),
        Joint::ball_socket(h(bottom), h(lower), Vec3::ZERO, Vec3::new(-hl, 0.0, 0.0)),
        Joint::ball_socket(h(upper), h(lower), Vec3::new(hu, 0.0, 0.0), Vec3::new(hl, 0.0, 0.0)),
        Joint::ball_socket(h(upper), h(weight.id()), Vec3::new(hu, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)),
    ];
    let mut sum = [0.0f32; 4];
    let n = 60;
    for i in 0..600 {
        gizmo_physics_rigid::system::physics_step_system(&world, 1.0 / 120.0);
        world.apply_commands();
        if i >= 600 - n {
            for (s, j) in sum.iter_mut().zip(&world.get_resource::<PhysicsWorld>().unwrap().joints) {
                *s += j.last_force() / n as f32;
            }
        }
    }
    let p = 50.0 * 9.81 + 5.0 * 9.81;
    let expected = [p, p * 2f32.sqrt(), p * 2f32.sqrt(), 50.0 * 9.81];
    for (k, (got, want)) in sum.iter().zip(expected).enumerate() {
        assert!((got - want).abs() < 0.08 * want, "joint {k}: {got:.0} N against statics' {want:.0} N (all: {sum:?})");
    }
}
