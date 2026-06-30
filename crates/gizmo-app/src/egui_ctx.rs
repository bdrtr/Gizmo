//! Immediate-mode overlay UI runtime for the windowed app.
//!
//! [`EguiContext`] owns the `egui` runtime, the `egui_winit` platform-input
//! integration and the `egui_wgpu` renderer used to paint an overlay on top of
//! the engine's frame. It is a generic egui integration with no editor-specific
//! knowledge, so it backs both lightweight in-game HUDs (the `set_ui` hook) and
//! the full in-engine editor (the `editor` feature, layered on top).
//!
//! This module is only compiled with the `egui` feature.

use egui::Context;
use egui_wgpu::{Renderer, ScreenDescriptor};
use egui_winit::State;
use winit::event::WindowEvent;
use winit::window::Window;

/// Owns the `egui` runtime, window-input integration and `wgpu` renderer
/// used to draw the overlay UI on top of the engine's frame.
pub struct EguiContext {
    /// The shared `egui` context driving the immediate-mode UI.
    pub context: Context,
    /// `egui_winit` platform state translating window events into egui input.
    pub state: State,
    /// `egui_wgpu` renderer that paints the tessellated UI into a wgpu pass.
    pub renderer: Renderer,
    /// Number of frames rendered so far (used for debug labels).
    pub frame_count: usize,
}

impl EguiContext {
    /// Creates a new egui context for the given `wgpu` device, surface
    /// format and window, applying the default dark theme.
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
            None, // theme (egui-winit 0.34)
            None, // max_texture_side
        );

        let renderer = Renderer::new(
            device,
            output_format,
            egui_wgpu::RendererOptions {
                msaa_samples: sample_count,
                depth_stencil_format: None,
                dithering: true,
                predictable_texture_filtering: false,
            },
        );

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
        let widget_rounding = egui::CornerRadius::same(6);
        visuals.window_corner_radius = egui::CornerRadius::same(10);
        visuals.menu_corner_radius = egui::CornerRadius::same(8);
        visuals.widgets.noninteractive.corner_radius = widget_rounding;
        visuals.widgets.inactive.corner_radius = widget_rounding;
        visuals.widgets.hovered.corner_radius = widget_rounding;
        visuals.widgets.active.corner_radius = widget_rounding;
        visuals.widgets.open.corner_radius = widget_rounding;

        // Modern Dark Colors (similar to Unreal Engine / VS Code)
        visuals.window_fill = egui::Color32::from_rgb(28, 28, 30); // Deep dark gray
        visuals.panel_fill = egui::Color32::from_rgb(34, 34, 36); // Slightly lighter panel
        visuals.faint_bg_color = egui::Color32::from_rgb(42, 42, 45);

        // Widget Backgrounds
        visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(45, 45, 48); // Buttons
        visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(58, 58, 62); // Hover
        visuals.widgets.active.bg_fill = egui::Color32::from_rgb(70, 70, 75); // Clicked

        // Accent Color: Modern Soft Blue/Purple
        let accent_color = egui::Color32::from_rgb(64, 120, 240); // Soft Tech Blue
        visuals.selection.bg_fill = accent_color;
        visuals.selection.stroke = egui::Stroke::new(1.0_f32, accent_color);

        // Improve text contrast slightly
        visuals.override_text_color = Some(egui::Color32::from_rgb(230, 230, 230));

        self.context.set_visuals(visuals);

        // Improve spacing and padding for a less cluttered look
        let mut style = (*self.context.global_style()).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 6.0);
        style.spacing.window_margin = egui::Margin::same(12);
        style.spacing.interact_size = egui::vec2(40.0, 24.0); // Make clickable areas taller

        self.context.set_global_style(style);
    }

    /// Forwards a window event to egui; returns `true` if egui consumed it.
    pub fn handle_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        let response = self.state.on_window_event(window, event);
        response.consumed
    }

    /// Runs one egui frame, invoking `ui_fn` to build the UI, and returns the
    /// resulting [`egui::FullOutput`] to be passed to [`Self::render`].
    pub fn run<F>(&mut self, window: &Window, ui_fn: F) -> egui::FullOutput
    where
        F: FnOnce(&Context),
    {
        let raw_input = self.state.take_egui_input(window);
        self.context.begin_pass(raw_input);
        ui_fn(&self.context);
        self.context.end_pass()
    }

    /// Paints the overlay UI on top of the already-rendered engine frame, using
    /// the output produced by [`Self::run`].
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
            let mut render_pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some(&format!("Egui Render Pass #{}", self.frame_count)),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load, // Önceki çizimleri SİLME, ÜZERİNE Bindir!
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                // egui-wgpu 0.34's `render` wants a `RenderPass<'static>`.
                .forget_lifetime();

            self.renderer
                .render(&mut render_pass, &paint_jobs, &screen_descriptor);
        }

        // Eski dokuları sil
        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
