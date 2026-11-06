//! A seeded storm of structural changes, and what must survive it.
//!
//! The soak tests run fixed scenes — a tower, a stack — for a long time. What they never do is
//! *change* the scene while it runs, and the bugs found on 2026-11-04/05 were all of that kind: a
//! dead body's joints and contacts inherited by the next body on its id, a joint added and removed
//! while the game plays. This test spawns, despawns (so ids are recycled constantly), ropes bodies
//! together and cuts the ropes, throws bodies and turns CCD on and off — from a seed, through the
//! ECS bridge a game uses — and checks after every step:
//!
//! - nothing is NaN or infinite, nothing has left the closed box it is in (which only a body
//!   going through a wall could), and no speed is absurd;
//! - no joint names a body that is no longer there;
//! - and, at the end, that the same seed gives the same world bit for bit.

use gizmo_core::Entity;
use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};
use gizmo_physics_rigid::joints::Joint;
use gizmo_physics_rigid::system::physics_step_system;
use gizmo_physics_rigid::world::PhysicsWorld;

const STEP: f32 = 1.0 / 60.0;
const SPEED_LIMIT: f32 = 300.0;

/// A tiny deterministic generator (xorshift64*): the test must not depend on a crate's RNG
/// changing under it.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }
    fn pick(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

struct Outcome {
    hash: u64,
    spawned: u32,
    despawned: u32,
    joints_made: u32,
    joints_cut: u32,
    worst_speed: f32,
    farthest: f32,
}

fn storm(seed: u64, frames: u32) -> Outcome {
    let mut rng = Seeded(seed);
    let mut world = World::new();
    world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -9.81, 0.0)));
    // A closed box — floor, four walls, ceiling — so that anything found outside it got there
    // through a wall. (With 6 m walls the kicks threw bodies over them, and the first run of this
    // test reported free falls off the floor's edge as tunnelling.)
    let statics = [
        (Vec3::new(0.0, -0.5, 0.0), Vec3::new(21.0, 0.5, 21.0)),
        (Vec3::new(0.0, 30.5, 0.0), Vec3::new(21.0, 0.5, 21.0)),
        (Vec3::new(20.5, 15.0, 0.0), Vec3::new(0.5, 15.0, 21.0)),
        (Vec3::new(-20.5, 15.0, 0.0), Vec3::new(0.5, 15.0, 21.0)),
        (Vec3::new(0.0, 15.0, 20.5), Vec3::new(21.0, 15.0, 0.5)),
        (Vec3::new(0.0, 15.0, -20.5), Vec3::new(21.0, 15.0, 0.5)),
    ];
    for (at, half) in statics {
        world.spawn_bundle((Transform::new(at), RigidBody::new_static(), Velocity::default(), Collider::box_collider(half)));
    }
    let mut bodies: Vec<Entity> = Vec::new();
    let mut out = Outcome { hash: 0, spawned: 0, despawned: 0, joints_made: 0, joints_cut: 0, worst_speed: 0.0, farthest: 0.0 };

    for frame in 0..frames {
        // A few changes a frame, more of them early so the scene fills.
        let changes = 1 + rng.pick(3);
        for _ in 0..changes {
            match rng.pick(10) {
                0..=3 if bodies.len() < 120 => {
                    let at = Vec3::new(rng.range(-15.0, 15.0), rng.range(1.0, 12.0), rng.range(-15.0, 15.0));
                    let col = if rng.unit() < 0.5 {
                        Collider::sphere(rng.range(0.2, 0.8))
                    } else {
                        Collider::box_collider(Vec3::new(rng.range(0.2, 0.8), rng.range(0.2, 0.8), rng.range(0.2, 0.8)))
                    };
                    let mut rb = RigidBody::new(rng.range(0.5, 20.0), true);
                    rb.ccd_enabled = rng.unit() < 0.3;
                    rb.update_inertia_from_collider(&col);
                    let v = Vec3::new(rng.range(-8.0, 8.0), rng.range(-4.0, 8.0), rng.range(-8.0, 8.0));
                    let id = world.spawn_bundle((Transform::new(at), rb, Velocity::new(v), col));
                    bodies.push(id);
                    out.spawned += 1;
                }
                4 | 5 if !bodies.is_empty() => {
                    let id = bodies.swap_remove(rng.pick(bodies.len()));
                    if let Some(e) = world.live(id) {
                        world.despawn(e);
                    }
                    out.despawned += 1;
                }
                6 if bodies.len() >= 2 => {
                    let a = bodies[rng.pick(bodies.len())];
                    let b = bodies[rng.pick(bodies.len())];
                    if a != b {
                        let length = rng.range(0.5, 4.0);
                        if let Some(mut pw) = world.get_resource_mut::<PhysicsWorld>() {
                            pw.joints.push(Joint::rope(BodyHandle::from_id(a.id()), BodyHandle::from_id(b.id()), Vec3::ZERO, Vec3::ZERO, length));
                        }
                        out.joints_made += 1;
                    }
                }
                7 => {
                    if let Some(mut pw) = world.get_resource_mut::<PhysicsWorld>() {
                        if !pw.joints.is_empty() {
                            let n = pw.joints.len();
                            pw.joints.swap_remove((rng.next() % n as u64) as usize);
                            out.joints_cut += 1;
                        }
                    }
                }
                8 if !bodies.is_empty() => {
                    let id = bodies[rng.pick(bodies.len())];
                    let kick = Vec3::new(rng.range(-15.0, 15.0), rng.range(0.0, 15.0), rng.range(-15.0, 15.0));
                    if let Some(mut v) = world.borrow_mut::<Velocity>().get_mut(id) {
                        v.linear = kick;
                    }
                    if let Some(mut rb) = world.borrow_mut::<RigidBody>().get_mut(id) {
                        rb.wake_up();
                    }
                }
                9 if !bodies.is_empty() => {
                    let id = bodies[rng.pick(bodies.len())];
                    if let Some(mut rb) = world.borrow_mut::<RigidBody>().get_mut(id) {
                        rb.ccd_enabled = !rb.ccd_enabled;
                    }
                }
                _ => {}
            }
        }
        world.apply_commands();
        physics_step_system(&world, STEP);

        // After every step: finite, in bounds, sane speeds, no joint naming a dead body.
        for &id in &bodies {
            let p = world.borrow::<Transform>().get(id).map(|t| t.position).expect("a live body has a Transform");
            let v = world.borrow::<Velocity>().get(id).map(|v| v.linear).unwrap_or(Vec3::ZERO);
            assert!(p.is_finite() && v.is_finite(), "frame {frame}: body {id} is not finite: p {p:?} v {v:?}");
            out.farthest = out.farthest.max(p.length());
            out.worst_speed = out.worst_speed.max(v.length());
            let inside = p.x.abs() < 20.0 && p.z.abs() < 20.0 && p.y > -0.5 && p.y < 30.0;
            assert!(inside, "frame {frame}: body {id} is outside the closed box, at {p:?} (v {v:?})");
            assert!(v.length() < SPEED_LIMIT, "frame {frame}: body {id} moves at {} m/s", v.length());
        }
        let pw = world.get_resource::<PhysicsWorld>().expect("physics world");
        for j in &pw.joints {
            for end in [j.entity_a, j.entity_b] {
                assert!(
                    pw.entity_index_map.get(&end.id()).is_some(),
                    "frame {frame}: a joint names body {} that is not in the world",
                    end.id()
                );
            }
        }
    }
    out.hash = world.get_resource::<PhysicsWorld>().expect("physics world").state_hash();
    out
}

fn check(seed: u64, frames: u32) {
    let a = storm(seed, frames);
    let b = storm(seed, frames);
    eprintln!(
        "seed {seed:#x}, {frames} frames: {} spawned, {} despawned, {} ropes made, {} cut; fastest {:.1} m/s, farthest {:.1} m; hash {:#x}",
        a.spawned, a.despawned, a.joints_made, a.joints_cut, a.worst_speed, a.farthest, a.hash
    );
    assert!(
        a.spawned > frames / 5 && a.despawned > frames / 10 && a.joints_made > frames / 30,
        "the storm was too calm to mean anything"
    );
    assert_eq!(a.hash, b.hash, "seed {seed:#x}: the same storm twice gave two different worlds");
}

/// One storm, twice. ~50 s unoptimised (the suite's profile), ~5 s with `--release`.
#[test]
fn a_seeded_storm_of_spawns_despawns_and_ropes_stays_finite_contained_and_deterministic() {
    check(0x5EED_0001, 900);
}

/// Three longer storms: ~4 min unoptimised, ~30 s with `--release`. Measured green 2026-11-05.
#[test]
#[ignore = "long (~4 min unoptimised); run with --ignored, ideally --release"]
fn three_long_storms() {
    for seed in [0x5EED_0001u64, 0xC0FF_EE42, 0x0BAD_CAFE] {
        check(seed, 1500);
    }
}
