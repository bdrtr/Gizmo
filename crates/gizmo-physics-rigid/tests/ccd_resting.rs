//! A body with CCD rests on the ground as one without it does: touching it, with friction and
//! rolling resistance. Two things stood in the way, both measured with a 0.2 m ball rolled at
//! 1 m/s on flat ground with rolling friction 0.08, which stops it in a quarter of a second:
//!
//! - **It never touched.** A speculative contact halts a body a skin (1 cm) short of the
//!   surface, and it did so every step, so a CCD body at rest hovered: on terrain it had no real
//!   contact at all, only speculative ones, which carry no friction.
//! - **The solver it went to had no rolling row.** An island with a CCD body is solved by the
//!   split-impulse path, and rolling resistance existed only in the TGS path: on a box floor,
//!   with real contacts, it rolled on for 6 s.
//!
//! Either alone left it rolling until damping stopped it — eight seconds, three to four metres.

use gizmo_core::world::World;
use gizmo_math::Vec3;
use gizmo_physics_core::components::PhysicsMaterial;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

fn material() -> PhysicsMaterial {
    PhysicsMaterial { rolling_friction: 0.08, restitution: 0.1, ..Default::default() }
}

/// How far a 0.2 m ball sent at 1 m/s over `ground` goes before it stops, and how many frames
/// (60 Hz) that takes.
fn roll(ground: Collider, ground_at: Vec3, ccd: bool) -> (f32, usize) {
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    let g = world.spawn();
    world.add_component(g, Transform::new(ground_at));
    world.add_component(g, RigidBody::new_static());
    world.add_component(g, Velocity::default());
    world.add_component(g, ground.with_material(material()));
    let col = Collider::sphere(0.2).with_material(material());
    let mut rb = RigidBody::new(0.5, true);
    rb.linear_damping = 0.35;
    rb.angular_damping = 0.6;
    rb.ccd_enabled = ccd;
    rb.update_inertia_from_collider(&col);
    let start = Vec3::new(1.3, 0.205, 0.4);
    let b = world.spawn();
    world.add_component(b, Transform::new(start));
    world.add_component(b, rb);
    world.add_component(b, Velocity::new(Vec3::new(1.0, 0.0, 0.3)));
    world.add_component(b, col);
    let mut stopped = None;
    for f in 0..900 {
        let next = world.tick;
        world.begin_change_frame(next);
        physics_step_system(&world, 1.0 / 60.0);
        world.apply_commands();
        let v = world.query::<&Velocity>().get(b).unwrap().linear.length();
        match (v < 0.05, stopped) {
            (true, None) => stopped = Some(f),
            (false, _) => stopped = None,
            _ => {}
        }
    }
    let p = world.query::<&Transform>().get(b).unwrap().position;
    (Vec3::new(p.x - start.x, 0.0, p.z - start.z).length(), stopped.unwrap_or(usize::MAX))
}

fn grounds() -> [(&'static str, Collider, Vec3); 3] {
    [
        ("a box floor", Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)), Vec3::new(0.0, -0.5, 0.0)),
        ("terrain", Collider::heightfield(vec![0.0; 41 * 41], 41, 41, Vec3::splat(0.5)), Vec3::ZERO),
        (
            "a mesh",
            Collider::trimesh(
                vec![Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, -10.0), Vec3::new(10.0, 0.0, 10.0), Vec3::new(-10.0, 0.0, 10.0)],
                vec![0, 2, 1, 0, 3, 2],
            ),
            Vec3::ZERO,
        ),
    ]
}

#[test]
fn a_ccd_ball_rolls_to_a_stop_as_one_without_ccd_does() {
    for (name, ground, at) in grounds() {
        let (plain, plain_frames) = roll(ground.clone(), at, false);
        let (ccd, ccd_frames) = roll(ground, at, true);
        assert!(plain_frames < 60, "premise on {name}: without CCD it stops within a second ({plain_frames} frames)");
        assert!(ccd_frames < 60, "on {name} the CCD ball rolled for {ccd_frames} frames, without CCD {plain_frames}");
        assert!((ccd - plain).abs() < 0.1, "on {name} the CCD ball went {ccd:.3} m, without CCD {plain:.3} m");
    }
}
