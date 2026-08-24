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
//! there is one shader — but there are two *pipelines*, because they disagree about depth: a world
//! label must be hidden by the wall in front of it, and a screen label must not be.
//!
//! # What it does not do
//!
//! Text is drawn into the HDR target, before tone mapping — so it is exposed and bloomed with the
//! rest of the frame. That is right for a world label lit by the same scene and wrong for crisp UI
//! chrome, and it is the first thing to revisit when there is a post-tonemap pass to hang UI on.
//! There is no batching by font: every glyph is an instance and every instance carries its own
//! colour, so a thousand-glyph frame is one draw call and 64 kB.

use gizmo_math::{Vec2, Vec3};

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
    /// Depth ignored, for screen text: UI is on top of the picture by definition.
    pipeline_screen: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    capacity: u32,
    /// This frame's world-space glyphs, then its screen-space ones. Two lists rather than one with
    /// a flag, because each is a contiguous instance range for its own pipeline.
    world: Vec<GlyphInstance>,
    screen: Vec<GlyphInstance>,
    /// How many of each actually reached the buffer, so the draw ranges cannot outrun it.
    uploaded_world: u32,
    uploaded_screen: u32,
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
        global_bind_group_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let atlas = GlyphAtlas::new(device, ATLAS_SIZE);
        let (pipeline_world, pipeline_screen) = Self::build_pipelines(
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
            pipeline_screen,
            buffer,
            capacity: INITIAL_CAPACITY,
            world: Vec::new(),
            screen: Vec::new(),
            uploaded_world: 0,
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
    ) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
        #[cfg(not(target_arch = "wasm32"))]
        let shader = crate::pipeline::load_shader_composed(
            device,
            "demo/assets/shaders/text.wgsl",
            include_str!("../shaders/text.wgsl"),
            "Text Shader",
        );
        #[cfg(target_arch = "wasm32")]
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

        (
            make("Text Pipeline (world)", wgpu::CompareFunction::LessEqual),
            make("Text Pipeline (screen)", wgpu::CompareFunction::Always),
        )
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
        let (world, screen) = Self::build_pipelines(
            device,
            global_bind_group_layout,
            self.atlas.bind_group_layout(),
            color_format,
            depth_format,
        );
        self.pipeline_world = world;
        self.pipeline_screen = screen;
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
    pub fn begin_frame(&mut self) {
        self.world.clear();
        self.screen.clear();
    }

    /// Whether this frame has anything to draw.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.world.is_empty() && self.screen.is_empty()
    }

    /// Adds one `Text` to this frame, rasterising any glyph the atlas has not seen.
    ///
    /// `world_origin` is where world-space text sits and is ignored for screen text;
    /// `screen_size` is the render target in pixels and is ignored for world text. Passing both
    /// rather than two methods keeps the caller's loop one branch shorter, and neither is
    /// expensive to compute.
    ///
    /// A `Text` whose font is not loaded, whose content is empty, or whose glyphs no longer fit in
    /// the atlas contributes nothing. It is not an error: the frame still renders, and
    /// [`GlyphAtlas::is_full`] is where a full atlas is visible.
    pub fn queue(
        &mut self,
        queue: &wgpu::Queue,
        text: &Text,
        world_origin: Vec3,
        screen_size: Vec2,
    ) {
        let Some(layout) = self.library.layout(text.font, text.size_px, &text.content) else {
            return;
        };
        // Split so the rasteriser is the only part of this that needs a GPU: `place` is pure
        // arithmetic over glyph entries, and it is where the anchor, the y flip and the pixels →
        // NDC conversion live — i.e. where the bugs are. The closure is what keeps the atlas
        // (which needs a queue and a `&mut self`) out of it.
        let Self { atlas, library, world, screen, .. } = self;
        let size_px = text.size_px;
        let font = text.font;
        place(&layout, text, world_origin, screen_size, world, screen, |glyph| {
            atlas.glyph(queue, library, font, glyph, size_px)
        });
    }

    /// Uploads this frame's quads, growing the buffer if it has to.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let needed = (self.world.len() + self.screen.len()) as u32;
        self.uploaded_world = 0;
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
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.world));
        let offset = std::mem::size_of_val(self.world.as_slice()) as u64;
        queue.write_buffer(&self.buffer, offset, bytemuck::cast_slice(&self.screen));
        self.uploaded_world = self.world.len() as u32;
        self.uploaded_screen = self.screen.len() as u32;
    }

    /// Draws this frame's text: world labels depth-tested, screen text over everything.
    ///
    /// The pass needs a depth attachment — the world pipeline tests against it — and the caller is
    /// expected to have run [`upload`](Self::upload) into the same frame's encoder.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, global_bind_group: &'a wgpu::BindGroup) {
        if self.uploaded_world == 0 && self.uploaded_screen == 0 {
            return;
        }
        pass.set_bind_group(0, global_bind_group, &[]);
        pass.set_bind_group(1, self.atlas.bind_group(), &[]);
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        if self.uploaded_world > 0 {
            pass.set_pipeline(&self.pipeline_world);
            pass.draw(0..6, 0..self.uploaded_world);
        }
        if self.uploaded_screen > 0 {
            pass.set_pipeline(&self.pipeline_screen);
            let start = self.uploaded_world;
            pass.draw(0..6, start..start + self.uploaded_screen);
        }
    }

    /// This frame's quad counts, `(world, screen)` — what the draw would issue.
    ///
    /// Exposed because "the text did not appear" has two very different causes: nothing was
    /// queued, or something was queued and drawn off-screen. A count separates them without a
    /// screenshot.
    #[must_use]
    pub fn queued(&self) -> (usize, usize) {
        (self.world.len(), self.screen.len())
    }
}

/// Turns a laid-out string into quads, appending to `world` or `screen` depending on the space.
///
/// `entry_of` answers where a glyph is in the atlas; `None` means it has no outline or did not
/// fit, and either way it contributes nothing. Pulled out of [`TextRenderer::queue`] so the
/// placement can be checked without an adapter — it is the half that decides where a glyph lands,
/// and a sign error here is a picture that is upside down rather than a compile error.
fn place(
    layout: &super::TextLayout,
    text: &Text,
    world_origin: Vec3,
    screen_size: Vec2,
    world: &mut Vec<GlyphInstance>,
    screen: &mut Vec<GlyphInstance>,
    mut entry_of: impl FnMut(u16) -> Option<super::GlyphEntry>,
) {
    if layout.glyphs.is_empty() {
        return;
    }
    // Where the text's box goes relative to the anchor point, in text-local pixels with y growing
    // down. Every glyph below is placed against this one offset.
    let box_offset = -text.anchor.factors() * Vec2::new(layout.width, layout.height);
    let color = text.color.to_array();

    for g in &layout.glyphs {
        let Some(entry) = entry_of(g.glyph) else { continue };
        // The glyph box's top-left in text-local pixels, y down.
        let min = box_offset + Vec2::new(g.x + entry.offset[0], g.y + entry.offset[1]);
        let max = min + Vec2::from(entry.size);

        match text.space {
            TextSpace::Screen { position } => {
                // Window pixels (y down, origin top-left) to NDC (y up, origin centre).
                let to_ndc = |p: Vec2| {
                    Vec2::new(
                        p.x / screen_size.x.max(1.0) * 2.0 - 1.0,
                        1.0 - p.y / screen_size.y.max(1.0) * 2.0,
                    )
                };
                let a = to_ndc(position + min);
                let b = to_ndc(position + max);
                screen.push(GlyphInstance {
                    rect: [a.x, a.y, b.x, b.y],
                    uv: entry.uv,
                    color,
                    origin: [0.0, 0.0, 0.0, -1.0],
                });
            }
            TextSpace::World { px_per_unit } => {
                let scale = 1.0 / px_per_unit.max(f32::MIN_POSITIVE);
                // The one place the y flip happens: text-local y grows down and the world's up
                // vector grows up, so the box's top becomes the larger world offset.
                world.push(GlyphInstance {
                    rect: [min.x * scale, -min.y * scale, max.x * scale, -max.y * scale],
                    uv: entry.uv,
                    color,
                    origin: [world_origin.x, world_origin.y, world_origin.z, 1.0],
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
    fn one_glyph() -> TextLayout {
        TextLayout {
            glyphs: vec![PositionedGlyph { glyph: 1, x: 0.0, y: 0.0 }],
            width: 10.0,
            height: 20.0,
            lines: 1,
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
        place(layout, t, Vec3::new(1.0, 2.0, 3.0), Vec2::new(100.0, 200.0), &mut w, &mut s, |_| Some(e));
        (w, s)
    }

    /// Screen text lands in NDC where the pixel arithmetic says, and the y axis flips exactly once.
    ///
    /// The flip is the assertion that matters. Window pixels grow down and NDC grows up, so a
    /// missing negation puts the text below the anchor instead of above it — which looks like an
    /// anchor bug and is not one.
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
                PositionedGlyph { glyph: 1, x: 0.0, y: 0.0 },
                PositionedGlyph { glyph: 2, x: 10.0, y: 0.0 },
                PositionedGlyph { glyph: 3, x: 20.0, y: 0.0 },
            ],
            width: 30.0,
            height: 20.0,
            lines: 1,
        };
        let t = text(TextSpace::Screen { position: Vec2::ZERO }, TextAnchor::TopLeft);
        let (mut w, mut s) = (Vec::new(), Vec::new());
        place(&layout, &t, Vec3::ZERO, Vec2::new(100.0, 100.0), &mut w, &mut s, |g| {
            (g != 2).then(|| entry([0.0, 0.0]))
        });
        assert_eq!(s.len(), 2, "the missing glyph took its neighbours with it");
    }

    /// An unknown font produces nothing rather than a panic, and does not disturb the frame.
    #[test]
    fn an_empty_layout_queues_nothing() {
        let t = text(TextSpace::Screen { position: Vec2::ZERO }, TextAnchor::TopLeft);
        let (mut w, mut s) = (Vec::new(), Vec::new());
        place(&TextLayout::default(), &t, Vec3::ZERO, Vec2::new(100.0, 100.0), &mut w, &mut s, |_| {
            unreachable!("an empty layout must not ask the atlas for anything")
        });
        assert!(w.is_empty() && s.is_empty());
    }

    /// The library says `None` for an id it did not hand out, which is what `queue` returns on.
    #[test]
    fn a_font_from_another_library_measures_to_nothing() {
        let lib = FontLibrary::new();
        assert!(lib.layout(crate::text::FontId(0), 12.0, "x").is_none());
    }
}
