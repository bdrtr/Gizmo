use super::*;

impl<State: 'static> App<State> {
    /// Runs the application.
    ///
    /// If a custom runner was configured via [`set_runner`](Self::set_runner)
    /// it is invoked and `Ok(())` is returned. Otherwise the default windowed
    /// event loop is driven. Returns an [`AppError`] if the event loop or
    /// window cannot be created, a required setup hook is missing, or the
    /// event loop terminates with an error.
    pub fn run(mut self) -> Result<(), crate::AppError> {
        if let Some(runner) = self.runner.take() {
            runner(self);
            return Ok(());
        }
        self.run_default()
    }

    fn run_default(mut self) -> Result<(), crate::AppError> {
        crate::setup_panic_hook();

        if let Some(ref path) = self.playback_file {
            match gizmo_core::input::PlaybackData::load(path) {
                Ok(data) => {
                    self.playback_data = Some(data);
                    tracing::info!("Playback loaded from: {}", path);
                }
                Err(e) => {
                    tracing::error!("Failed to load playback data: {}", e);
                }
            }
        }

        let event_loop = EventLoop::new().map_err(crate::AppError::EventLoopCreation)?;
        let mut builder = WindowAttributes::default()
            .with_title(&self.window_title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.window_size.0,
                self.window_size.1,
            ));

        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            use winit::platform::web::WindowAttributesExtWebSys;
            // Canvas'ı body'ye ekle ve boyutu ayarla
            let canvas = web_sys::window()
                .and_then(|win| win.document())
                .and_then(|doc| {
                    let canvas = doc.create_element("canvas").ok()?;
                    let canvas: web_sys::HtmlCanvasElement = canvas.dyn_into().ok()?;
                    canvas.set_width(1280);
                    canvas.set_height(720);
                    canvas.style().set_property("width", "100%").ok()?;
                    canvas.style().set_property("height", "100%").ok()?;
                    doc.body()?.append_child(&canvas).ok()?;
                    Some(canvas)
                });
            if let Some(canvas) = canvas {
                builder = builder.with_canvas(Some(canvas));
            } else {
                builder = builder.with_append(true);
            }
        }

        if let Some(icon_bytes) = self.window_icon {
            if let Ok(image) = image::load_from_memory(icon_bytes) {
                let rgba = image.into_rgba8();
                let (width, height) = rgba.dimensions();
                if let Ok(icon) = winit::window::Icon::from_rgba(rgba.into_raw(), width, height) {
                    builder = builder.with_window_icon(Some(icon));
                }
            }
        }

        // winit 0.30: the window is created lazily in `ApplicationHandler::resumed`
        // (the only place a `&ActiveEventLoop` — and thus the non-deprecated
        // `create_window` — is available). Stash the attributes for `resumed`,
        // then drive the loop with `run_app`.
        self.window_attributes = Some(builder);

        #[cfg(not(target_arch = "wasm32"))]
        {
            event_loop
                .run_app(&mut self)
                .map_err(crate::AppError::EventLoop)?;
            // `resumed` returns `()`, so a lazy-init failure can't propagate through
            // `run_app`. Surface it here so `run` honors its documented AppError
            // contract (e.g. missing setup hook, renderer build failure).
            if let Some(e) = self.init_error.take() {
                return Err(e);
            }
            Ok(())
        }
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::EventLoopExtWebSys;
            event_loop.spawn_app(self);
            Ok(())
        }
    }

    /// Build the renderer, user state, and editor once the window exists (called
    /// from `resumed`). Stores the resulting runtime into `self`.
    #[cfg(not(target_arch = "wasm32"))]
    async fn initialize(
        &mut self,
        window: Arc<winit::window::Window>,
    ) -> Result<(), crate::AppError> {
        let renderer = Renderer::new(window.clone()).await;
        self.finish_initialize(window, renderer)
    }

    /// The synchronous tail of initialization, shared by the native path
    /// (`initialize`, via `pollster::block_on`) and the web path (the renderer
    /// arrives later from a `spawn_local` future — see `try_finish_web_init`).
    fn finish_initialize(
        &mut self,
        window: Arc<winit::window::Window>,
        renderer: Renderer,
    ) -> Result<(), crate::AppError> {
        // Initialize Core Dev Console Systems BEFORE setup so setup can register cvars
        self.world
            .insert_resource(gizmo_core::cvar::CVarRegistry::new());
        // Window Resource oluştur ve World'e ekle
        self.world.insert_resource(gizmo_core::window::WindowInfo {
            width: self.window_size.0 as f32,
            height: self.window_size.1 as f32,
        });

        // Renderer Resource oluştur ve World'e ekle
        renderer.asset_manager.write().unwrap().embedded_assets =
            std::mem::take(&mut self.embedded_assets);
        self.world.insert_resource(renderer);

        let state = if let Some(setup) = self.setup_fn.take() {
            let r = self
                .world
                .remove_resource::<Renderer>()
                .ok_or(crate::AppError::MissingResource("Renderer"))?;
            let state = setup(&mut self.world, &r);
            self.world.insert_resource(r);
            state
        } else {
            return Err(crate::AppError::MissingSetup);
        };

        #[cfg(feature = "scene")]
        if let Some(scene_path) = self.initial_scene.take() {
            if let Some(asset_manager) = self
                .world
                .remove_resource::<gizmo_renderer::asset::AssetManager>()
            {
                let dummy_rgba = [255, 255, 255, 255];
                let r = self.world.remove_resource::<Renderer>().unwrap();
                let _dummy_bg = r.create_texture(&dummy_rgba, 1, 1);

                {
                    // wasm32: gizmo-scripting (mlua) web'de derlenmez — sahneler
                    // Script bileşeni kaydı olmadan yüklenir (web scripting ertelendi).
                    #[cfg_attr(target_arch = "wasm32", allow(unused_mut))]
                    let mut registry = gizmo_scene::registry::default_scene_registry();
                    #[cfg(not(target_arch = "wasm32"))]
                    gizmo_scripting::register_script_components(&mut registry);
                    if let Err(e) = gizmo_scene::scene::SceneData::load_into(
                        &scene_path,
                        &mut self.world,
                        &registry,
                    ) {
                        tracing::error!("[App::run] Sahne yüklenemedi '{}': {}", scene_path, e);
                    }
                }

                self.world.insert_resource(r);
                self.world.insert_resource(asset_manager);
            } else {
                tracing::error!("[App::run] AssetManager bulunamadı, sahne yüklenemiyor!");
            }
        }

        #[cfg(feature = "egui")]
        {
            let editor = {
                let r = self.world.get_resource::<Renderer>().unwrap();
                EguiContext::new(&r.device, r.config.format, &window, 1)
            };
            self.editor = Some(editor);
        }

        self.app_state = Some(state);
        self.window = Some(window);
        self.last_frame_time = Some(super::FrameInstant::now());
        self.light_time = 0.0;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
impl<State: 'static> App<State> {
    /// Web: `resumed`'da başlatılan async renderer init'i tamamlandıysa kalan
    /// senkron kurulumu koşar. Renderer henüz hazır değilse hiçbir şey yapmaz
    /// (init bitene dek `handle_event` `self.window == None` ile erken döner).
    fn try_finish_web_init(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            self.pending_web_init = None;
            return;
        }
        let Some(pending) = self.pending_web_init.as_ref() else {
            return;
        };
        let Some(renderer) = pending.renderer_slot.borrow_mut().take() else {
            return;
        };
        let window = pending.window.clone();
        self.pending_web_init = None;
        if let Err(e) = self.finish_initialize(window.clone(), renderer) {
            tracing::error!("App initialization failed: {}", e);
            self.init_error = Some(e);
            event_loop.exit();
            return;
        }
        // İlk kareyi tetikle — web'de sonraki kareler RedrawRequested
        // zincirinden akar.
        window.request_redraw();
    }
}

impl<State: 'static> ApplicationHandler for App<State> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // `resumed` can fire more than once (e.g. on mobile); only build the
        // window + runtime the first time.
        if self.window.is_some() {
            return;
        }
        let attrs = match self.window_attributes.take() {
            Some(a) => a,
            None => return,
        };
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                tracing::error!("Window creation failed: {}", e);
                event_loop.exit();
                return;
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Err(e) = pollster::block_on(self.initialize(window)) {
                tracing::error!("App initialization failed: {}", e);
                // Stash it so `run`/`run_default` can return the error after the
                // loop exits — otherwise a missing setup hook / renderer failure
                // would be silently swallowed and `run` would return `Ok(())`.
                self.init_error = Some(e);
                event_loop.exit();
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            // Web: `resumed` bloklayamaz. Async WebGPU init'i (adapter/device
            // istekleri) `spawn_local`'a at; renderer hazır olunca slota konur
            // ve `request_redraw` event loop'u uyandırır — ilk window_event /
            // about_to_wait `try_finish_web_init` ile kalan senkron kurulumu
            // (setup hook, sahne, egui) tamamlar.
            let slot: std::rc::Rc<std::cell::RefCell<Option<Renderer>>> =
                std::rc::Rc::new(std::cell::RefCell::new(None));
            self.pending_web_init = Some(super::PendingWebInit {
                window: window.clone(),
                renderer_slot: slot.clone(),
            });
            wasm_bindgen_futures::spawn_local(async move {
                let renderer = Renderer::new(window.clone()).await;
                *slot.borrow_mut() = Some(renderer);
                window.request_redraw();
            });
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        #[cfg(target_arch = "wasm32")]
        self.try_finish_web_init(event_loop);
        self.handle_event(Event::WindowEvent { window_id, event }, event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        self.try_finish_web_init(event_loop);
        self.handle_event(Event::AboutToWait, event_loop);
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        device_id: DeviceId,
        event: DeviceEvent,
    ) {
        self.handle_event(Event::DeviceEvent { device_id, event }, event_loop);
    }
}
