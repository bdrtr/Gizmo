pub struct FluidPipelines {
    pub compute_bind_group_layout: wgpu::BindGroupLayout,
    pub compute_bind_group: wgpu::BindGroup,

    pub pipeline_clear: wgpu::ComputePipeline,
    pub pipeline_hash: wgpu::ComputePipeline,
    pub pipeline_sort: wgpu::ComputePipeline,
    pub pipeline_offsets: wgpu::ComputePipeline,
    pub pipeline_predict: wgpu::ComputePipeline,
    pub pipeline_calc_lambda: wgpu::ComputePipeline,
    pub pipeline_apply_delta_p: wgpu::ComputePipeline,
    // AAA: Vorticity Confinement — computes curl of velocity field
    pub pipeline_compute_vorticity: wgpu::ComputePipeline,
    pub pipeline_update_velocity: wgpu::ComputePipeline,
    // AAA: Foam/Spray classification
    pub pipeline_classify: wgpu::ComputePipeline,

    pub pipeline_depth: wgpu::RenderPipeline,
    pub pipeline_thickness: wgpu::RenderPipeline,
    pub pipeline_blur: wgpu::ComputePipeline,
    pub pipeline_composite: wgpu::RenderPipeline,
    // AAA: Foam/Spray/Droplet rendering
    pub pipeline_foam: wgpu::RenderPipeline,

    pub particle_render_bg_layout: wgpu::BindGroupLayout,
    pub blur_bind_group_layout: wgpu::BindGroupLayout,
    pub composite_bind_group_layout: wgpu::BindGroupLayout,
}

pub fn create_fluid_pipelines(
    device: &wgpu::Device,
    global_bind_group_layout: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
    params_buffer: &wgpu::Buffer,
    particles_buffer: &wgpu::Buffer,
    grid_buffer: &wgpu::Buffer,
    colliders_buffer: &wgpu::Buffer,
    sort_buffer: &wgpu::Buffer,
    sort_params_buffer: &wgpu::Buffer,
) -> FluidPipelines {
    let compute_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
            ],
            label: Some("fluid_compute_layout"),
        });

    let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &compute_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: particles_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: grid_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: colliders_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: sort_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: sort_params_buffer,
                    offset: 0,
                    size: Some(wgpu::BufferSize::new(16).unwrap()),
                }),
            },
        ],
        label: Some("fluid_compute_bg"),
    });

    let shader_src = format!(
        "{}\n{}\n{}\n{}",
        include_str!("../shaders/kernels.wgsl"),
        include_str!("../shaders/fluid_bindings.wgsl"),
        include_str!("../shaders/spatial_hash.wgsl"),
        include_str!("../shaders/fluid_compute.wgsl")
    );
    let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Fluid Compute Shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Fluid Compute Pipeline Layout"),
        bind_group_layouts: &[&compute_bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline_clear = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fluid_clear"),
        layout: Some(&pipeline_layout),
        module: &compute_shader,
        entry_point: "clear_grid",
        compilation_options: Default::default(),
    });
    let pipeline_hash = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fluid_hash"),
        layout: Some(&pipeline_layout),
        module: &compute_shader,
        entry_point: "hash_pass",
        compilation_options: Default::default(),
    });
    let pipeline_sort = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fluid_sort"),
        layout: Some(&pipeline_layout),
        module: &compute_shader,
        entry_point: "bitonic_sort_pass",
        compilation_options: Default::default(),
    });
    let pipeline_offsets = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fluid_offsets"),
        layout: Some(&pipeline_layout),
        module: &compute_shader,
        entry_point: "grid_offsets_pass",
        compilation_options: Default::default(),
    });
    let pipeline_predict = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Fluid Predict Pipeline"),
        layout: Some(&pipeline_layout),
        module: &compute_shader,
        entry_point: "predict",
        compilation_options: Default::default(),
    });
    let pipeline_calc_lambda = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Fluid Calc Lambda"),
        layout: Some(&pipeline_layout),
        module: &compute_shader,
        entry_point: "calc_lambda",
        compilation_options: Default::default(),
    });
    let pipeline_apply_delta_p = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Fluid Apply Delta P"),
        layout: Some(&pipeline_layout),
        module: &compute_shader,
        entry_point: "apply_delta_p",
        compilation_options: Default::default(),
    });
    // AAA: Vorticity Confinement pipeline (computes ω = ∇ × v)
    let pipeline_compute_vorticity =
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Fluid Compute Vorticity"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: "compute_vorticity",
            compilation_options: Default::default(),
        });
    let pipeline_update_velocity =
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Fluid Update Velocity"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: "update_velocity",
            compilation_options: Default::default(),
        });
    // AAA: Foam/Spray classification pipeline
    let pipeline_classify = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Fluid Classify Particles"),
        layout: Some(&pipeline_layout),
        module: &compute_shader,
        entry_point: "classify_particles",
        compilation_options: Default::default(),
    });

    // SSFR Layouts
    let particle_render_bg_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SSFR Particle BG Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

    let blur_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SSFR Blur BG Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

    let composite_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SSFR Composite BG Layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

    // SSFR Shaders
    let depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Depth Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_depth.wgsl").into()),
    });
    let thickness_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Thickness Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_thickness.wgsl").into()),
    });
    let composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Composite Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_composite.wgsl").into()),
    });
    let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Blur Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_blur.wgsl").into()),
    });

    let ssfr_render_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("SSFR Render Layout"),
        bind_group_layouts: &[global_bind_group_layout, &particle_render_bg_layout],
        push_constant_ranges: &[],
    });

    let pipeline_depth = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("SSFR Depth"),
        layout: Some(&ssfr_render_layout),
        vertex: wgpu::VertexState {
            module: &depth_shader,
            entry_point: "vs_main",
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &depth_shader,
            entry_point: "fs_main",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    let pipeline_thickness = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("SSFR Thickness"),
        layout: Some(&ssfr_render_layout),
        vertex: wgpu::VertexState {
            module: &thickness_shader,
            entry_point: "vs_main",
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &thickness_shader,
            entry_point: "fs_main",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::R16Float, // Thickness uses R16Float
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    let pipeline_blur = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("SSFR Blur"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Blur Layout"),
                bind_group_layouts: &[&blur_bind_group_layout],
                push_constant_ranges: &[],
            }),
        ),
        module: &blur_shader,
        entry_point: "blur_main",
        compilation_options: Default::default(),
    });

    let pipeline_composite = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("SSFR Composite"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Composite Layout"),
                bind_group_layouts: &[global_bind_group_layout, &composite_bind_group_layout],
                push_constant_ranges: &[],
            }),
        ),
        vertex: wgpu::VertexState {
            module: &composite_shader,
            entry_point: "vs_main",
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &composite_shader,
            entry_point: "fs_main",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    // AAA: Foam/Spray render pipeline
    let foam_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Foam Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_foam.wgsl").into()),
    });

    let pipeline_foam = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("SSFR Foam/Spray"),
        layout: Some(&ssfr_render_layout),
        vertex: wgpu::VertexState {
            module: &foam_shader,
            entry_point: "vs_main",
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &foam_shader,
            entry_point: "fs_main",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::One, // Additive
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::OVER,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    FluidPipelines {
        compute_bind_group_layout,
        compute_bind_group,
        pipeline_clear,
        pipeline_hash,
        pipeline_sort,
        pipeline_offsets,
        pipeline_predict,
        pipeline_calc_lambda,
        pipeline_apply_delta_p,
        pipeline_compute_vorticity,
        pipeline_update_velocity,
        pipeline_classify,
        pipeline_depth,
        pipeline_thickness,
        pipeline_blur,
        pipeline_composite,
        pipeline_foam,
        particle_render_bg_layout,
        blur_bind_group_layout,
        composite_bind_group_layout,
    }
}
