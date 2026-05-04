pub mod dev_console;

use gizmo_core::system::Schedule;
use gizmo_core::world::World;
use gizmo_editor::gui::EditorContext;
use gizmo_renderer::renderer::Renderer;
use std::sync::Arc;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use std::sync::atomic::{AtomicPtr, Ordering};

static WORLD_PTR: AtomicPtr<gizmo_core::world::World> = AtomicPtr::new(std::ptr::null_mut());

pub fn setup_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        let payload = panic_info.payload();
        let message = if let Some(s) = payload.downcast_ref::<&str>() {
            *s
        } else if let Some(s) = payload.downcast_ref::<String>() {
            s.as_str()
        } else {
            "Bilinmeyen hata"
        };

        let location = if let Some(loc) = panic_info.location() {
            format!("{}:{}", loc.file(), loc.line())
        } else {
            "Bilinmeyen konum".to_string()
        };

        let error_msg = format!("Gizmo Engine Coktu!\n\nKonum: {}\nHata: {}\n\nOlay Yeri Inceleme Raporu 'gizmo_crash_report.json' olarak kaydedildi.", location, message);

        println!("{}", error_msg);
        
        let backtrace = backtrace::Backtrace::new();
        println!("--- BACKTRACE ---\n{:?}", backtrace);

        unsafe {
            let ptr = WORLD_PTR.load(Ordering::Acquire);
            if !ptr.is_null() {
                let world = &*ptr;
                let registry = gizmo_scene::registry::SceneRegistry::default();
                let _ = gizmo_scene::scene::SceneData::save(world, "gizmo_crash_report.json", &registry);
            }
        }

        rfd::MessageDialog::new()
            .set_title("Gizmo Engine Fatal Error")
            .set_description(&error_msg)
            .set_level(rfd::MessageLevel::Error)
            .show();
    }));
}

pub struct App<State: 'static = ()> {
    pub world: World,
    pub schedule: Schedule,
    window_title: String,
    window_size: (u32, u32),

    setup_fn: Option<Box<dyn FnOnce(&mut World, &Renderer) -> State>>,
    update_fn: Option<Box<dyn FnMut(&mut World, &mut State, f32, &gizmo_core::input::Input)>>, // dt, input
    render_fn: Option<
        Box<
            dyn FnMut(
                &mut World,
                &State,
                &mut wgpu::CommandEncoder,
                &wgpu::TextureView,
                &mut Renderer,
                f32,
            ),
        >,
    >, // light_time
    input_fn: Option<Box<dyn FnMut(&mut World, &mut State, &winit::event::Event<()>) -> bool>>, // Input handler
    ui_fn: Option<Box<dyn FnMut(&mut World, &mut State, &egui::Context)>>, // Editor UI handler
    pub input: gizmo_core::input::Input,
    #[allow(clippy::type_complexity)]
    event_updaters: Vec<Box<dyn FnMut(&mut World)>>,
    initial_scene: Option<String>,
    window_icon: Option<&'static [u8]>,
    pub record_mode: bool,
    pub playback_file: Option<String>,
    record_data: Option<gizmo_core::input::PlaybackData>,
    playback_data: Option<gizmo_core::input::PlaybackData>,
    playback_frame_index: usize,
}

impl<State: 'static> App<State> {
    pub fn new(title: &str, width: u32, height: u32) -> Self {
        Self {
            world: World::new(),
            schedule: Schedule::new(),
            window_title: title.to_string(),
            window_size: (width, height),
            setup_fn: None,
            update_fn: None,
            render_fn: None,
            input_fn: None,
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
        }
    }

    pub fn start_recording(mut self) -> Self {
        self.record_mode = true;
        self.record_data = Some(gizmo_core::input::PlaybackData { frames: Vec::new() });
        self
    }

    pub fn start_playback(mut self, path: &str) -> Self {
        self.playback_file = Some(path.to_string());
        self
    }

    /// Sisteme yeni bir Olay (Event) türü kaydeder.
    /// Bu işlem sayesinde her kare bitişinde çift-buffer `update()` otomatik çalışır.
    pub fn add_event<T: 'static + Send + Sync>(mut self) -> Self {
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

    pub fn set_input<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &mut State, &Event<()>) -> bool + 'static,
    {
        self.input_fn = Some(Box::new(f));
        self
    }

    pub fn set_ui<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &mut State, &egui::Context) + 'static,
    {
        self.ui_fn = Some(Box::new(f));
        self
    }

    pub fn add_system(mut self, system: fn(&World, f32)) -> Self {
        self.schedule.add_system(system);
        self
    }

    pub fn load_scene(mut self, path: &str) -> Self {
        self.initial_scene = Some(path.to_string());
        self
    }

    pub fn run(mut self) {
        setup_panic_hook();
        WORLD_PTR.store(&mut self.world as *mut _, Ordering::Release);

        if let Some(ref path) = self.playback_file {
            match gizmo_core::input::PlaybackData::load(path) {
                Ok(data) => {
                    self.playback_data = Some(data);
                    println!("Playback loaded from: {}", path);
                }
                Err(e) => {
                    eprintln!("Failed to load playback data: {}", e);
                }
            }
        }
        
        let event_loop = EventLoop::new().expect("Event Loop başlatılamadı");
        let mut builder = WindowBuilder::new()
            .with_title(&self.window_title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.window_size.0,
                self.window_size.1,
            ));

        if let Some(icon_bytes) = self.window_icon {
            if let Ok(image) = image::load_from_memory(icon_bytes) {
                let rgba = image.into_rgba8();
                let (width, height) = rgba.dimensions();
                if let Ok(icon) = winit::window::Icon::from_rgba(rgba.into_raw(), width, height) {
                    builder = builder.with_window_icon(Some(icon));
                }
            }
        }

        let window = Arc::new(builder.build(&event_loop).expect("Pencere oluşturulamadı!"));

        let mut renderer = pollster::block_on(Renderer::new(window.clone()));

        // Initialize Core Dev Console Systems BEFORE setup so setup can register cvars
        self.world.insert_resource(gizmo_core::cvar::CVarRegistry::new());
        self.world.insert_resource(gizmo_core::cvar::DevConsoleState::default());

        let mut state = if let Some(setup) = self.setup_fn.take() {
            setup(&mut self.world, &renderer)
        } else {
            panic!("setup() fonksiyonu atanmadi! Lütfen set_setup çağırın veya State yapılandırmanızı kontrol edin.");
        };

        if let Some(scene_path) = self.initial_scene.take() {
            if let Some(mut asset_manager) = self
                .world
                .remove_resource::<gizmo_renderer::asset::AssetManager>()
            {
                let dummy_rgba = [255, 255, 255, 255];
                let dummy_bg = renderer.create_texture(&dummy_rgba, 1, 1);

                gizmo_scene::scene::SceneData::load_into(
                    &scene_path,
                    &mut self.world,
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                    &mut asset_manager,
                    Arc::new(dummy_bg),
                    &gizmo_scene::registry::SceneRegistry::default(),
                );

                self.world.insert_resource(asset_manager);
            } else {
                eprintln!("[App::run] AssetManager bulunamadı, sahne yüklenemiyor!");
            }
        }


        let mut editor = EditorContext::new(&renderer.device, renderer.config.format, &window, 1);

        let mut last_frame_time = std::time::Instant::now();
        let mut light_time = 0.0;

        event_loop
            .run(move |event, current_window| {
                current_window.set_control_flow(ControlFlow::Poll);

                let mut consumes_input = false;

                // UI Entegrasyonu: Winit Olaylarını EGUI'ye Gönder
                if let Event::WindowEvent {
                    ref event,
                    window_id,
                } = event
                {
                    if window_id == window.id() {
                        consumes_input = editor.handle_event(&window, event);
                    }
                }

                // Eğer UI girdiyi yakalamadıysa Kullanıcı Input Hook'a Yolla
                if !consumes_input {
                    if let Some(input_hk) = self.input_fn.as_mut() {
                        let _ = input_hk(&mut self.world, &mut state, &event);
                    }
                }

                match event {
                    Event::WindowEvent {
                        ref event,
                        window_id,
                    } if window_id == window.id() => {
                        match event {
                            WindowEvent::CloseRequested => {
                                if let Some(record) = &self.record_data {
                                    let _ = record.save("gizmo_record.ron");
                                    println!("Kayit basariyla 'gizmo_record.ron' dosyasina kaydedildi.");
                                }
                                current_window.exit();
                            }
                            WindowEvent::Resized(physical_size) => {
                                renderer.resize(*physical_size);
                                let mut win_info = self
                                    .world
                                    .get_resource_mut_or_default::<gizmo_core::window::WindowInfo>(
                                    );
                                win_info.width = physical_size.width as f32;
                                win_info.height = physical_size.height as f32;
                            }
                            WindowEvent::KeyboardInput {
                                event: kb_event, ..
                            } => {
                                let mut codes_to_press = Vec::new();
                                // Fiziksel Tuş (PhysicalKey)
                                if let winit::keyboard::PhysicalKey::Code(keycode) =
                                    kb_event.physical_key
                                {
                                    codes_to_press.push(keycode as u32);
                                }
                                // Mantıksal Tuş (LogicalKey Fallback)
                                if codes_to_press.is_empty() {
                                    if let winit::keyboard::Key::Character(c) =
                                        kb_event.logical_key.as_ref()
                                    {
                                        match c.to_lowercase().as_str() {
                                            "w" => codes_to_press
                                                .push(winit::keyboard::KeyCode::KeyW as u32),
                                            "a" => codes_to_press
                                                .push(winit::keyboard::KeyCode::KeyA as u32),
                                            "s" => codes_to_press
                                                .push(winit::keyboard::KeyCode::KeyS as u32),
                                            "d" => codes_to_press
                                                .push(winit::keyboard::KeyCode::KeyD as u32),
                                            _ => {}
                                        }
                                    } else if let winit::keyboard::Key::Named(named) =
                                        kb_event.logical_key
                                    {
                                        match named {
                                            winit::keyboard::NamedKey::ArrowUp => codes_to_press
                                                .push(winit::keyboard::KeyCode::ArrowUp as u32),
                                            winit::keyboard::NamedKey::ArrowDown => codes_to_press
                                                .push(winit::keyboard::KeyCode::ArrowDown as u32),
                                            winit::keyboard::NamedKey::ArrowLeft => codes_to_press
                                                .push(winit::keyboard::KeyCode::ArrowLeft as u32),
                                            winit::keyboard::NamedKey::ArrowRight => codes_to_press
                                                .push(winit::keyboard::KeyCode::ArrowRight as u32),
                                            winit::keyboard::NamedKey::Space => codes_to_press
                                                .push(winit::keyboard::KeyCode::Space as u32),
                                            winit::keyboard::NamedKey::Escape => codes_to_press
                                                .push(winit::keyboard::KeyCode::Escape as u32),
                                            _ => {}
                                        }
                                    }
                                } // Ends the 'if codes_to_press.is_empty()' block

                                for code in codes_to_press {
                                    if kb_event.state == winit::event::ElementState::Pressed {
                                        self.input.on_key_pressed(code);
                                    } else {
                                        self.input.on_key_released(code);
                                    }
                                }
                            }
                            WindowEvent::MouseInput {
                                state: m_state,
                                button,
                                ..
                            } => {
                                let btn_code = match button {
                                    winit::event::MouseButton::Left => {
                                        gizmo_core::input::mouse::LEFT
                                    }
                                    winit::event::MouseButton::Right => {
                                        gizmo_core::input::mouse::RIGHT
                                    }
                                    winit::event::MouseButton::Middle => {
                                        gizmo_core::input::mouse::MIDDLE
                                    }
                                    _ => u32::MAX,
                                };
                                if btn_code != u32::MAX {
                                    if *m_state == winit::event::ElementState::Pressed {
                                        self.input.on_mouse_button_pressed(btn_code);
                                    } else {
                                        self.input.on_mouse_button_released(btn_code);
                                    }
                                }
                            }
                            WindowEvent::CursorMoved { position, .. } => {
                                self.input
                                    .on_mouse_moved(position.x as f32, position.y as f32);
                            }
                            _ => {}
                        }
                        if let WindowEvent::RedrawRequested = event {
                            let now = std::time::Instant::now();
                            let mut dt = now.duration_since(last_frame_time).as_secs_f32();
                            dt = dt.min(0.05); // Güvenlik çemberi: Frame takılırsa 50ms'den fazla zıplamayacak, yerçekiminden düşme engellenecek.
                            last_frame_time = now;
                            
                            // Playback / Record mantigi
                            if let Some(playback) = &self.playback_data {
                                if self.playback_frame_index < playback.frames.len() {
                                    let frame = &playback.frames[self.playback_frame_index];
                                    dt = frame.dt;
                                    self.input = frame.input.clone();
                                    self.playback_frame_index += 1;
                                } else {
                                    println!("Playback bitti. Uygulama kapaniyor...");
                                    current_window.exit();
                                }
                            } else if self.record_mode {
                                if let Some(record) = &mut self.record_data {
                                    record.frames.push(gizmo_core::input::FrameRecord {
                                        dt,
                                        input: self.input.clone(),
                                    });
                                }
                            }

                            light_time += dt;

                            // Update
                            let full_output = editor.run(&window, |ctx| {
                                if let Some(ui_hk) = self.ui_fn.as_mut() {
                                    ui_hk(&mut self.world, &mut state, ctx);
                                }
                                
                                // Render Global Dev Console on top of everything
                                dev_console::ui_dev_console(&mut self.world, ctx, &self.input);
                            });


                            // --- Scene View RTT (Render To Texture) YÖNETİMİ ---
                            if self
                                .world
                                .get_resource::<gizmo_editor::EditorState>()
                                .is_some()
                            {
                                let mut ed_state_ref = self
                                    .world
                                    .get_resource_mut::<gizmo_editor::EditorState>()
                                    .unwrap();
                                let scene_w = ed_state_ref
                                    .scene_view_size
                                    .map(|s| s.x as u32)
                                    .unwrap_or(renderer.size.width);
                                let scene_h = ed_state_ref
                                    .scene_view_size
                                    .map(|s| s.y as u32)
                                    .unwrap_or(renderer.size.height);
                                let game_w = ed_state_ref
                                    .game_view_size
                                    .map(|s| s.x as u32)
                                    .unwrap_or(renderer.size.width);
                                let game_h = ed_state_ref
                                    .game_view_size
                                    .map(|s| s.y as u32)
                                    .unwrap_or(renderer.size.height);

                                let mut new_scene_target = None;
                                let mut new_game_target = None;

                                // Scene View RTT
                                let mut needs_recreate_scene = false;
                                if let Some(target) = self
                                    .world
                                    .get_resource::<gizmo_renderer::components::EditorRenderTarget>(
                                ) {
                                    if target.0.width != scene_w || target.0.height != scene_h {
                                        needs_recreate_scene = true;
                                    }
                                } else {
                                    needs_recreate_scene = true;
                                }

                                if needs_recreate_scene && scene_w > 0 && scene_h > 0 {
                                    if let Some(old_id) = ed_state_ref.scene_texture_id {
                                        editor.renderer.free_texture(&old_id);
                                    }
                                    let texture =
                                        renderer.device.create_texture(&wgpu::TextureDescriptor {
                                            label: Some("Editor RTT"),
                                            size: wgpu::Extent3d {
                                                width: scene_w,
                                                height: scene_h,
                                                depth_or_array_layers: 1,
                                            },
                                            mip_level_count: 1,
                                            sample_count: 1,
                                            dimension: wgpu::TextureDimension::D2,
                                            format: renderer.config.format,
                                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                                | wgpu::TextureUsages::TEXTURE_BINDING,
                                            view_formats: &[],
                                        });
                                    let view = texture
                                        .create_view(&wgpu::TextureViewDescriptor::default());
                                    ed_state_ref.scene_texture_id =
                                        Some(editor.renderer.register_native_texture(
                                            &renderer.device,
                                            &view,
                                            wgpu::FilterMode::Linear,
                                        ));
                                    new_scene_target =
                                        Some((std::sync::Arc::new(view), scene_w, scene_h));
                                }

                                // Game View RTT
                                let mut needs_recreate_game = false;
                                if let Some(target) = self
                                    .world
                                    .get_resource::<gizmo_renderer::components::GameRenderTarget>(
                                ) {
                                    if target.0.width != game_w || target.0.height != game_h {
                                        needs_recreate_game = true;
                                    }
                                } else {
                                    needs_recreate_game = true;
                                }

                                if needs_recreate_game && game_w > 0 && game_h > 0 {
                                    if let Some(old_id) = ed_state_ref.game_texture_id {
                                        editor.renderer.free_texture(&old_id);
                                    }
                                    let texture =
                                        renderer.device.create_texture(&wgpu::TextureDescriptor {
                                            label: Some("Game RTT"),
                                            size: wgpu::Extent3d {
                                                width: game_w,
                                                height: game_h,
                                                depth_or_array_layers: 1,
                                            },
                                            mip_level_count: 1,
                                            sample_count: 1,
                                            dimension: wgpu::TextureDimension::D2,
                                            format: renderer.config.format,
                                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                                                | wgpu::TextureUsages::TEXTURE_BINDING,
                                            view_formats: &[],
                                        });
                                    let view = texture
                                        .create_view(&wgpu::TextureViewDescriptor::default());
                                    ed_state_ref.game_texture_id =
                                        Some(editor.renderer.register_native_texture(
                                            &renderer.device,
                                            &view,
                                            wgpu::FilterMode::Linear,
                                        ));
                                    new_game_target =
                                        Some((std::sync::Arc::new(view), game_w, game_h));
                                }

                                drop(ed_state_ref);

                                if let Some((view, w, h)) = new_scene_target {
                                    self.world.insert_resource(
                                        gizmo_renderer::components::EditorRenderTarget(
                                            gizmo_renderer::components::RenderTarget {
                                                view,
                                                width: w,
                                                height: h,
                                            }
                                        ),
                                    );
                                }
                                if let Some((view, w, h)) = new_game_target {
                                    self.world.insert_resource(
                                        gizmo_renderer::components::GameRenderTarget(
                                            gizmo_renderer::components::RenderTarget {
                                                view,
                                                width: w,
                                                height: h,
                                            }
                                        ),
                                    );
                                }
                            }

                            // --- EDITOR SCENE REQUESTS ---
                            // 1. Poll the file-dialog channel and promote result to save/load request.
                            let maybe_dialog_result = {
                                let mut st =
                                    self.world.get_resource_mut::<gizmo_editor::EditorState>();
                                if let Some(ref mut ed) = st {
                                    if let Some(rx_mutex) = ed.pending_dialog_rx.take() {
                                        match rx_mutex.into_inner() {
                                            Ok(rx) => match rx.try_recv() {
                                                Ok((is_save, Some(path))) => {
                                                    Some((is_save, Some(path)))
                                                }
                                                Ok((_, None)) => None, // dialog dismissed
                                                Err(std::sync::mpsc::TryRecvError::Empty) => {
                                                    // still waiting — put it back
                                                    ed.pending_dialog_rx =
                                                        Some(std::sync::Mutex::new(rx));
                                                    None
                                                }
                                                Err(_) => None,
                                            },
                                            Err(_) => None,
                                        }
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                }
                            };
                            if let Some((is_save, Some(path))) = maybe_dialog_result {
                                if let Some(mut ed) =
                                    self.world.get_resource_mut::<gizmo_editor::EditorState>()
                                {
                                    ed.scene_path = path.clone();
                                    if is_save {
                                        ed.scene.save_request = Some(path);
                                    } else {
                                        ed.scene.load_request = Some(path);
                                    }
                                }
                            }

                            // 2. Extract requests before borrowing world mutably.
                            let (save_req, load_req, clear_req) = {
                                if let Some(mut ed) =
                                    self.world.get_resource_mut::<gizmo_editor::EditorState>()
                                {
                                    (
                                        ed.scene.save_request.take(),
                                        ed.scene.load_request.take(),
                                        std::mem::replace(&mut ed.scene.clear_request, false),
                                    )
                                } else {
                                    (None, None, false)
                                }
                            };

                            // 3. Save
                            if let Some(ref path) = save_req {
                                let registry = gizmo_scene::registry::SceneRegistry::default();
                                match gizmo_scene::scene::SceneData::save(
                                    &self.world,
                                    path,
                                    &registry,
                                ) {
                                    Ok(()) => {
                                        if let Some(mut ed) = self
                                            .world
                                            .get_resource_mut::<gizmo_editor::EditorState>()
                                        {
                                            ed.has_unsaved_changes = false;
                                            ed.status_message =
                                                format!("Kaydedildi: {}", path);
                                        }
                                    }
                                    Err(e) => eprintln!("[App] Sahne kayıt hatası: {}", e),
                                }
                            }

                            // 4. Clear + Load
                            if clear_req || load_req.is_some() {
                                let editor_entities: std::collections::HashSet<u32> = {
                                    let names =
                                        self.world.borrow::<gizmo_core::EntityName>();
                                    names
                                        .iter()
                                        .filter_map(|(id, _)| {
                                            names.get(id).and_then(|n| {
                                                if n.0.starts_with("Editor ")
                                                    || n.0 == "Highlight Box"
                                                {
                                                    Some(id)
                                                } else {
                                                    None
                                                }
                                            })
                                        })
                                        .collect()
                                };
                                let to_despawn: Vec<_> = self
                                    .world
                                    .iter_alive_entities()
                                    .into_iter()
                                    .filter(|e| !editor_entities.contains(&e.id()))
                                    .collect();
                                for e in to_despawn {
                                    self.world.despawn(e);
                                }
                            }
                            if let Some(ref path) = load_req {
                                if let Some(mut asset_manager) = self
                                    .world
                                    .remove_resource::<gizmo_renderer::asset::AssetManager>()
                                {
                                    let dummy_rgba = [255u8, 255, 255, 255];
                                    let dummy_bg =
                                        renderer.create_texture(&dummy_rgba, 1, 1);
                                    let registry =
                                        gizmo_scene::registry::SceneRegistry::default();
                                    let ok = gizmo_scene::scene::SceneData::load_into(
                                        path,
                                        &mut self.world,
                                        &renderer.device,
                                        &renderer.queue,
                                        &renderer.scene.texture_bind_group_layout,
                                        &mut asset_manager,
                                        Arc::new(dummy_bg),
                                        &registry,
                                    );
                                    self.world.insert_resource(asset_manager);
                                    if let Some(mut ed) = self
                                        .world
                                        .get_resource_mut::<gizmo_editor::EditorState>()
                                    {
                                        ed.status_message = if ok {
                                            format!("Yüklendi: {}", path)
                                        } else {
                                            format!("Sahne yüklenemedi: {}", path)
                                        };
                                        ed.has_unsaved_changes = false;
                                    }
                                }
                            }

                            // ECS Sistemlerini Çalıştırmadan önce DI için Core Resource'ları Güncelle
                            self.world.insert_resource(self.input.clone());
                            {
                                let has_time = self
                                    .world
                                    .get_resource::<gizmo_core::time::Time>()
                                    .is_some();
                                if has_time {
                                    let mut time = self
                                        .world
                                        .get_resource_mut::<gizmo_core::time::Time>()
                                        .unwrap();
                                    time.update(dt);
                                } else {
                                    let mut time = gizmo_core::time::Time::new();
                                    time.update(dt);
                                    self.world.insert_resource(time);
                                }
                            }

                            // ═══ Fixed Timestep Fizik Döngüsü ═══
                            // PhysicsTime resource'u yoksa oluştur
                            if self.world.get_resource::<gizmo_core::time::PhysicsTime>().is_none() {
                                self.world.insert_resource(gizmo_core::time::PhysicsTime::default());
                            }
                            {
                                let mut phys_time = self
                                    .world
                                    .get_resource_mut::<gizmo_core::time::PhysicsTime>()
                                    .unwrap();
                                phys_time.accumulate(dt);
                            }

                            // Sabit dt'de fizik adımları — frame rate'ten bağımsız
                            loop {
                                let should = self
                                    .world
                                    .get_resource::<gizmo_core::time::PhysicsTime>()
                                    .map(|pt| pt.should_step())
                                    .unwrap_or(false);
                                if !should { break; }

                                let fixed_dt = self
                                    .world
                                    .get_resource::<gizmo_core::time::PhysicsTime>()
                                    .map(|pt| pt.fixed_dt())
                                    .unwrap_or(1.0 / 60.0);

                                // ECS fizik sistemlerini sabit dt ile çalıştır
                                self.schedule.run(&mut self.world, fixed_dt);

                                let mut phys_time = self
                                    .world
                                    .get_resource_mut::<gizmo_core::time::PhysicsTime>()
                                    .unwrap();
                                phys_time.consume_step();
                            }

                            // İnterpolasyon alpha'sını hesapla (render için)
                            {
                                let mut phys_time = self
                                    .world
                                    .get_resource_mut::<gizmo_core::time::PhysicsTime>()
                                    .unwrap();
                                phys_time.compute_alpha();
                            }

                            // Kullanıcı update hook'u (render dt ile — kamera, UI, vb.)
                            if let Some(update_hk) = self.update_fn.as_mut() {
                                update_hk(&mut self.world, &mut state, dt, &self.input);
                            }

                            // Update sonrası olası ertelenmiş komutları (CommandQueue) hemen işle
                            self.world.apply_commands();

                            // --- DYNAMIC FRACTURE & PARTICLE INTEGRATION ---
                            if let Some(physics_world) = self.world.get_resource::<gizmo_physics::world::PhysicsWorld>() {
                                if !physics_world.fracture_events.is_empty() {
                                    if let Some(gpu_particles) = &renderer.gpu_particles {
                                        for event in &physics_world.fracture_events {
                                            let center = [event.impact_point.x, event.impact_point.y, event.impact_point.z];
                                            let dust_color = [0.6, 0.55, 0.5, 0.8]; // Dust color
                                            let force = (event.impact_force * 0.01).clamp(2.0, 15.0);
                                            let particle_count = (event.impact_force * 0.1).clamp(50.0, 500.0) as u32;
                                            gpu_particles.spawn_explosion(&renderer.queue, center, particle_count, dust_color, force);
                                        }
                                    }
                                }
                            }

                            // Olayları Güncelle (Çift-buffer temizliği)
                            for updater in &mut self.event_updaters {
                                updater(&mut self.world);
                            }

                            // --- DRAW KISMI ---
                            let output = match renderer.surface.get_current_texture() {
                                Ok(texture) => texture,
                                Err(wgpu::SurfaceError::Outdated) => return,
                                Err(e) => {
                                    eprintln!("Surface hatasi: {:?}", e);
                                    return;
                                }
                            };

                            let view = output
                                .texture
                                .create_view(&wgpu::TextureViewDescriptor::default());

                            let mut encoder = renderer.device.create_command_encoder(
                                &wgpu::CommandEncoderDescriptor {
                                    label: Some("Render Encoder"),
                                },
                            );

                            // Kullaniciya CommandEncoder verip cizdiriyoruz!
                            if let Some(render_hk) = self.render_fn.as_mut() {
                                render_hk(
                                    &mut self.world,
                                    &state,
                                    &mut encoder,
                                    &view,
                                    &mut renderer,
                                    light_time,
                                );
                            }

                            editor.render(
                                &window,
                                &renderer.device,
                                &renderer.queue,
                                &mut encoder,
                                &view,
                                full_output,
                            );

                            renderer.queue.submit(std::iter::once(encoder.finish()));
                            output.present();
                            
                            // İşlemlerin bitiminde frame-özel input girdilerini (fare delta vs.) temizle
                            self.input.begin_frame();
                        }
                    }
                    Event::AboutToWait => {
                        window.request_redraw();
                    }
                    Event::DeviceEvent {
                        event: winit::event::DeviceEvent::MouseMotion { delta },
                        ..
                    } => {
                        self.input.on_mouse_delta(delta.0 as f32, delta.1 as f32);
                    }
                    _ => {}
                }
            })
            .unwrap();
    }
}
