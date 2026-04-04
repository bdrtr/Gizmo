use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Window, WindowBuilder},
};

pub struct AppWindow {
    window: Window,
}

impl AppWindow {
    pub fn new(title: &str, width: u32, height: u32, event_loop: &EventLoop<()>) -> Self {
        let window = WindowBuilder::new()
            .with_title(title)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height))
            .build(event_loop)
            .expect("HATA: Pencere oluşturulamadı!");
            
        Self { window }
    }

    pub fn window(&self) -> &Window {
        &self.window
    }
}

// Basit bir test için direkt pencereyi tutan ve ayağa kaldıran örnek kod
pub fn run_window(title: &str, width: u32, height: u32) {
    let event_loop = EventLoop::new().expect("Event Loop başlatılamadı");
    let _app = AppWindow::new(title, width, height, &event_loop);

    event_loop.set_control_flow(ControlFlow::Poll);

    println!("{} {}x{} çözünürlüğünde başlatıldı. Ekranda bir pencere görmelisin!", title, width, height);

    let _ = event_loop.run(move |event, elwt| {
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => elwt.exit(),
            Event::AboutToWait => {
                // Her frame'de yapılacak işler...
            }
            _ => ()
        }
    });
}
