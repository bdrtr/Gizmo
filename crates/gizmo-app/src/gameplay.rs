//! Registration of the high-level gameplay physics systems (Faz 7 M7.2).
//!
//! `gizmo-physics-dynamics` ships the deep controllers (Pacejka vehicle,
//! kinematic character controller, ragdoll) and the thin ECS system wrappers
//! that drive them ([`vehicle_controller_system`](gizmo_physics_dynamics::vehicle_controller_system),
//! [`character_controller_system`](gizmo_physics_dynamics::character_controller_system),
//! [`fighter_frame_system`](gizmo_physics_dynamics::fighter_frame_system)).
//! This module wires those systems into an [`App`](crate::App)'s schedule so a
//! scene author gets working vehicles/characters/fighters "for free".
//!
//! The systems are placed in [`Phase::Physics`](gizmo_core::system::Phase::Physics)
//! and ordered **before** the rigid physics step (label `"physics_step_system"`):
//! the vehicle controller writes suspension/tire forces into `Velocity`, which
//! the physics step then integrates. Register the rigid `physics_step_system`
//! under that same label for the ordering edge to bind.
//!
//! `fighter_frame_system` takes the same ordering edge for a different reason: it does not
//! touch physics state at all, but a fighter's frame counters must be spent in the **fixed**
//! schedule (one call = one frame), and this is the phase that runs there.
//!
//! All three are no-ops on entities without vehicle/character/fighter components, so
//! adding this plugin does not perturb a plain rigid-body simulation
//! (determinism oracle hash is unaffected).

use gizmo_core::system::{Phase, Schedule, SystemConfig};

/// Add `vehicle_controller_system`, `character_controller_system` and
/// `fighter_frame_system` to `schedule` in the physics phase, ordered before
/// `"physics_step_system"`.
pub fn register_gameplay_physics_systems(schedule: &mut Schedule) {
    schedule.add_di_system(
        SystemConfig::new(Box::new(gizmo_physics_dynamics::vehicle_controller_system))
            .in_phase(Phase::Physics)
            .label("vehicle_controller_system")
            .before("physics_step_system"),
    );
    schedule.add_di_system(
        SystemConfig::new(Box::new(gizmo_physics_dynamics::character_controller_system))
            .in_phase(Phase::Physics)
            .label("character_controller_system")
            .before("physics_step_system"),
    );
    schedule.add_di_system(
        SystemConfig::new(Box::new(gizmo_physics_dynamics::fighter_frame_system))
            .in_phase(Phase::Physics)
            .label("fighter_frame_system")
            .before("physics_step_system"),
    );
    tracing::info!(
        "[Gameplay] registered vehicle_controller_system + character_controller_system + fighter_frame_system (Phase::Physics, before physics_step_system)"
    );
}

/// Plugin that registers the gameplay physics systems (see
/// [`register_gameplay_physics_systems`]).
///
/// ```
/// use gizmo_app::gameplay::GameplayPhysicsPlugin;
/// use gizmo_app::App;
/// # use gizmo_core::entity::Entity;
/// # use gizmo_core::world::World;
/// # use gizmo_math::Vec3;
/// # use gizmo_physics_core::components::CharacterController;
/// # use gizmo_physics_core::{BoxShape, Collider, ColliderShape, Transform};
/// # use gizmo_physics_rigid::components::Velocity;
/// # fn spawn_character(world: &mut World, at: Vec3) -> Entity {
/// #     let e = world.spawn();
/// #     world.add_component(e, CharacterController::default());
/// #     world.add_component(e, Transform::new(at));
/// #     world.add_component(e, Velocity::default());
/// #     world.add_component(e, Collider::from_shape(ColliderShape::Box(BoxShape {
/// #         half_extents: Vec3::new(0.3, 0.9, 0.3),
/// #     })));
/// #     e
/// # }
/// let mut app = App::<()>::new("game", 1280, 720)
///     .add_plugin(GameplayPhysicsPlugin);
/// # // `App::new` installs `setup_panic_hook`, which on a `window` build pops a BLOCKING
/// # // rfd modal on any panic. Restore the default hook so a failed assert below reports
/// # // as an ordinary doc-test failure instead of hanging `cargo test` on a dialog.
/// # let _ = std::panic::take_hook();
///
/// // `character_controller_system` now runs in the app's physics phase, so driving
/// // the schedule is enough to make a kinematic character with nothing under it fall.
/// let player = spawn_character(&mut app.world, Vec3::new(0.0, 10.0, 0.0));
/// for _ in 0..60 {
///     app.schedule.run(&mut app.world, 1.0 / 60.0);
/// }
///
/// let y = app.world.query::<&Transform>().unwrap().get(player.id()).unwrap().position.y;
/// assert!(y < 10.0, "the registered controller must have applied its gravity");
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct GameplayPhysicsPlugin;

impl crate::plugin::Plugin for GameplayPhysicsPlugin {
    fn build(&self, app: &mut dyn crate::plugin::AppLike) {
        let app = app.parts_mut();
        register_gameplay_physics_systems(&mut *app.schedule);
    }
}

#[cfg(test)]
mod tests {
    use super::register_gameplay_physics_systems;
    use gizmo_core::system::{Phase, Schedule, SystemConfig};
    use gizmo_core::world::World;
    use gizmo_math::Vec3;
    use gizmo_physics_core::components::FighterController;
    use gizmo_physics_core::{BoxShape, Collider, ColliderShape, Transform};
    use gizmo_physics_dynamics::vehicle::{Axle, VehicleController, Wheel};
    use gizmo_physics_rigid::components::{RigidBody, Velocity};
    use gizmo_physics_rigid::physics_step_system;
    use gizmo_physics_rigid::world::PhysicsWorld;

    fn box_collider(hx: f32, hy: f32, hz: f32) -> Collider {
        Collider::from_shape(ColliderShape::Box(BoxShape {
            half_extents: Vec3::new(hx, hy, hz),
        }))
    }

    /// The plugin's registration function wires the vehicle controller into the
    /// physics phase before the rigid step; driving that schedule moves a
    /// throttled chassis forward (`-Z`). This proves the `.before` ordering edge
    /// binds and the forces land in `Velocity` ahead of integration.
    #[test]
    fn plugin_registers_and_drives_a_vehicle_via_schedule() {
        let mut world = World::new();
        world.insert_resource(PhysicsWorld::new());

        // Static floor (top at y = 0).
        let g = world.spawn();
        world.add_component(g, RigidBody::new_static());
        world.add_component(g, box_collider(100.0, 1.0, 100.0));
        world.add_component(g, Transform::new(Vec3::new(0.0, -1.0, 0.0)));
        world.add_component(g, Velocity::default());

        // A throttled RWD vehicle.
        let mut rb = RigidBody::new(900.0, true);
        let collider = box_collider(0.9, 0.3, 2.0);
        rb.update_inertia_from_collider(&collider);
        rb.wake_up();
        let mut vc = VehicleController::new();
        for (x, z, axle, is_left) in [
            (-0.8, -1.4, Axle::Front, true),
            (0.8, -1.4, Axle::Front, false),
            (-0.8, 1.4, Axle::Rear, true),
            (0.8, 1.4, Axle::Rear, false),
        ] {
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
        vc.throttle_input = 1.0;
        let car = world.spawn();
        world.add_component(car, rb);
        world.add_component(car, collider);
        world.add_component(car, Transform::new(Vec3::new(0.0, 0.6, 0.0)));
        world.add_component(car, Velocity::default());
        world.add_component(car, vc);

        let start_z = world.query::<&Transform>().unwrap().get(car.id()).unwrap().position.z;

        let mut schedule = Schedule::new();
        register_gameplay_physics_systems(&mut schedule);
        // The rigid step must carry the label the controllers order themselves before.
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
            "plugin-driven vehicle should move forward (-Z): Δz {}",
            start_z - end.position.z
        );
    }

    /// The same registration also gives a scene its fight clock: driving the schedule spends a
    /// fighter's hitstop frames, which nothing in the engine did before 0.10.
    ///
    /// It is asserted through the *schedule* rather than by calling the system, because the
    /// registration is the part that can silently not happen — a system in a phase nobody runs
    /// behaves exactly like the missing clock it replaces.
    #[test]
    fn plugin_registers_the_fight_clock() {
        let mut world = World::new();
        let fighter_entity = world.spawn();
        let mut fighter = FighterController::new(1);
        fighter.apply_hitstop(4);
        world.add_component(fighter_entity, fighter);

        let mut schedule = Schedule::new();
        register_gameplay_physics_systems(&mut schedule);

        let mut locked_after = Vec::new();
        for frame in 1..=6 {
            schedule.run(&mut world, 1.0 / 60.0);
            let locked = world
                .borrow::<FighterController>()
                .get(fighter_entity.id())
                .expect("the fighter is still there")
                .is_locked();
            if locked {
                locked_after.push(frame);
            }
        }

        assert_eq!(
            locked_after,
            vec![1, 2, 3],
            "four frames of hitstop must be spent by four runs of the schedule and no more"
        );
    }
}
