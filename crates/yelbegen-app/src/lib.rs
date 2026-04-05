use std::sync::Arc;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::WindowBuilder,
};
use yelbegen_core::world::World;
use yelbegen_core::system::Schedule;
use yelbegen_renderer::renderer::Renderer;
use yelbegen_editor::gui::EditorContext;

pub struct App<State: 'static> {
    pub world: World,
    pub schedule: Schedule,
    window_title: String,
    window_size: (u32, u32),

    setup_fn: Option<Box<dyn FnOnce(&mut World, &Renderer) -> State>>,
    update_fn: Option<Box<dyn FnMut(&mut World, &mut State, f32, &yelbegen_core::input::Input)>>, // dt, input
    render_fn: Option<Box<dyn FnMut(&mut World, &State, &mut wgpu::CommandEncoder, &wgpu::TextureView, &Renderer, f32)>>, // light_time
    input_fn: Option<Box<dyn FnMut(&mut World, &mut State, &winit::event::Event<()>) -> bool>>, // Input handler
    ui_fn: Option<Box<dyn FnMut(&mut World, &mut State, &egui::Context)>>, // Editor UI handler
    pub input: yelbegen_core::input::Input,
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
            input: yelbegen_core::input::Input::new(),
        }
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
        F: FnMut(&mut World, &mut State, f32, &yelbegen_core::input::Input) + 'static,
    {
        self.update_fn = Some(Box::new(f));
        self
    }

    pub fn set_render<F>(mut self, f: F) -> Self
    where
        F: FnMut(&mut World, &State, &mut wgpu::CommandEncoder, &wgpu::TextureView, &Renderer, f32) + 'static,
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

    pub fn run(mut self) {
        let event_loop = EventLoop::new().expect("Event Loop başlatılamadı");
        let window = Arc::new(
            WindowBuilder::new()
                .with_title(&self.window_title)
                .with_inner_size(winit::dpi::LogicalSize::new(self.window_size.0, self.window_size.1))
                .build(&event_loop)
                .expect("Pencere oluşturulamadı!"),
        );

        let mut renderer = pollster::block_on(Renderer::new(window.clone()));
        
        let mut state = if let Some(setup) = self.setup_fn.take() {
            setup(&mut self.world, &renderer)
        } else {
            panic!("setup() fonksiyonu atanmadi! (App State yaratilamadi)");
        };

        let mut editor = EditorContext::new(&renderer.device, renderer.config.format, &window);

        let mut last_frame_time = std::time::Instant::now();
        let mut light_time = 0.0;

        event_loop.run(move |event, current_window| {
            current_window.set_control_flow(ControlFlow::Poll);

            let mut consumes_input = false;
            
            // UI Entegrasyonu: Winit Olaylarını EGUI'ye Gönder
            if let Event::WindowEvent { ref event, window_id } = event {
                if window_id == window.id() {
                    consumes_input = editor.handle_event(&window, event);
                }
            }

            // Eğer UI girdiyi yakalamadıysa Kullanıcı Input Hook'a Yolla
            if !consumes_input {
                if let Some(input_hk) = self.input_fn.as_mut() {
                    consumes_input = input_hk(&mut self.world, &mut state, &event);
                }
            }

            match event {
                Event::WindowEvent { ref event, window_id } if window_id == window.id() => {
                    if !consumes_input {
                        match event {
                            WindowEvent::CloseRequested => current_window.exit(),
                            WindowEvent::Resized(physical_size) => {
                                renderer.resize(*physical_size);
                                self.input.on_window_resized(physical_size.width as f32, physical_size.height as f32);
                            }
                            WindowEvent::KeyboardInput { event: kb_event, .. } => {
                                if let winit::keyboard::PhysicalKey::Code(keycode) = kb_event.physical_key {
                                    if kb_event.state == winit::event::ElementState::Pressed {
                                        self.input.on_key_pressed(keycode as u32);
                                    } else {
                                        self.input.on_key_released(keycode as u32);
                                    }
                                }
                            }
                            WindowEvent::MouseInput { state: m_state, button, .. } => {
                                let btn_code = match button {
                                    winit::event::MouseButton::Left => yelbegen_core::input::mouse::LEFT,
                                    winit::event::MouseButton::Right => yelbegen_core::input::mouse::RIGHT,
                                    winit::event::MouseButton::Middle => yelbegen_core::input::mouse::MIDDLE,
                                    _ => 99,
                                };
                                if *m_state == winit::event::ElementState::Pressed {
                                    self.input.on_mouse_button_pressed(btn_code);
                                } else {
                                    self.input.on_mouse_button_released(btn_code);
                                }
                            }
                            WindowEvent::CursorMoved { position, .. } => {
                                self.input.on_mouse_moved(position.x as f32, position.y as f32);
                            }
                            _ => {}
                        }
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

                        if let Some(update_hk) = self.update_fn.as_mut() {
                            update_hk(&mut self.world, &mut state, dt, &self.input);
                        }
                        
                        self.input.begin_frame();
                        
                        // ECS Sistemlerini Çalıştır
                        self.schedule.run(&mut self.world, dt);

                        // --- DRAW KISMI ---
                        let output = match renderer.surface.get_current_texture() {
                            Ok(texture) => texture,
                            Err(wgpu::SurfaceError::Outdated) => return,
                            Err(e) => {
                                eprintln!("Surface hatasi: {:?}", e);
                                return;
                            }
                        };

                        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

                        let mut encoder = renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("Render Encoder"),
                        });

                        // Kullaniciya CommandEncoder verip cizdiriyoruz!
                        if let Some(render_hk) = self.render_fn.as_mut() {
                            render_hk(&mut self.world, &state, &mut encoder, &view, &renderer, light_time);
                        }

                        editor.render(&window, &renderer.device, &renderer.queue, &mut encoder, &view);

                        renderer.queue.submit(std::iter::once(encoder.finish()));
                        output.present();
                    }
                }
                Event::AboutToWait => {
                    window.request_redraw();
                }
                Event::DeviceEvent { event: winit::event::DeviceEvent::MouseMotion { delta }, .. } => {
                    self.input.on_mouse_delta(delta.0 as f32, delta.1 as f32);
                }
                _ => {}
            }
        }).unwrap();
    }
}
