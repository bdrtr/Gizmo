use crate::deferred::DeferredState;
use crate::pipeline::{load_shader_composed, SceneState};
use wgpu::util::DeviceExt;

/// The deferred decal pass: projector boxes drawn against the G-buffer.
///
/// A decal is drawn as a box; where that box contains geometry, the decal's texture is projected
/// onto it and blended into the albedo target. Doing it against the G-buffer rather than as
/// geometry is what lets a decal wrap over whatever it lands on without matching its shape.
pub struct DecalState {
    /// The projection pass.
    pub pipeline: wgpu::RenderPipeline,
    /// Layout of the per-decal uniform group, which is bound with a dynamic offset — hence the
    /// 256-byte stride on [`DecalUniforms`] ([`DECAL_STRIDE`]).
    pub decal_uniform_bgl: wgpu::BindGroupLayout,
    /// That group.
    pub decal_uniform_bg: wgpu::BindGroup,
    /// Layout of the world-position group the pass reconstructs surface positions from.
    pub world_pos_bgl: wgpu::BindGroupLayout,
    /// That group. Rebuilt on resize, because the G-buffer view is.
    pub world_pos_bg: wgpu::BindGroup,

    // Cube mesh for volume rendering
    /// The unit cube every decal is drawn as.
    pub vertex_buffer: wgpu::Buffer,
    /// One [`DecalUniforms`] per decal, at [`DECAL_STRIDE`], [`MAX_DECALS`] of them.
    pub uniform_buffer: wgpu::Buffer,
    /// Whether the pass runs this frame.
    ///
    /// This is the **reversible** off switch. The destructive one — setting the renderer's
    /// `Option` field to `None` — frees the GPU objects and cannot be undone without rebuilding
    /// them; that is the only switch the engine used to have, and it is why a comparison render
    /// could not be turned back on.
    ///
    /// Off, the pass is skipped entirely rather than drawing a neutral result. That is safe here
    /// because every texture this state owns is read only by this state's own apply pass, which is
    /// skipped with it, and the apply pass composites into the HDR target with `LoadOp::Load` —
    /// so a skipped effect leaves the frame exactly as it found it. Resizing still happens while
    /// off, so switching back on costs no rebuild.
    pub enabled: bool,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
/// One decal's transform and tint, padded to the dynamic-offset alignment.
pub struct DecalUniforms {
    /// World → decal space. This is the one that does the work: a surface point is transformed
    /// into the projector's space, and lands inside the unit cube or is discarded.
    pub inv_model: [f32; 16],
    /// Decal → world.
    pub model: [f32; 16],
    /// The tint multiplied into the projected texture; `a` scales its opacity.
    pub albedo_color: [f32; 4],
    /// Padding to 256 bytes — the minimum stride wgpu allows for a dynamically-offset uniform
    /// binding.
    pub _pad: [f32; 28],
}

/// How many decals a frame draws: the uniform arena's slots. A decal past them has nowhere to be
/// written, and the draws stop there.
pub const MAX_DECALS: usize = 1024;

/// One decal's slot in the uniform arena, in bytes: [`DecalUniforms`]' size, which is the 256-byte
/// minimum a dynamically-offset uniform binding may step by. A draw binds decal `i` at `i ×` this.
pub const DECAL_STRIDE: wgpu::BufferAddress = std::mem::size_of::<DecalUniforms>() as wgpu::BufferAddress;
const _: () = assert!(DECAL_STRIDE == 256, "a dynamic uniform offset steps by 256 bytes");

/// The unit cube from -0.5 to 0.5 every decal is drawn as, one triangle list.
const CUBE: [[f32; 3]; 36] = [
    // Front
    [-0.5, -0.5, 0.5],
    [0.5, -0.5, 0.5],
    [0.5, 0.5, 0.5],
    [-0.5, -0.5, 0.5],
    [0.5, 0.5, 0.5],
    [-0.5, 0.5, 0.5],
    // Back
    [-0.5, -0.5, -0.5],
    [-0.5, 0.5, -0.5],
    [0.5, 0.5, -0.5],
    [-0.5, -0.5, -0.5],
    [0.5, 0.5, -0.5],
    [0.5, -0.5, -0.5],
    // Top
    [-0.5, 0.5, -0.5],
    [-0.5, 0.5, 0.5],
    [0.5, 0.5, 0.5],
    [-0.5, 0.5, -0.5],
    [0.5, 0.5, 0.5],
    [0.5, 0.5, -0.5],
    // Bottom
    [-0.5, -0.5, -0.5],
    [0.5, -0.5, -0.5],
    [0.5, -0.5, 0.5],
    [-0.5, -0.5, -0.5],
    [0.5, -0.5, 0.5],
    [-0.5, -0.5, 0.5],
    // Right
    [0.5, -0.5, -0.5],
    [0.5, 0.5, -0.5],
    [0.5, 0.5, 0.5],
    [0.5, -0.5, -0.5],
    [0.5, 0.5, 0.5],
    [0.5, -0.5, 0.5],
    // Left
    [-0.5, -0.5, -0.5],
    [-0.5, -0.5, 0.5],
    [-0.5, 0.5, 0.5],
    [-0.5, -0.5, -0.5],
    [-0.5, 0.5, 0.5],
    [-0.5, 0.5, -0.5],
];

/// The vertices the cube's draw takes.
pub const CUBE_VERTICES: u32 = CUBE.len() as u32;

/// The decal pipeline: the cube drawn from `shader` over `groups` — the scene's global group, the
/// G-buffer's world positions, the decal's texture and its uniforms — blended by alpha into
/// `target`.
///
/// Front faces are culled, so the box is drawn by its back faces: it is drawn while the camera is
/// inside it too, and, with no depth attachment, a covered pixel is not blended twice.
fn decal_pipeline(
    device: &wgpu::Device,
    label: &str,
    shader: &wgpu::ShaderModule,
    groups: [&wgpu::BindGroupLayout; 4],
    target: wgpu::ColorTargetState,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{label} Layout")),
        bind_group_layouts: &groups.map(Some),
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![0 => Float32x3],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(target)],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Front),
            polygon_mode: wgpu::PolygonMode::Fill,
            unclipped_depth: false,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl DecalState {
    /// Builds the decal pipeline, the unit cube and the uniform buffer.
    pub fn new(device: &wgpu::Device, scene: &SceneState, deferred: &DeferredState) -> Self {
        use crate::descriptors::fragment_texture;
        let layout_of = |label: &str, entry: wgpu::BindGroupLayoutEntry| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(label), entries: &[entry] })
        };
        let decal_uniform_bgl = layout_of(
            "Decal Uniform BGL",
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(DECAL_STRIDE),
                },
                count: None,
            },
        );
        let world_pos_bgl = layout_of(
            "Decal WorldPos BGL",
            fragment_texture(0, wgpu::TextureViewDimension::D2, wgpu::TextureSampleType::Float { filterable: false }),
        );
        let world_pos_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Decal WorldPos BG"),
            layout: &world_pos_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&deferred.world_position_view),
            }],
        });

        let global = &scene.global_bind_group_layout;
        let texture = &scene.texture_bind_group_layout;
        let pipeline = decal_pipeline(
            device,
            "Decal Pipeline",
            &load_shader_composed(device, "demo/assets/shaders/decal.wgsl", include_str!("shaders/decal.wgsl"), "decal"),
            [global, &world_pos_bgl, texture, &decal_uniform_bgl],
            // Decals alpha-blend into the albedo G-buffer (deferred RT0), so this MUST match that
            // texture's format. Bound to the shared constant so it can never drift back to the
            // `Rgba16Float` that used to make wgpu abort with a validation error the moment any
            // decal was drawn.
            wgpu::ColorTargetState {
                format: crate::deferred::GBUFFER_ALBEDO_METALLIC_FORMAT,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                // RGB only: RT0.a is the METALLIC channel, not alpha. Writing the decal's
                // coverage/fade into .a (ColorWrites::ALL) alpha-blends it into metallic, so a
                // decal on a dielectric surface turned the patch dark and metallic. Preserve the
                // underlying metallic.
                write_mask: wgpu::ColorWrites::COLOR,
            },
        );
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Decal Cube VB"),
            contents: bytemuck::cast_slice(&CUBE),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Decal Uniform Buffer"),
            size: DECAL_STRIDE * MAX_DECALS as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let decal_uniform_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Decal Uniform BG"),
            layout: &decal_uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniform_buffer,
                    offset: 0,
                    size: wgpu::BufferSize::new(DECAL_STRIDE),
                }),
            }],
        });

        Self {
            enabled: true,
            pipeline,
            decal_uniform_bgl,
            decal_uniform_bg,
            world_pos_bgl,
            world_pos_bg,
            vertex_buffer,
            uniform_buffer,
        }
    }

    /// Rebinds the world-position group after the G-buffer has been rebuilt.
    pub fn resize(&mut self, device: &wgpu::Device, deferred: &DeferredState) {
        self.world_pos_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Decal WorldPos BG"),
            layout: &self.world_pos_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&deferred.world_position_view),
            }],
        });
    }
}

crate::view_targets::screen_targets! {
    DecalState => DecalTargets {
        key world_pos_bgl: wgpu::BindGroupLayout;
        world_pos_bg: wgpu::BindGroup,
    }
}

/// Edge length of every layer of the forward-only path's decal array, in texels: a decal's
/// texture is filtered down to it when it is copied in ([`ForwardDecals::place`]).
pub const DECAL_LAYER_SIZE: u32 = 256;
/// How many different decal textures one frame can show on the forward-only path. A decal whose
/// texture finds no layer is not drawn, and that is said once.
pub const DECAL_LAYERS: u32 = 16;
/// The layers' mip levels, 256² down to 1².
pub const DECAL_LAYER_MIPS: u32 = DECAL_LAYER_SIZE.ilog2() + 1;
/// The layer of a decal with no texture of its own — a material with none, the white one — whose
/// colour is its tint.
pub const NO_DECAL_TEXTURE: u32 = u32::MAX;
/// The decal array's format: colour images, authored in sRGB, as a material's base colour is.
pub const DECAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// One decal as the forward-only path's lit shader reads it (`DECALS` in `shader.wgsl`): a
/// storage array of these, indexed from the decals' half of the cluster lists
/// ([`ClusterAssignment::with_decals`](crate::clustered::ClusterAssignment::with_decals)).
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuDecal {
    /// World → decal space, in which the projector box is the unit cube about the origin.
    pub inv_model: [f32; 16],
    /// [`Decal::color`](crate::components::Decal::color): multiplied into the texture, `a` its
    /// opacity.
    pub tint: [f32; 4],
    /// The texture's layer in the decal array, or [`NO_DECAL_TEXTURE`].
    pub layer: u32,
    /// To the 16-byte stride of a storage array of structs.
    pub _pad: [u32; 3],
}
const _: () = assert!(std::mem::size_of::<GpuDecal>() == 96, "shader.wgsl's GpuDecal is 96 bytes");

impl GpuDecal {
    /// The decal whose box `model` maps the unit cube to, tinted `tint`, its texture in `layer`.
    #[must_use]
    pub fn new(model: gizmo_math::Mat4, tint: gizmo_math::Vec4, layer: u32) -> Self {
        Self { inv_model: model.inverse().to_cols_array(), tint: tint.to_array(), layer, _pad: [0; 3] }
    }
}

/// The forward-only path's decals: the array their textures are copied into, the storage buffer of
/// [`GpuDecal`]s, and the sampler — bound on that path's group 0 (bindings 15–17), where its lit
/// shader projects every decal over the surface it shades, before the light, as the deferred pass
/// paints the G-buffer's albedo. A texel and one decal's room until a frame first draws a decal
/// ([`resident`](Self::resident)), so a scene with none holds none of the array's 5 MiB.
pub struct ForwardDecals {
    texture: wgpu::Texture,
    /// A `D2Array` view of the array, for sampling.
    pub view: wgpu::TextureView,
    /// Bilinear between mips, clamped to the edge.
    pub sampler: wgpu::Sampler,
    /// [`MAX_DECALS`] of [`GpuDecal`], or one.
    pub buffer: wgpu::Buffer,
    /// What each layer holds — kept, so the view a layer was copied from is not freed and another
    /// made at its address while the layer is still taken for it.
    layers: Vec<Option<std::sync::Arc<wgpu::TextureView>>>,
    copy: Option<DecalCopy>,
}

/// The copy of a decal's texture into a level of a layer: the pipeline and the source's layout.
struct DecalCopy {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl ForwardDecals {
    /// A texel and room for one decal, until a frame draws one ([`resident`](Self::resident)).
    pub fn placeholder(device: &wgpu::Device) -> Self {
        Self::sized(device, 1, 1, 1, 1)
    }

    /// The array and buffer a frame with decals needs: [`DECAL_LAYERS`] of [`DECAL_LAYER_SIZE`]²,
    /// and room for [`MAX_DECALS`].
    pub fn resident(device: &wgpu::Device) -> Self {
        Self::sized(device, DECAL_LAYER_SIZE, DECAL_LAYERS, DECAL_LAYER_MIPS, MAX_DECALS)
    }

    /// Whether this is the full array rather than [`placeholder`](Self::placeholder)'s texel.
    #[must_use]
    pub fn is_resident(&self) -> bool {
        self.texture.size().width == DECAL_LAYER_SIZE
    }

    fn sized(device: &wgpu::Device, size: u32, layers: u32, mips: u32, decals: usize) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("forward_decals"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: layers },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DECAL_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("forward_decals_view"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("forward_decals_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("forward_decals_buffer"),
            size: (decals * std::mem::size_of::<GpuDecal>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { texture, view, sampler, buffer, layers: vec![None; layers as usize], copy: None }
    }

    /// The layer of each of `textures` — `None` for one that found none, when this frame shows more
    /// than [`DECAL_LAYERS`] — copying in each that is not in one yet. A layer keeps its texture
    /// while frames show it, and is taken for another once one does not. Call on the
    /// [`resident`](Self::resident) array.
    pub fn place(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        textures: &[std::sync::Arc<wgpu::TextureView>],
    ) -> Vec<Option<u32>> {
        use std::sync::Arc;
        let shown = |layer: &Option<Arc<wgpu::TextureView>>| layer.as_ref().is_some_and(|held| textures.iter().any(|t| Arc::ptr_eq(t, held)));
        let mut copies: Vec<(u32, Arc<wgpu::TextureView>)> = Vec::new();
        let placed = textures
            .iter()
            .map(|texture| {
                if let Some(at) = self.layers.iter().position(|l| l.as_ref().is_some_and(|held| Arc::ptr_eq(held, texture))) {
                    return Some(at as u32);
                }
                let free = self.layers.iter().position(|l| l.is_none()).or_else(|| self.layers.iter().position(|l| !shown(l)))?;
                self.layers[free] = Some(texture.clone());
                copies.push((free as u32, texture.clone()));
                Some(free as u32)
            })
            .collect();
        if !copies.is_empty() {
            self.copy_in(device, queue, &copies);
        }
        placed
    }

    /// Writes `decals`, at most [`MAX_DECALS`], to the buffer the lit shader reads.
    pub fn upload(&self, queue: &wgpu::Queue, decals: &[GpuDecal]) {
        let room = (self.buffer.size() as usize) / std::mem::size_of::<GpuDecal>();
        let decals = &decals[..decals.len().min(room)];
        if !decals.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(decals));
        }
    }

    /// Every level of each `(layer, texture)`, filtered down from the texture, in one submission.
    fn copy_in(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, copies: &[(u32, std::sync::Arc<wgpu::TextureView>)]) {
        let copy = self.copy.get_or_insert_with(|| DecalCopy::new(device));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("forward_decals_copy") });
        for (layer, source) in copies {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("forward_decals_copy_source"),
                layout: &copy.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&copy.sampler) },
                ],
            });
            for level in 0..self.texture.mip_level_count() {
                let target = self.texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("forward_decals_copy_target"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    base_array_layer: *layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("forward_decals_copy"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&copy.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        queue.submit([encoder.finish()]);
    }
}

impl DecalCopy {
    fn new(device: &wgpu::Device) -> Self {
        use crate::descriptors::{fragment_sampler, fragment_texture};
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("forward_decals_copy_layout"),
            entries: &[
                fragment_texture(0, wgpu::TextureViewDimension::D2, wgpu::TextureSampleType::Float { filterable: true }),
                fragment_sampler(1, wgpu::SamplerBindingType::Filtering),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("forward_decals_copy"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/decal_copy.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("forward_decals_copy"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("forward_decals_copy"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: DECAL_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("forward_decals_copy_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        Self { pipeline, layout, sampler }
    }
}
