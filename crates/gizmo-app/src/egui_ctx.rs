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

    /// Applies the editor's visual design.
    ///
    /// The design itself lives in [`crate::editor_theme`], which implements the
    /// `Gizmo Editor Prototype` mockup — palette, geometry and type scale, with the reasoning for
    /// each. It used to be forty lines of literals here; a theme spread through a constructor is
    /// one nobody can read as a whole, and this one had drifted into a different look entirely
    /// (rounded, cool grey, touch-sized) from the design it was meant to be.
    pub fn apply_theme(&self) {
        // The design belongs to the editor, so it lives in `gizmo_editor::theme` — which is also
        // the only place both consumers can see it: this call site, and the editor's own widgets,
        // which reach for the palette directly. `gizmo-editor` sits below `gizmo-app`, so the
        // reverse arrangement would have been a dependency cycle.
        //
        // Without the editor feature this is a plain egui overlay for a game, and imposing the
        // editor's chrome on it would be wrong.
        #[cfg(feature = "editor")]
        gizmo_editor::theme::apply(&self.context);
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
