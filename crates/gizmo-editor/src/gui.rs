use egui::Context;
use egui_wgpu::{Renderer, ScreenDescriptor};
use egui_winit::State;
use winit::event::WindowEvent;
use winit::window::Window;

pub struct EditorContext {
    pub context: Context,
    pub state: State,
    pub renderer: Renderer,
}

impl EditorContext {
    pub fn new(device: &wgpu::Device, output_format: wgpu::TextureFormat, window: &Window) -> Self {
        let context = Context::default();

        // 1. Gelişmiş Dark Tema ve Renkler (Professional Look)
        let mut visuals = egui::Visuals::dark();
        visuals.window_rounding = egui::Rounding::same(8.0);
        visuals.menu_rounding = egui::Rounding::same(6.0);
        let widget_rounding = egui::Rounding::same(4.0);
        visuals.widgets.noninteractive.rounding = widget_rounding;
        visuals.widgets.inactive.rounding = widget_rounding;
        visuals.widgets.hovered.rounding = widget_rounding;
        visuals.widgets.active.rounding = widget_rounding;

        // Dark Slate (Uzay Grisi) Tonları
        visuals.window_fill = egui::Color32::from_rgb(22, 22, 24);
        visuals.panel_fill = egui::Color32::from_rgb(28, 28, 30);
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(45, 45, 48); // Buton arkaplan
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(60, 60, 63);  // Hover
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(80, 80, 85);   // Click
        visuals.selection.bg_fill = egui::Color32::from_rgb(0, 110, 200);       // Vurgu (Highlight)
        context.set_visuals(visuals);

        // 2. Padding ve Ferahlık (Cramped görünümü iptal eder)
        let mut style = (*context.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.window_margin = egui::Margin::same(10.0);
        context.set_style(style);

        let viewport_id = context.viewport_id();

        let state = State::new(
            context.clone(),
            viewport_id,
            window,
            Some(window.scale_factor() as f32),
            None, // TEMA
        );

        let renderer = Renderer::new(device, output_format, None, 1);

        Self {
            context,
            state,
            renderer,
        }
    }

    pub fn handle_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        let response = self.state.on_window_event(window, event);
        response.consumed
    }

    // Her frame öncesi winit inputlarını Egui Context'e iletir.
    pub fn begin_frame(&mut self, window: &Window) {
        let raw_input = self.state.take_egui_input(window);
        self.context.begin_frame(raw_input);
    }

    // Oyun Çizildikten SONRA bu fonksiyon ekrana Overlay UI çizdirecek!
    pub fn render(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView, // Wgpu'nun sahneyi boyadığı tuvalin referansı
    ) {
        let full_output = self.context.end_frame();
        self.state
            .handle_platform_output(window, full_output.platform_output.clone());

        let paint_jobs = self
            .context
            .tessellate(full_output.shapes, window.scale_factor() as f32);

        // Dokuları Yükle (Fontlar, Pencereler vs)
        for (id, image_delta) in &full_output.textures_delta.set {
            self.renderer
                .update_texture(device, queue, *id, image_delta);
        }

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: [window.inner_size().width, window.inner_size().height],
            pixels_per_point: window.scale_factor() as f32,
        };

        self.renderer
            .update_buffers(device, queue, encoder, &paint_jobs, &screen_descriptor);

        // -- EGUI ÇİZİCİSİNİ AKTİFLEŞTİR: Motorun Pass'inin Üzerine Ek Çizim Yapar --
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Egui Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Önceki çizimleri SİLME, ÜZERİNE Bindir!
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            self.renderer
                .render(&mut render_pass, &paint_jobs, &screen_descriptor);
        }

        // Eski dokuları sil
        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
