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
    /// The system schedule executed every update step.
    pub schedule: Schedule,
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
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        tracing::info!(title = %title, width, height, "[App:headless] created");
        Self {
            world: World::new(),
            schedule: Schedule::new(),
            setup_fn: None,
            update_fn: None,
            runner: None,
        }
    }

    pub fn set_runner<F>(mut self, f: F) -> Self
    where
        F: FnOnce(App<State>) + 'static,
    {
        self.runner = Some(Box::new(f));
        self
    }

    pub fn add_plugin<P: Plugin<State>>(mut self, plugin: P) -> Self {
        tracing::info!(plugin = %std::any::type_name::<P>(), "[App:headless] plugin build");
        plugin.build(&mut self);
        self
    }

    pub fn set_setup<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut World) -> State + 'static,
    {
        self.setup_fn = Some(Box::new(f));
        self
    }

    pub fn set_update<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &mut State, f32) + 'static,
    {
        self.update_fn = Some(Box::new(f));
        self
    }

    pub fn add_system<Params, S: gizmo_core::system::IntoSystemConfig<Params>>(
        mut self,
        system: S,
    ) -> Self {
        self.schedule.add_di_system(system);
        self
    }

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

            self.schedule.run(&mut self.world, dt);

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
