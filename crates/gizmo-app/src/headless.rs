use gizmo_core::system::Schedule;
use gizmo_core::world::World;
use crate::plugin::Plugin;

/// The headless application builder and runtime.
///
/// Exported when the `window` feature is disabled. It owns the ECS
/// [`World`] and [`Schedule`] and runs a minimal update loop without a window
/// or GPU. Builder methods are typically chained ending with [`App::run`], in
/// the order `new` -> `set_setup` -> `set_update` -> `run`.
///
/// Unlike the windowed variant, the setup/update hooks here do not receive a
/// renderer or input handle.
pub struct App<State: 'static = ()> {
    /// The ECS world holding all entities, components and resources.
    pub world: World,
    /// The system schedule executed every fixed simulation step — `0..N` times per tick,
    /// always with the same `dt`. Physics and anything that must be tick-rate independent.
    pub schedule: Schedule,
    /// Mirror of [`windowed::App::update_schedule`](crate::windowed::App::update_schedule),
    /// so a [`Plugin`] can register per-frame systems without knowing which runtime it is
    /// being built into.
    ///
    /// The cadence matches the windowed runtime: `schedule` drains a fixed-timestep
    /// accumulator (`0..N` times per tick, constant `dt`) and this one runs exactly once per
    /// tick with the real elapsed delta. A plugin can therefore be written once and behave
    /// the same in either runtime.
    pub update_schedule: Schedule,
    setup_fn: Option<Box<dyn FnOnce(&mut World) -> State + 'static>>,
    update_fn: Option<Box<dyn FnMut(&mut World, &mut State, f32)>>, // dt
    runner: Option<Box<dyn FnOnce(App<State>)>>,
}

impl<State: 'static> std::fmt::Debug for App<State> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("setup_fn", &self.setup_fn.as_ref().map(|_| "<closure>"))
            .field("update_fn", &self.update_fn.as_ref().map(|_| "<closure>"))
            .field("runner", &self.runner.as_ref().map(|_| "<closure>"))
            .finish_non_exhaustive()
    }
}

impl<State: 'static> App<State> {
    /// A headless application. `title`, `width` and `height` are recorded for logs and for the
    /// window a headless build does not open — they keep the constructor identical to the
    /// windowed runtime's, so moving a program between the two is a type change and nothing else.
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        tracing::info!(title = %title, width, height, "[App:headless] created");
        Self {
            world: World::new(),
            schedule: Schedule::new(),
            update_schedule: Schedule::new(),
            setup_fn: None,
            update_fn: None,
            runner: None,
        }
    }

    /// Replaces what [`run`](Self::run) does with `f`. The escape hatch for a server that owns
    /// its own loop — a fixed-rate tick driven by the network, say, rather than by this crate.
    pub fn set_runner<F>(mut self, f: F) -> Self
    where
        F: FnOnce(App<State>) + 'static,
    {
        self.runner = Some(Box::new(f));
        self
    }

    /// Applies a [`Plugin`] to this app.
    ///
    /// Available when the headless runtime is the crate's root `App` — that is, whenever the
    /// windowed runtime is not compiled in. [`Plugin::build`] is typed against the root
    /// `App`, so with both runtimes present a plugin written for the windowed app cannot be
    /// applied to the headless one; the two are different types.
    ///
    /// Everything else on `headless::App` (`set_setup`, `set_update`, `set_runner`, `run`)
    /// stays available unconditionally, so a simulation server keeps working even if some
    /// unrelated crate in the graph turns `window` on — which is the whole point of the two
    /// runtimes no longer being mutually exclusive. Since `Plugin` speaks `AppLike` rather than
    /// a concrete runtime, this is available in EVERY build — including a windowed one, where a
    /// headless app used to be unable to take a plugin at all.
    pub fn add_plugin<P: Plugin>(mut self, plugin: P) -> Self {
        tracing::info!(plugin = %std::any::type_name::<P>(), "[App:headless] plugin build");
        plugin.build(&mut self);
        self
    }

    /// Builds the simulation's own state once, before the first tick. No renderer is passed —
    /// that is the whole difference from the windowed runtime's `set_setup`.
    pub fn set_setup<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut World) -> State + 'static,
    {
        self.setup_fn = Some(Box::new(f));
        self
    }

    /// The per-tick hook: world, state and `dt`. There is no input parameter, because a headless
    /// app has no window to receive any.
    pub fn set_update<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &mut State, f32) + 'static,
    {
        self.update_fn = Some(Box::new(f));
        self
    }

    /// Registers a system on the fixed-timestep schedule — the same schedule and the same
    /// constant `dt` the windowed runtime steps physics on.
    pub fn add_system<Params, S: gizmo_core::system::IntoSystemConfig<Params>>(
        mut self,
        system: S,
    ) -> Self {
        self.schedule.add_di_system(system);
        self
    }

    /// Configures a system set on the fixed-timestep schedule — ordering and run conditions.
    pub fn configure_set(mut self, config: gizmo_core::system::SetConfig) -> Self {
        self.schedule.configure_set(config);
        self
    }

    /// Runs the headless application.
    ///
    /// If a custom runner was configured via [`set_runner`](Self::set_runner)
    /// it is invoked and `Ok(())` is returned. Otherwise the default update
    /// loop is driven (which does not return under normal operation). Returns
    /// [`AppError::MissingSetup`](crate::AppError::MissingSetup) if no setup
    /// hook was assigned.
    pub fn run(mut self) -> Result<(), crate::AppError> {
        if let Some(runner) = self.runner.take() {
            tracing::info!("[App:headless] delegating to custom runner");
            runner(self);
            return Ok(());
        }
        self.run_default()
    }

    #[tracing::instrument(skip_all, name = "app_headless_run")]
    fn run_default(mut self) -> Result<(), crate::AppError> {
        let mut state = if let Some(setup) = self.setup_fn.take() {
            let s = setup(&mut self.world);
            tracing::info!("[App:headless] setup hook complete");
            s
        } else {
            tracing::error!("[App:headless] setup hook missing; cannot run");
            return Err(crate::AppError::MissingSetup);
        };

        tracing::info!("[App:headless] entering update loop");
        let mut last_time = std::time::Instant::now();

        loop {
            let now = std::time::Instant::now();
            let dt = now.duration_since(last_time).as_secs_f32();
            last_time = now;

            if let Some(update) = self.update_fn.as_mut() {
                update(&mut self.world, &mut state, dt);
            }

            // Same sequencing as the windowed runtime: drain the fixed-timestep accumulator
            // into `schedule`, then run `update_schedule` exactly once.
            //
            // This loop used to call `schedule.run(world, dt)` directly with the real
            // elapsed delta, once per iteration. With the 1 ms sleep below that is roughly a
            // thousand ticks a second, so a server registering `PhysicsPlugin` stepped its
            // physics systems ~1000 times per second at a wall-clock dt, while the same
            // plugin in the windowed runtime stepped at a fixed 60 Hz. A plugin could not be
            // written once and behave the same in both — which for a dedicated server, the
            // one place a fixed step matters most, was the wrong way round.
            // The SIMULATED delta is `Time`'s, not the wall clock's — the same rule the windowed
            // runtime follows, and for the same reason one layer down. This loop used to hand the
            // raw delta straight in and never create a `Time` at all, so a headless game had no
            // `Res<Time>` (no `dt()`, no `elapsed()`, no `frame()`) and `set_time_scale` did
            // nothing here, `0.0` included — pause worked in a window and not on a server.
            let sim_dt = crate::frame::advance_time(&mut self.world, dt);

            crate::frame::run_fixed_and_update(
                &mut self.world,
                &mut self.schedule,
                &mut self.update_schedule,
                sim_dt,
                dt,
                |_| {},
            );

            // Flush deferred commands (Commands/CommandQueue) queued by the update
            // hook — mirrors the windowed loop. `Schedule::run` only flushes BETWEEN
            // batches, so with no systems registered nothing would flush and the
            // update hook's spawns/despawns would never take effect.
            self.world.apply_commands();

            // Simple busy wait or sleep to avoid 100% CPU in headless if not limited
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

impl<State: 'static> crate::plugin::AppLike for App<State> {
    fn parts_mut(&mut self) -> crate::plugin::AppParts<'_> {
        crate::plugin::AppParts {
            world: &mut self.world,
            schedule: &mut self.schedule,
            update_schedule: &mut self.update_schedule,
        }
    }
}
