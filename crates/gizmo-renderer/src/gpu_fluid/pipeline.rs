/// Every pipeline the fluid runs: the position-based solver's compute stages, and the
/// screen-space passes that turn its particles into a water surface.
///
/// The surface is rendered in screen space (SSFR) rather than by extracting a mesh: depth and
/// thickness are splatted from the particles, blurred into a smooth surface, and shaded — which
/// costs the same whether there are ten thousand particles or a hundred thousand, and needs no
/// marching-cubes pass on the CPU.
pub struct FluidPipelines {
    /// Layout of the group holding every simulation buffer.
    pub compute_bind_group_layout: wgpu::BindGroupLayout,
    /// That group.
    pub compute_bind_group: wgpu::BindGroup,

    /// Clears the grid.
    pub pipeline_clear: wgpu::ComputePipeline,
    /// Hashes each particle into its cell.
    pub pipeline_hash: wgpu::ComputePipeline,
    /// One stage of the bitonic sort over those hashes — dispatched repeatedly, with a different
    /// [`SortParams`](crate::gpu_fluid::types::SortParams) each time.
    pub pipeline_sort: wgpu::ComputePipeline,
    /// Finds each cell's range in the sorted list.
    pub pipeline_offsets: wgpu::ComputePipeline,
    /// Applies external forces and predicts a position.
    pub pipeline_predict: wgpu::ComputePipeline,
    /// Computes each particle's density constraint multiplier.
    pub pipeline_calc_lambda: wgpu::ComputePipeline,
    /// Computes the resulting position correction into the scratch buffer. It, the stage above and
    /// [`Self::pipeline_commit_positions`] run once per solver iteration.
    pub pipeline_apply_delta_p: wgpu::ComputePipeline,
    /// Copies the corrected positions in, once every particle has read its neighbours'.
    pub pipeline_commit_positions: wgpu::ComputePipeline,
    /// Stores each particle's velocity from its correction, for its neighbours to read.
    pub pipeline_derive_velocity: wgpu::ComputePipeline,
    /// Copies the new velocities in and takes the step.
    pub pipeline_commit_velocity: wgpu::ComputePipeline,
    /// Counts, per obstacle, the water against it (`count_wetting`).
    pub pipeline_count_wetting: wgpu::ComputePipeline,
    // AAA: Vorticity Confinement — computes curl of velocity field
    /// Computes the curl of the velocity field, for vorticity confinement.
    pub pipeline_compute_vorticity: wgpu::ComputePipeline,
    /// Derives the new velocity from the position correction, then applies vorticity and XSPH.
    pub pipeline_update_velocity: wgpu::ComputePipeline,
    // AAA: Foam/Spray classification
    /// Classifies particles into phases — which is what promotes agitated surface water to foam.
    pub pipeline_classify: wgpu::ComputePipeline,

    /// Splats each particle's depth, as a sphere impostor.
    pub pipeline_depth: wgpu::RenderPipeline,
    /// Splats accumulated thickness, which is what drives the absorption through the water.
    pub pipeline_thickness: wgpu::RenderPipeline,
    /// Blurs the splatted depth into a smooth surface — the step that turns visible spheres into
    /// water.
    pub pipeline_blur: wgpu::ComputePipeline,
    /// Shades that surface: normals from the blurred depth, refraction and reflection over the
    /// scene behind it.
    pub pipeline_composite: wgpu::RenderPipeline,
    // AAA: Foam/Spray/Droplet rendering
    /// Draws foam and bubble particles as sprites over the surface.
    pub pipeline_foam: wgpu::RenderPipeline,

    /// Layout of the group the splat passes read particles through.
    pub particle_render_bg_layout: wgpu::BindGroupLayout,
    /// Layout of the blur pass's group.
    pub blur_bind_group_layout: wgpu::BindGroupLayout,
    /// Layout of the composite pass's group.
    pub composite_bind_group_layout: wgpu::BindGroupLayout,
}

/// Builds every fluid pipeline and bind group over the given buffers and targets.
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
    scratch_buffer: &wgpu::Buffer,
    wetting_buffer: &wgpu::Buffer,
) -> FluidPipelines {
    use crate::descriptors::compute_pipeline;

    let compute_bind_group_layout = simulation_layout(device);
    let compute_bind_group = create_compute_bind_group(
        device,
        &compute_bind_group_layout,
        params_buffer,
        particles_buffer,
        grid_buffer,
        colliders_buffer,
        sort_buffer,
        sort_params_buffer,
        scratch_buffer,
        wetting_buffer,
    );

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
        bind_group_layouts: &[Some(&compute_bind_group_layout)],
        immediate_size: 0,
    });
    let stage = |label: &str, entry: &str| compute_pipeline(device, label, &pipeline_layout, &compute_shader, entry);

    let pipeline_clear = stage("fluid_clear", "clear_grid");
    let pipeline_hash = stage("fluid_hash", "hash_pass");
    let pipeline_sort = stage("fluid_sort", "bitonic_sort_pass");
    let pipeline_offsets = stage("fluid_offsets", "grid_offsets_pass");
    let pipeline_predict = stage("Fluid Predict Pipeline", "predict");
    let pipeline_calc_lambda = stage("Fluid Calc Lambda", "calc_lambda");
    let pipeline_apply_delta_p = stage("Fluid Apply Delta P", "apply_delta_p");
    // AAA: Vorticity Confinement pipeline (computes ω = ∇ × v)
    let pipeline_compute_vorticity = stage("Fluid Compute Vorticity", "compute_vorticity");
    let pipeline_update_velocity = stage("Fluid Update Velocity", "update_velocity");
    let pipeline_commit_positions = stage("Fluid Commit Positions", "commit_positions");
    let pipeline_derive_velocity = stage("Fluid Derive Velocity", "derive_velocity");
    let pipeline_commit_velocity = stage("Fluid Commit Velocity", "commit_velocity");
    let pipeline_count_wetting = stage("Fluid Count Wetting", "count_wetting");
    // AAA: Foam/Spray classification pipeline
    let pipeline_classify = stage("Fluid Classify Particles", "classify_particles");

    // SSFR Layouts
    let (particle_render_bg_layout, blur_bind_group_layout, composite_bind_group_layout) = surface_layouts(device);

    // SSFR Shaders
    let depth_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Depth Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_depth.wgsl").into()),
    });
    let thickness_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Thickness Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_thickness.wgsl").into()),
    });
    let composite_shader = crate::pipeline::load_shader_composed(
        device,
        "demo/assets/shaders/fluid_composite.wgsl",
        include_str!("../shaders/fluid_composite.wgsl"),
        "Composite Shader",
    );
    let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Blur Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fluid_blur.wgsl").into()),
    });

    let ssfr_render_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("SSFR Render Layout"),
        bind_group_layouts: &[Some(global_bind_group_layout), Some(&particle_render_bg_layout)],
        immediate_size: 0,
    });

    let pipeline_depth = surface_pipeline(
        device,
        "SSFR Depth",
        &ssfr_render_layout,
        &depth_shader,
        wgpu::PrimitiveTopology::TriangleStrip,
        wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba16Float,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        },
        Some(depth_test(true)),
    );

    let pipeline_thickness = surface_pipeline(
        device,
        "SSFR Thickness",
        &ssfr_render_layout,
        &thickness_shader,
        wgpu::PrimitiveTopology::TriangleStrip,
        wgpu::ColorTargetState {
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
        },
        None,
    );

    let blur_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Blur Layout"),
        bind_group_layouts: &[Some(&blur_bind_group_layout)],
        immediate_size: 0,
    });
    let pipeline_blur = compute_pipeline(device, "SSFR Blur", &blur_layout, &blur_shader, "blur_main");

    let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Composite Layout"),
        bind_group_layouts: &[Some(global_bind_group_layout), Some(&composite_bind_group_layout)],
        immediate_size: 0,
    });
    let pipeline_composite = surface_pipeline(
        device,
        "SSFR Composite",
        &composite_layout,
        &composite_shader,
        wgpu::PrimitiveTopology::TriangleList,
        wgpu::ColorTargetState {
            format: output_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        },
        Some(depth_test(false)),
    );

    // AAA: Foam/Spray render pipeline
    let foam_shader = crate::pipeline::load_shader_composed(
        device,
        "demo/assets/shaders/fluid_foam.wgsl",
        include_str!("../shaders/fluid_foam.wgsl"),
        "Foam Shader",
    );
    let pipeline_foam = surface_pipeline(
        device,
        "SSFR Foam/Spray",
        &ssfr_render_layout,
        &foam_shader,
        wgpu::PrimitiveTopology::TriangleStrip,
        wgpu::ColorTargetState {
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
        },
        Some(depth_test(false)),
    );

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
        pipeline_commit_positions,
        pipeline_derive_velocity,
        pipeline_commit_velocity,
        pipeline_count_wetting,
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

/// The layout of the group every simulation pass binds: the parameters, the particles, the grid,
/// the colliders (read only), the sort keys, the sort stage's parameters (a 16-byte slot at a
/// dynamic offset, one per stage), the scratch and the wetting counters.
fn simulation_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    use crate::descriptors::buffer_entry;
    use wgpu::BufferBindingType::{Storage, Uniform};
    let compute = wgpu::ShaderStages::COMPUTE;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[
            buffer_entry(0, compute, Uniform),
            buffer_entry(1, compute, Storage { read_only: false }),
            buffer_entry(2, compute, Storage { read_only: false }),
            buffer_entry(3, compute, Storage { read_only: true }),
            buffer_entry(4, compute, Storage { read_only: false }),
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: compute,
                ty: wgpu::BindingType::Buffer {
                    ty: Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            },
            buffer_entry(6, compute, Storage { read_only: false }),
            buffer_entry(7, compute, Storage { read_only: false }),
        ],
        label: Some("fluid_compute_layout"),
    })
}

/// The surface passes' layouts: the particles the splats read, the blur's group (the depth in,
/// the smoothed depth out, its parameters) and the composite's (depth, thickness, scene colour and
/// their samplers).
fn surface_layouts(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
    use crate::descriptors::{buffer_entry, texture_2d_entry};
    let particle_render_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("SSFR Particle BG Layout"),
        entries: &[buffer_entry(1, wgpu::ShaderStages::VERTEX, wgpu::BufferBindingType::Storage { read_only: true })],
    });

    let compute = wgpu::ShaderStages::COMPUTE;
    let blur_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("SSFR Blur BG Layout"),
        entries: &[
            texture_2d_entry(0, compute, false),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: compute,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba16Float,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
            buffer_entry(2, compute, wgpu::BufferBindingType::Uniform),
        ],
    });

    let fragment = wgpu::ShaderStages::FRAGMENT;
    let sampler = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: fragment,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
        count: None,
    };
    let composite_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("SSFR Composite BG Layout"),
        entries: &[
            texture_2d_entry(0, fragment, false),
            sampler(1),
            texture_2d_entry(2, fragment, false),
            sampler(3),
            texture_2d_entry(4, fragment, true),
        ],
    });
    (particle_render_bg_layout, blur_bind_group_layout, composite_bind_group_layout)
}

/// One of the surface's screen passes: `vs_main` and `fs_main` of `shader`, no vertex buffers,
/// drawing into `target`.
fn surface_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    topology: wgpu::PrimitiveTopology,
    target: wgpu::ColorTargetState,
    depth_stencil: Option<wgpu::DepthStencilState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(target)],
        }),
        primitive: wgpu::PrimitiveState { topology, ..Default::default() },
        depth_stencil,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// A `Less` test against the scene's `Depth32Float` depth, writing to it when `write`.
fn depth_test(write: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(write),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

/// The group every compute pass binds, over these buffers — made again when one of them is (the
/// grid, for a new box).
#[allow(clippy::too_many_arguments)]
pub(crate) fn create_compute_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    params_buffer: &wgpu::Buffer,
    particles_buffer: &wgpu::Buffer,
    grid_buffer: &wgpu::Buffer,
    colliders_buffer: &wgpu::Buffer,
    sort_buffer: &wgpu::Buffer,
    sort_params_buffer: &wgpu::Buffer,
    scratch_buffer: &wgpu::Buffer,
    wetting_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout,
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
            wgpu::BindGroupEntry {
                binding: 6,
                resource: scratch_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wetting_buffer.as_entire_binding(),
            },
        ],
        label: Some("fluid_compute_bg"),
    })
}
