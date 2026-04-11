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

pub struct App<State: 'static> {
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
        }
    }

    /// Sisteme yeni bir Olay (Event) türü kaydeder.
    /// Bu işlem sayesinde her kare bitişinde çift-buffer `update()` otomatik çalışır.
    pub fn add_event<T: 'static>(mut self) -> Self {
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

    pub fn add_system(mut self, system: fn(&mut World, f32)) -> Self {
        self.schedule.add_system(system);
        self
    }

    pub fn load_scene(mut self, path: &str) -> Self {
        self.initial_scene = Some(path.to_string());
        self
    }

    pub fn run(mut self) {
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

        let mut state = if let Some(setup) = self.setup_fn.take() {
            setup(&mut self.world, &renderer)
        } else {
            panic!("setup() fonksiyonu atanmadi! (App State yaratilamadi)");
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
                );

                self.world.insert_resource(asset_manager);
            } else {
                eprintln!("[App::run] AssetManager bulunamadı, sahne yüklenemiyor!");
            }
        }

        let mut editor = EditorContext::new(&renderer.device, renderer.config.format, &window);

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
                            WindowEvent::CloseRequested => current_window.exit(),
                            WindowEvent::Resized(physical_size) => {
                                renderer.resize(*physical_size);
                                self.input.on_window_resized(
                                    physical_size.width as f32,
                                    physical_size.height as f32,
                                );
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
                                    _ => 99,
                                };
                                if *m_state == winit::event::ElementState::Pressed {
                                    self.input.on_mouse_button_pressed(btn_code);
                                } else {
                                    self.input.on_mouse_button_released(btn_code);
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
                            light_time += dt;

                            // Update
                            editor.begin_frame(&window);
                            if let Some(ui_hk) = self.ui_fn.as_mut() {
                                ui_hk(&mut self.world, &mut state, &editor.context);
                            }

                            // --- Scene View RTT (Render To Texture) YÖNETİMİ ---
                            if self
                                .world
                                .get_resource::<gizmo_editor::EditorState>()
                                .is_some()
                            {
                                let mut ed_state = self
                                    .world
                                    .remove_resource::<gizmo_editor::EditorState>()
                                    .unwrap();

                                let w = renderer.size.width;
                                let h = renderer.size.height;

                                let mut needs_recreate = false;
                                if let Some(target) = self
                                    .world
                                    .get_resource::<gizmo_renderer::components::EditorRenderTarget>(
                                ) {
                                    if target.width != w || target.height != h {
                                        needs_recreate = true;
                                    }
                                } else {
                                    needs_recreate = true;
                                }

                                if needs_recreate && w > 0 && h > 0 {
                                    if let Some(old_id) = ed_state.scene_texture_id {
                                        editor.renderer.free_texture(&old_id);
                                    }

                                    let texture =
                                        renderer.device.create_texture(&wgpu::TextureDescriptor {
                                            label: Some("Editor RTT"),
                                            size: wgpu::Extent3d {
                                                width: w,
                                                height: h,
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

                                    let id = editor.renderer.register_native_texture(
                                        &renderer.device,
                                        &view,
                                        wgpu::FilterMode::Linear,
                                    );

                                    ed_state.scene_texture_id = Some(id);
                                    self.world.insert_resource(
                                        gizmo_renderer::components::EditorRenderTarget {
                                            view: std::sync::Arc::new(view),
                                            width: w,
                                            height: h,
                                        },
                                    );
                                }

                                self.world.insert_resource(ed_state);
                            }

                            if let Some(update_hk) = self.update_fn.as_mut() {
                                update_hk(&mut self.world, &mut state, dt, &self.input);
                            }

                            // ECS Sistemlerini Çalıştırmadan önce DI için Core Resource'ları Güncelle
                            self.world.insert_resource(self.input.clone());
                            self.world.insert_resource(gizmo_core::time::Time {
                                dt,
                                elapsed_seconds: light_time as f64,
                            });

                            // ECS Sistemlerini Çalıştır
                            self.schedule.run(&mut self.world, dt);

                            // Olayları Güncelle (Çift-buffer temizliği)
                            for updater in &mut self.event_updaters {
                                updater(&mut self.world);
                            }

                            // İşlemlerin bitiminde frame-özel input girdilerini temizle
                            self.input.begin_frame();

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
                            );

                            renderer.queue.submit(std::iter::once(encoder.finish()));
                            output.present();
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
