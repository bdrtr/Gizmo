use super::*;

impl<State: 'static> App<State> {
    /// A windowed application with a title and an initial inner size in physical pixels.
    ///
    /// Nothing is created yet: the window, the GPU device and the user state all come up on the
    /// first `resumed` event, once [`run`](Self::run) has an event loop to build them from. That
    /// is why `new` cannot fail and why a builder method may be called in any order after it.
    ///
    /// The engine's panic hook is installed here, and [`AssetPlugin`](super::AssetPlugin) is
    /// added so the renderer's asset collections exist before any setup hook runs.
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        crate::setup_panic_hook();
        let mut app = Self {
            world: World::new(),
            schedule: Schedule::new(),
            update_schedule: Schedule::new(),
            window_title: title.to_string(),
            window_size: (width, height),
            setup_fn: None,
            update_fn: None,
            render_fn: None,
            simple_render_fn: None,
            input_fn: None,
            #[cfg(feature = "egui")]
            ui_fn: None,
            input: gizmo_core::input::Input::new(),
            #[cfg(feature = "gamepad")]
            gamepad_backend: None,
            event_updaters: Vec::new(),
            initial_scene: None,
            window_icon: None,
            record_mode: false,
            playback_file: None,
            record_data: None,
            playback_data: None,
            playback_frame_index: 0,
            runner: None,
            embedded_assets: std::collections::HashMap::new(),
            window_attributes: None,
            window: None,
            #[cfg(feature = "egui")]
            editor: None,
            app_state: None,
            #[cfg(target_arch = "wasm32")]
            pending_web_init: None,
            last_frame_time: None,
            light_time: 0.0,
            init_error: None,
        };
        app = app.add_plugin(AssetPlugin);
        tracing::info!(
            title = %app.window_title,
            width,
            height,
            "[App] created (windowed)"
        );
        app
    }

    /// Replaces what [`run`](Self::run) does with `f`, handing it the whole app.
    ///
    /// The escape hatch for an embedder that owns its own event loop — the editor's play mode is
    /// the in-tree example. `f` receives the fully built app and is responsible for everything
    /// afterwards, including creating a window; nothing in the normal loop runs unless it calls
    /// it.
    pub fn set_runner<F>(mut self, f: F) -> Self
    where
        F: FnOnce(App<State>) + 'static,
    {
        self.runner = Some(Box::new(f));
        self
    }

    /// [`set_runner`](Self::set_runner) for the `&mut self` builder style.
    pub fn set_runner_mut<F>(&mut self, f: F)
    where
        F: FnOnce(App<State>) + 'static,
    {
        self.runner = Some(Box::new(f));
    }

    /// Records every frame's input and its duration, and writes the recording out on exit.
    ///
    /// What is captured is the whole [`Input`](gizmo_core::input::Input) snapshot — keys, mouse,
    /// and the gamepad — plus that frame's `dt`, so a replay reproduces one-shot triggers and the
    /// original stepping rather than the replaying machine's frame times.
    pub fn start_recording(mut self) -> Self {
        tracing::info!("[App] input recording enabled");
        self.record_mode = true;
        self.record_data = Some(gizmo_core::input::PlaybackData { frames: Vec::new() });
        self
    }

    /// Replays a recording made by [`start_recording`](Self::start_recording), from `path`.
    ///
    /// Each frame the live input is **overwritten** by the recorded one before any system runs,
    /// so a replay cannot be steered — the keyboard, the mouse and the pad are all ignored while
    /// it plays. A replay only reproduces the original run on a build whose simulation behaves
    /// identically; nothing here checks that, and there is no state or checksum in the file.
    pub fn start_playback(mut self, path: &str) -> Self {
        tracing::info!(path = %path, "[App] input playback enabled");
        self.playback_file = Some(path.to_string());
        self
    }

    /// Registers a new Event type with the system.
    /// Thanks to this, the double-buffer `update()` runs automatically at the end of every frame.
    pub fn add_event<T: 'static + Send + Sync>(mut self) -> Self {
        tracing::debug!(event = %std::any::type_name::<T>(), "[App] event type registered");
        self.world
            .insert_resource(gizmo_core::event::Events::<T>::new());
        self.event_updaters.push(Box::new(|world| {
            if let Some(mut events) = world.get_resource_mut::<gizmo_core::event::Events<T>>() {
                events.update();
            }
        }));
        self
    }

    /// Sets the window icon from encoded image bytes (PNG and the rest of what `image` reads).
    ///
    /// Decoded when the window is created, not here, so a format the decoder refuses — or an
    /// image winit rejects, which includes anything that is not square — costs a warning and the
    /// platform default rather than a failure.
    pub fn with_icon(mut self, icon_bytes: &'static [u8]) -> Self {
        self.window_icon = Some(icon_bytes);
        self
    }

    /// Applies a [`Plugin`](crate::Plugin): it gets to add resources, systems and schedules now,
    /// before the app runs.
    pub fn add_plugin<P: crate::Plugin>(mut self, plugin: P) -> Self {
        tracing::info!(plugin = %std::any::type_name::<P>(), "[App] plugin build");
        plugin.build(&mut self);
        self
    }

    /// Registers bytes to be served as if they were the file at `path`.
    ///
    /// What a shipped build uses instead of a directory: the loaders consult this table before
    /// touching the filesystem, so an `include_bytes!` asset and one on disk are addressed the
    /// same way.
    pub fn add_embedded_asset(mut self, path: &str, data: std::borrow::Cow<'static, [u8]>) -> Self {
        self.embedded_assets.insert(path.to_string(), data);
        self
    }

    /// Builds the game's own state once, after the GPU is up and before the first frame.
    ///
    /// `f` receives the world and the live [`Renderer`], which is what makes it the place to
    /// create meshes, materials and entities; whatever it returns becomes the `State` every other
    /// hook is handed. It runs exactly once, on the first `resumed` event.
    pub fn set_setup<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut World, &Renderer) -> State + 'static,
    {
        self.setup_fn = Some(Box::new(f));
        self
    }

    /// The per-frame game hook: world, state, `dt` and the current input.
    ///
    /// The `dt` here is the **raw** frame time (clamped to 50 ms), not the one the schedules see:
    /// `Time`'s `time_scale` is applied to the simulation's delta, so a paused or slow-motion
    /// game still gets a smoothly advancing camera and UI through this hook.
    ///
    /// Runs after input has been gathered for the frame and before the schedules, so
    /// `is_key_just_pressed` and the gamepad's edges are live here.
    pub fn set_update<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &mut State, f32, &gizmo_core::input::Input) + 'static,
    {
        self.update_fn = Some(Box::new(f));
        self
    }

    /// The low-level render hook: raw encoder, target view, renderer and the scene's light time.
    ///
    /// Everything is handed over as wgpu types, so this is the hook for a pass the engine does
    /// not have. Most games want [`set_simple_render`](Self::set_simple_render) instead, which
    /// wraps the same call in a [`RenderContext`] and keeps `wgpu` out of the signature.
    pub fn set_render<F>(mut self, f: F) -> Self
    where
        F: FnMut(
                &mut World,
                &State,
                &mut wgpu::CommandEncoder,
                &wgpu::TextureView,
                &mut Renderer,
                f32,
            ) + 'static,
    {
        self.render_fn = Some(Box::new(f));
        self
    }

    /// Adds the new, simple Render function (RenderContext)
    pub fn set_simple_render<F>(mut self, f: F) -> Self
    where
        F: for<'a> FnMut(&mut World, &State, &mut RenderContext<'a>) + 'static,
    {
        self.simple_render_fn = Some(Box::new(f));
        self
    }

    /// A raw winit event hook, called before the engine interprets the event.
    ///
    /// Returning `true` **consumes** the event: the engine's own handling — key state, mouse
    /// deltas, resize — does not see it. That is what an embedder needs and what a game almost
    /// never does; returning `false` is the normal answer.
    pub fn set_input<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &mut State, &Event<()>) -> bool + 'static,
    {
        self.input_fn = Some(Box::new(f));
        self
    }

    /// Sets the immediate-mode overlay UI hook (egui). Only available with the
    /// `egui` feature.
    #[cfg(feature = "egui")]
    pub fn set_ui<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &mut State, &egui::Context) + 'static,
    {
        self.ui_fn = Some(Box::new(f));
        self
    }

    /// Registers a system on the **fixed-timestep** schedule — the one physics runs on.
    ///
    /// It runs `0..N` times per rendered frame depending on the accumulator, with a constant
    /// `dt`. Right for simulation; wrong for anything reading input edges or driving a camera,
    /// which want [`add_update_system`](Self::add_update_system).
    pub fn add_system<Params, S: gizmo_core::system::IntoSystemConfig<Params>>(
        mut self,
        system: S,
    ) -> Self {
        self.schedule.add_di_system(system);
        self
    }

    
    /// Registers a system on the **per-frame** schedule: it runs exactly once per rendered
    /// frame, with the real frame `dt`.
    ///
    /// This is what gameplay code almost always wants. [`add_system`](Self::add_system)
    /// registers on the fixed-timestep schedule instead, which runs `0..N` times per frame
    /// depending on the accumulator — correct for physics, and silently wrong for anything
    /// reading `Input::is_key_just_pressed` or `mouse_delta`, since those edges are captured
    /// and cleared once per *rendered* frame.
    ///
    /// ```no_run
    /// # use gizmo_app::windowed::App;
    /// App::<()>::new("demo", 800, 600)
    ///     // camera, UI, input — once per rendered frame, with the real frame delta
    ///     .add_update_system(|| { /* … */ })
    ///     // physics-adjacent — constant dt, may run zero or several times per frame
    ///     .add_system(|| { /* … */ });
    /// ```
    pub fn add_update_system<Params, S: gizmo_core::system::IntoSystemConfig<Params>>(
        mut self,
        system: S,
    ) -> Self {
        self.update_schedule.add_di_system(system);
        self
    }

    /// [`add_update_system`](Self::add_update_system) for the `&mut self` builder style.
    pub fn add_update_system_mut<Params, S: gizmo_core::system::IntoSystemConfig<Params>>(
        &mut self,
        system: S,
    ) {
        self.update_schedule.add_di_system(system);
    }

/// Configures a system set on the fixed-timestep schedule — ordering and run conditions.
    pub fn configure_set(mut self, config: gizmo_core::system::SetConfig) -> Self {
        self.schedule.configure_set(config);
        self
    }

    /// Queues a scene file to load once the app is up, after `set_setup` has run.
    ///
    /// Loaded through the app's own asset-identity repair, so a scene whose assets have moved
    /// still finds them by id (see [`asset_identity`](crate::asset_identity)).
    pub fn load_scene(mut self, path: &str) -> Self {
        tracing::debug!(scene = %path, "[App] initial scene queued");
        self.initial_scene = Some(path.to_string());
        self
    }
}
