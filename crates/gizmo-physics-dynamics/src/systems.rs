//! ECS system wrappers that drive the (manually-written, audited) gameplay
//! controllers in this crate from the schedule.
//!
//! Both systems use the exclusive-barrier signature `fn(&World, dt)` — the same
//! shape as [`gizmo_physics_rigid::physics_step_system`] — so they can be added
//! straight into a [`gizmo_core::system::Schedule`] (see
//! `gizmo_app::gameplay`). A `fn(&World, f32)` reports itself as an *exclusive*
//! system, so the scheduler runs it alone; that is what makes the
//! `query_unchecked` mutable borrows below sound.
//!
//! # Ordering / determinism
//! * [`vehicle_controller_system`] applies suspension + tire forces to the
//!   chassis `Velocity`, so it must run **before** the rigid physics step
//!   integrates that velocity. Register it in [`gizmo_core::system::Phase::Physics`]
//!   with `.before("physics_step_system")`.
//! * [`character_controller_system`] performs its own kinematic
//!   move/step/slide and writes `Transform`/`Velocity` directly; it does not
//!   depend on the rigid solver.
//!
//! Both systems are **no-ops** on worlds that contain no vehicle/character
//! entities (the component-tuple query matches nothing), so registering them
//! does not perturb a plain rigid-body scene (e.g. the determinism oracle).

use gizmo_core::component::IsDeleted;
use gizmo_core::query::{Mut, Without};
use gizmo_core::world::World;
use gizmo_physics_core::components::CharacterController;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_rigid::components::{RigidBody, Velocity};

use crate::character::update_character;
use crate::vehicle::{update_vehicle, weather_grip_factor, VehicleController};

/// Snapshot every live `(Transform, Collider)` into an owned buffer so the
/// controllers can raycast against the scene while we later hold *mutable*
/// borrows on the moving entities. `Transform` is `Copy` and `Collider` is
/// cheap to clone (shapes are `Arc`-backed), matching `physics_step_system`.
///
/// The read-only query is fully drained into the returned `Vec` and dropped
/// here, so it never overlaps the mutable query opened by the callers.
fn gather_colliders(world: &World) -> Vec<(BodyHandle, Transform, Collider)> {
    let mut colliders = Vec::new();
    if let Some(query) = world.query::<(&Transform, &Collider, Without<IsDeleted>)>() {
        for (id, (transform, collider, _)) in query.iter() {
            colliders.push((BodyHandle::from_id(id), *transform, collider.clone()));
        }
    }
    colliders
}

/// Drives [`VehicleController`] (Pacejka tire model + suspension + gearbox) for
/// every vehicle entity each fixed step.
///
/// Query: `Mut<VehicleController>` + `Mut<RigidBody>` + `&Transform` +
/// `Mut<Velocity>`. The controller reads steering/throttle/brake inputs off the
/// `VehicleController` component and writes the resulting forces into `Velocity`
/// (which the rigid physics step then integrates), so this must run *before*
/// `physics_step_system`.
#[tracing::instrument(skip_all, name = "vehicle_controller_system")]
pub fn vehicle_controller_system(world: &World, dt: f32) {
    if dt <= 0.0 {
        return;
    }

    let all_colliders = gather_colliders(world);

    // Hava durumu grip çarpanı için sahnenin PhysicsWorld'ünden Weather'ı oku (yoksa Sunny).
    // Copy değer olarak alınır → aşağıdaki unsafe query'den önce borrow düşer.
    let weather = world
        .get_resource::<gizmo_physics_rigid::world::PhysicsWorld>()
        .map(|w| w.weather)
        .unwrap_or_default();

    // SAFETY: a `fn(&World, f32)` system reports `is_exclusive`, so the scheduler
    // runs it alone — no other query mutably aliases these components while this
    // one is live. The read-only `gather_colliders` query above was already
    // dropped, so there is no overlapping `&`/`&mut` on `Transform` either.
    let query = unsafe {
        world.query_unchecked::<(
            Mut<VehicleController>,
            Mut<RigidBody>,
            &Transform,
            Mut<Velocity>,
            Without<IsDeleted>,
        )>()
    };
    if let Some(mut query) = query {
        for (id, (mut vehicle, mut rb, transform, mut vel, _)) in query.iter_mut() {
            // Aquaplaning hıza bağlı → her araç kendi hızıyla değerlendirilir.
            let wg = weather_grip_factor(weather, vel.linear.length());
            update_vehicle(
                BodyHandle::from_id(id),
                &mut vehicle,
                &mut rb,
                transform,
                &mut vel,
                &all_colliders,
                wg,
                dt,
            );
        }
    }
}

/// Drives the kinematic character controller ([`CharacterController`] +
/// [`update_character`]) for every character entity each fixed step.
///
/// Query: `Mut<CharacterController>` + `Mut<Transform>` + `Mut<Velocity>` +
/// `&Collider`. The KCC does its own gravity / ground-snap / step / slide
/// integration and writes `Transform`/`Velocity` directly, so KCC entities must
/// **not** also carry a dynamic `RigidBody` (that would double-integrate
/// gravity via the rigid step).
#[tracing::instrument(skip_all, name = "character_controller_system")]
pub fn character_controller_system(world: &World, dt: f32) {
    if dt <= 0.0 {
        return;
    }

    let all_colliders = gather_colliders(world);

    // SAFETY: see `vehicle_controller_system` — exclusive barrier system, and the
    // read-only gather query is dropped before this mutable query is opened, so
    // the `Mut<Transform>` here never aliases the `&Transform` used above.
    let query = unsafe {
        world.query_unchecked::<(
            Mut<CharacterController>,
            Mut<Transform>,
            Mut<Velocity>,
            &Collider,
            Without<IsDeleted>,
        )>()
    };
    if let Some(mut query) = query {
        for (id, (mut kcc, mut transform, mut vel, collider, _)) in query.iter_mut() {
            update_character(
                BodyHandle::from_id(id),
                &mut kcc,
                &mut transform,
                &mut vel,
                collider,
                &all_colliders,
                dt,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{character_controller_system, vehicle_controller_system};
    use crate::vehicle::{Axle, VehicleController, Wheel};
    use gizmo_core::entity::Entity;
    use gizmo_core::system::{Phase, Schedule, SystemConfig};
    use gizmo_core::world::World;
    use gizmo_math::Vec3;
    use gizmo_physics_core::components::CharacterController;
    use gizmo_physics_core::{BoxShape, Collider, ColliderShape, Transform};
    use gizmo_physics_rigid::components::{RigidBody, Velocity};
    use gizmo_physics_rigid::physics_step_system;
    use gizmo_physics_rigid::world::PhysicsWorld;

    fn box_collider(hx: f32, hy: f32, hz: f32) -> Collider {
        Collider::from_shape(ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(hx, hy, hz),
        }))
    }

    /// A large static floor whose top surface sits at y = 0.
    fn spawn_ground(world: &mut World) -> Entity {
        let e = world.spawn();
        world.add_component(e, RigidBody::new_static());
        world.add_component(e, box_collider(100.0, 1.0, 100.0));
        world.add_component(e, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
        world.add_component(e, Velocity::default());
        e
    }

    /// A four-wheeled rear-wheel-drive vehicle sized so its wheels rest on a
    /// floor whose top is at y = 0.
    fn make_vehicle() -> VehicleController {
        let mut vc = VehicleController::new();
        let corners = [
            (-0.8, -1.4, Axle::Front, true),
            (0.8, -1.4, Axle::Front, false),
            (-0.8, 1.4, Axle::Rear, true),
            (0.8, 1.4, Axle::Rear, false),
        ];
        for (x, z, axle, is_left) in corners {
            vc.add_wheel(Wheel {
                attachment_local_pos: Vec3::new(x, 0.0, z),
                axle_type: axle,
                is_left,
                radius: 0.35,
                suspension_rest_length: 0.4,
                suspension_max_travel: 0.3,
                ..Default::default()
            });
        }
        vc
    }

    fn spawn_vehicle(world: &mut World, throttle: f32) -> Entity {
        let mut rb = RigidBody::new(900.0, true);
        let collider = box_collider(0.9, 0.3, 2.0);
        rb.update_inertia_from_collider(&collider);
        rb.wake_up();

        let mut vc = make_vehicle();
        vc.throttle_input = throttle;

        let e = world.spawn();
        world.add_component(e, rb);
        world.add_component(e, collider);
        world.add_component(e, Transform::new(Vec3::new(0.0, 0.6, 0.0)));
        world.add_component(e, Velocity::default());
        world.add_component(e, vc);
        e
    }

    /// Forward is `-Z` (see `update_vehicle`): applying throttle over several
    /// fixed steps must move the chassis in `-Z`. Drives the system function
    /// directly (interleaved with the rigid step that integrates the forces).
    #[test]
    fn vehicle_drives_forward_when_called_directly() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        spawn_ground(&mut world);
        let car = spawn_vehicle(&mut world, 1.0);

        let start_z = world
            .query::<&Transform>()
            .unwrap()
            .get(car.id())
            .unwrap()
            .position
            .z;

        let dt = 1.0 / 120.0;
        for _ in 0..300 {
            vehicle_controller_system(&world, dt);
            physics_step_system(&world, dt);
        }

        let end = *world.query::<&Transform>().unwrap().get(car.id()).unwrap();
        assert!(end.position.is_finite(), "vehicle position went non-finite: {:?}", end.position);
        assert!(
            start_z - end.position.z > 0.2,
            "vehicle should drive forward (-Z): start_z {start_z} -> end_z {} (Δ {})",
            end.position.z,
            start_z - end.position.z
        );
    }

    /// Same scenario, but the systems are wired into a `Schedule` (vehicle
    /// controller `.before("physics_step_system")` in `Phase::Physics`).
    #[test]
    fn vehicle_drives_forward_via_schedule() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());
        spawn_ground(&mut world);
        let car = spawn_vehicle(&mut world, 1.0);

        let start_z = world
            .query::<&Transform>()
            .unwrap()
            .get(car.id())
            .unwrap()
            .position
            .z;

        let mut schedule = Schedule::new();
        schedule.add_di_system(
            SystemConfig::new(Box::new(vehicle_controller_system))
                .in_phase(Phase::Physics)
                .label("vehicle_controller_system")
                .before("physics_step_system"),
        );
        schedule.add_di_system(
            SystemConfig::new(Box::new(physics_step_system))
                .in_phase(Phase::Physics)
                .label("physics_step_system"),
        );

        let dt = 1.0 / 120.0;
        for _ in 0..300 {
            schedule.run(&mut world, dt);
        }

        let end = *world.query::<&Transform>().unwrap().get(car.id()).unwrap();
        assert!(end.position.is_finite());
        assert!(
            start_z - end.position.z > 0.2,
            "vehicle (schedule) should drive forward (-Z): Δz {}",
            start_z - end.position.z
        );
    }

    /// A KCC with an `+X` target velocity walks forward on flat ground. The KCC
    /// does its own integration, so no `RigidBody` / physics step is involved.
    #[test]
    fn character_walks_forward_on_flat_ground() {
        let mut world = World::new();
        spawn_ground(&mut world);

        let kcc = CharacterController {
            target_velocity: Vec3::new(2.0, 0.0, 0.0),
            is_grounded: true,
            ..Default::default()
        };
        let e = world.spawn();
        world.add_component(e, kcc);
        world.add_component(e, Collider::capsule(0.3, 0.6));
        world.add_component(e, Transform::new(Vec3::new(0.0, 0.9, 0.0)));
        world.add_component(e, Velocity::default());

        let dt = 1.0 / 60.0;
        for _ in 0..60 {
            character_controller_system(&world, dt);
        }

        let pos = world.query::<&Transform>().unwrap().get(e.id()).unwrap().position;
        assert!(pos.is_finite(), "character position went non-finite: {pos:?}");
        assert!(pos.x > 0.5, "character should have walked forward in +X, got x = {}", pos.x);
    }

    /// A KCC walking into a low ledge (< step_height) climbs onto it: its `y`
    /// rises. Exercises the step/slide path through the ECS system wrapper.
    #[test]
    fn character_steps_up_a_low_ledge() {
        let mut world = World::new();
        // Flat ground (top at y = 0).
        let g = world.spawn();
        world.add_component(g, box_collider(50.0, 0.5, 50.0));
        world.add_component(g, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
        // A 0.15-high step whose vertical face is at x = 1.0.
        let s = world.spawn();
        world.add_component(s, box_collider(25.0, 0.075, 50.0));
        world.add_component(s, Transform::new(Vec3::new(26.0, 0.075, 0.0)));

        // Thin character so the low sweep ray can hit the short step.
        let kcc = CharacterController {
            target_velocity: Vec3::new(2.0, 0.0, 0.0),
            is_grounded: true,
            step_height: 0.3,
            ..Default::default()
        };
        let e = world.spawn();
        world.add_component(e, kcc);
        world.add_component(e, box_collider(0.1, 0.9, 0.1));
        world.add_component(e, Transform::new(Vec3::new(0.88, 0.9, 0.0)));
        world.add_component(e, Velocity::default());

        let dt = 0.0125;
        let start_y = 0.9;
        for _ in 0..40 {
            character_controller_system(&world, dt);
        }

        let pos = world.query::<&Transform>().unwrap().get(e.id()).unwrap().position;
        assert!(pos.is_finite());
        assert!(
            pos.y > start_y + 0.05,
            "character should have stepped up onto the ledge, y {start_y} -> {}",
            pos.y
        );
        assert!(pos.x > 1.0, "character should have advanced past the step face, x = {}", pos.x);
    }

    /// Determinism guard: on a scene with no vehicle/character components, both
    /// gameplay systems are strict no-ops, so a plain rigid-body simulation
    /// evolves bit-identically whether or not they run each step (the
    /// determinism oracle scene relies on this).
    #[test]
    fn gameplay_systems_are_noop_without_components() {
        fn run(with_gameplay: bool) -> Vec3 {
            let mut world = World::new();
            world.insert_resource(PhysicsWorld::new());
            // static floor
            let g = world.spawn();
            world.add_component(g, RigidBody::new_static());
            world.add_component(g, box_collider(50.0, 0.5, 50.0));
            world.add_component(g, Transform::new(Vec3::new(0.0, -0.5, 0.0)));
            world.add_component(g, Velocity::default());
            // a plain falling dynamic box
            let mut rb = RigidBody::new(1.0, true);
            let c = box_collider(0.5, 0.5, 0.5);
            rb.update_inertia_from_collider(&c);
            rb.wake_up();
            let b = world.spawn();
            world.add_component(b, rb);
            world.add_component(b, c);
            world.add_component(b, Transform::new(Vec3::new(0.0, 5.0, 0.0)));
            world.add_component(b, Velocity::default());

            let dt = 1.0 / 120.0;
            for _ in 0..120 {
                if with_gameplay {
                    vehicle_controller_system(&world, dt);
                    character_controller_system(&world, dt);
                }
                physics_step_system(&world, dt);
            }
            world.query::<&Transform>().unwrap().get(b.id()).unwrap().position
        }

        let baseline = run(false);
        let with_systems = run(true);
        assert_eq!(
            baseline, with_systems,
            "gameplay systems must not perturb a plain rigid-body scene"
        );
    }

    /// Track C wiring: vehicle_controller_system, PhysicsWorld.weather'ı okuyup grip'e uygular.
    /// Kar (weather_grip 0.3), güneşe göre belirgin daha az ileri mesafe → weather-oku→wg→grip
    /// zincirini uçtan uca zorlar (tüm eski testler yalnız Sunny idi).
    #[test]
    fn snow_weather_reduces_travel_vs_sunny() {
        use gizmo_physics_rigid::world::Weather;
        fn run(weather: Weather) -> f32 {
            let mut world = World::new();
            let mut pw = PhysicsWorld::new();
            pw.weather = weather;
            world.insert_resource(pw);
            spawn_ground(&mut world);
            let car = spawn_vehicle(&mut world, 1.0);
            let start_z = world.query::<&Transform>().unwrap().get(car.id()).unwrap().position.z;
            let dt = 1.0 / 120.0;
            for _ in 0..300 {
                vehicle_controller_system(&world, dt);
                physics_step_system(&world, dt);
            }
            let end_z = world.query::<&Transform>().unwrap().get(car.id()).unwrap().position.z;
            start_z - end_z // ileri (-Z) kat edilen mesafe
        }
        let sunny = run(Weather::Sunny);
        let snow = run(Weather::Snow);
        assert!(sunny > 0.2, "sunny'de araç ilerlemeli, Δ {sunny}");
        assert!(
            snow < sunny * 0.75,
            "kar hava PhysicsWorld'den okunup grip'i düşürmeli: sunny Δ{sunny:.2} vs snow Δ{snow:.2}"
        );
    }

    /// Track C wiring: PhysicsWorld kaynağı YOKKEN weather unwrap_or_default()=Sunny → panic yok,
    /// araç normal sürülür (fallback yolu).
    #[test]
    fn vehicle_controller_system_ok_without_physics_world_resource() {
        let mut world = World::new();
        spawn_ground(&mut world);
        let car = spawn_vehicle(&mut world, 1.0);
        let dt = 1.0 / 120.0;
        for _ in 0..30 {
            vehicle_controller_system(&world, dt); // yalnız kontrolcü; kuvvetleri Velocity'ye yazar
        }
        let v = world.query::<&Velocity>().unwrap().get(car.id()).unwrap().linear;
        assert!(v.is_finite(), "PhysicsWorld'süz kontrolcü sonlu kalmalı, bulundu {v:?}");
    }
}
