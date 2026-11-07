//! The GPU half of text: the atlas' pipelines, the per-frame instance buffer, and the draw.
//!
//! [`super`] turns a string into placed glyphs and rasterised coverage; this turns those into
//! quads and issues one draw call per space. Nothing here decides *what* the text says or where
//! the entity is — a draw loop does that and calls [`TextRenderer::queue`] — which is what keeps
//! the two hosts (the game's `systems::render` and `gizmo-studio`'s pipeline) from each growing
//! their own version of glyph placement.
//!
//! # Two spaces, one pipeline pair
//!
//! Screen text is positioned on the CPU, where the window size is known, and reaches the shader
//! already in clip space. World text carries an anchor and a local offset in world units, and the
//! shader builds a camera-facing basis for it. They are told apart by one sign in the instance, so
//! there is one shader — but they go to different *places*: a world label is drawn into the HDR
//! pass against the scene's depth (hidden by the wall in front of it, exposed with the scene it
//! sits in), and screen text and UI backgrounds are drawn onto the final image after tone mapping
//! by `crate::overlay` — so a HUD keeps the colour it was given. Until 2026-11-01 both went into
//! the HDR target, and an opaque HUD colour came out tone-mapped.
//!
//! # What it does not do
//!
//! There is no batching by font: every glyph is an instance and every instance carries its own
//! colour, so a thousand-glyph frame is one draw call and 64 kB.

use gizmo_math::{Vec2, Vec3, Vec4};

use super::{FontError, FontId, FontLibrary, GlyphAtlas};
use crate::components::{Text, TextSpace};

/// One glyph quad, as the shader reads it. Mirrors `GlyphInstance` in `shaders/text.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GlyphInstance {
    /// Screen: the quad in NDC. World: the quad in world units around `origin`, y up.
    rect: [f32; 4],
    /// The glyph's box in the atlas.
    uv: [f32; 4],
    color: [f32; 4],
    /// World: `xyz` = the anchor, `w` = 1. Screen: `w` = −1.
    origin: [f32; 4],
}

impl GlyphInstance {
    /// The four `vec4`s, at the locations `shaders/text.wgsl` declares. Written out rather than
    /// through `vertex_attr_array!`, whose expansion is a temporary that cannot outlive the
    /// function returning the layout.
    const ATTRS: [wgpu::VertexAttribute; 4] = [
        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x4 },
        wgpu::VertexAttribute { offset: 16, shader_location: 1, format: wgpu::VertexFormat::Float32x4 },
        wgpu::VertexAttribute { offset: 32, shader_location: 2, format: wgpu::VertexFormat::Float32x4 },
        wgpu::VertexAttribute { offset: 48, shader_location: 3, format: wgpu::VertexFormat::Float32x4 },
    ];

    fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRS,
        }
    }
}

/// Fonts, the glyph atlas, and the pipelines that draw them.
pub struct TextRenderer {
    library: FontLibrary,
    atlas: GlyphAtlas,
    /// Depth-tested, for world labels: a label behind a wall is behind the wall.
    pipeline_world: wgpu::RenderPipeline,
    /// Screen text and UI backgrounds, drawn onto the FINAL image after tone mapping — see
    /// [`crate::overlay`]. One pipeline per output format, built on first use.
    overlay: crate::overlay::OverlayPipelines,
    buffer: wgpu::Buffer,
    capacity: u32,
    /// This frame's world-space glyphs, then its screen-space ones. Two lists rather than one with
    /// a flag, because each is a contiguous instance range for its own pipeline.
    world: Vec<GlyphInstance>,
    /// Solid screen-space quads — UI backgrounds. Their own list because they must be painted
    /// UNDER the glyphs, and the buffer's order is the paint order: nothing here depth-tests.
    rects: Vec<GlyphInstance>,
    screen: Vec<GlyphInstance>,
    /// The overlay layer of each entry in `rects` / `screen` — see [`set_layer`](Self::set_layer).
    rect_layers: Vec<i32>,
    screen_layers: Vec<i32>,
    /// The layer the next queued quads go to.
    layer: i32,
    /// How many of each actually reached the buffer, so the draw ranges cannot outrun it.
    uploaded_world: u32,
    uploaded_rects: u32,
    uploaded_screen: u32,
    /// The uploaded overlay, per layer, low to high: `(layer, rect range, glyph range)` in buffer
    /// instance indices.
    overlay_runs: Vec<(i32, std::ops::Range<u32>, std::ops::Range<u32>)>,
}

/// Where each layer's entries sit in `layers` once it is sorted (stably) low to high:
/// `(layer, range)`. Pure, so the grouping the overlay draws by is tested without a device.
fn layer_runs(sorted: &[i32]) -> Vec<(i32, std::ops::Range<usize>)> {
    let mut out: Vec<(i32, std::ops::Range<usize>)> = Vec::new();
    for (i, &l) in sorted.iter().enumerate() {
        match out.last_mut() {
            Some((last, r)) if *last == l => r.end = i + 1,
            _ => out.push((l, i..i + 1)),
        }
    }
    out
}

/// `items` reordered stably by `layers`, and the layers in the same order.
fn sort_by_layer(items: &[GlyphInstance], layers: &[i32]) -> (Vec<GlyphInstance>, Vec<i32>) {
    let mut idx: Vec<usize> = (0..items.len()).collect();
    idx.sort_by_key(|&i| layers.get(i).copied().unwrap_or(0));
    (idx.iter().map(|&i| items[i]).collect(), idx.iter().map(|&i| layers.get(i).copied().unwrap_or(0)).collect())
}

impl std::fmt::Debug for TextRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextRenderer")
            .field("library", &self.library)
            .field("atlas", &self.atlas)
            .field("capacity", &self.capacity)
            .finish()
    }
}

/// The atlas side, in pixels. At 32 px that is roughly a thousand glyphs for one megabyte.
const ATLAS_SIZE: u32 = 1024;

/// How many glyph quads the buffer starts with. It grows; this only decides how often.
const INITIAL_CAPACITY: u32 = 2048;

impl TextRenderer {
    /// Builds the atlas and both pipelines.
    ///
    /// `global_bind_group_layout` is the engine's group 0 (the scene uniforms) — the same one
    /// every other pipeline declares, because world text needs the camera.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let atlas = GlyphAtlas::new(device, queue, ATLAS_SIZE);
        let (pipeline_world, overlay) = Self::build_pipelines(
            device,
            global_bind_group_layout,
            atlas.bind_group_layout(),
            color_format,
            depth_format,
        );

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Text Instance Buffer"),
            size: u64::from(INITIAL_CAPACITY) * std::mem::size_of::<GlyphInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            library: FontLibrary::new(),
            atlas,
            pipeline_world,
            overlay,
            buffer,
            capacity: INITIAL_CAPACITY,
            world: Vec::new(),
            rects: Vec::new(),
            screen: Vec::new(),
            rect_layers: Vec::new(),
            screen_layers: Vec::new(),
            layer: 0,
            overlay_runs: Vec::new(),
            uploaded_world: 0,
            uploaded_rects: 0,
            uploaded_screen: 0,
        }
    }


    /// Compiles the shader and builds the two pipelines. Shared by [`new`](Self::new) and
    /// [`rebuild_pipelines`](Self::rebuild_pipelines) so the two cannot drift — a hot reload that
    /// rebuilt one of them differently is a picture that changes for no reason the shader gives.
    fn build_pipelines(
        device: &wgpu::Device,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        atlas_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, crate::overlay::OverlayPipelines) {
        #[cfg(not(gizmo_forward_only))]
        let shader = crate::pipeline::load_shader_composed(
            device,
            "demo/assets/shaders/text.wgsl",
            include_str!("../shaders/text.wgsl"),
            "Text Shader",
        );
        #[cfg(gizmo_forward_only)]
        let shader = crate::pipeline::load_shader_composed_web(
            device,
            include_str!("../shaders/text.wgsl"),
            "Text Shader",
        );

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Text Pipeline Layout"),
            bind_group_layouts: &[Some(global_bind_group_layout), Some(atlas_layout)],
            immediate_size: 0,
        });

        let make = |label: &str, depth_compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(GlyphInstance::desc())],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    // A glyph quad seen from behind is still a glyph: a world label the camera has
                    // walked past should read mirrored, not vanish.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth_format,
                    // Never: glyph quads overlap each other by construction (a descender reaches
                    // into the next line's box), and a depth-writing overlay would punch holes in
                    // itself.
                    depth_write_enabled: Some(false),
                    depth_compare: Some(depth_compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let world = make("Text Pipeline (world)", wgpu::CompareFunction::LessEqual);
        let overlay = crate::overlay::OverlayPipelines::new(
            "Text Pipeline (overlay)",
            shader.clone(),
            layout.clone(),
            GlyphInstance::desc,
            ("fs_main", "fs_main_encoded"),
        );
        (world, overlay)
    }

    /// Recompiles `text.wgsl` and rebuilds both pipelines, keeping the fonts and the atlas.
    ///
    /// What shader hot-reload calls. Only the *pipelines* are replaced: the loaded faces and every
    /// glyph already rasterised survive, so editing the shader mid-run does not empty the atlas
    /// and re-rasterise the frame's text — which is the difference between a reload that takes a
    /// frame and one that takes a stutter.
    pub fn rebuild_pipelines(
        &mut self,
        device: &wgpu::Device,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) {
        let (world, overlay) = Self::build_pipelines(
            device,
            global_bind_group_layout,
            self.atlas.bind_group_layout(),
            color_format,
            depth_format,
        );
        self.pipeline_world = world;
        self.overlay = overlay;
    }

    /// Loads a face from `ttf`/`otf` bytes.
    ///
    /// # Errors
    ///
    /// See [`FontLibrary::load`].
    pub fn load_font(&mut self, bytes: Vec<u8>) -> Result<FontId, FontError> {
        self.library.load(bytes)
    }

    /// Loads a face from a file.
    ///
    /// # Errors
    ///
    /// See [`FontLibrary::load_file`].
    pub fn load_font_file(&mut self, path: impl AsRef<std::path::Path>) -> Result<FontId, FontError> {
        self.library.load_file(path)
    }

    /// Adds a face to the fallback chain — see [`FontLibrary::add_fallback`].
    pub fn add_font_fallback(&mut self, font: FontId) {
        self.library.add_fallback(font);
    }

    /// The loaded faces, for measuring a string before drawing it.
    #[must_use]
    pub fn library(&self) -> &FontLibrary {
        &self.library
    }

    /// The glyph atlas.
    #[must_use]
    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    /// Drops last frame's quads. Call once per frame before any [`queue`](Self::queue).
    ///
    /// Also forgets what was uploaded: the overlay is drawn from the upload counts by
    /// [`Renderer::run_post_processing`](crate::Renderer::run_post_processing), so a frame that
    /// queues nothing must not repaint last frame's HUD.
    pub fn begin_frame(&mut self) {
        self.world.clear();
        self.rects.clear();
        self.screen.clear();
        self.rect_layers.clear();
        self.screen_layers.clear();
        self.layer = 0;
        self.overlay_runs.clear();
        self.uploaded_world = 0;
        self.uploaded_rects = 0;
        self.uploaded_screen = 0;
    }

    /// The overlay layer the quads queued from now on go to — a UI node's `ZIndex`.
    ///
    /// The overlay paints layer by layer, low to high, and within one layer backgrounds under
    /// glyphs. Without layers every background was under every glyph, so a panel opened over
    /// another showed the lower one's labels through it. Screen sprites are painted between
    /// layer 0's backgrounds and its glyphs, which is where they always were; with every quad on
    /// layer 0 (the default, and what [`begin_frame`](Self::begin_frame) resets to) the frame is
    /// exactly what it was before layers existed.
    pub fn set_layer(&mut self, layer: i32) {
        self.layer = layer;
    }

    /// Whether this frame has anything to draw.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.world.is_empty() && self.rects.is_empty() && self.screen.is_empty()
    }

    /// Adds a solid screen-space quad — a UI background, a highlight, a separator.
    ///
    /// `top_left` and `size` are window pixels with the origin at the top-left, the same
    /// coordinates `gizmo-ui`'s `Node` publishes, so a laid-out box can be handed straight over.
    /// Quads are painted **under** every glyph this frame whatever order they were queued in,
    /// which is the only order a UI can mean.
    ///
    /// It goes through the text pipeline: the atlas holds a fully-opaque patch, so a "solid" quad
    /// is a glyph quad whose coverage happens to be 1. That is why a background costs no pipeline,
    /// no shader and no second buffer.
    pub fn queue_rect(&mut self, top_left: Vec2, size: Vec2, color: Vec4, screen_size: Vec2) {
        self.queue_rect_clipped(top_left, size, color, screen_size, None);
    }

    /// [`queue_rect`](Self::queue_rect), cut to `clip` (`[min, max]` in window pixels) — what a UI
    /// panel that clips its content does to a child's background. Nothing is queued when the two
    /// do not overlap.
    pub fn queue_rect_clipped(&mut self, top_left: Vec2, size: Vec2, color: Vec4, screen_size: Vec2, clip: Option<[Vec2; 2]>) {
        let (top_left, size) = match clip {
            Some([lo, hi]) => {
                let a = top_left.max(lo);
                let b = (top_left + size).min(hi);
                (a, b - a)
            }
            None => (top_left, size),
        };
        if size.x <= 0.0 || size.y <= 0.0 || color.w <= 0.0 {
            return;
        }
        let to_ndc = |p: Vec2| {
            Vec2::new(
                p.x / screen_size.x.max(1.0) * 2.0 - 1.0,
                1.0 - p.y / screen_size.y.max(1.0) * 2.0,
            )
        };
        let a = to_ndc(top_left);
        let b = to_ndc(top_left + size);
        self.rect_layers.push(self.layer);
        self.rects.push(GlyphInstance {
            rect: [a.x, a.y, b.x, b.y],
            uv: self.atlas.solid_uv(),
            color: color.to_array(),
            origin: [0.0, 0.0, 0.0, -1.0],
        });
    }

    /// Adds one `Text` to this frame, rasterising any glyph the atlas has not seen.
    ///
    /// `world_origin` is where world-space text sits and is ignored for screen text;
    /// `screen_size` is the render target in pixels and is ignored for world text. Passing both
    /// rather than two methods keeps the caller's loop one branch shorter, and neither is
    /// expensive to compute.
    ///
    /// `screen_anchor` **overrides** the position the component authored, and `None` means "use
    /// what it says". That is how a UI `Node` positions a label without this crate knowing what a
    /// `Node` is: the host resolves the box, this draws at it.
    ///
    /// A `Text` whose font is not loaded, whose content is empty, or whose glyphs no longer fit in
    /// the atlas contributes nothing. It is not an error: the frame still renders, and
    /// [`GlyphAtlas::is_full`] is where a full atlas is visible.
    pub fn queue(
        &mut self,
        queue: &wgpu::Queue,
        text: &Text,
        world_origin: Vec3,
        screen_anchor: Option<Vec2>,
        screen_size: Vec2,
    ) {
        self.queue_clipped(queue, text, world_origin, screen_anchor, screen_size, None);
    }

    /// [`queue`](Self::queue), with screen-space glyphs cut to `clip` (`[min, max]` in window
    /// pixels): a glyph straddling the edge is trimmed — quad and texture coordinates together, so
    /// what remains is not squashed — and one wholly outside is dropped. World text ignores it.
    /// This is how a UI box that clips its content keeps a long label inside itself.
    #[allow(clippy::too_many_arguments)]
    pub fn queue_clipped(
        &mut self,
        queue: &wgpu::Queue,
        text: &Text,
        world_origin: Vec3,
        screen_anchor: Option<Vec2>,
        screen_size: Vec2,
        clip: Option<[Vec2; 2]>,
    ) {
        let wrap = wrap_width(text, screen_anchor, screen_size);
        let runs = text.style_runs();
        let laid_out = if text.vertical && text.upright {
            self.library.layout_vertical_upright(text.font, text.size_px, &text.content, wrap, &runs)
        } else if text.vertical {
            self.library.layout_vertical(text.font, text.size_px, &text.content, wrap, &runs)
        } else {
            self.library.layout_styled(text.font, text.size_px, &text.content, wrap, &runs)
        };
        let Some(mut layout) = laid_out else {
            return;
        };
        if text.align == crate::components::TextAlign::Start {
            layout.align_start();
        } else {
            layout.align(text.align.factor());
        }
        // Split so the rasteriser is the only part of this that needs a GPU: `place` is pure
        // arithmetic over glyph entries, and it is where the anchor, the y flip and the pixels →
        // NDC conversion live — i.e. where the bugs are. The closure is what keeps the atlas
        // (which needs a queue and a `&mut self`) out of it.
        let before = self.screen.len();
        let Self { atlas, library, world, screen, .. } = self;
        place_clipped(&layout, text, world_origin, screen_anchor, screen_size, clip, world, screen, |g| {
            atlas.glyph(queue, library, g.font, g.glyph, g.px)
        });
        let added = self.screen.len() - before;
        self.screen_layers.extend(std::iter::repeat_n(self.layer, added));
    }

    /// Uploads this frame's quads, growing the buffer if it has to.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let needed = (self.world.len() + self.rects.len() + self.screen.len()) as u32;
        self.uploaded_world = 0;
        self.uploaded_rects = 0;
        self.uploaded_screen = 0;
        if needed == 0 {
            return;
        }
        if needed > self.capacity {
            // Grow by doubling until it fits, so a frame that adds one glyph does not reallocate.
            let mut capacity = self.capacity.max(1);
            while capacity < needed {
                capacity = capacity.saturating_mul(2);
            }
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Text Instance Buffer"),
                size: u64::from(capacity) * std::mem::size_of::<GlyphInstance>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.capacity = capacity;
        }
        // World, then the solid quads, then the glyphs — each overlay half sorted by layer, so a
        // layer's backgrounds and its glyphs are each one contiguous range.
        let (rects, rect_layers) = sort_by_layer(&self.rects, &self.rect_layers);
        let (screen, screen_layers) = sort_by_layer(&self.screen, &self.screen_layers);
        let mut offset = 0u64;
        for chunk in [&self.world, &rects, &screen] {
            queue.write_buffer(&self.buffer, offset, bytemuck::cast_slice(chunk));
            offset += std::mem::size_of_val(chunk.as_slice()) as u64;
        }
        self.uploaded_world = self.world.len() as u32;
        self.uploaded_rects = rects.len() as u32;
        self.uploaded_screen = screen.len() as u32;

        // Per layer: where its backgrounds and its glyphs are in the buffer.
        let rect_base = self.uploaded_world as usize;
        let glyph_base = rect_base + rects.len();
        let rect_runs = layer_runs(&rect_layers);
        let glyph_runs = layer_runs(&screen_layers);
        let mut layers: Vec<i32> = rect_runs.iter().chain(glyph_runs.iter()).map(|(l, _)| *l).collect();
        layers.sort_unstable();
        layers.dedup();
        let find = |runs: &[(i32, std::ops::Range<usize>)], l: i32, base: usize| {
            runs.iter()
                .find(|(x, _)| *x == l)
                .map_or(0..0, |(_, r)| (base + r.start) as u32..(base + r.end) as u32)
        };
        self.overlay_runs = layers
            .into_iter()
            .map(|l| (l, find(&rect_runs, l, rect_base), find(&glyph_runs, l, glyph_base)))
            .collect();
    }

    /// Draws this frame's **world** labels, depth-tested, into the HDR pass.
    ///
    /// Screen text and UI backgrounds are not drawn here: they go onto the final image after tone
    /// mapping, through `crate::overlay`. The pass needs a depth attachment, and
    /// [`upload`](Self::upload) must have run this frame.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, global_bind_group: &'a wgpu::BindGroup) {
        if self.uploaded_world == 0 {
            return;
        }
        pass.set_bind_group(0, global_bind_group, &[]);
        pass.set_bind_group(1, self.atlas.bind_group(), &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.set_pipeline(&self.pipeline_world);
        pass.draw(0..6, 0..self.uploaded_world);
    }

    /// Whether this frame has screen text or UI backgrounds for the overlay.
    pub(crate) fn has_overlay(&self) -> bool {
        self.uploaded_rects + self.uploaded_screen > 0
    }

    /// The overlay's layers, low to high, as uploaded this frame.
    pub(crate) fn overlay_layers(&self) -> Vec<i32> {
        self.overlay_runs.iter().map(|(l, _, _)| *l).collect()
    }

    /// One layer's UI backgrounds.
    pub(crate) fn render_overlay_rects(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        global_bind_group: &wgpu::BindGroup,
        layer: i32,
    ) {
        if let Some((_, r, _)) = self.overlay_runs.iter().find(|(l, _, _)| *l == layer) {
            self.draw_overlay(pass, device, format, global_bind_group, r.clone());
        }
    }

    /// One layer's screen glyphs.
    pub(crate) fn render_overlay_glyphs(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        global_bind_group: &wgpu::BindGroup,
        layer: i32,
    ) {
        if let Some((_, _, g)) = self.overlay_runs.iter().find(|(l, _, _)| *l == layer) {
            self.draw_overlay(pass, device, format, global_bind_group, g.clone());
        }
    }

    fn draw_overlay(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        global_bind_group: &wgpu::BindGroup,
        range: std::ops::Range<u32>,
    ) {
        if range.is_empty() {
            return;
        }
        pass.set_pipeline(&self.overlay.get(device, format));
        pass.set_bind_group(0, global_bind_group, &[]);
        pass.set_bind_group(1, self.atlas.bind_group(), &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        pass.draw(0..6, range);
    }

    /// This frame's quad counts, `(world glyphs, solid quads, screen glyphs)`.
    ///
    /// Exposed because "the text did not appear" has two very different causes: nothing was
    /// queued, or something was queued and drawn off-screen. A count separates them without a
    /// screenshot.
    #[must_use]
    pub fn queued(&self) -> (usize, usize, usize) {
        (self.world.len(), self.rects.len(), self.screen.len())
    }
}

/// The width `text` wraps at: its own [`max_width`](Text::max_width), but never wider than the
/// window leaves it. A screen or pinned text placed by its own position (no `screen_anchor` from a
/// host) is cut to the room between its anchor point and the window's edges, by how its box hangs
/// off that point: a top-left box has the room to the right edge, a centred one twice the room to
/// the nearer edge, a right-anchored one the room to the left edge.
///
/// Without this a HUD line ran off the side of a narrow window. `pinbol`'s
/// `A/D: paletler   Space: at   R: baştan  (top kanalda)` at 22 px on an upright phone (350 frame
/// pixels across) lost its last third past the right edge, and nothing on the page said so.
///
/// A host that places the text itself — a UI box, a tooltip — passes `screen_anchor` and has
/// decided the width already, so it gets `max_width` unchanged; so does a world label. A point
/// outside the window leaves no room to measure from, and the text keeps its own width there too.
pub(crate) fn wrap_width(text: &Text, screen_anchor: Option<Vec2>, screen_size: Vec2) -> Option<f32> {
    if screen_anchor.is_some() {
        return text.max_width;
    }
    let Some(point) = text.space.screen_point(screen_size) else {
        return text.max_width;
    };
    let hang = text.anchor.factors().x;
    let right = if hang < 1.0 { (screen_size.x - point.x) / (1.0 - hang) } else { f32::INFINITY };
    let left = if hang > 0.0 { point.x / hang } else { f32::INFINITY };
    let room = right.min(left);
    if !(room.is_finite() && room > 0.0) {
        return text.max_width;
    }
    Some(text.max_width.map_or(room, |w| w.min(room)))
}

/// Turns a laid-out string into quads, appending to `world` or `screen` depending on the space.
///
/// `entry_of` answers where a glyph is in the atlas; `None` means it has no outline or did not
/// fit, and either way it contributes nothing. Pulled out of [`TextRenderer::queue`] so the
/// placement can be checked without an adapter — it is the half that decides where a glyph lands,
/// and a sign error here is a picture that is upside down rather than a compile error.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn place(
    layout: &super::TextLayout,
    text: &Text,
    world_origin: Vec3,
    screen_anchor: Option<Vec2>,
    screen_size: Vec2,
    world: &mut Vec<GlyphInstance>,
    screen: &mut Vec<GlyphInstance>,
    entry_of: impl FnMut(&super::PositionedGlyph) -> Option<super::GlyphEntry>,
) {
    place_clipped(layout, text, world_origin, screen_anchor, screen_size, None, world, screen, entry_of);
}

/// A screen quad `[min, max]` (window pixels) with texture rectangle `uv`, cut to `clip`: the part
/// inside, with `uv` cut by the same fractions so the glyph is trimmed rather than squashed.
/// `None` when nothing of it is inside.
fn clip_quad(min: Vec2, max: Vec2, uv: [f32; 4], clip: [Vec2; 2]) -> Option<(Vec2, Vec2, [f32; 4])> {
    let a = min.max(clip[0]);
    let b = max.min(clip[1]);
    if b.x <= a.x || b.y <= a.y {
        return None;
    }
    let size = (max - min).max(Vec2::splat(f32::MIN_POSITIVE));
    let (f0, f1) = ((a - min) / size, (b - min) / size);
    let (du, dv) = (uv[2] - uv[0], uv[3] - uv[1]);
    Some((a, b, [uv[0] + du * f0.x, uv[1] + dv * f0.y, uv[0] + du * f1.x, uv[1] + dv * f1.y]))
}

#[allow(clippy::too_many_arguments)]
fn place_clipped(
    layout: &super::TextLayout,
    text: &Text,
    world_origin: Vec3,
    screen_anchor: Option<Vec2>,
    screen_size: Vec2,
    clip: Option<[Vec2; 2]>,
    world: &mut Vec<GlyphInstance>,
    screen: &mut Vec<GlyphInstance>,
    mut entry_of: impl FnMut(&super::PositionedGlyph) -> Option<super::GlyphEntry>,
) {
    if layout.glyphs.is_empty() {
        return;
    }
    // Where the text's box goes relative to the anchor point, in text-local pixels with y growing
    // down. Every glyph below is placed against this one offset.
    let box_offset = -text.anchor.factors() * Vec2::new(layout.width, layout.height);

    for g in &layout.glyphs {
        let Some(entry) = entry_of(g) else { continue };
        let color = text.color_at(g.cluster).to_array();
        // The glyph box's top-left in text-local pixels, y down.
        let [w, h] = entry.size;
        let [ox, oy] = entry.offset;
        // A quarter turn clockwise about the pen sends (x, y) to (-y, x), so the box swaps its sides and its
        // texture runs down the screen. The instance says so by `|origin.w| == 2`.
        let min = if g.sideways {
            box_offset + Vec2::new(g.x - (oy + h), g.y + ox)
        } else {
            box_offset + Vec2::new(g.x + ox, g.y + oy)
        };
        let max = min + if g.sideways { Vec2::new(h, w) } else { Vec2::new(w, h) };
        // The atlas box as the shader reads it for a turned glyph: u from top to bottom, v from left to right.
        let [u0, v0, u1, v1] = entry.uv;
        let turned = [u0, v1, u1, v0];
        let turn = if g.sideways { 2.0 } else { 1.0 };

        match text.space {
            TextSpace::Screen { .. } | TextSpace::Pinned { .. } => {
                // The host's answer wins; the component's own is the fallback.
                let position = screen_anchor
                    .or_else(|| text.space.screen_point(screen_size))
                    .unwrap_or_default();
                // Window pixels (y down, origin top-left) to NDC (y up, origin centre).
                let to_ndc = |p: Vec2| {
                    Vec2::new(
                        p.x / screen_size.x.max(1.0) * 2.0 - 1.0,
                        1.0 - p.y / screen_size.y.max(1.0) * 2.0,
                    )
                };
                let (lo, hi, uv) = match clip {
                    Some(c) if g.sideways => match clip_quad(position + min, position + max, [0.0, 0.0, 1.0, 1.0], c) {
                        // The fractions of the box that are left, along the screen's axes.
                        Some((lo, hi, [fx0, fy0, fx1, fy1])) => {
                            let along = |from: f32, to: f32, t: f32| from + (to - from) * t;
                            let uv = [
                                along(turned[0], turned[2], fy0),
                                along(turned[1], turned[3], fx0),
                                along(turned[0], turned[2], fy1),
                                along(turned[1], turned[3], fx1),
                            ];
                            (lo, hi, uv)
                        }
                        None => continue,
                    },
                    Some(c) => match clip_quad(position + min, position + max, entry.uv, c) {
                        Some(q) => q,
                        None => continue,
                    },
                    None if g.sideways => (position + min, position + max, turned),
                    None => (position + min, position + max, entry.uv),
                };
                let a = to_ndc(lo);
                let b = to_ndc(hi);
                screen.push(GlyphInstance {
                    rect: [a.x, a.y, b.x, b.y],
                    uv,
                    color,
                    origin: [0.0, 0.0, 0.0, -turn],
                });
            }
            TextSpace::World { px_per_unit } => {
                let scale = 1.0 / px_per_unit.max(f32::MIN_POSITIVE);
                // The one place the y flip happens: text-local y grows down and the world's up
                // vector grows up, so the box's top becomes the larger world offset.
                world.push(GlyphInstance {
                    rect: [min.x * scale, -min.y * scale, max.x * scale, -max.y * scale],
                    uv: if g.sideways { turned } else { entry.uv },
                    color,
                    origin: [world_origin.x, world_origin.y, world_origin.z, turn],
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{TextAnchor, TextSpace};
    use crate::text::{FontLibrary, GlyphEntry, PositionedGlyph, TextLayout};

    /// A 10×10 glyph sitting exactly on the pen and the baseline, so every number below is the
    /// placement and nothing else. A real entry's offset is a bearing; zero here makes the
    /// arithmetic readable, and [`an_offset_moves_the_box_and_nothing_else`] puts one back.
    fn entry(offset: [f32; 2]) -> GlyphEntry {
        GlyphEntry { uv: [0.0, 0.0, 0.1, 0.1], size: [10.0, 10.0], offset }
    }

    /// One glyph on the baseline at pen 0, in a 10-wide, 20-tall box.
    /// A part of a rich text says its look; a text of colours alone asks for no style runs, so it costs
    /// no allocation and takes the cheap path.
    #[test]
    fn rich_parts_carry_their_face_and_size() {
        use crate::components::Span;
        let red = Vec4::new(1.0, 0.0, 0.0, 1.0);
        let colours = Text::screen("", FontId(0), 16.0, Vec2::ZERO).rich([("a", red), ("b", Vec4::ONE)]);
        assert_eq!(colours.content, "ab");
        assert!(colours.style_runs().is_empty());
        assert_eq!(colours.color_at(0), red);

        let t = Text::screen("", FontId(0), 16.0, Vec2::ZERO)
            .rich([Span::new("Can: "), Span::new("5").color(red).size(32.0), Span::new("!").font(FontId(1))]);
        assert_eq!(t.content, "Can: 5!");
        let runs = t.style_runs();
        assert_eq!(runs.len(), 3);
        assert_eq!((runs[0].start, runs[0].px, runs[0].font), (0, 16.0, FontId(0)), "the text's own");
        assert_eq!((runs[1].start, runs[1].px, runs[1].font), (5, 32.0, FontId(0)));
        assert_eq!((runs[2].start, runs[2].px, runs[2].font), (6, 16.0, FontId(1)));
        assert_eq!(t.color_at(5), red);
    }

    fn one_glyph() -> TextLayout {
        TextLayout {
            glyphs: vec![PositionedGlyph { font: FontId(0), glyph: 1, x: 0.0, y: 0.0, line: 0, px: 16.0, cluster: 0 , sideways: false }],
            width: 10.0,
            height: 20.0,
            lines: 1,
            line_widths: vec![10.0],
            line_rtl: vec![false],
        }
    }

    fn text(space: TextSpace, anchor: TextAnchor) -> Text {
        let mut t = Text::screen("A", crate::text::FontId(0), 10.0, Vec2::ZERO);
        t.space = space;
        t.anchor = anchor;
        t
    }

    fn quads(t: &Text, layout: &TextLayout, e: GlyphEntry) -> (Vec<GlyphInstance>, Vec<GlyphInstance>) {
        let (mut w, mut s) = (Vec::new(), Vec::new());
        place(layout, t, Vec3::new(1.0, 2.0, 3.0), None, Vec2::new(100.0, 200.0), &mut w, &mut s, |_| Some(e));
        (w, s)
    }

    /// Screen text lands in NDC where the pixel arithmetic says, and the y axis flips exactly once.
    ///
    /// The flip is the assertion that matters. Window pixels grow down and NDC grows up, so a
    /// missing negation puts the text below the anchor instead of above it — which looks like an
    /// anchor bug and is not one.
    /// **A coloured span reaches the glyphs it covers,** in the right colour, with the text's own colour
    /// on top so a fade still fades all of it.
    #[test]
    fn each_glyph_takes_the_colour_of_its_span() {
        let (red, green) = (Vec4::new(1.0, 0.0, 0.0, 1.0), Vec4::new(0.0, 1.0, 0.0, 1.0));
        let mut t = Text::screen("", FontId(0), 16.0, Vec2::ZERO).rich([("ab", red), ("c", green)]);
        t.color = Vec4::new(1.0, 1.0, 1.0, 0.5);
        assert_eq!(t.content, "abc");
        let glyph = |cluster: u32| PositionedGlyph { font: FontId(0), glyph: 1, x: 0.0, y: 0.0, line: 0, px: 16.0, cluster, sideways: false };
        let layout = TextLayout {
            glyphs: vec![glyph(0), glyph(1), glyph(2)],
            width: 30.0,
            height: 20.0,
            lines: 1,
            line_widths: vec![30.0],
            line_rtl: vec![false],
        };
        let (mut w, mut s) = (Vec::new(), Vec::new());
        place(&layout, &t, Vec3::ZERO, None, Vec2::new(100.0, 100.0), &mut w, &mut s, |_| Some(entry([0.0, 0.0])));
        let colours: Vec<[f32; 4]> = s.iter().map(|g| g.color).collect();
        assert_eq!(colours, [[1.0, 0.0, 0.0, 0.5], [1.0, 0.0, 0.0, 0.5], [0.0, 1.0, 0.0, 0.5]]);

        // Plain text has no spans and takes the text's colour.
        let plain = Text::screen("abc", FontId(0), 16.0, Vec2::ZERO).with_color(green);
        assert_eq!(plain.color_at(2), green);
    }

    #[test]
    fn screen_text_converts_pixels_to_ndc_with_y_flipped() {
        let t = text(TextSpace::Screen { position: Vec2::new(50.0, 100.0) }, TextAnchor::TopLeft);
        let (world, screen) = quads(&t, &one_glyph(), entry([0.0, 0.0]));
        assert!(world.is_empty(), "screen text must not reach the world list");
        assert_eq!(screen.len(), 1);
        let r = screen[0].rect;
        // x: 50/100 of the width → NDC 0; 60/100 → 0.2.
        assert!((r[0] - 0.0).abs() < 1e-5, "left {} should be 0", r[0]);
        assert!((r[2] - 0.2).abs() < 1e-5, "right {} should be 0.2", r[2]);
        // y: 100/200 → NDC 0, and 110/200 → −0.1. DOWN in pixels is DOWN in NDC, i.e. smaller.
        assert!((r[1] - 0.0).abs() < 1e-5, "top {} should be 0", r[1]);
        assert!((r[3] + 0.1).abs() < 1e-5, "bottom {} should be −0.1", r[3]);
        assert!(r[3] < r[1], "the box's bottom must be below its top in NDC");
        assert_eq!(screen[0].origin[3], -1.0, "the screen marker is a negative w");
    }

    /// World text lands in world units around the origin, with y up.
    #[test]
    fn world_text_is_local_units_around_the_origin_with_y_up() {
        let t = text(TextSpace::World { px_per_unit: 10.0 }, TextAnchor::TopLeft);
        let (world, screen) = quads(&t, &one_glyph(), entry([0.0, 0.0]));
        assert!(screen.is_empty(), "world text must not reach the screen list");
        assert_eq!(world.len(), 1);
        let r = world[0].rect;
        // 10 px at 10 px/unit is one unit wide, and the box hangs BELOW the baseline going down…
        assert!((r[0] - 0.0).abs() < 1e-5);
        assert!((r[2] - 1.0).abs() < 1e-5, "one unit wide, got {}", r[2]);
        // …which in world space means the second y is the smaller one.
        assert!(r[3] < r[1], "world y must grow up: {r:?}");
        assert!((r[1] - 0.0).abs() < 1e-5);
        assert!((r[3] + 1.0).abs() < 1e-5, "one unit tall, got {}", r[3]);
        assert_eq!(&world[0].origin[..3], &[1.0, 2.0, 3.0], "the anchor is the entity's position");
        assert_eq!(world[0].origin[3], 1.0, "the world marker is a non-negative w");
    }

    /// `px_per_unit` scales the quad and nothing else.
    #[test]
    fn px_per_unit_scales_the_world_quad() {
        let ten = text(TextSpace::World { px_per_unit: 10.0 }, TextAnchor::TopLeft);
        let twenty = text(TextSpace::World { px_per_unit: 20.0 }, TextAnchor::TopLeft);
        let (a, _) = quads(&ten, &one_glyph(), entry([0.0, 0.0]));
        let (b, _) = quads(&twenty, &one_glyph(), entry([0.0, 0.0]));
        assert!(
            (a[0].rect[2] - 2.0 * b[0].rect[2]).abs() < 1e-5,
            "doubling px_per_unit halves the quad: {} vs {}",
            a[0].rect[2],
            b[0].rect[2]
        );
    }

    /// Each anchor moves the box by its own fraction of the box size, and only that.
    ///
    /// The box is 10×20, so centring must move it 5 left and 10 up — different numbers on the two
    /// axes, which is what catches a factor applied to the wrong one.
    #[test]
    fn the_anchor_shifts_the_box_by_its_fraction_of_the_box_size() {
        let at = |anchor| {
            let t = text(TextSpace::Screen { position: Vec2::new(50.0, 100.0) }, anchor);
            let (_, s) = quads(&t, &one_glyph(), entry([0.0, 0.0]));
            // Back to pixels, so the assertion reads in the units the anchor is specified in.
            Vec2::new((s[0].rect[0] + 1.0) * 50.0, (1.0 - s[0].rect[1]) * 100.0)
        };
        let top_left = at(TextAnchor::TopLeft);
        assert!((top_left - Vec2::new(50.0, 100.0)).length() < 1e-3, "{top_left:?}");
        let centre = at(TextAnchor::Center);
        assert!(
            (centre - Vec2::new(45.0, 90.0)).length() < 1e-3,
            "centring a 10×20 box moves it 5 left and 10 up, got {centre:?}"
        );
        let bottom_right = at(TextAnchor::BottomRight);
        assert!(
            (bottom_right - Vec2::new(40.0, 80.0)).length() < 1e-3,
            "{bottom_right:?}"
        );
    }

    /// A pinned text follows the window: centred on a 100×200 target and on a 300×400 one, it
    /// sits at the middle of each, and a corner pin plus an offset is that corner plus the
    /// offset.
    ///
    /// The second window size is the assertion — a `Pinned` that read an absolute position would
    /// pass at one size and fail at the other.
    #[test]
    fn a_window_text_wraps_at_the_window_edge() {
        let screen = Vec2::new(350.0, 657.0);
        let f = FontId(0);
        // Top-left, 16 px in: everything to the right edge.
        let hud = Text::pinned("x", f, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0));
        assert_eq!(wrap_width(&hud, None, screen), Some(334.0));
        // Its own narrower width wins; a wider one is cut to the window.
        assert_eq!(wrap_width(&hud.clone().with_max_width(200.0), None, screen), Some(200.0));
        assert_eq!(wrap_width(&hud.clone().with_max_width(900.0), None, screen), Some(334.0));
        // Centred at 40 % across: twice the room to the nearer (left) edge.
        let title = Text::pinned_at("x", f, 44.0, Vec2::new(0.4, 0.3), Vec2::ZERO).with_anchor(TextAnchor::Center);
        assert_eq!(wrap_width(&title, None, screen), Some(280.0));
        // Hung from the top-right corner 16 px in: the room to the left edge.
        let score = Text::pinned("x", f, 22.0, TextAnchor::TopRight, Vec2::new(-16.0, 16.0));
        assert_eq!(wrap_width(&score, None, screen), Some(334.0));
        // An absolute screen position measures the same way.
        assert_eq!(wrap_width(&Text::screen("x", f, 22.0, Vec2::new(50.0, 0.0)), None, screen), Some(300.0));
    }

    #[test]
    fn a_placed_text_and_a_world_label_keep_their_own_width() {
        let screen = Vec2::new(350.0, 657.0);
        let f = FontId(0);
        let hud = Text::pinned("x", f, 22.0, TextAnchor::TopLeft, Vec2::new(16.0, 16.0));
        // A host that places it (a UI box, a tooltip) has chosen the width already.
        assert_eq!(wrap_width(&hud, Some(Vec2::new(10.0, 10.0)), screen), None);
        assert_eq!(wrap_width(&Text::world("x", f, 22.0, 64.0), None, screen), None);
        // A point off the window leaves nothing to measure from.
        let off = Text::screen("x", f, 22.0, Vec2::new(400.0, 0.0));
        assert_eq!(wrap_width(&off, None, screen), None);
    }

    #[test]
    fn a_pinned_text_follows_the_window_size() {
        let centre_on = |anchor, offset, size: Vec2| {
            let mut t = text(TextSpace::Pinned { at: TextAnchor::factors(anchor), offset }, anchor);
            t.anchor = anchor;
            let (mut w, mut s) = (Vec::new(), Vec::new());
            place(&one_glyph(), &t, Vec3::ZERO, None, size, &mut w, &mut s, |_| Some(entry([0.0, 0.0])));
            assert!(w.is_empty(), "a pinned text is screen text");
            // The glyph's centre, back in window pixels.
            let r = s[0].rect;
            Vec2::new(((r[0] + r[2]) * 0.5 + 1.0) * 0.5 * size.x, (1.0 - (r[1] + r[3]) * 0.5) * 0.5 * size.y)
        };
        // The one glyph is 10×10 at the top of a 10×20 box; centring the box puts its centre
        // 5 px above the window's centre.
        for size in [Vec2::new(100.0, 200.0), Vec2::new(300.0, 400.0)] {
            let c = centre_on(TextAnchor::Center, Vec2::ZERO, size);
            let want = size * 0.5 + Vec2::new(0.0, -5.0);
            assert!((c - want).length() < 1e-3, "centred on {size:?}: {c:?}, want {want:?}");
            let corner = centre_on(TextAnchor::TopRight, Vec2::new(-16.0, 16.0), size);
            let want = Vec2::new(size.x - 16.0 - 5.0, 16.0 + 5.0);
            assert!((corner - want).length() < 1e-3, "top-right on {size:?}: {corner:?}, want {want:?}");
        }
    }

    /// A point that is not one of the nine anchors: 30 % down, a quarter across, at two window
    /// sizes — the position `kule_yikici`'s banner and `text.rs`'s labels rewrote by hand every
    /// frame.
    #[test]
    fn a_text_pinned_at_a_fraction_follows_the_window_size() {
        for size in [Vec2::new(100.0, 200.0), Vec2::new(300.0, 400.0)] {
            let t = text(TextSpace::Pinned { at: Vec2::new(0.25, 0.3), offset: Vec2::new(2.0, -4.0) }, TextAnchor::TopLeft);
            let (mut w, mut s) = (Vec::new(), Vec::new());
            place(&one_glyph(), &t, Vec3::ZERO, None, size, &mut w, &mut s, |_| Some(entry([0.0, 0.0])));
            // The glyph's top-left corner, back in window pixels.
            let r = s[0].rect;
            let corner = Vec2::new((r[0] + 1.0) * 0.5 * size.x, (1.0 - r[1]) * 0.5 * size.y);
            let want = size * Vec2::new(0.25, 0.3) + Vec2::new(2.0, -4.0);
            assert!((corner - want).length() < 1e-3, "on {size:?}: {corner:?}, want {want:?}");
        }
    }

    /// Sorting by layer is stable (queue order kept within a layer), and the runs cover each
    /// layer's entries exactly once, low to high.
    #[test]
    fn layers_sort_stably_and_group_into_runs() {
        let quad = |x: f32| GlyphInstance { rect: [x, 0.0, 0.0, 0.0], uv: [0.0; 4], color: [0.0; 4], origin: [0.0; 4] };
        let items = [quad(0.0), quad(1.0), quad(2.0), quad(3.0), quad(4.0)];
        let (sorted, layers) = sort_by_layer(&items, &[1, 0, 1, -2, 0]);
        assert_eq!(layers, vec![-2, 0, 0, 1, 1]);
        assert_eq!(sorted.iter().map(|q| q.rect[0]).collect::<Vec<_>>(), vec![3.0, 1.0, 4.0, 0.0, 2.0], "not stable");
        assert_eq!(layer_runs(&layers), vec![(-2, 0..1), (0, 1..3), (1, 3..5)]);
        assert!(layer_runs(&[]).is_empty());
    }

    /// A glyph straddling the clip edge keeps the part inside, with its texture cut by the same
    /// fraction; one wholly outside is gone; one wholly inside is untouched.
    /// A glyph on its side: the box swaps its sides about the pen, the instance says it is turned, and the
    /// texture runs the other way. Clipped, the texture goes with the part of the box that is left.
    #[test]
    fn a_sideways_glyph_turns_its_box_and_its_texture() {
        let layout = TextLayout {
            glyphs: vec![PositionedGlyph { font: FontId(0), glyph: 1, x: 100.0, y: 200.0, line: 0, px: 16.0, cluster: 0, sideways: true }],
            width: 0.0,
            height: 0.0,
            lines: 1,
            line_widths: vec![0.0],
            line_rtl: vec![false],
        };
        // 4 wide and 10 tall, with its top 10 above the baseline and starting 1 right of the pen. Pixels
        // here equal NDC scale, so the box can be read off the instance: a window of 2 by 2 would flip y.
        let entry = GlyphEntry { uv: [0.1, 0.2, 0.3, 0.4], size: [4.0, 10.0], offset: [1.0, -10.0] };
        let text = Text::screen("x", FontId(0), 16.0, Vec2::ZERO);
        let size = Vec2::new(1000.0, 1000.0);
        let run = |clip: Option<[Vec2; 2]>| {
            let (mut world, mut screen) = (Vec::new(), Vec::new());
            place_clipped(&layout, &text, Vec3::ZERO, Some(Vec2::ZERO), size, clip, &mut world, &mut screen, |_| Some(entry));
            screen
        };
        let whole = run(None);
        let px = |ndc: f32| (ndc + 1.0) * 500.0;
        let py = |ndc: f32| (1.0 - ndc) * 500.0;
        let i = whole[0];
        // (x, y) goes to (-y, x): x from 100 + 0 to 100 + 10, y from 200 + 1 to 200 + 5.
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert!(close(px(i.rect[0]), 100.0) && close(px(i.rect[2]), 110.0), "{:?}", i.rect);
        assert!(close(py(i.rect[1]), 201.0) && close(py(i.rect[3]), 205.0), "{:?}", i.rect);
        assert_eq!(i.origin[3], -2.0, "the shader is told");
        assert_eq!(i.uv, [0.1, 0.4, 0.3, 0.2], "u down the screen, v from the glyph's foot at the left");

        let right_half = run(Some([Vec2::new(105.0, 0.0), Vec2::new(500.0, 500.0)]));
        let c = right_half[0];
        assert!(close(px(c.rect[0]), 105.0), "{:?}", c.rect);
        assert!(close(c.uv[0], 0.1) && close(c.uv[2], 0.3), "u is untouched: {:?}", c.uv);
        assert!(close(c.uv[1], 0.3) && close(c.uv[3], 0.2), "v starts halfway: {:?}", c.uv);
    }

    #[test]
    fn a_clip_trims_a_glyph_and_its_texture_together() {
        let uv = [0.2, 0.4, 0.6, 0.8];
        // A 10×10 glyph at (0,0); the clip ends at x = 5: the left half, the left half of the uv.
        let (a, b, cut) = clip_quad(Vec2::ZERO, Vec2::splat(10.0), uv, [Vec2::splat(-100.0), Vec2::new(5.0, 100.0)]).unwrap();
        assert_eq!((a, b), (Vec2::ZERO, Vec2::new(5.0, 10.0)));
        assert!((cut[0] - 0.2).abs() < 1e-6 && (cut[2] - 0.4).abs() < 1e-6, "u halved: {cut:?}");
        assert!((cut[1] - 0.4).abs() < 1e-6 && (cut[3] - 0.8).abs() < 1e-6, "v untouched: {cut:?}");
        assert!(clip_quad(Vec2::ZERO, Vec2::splat(10.0), uv, [Vec2::splat(20.0), Vec2::splat(30.0)]).is_none());
        assert_eq!(clip_quad(Vec2::ZERO, Vec2::splat(10.0), uv, [Vec2::splat(-1.0), Vec2::splat(11.0)]).unwrap().2, uv);
    }

    /// Clipping a three-glyph string to the first glyph's box keeps one glyph.
    #[test]
    fn a_clipped_text_keeps_only_what_is_inside() {
        let layout = TextLayout {
            glyphs: vec![
                PositionedGlyph { font: FontId(0), glyph: 1, x: 0.0, y: 0.0, line: 0, px: 16.0, cluster: 0 , sideways: false },
                PositionedGlyph { font: FontId(0), glyph: 2, x: 10.0, y: 0.0, line: 0, px: 16.0, cluster: 0 , sideways: false },
                PositionedGlyph { font: FontId(0), glyph: 3, x: 20.0, y: 0.0, line: 0, px: 16.0, cluster: 0 , sideways: false },
            ],
            width: 30.0,
            height: 20.0,
            lines: 1,
            line_widths: vec![30.0],
            line_rtl: vec![false],
        };
        let t = text(TextSpace::Screen { position: Vec2::ZERO }, TextAnchor::TopLeft);
        let (mut w, mut s) = (Vec::new(), Vec::new());
        let clip = Some([Vec2::ZERO, Vec2::new(10.0, 50.0)]);
        place_clipped(&layout, &t, Vec3::ZERO, None, Vec2::new(100.0, 100.0), clip, &mut w, &mut s, |_| Some(entry([0.0, 0.0])));
        assert_eq!(s.len(), 1, "only the first glyph is inside the box");
    }

    /// A pinned text under a UI box takes the box's answer, like a `Screen` one.
    #[test]
    fn a_supplied_anchor_wins_over_a_pin_too() {
        let t = text(TextSpace::Pinned { at: Vec2::ZERO, offset: Vec2::ZERO }, TextAnchor::TopLeft);
        let (mut w, mut s) = (Vec::new(), Vec::new());
        place(&one_glyph(), &t, Vec3::ZERO, Some(Vec2::new(50.0, 50.0)), Vec2::new(100.0, 100.0), &mut w, &mut s, |_| {
            Some(entry([0.0, 0.0]))
        });
        assert!((s[0].rect[0] - 0.0).abs() < 1e-5, "the host's 50 px is NDC 0, got {}", s[0].rect[0]);
    }

    /// The glyph's own bearing moves the box and leaves its size alone.
    #[test]
    fn an_offset_moves_the_box_and_nothing_else() {
        let t = text(TextSpace::Screen { position: Vec2::ZERO }, TextAnchor::TopLeft);
        let (_, plain) = quads(&t, &one_glyph(), entry([0.0, 0.0]));
        let (_, borne) = quads(&t, &one_glyph(), entry([3.0, -7.0]));
        let width = |i: &GlyphInstance| i.rect[2] - i.rect[0];
        let height = |i: &GlyphInstance| i.rect[1] - i.rect[3];
        assert!((width(&plain[0]) - width(&borne[0])).abs() < 1e-6, "the bearing changed the size");
        assert!((height(&plain[0]) - height(&borne[0])).abs() < 1e-6);
        assert!(borne[0].rect[0] > plain[0].rect[0], "a positive x bearing moves it right");
        assert!(borne[0].rect[1] > plain[0].rect[1], "a negative y bearing moves it UP the screen");
    }

    /// A glyph the atlas cannot supply is skipped, and the rest of the string still draws.
    #[test]
    fn a_missing_atlas_entry_drops_that_glyph_and_no_other() {
        let layout = TextLayout {
            glyphs: vec![
                PositionedGlyph { font: FontId(0), glyph: 1, x: 0.0, y: 0.0, line: 0, px: 16.0, cluster: 0 , sideways: false },
                PositionedGlyph { font: FontId(0), glyph: 2, x: 10.0, y: 0.0, line: 0, px: 16.0, cluster: 0 , sideways: false },
                PositionedGlyph { font: FontId(0), glyph: 3, x: 20.0, y: 0.0, line: 0, px: 16.0, cluster: 0 , sideways: false },
            ],
            width: 30.0,
            height: 20.0,
            lines: 1,
            line_widths: vec![30.0],
            line_rtl: vec![false],
        };
        let t = text(TextSpace::Screen { position: Vec2::ZERO }, TextAnchor::TopLeft);
        let (mut w, mut s) = (Vec::new(), Vec::new());
        place(&layout, &t, Vec3::ZERO, None, Vec2::new(100.0, 100.0), &mut w, &mut s, |g| {
            (g.glyph != 2).then(|| entry([0.0, 0.0]))
        });
        assert_eq!(s.len(), 2, "the missing glyph took its neighbours with it");
    }

    /// An unknown font produces nothing rather than a panic, and does not disturb the frame.
    #[test]
    fn an_empty_layout_queues_nothing() {
        let t = text(TextSpace::Screen { position: Vec2::ZERO }, TextAnchor::TopLeft);
        let (mut w, mut s) = (Vec::new(), Vec::new());
        place(&TextLayout::default(), &t, Vec3::ZERO, None, Vec2::new(100.0, 100.0), &mut w, &mut s, |_| {
            unreachable!("an empty layout must not ask the atlas for anything")
        });
        assert!(w.is_empty() && s.is_empty());
    }

    /// A host-supplied anchor replaces the component's own, and changes nothing else.
    ///
    /// This is the whole UI bridge in one assertion: `gizmo-ui` publishes a box, the facade hands
    /// its corner over, and the text lands there rather than where the component was authored. The
    /// second half matters as much — an override that also moved the glyphs relative to each other
    /// would be a layout bug wearing a placement bug's clothes.
    #[test]
    fn a_supplied_anchor_replaces_the_authored_one_and_moves_nothing_else() {
        let t = text(TextSpace::Screen { position: Vec2::new(10.0, 10.0) }, TextAnchor::TopLeft);
        let (mut w1, mut s1) = (Vec::new(), Vec::new());
        place(&one_glyph(), &t, Vec3::ZERO, None, Vec2::new(100.0, 100.0), &mut w1, &mut s1, |_| {
            Some(entry([0.0, 0.0]))
        });
        let (mut w2, mut s2) = (Vec::new(), Vec::new());
        place(
            &one_glyph(),
            &t,
            Vec3::ZERO,
            Some(Vec2::new(60.0, 10.0)),
            Vec2::new(100.0, 100.0),
            &mut w2,
            &mut s2,
            |_| Some(entry([0.0, 0.0])),
        );
        assert!(w1.is_empty() && w2.is_empty());
        // 50 px to the right on a 100 px-wide target is one NDC unit.
        assert!(
            (s2[0].rect[0] - s1[0].rect[0] - 1.0).abs() < 1e-5,
            "the override moved the text by {} NDC, expected 1.0",
            s2[0].rect[0] - s1[0].rect[0]
        );
        assert!((s2[0].rect[1] - s1[0].rect[1]).abs() < 1e-6, "it moved vertically too");
        let width = |i: &GlyphInstance| i.rect[2] - i.rect[0];
        assert!((width(&s1[0]) - width(&s2[0])).abs() < 1e-6, "the override resized the glyph");
        assert_eq!(s1[0].uv, s2[0].uv);
    }

    /// The library says `None` for an id it did not hand out, which is what `queue` returns on.
    #[test]
    fn a_font_from_another_library_measures_to_nothing() {
        let lib = FontLibrary::new();
        assert!(lib.layout(crate::text::FontId(0), 12.0, "x").is_none());
    }
}
