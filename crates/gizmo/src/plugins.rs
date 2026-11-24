// Every item in this module is a physics plugin, so the imports carry that gate too: without the
// feature the module is empty, and these two `use` lines plus the prelude's re-export of it were
// the only things left in it — all three warning, in a configuration nothing linted until
// 2026-08-18.
#[cfg(feature = "physics")]
use crate::app::Plugin;
#[cfg(feature = "physics")]
use crate::math::Vec3;
#[cfg(feature = "physics")]
use gizmo_physics_rigid::world::PhysicsWorld;
/// The substep rate a [`PhysicsPlugin`] is asked for ([`PhysicsPlugin::with_substep_rate`]): the
/// rates a world accepts, the one it starts at, and the error for one outside them.
#[cfg(feature = "physics")]
pub use gizmo_physics_rigid::{SubstepRateError, DEFAULT_SUBSTEP_RATE, SUBSTEP_RATES};

/// Marks the `PhysicsWorld` in a world as one nobody asked for — the simple scene's default
/// (`insert_scene_defaults`), there because a scene needs one. A [`PhysicsPlugin`] built over it
/// replaces it with its own; a world without this mark is the game's, and the plugin keeps it.
///
/// It is what keeps a headless run and a window alike: in a window the plugin builds before the
/// game's setup, so the setup's own world replaces the plugin's and the default is never made; a
/// `HeadlessFrames` run makes the default first and adds the plugin after.
#[cfg(feature = "physics")]
pub(crate) struct DefaultPhysicsWorld;

/// Marks a world whose physics the **fixed schedule** steps — what [`PhysicsPlugin`] leaves behind.
/// A frame hook that would step physics itself reads it and leaves the stepping to the schedule:
/// [`simple_scene_update`](crate::simple::simple_scene_update) does, and
/// [`FixedPhysics::advance`](crate::simple::FixedPhysics::advance) warns. A game with its own loop
/// that steps physics in a system of its own inserts it too.
///
/// Before it, `with_simple_scene` and `PhysicsPlugin` together stepped physics twice a frame — a
/// ball fell 19.5 m in a second instead of 4.9 — and only in the window: a game's `--test` runs the
/// plugin and not the hook.
#[cfg(feature = "physics")]
#[derive(Debug, Clone, Copy, Default)]
pub struct PhysicsSteppedBySchedule;

/// Gizmo Engine Physics Plugin.
/// When added, it initializes the physics world (PhysicsWorld), steps it on the fixed schedule and
/// marks the world so ([`PhysicsSteppedBySchedule`]).
///
/// # The world it steps
///
/// It builds one at its [`gravity`](Self::gravity) and [substep rate](Self::with_substep_rate)
/// when the app has none, or only the simple scene's default. A `PhysicsWorld` the game inserted
/// before adding the plugin is **kept whole** — its rate, its gravity, its bodies and joints — and
/// not replaced: it is the game's, and a window does the same, where the game's setup runs after
/// the plugin and its world replaces the plugin's. Either way, when the plugin was asked for a rate
/// and the world it ends up stepping runs at another, the step says so once, as an error.
///
/// # The two clocks
///
/// The fixed schedule ticks at [`PhysicsTime`](crate::core::time::PhysicsTime)'s rate (60 Hz
/// unless the game sets another) and each tick hands the world one tick of time, which the world
/// spends in its own substeps. A tick must be a whole number of them: at 60 Hz, any world rate
/// that is a multiple of 60 (60, 120, 180, 240, …). Where it first runs, the step checks the two
/// and logs an error naming the rates that would fit ([`substeps_per_tick`](crate::systems::substeps_per_tick)).
#[non_exhaustive]
#[cfg(feature = "physics")]
pub struct PhysicsPlugin {
    /// The world's gravity, in m/s². The default is Earth's, straight down.
    pub gravity: Vec3,
    /// The substep rate asked for ([`with_substep_rate`](Self::with_substep_rate)); `None` builds
    /// the world at [`DEFAULT_SUBSTEP_RATE`].
    substep_rate: Option<u32>,
}

#[cfg(feature = "physics")]
impl Default for PhysicsPlugin {
    fn default() -> Self {
        Self {
            gravity: Vec3::new(0.0, -9.81, 0.0),
            substep_rate: None,
        }
    }
}

#[cfg(feature = "physics")]
impl PhysicsPlugin {
    /// Creates a new PhysicsPlugin with the default gravity.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the gravity vector (chainable).
    pub fn with_gravity(mut self, gravity: Vec3) -> Self {
        self.gravity = gravity;
        self
    }

    /// Builds the world at `hz` substeps a second instead of [`DEFAULT_SUBSTEP_RATE`]
    /// ([`PhysicsWorld::with_substep_rate`]). Keep it a whole multiple of the fixed tick — at the
    /// default 60 Hz tick, 60, 120, 180, 240 … — or every tick runs an uneven number of substeps
    /// (see the type's docs).
    ///
    /// # Errors
    /// [`SubstepRateError::OutOfRange`] outside [`SUBSTEP_RATES`].
    pub fn with_substep_rate(mut self, hz: u32) -> Result<Self, SubstepRateError> {
        // The world's own check, so the two cannot disagree about what a world accepts.
        PhysicsWorld::new().with_substep_rate(hz)?;
        self.substep_rate = Some(hz);
        Ok(self)
    }

    /// The substep rate the plugin builds its world at, in hertz.
    pub fn substep_rate(&self) -> u32 {
        self.substep_rate.unwrap_or(DEFAULT_SUBSTEP_RATE)
    }

    /// The world this plugin builds when the app has none of its own.
    fn build_world(&self) -> PhysicsWorld {
        PhysicsWorld::new()
            .with_gravity(self.gravity)
            .with_substep_rate(self.substep_rate())
            .expect("the rate was checked by `with_substep_rate`, or is the default")
    }
}

#[cfg(feature = "physics")]
impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut dyn gizmo_app::AppLike) {
        let app = app.parts_mut();
        let games_own = app.world.get_resource::<PhysicsWorld>().is_some()
            && app.world.get_resource::<DefaultPhysicsWorld>().is_none();
        if games_own {
            // Kept, not replaced: see the type's docs. A rate asked of the plugin that the kept
            // world does not run at is reported by the step, where it is also caught for a world
            // a setup inserts later.
            let kept = app.world.get_resource::<PhysicsWorld>().map(|p| (p.substep_rate(), p.integrator.gravity));
            tracing::info!(?kept, "[Plugin] PhysicsPlugin keeps the app's own PhysicsWorld (substep rate, gravity)");
        } else {
            tracing::info!(gravity = ?self.gravity, substep_rate = self.substep_rate(), "[Plugin] PhysicsPlugin builds its PhysicsWorld");
            app.world.remove_resource::<DefaultPhysicsWorld>();
            app.world.insert_resource(self.build_world());
        }
        app.world.insert_resource(PhysicsSteppedBySchedule);
        // Run the physics step automatically at the app's fixed timestep (the
        // `PhysicsTime` accumulator loop that also drives `TransformPlugin`), so
        // callers don't hand-call `cpu_physics_step_system` every frame. Labelled
        // so transform systems can order themselves after it if both are added.
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::physics::PhysicsStepSystem::for_rate(self.substep_rate),
            ))
            .label("physics_step"),
        );
        // Resolve any `AutoBoxCollider` markers (box collider sized from Transform.scale)
        // strictly BEFORE the physics step reads them, so a marked body never takes its
        // first step with the placeholder unit box. Registered here (same plugin, same
        // default phase) so the `physics_step` label is guaranteed to exist and the
        // `.before` edge actually binds.
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::auto_collider::AutoBoxColliderSystem,
            ))
            .label("auto_box_collider")
            .before("physics_step"),
        );
        // Bodies drawn between their steps (`systems::interpolation`): the poses either side of each
        // step, and the frame's coefficient once a frame after the steps. After `physics_step` is
        // registered, for the same reason as the collider above: the edges bind to a label that exists.
        app.world.insert_resource(crate::systems::interpolation::InterpolationHistory::default());
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(crate::systems::interpolation::InterpolationBeginSystem))
                .label("interpolation_begin")
                .before("physics_step"),
        );
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(crate::systems::interpolation::InterpolationEndSystem))
                .label("interpolation_end")
                .after("physics_step"),
        );
        app.update_schedule.add_di_system(gizmo_core::system::SystemConfig::new(Box::new(
            crate::systems::interpolation::InterpolationAlphaSystem,
        )));
    }
}

/// The plugin that starts the Transform (hierarchy and synchronization) systems.
#[cfg(feature = "physics")]
pub struct TransformPlugin;

#[cfg(feature = "physics")]
impl Plugin for TransformPlugin {
    fn build(&self, app: &mut dyn gizmo_app::AppLike) {
        let app = app.parts_mut();
        // Per-frame, not per fixed step — and this is an ordering fix as much as a cost one.
        //
        // These ran in the fixed-timestep schedule, so they propagated `0..N` times per
        // rendered frame while the result is only ever consumed once, at draw. The comment
        // on `PhysicsPlugin`'s `physics_step` label says transform systems "can order
        // themselves after it", but no such edge was ever wired — so within a single fixed
        // step the order was whatever the batcher chose.
        //
        // The update schedule runs after *every* fixed step of the frame (see
        // `gizmo_app::frame::run_fixed_and_update`), so "transforms propagate after physics"
        // is now structural rather than dependent on a label edge that did not exist. It is
        // also after the per-frame update systems, which is what a camera moved by
        // `FpsLookSystem` needs.
        //
        // `default_render_pass` still calls `ensure_global_transforms` immediately before
        // drawing. That stays: it is the safety net for a custom `App` that never registers
        // this plugin, and it is what backfills a `GlobalTransform` onto a freshly spawned
        // mesh. With this plugin registered the propagation is simply already current.
        app.update_schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::transform::TransformSyncSystem,
            ))
            .label("transform_sync"),
        );
        app.update_schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(
                crate::systems::transform::TransformPropagateSystem,
            ))
            .label("transform_propagate")
            .after("transform_sync"),
        );
    }
}

#[cfg(all(test, feature = "physics", feature = "window", feature = "render"))]
mod tests {
    use super::*;
    use crate::core::World;
    use crate::simple::HeadlessFrames;

    /// The plugin builds its world at the rate it was asked for, in place of the simple scene's
    /// default one — and at the default rate when it was asked for none.
    #[test]
    fn the_plugin_builds_its_world_at_the_asked_rate() {
        for hz in [60, 120, 240, 480] {
            let plugin = PhysicsPlugin::new().with_gravity(Vec3::new(0.0, -1.62, 0.0)).with_substep_rate(hz).expect("a rate a world accepts");
            assert_eq!(plugin.substep_rate(), hz);
            let run = HeadlessFrames::new(World::new()).add_plugin(plugin);
            let built = run.world.get_resource::<PhysicsWorld>().map(|p| (p.substep_rate(), p.integrator.gravity));
            assert_eq!(built, Some((hz, Vec3::new(0.0, -1.62, 0.0))));
        }
        let run = HeadlessFrames::new(World::new()).add_plugin(PhysicsPlugin::default());
        assert_eq!(run.world.get_resource::<PhysicsWorld>().map(|p| p.substep_rate()), Some(DEFAULT_SUBSTEP_RATE));
    }

    /// A rate no world accepts is refused when it is asked for, with the world's own error.
    #[test]
    fn the_plugin_refuses_a_rate_out_of_range() {
        for hz in [0, SUBSTEP_RATES.start() - 1, SUBSTEP_RATES.end() + 1] {
            assert!(
                matches!(PhysicsPlugin::new().with_substep_rate(hz), Err(SubstepRateError::OutOfRange { hz: h }) if h == hz),
                "{hz} Hz was accepted"
            );
        }
        assert!(PhysicsPlugin::new().with_substep_rate(*SUBSTEP_RATES.start()).is_ok());
        assert!(PhysicsPlugin::new().with_substep_rate(*SUBSTEP_RATES.end()).is_ok());
    }

    /// **A world the game made is kept, not replaced.** It is the game's — its rate, its gravity —
    /// as it is in a window, where the setup's world replaces the plugin's. The plugin used to put a
    /// fresh default world in its place, silently, so a 120 Hz world became a 240 Hz one. Asked for
    /// another rate than the kept world's, the plugin's step says so, once.
    #[test]
    fn the_games_own_world_is_kept_and_a_different_asked_rate_is_reported() {
        let games = || {
            let mut world = World::new();
            world.insert_resource(PhysicsWorld::new().with_gravity(Vec3::new(0.0, -1.62, 0.0)).with_substep_rate(120).expect("in range"));
            world
        };
        let run = HeadlessFrames::new(games()).add_plugin(PhysicsPlugin::default());
        let kept = run.world.get_resource::<PhysicsWorld>().map(|p| (p.substep_rate(), p.integrator.gravity));
        assert_eq!(kept, Some((120, Vec3::new(0.0, -1.62, 0.0))));

        let mut run = HeadlessFrames::new(games()).add_plugin(PhysicsPlugin::new().with_substep_rate(60).expect("in range"));
        let errors = crate::systems::physics::errors_during(|| {
            for _ in 0..10 {
                run.frame(1.0 / 60.0, |_, _, _, _| {});
            }
        });
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].contains("asked for 60 Hz") && errors[0].contains("120 Hz"), "{}", errors[0]);
    }

    /// **The fixed tick and the world's rate are checked against each other.** At the default 60 Hz
    /// tick, a 90 Hz world is reported (once, naming both rates) and a 120 Hz one is not.
    #[test]
    fn a_world_rate_that_is_not_a_multiple_of_the_tick_is_reported() {
        let errors = |hz: u32| {
            let mut run = HeadlessFrames::new(World::new()).add_plugin(PhysicsPlugin::new().with_substep_rate(hz).expect("in range"));
            crate::systems::physics::errors_during(|| {
                for _ in 0..30 {
                    run.frame(1.0 / 60.0, |_, _, _, _| {});
                }
            })
        };
        let at_90 = errors(90);
        assert_eq!(at_90.len(), 1, "{at_90:?}");
        assert!(at_90[0].contains("90 Hz") && at_90[0].contains("60 Hz or 120 Hz"), "{}", at_90[0]);
        assert!(errors(120).is_empty());
    }
}
