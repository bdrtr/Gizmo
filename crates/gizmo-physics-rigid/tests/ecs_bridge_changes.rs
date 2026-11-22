//! The ECS bridge (`physics_step_system`) reads only the bodies that changed since its last step,
//! and writes back only those the step changed. What that has to keep true, with frames advancing
//! as a schedule advances them (a test that never opens a frame sees every component as changed,
//! and so exercises none of this):
//!
//! - the ECS mirrors the physics world after every step, for every kind of body and every kind of
//!   write a game makes;
//! - what no change tick reports is still seen: a component taken off, an entity coming back
//!   from the pool.

use gizmo_core::component::Children;
use gizmo_core::world::World;
use gizmo_core::Entity;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::components::PhysicsMaterial;
use gizmo_physics_core::{Collider, Transform};
use gizmo_physics_rigid::components::{ExternalForce, KinematicTarget, RigidBody, Velocity};
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const DT: f32 = 1.0 / 60.0;

/// One frame, as a schedule runs it: open the change window, step, apply the queued commands.
fn frame(world: &mut World) {
    let next = world.tick;
    world.begin_change_frame(next);
    physics_step_system(world, DT);
    world.apply_commands();
}

fn body(world: &mut World, rb: RigidBody, at: Vec3, col: Collider, vel: bool) -> Entity {
    let e = world.spawn();
    world.add_component(e, Transform::new(at));
    world.add_component(e, rb);
    if vel {
        world.add_component(e, Velocity::default());
    }
    world.add_component(e, col);
    e
}

fn boxed(mass: f32) -> RigidBody {
    let mut rb = RigidBody::new(mass, true);
    rb.calculate_box_inertia(1.0, 1.0, 1.0);
    rb
}

fn new_world() -> World {
    let mut world = World::new();
    world.register_component_type::<Children>();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    world
}

/// Every ECS body's `RigidBody`, `Transform` and `Velocity` equal the physics world's row for it,
/// and every physics row has its ECS body.
fn assert_mirrored(world: &World, frame: u32) {
    let physics = world.get_resource::<PhysicsWorld>().unwrap();
    let mut seen = 0;
    for (e, rb, t, v) in world.query::<(Entity, &RigidBody, &Transform, Option<&Velocity>)>().iter() {
        let Some(&i) = physics.entity_index_map.get(&e.id()) else { continue };
        seen += 1;
        assert_eq!(physics.rigid_bodies[i], *rb, "frame {frame}: {e:?}'s RigidBody");
        assert_eq!(physics.transforms[i], *t, "frame {frame}: {e:?}'s Transform");
        if let Some(v) = v {
            assert_eq!(physics.velocities[i], *v, "frame {frame}: {e:?}'s Velocity");
        }
    }
    assert_eq!(seen, physics.entities.len(), "frame {frame}: a physics body without its ECS body");
}

/// A pile that settles and sleeps, and around it everything a game does to bodies: a compound
/// body, a kinematic body on a target and one moved by hand, a pushed box, a CCD bullet, a ball
/// with an entity material; a teleport, a velocity written to a sleeper, a woken body thrown into
/// the pile, a despawn and a spawn, a collider taken off and put back, a static wall moved, a mass
/// and a collider changed, a child moved. After every frame the ECS is the physics world's mirror.
#[test]
fn the_ecs_mirrors_the_physics_world_after_every_step() {
    let mut world = new_world();
    body(&mut world, RigidBody::new_static(), Vec3::new(0.0, -1.0, 0.0), Collider::box_collider(Vec3::new(40.0, 1.0, 40.0)), true);
    let wall = body(&mut world, RigidBody::new_static(), Vec3::new(12.0, 2.0, 0.0), Collider::box_collider(Vec3::new(0.5, 3.0, 10.0)), false);
    let boxes: Vec<Entity> = (0..30)
        .map(|k| {
            let at = Vec3::new((k % 5) as f32 * 1.2 - 3.0, 0.5 + (k / 5) as f32 * 1.05, (k % 3) as f32 * 0.3);
            body(&mut world, boxed(5.0), at, Collider::box_collider(Vec3::splat(0.5)), true)
        })
        .collect();
    let ball = body(&mut world, RigidBody::new(2.0, true), Vec3::new(-8.0, 6.0, 0.0), Collider::sphere(0.4), true);
    world.add_component(ball, PhysicsMaterial::RUBBER);
    let parent = body(&mut world, RigidBody::new(6.0, true), Vec3::new(6.0, 4.0, 4.0), Collider::box_collider(Vec3::new(0.6, 0.2, 0.6)), true);
    let child = world.spawn();
    world.add_component(child, Transform::new(Vec3::new(0.0, 0.6, 0.0))); // local to the parent
    world.add_component(child, Collider::sphere(0.3));
    world.add_component(parent, Children(vec![child]));
    let lift = body(&mut world, RigidBody::new_kinematic(), Vec3::new(-6.0, 0.5, -6.0), Collider::box_collider(Vec3::new(1.0, 0.2, 1.0)), true);
    world.add_component(lift, KinematicTarget::new(Vec3::new(-6.0, 0.5, -6.0), Quat::IDENTITY));
    let slide = body(&mut world, RigidBody::new_kinematic(), Vec3::new(-6.0, 0.5, 6.0), Collider::box_collider(Vec3::new(1.0, 0.2, 1.0)), true);
    let pushed = body(&mut world, RigidBody::new(3.0, true), Vec3::new(4.0, 0.5, -6.0), Collider::box_collider(Vec3::splat(0.5)), true);
    world.add_component(pushed, ExternalForce::new(Vec3::new(4.0, 0.0, 0.0)));
    let mut bullet_rb = RigidBody::new(0.2, true);
    bullet_rb.ccd_enabled = true;
    let bullet = body(&mut world, bullet_rb, Vec3::new(-15.0, 1.5, 0.3), Collider::sphere(0.1), true);
    world.query_mut::<&mut Velocity>().get_mut(bullet).unwrap().linear = Vec3::new(60.0, 0.0, 0.0);

    let mut asleep_at_300 = 0;
    for f in 0..700u32 {
        if (60..=240).contains(&f) {
            let y = 0.5 + (f - 60) as f32 * 0.01;
            world.query_mut::<&mut KinematicTarget>().get_mut(lift).unwrap().position = Vec3::new(-6.0, y, -6.0);
        }
        if (100..200).contains(&f) {
            world.query_mut::<&mut Transform>().get_mut(slide).unwrap().position.x += 0.02;
        }
        match f {
            250 => world.query_mut::<&mut PhysicsMaterial>().get_mut(ball).unwrap().restitution = 0.05,
            300 => {
                asleep_at_300 = world.query::<&RigidBody>().iter().filter(|rb| rb.is_dynamic() && rb.is_sleeping).count();
                world.query_mut::<&mut Transform>().get_mut(boxes[3]).unwrap().position = Vec3::new(0.0, 8.0, -4.0);
            }
            350 => world.query_mut::<&mut Velocity>().get_mut(boxes[7]).unwrap().linear = Vec3::new(0.0, 5.0, 0.0),
            360 => {
                let mut q = world.query_mut::<(&mut RigidBody, &mut Velocity)>();
                let (mut rb, mut v) = q.get_mut(boxes[9]).unwrap();
                rb.wake_up();
                v.linear = Vec3::new(2.0, 3.0, 0.0);
            }
            400 => world.despawn(boxes[11]),
            410 => {
                body(&mut world, boxed(5.0), Vec3::new(1.0, 6.0, 1.0), Collider::box_collider(Vec3::splat(0.5)), true);
            }
            450 => world.remove_component::<Collider>(boxes[13]),
            470 => world.add_component(boxes[13], Collider::box_collider(Vec3::splat(0.5))),
            500 => world.query_mut::<&mut Transform>().get_mut(wall).unwrap().position.x = 9.0,
            520 => world.query_mut::<&mut RigidBody>().get_mut(boxes[15]).unwrap().mass = 20.0,
            540 => world.add_component(boxes[17], Collider::box_collider(Vec3::splat(0.6))),
            600 => world.query_mut::<&mut Transform>().get_mut(child).unwrap().position.y += 0.1,
            _ => {}
        }
        frame(&mut world);
        assert_mirrored(&world, f);
    }
    assert!(asleep_at_300 >= 20, "premise: the pile is mostly asleep when the game starts poking it ({asleep_at_300})");
    let physics = world.get_resource::<PhysicsWorld>().unwrap();
    let wall_row = physics.entity_index_map.get(&wall.id()).copied().unwrap();
    assert_eq!(physics.transforms[wall_row].position.x, 9.0, "the static wall moved by the game");
    assert!(physics.entity_index_map.get(&boxes[13].id()).is_some(), "the collider put back brought the box back");
}

/// A `PhysicsMaterial` taken off a body that has gone to sleep — which no change tick reports —
/// gives it back its collider's own material.
#[test]
fn a_material_taken_off_a_sleeping_body_is_seen() {
    let mut world = new_world();
    body(&mut world, RigidBody::new_static(), Vec3::new(0.0, -1.0, 0.0), Collider::box_collider(Vec3::new(10.0, 1.0, 10.0)), true);
    let own = Collider::box_collider(Vec3::splat(0.5));
    let b = body(&mut world, boxed(5.0), Vec3::new(0.0, 0.5, 0.0), own.clone(), true);
    world.add_component(b, PhysicsMaterial::ICE);
    for _ in 0..240 {
        frame(&mut world);
    }
    let row = |w: &World| w.get_resource::<PhysicsWorld>().unwrap().entity_index_map.get(&b.id()).copied().unwrap();
    {
        let physics = world.get_resource::<PhysicsWorld>().unwrap();
        assert!(physics.rigid_bodies[row(&world)].is_sleeping, "premise: the box has gone to sleep");
        assert_eq!(physics.colliders[row(&world)].material, own.material_with(Some(&PhysicsMaterial::ICE)), "premise: simulated on ice");
    }
    world.remove_component::<PhysicsMaterial>(b);
    frame(&mut world);
    let physics = world.get_resource::<PhysicsWorld>().unwrap();
    assert_eq!(physics.colliders[row(&world)].material, own.material, "the material taken off is still simulated");
}

/// A body put in the pool leaves the physics world; taken out again — which writes none of its
/// components — it comes back.
#[test]
fn a_body_back_from_the_pool_is_simulated_again() {
    let mut world = new_world();
    body(&mut world, RigidBody::new_static(), Vec3::new(0.0, -1.0, 0.0), Collider::box_collider(Vec3::new(10.0, 1.0, 10.0)), true);
    let b = body(&mut world, boxed(5.0), Vec3::new(0.0, 0.5, 0.0), Collider::box_collider(Vec3::splat(0.5)), true);
    let simulated = |w: &World| w.get_resource::<PhysicsWorld>().unwrap().entity_index_map.contains_key(&b.id());
    for _ in 0..240 {
        frame(&mut world);
    }
    assert!(simulated(&world));
    assert!(world.query::<&RigidBody>().get(b).unwrap().is_sleeping, "premise: the box has gone to sleep");
    world.add_component(b, gizmo_core::pool::Pooled);
    frame(&mut world);
    assert!(!simulated(&world), "a pooled body leaves the world");
    frame(&mut world);
    world.remove_component::<gizmo_core::pool::Pooled>(b);
    frame(&mut world);
    assert!(simulated(&world), "a body back from the pool is simulated again");
}
