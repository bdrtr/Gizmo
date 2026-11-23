use super::*;

impl<State: 'static> App<State> {
    /// Runs the application.
    ///
    /// If a custom runner was configured via [`set_runner`](Self::set_runner)
    /// it is invoked and `Ok(())` is returned. Otherwise the default windowed
    /// event loop is driven. Returns an [`AppError`](crate::AppError) if the event loop or
    /// window cannot be created, a required setup hook is missing, or the
    /// event loop terminates with an error.
    pub fn run(mut self) -> Result<(), crate::AppError> {
        crate::hotpatch::connect();
        if let Some(runner) = self.runner.take() {
            tracing::info!("[App::run] delegating to custom runner");
            runner(self);
            return Ok(());
        }
        self.run_default()
    }

    fn run_default(mut self) -> Result<(), crate::AppError> {
        crate::setup_panic_hook();
        tracing::info!(
            title = %self.window_title,
            width = self.window_size.0,
            height = self.window_size.1,
            "[App::run] starting windowed event loop"
        );

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

        #[cfg(not(target_os = "android"))]
        let event_loop = EventLoop::new().map_err(crate::AppError::EventLoopCreation)?;
        // Android builds its loop from the activity the host handed over (`android::set_app`);
        // without one, winit's own error says so.
        #[cfg(target_os = "android")]
        let event_loop = {
            use winit::platform::android::EventLoopBuilderExtAndroid;
            let mut builder = EventLoop::builder();
            if let Some(app) = crate::android::app() {
                builder.with_android_app(app);
            } else {
                tracing::error!("[App::run] on Android without `gizmo_app::android::set_app`");
            }
            builder.build().map_err(crate::AppError::EventLoopCreation)?
        };
        // A game on a phone or a tablet fills the screen: the system bars (a tablet's taskbar
        // among them) are hidden unless the game asks for `Windowed`. Inserted, not assumed, so a
        // game that reads the mode to toggle it reads what the screen shows.
        #[cfg(target_os = "android")]
        if self.world.get_resource::<gizmo_core::window::WindowMode>().is_none() {
            self.world.insert_resource(gizmo_core::window::WindowMode::Fullscreen);
        }
        let mut builder = WindowAttributes::default().with_title(&self.window_title);
        // In the browser the page decides the canvas's size: it is styled 100 % of the page below,
        // and winit reports that size as the window's. Asking for an inner size there set the
        // canvas's CSS size to it, over the 100 % — a phone 392 px wide got a 1280 px canvas and
        // showed its top-left corner, the game in the middle of it off screen.
        #[cfg(not(target_arch = "wasm32"))]
        {
            builder = builder.with_inner_size(winit::dpi::LogicalSize::new(
                self.window_size.0,
                self.window_size.1,
            ));
        }

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
            let ran = event_loop.run_app(&mut self).map_err(crate::AppError::EventLoop);
            // The frame still on the render thread is finished here, on this thread, before
            // anything else of the app is dropped: a panic in it is raised now, as any draw's
            // panic is at a sync point — dropping the pipeline joins the thread and has to discard
            // one — and the renderer, whose surface holds the window, goes before the window does.
            // On macOS the last handle on a window must not be dropped off the main thread.
            if let Some(mut gpu) = self.gpu.take() {
                gpu.sync();
            }
            ran?;
            tracing::info!("[App::run] event loop exited");
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
    /// Begins the renderer on `window` — what touches the window, here (`Renderer::start`) — and builds
    /// its pipelines on a thread of their own, which hands the renderer to `try_finish_init` and wakes
    /// the loop. See `PendingInit` for why not here.
    #[cfg(not(target_arch = "wasm32"))]
    fn begin_initialize(&mut self, window: Arc<winit::window::Window>) {
        let (instance, surface, gpu) = pollster::block_on(Renderer::start(window.clone())).split();
        let (handed, built) = std::sync::mpsc::sync_channel(1);
        let waker = window.clone();
        let builder = std::thread::Builder::new()
            .name("gizmo-renderer-build".into())
            .spawn(move || {
                let _ = handed.send(gpu.build());
                waker.request_redraw();
            })
            .expect("the renderer's build thread could not be started");
        self.pending_init = Some(super::PendingInit {
            window,
            instance,
            surface: Some(surface),
            on_screen: true,
            built,
            builder: Some(builder),
        });
    }

    /// Native: if the renderer `begin_initialize` started has been built, gives it its surface — or a
    /// new one, where the window went and came back meanwhile — and runs the rest of the
    /// initialization. Nothing while it is being built; until then `handle_event` returns early, as
    /// `self.window` is `None`. A build that panicked panics here, with its message.
    #[cfg(not(target_arch = "wasm32"))]
    fn try_finish_init(&mut self, event_loop: &ActiveEventLoop) {
        let Some(pending) = self.pending_init.as_mut() else { return };
        let mut renderer = match pending.built.try_recv() {
            Ok(renderer) => renderer,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let result = pending.builder.take().map(std::thread::JoinHandle::join);
                let message = match result {
                    Some(Err(payload)) => payload
                        .downcast_ref::<&str>()
                        .map(|s| (*s).to_string())
                        .or_else(|| payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "a panic with no message".into()),
                    _ => "the build thread ended without a renderer".into(),
                };
                panic!("building the renderer failed: {message}")
            }
        };
        let Some(pending) = self.pending_init.take() else { return };
        let had_surface = pending.surface.is_some();
        renderer.attach_surface(pending.instance, pending.surface);
        if !had_surface && pending.on_screen {
            renderer.resume_surface(pending.window.clone());
        }
        // `GIZMO_VSYNC=0`: present without waiting for the display, for measuring a frame. With it on —
        // the default — a frame on a 60 Hz screen takes 16.7 ms whatever it cost, and one that misses a
        // refresh takes 33.3: what a frame time says is then the screen's, not the frame's.
        #[cfg(not(target_os = "android"))]
        if std::env::var("GIZMO_VSYNC").as_deref() == Ok("0") {
            renderer.set_present_mode(wgpu::PresentMode::AutoNoVsync);
        }
        self.complete_initialize(pending.window, renderer, event_loop);
    }

    /// The renderer is here: the rest of the initialization, then the window's size as it is now — it
    /// may have changed while the renderer was made, and the `Resized` that said so found none to
    /// resize — and the first frame asked for. Both paths' end: the native build thread's and the web's
    /// `spawn_local`.
    fn complete_initialize(&mut self, window: Arc<winit::window::Window>, renderer: Renderer, event_loop: &ActiveEventLoop) {
        if let Err(e) = self.finish_initialize(window.clone(), renderer) {
            tracing::error!("App initialization failed: {}", e);
            self.init_error = Some(e);
            event_loop.exit();
            return;
        }
        // The canvas's real size. The renderer was made from the window's size at creation, before
        // the browser had laid the page out, and the `Resized` that then reported the real one came
        // while the renderer was still being built and found none to resize: a phone's canvas
        // stayed at 640 × 360, stretched over an upright screen, and the camera kept a landscape
        // aspect. Apply it now, as that `Resized` would have.
        let size = window.inner_size();
        let effective = self.renderer_mut().map(|r| {
            if r.size != size {
                r.resize(size);
            }
            r.size
        });
        if let Some(effective) = effective {
            let mut info = self.world.get_resource_mut_or_default::<gizmo_core::window::WindowInfo>();
            info.width = effective.width as f32;
            info.height = effective.height as f32;
        }
        window.request_redraw();
    }

    /// The synchronous tail of initialization, shared by the native path (`try_finish_init`, once the
    /// build thread has made the renderer) and the web path (the renderer arrives later from a
    /// `spawn_local` future — see `try_finish_web_init`).
    #[tracing::instrument(skip_all, name = "app_finish_initialize")]
    fn finish_initialize(
        &mut self,
        window: Arc<winit::window::Window>,
        mut renderer: Renderer,
    ) -> Result<(), crate::AppError> {
        // Initialize Core Dev Console Systems BEFORE setup so setup can register cvars
        self.world
            .insert_resource(gizmo_core::cvar::CVarRegistry::new());
        // Window Resource oluştur ve World'e ekle
        self.world.insert_resource(gizmo_core::window::WindowInfo {
            width: self.window_size.0 as f32,
            height: self.window_size.1 as f32,
        });

        // The game world gets the GPU's creation side (`GpuContext`); the renderer itself stays
        // with the app, for the frame's draw.
        let embedded_count = self.embedded_assets.len();
        renderer.asset_manager.write().unwrap().embedded_assets =
            std::mem::take(&mut self.embedded_assets);
        self.world.insert_resource(renderer.gpu_context());
        tracing::debug!(
            width = self.window_size.0,
            height = self.window_size.1,
            embedded_assets = embedded_count,
            "[App] core resources + GPU context inserted"
        );

        let state = if let Some(setup) = self.setup_fn.take() {
            let state = setup(&mut self.world, &mut renderer);
            tracing::info!("[App] user setup hook complete");
            state
        } else {
            tracing::error!("[App] setup hook missing; cannot initialize");
            return Err(crate::AppError::MissingSetup);
        };

        #[cfg(feature = "scene")]
        if let Some(scene_path) = self.initial_scene.take() {
            if let Some(asset_manager) = self
                .world
                .remove_resource::<gizmo_renderer::asset::AssetManager>()
            {
                {
                    // wasm32: gizmo-scripting (mlua) web'de derlenmez — sahneler
                    // Script bileşeni kaydı olmadan yüklenir (web scripting ertelendi).
                    let registry = crate::scene_registry::scene_registry_for_world(&self.world);
                    // With identity: an asset reference whose path has gone stale is repointed at
                    // the file it names, using the UUID the scene recorded next to that path. Costs
                    // nothing when the registry is empty (every lookup answers "don't know", every
                    // path is left alone), which is the state of any application that never scans
                    // its asset tree — see `crate::asset_identity`.
                    let identity = crate::asset_identity::ManagerIdentity(&asset_manager);
                    match gizmo_scene::scene::SceneData::load_into_with_identity(
                        &scene_path,
                        &mut self.world,
                        &registry,
                        &identity,
                    ) {
                        Ok(()) => {
                            tracing::info!(scene = %scene_path, "[App] initial scene loaded")
                        }
                        Err(e) => tracing::error!(
                            scene = %scene_path,
                            error = %e,
                            "[App::run] initial scene load failed"
                        ),
                    }
                }

                self.world.insert_resource(asset_manager);
            } else {
                tracing::error!("[App::run] AssetManager bulunamadı, sahne yüklenemiyor!");
            }
        }

        #[cfg(feature = "egui")]
        let painter = {
            let (editor, painter) = EguiContext::new(&renderer.device, renderer.config.format, &window, 1);
            self.editor = Some(editor);
            tracing::debug!("[App] egui overlay context initialized");
            painter
        };

        // A render stage's draw belongs to the draw's side, and goes wherever it goes; the other
        // hooks need the game world and the app state, and stay with the app.
        let stage = match std::mem::replace(&mut self.render, RenderHook::None) {
            RenderHook::Stage(stage) => Some(stage),
            other => {
                self.render = other;
                None
            }
        };
        let threaded = super::stage::draws_on_a_render_thread(self.render_thread, stage.is_some());
        if self.render_thread == Some(true) && !threaded {
            tracing::warn!(
                hook = self.render.name(),
                "[App] a render thread was asked for, and every frame is drawn on the loop's thread: \
                 only a render stage can be (another hook writes the game world in the middle of the \
                 draw), and wasm32 has one thread"
            );
        }
        let side = DrawSide {
            renderer,
            #[cfg(feature = "egui")]
            painter,
            stage,
        };
        #[cfg(not(target_arch = "wasm32"))]
        let gpu = if threaded { FramePipeline::threaded(side) } else { FramePipeline::inline(side) };
        #[cfg(target_arch = "wasm32")]
        let gpu = FramePipeline::inline(side);
        tracing::info!(render_thread = gpu.is_threaded(), "[App] draw side ready");

        self.app_state = Some(state);
        self.window = Some(window);
        self.gpu = Some(gpu);
        self.last_frame_time = Some(super::FrameInstant::now());
        self.light_time = 0.0;
        tracing::info!("[App] initialization complete");
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
impl<State: 'static> App<State> {
    /// Web: if the async renderer init started in `resumed` has completed, runs the
    /// remaining synchronous setup. Does nothing if the renderer is not ready yet
    /// (until init finishes, `handle_event` returns early with `self.window == None`).
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
        self.complete_initialize(window, renderer, event_loop);
    }
}

impl<State: 'static> ApplicationHandler for App<State> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Back while the renderer is being built: it is given a new surface when it is done.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(pending) = self.pending_init.as_mut() {
            pending.on_screen = true;
            return;
        }
        // `resumed` can fire more than once (e.g. on mobile); only build the
        // window + runtime the first time.
        if let Some(window) = self.window.clone() {
            // Back on the screen (Android brings an app back with a new native window): draw to
            // it again. Everything else lived through the pause.
            if let Some(renderer) = self.renderer_mut() {
                if renderer.surface.is_none() {
                    tracing::info!("[App] resumed: making the surface again");
                    renderer.resume_surface(window.clone());
                    window.request_redraw();
                }
            }
            return;
        }
        tracing::debug!("[App] resumed: creating window + runtime");
        let attrs = match self.window_attributes.take() {
            Some(a) => a,
            None => {
                tracing::warn!("[App] resumed without stashed window attributes; skipping init");
                return;
            }
        };
        #[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
        let attrs = match on_requested_monitor(event_loop, attrs) {
            Some(attrs) => attrs,
            None => {
                event_loop.exit();
                return;
            }
        };
        let window = match event_loop.create_window(attrs) {
            Ok(w) => {
                tracing::debug!("[App] window created");
                Arc::new(w)
            }
            Err(e) => {
                tracing::error!("Window creation failed: {}", e);
                // `run`/`run_default`'ın belgeli hata sözleşmesini onurlandır:
                // aksi halde pencere kurulamayınca run() yine de Ok(()) dönerdi.
                self.init_error = Some(crate::AppError::WindowCreation(e));
                event_loop.exit();
                return;
            }
        };
        // The pipelines are built off this thread; `try_finish_init` finishes the initialization when
        // they are (an error there is stashed for `run` to return, as before).
        #[cfg(not(target_arch = "wasm32"))]
        self.begin_initialize(window);
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

    /// Off the screen: Android is about to destroy the native window, and the surface must not
    /// outlive it. Nothing is drawn until `resumed` makes another.
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        // Gone while the renderer is being built: its surface goes now, which the build never touches.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(pending) = self.pending_init.as_mut() {
            tracing::info!("[App] suspended while the renderer is built: letting the surface go");
            pending.surface = None;
            pending.on_screen = false;
            return;
        }
        if let Some(renderer) = self.renderer_mut() {
            tracing::info!("[App] suspended: letting the surface go");
            renderer.suspend_surface();
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
        #[cfg(not(target_arch = "wasm32"))]
        self.try_finish_init(event_loop);
        self.handle_event(Event::WindowEvent { window_id, event }, event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_arch = "wasm32")]
        self.try_finish_web_init(event_loop);
        #[cfg(not(target_arch = "wasm32"))]
        self.try_finish_init(event_loop);
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

/// `GIZMO_MONITOR=<name>`: the window opens borderless-fullscreen on the monitor whose name holds
/// `<name>` — on Linux the output's own name, `DP-1`, `HDMI-A-1`. A development switch: a measurement
/// or a screenshot run that must not cover the screen being worked on. Fullscreen because that is the
/// one way to put a window on a given screen under Wayland, where a client cannot place its own; the
/// frame is then the size of that screen, the same every run.
///
/// `None` — no window at all, and an error naming the monitors there are — when no monitor has that
/// name: the switch exists to keep a window off a screen, and opening it wherever the compositor
/// chose would put it exactly there.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
fn on_requested_monitor(
    event_loop: &winit::event_loop::ActiveEventLoop,
    attrs: winit::window::WindowAttributes,
) -> Option<winit::window::WindowAttributes> {
    let Ok(wanted) = std::env::var("GIZMO_MONITOR") else { return Some(attrs) };
    match event_loop.available_monitors().find(|m| m.name().is_some_and(|n| n.contains(&wanted))) {
        Some(monitor) => Some(attrs.with_fullscreen(Some(winit::window::Fullscreen::Borderless(Some(monitor))))),
        None => {
            let names: Vec<String> = event_loop.available_monitors().filter_map(|m| m.name()).collect();
            tracing::error!("[App] GIZMO_MONITOR={wanted}: no monitor by that name (there are {names:?}); not opening a window");
            eprintln!("GIZMO_MONITOR={wanted}: no monitor by that name (there are {names:?}); not opening a window");
            None
        }
    }
}
