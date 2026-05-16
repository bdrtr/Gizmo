use egui::Context;
use egui_wgpu::{Renderer, ScreenDescriptor};
use egui_winit::State;
use winit::event::WindowEvent;
use winit::window::Window;

pub struct EditorContext {
    pub context: Context,
    pub state: State,
    pub renderer: Renderer,
    pub frame_count: usize,
}

impl EditorContext {
    pub fn new(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        window: &Window,
        sample_count: u32,
    ) -> Self {
        let context = Context::default();

        let fonts = egui::FontDefinitions::default();
        // TODO: Load missing Emoji/Turkish TTF bytes here for comprehensive support.
        // fonts.font_data.insert("emoji".to_owned(), egui::FontData::from_static(include_bytes!("...")));
        context.set_fonts(fonts);

        let viewport_id = context.viewport_id();
        let state = State::new(
            context.clone(),
            viewport_id,
            window,
            Some(window.scale_factor() as f32),
            None,
        );

        let renderer = Renderer::new(device, output_format, None, sample_count);

        let ctx = Self {
            context,
            state,
            renderer,
            frame_count: 0,
        };
        ctx.apply_theme();
        ctx
    }

    pub fn apply_theme(&self) {
        let mut visuals = egui::Visuals::dark();
        
        // Modern, sleek rounding
        let widget_rounding = egui::Rounding::same(6.0);
        visuals.window_rounding = egui::Rounding::same(10.0);
        visuals.menu_rounding = egui::Rounding::same(8.0);
        visuals.widgets.noninteractive.rounding = widget_rounding;
        visuals.widgets.inactive.rounding = widget_rounding;
        visuals.widgets.hovered.rounding = widget_rounding;
        visuals.widgets.active.rounding = widget_rounding;
        visuals.widgets.open.rounding = widget_rounding;

        // Modern Dark Colors (similar to Unreal Engine / VS Code)
        visuals.window_fill = egui::Color32::from_rgb(28, 28, 30); // Deep dark gray
        visuals.panel_fill = egui::Color32::from_rgb(34, 34, 36);  // Slightly lighter panel
        visuals.faint_bg_color = egui::Color32::from_rgb(42, 42, 45);

        // Widget Backgrounds
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(45, 45, 48); // Buttons
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(58, 58, 62);  // Hover
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(70, 70, 75);   // Clicked

        // Accent Color: Modern Soft Blue/Purple
        let accent_color = egui::Color32::from_rgb(64, 120, 240); // Soft Tech Blue
        visuals.selection.bg_fill = accent_color;
        visuals.selection.stroke = egui::Stroke::new(1.0, accent_color);
        
        // Improve text contrast slightly
        visuals.override_text_color = Some(egui::Color32::from_rgb(230, 230, 230));

        self.context.set_visuals(visuals);

        // Improve spacing and padding for a less cluttered look
        let mut style = (*self.context.style()).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(12.0);
        style.spacing.interact_size = egui::vec2(40.0, 24.0); // Make clickable areas taller
        
        self.context.set_style(style);
    }

    pub fn handle_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        let response = self.state.on_window_event(window, event);
        response.consumed
    }

    pub fn run<F>(&mut self, window: &Window, ui_fn: F) -> egui::FullOutput
    where
        F: FnOnce(&Context),
    {
        let raw_input = self.state.take_egui_input(window);
        self.context.begin_frame(raw_input);
        ui_fn(&self.context);
        self.context.end_frame()
    }

    // Oyun Çizildikten SONRA bu fonksiyon ekrana Overlay UI çizdirecek!
    pub fn render(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        full_output: egui::FullOutput,
    ) {
        self.state
            .handle_platform_output(window, full_output.platform_output);
        self.frame_count += 1;

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
                label: Some(&format!("Egui Render Pass #{}", self.frame_count)),
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
