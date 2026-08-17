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
    /// 256-byte stride on [`DecalUniforms`].
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
    /// One [`DecalUniforms`] per decal, at 256-byte stride.
    pub uniform_buffer: wgpu::Buffer,
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

impl DecalState {
    /// Builds the decal pipeline, the unit cube and the uniform buffer.
    pub fn new(device: &wgpu::Device, scene: &SceneState, deferred: &DeferredState) -> Self {
        let shader = load_shader_composed(
            device,
            "crates/gizmo-renderer/src/shaders/decal.wgsl",
            include_str!("shaders/decal.wgsl"),
            "decal",
        );

        let decal_uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Decal Uniform BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(256),
                },
                count: None,
            }],
        });

        let world_pos_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Decal WorldPos BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let world_pos_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Decal WorldPos BG"),
            layout: &world_pos_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&deferred.world_position_view),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Decal Pipeline Layout"),
            bind_group_layouts: &[
                Some(&scene.global_bind_group_layout),  // 0
                Some(&world_pos_bgl),                   // 1
                Some(&scene.texture_bind_group_layout), // 2
                Some(&decal_uniform_bgl),               // 3
            ],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Decal Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[
                    // Decals alpha-blend into the albedo G-buffer (deferred RT0), so this
                    // MUST match that texture's format. Bound to the shared constant so it
                    // can never drift back to the `Rgba16Float` that used to make wgpu abort
                    // with a validation error the moment any decal was drawn.
                    Some(wgpu::ColorTargetState {
                        format: crate::deferred::GBUFFER_ALBEDO_METALLIC_FORMAT,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        // RGB only: RT0.a is the METALLIC channel, not alpha. Writing the
                        // decal's coverage/fade into .a (ColorWrites::ALL) alpha-blends it
                        // into metallic, so a decal on a dielectric surface turned the patch
                        // dark and metallic. Preserve the underlying metallic.
                        write_mask: wgpu::ColorWrites::COLOR,
                    }),
                ],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Front), // Cull front faces so we render when inside the decal box
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let depth_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Decal Depth BGL"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let forward_shader = load_shader_composed(
            device,
            "crates/gizmo-renderer/src/shaders/decal_forward.wgsl",
            include_str!("shaders/decal_forward.wgsl"),
            "decal_forward",
        );
        let forward_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Decal Forward Pipeline Layout"),
            bind_group_layouts: &[
                Some(&scene.global_bind_group_layout),  // 0
                Some(&depth_bgl),                       // 1 — depth, not the G-buffer
                Some(&scene.texture_bind_group_layout), // 2
                Some(&decal_uniform_bgl),               // 3
            ],
            immediate_size: 0,
        });
        let forward_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Decal Forward Pipeline"),
            layout: Some(&forward_layout),
            vertex: wgpu::VertexState {
                module: &forward_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &forward_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    // The lit HDR image, not the albedo G-buffer — hence `ColorWrites::ALL`
                    // here where the deferred variant must preserve RT0's metallic channel.
                    format: wgpu::TextureFormat::Rgba16Float,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // Back faces only: with no depth attachment both faces of the volume would
                // rasterise and every covered pixel would be blended twice.
                cull_mode: Some(wgpu::Face::Front),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Unit cube from -0.5 to 0.5
        let cube_vertices: &[[f32; 3]] = &[
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

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Decal Cube VB"),
            contents: bytemuck::cast_slice(cube_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Decal Uniform Buffer"),
            size: 256 * 1024, // 1024 decals max
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
                    size: Some(wgpu::BufferSize::new(256).unwrap()),
                }),
            }],
        });

        Self {
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
