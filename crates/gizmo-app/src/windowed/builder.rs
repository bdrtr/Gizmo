use super::*;

impl<State: 'static> App<State> {
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        crate::setup_panic_hook();
        let mut app = Self {
            world: World::new(),
            schedule: Schedule::new(),
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

    pub fn set_runner<F>(mut self, f: F) -> Self
    where
        F: FnOnce(App<State>) + 'static,
    {
        self.runner = Some(Box::new(f));
        self
    }

    pub fn set_runner_mut<F>(&mut self, f: F)
    where
        F: FnOnce(App<State>) + 'static,
    {
        self.runner = Some(Box::new(f));
    }

    pub fn start_recording(mut self) -> Self {
        tracing::info!("[App] input recording enabled");
        self.record_mode = true;
        self.record_data = Some(gizmo_core::input::PlaybackData { frames: Vec::new() });
        self
    }

    pub fn start_playback(mut self, path: &str) -> Self {
        tracing::info!(path = %path, "[App] input playback enabled");
        self.playback_file = Some(path.to_string());
        self
    }

    /// Sisteme yeni bir Olay (Event) türü kaydeder.
    /// Bu işlem sayesinde her kare bitişinde çift-buffer `update()` otomatik çalışır.
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

    pub fn with_icon(mut self, icon_bytes: &'static [u8]) -> Self {
        self.window_icon = Some(icon_bytes);
        self
    }

    pub fn add_plugin<P: crate::Plugin<State>>(mut self, plugin: P) -> Self {
        tracing::info!(plugin = %std::any::type_name::<P>(), "[App] plugin build");
        plugin.build(&mut self);
        self
    }

    pub fn add_embedded_asset(mut self, path: &str, data: std::borrow::Cow<'static, [u8]>) -> Self {
        self.embedded_assets.insert(path.to_string(), data);
        self
    }

    pub fn set_setup<F>(mut self, f: F) -> Self
    where
        F: FnOnce(&mut World, &Renderer) -> State + 'static,
    {
        self.setup_fn = Some(Box::new(f));
        self
    }

    pub fn set_update<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &mut State, f32, &gizmo_core::input::Input) + 'static,
    {
        self.update_fn = Some(Box::new(f));
        self
    }

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

    /// Yeni, basit Render fonksiyonunu (RenderContext) ekler
    pub fn set_simple_render<F>(mut self, f: F) -> Self
    where
        F: for<'a> FnMut(&mut World, &State, &mut RenderContext<'a>) + 'static,
    {
        self.simple_render_fn = Some(Box::new(f));
        self
    }

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

    pub fn load_scene(mut self, path: &str) -> Self {
        tracing::debug!(scene = %path, "[App] initial scene queued");
        self.initial_scene = Some(path.to_string());
        self
    }
}
