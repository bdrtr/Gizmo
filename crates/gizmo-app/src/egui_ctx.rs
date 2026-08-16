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
        // Width and height of `view` in physical pixels — see the note on the screen descriptor.
        target_size: [u32; 2],
        mut full_output: egui::FullOutput,
    ) {
        self.state
            .handle_platform_output(window, full_output.platform_output);
        self.frame_count += 1;

        let paint_jobs = self
            .context
            .tessellate(full_output.shapes, window.scale_factor() as f32);

        // Dokuları Yükle (Fontlar, Pencereler vs)
        // egui 0.36 turned each entry into a LIST of deltas for one texture: a font atlas that
        // grows in several regions in one frame now arrives as several partial updates instead of
        // one. Uploading only the first would leave the rest of the atlas stale — glyphs that
        // render as blank boxes a frame later.
        for (id, image_deltas) in &full_output.textures_delta.set {
            for image_delta in image_deltas {
                self.renderer
                    .update_texture(device, queue, *id, image_delta);
            }
        }

        // The size comes from the TARGET, not from the window.
        //
        // These are the same number until the moment they are not: winit can hand us a redraw with
        // an already-updated `inner_size()` before the `Resized` event that reconfigures the
        // surface has been processed. egui then emits a scissor for the new size against a texture
        // still sized for the old one, and wgpu rejects the whole command encoder:
        //
        //   Scissor Rect { w: 1904, h: 1028 } is not contained in the render target (948, 1028)
        //
        // which is a hard crash on any window resize — a tiling WM rearranging the desktop is
        // enough. A screen descriptor describes the surface being painted, so it has to be read
        // from that surface.
        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: target_size,
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

        // Eski dokuları sil — çizimden SONRA: bu karenin paint job'ları hâlâ o dokulara bakıyor.
        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }

        // Applied, so say so. `TexturesDelta`'s `Drop` is a `debug_assert!` that the delta was
        // handled, and this function handled it by reference — the struct still held both lists,
        // so every debug frame that carried one died here with "Dropped TexturesDelta with 1
        // unapplied deltas". Release builds did not panic and did not need to: the assert is the
        // only thing that fires. That is the worse half of it, because egui emits each delta
        // exactly ONCE — see `absorb_unpainted_frame`, which exists for the same reason.
        full_output.textures_delta.clear();
    }

    /// Take a frame that will never be painted: apply its texture deltas, drop the rest.
    ///
    /// The renderer skips a frame whenever the swapchain image cannot be acquired — an outdated
    /// surface, a resize in flight, a timeout — and the first frame of a freshly mapped window is
    /// the common case, not the rare one. But the egui frame for it has already run, and egui
    /// hands over each texture delta **once**: whoever holds that output owns the only copy of,
    /// say, the font atlas upload. Dropping it costs the atlas for the rest of the run (glyphs
    /// then paint as blank boxes) and, in a debug build, kills the process on epaint's assert.
    ///
    /// So the pixels are what a skipped frame skips — not the uploads.
    pub fn absorb_unpainted_frame(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut full_output: egui::FullOutput,
    ) {
        // The UI ran; its platform side effects (cursor shape, clipboard, IME) are as true for an
        // unpainted frame as for a painted one.
        self.state
            .handle_platform_output(window, full_output.platform_output);

        for (id, image_deltas) in &full_output.textures_delta.set {
            for image_delta in image_deltas {
                self.renderer
                    .update_texture(device, queue, *id, image_delta);
            }
        }
        // Nothing was painted this frame, so unlike `render` there is no paint job left holding a
        // reference and the free list can go immediately.
        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        full_output.textures_delta.clear();
    }
}

#[cfg(test)]
mod screen_descriptor_tests {
    /// egui's screen descriptor must be sized from the render target, never from the window.
    ///
    /// The two agree until a resize, and then they do not: winit can deliver a redraw whose
    /// `inner_size()` is already the new size while the surface is still configured for the old
    /// one. egui emits a scissor for the new size, wgpu rejects the command encoder, and the editor
    /// dies —
    ///
    ///   Scissor Rect { w: 1904, h: 1028 } is not contained in the render target (948, 1028)
    ///
    /// — which is what a tiling window manager rearranging the desktop did to it. There is no unit
    /// test for "resize the window", so this pins the shape of the fix instead: the descriptor
    /// reads `target_size`, and the call site fills it from the acquired backbuffer.
    #[test]
    fn the_descriptor_is_sized_from_the_target_not_the_window() {
        let src = include_str!("egui_ctx.rs");
        let code = src.split("#[cfg(test)]").next().unwrap_or("");
        assert!(
            code.contains("size_in_pixels: target_size"),
            "the screen descriptor must take the target's size"
        );
        assert!(
            !code.contains("size_in_pixels: [window.inner_size()"),
            "the window's size is not the target's size on the frame a resize lands"
        );

        let call_site = include_str!("windowed/event.rs");
        assert!(
            call_site.contains("[output.texture.width(), output.texture.height()]"),
            "the caller must measure the backbuffer it just acquired, not the window"
        );
    }
}

#[cfg(test)]
mod egui_frame_ownership_tests {
    /// Every egui frame the loop starts has to reach something that applies its texture deltas:
    /// [`super::EguiContext::render`] when the frame is painted, `absorb_unpainted_frame` when the
    /// swapchain image could not be acquired and it is not.
    ///
    /// egui hands each delta over exactly **once**, so a dropped `FullOutput` is not a dropped
    /// frame — it is a font atlas that never reaches the GPU and never comes back. Debug builds
    /// said so and took the process with them (`TexturesDelta`'s `Drop` is a `debug_assert!`):
    /// measured, `cargo run -p demo --bin advanced_physics` died about a second after launch with
    /// "Dropped TexturesDelta with 1 unapplied deltas", because the first frame of a freshly
    /// mapped window is a skipped frame. Release builds did not panic and were worse off for it:
    /// the same loss is silent there, and the glyphs paint as blank boxes for the rest of the run.
    ///
    /// There is no unit test for "the surface went outdated" — the panic needs a real swapchain —
    /// so these pin the shape of the fix instead.
    #[test]
    fn the_skipped_frame_still_absorbs_its_egui_output() {
        let src = include_str!("windowed/event.rs");
        let skip = src
            .find("acquire_backbuffer(&mut renderer) else {")
            .expect("the frame loop still skips through acquire_backbuffer");
        let epilogue_end = src[skip..]
            .find("};")
            .expect("the skip epilogue is a block")
            + skip;
        let epilogue = &src[skip..epilogue_end];

        assert!(
            epilogue.contains("absorb_unpainted_frame("),
            "the frame that is not painted still owns this frame's texture deltas — hand them \
             over before returning, or the atlas is gone and debug builds die on the assert"
        );
    }

    #[test]
    fn a_full_output_taken_by_value_is_cleared_before_it_drops() {
        let src = include_str!("egui_ctx.rs");
        let code = src.split("#[cfg(test)]").next().unwrap_or("");

        let takes_by_value = code.matches("mut full_output: egui::FullOutput").count();
        let clears = code.matches("full_output.textures_delta.clear();").count();
        assert_eq!(
            takes_by_value, clears,
            "every function that takes a FullOutput by value ends up dropping it, and dropping an \
             unhandled TexturesDelta is the assert that killed the demos: clear it once its \
             contents have been applied"
        );
        assert!(clears >= 2, "both the painted and the unpainted path clear it");
    }
}
