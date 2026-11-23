use super::types::GpuParticle;

/// The particle system's two pipelines: one compute pass that steps every particle, one draw that
/// billboards them.
pub struct ParticlePipelines {
    /// The simulation pass.
    pub compute_pipeline: wgpu::ComputePipeline,
    /// The particle buffer and parameters, as that pass reads them.
    pub compute_bind_group: wgpu::BindGroup,
    /// The billboard draw, taking the same buffer as instance data.
    pub render_pipeline: wgpu::RenderPipeline,
    /// Group 1: the scene depth texture (sampled in the FS for soft particles). The bind group
    /// is rebuilt every frame from the current `depth_texture_view`, because the view changes on
    /// resize.
    pub depth_bind_group_layout: wgpu::BindGroupLayout,
    /// Group 2: the flipbook/SubUV atlas texture and sampler (smoke sprites). 1×1 white by
    /// default; `set_flipbook` uploads the real atlas. The `misc.z` flag switches it on and off
    /// in the FS.
    pub flipbook_bind_group_layout: wgpu::BindGroupLayout,
    /// The same billboards, depth only, where they show at all (`fs_veil`): what the volumetric
    /// march splits the air at (`VolumetricState::veil_depth_view`). Native: the forward-only path
    /// has no volumetric light.
    #[cfg(not(gizmo_forward_only))]
    pub veil_pipeline: wgpu::RenderPipeline,
}

/// Builds both particle pipelines and the compute bind group over the given buffers.
pub fn create_particle_pipelines(
    device: &wgpu::Device,
    global_bind_group_layout: &wgpu::BindGroupLayout,
    output_format: wgpu::TextureFormat,
    params_buffer: &wgpu::Buffer,
    particles_buffer: &wgpu::Buffer,
) -> ParticlePipelines {
    use crate::descriptors::{buffer_entry, compute_pipeline, whole_buffer_entries};
    use wgpu::BufferBindingType::{Storage, Uniform};
    let compute = wgpu::ShaderStages::COMPUTE;
    let compute_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("particle_compute_layout"),
        entries: &[buffer_entry(0, compute, Uniform), buffer_entry(1, compute, Storage { read_only: false })],
    });
    let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("particle_compute_bind_group"),
        layout: &compute_bind_group_layout,
        entries: &whole_buffer_entries(&[params_buffer, particles_buffer]),
    });
    let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Particle Compute Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/particle_compute.wgsl").into()),
    });
    let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Particle Compute Pipeline Layout"),
        bind_group_layouts: &[Some(&compute_bind_group_layout)],
        immediate_size: 0,
    });
    let compute_pipeline = compute_pipeline(device, "Particle Compute Pipeline", &compute_pipeline_layout, &compute_shader, "main");

    let (depth_bind_group_layout, flipbook_bind_group_layout) = billboard_layouts(device);
    // Composed so particle_render.wgsl can `#import gizmo::common::{SceneUniforms}`.
    let render_shader = crate::pipeline::load_shader_composed(
        device,
        "demo/assets/shaders/particle_render.wgsl",
        include_str!("../shaders/particle_render.wgsl"),
        "Particle Render Shader",
    );
    let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Particle Render Pipeline Layout"),
        bind_group_layouts: &[
            Some(global_bind_group_layout),
            Some(&depth_bind_group_layout),
            Some(&flipbook_bind_group_layout),
        ],
        immediate_size: 0,
    });
    let billboard = |label: &str, fragment_entry: &str, targets: &[Option<wgpu::ColorTargetState>], depth_stencil| {
        billboard_pipeline(device, label, &render_pipeline_layout, &render_shader, fragment_entry, targets, depth_stencil)
    };
    let render_pipeline = billboard(
        "Particle Render Pipeline",
        "fs_main",
        &[Some(wgpu::ColorTargetState {
            format: output_format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })],
        // Depth attachment YOK: particle'lar ayrı pass'te (soft-particle derinlik örneklemesi
        // için) çizilir; occlusion + soft-fade FS'te sahne derinliğinden manuel yapılır.
        None,
    );
    #[cfg(not(gizmo_forward_only))]
    let veil_pipeline = billboard(
        "Particle Veil Pipeline",
        "fs_veil",
        &[],
        Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
    );

    ParticlePipelines {
        compute_pipeline,
        compute_bind_group,
        render_pipeline,
        depth_bind_group_layout,
        flipbook_bind_group_layout,
        #[cfg(not(gizmo_forward_only))]
        veil_pipeline,
    }
}

/// The billboard draws' groups 1 and 2.
///
/// Group 1: the scene's depth (soft particles, read with `textureLoad`), and what a lit particle is
/// lit by at its centre, in the vertex stage: the view's clustered lights — the same buffers group 0
/// has, bound again here because there they are the fragment stage's alone — and the spots' cookies.
///
/// Group 2: the flipbook atlas (filterable) and its sampler, and the flipbook's config uniform
/// (x = tiles per side, y = on 1 / off 0), which the vertex stage reads too: whether the particles
/// are lit, and how they scatter.
fn billboard_layouts(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::BindGroupLayout) {
    use crate::descriptors::{buffer_entry, fragment_sampler, fragment_texture, sampler_entry, texture_entry};
    use wgpu::{BufferBindingType, SamplerBindingType, ShaderStages, TextureSampleType, TextureViewDimension};
    let lights = |binding| buffer_entry(binding, ShaderStages::VERTEX, BufferBindingType::Storage { read_only: true });
    let filtered = TextureSampleType::Float { filterable: true };
    let depth = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("particle_depth_layout"),
        entries: &[
            fragment_texture(0, TextureViewDimension::D2, TextureSampleType::Depth),
            lights(1),
            lights(2),
            texture_entry(3, ShaderStages::VERTEX, TextureViewDimension::D2Array, filtered),
            sampler_entry(4, ShaderStages::VERTEX, SamplerBindingType::Filtering),
        ],
    });
    let flipbook = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("particle_flipbook_layout"),
        entries: &[
            fragment_texture(0, TextureViewDimension::D2, filtered),
            fragment_sampler(1, SamplerBindingType::Filtering),
            buffer_entry(2, ShaderStages::VERTEX_FRAGMENT, BufferBindingType::Uniform),
        ],
    });
    (depth, flipbook)
}

/// A billboard draw of the particles: a quad's corners (location 4) per vertex, a [`GpuParticle`]
/// per instance, a strip with nothing culled.
fn billboard_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    fragment_entry: &str,
    targets: &[Option<wgpu::ColorTargetState>],
    depth_stencil: Option<wgpu::DepthStencilState>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[
                Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![4 => Float32x2],
                }),
                Some(GpuParticle::desc()),
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fragment_entry),
            compilation_options: Default::default(),
            targets,
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
