use crate::pipeline::{load_shader, SceneState};
use crate::deferred::DeferredState;
use wgpu::util::DeviceExt;

pub struct DecalState {
    pub pipeline: wgpu::RenderPipeline,
    pub decal_uniform_bgl: wgpu::BindGroupLayout,
    pub decal_uniform_bg: wgpu::BindGroup,
    pub world_pos_bgl: wgpu::BindGroupLayout,
    pub world_pos_bg: wgpu::BindGroup,
    
    // Cube mesh for volume rendering
    pub vertex_buffer: wgpu::Buffer,
    pub uniform_buffer: wgpu::Buffer,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DecalUniforms {
    pub inv_model: [f32; 16],
    pub model: [f32; 16],
    pub albedo_color: [f32; 4],
    pub _pad: [f32; 28], // pad to 256 bytes for dynamic offset alignment
}

impl DecalState {
    pub fn new(device: &wgpu::Device, scene: &SceneState, deferred: &DeferredState) -> Self {
        let shader = load_shader(device, "crates/gizmo-renderer/src/shaders/decal.wgsl", include_str!("shaders/decal.wgsl"), "decal");



        let decal_uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Decal Uniform BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(256),
                    },
                    count: None,
                },
            ],
        });

        let world_pos_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Decal WorldPos BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let world_pos_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Decal WorldPos BG"),
            layout: &world_pos_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&deferred.world_position_view),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Decal Pipeline Layout"),
            bind_group_layouts: &[
                &scene.global_bind_group_layout, // 0
                &world_pos_bgl,                  // 1
                &scene.texture_bind_group_layout, // 2
                &decal_uniform_bgl,              // 3
            ],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Decal Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                    }
                ],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[
                    // Albedo blending!
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
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
            multiview: None,
        });

        // Unit cube from -0.5 to 0.5
        let cube_vertices: &[[f32; 3]] = &[
            // Front
            [-0.5, -0.5,  0.5], [ 0.5, -0.5,  0.5], [ 0.5,  0.5,  0.5],
            [-0.5, -0.5,  0.5], [ 0.5,  0.5,  0.5], [-0.5,  0.5,  0.5],
            // Back
            [-0.5, -0.5, -0.5], [-0.5,  0.5, -0.5], [ 0.5,  0.5, -0.5],
            [-0.5, -0.5, -0.5], [ 0.5,  0.5, -0.5], [ 0.5, -0.5, -0.5],
            // Top
            [-0.5,  0.5, -0.5], [-0.5,  0.5,  0.5], [ 0.5,  0.5,  0.5],
            [-0.5,  0.5, -0.5], [ 0.5,  0.5,  0.5], [ 0.5,  0.5, -0.5],
            // Bottom
            [-0.5, -0.5, -0.5], [ 0.5, -0.5, -0.5], [ 0.5, -0.5,  0.5],
            [-0.5, -0.5, -0.5], [ 0.5, -0.5,  0.5], [-0.5, -0.5,  0.5],
            // Right
            [ 0.5, -0.5, -0.5], [ 0.5,  0.5, -0.5], [ 0.5,  0.5,  0.5],
            [ 0.5, -0.5, -0.5], [ 0.5,  0.5,  0.5], [ 0.5, -0.5,  0.5],
            // Left
            [-0.5, -0.5, -0.5], [-0.5, -0.5,  0.5], [-0.5,  0.5,  0.5],
            [-0.5, -0.5, -0.5], [-0.5,  0.5,  0.5], [-0.5,  0.5, -0.5],
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
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &uniform_buffer,
                        offset: 0,
                        size: Some(wgpu::BufferSize::new(256).unwrap()),
                    }),
                },
            ],
        });

        Self {
            pipeline,
            decal_uniform_bgl,
            decal_uniform_bg,
            world_pos_bgl,
            world_pos_bg,
            vertex_buffer,
            uniform_buffer,
        }
    }

    pub fn resize(&mut self, device: &wgpu::Device, deferred: &DeferredState) {
        self.world_pos_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Decal WorldPos BG"),
            layout: &self.world_pos_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&deferred.world_position_view),
                },
            ],
        });
    }
}
