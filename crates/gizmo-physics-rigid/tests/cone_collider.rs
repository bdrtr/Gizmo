//! What a cone collider has to do that a cylinder of the same numbers cannot.
//!
//! The two share an axis convention, a `half_height` meaning and the GJK/EPA route, which is
//! exactly why every test here is written to fail if the cone were quietly being treated as one:
//! a cone standing on its base rests at the same height a cylinder does, but it **tips** where a
//! cylinder stands, its apex is a point rather than a face, and its resting height on its side is
//! its base radius rather than a constant one.
//!
//! Driven through the real ECS path (`physics_step_system`), like `cylinder_collider.rs`, because
//! the question is what the solver ends up doing rather than what one function returns.

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::components::{CombineMode, PhysicsMaterial};
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 240.0;

fn scene() -> World {
    let mut w = World::new();
    w.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    w
}

/// A static floor whose top surface is at `y = 0`.
fn floor(world: &mut World) {
    let e = world.spawn();
    world.add_component(e, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
    world.add_component(e, RigidBody::new_static());
    world.add_component(e, Velocity::default());
    world.add_component(e, Collider::box_collider(Vec3::new(50.0, 0.5, 50.0)));
}

/// High friction and no bounce: the shapes here are meant to settle, not to skate or hop.
fn grippy() -> PhysicsMaterial {
    PhysicsMaterial {
        restitution: 0.0,
        static_friction: 0.9,
        dynamic_friction: 0.8,
        friction_combine: CombineMode::Max,
        restitution_combine: CombineMode::Min,
        ..Default::default()
    }
}

fn drop_body(world: &mut World, collider: Collider, position: Vec3, rotation: Quat) -> u32 {
    let e = world.spawn();
    let mut t = Transform::new(position);
    t.rotation = rotation;
    let mut rb = RigidBody::new(2.0, true); // (mass, use_gravity)
    rb.update_inertia_from_collider(&collider);
    world.add_component(e, t);
    world.add_component(e, rb);
    world.add_component(e, Velocity::default());
    world.add_component(e, collider);
    e.id()
}

fn step(world: &World, n: usize) {
    for _ in 0..n {
        physics_step_system(world, DT);
    }
}

fn transform(world: &World, id: u32) -> Transform {
    *world.borrow::<Transform>().get(id).unwrap()
}

/// A cone stands on its base at its half-height — the same resting height as the cylinder that
/// bounds it, which is the *shared* half of the two shapes' behaviour.
#[test]
fn a_cone_rests_on_its_base_at_its_half_height() {
    let mut w = scene();
    floor(&mut w);
    let mut collider = Collider::cone(0.4, 0.5);
    collider.material = grippy();
    let id = drop_body(&mut w, collider, Vec3::new(0.0, 1.2, 0.0), Quat::IDENTITY);

    step(&w, 900);
    let y = transform(&w, id).position.y;
    assert!(
        (y - 0.5).abs() < 0.02,
        "a cone standing on its base should rest at its half-height (0.5), got {y}"
    );
}

/// The discriminator. Balanced apex-down, a cone has a **point** on the floor and must fall over;
/// a cylinder in the same pose has a flat face and stands.
///
/// This is the test that fails if `cone_support` were the cylinder's — with a flat top invented at
/// the apex, the shape would sit there indefinitely. Verified red by doing exactly that: choosing
/// the radial and axial extremes independently left the tilt at 0.00 rad after 900 steps.
#[test]
fn a_cone_balanced_on_its_apex_falls_over_where_a_cylinder_stands() {
    // Apex down: rotate 180° about X, so local +Y (the apex) points at the floor.
    let upside_down = Quat::from_rotation_x(std::f32::consts::PI);

    let tilt_of = |collider: Collider| {
        let mut w = scene();
        floor(&mut w);
        let mut c = collider;
        c.material = grippy();
        // A hair off-centre, because a mathematically perfect balance is not a physical claim —
        // the question is whether the shape RECOVERS from a nudge, and a cylinder does.
        let id = drop_body(
            &mut w,
            c,
            Vec3::new(0.0, 0.55, 0.0),
            Quat::from_rotation_z(0.02) * upside_down,
        );
        step(&w, 900);
        // How far the body's local +Y has swung away from where it started.
        let axis = transform(&w, id).rotation * Vec3::Y;
        axis.dot(upside_down * Vec3::Y).clamp(-1.0, 1.0).acos()
    };

    let cone_tilt = tilt_of(Collider::cone(0.4, 0.5));
    let cylinder_tilt = tilt_of(Collider::cylinder(0.4, 0.5));

    assert!(
        cylinder_tilt < 0.15,
        "premise: a cylinder on its flat end recovers from a nudge; it moved {cylinder_tilt} rad"
    );
    assert!(
        cone_tilt > 0.5,
        "a cone balanced on its apex must fall over — it moved only {cone_tilt} rad, which is what \
         a cylinder does. The support function has invented a flat top."
    );
}

/// On its side, a cone rests on its base **radius**, exactly as a cylinder does — but its axis
/// must end up horizontal, which is what separates it from a sphere or a box of the same bound.
#[test]
fn a_cone_laid_on_its_side_rests_on_its_radius() {
    let mut w = scene();
    floor(&mut w);
    let radius = 0.3;
    let mut collider = Collider::cone(radius, 0.6);
    collider.material = grippy();
    // Axis laid along world X.
    let id = drop_body(
        &mut w,
        collider,
        Vec3::new(0.0, 1.0, 0.0),
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
    );

    step(&w, 1200);
    let t = transform(&w, id);
    // It settles at or below the radius: a cone on its side touches along a slanted line, so it
    // rolls onto its lowest rest — never *above* the radius, which is what a bounding-box or
    // capsule treatment would produce.
    assert!(
        t.position.y <= radius + 0.03,
        "a cone on its side must not rest above its base radius ({radius}), got {}",
        t.position.y
    );
    assert!(t.position.y > 0.0, "and it must not sink through the floor: {}", t.position.y);
}

/// Volume, AABB and inertia are each their own arm rather than a fallback, and each differs from
/// the cylinder's in a way a wrong copy would show up as.
#[test]
fn the_derived_quantities_are_a_cone_s_and_not_a_cylinder_s() {
    let (r, h) = (0.5_f32, 0.75_f32);
    let cone = Collider::cone(r, h);
    let cylinder = Collider::cylinder(r, h);

    // Volume: exactly a third.
    let ratio = cone.volume() / cylinder.volume();
    assert!(
        (ratio - 1.0 / 3.0).abs() < 1e-5,
        "a cone is a third of its bounding cylinder, got {ratio}"
    );

    // AABB, upright: the same box, because the base disc reaches the full radius and the apex is
    // inside it.
    let up = cone.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
    let up_cyl = cylinder.compute_aabb(Vec3::ZERO, Quat::IDENTITY);
    assert!((up.min - up_cyl.min).length() < 1e-4 && (up.max - up_cyl.max).length() < 1e-4);

    // Laid flat on its side the two boxes agree to within the *numerical* wrinkle both AABBs
    // document, not to the bit. Asserting a geometric difference here was this test's own first
    // mistake, and asserting exact equality was its second: with the axis on a world axis the
    // base disc already spans the full radius on the other two and the apex sits inside it, so
    // the shapes really are the same box — but `sqrt(1 - a_i²)` sits on its singularity there,
    // and a quaternion's last bits come out as ≈ r·sqrt(2ε). Measured: 1.7e-4 for r = 0.5.
    //
    // The cone picks up that error on ONE side only, because its apex end has no disc to take the
    // root of, where a cylinder pays it at both. So the cone's box is fractionally tighter here
    // for a numerical reason rather than a geometric one — worth knowing before reading a
    // difference of that size as a bug.
    let side = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
    let lying = cone.compute_aabb(Vec3::ZERO, side);
    let lying_cyl = cylinder.compute_aabb(Vec3::ZERO, side);
    assert!((lying.min - lying_cyl.min).length() < 1e-3, "{lying:?} vs {lying_cyl:?}");
    assert!((lying.max - lying_cyl.max).length() < 1e-3, "{lying:?} vs {lying_cyl:?}");

    // TILTED is where the shapes separate. At 45° the apex leans out along +X while the base disc
    // leans the other way, so the cone's box is bounded by the apex on one side and the disc on
    // the other — where a cylinder has a disc at BOTH ends and reaches further. A cone given the
    // cylinder's formula would report the wider box here, which is a broadphase false positive on
    // every tilted cone in the scene.
    let tilted = Quat::from_rotation_z(std::f32::consts::FRAC_PI_4);
    let cone_box = cone.compute_aabb(Vec3::ZERO, tilted);
    let cyl_box = cylinder.compute_aabb(Vec3::ZERO, tilted);
    let cone_x = cone_box.max.x - cone_box.min.x;
    let cyl_x = cyl_box.max.x - cyl_box.min.x;
    assert!(
        cone_x < cyl_x - 1e-3,
        "a tilted cone's box must be tighter than the cylinder's along the tilt ({cone_x} vs \
         {cyl_x}) — equal means the apex was handed the base's radius"
    );

    // Inertia: a cone's spin term is (3/10)mr², a cylinder's is (1/2)mr². Same mass, same
    // dimensions, and the ratio is 3/5 — so a copied cylinder tensor is visible as a number.
    let mut rb_cone = RigidBody::new(2.0, true);
    rb_cone.update_inertia_from_collider(&cone);
    let mut rb_cyl = RigidBody::new(2.0, true);
    rb_cyl.update_inertia_from_collider(&cylinder);
    let spin_ratio = rb_cone.local_inertia.y / rb_cyl.local_inertia.y;
    assert!(
        (spin_ratio - 0.6).abs() < 1e-4,
        "I_y(cone)/I_y(cylinder) must be 3/5, got {spin_ratio}"
    );
    assert!(
        rb_cone.local_inertia.x > 0.0 && rb_cone.local_inertia.z > 0.0,
        "the transverse terms must be finite and positive"
    );
    assert!(
        (rb_cone.local_inertia.x - rb_cone.local_inertia.z).abs() < 1e-6,
        "a cone is symmetric about its axis, so X and Z must match"
    );
}

/// The ray test is the cone's own, not the cylinder's quadratic with a different radius.
#[test]
fn a_ray_at_the_tip_hits_the_tip_and_not_the_base_radius() {
    use gizmo_physics_core::raycast::{Ray, Raycast};

    let (r, h) = (0.5_f32, 1.0_f32);
    // Straight down the axis from above: the first surface is the apex, at y = +h.
    let ray = Ray::new(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y);
    let (t, n) = Raycast::ray_cone(&ray, Vec3::ZERO, Quat::IDENTITY, r, h)
        .expect("a ray down the axis must hit the cone");
    assert!(
        (t - 4.0).abs() < 1e-3,
        "the apex is at y = 1, so the hit is 4 m down; got {t} — a cylinder's flat top would give \
         the same number here, which is why the off-axis case below is the real check"
    );
    assert!(n.y > 0.0, "the normal at the tip points up, got {n:?}");

    // Off the axis by more than the radius AT THAT HEIGHT but less than the base radius: a
    // cylinder would report a hit on its wall, a cone must miss the solid there and fall through
    // to nothing.
    let near_tip = Ray::new(Vec3::new(0.4, 5.0, 0.0), Vec3::NEG_Y);
    let cone_hit = Raycast::ray_cone(&near_tip, Vec3::ZERO, Quat::IDENTITY, r, h);
    let (t2, _) = cone_hit.expect("it still crosses the lower, wider part of the cone");
    let y_hit = 5.0 - t2;
    let radius_at_hit = r * (h - y_hit) / (h * 2.0);
    assert!(
        (0.4 - radius_at_hit).abs() < 1e-2,
        "the hit must land where the cone is exactly 0.4 wide (y = {y_hit}, radius there \
         {radius_at_hit}) — a cylinder's quadratic would have hit at the top instead"
    );

    // Above the apex, aimed up: the infinite double cone the quadratic describes has a solution
    // there, and it is not on this shape.
    let upward = Ray::new(Vec3::new(0.0, 2.0, 0.0), Vec3::Y);
    assert!(
        Raycast::ray_cone(&upward, Vec3::ZERO, Quat::IDENTITY, r, h).is_none(),
        "the mirror cone above the apex is not part of the solid"
    );
}
