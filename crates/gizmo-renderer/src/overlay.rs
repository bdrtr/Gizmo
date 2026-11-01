//! The post-tonemap overlay: screen-space sprites, UI backgrounds and screen text, drawn onto the
//! **final** image after bloom, tone mapping, vignette and FXAA.
//!
//! # Why it exists
//!
//! Until 2026-11-01 every screen-space element was drawn into the HDR target with the world, so a
//! HUD was exposed, bloomed, tone-mapped and vignetted as if it were scene radiance. Measured: an
//! opaque `(0, 1, 0)` screen sprite read back as `(0, 219, 0)`, and the `sprite_2d` demo's green
//! energy bar came out mint. A HUD colour is a *pixel* colour, not a radiance, and the only place
//! it can stay one is after the post chain. World-space sprites and labels stay in the HDR pass:
//! they are lit, fogged and occluded with the scene they sit in.
//!
//! # Where it runs
//!
//! Inside [`Renderer::run_post_processing`](crate::Renderer::run_post_processing), after the post
//! chain has written the output. That function is the one every host funnels every output
//! through — the game's window, `gizmo-studio`'s editor target, its Game panel and its
//! separately rendered game view — so hanging the overlay there is what puts the HUD on all of
//! them without a second call site to forget.
//!
//! # Pipelines per format
//!
//! The output's format is whatever the caller's view says — the swapchain's, an editor render
//! target's — so overlay pipelines are built on first use for each format and cached. On a
//! non-sRGB target the shader encodes to sRGB itself (`fs_main_encoded`), because the hardware
//! will not. (The post chain does NOT do the same — `post_process.wgsl` relies on the hardware —
//! so on such a target the scene comes out a gamma step dark while the HUD is right. The engine
//! asks for an sRGB surface whenever one is offered, which is everywhere it has been run.)
//!
//! The format is read from `output.texture()`, so the view must be in its texture's own format
//! rather than a reinterpreting one — the editor's viewport targets render through exactly that
//! view and hand egui a separate one.

use std::collections::HashMap;
use std::sync::Mutex;

/// One shader's overlay pipelines, one per output format, built on demand.
pub(crate) struct OverlayPipelines {
    label: &'static str,
    shader: wgpu::ShaderModule,
    layout: wgpu::PipelineLayout,
    vertex: fn() -> wgpu::VertexBufferLayout<'static>,
    /// Fragment entry point for an sRGB target, and for a linear one that needs encoding.
    entry: (&'static str, &'static str),
    cache: Mutex<HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>>,
}

impl OverlayPipelines {
    pub(crate) fn new(
        label: &'static str,
        shader: wgpu::ShaderModule,
        layout: wgpu::PipelineLayout,
        vertex: fn() -> wgpu::VertexBufferLayout<'static>,
        entry: (&'static str, &'static str),
    ) -> Self {
        Self { label, shader, layout, vertex, entry, cache: Mutex::new(HashMap::new()) }
    }

    /// The pipeline for `format`, building it the first time that format is seen.
    pub(crate) fn get(&self, device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        // A poisoned lock means a panic mid-insert elsewhere; the map is still a valid cache.
        let mut cache = self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        cache.entry(format).or_insert_with(|| self.build(device, format)).clone()
    }

    fn build(&self, device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::RenderPipeline {
        let fragment = if format.is_srgb() { self.entry.0 } else { self.entry.1 };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(self.label),
            layout: Some(&self.layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some((self.vertex)())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            // No depth: the overlay is on top of the picture by definition, and the output has
            // no depth attachment to test against.
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    }
}

impl std::fmt::Debug for OverlayPipelines {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let formats = self.cache.lock().map(|c| c.len()).unwrap_or(0);
        f.debug_struct("OverlayPipelines").field("label", &self.label).field("formats", &formats).finish()
    }
}

/// Draws this frame's overlay onto `output`: UI backgrounds, then screen sprites, then screen
/// text — so an icon sits on its panel and a label on both.
///
/// A no-op when nothing screen-space was queued, so a frame without a HUD costs no pass.
pub(crate) fn record(renderer: &crate::Renderer, encoder: &mut wgpu::CommandEncoder, output: &wgpu::TextureView) {
    let text = renderer.text.as_ref().filter(|t| t.has_overlay());
    let sprites = renderer.sprites.as_ref().filter(|s| s.has_overlay());
    if text.is_none() && sprites.is_none() {
        return;
    }
    let format = output.texture().format();
    let device = &renderer.device;
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Overlay Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: output,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let globals = renderer.scene.view_bind_group();
    if let Some(t) = text {
        t.render_overlay_rects(&mut pass, device, format, globals);
    }
    if let Some(s) = sprites {
        s.render_overlay(&mut pass, device, format);
    }
    if let Some(t) = text {
        t.render_overlay_glyphs(&mut pass, device, format, globals);
    }
}
