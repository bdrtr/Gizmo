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
    /// The same decal, drawn for a pipeline that has no G-buffer.
    ///
    /// The editor draws forward, so `world_pos_bg` (deferred RT2) is empty there and a decal
    /// placed in the editor was invisible until the game ran. This variant reconstructs the
    /// surface position from the depth buffer and blends into the lit HDR image instead. Same
    /// uniforms, same cube, same shader logic — see `decal_forward.wgsl`.
    pub forward_pipeline: wgpu::RenderPipeline,
    /// Layout for the depth texture the forward variant samples (built per frame, like the
    /// particle pass's, because the depth view is recreated on every resize).
    pub depth_bgl: wgpu::BindGroupLayout,
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

/// A decal pipeline: the cube drawn from `shader` over `groups` — the scene's global group, the
/// surface it reads (the G-buffer's world positions, or the depth), the decal's texture and its
/// uniforms — blended by alpha into `target`.
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
        let depth_bgl = layout_of("Decal Depth BGL", fragment_texture(0, wgpu::TextureViewDimension::D2, wgpu::TextureSampleType::Depth));
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
        let forward_pipeline = decal_pipeline(
            device,
            "Decal Forward Pipeline",
            &load_shader_composed(
                device,
                "demo/assets/shaders/decal_forward.wgsl",
                include_str!("shaders/decal_forward.wgsl"),
                "decal_forward",
            ),
            // Group 1 is the depth, not the G-buffer.
            [global, &depth_bgl, texture, &decal_uniform_bgl],
            // The lit HDR image, not the albedo G-buffer — hence `ColorWrites::ALL` here where the
            // deferred variant must preserve RT0's metallic channel.
            wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
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
            forward_pipeline,
            depth_bgl,
            decal_uniform_bgl,
            decal_uniform_bg,
            world_pos_bgl,
            world_pos_bg,
            vertex_buffer,
            uniform_buffer,
        }
    }

    /// Bind group for the forward variant's depth input. Built per frame like the particle
    /// pass's: the depth view is a new object after every resize, so caching it would hand the
    /// GPU a view of a texture that no longer exists.
    pub fn create_depth_bind_group(
        &self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Decal Depth BG"),
            layout: &self.depth_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(depth_view),
            }],
        })
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
