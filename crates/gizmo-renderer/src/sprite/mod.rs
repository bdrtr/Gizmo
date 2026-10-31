//! Sprites: loaded images, and the pass that draws [`Sprite`] quads with them.
//!
//! The engine's first 2D path (`docs/CAPABILITY_GAPS.md` §A1). It is deliberately the same shape
//! as [`crate::text`]: a library of GPU resources handed out as small ids, a per-frame instance
//! list a draw loop fills through [`SpriteRenderer::queue`], and one shared host-side pass
//! (`gizmo::systems::render::record_sprites`) that both the game and `gizmo-studio` call.
//!
//! # Corners on the CPU
//!
//! Each instance carries its four corners **already in clip space**. The caller hands
//! [`SpriteRenderer::queue`] a view-projection, and the game path hands it the *unjittered* one:
//! sprites are drawn after TAA, and the scene uniforms' `view_proj` carries TAA's sub-pixel jitter,
//! so a sprite projected by the shader would shimmer by exactly that much — invisible on a 3D
//! label, obvious on pixel art. It also means the shader needs no scene uniforms at all.
//!
//! # Order
//!
//! Lower `layer` first; then, for world sprites, back to front by clip-space depth; then entity
//! order. The sort is stable and keyed on values the world decides, so the same world draws the
//! same picture. Consecutive sprites that share an image become one draw call.
//!
//! # What it does not do
//!
//! - **No mipmaps.** A large image drawn small aliases. Right for pixel art and for UI drawn near
//!   its native size; wrong for a big texture shrunk on a distant card.
//! - **Pre-tonemap.** Like text, sprites are drawn into the HDR target and so are exposed and
//!   bloomed with the frame: a sprite's colours are scene radiance, not final pixel values. For a
//!   2D game with neutral exposure that is invisible; for HUD chrome over a dark 3D scene with
//!   auto-exposure it is not, and it is the same thing to revisit when there is a post-tonemap pass.
//! - **No texture atlas packing.** One image is one texture and one bind group; a sheet of frames
//!   is one image with [`Sprite::uv_rect`](crate::components::Sprite::uv_rect) choosing the cell.

use gizmo_math::{Mat4, Vec2, Vec4};

use crate::components::{Sprite, SpriteSpace};

/// A loaded image, as a [`Sprite`] refers to it.
///
/// Opaque on purpose, like [`FontId`](crate::text::FontId): it is an index, and only meaningful
/// for the [`SpriteRenderer`] that handed it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpriteImageId(pub(crate) u32);

/// How an image is sampled between texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SpriteFilter {
    /// Bilinear — smooth when scaled. Photographs, painted art, UI.
    #[default]
    Linear,
    /// Nearest texel — hard edges when scaled. Pixel art.
    Nearest,
}

/// Why an image could not be loaded.
#[derive(Debug)]
#[non_exhaustive]
pub enum SpriteError {
    /// A width or height of zero.
    Empty,
    /// Larger than the device can hold in one texture.
    TooLarge {
        /// The image's width.
        width: u32,
        /// The image's height.
        height: u32,
        /// The device's `max_texture_dimension_2d`.
        max: u32,
    },
    /// The pixel buffer is not `width × height × 4` bytes.
    WrongLength {
        /// `width × height × 4`.
        expected: usize,
        /// What was passed.
        actual: usize,
    },
    /// The bytes are not an image format this engine can decode.
    ///
    /// Carries no decoder type, for the reason [`crate::text::FontError::Invalid`] gives.
    Invalid,
    /// The file could not be read.
    Io(std::io::Error),
}

impl std::fmt::Display for SpriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "the image has no pixels"),
            Self::TooLarge { width, height, max } => {
                write!(f, "a {width}×{height} image exceeds the device's {max}-texel limit")
            }
            Self::WrongLength { expected, actual } => {
                write!(f, "expected {expected} bytes of RGBA8, got {actual}")
            }
            Self::Invalid => write!(f, "not a decodable image"),
            Self::Io(e) => write!(f, "could not read the image file: {e}"),
        }
    }
}

impl std::error::Error for SpriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// One sprite quad, as the shader reads it. Mirrors `SpriteInstance` in `shaders/sprite.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct SpriteInstance {
    /// Clip-space corners: top-left, top-right, bottom-left, bottom-right of the image.
    corners: [[f32; 4]; 4],
    /// `(u0, v0, u1, v1)` — `u0, v0` at the top-left corner. Flips are already applied.
    uv: [f32; 4],
    color: [f32; 4],
}

impl SpriteInstance {
    const ATTRS: [wgpu::VertexAttribute; 6] = [
        wgpu::VertexAttribute { offset: 0, shader_location: 0, format: wgpu::VertexFormat::Float32x4 },
        wgpu::VertexAttribute { offset: 16, shader_location: 1, format: wgpu::VertexFormat::Float32x4 },
        wgpu::VertexAttribute { offset: 32, shader_location: 2, format: wgpu::VertexFormat::Float32x4 },
        wgpu::VertexAttribute { offset: 48, shader_location: 3, format: wgpu::VertexFormat::Float32x4 },
        wgpu::VertexAttribute { offset: 64, shader_location: 4, format: wgpu::VertexFormat::Float32x4 },
        wgpu::VertexAttribute { offset: 80, shader_location: 5, format: wgpu::VertexFormat::Float32x4 },
    ];

    fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRS,
        }
    }
}

/// A sprite waiting for this frame's sort.
#[derive(Debug, Clone, Copy)]
struct Queued {
    image: u32,
    layer: i32,
    /// Clip-space depth of the anchor; larger is farther. Zero for screen sprites.
    depth: f32,
    /// Queue order — the last tie-break, so equal keys keep the order the world gave them.
    seq: u32,
    instance: SpriteInstance,
}

/// A contiguous instance range drawn with one image.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Run {
    image: u32,
    start: u32,
    end: u32,
}

struct Image {
    bind_group: wgpu::BindGroup,
    size: [u32; 2],
}

/// Loaded images, the sprite pipelines, and this frame's quads.
pub struct SpriteRenderer {
    layout: wgpu::BindGroupLayout,
    linear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    images: Vec<Image>,
    /// Depth-tested against the scene, writing none.
    pipeline_world: wgpu::RenderPipeline,
    /// Over everything.
    pipeline_screen: wgpu::RenderPipeline,
    buffer: wgpu::Buffer,
    capacity: u32,
    world: Vec<Queued>,
    screen: Vec<Queued>,
    runs_world: Vec<Run>,
    runs_screen: Vec<Run>,
}

impl std::fmt::Debug for SpriteRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpriteRenderer")
            .field("images", &self.images.len())
            .field("capacity", &self.capacity)
            .field("queued_world", &self.world.len())
            .field("queued_screen", &self.screen.len())
            .finish()
    }
}

const INITIAL_CAPACITY: u32 = 1024;

impl SpriteRenderer {
    /// Builds the samplers, the image layout and both pipelines. No image is loaded.
    #[must_use]
    pub fn new(
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Sprite Image Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = |label: &str, filter: wgpu::FilterMode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(label),
                mag_filter: filter,
                min_filter: filter,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                // Clamp: a sheet cell sampled at its edge must not bleed in the opposite edge of
                // the image, which is what repeat does at u = 1.
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                ..Default::default()
            })
        };
        let linear = sampler("Sprite Sampler (linear)", wgpu::FilterMode::Linear);
        let nearest = sampler("Sprite Sampler (nearest)", wgpu::FilterMode::Nearest);
        let (pipeline_world, pipeline_screen) =
            Self::build_pipelines(device, &layout, color_format, depth_format);
        let buffer = Self::instance_buffer(device, INITIAL_CAPACITY);
        Self {
            layout,
            linear,
            nearest,
            images: Vec::new(),
            pipeline_world,
            pipeline_screen,
            buffer,
            capacity: INITIAL_CAPACITY,
            world: Vec::new(),
            screen: Vec::new(),
            runs_world: Vec::new(),
            runs_screen: Vec::new(),
        }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Sprite Instance Buffer"),
            size: u64::from(capacity) * std::mem::size_of::<SpriteInstance>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Shared by [`new`](Self::new) and [`rebuild_pipelines`](Self::rebuild_pipelines) so a hot
    /// reload cannot build them differently.
    fn build_pipelines(
        device: &wgpu::Device,
        image_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> (wgpu::RenderPipeline, wgpu::RenderPipeline) {
        #[cfg(not(target_arch = "wasm32"))]
        let shader = crate::pipeline::load_shader_composed(
            device,
            "demo/assets/shaders/sprite.wgsl",
            include_str!("../shaders/sprite.wgsl"),
            "Sprite Shader",
        );
        #[cfg(target_arch = "wasm32")]
        let shader = crate::pipeline::load_shader_composed_web(
            device,
            include_str!("../shaders/sprite.wgsl"),
            "Sprite Shader",
        );

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Sprite Pipeline Layout"),
            bind_group_layouts: &[Some(image_layout)],
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
                    buffers: &[Some(SpriteInstance::desc())],
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
                    // A mirrored sprite — `flip_x`, or a negative scale on the entity — reverses
                    // the winding, and a mirrored sprite is still meant to be seen.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth_format,
                    // Never written: sprites are alpha-blended and ordered by the sort, and a
                    // sprite that wrote depth would cut its transparent corners out of whatever
                    // is painted behind it later.
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
            make("Sprite Pipeline (world)", wgpu::CompareFunction::LessEqual),
            make("Sprite Pipeline (screen)", wgpu::CompareFunction::Always),
        )
    }

    /// Recompiles `sprite.wgsl` and rebuilds both pipelines, keeping every loaded image.
    pub fn rebuild_pipelines(
        &mut self,
        device: &wgpu::Device,
        color_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) {
        let (world, screen) = Self::build_pipelines(device, &self.layout, color_format, depth_format);
        self.pipeline_world = world;
        self.pipeline_screen = screen;
    }

    /// Uploads `width × height` RGBA8 pixels (sRGB, straight alpha, rows top to bottom).
    ///
    /// # Errors
    ///
    /// [`SpriteError::Empty`], [`SpriteError::WrongLength`] or [`SpriteError::TooLarge`].
    pub fn load_rgba(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgba: &[u8],
        width: u32,
        height: u32,
        filter: SpriteFilter,
    ) -> Result<SpriteImageId, SpriteError> {
        if width == 0 || height == 0 {
            return Err(SpriteError::Empty);
        }
        let max = device.limits().max_texture_dimension_2d;
        if width > max || height > max {
            return Err(SpriteError::TooLarge { width, height, max });
        }
        let expected = width as usize * height as usize * 4;
        if rgba.len() != expected {
            return Err(SpriteError::WrongLength { expected, actual: rgba.len() });
        }
        let id = u32::try_from(self.images.len()).map_err(|_| SpriteError::TooLarge {
            width,
            height,
            max,
        })?;

        let extent = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Sprite Image"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            extent,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = match filter {
            SpriteFilter::Linear => &self.linear,
            SpriteFilter::Nearest => &self.nearest,
        };
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Sprite Image Bind Group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        });
        self.images.push(Image { bind_group, size: [width, height] });
        Ok(SpriteImageId(id))
    }

    /// Decodes an encoded image (PNG, JPEG, WebP, …) and uploads it.
    ///
    /// # Errors
    ///
    /// [`SpriteError::Invalid`] if the bytes do not decode; otherwise as
    /// [`load_rgba`](Self::load_rgba).
    pub fn load_encoded(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
        filter: SpriteFilter,
    ) -> Result<SpriteImageId, SpriteError> {
        let img = image::load_from_memory(bytes).map_err(|_| SpriteError::Invalid)?.to_rgba8();
        let (w, h) = img.dimensions();
        self.load_rgba(device, queue, img.as_raw(), w, h, filter)
    }

    /// The size in pixels of a loaded image, or `None` for an id this renderer did not hand out.
    #[must_use]
    pub fn image_size(&self, id: SpriteImageId) -> Option<[u32; 2]> {
        self.images.get(id.0 as usize).map(|i| i.size)
    }

    /// Drops last frame's quads. Call once per frame before any [`queue`](Self::queue).
    pub fn begin_frame(&mut self) {
        self.world.clear();
        self.screen.clear();
        self.runs_world.clear();
        self.runs_screen.clear();
    }

    /// Adds one sprite to this frame.
    ///
    /// `model` is the entity's world matrix and is ignored in screen space; `view_proj` is the
    /// camera's (unjittered — see the module docs); `screen_size` is the target in pixels.
    ///
    /// A sprite with an unknown image, a non-positive size, a fully transparent tint, or a
    /// non-finite corner contributes nothing.
    pub fn queue(&mut self, sprite: &Sprite, model: Mat4, view_proj: Mat4, screen_size: Vec2) {
        if self.images.get(sprite.image.0 as usize).is_none() {
            return;
        }
        let Some((instance, depth)) = place(sprite, model, view_proj, screen_size) else {
            return;
        };
        let world = sprite.space.needs_world_transform();
        let list = if world { &mut self.world } else { &mut self.screen };
        let seq = list.len() as u32;
        list.push(Queued { image: sprite.image.0, layer: sprite.layer, depth, seq, instance });
    }

    /// Sorts this frame's quads, uploads them, and groups them into draws.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        sort_world(&mut self.world);
        sort_screen(&mut self.screen);
        let total = self.world.len() + self.screen.len();
        if total == 0 {
            return;
        }
        let needed = u32::try_from(total).unwrap_or(u32::MAX);
        if needed > self.capacity {
            let mut capacity = self.capacity.max(1);
            while capacity < needed {
                capacity = capacity.saturating_mul(2);
            }
            self.buffer = Self::instance_buffer(device, capacity);
            self.capacity = capacity;
        }
        let instances: Vec<SpriteInstance> =
            self.world.iter().chain(self.screen.iter()).map(|q| q.instance).collect();
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&instances));
        self.runs_world = runs(&self.world, 0);
        self.runs_screen = runs(&self.screen, self.world.len() as u32);
    }

    /// Draws world sprites (depth-tested), then screen sprites (over everything).
    ///
    /// The pass needs a depth attachment, and [`upload`](Self::upload) must have run this frame.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.runs_world.is_empty() && self.runs_screen.is_empty() {
            return;
        }
        pass.set_vertex_buffer(0, self.buffer.slice(..));
        for (pipeline, list) in
            [(&self.pipeline_world, &self.runs_world), (&self.pipeline_screen, &self.runs_screen)]
        {
            if list.is_empty() {
                continue;
            }
            pass.set_pipeline(pipeline);
            for run in list {
                pass.set_bind_group(0, &self.images[run.image as usize].bind_group, &[]);
                pass.draw(0..6, run.start..run.end);
            }
        }
    }

    /// This frame's quad counts, `(world, screen)`.
    #[must_use]
    pub fn queued(&self) -> (usize, usize) {
        (self.world.len(), self.screen.len())
    }

    /// How many draw calls this frame costs, `(world, screen)` — one per run of consecutive
    /// sprites sharing an image, after the sort.
    #[must_use]
    pub fn draw_calls(&self) -> (usize, usize) {
        (self.runs_world.len(), self.runs_screen.len())
    }
}

/// The sprite's instance and its sort depth, or `None` if it draws nothing.
///
/// Pure arithmetic, so the placement — the part with the sign errors — is testable without a GPU.
fn place(sprite: &Sprite, model: Mat4, view_proj: Mat4, screen_size: Vec2) -> Option<(SpriteInstance, f32)> {
    if !(sprite.size.x > 0.0 && sprite.size.y > 0.0) || sprite.color.w <= 0.0 {
        return None;
    }
    // Image-space corners, (0, 0) the top-left: TL, TR, BL, BR — the order the shader expects.
    const CORNERS: [Vec2; 4] =
        [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0)];
    let mut corners = [[0.0f32; 4]; 4];
    let depth = match sprite.space {
        SpriteSpace::World => {
            let mvp = view_proj * model;
            for (out, c) in corners.iter_mut().zip(CORNERS) {
                // Image y grows down; the entity's local y grows up. This is the one flip.
                let local = (c - sprite.anchor) * Vec2::new(sprite.size.x, -sprite.size.y);
                *out = (mvp * Vec4::new(local.x, local.y, 0.0, 1.0)).to_array();
            }
            let anchor = mvp * Vec4::new(0.0, 0.0, 0.0, 1.0);
            // NDC depth grows with distance under both of the engine's projections, so one key
            // serves perspective AND orthographic — where clip `w` is a constant 1 and would sort
            // nothing. Behind the camera counts as farthest; the clipper discards it anyway.
            if anchor.w > 0.0 { anchor.z / anchor.w } else { f32::INFINITY }
        }
        SpriteSpace::Screen { position } => {
            let size = Vec2::new(screen_size.x.max(1.0), screen_size.y.max(1.0));
            for (out, c) in corners.iter_mut().zip(CORNERS) {
                let px = position + (c - sprite.anchor) * sprite.size;
                *out = [px.x / size.x * 2.0 - 1.0, 1.0 - px.y / size.y * 2.0, 0.0, 1.0];
            }
            0.0
        }
    };
    if !corners.iter().flatten().all(|v| v.is_finite()) {
        return None;
    }
    let [mut u0, mut v0, mut u1, mut v1] = sprite.uv_rect;
    if sprite.flip_x {
        std::mem::swap(&mut u0, &mut u1);
    }
    if sprite.flip_y {
        std::mem::swap(&mut v0, &mut v1);
    }
    Some((SpriteInstance { corners, uv: [u0, v0, u1, v1], color: sprite.color.to_array() }, depth))
}

/// Layer, then far to near, then queue order.
fn sort_world(list: &mut [Queued]) {
    list.sort_by(|a, b| {
        a.layer.cmp(&b.layer).then(b.depth.total_cmp(&a.depth)).then(a.seq.cmp(&b.seq))
    });
}

/// Layer, then queue order.
fn sort_screen(list: &mut [Queued]) {
    list.sort_by(|a, b| a.layer.cmp(&b.layer).then(a.seq.cmp(&b.seq)));
}

/// Consecutive same-image stretches of a sorted list, offset by `base` into the buffer.
fn runs(list: &[Queued], base: u32) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for (i, q) in list.iter().enumerate() {
        let i = base + i as u32;
        match out.last_mut() {
            Some(run) if run.image == q.image => run.end = i + 1,
            _ => out.push(Run { image: q.image, start: i, end: i + 1 }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_math::{Quat, Vec3};

    const SCREEN: Vec2 = Vec2::new(200.0, 100.0);

    fn ndc_to_px(c: [f32; 4]) -> Vec2 {
        Vec2::new((c[0] / c[3] + 1.0) * 0.5 * SCREEN.x, (1.0 - c[1] / c[3]) * 0.5 * SCREEN.y)
    }

    /// Screen sprites land in pixels where the arithmetic says, with y flipped exactly once.
    #[test]
    fn a_screen_sprite_covers_its_pixel_box() {
        let s = Sprite::screen(SpriteImageId(0), Vec2::new(20.0, 10.0), Vec2::new(50.0, 30.0));
        let (inst, depth) = place(&s, Mat4::IDENTITY, Mat4::IDENTITY, SCREEN).unwrap();
        assert_eq!(depth, 0.0);
        let px: Vec<Vec2> = inst.corners.iter().map(|&c| ndc_to_px(c)).collect();
        assert!((px[0] - Vec2::new(50.0, 30.0)).length() < 1e-4, "top-left {:?}", px[0]);
        assert!((px[1] - Vec2::new(70.0, 30.0)).length() < 1e-4, "top-right {:?}", px[1]);
        assert!((px[2] - Vec2::new(50.0, 40.0)).length() < 1e-4, "bottom-left {:?}", px[2]);
        assert!((px[3] - Vec2::new(70.0, 40.0)).length() < 1e-4, "bottom-right {:?}", px[3]);
        // In NDC the top must be ABOVE the bottom — the assertion a missing flip fails.
        assert!(inst.corners[0][1] > inst.corners[2][1]);
    }

    /// The anchor moves the box by its fraction of the size, on each axis separately.
    #[test]
    fn the_anchor_is_a_fraction_of_the_size_per_axis() {
        let s = Sprite::screen(SpriteImageId(0), Vec2::new(20.0, 10.0), Vec2::new(50.0, 30.0))
            .with_anchor(Vec2::new(0.5, 1.0));
        let (inst, _) = place(&s, Mat4::IDENTITY, Mat4::IDENTITY, SCREEN).unwrap();
        // Centred horizontally (10 px left), standing on the point (10 px up).
        assert!((ndc_to_px(inst.corners[0]) - Vec2::new(40.0, 20.0)).length() < 1e-4);
    }

    /// A world sprite lies in the entity's XY plane with its image top at +Y, and follows the
    /// entity's translation, rotation and scale.
    #[test]
    fn a_world_sprite_follows_the_entity_transform_with_y_up() {
        let s = Sprite::world(SpriteImageId(0), Vec2::new(2.0, 4.0));
        let model = Mat4::from_scale_rotation_translation(
            Vec3::new(3.0, 1.0, 1.0),
            Quat::IDENTITY,
            Vec3::new(10.0, 0.0, -5.0),
        );
        let (inst, _) = place(&s, model, Mat4::IDENTITY, SCREEN).unwrap();
        let p = |i: usize| Vec3::new(inst.corners[i][0], inst.corners[i][1], inst.corners[i][2]);
        // Centred: the 2×4 quad spans x ±1 (×3 scale = ±3) and y ±2, on the z = −5 plane.
        assert!((p(0) - Vec3::new(7.0, 2.0, -5.0)).length() < 1e-5, "top-left {}", p(0));
        assert!((p(3) - Vec3::new(13.0, -2.0, -5.0)).length() < 1e-5, "bottom-right {}", p(3));

        // A quarter turn about Z takes the image's top (+Y) to −X.
        let turned = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let (inst, _) = place(&s.with_anchor(Vec2::new(0.5, 0.0)), turned, Mat4::IDENTITY, SCREEN).unwrap();
        let mid_top = (Vec4::from(inst.corners[0]) + Vec4::from(inst.corners[1])) * 0.5;
        assert!(mid_top.truncate().length() < 1e-5, "the anchored top edge stays on the origin");
        let mid_bottom = (Vec4::from(inst.corners[2]) + Vec4::from(inst.corners[3])) * 0.5;
        assert!((mid_bottom.x - 4.0).abs() < 1e-5, "the bottom swings to +X: {mid_bottom}");
    }

    /// Depth is a usable sort key under an orthographic camera, where clip `w` is constant.
    #[test]
    fn orthographic_depth_still_orders_sprites_by_distance() {
        let proj = Mat4::orthographic_rh(-10.0, 10.0, -10.0, 10.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, Vec3::Y);
        let vp = proj * view;
        let s = Sprite::world(SpriteImageId(0), Vec2::ONE);
        let at = |z: f32| place(&s, Mat4::from_translation(Vec3::new(0.0, 0.0, z)), vp, SCREEN).unwrap().1;
        assert!(at(-3.0) > at(0.0) && at(0.0) > at(2.0), "farther must sort larger");
    }

    #[test]
    fn flips_swap_the_uv_edges_and_leave_the_corners_alone() {
        let s = Sprite::screen(SpriteImageId(0), Vec2::ONE, Vec2::ZERO).with_uv_rect([0.1, 0.2, 0.3, 0.4]);
        let (plain, _) = place(&s, Mat4::IDENTITY, Mat4::IDENTITY, SCREEN).unwrap();
        let (flipped, _) = place(&s.with_flip(true, true), Mat4::IDENTITY, Mat4::IDENTITY, SCREEN).unwrap();
        assert_eq!(plain.uv, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(flipped.uv, [0.3, 0.4, 0.1, 0.2]);
        assert_eq!(plain.corners, flipped.corners);
    }

    #[test]
    fn degenerate_sprites_draw_nothing() {
        let base = Sprite::world(SpriteImageId(0), Vec2::ONE);
        let mut zero = base.clone();
        zero.size = Vec2::new(0.0, 1.0);
        let mut nan = base.clone();
        nan.size = Vec2::new(f32::NAN, 1.0);
        let clear = base.clone().with_color(Vec4::new(1.0, 1.0, 1.0, 0.0));
        let broken = Mat4::from_translation(Vec3::new(f32::INFINITY, 0.0, 0.0));
        for (label, s, m) in [("zero size", &zero, Mat4::IDENTITY), ("NaN size", &nan, Mat4::IDENTITY), ("transparent", &clear, Mat4::IDENTITY), ("infinite transform", &base, broken)] {
            assert!(place(s, m, Mat4::IDENTITY, SCREEN).is_none(), "{label} drew something");
        }
    }

    fn q(image: u32, layer: i32, depth: f32, seq: u32) -> Queued {
        Queued { image, layer, depth, seq, instance: SpriteInstance::default_zeroed() }
    }

    impl SpriteInstance {
        fn default_zeroed() -> Self {
            bytemuck::Zeroable::zeroed()
        }
    }

    /// Layer beats depth, depth goes far to near, and equal keys keep queue order.
    #[test]
    fn world_sort_is_layer_then_far_to_near_then_queue_order() {
        let mut list = vec![q(0, 1, 0.9, 0), q(1, 0, 0.2, 1), q(2, 0, 0.8, 2), q(3, 0, 0.2, 3)];
        sort_world(&mut list);
        let order: Vec<u32> = list.iter().map(|q| q.image).collect();
        assert_eq!(order, vec![2, 1, 3, 0]);
    }

    #[test]
    fn screen_sort_ignores_depth() {
        let mut list = vec![q(0, 2, 0.0, 0), q(1, -1, 0.0, 1), q(2, 2, 0.0, 2)];
        sort_screen(&mut list);
        let order: Vec<u32> = list.iter().map(|q| q.image).collect();
        assert_eq!(order, vec![1, 0, 2]);
    }

    /// Consecutive same-image sprites share a draw; an interleaved image splits them, because the
    /// paint order is the sort's and batching must not reorder it.
    #[test]
    fn runs_merge_neighbours_and_never_reorder() {
        let list = vec![q(5, 0, 0.0, 0), q(5, 0, 0.0, 1), q(7, 0, 0.0, 2), q(5, 0, 0.0, 3)];
        assert_eq!(
            runs(&list, 10),
            vec![
                Run { image: 5, start: 10, end: 12 },
                Run { image: 7, start: 12, end: 13 },
                Run { image: 5, start: 13, end: 14 },
            ]
        );
        assert!(runs(&[], 0).is_empty());
    }

    #[test]
    fn the_instance_is_the_six_vec4s_the_shader_reads() {
        assert_eq!(std::mem::size_of::<SpriteInstance>(), 96);
        assert_eq!(std::mem::offset_of!(SpriteInstance, uv), 64);
        assert_eq!(std::mem::offset_of!(SpriteInstance, color), 80);
    }
}
