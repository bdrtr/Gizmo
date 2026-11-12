//! Baking an [`IrradianceVolume`](crate::irradiance::IrradianceVolume) on the GPU: each probe's six
//! captured faces, box-filtered to [`FACE`] texels a side, projected into the probe's spherical
//! harmonics (`shaders/irradiance_bake.wgsl`). Drawing the faces is the game path's (`gizmo`'s
//! `capture_irradiance_volume`), as a reflection probe's are, because only it knows how to draw
//! the world; this is the half after each face is drawn.
//!
//! The probes are projected into a buffer of their own and copied into the lighting pass's when
//! every one is done, so no probe's capture sees light another probe of the same bake holds: a bake
//! is one bounce, whatever order its probes are taken in.

use wgpu::util::DeviceExt;

/// Texels per side of the faces a probe is projected from. Irradiance is smooth — a second-order
/// projection keeps nothing finer than about 60° — so 32 texels (2.8° each) hold all it can use.
pub const FACE: u32 = 32;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Floats per probe — `SHCoeffs::to_gpu_data`'s width.
const PROBE_FLOATS: u64 = 28;
/// A job uniform per probe, each at an offset the device accepts for a dynamic one.
const JOB_STRIDE: u64 = 256;

/// The pipelines and the faces texture, built once; and the buffers of the bake in progress.
pub(crate) struct ProbeBake {
    faces: wgpu::Texture,
    face_views: Vec<wgpu::TextureView>,
    downsample: wgpu::RenderPipeline,
    downsample_layout: wgpu::BindGroupLayout,
    project: wgpu::ComputePipeline,
    project_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The bake in progress: its probes' coefficients as they are projected, the jobs, and the
    /// projection's bind group over them.
    pending: Option<Pending>,
}

struct Pending {
    probes: wgpu::Buffer,
    group: wgpu::BindGroup,
    count: u64,
}

impl ProbeBake {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let faces = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("irradiance_bake_faces"),
            size: wgpu::Extent3d { width: FACE, height: FACE, depth_or_array_layers: 6 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let face_views = (0..6)
            .map(|face| {
                faces.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("irradiance_bake_face"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: face,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("irradiance_bake"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/irradiance_bake.wgsl").into()),
        });

        let downsample_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("irradiance_bake_downsample"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
        let downsample = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("irradiance_bake_downsample"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("irradiance_bake_downsample"),
                bind_group_layouts: &[Some(&downsample_layout)],
                immediate_size: 0,
            })),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_downsample"),
                targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let project_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("irradiance_bake_project"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
            ],
        });
        let project = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("irradiance_bake_project"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("irradiance_bake_project"),
                bind_group_layouts: &[Some(&project_layout)],
                immediate_size: 0,
            })),
            module: &module,
            entry_point: Some("cs_project"),
            compilation_options: Default::default(),
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("irradiance_bake_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self { faces, face_views, downsample, downsample_layout, project, project_layout, sampler, pending: None }
    }

    /// Starts a bake of `count` probes with `ambient` (a radiance) added to each.
    pub(crate) fn begin(&mut self, device: &wgpu::Device, count: usize, ambient: gizmo_math::Vec3) {
        let count = count.max(1) as u64;
        let probes = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("irradiance_bake_probes"),
            size: count * PROBE_FLOATS * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let mut jobs = vec![0u8; (count * JOB_STRIDE) as usize];
        for probe in 0..count {
            let job = [probe as f32, ambient.x, ambient.y, ambient.z];
            let at = (probe * JOB_STRIDE) as usize;
            jobs[at..at + 16].copy_from_slice(bytemuck::cast_slice(&job));
        }
        let jobs = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("irradiance_bake_jobs"),
            contents: &jobs,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let faces = self.faces.create_view(&wgpu::TextureViewDescriptor {
            label: Some("irradiance_bake_faces"),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("irradiance_bake_project"),
            layout: &self.project_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&faces) },
                wgpu::BindGroupEntry { binding: 3, resource: probes.as_entire_binding() },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &jobs,
                        offset: 0,
                        size: wgpu::BufferSize::new(16),
                    }),
                },
            ],
        });
        self.pending = Some(Pending { probes, group, count });
    }

    /// Records filtering `frame` — the HDR target, just drawn by a 90° square camera looking along
    /// [`face_basis`](crate::reflection::face_basis)`(face)` — into face `face`.
    pub(crate) fn record_face(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::TextureView, face: usize) {
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("irradiance_bake_downsample"),
            layout: &self.downsample_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(frame) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("irradiance_bake_face"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.face_views[face],
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.downsample);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Records projecting the six faces into probe `probe` of the bake in progress. Call once its
    /// six faces are recorded, before the next probe's first.
    pub(crate) fn record_probe(&self, encoder: &mut wgpu::CommandEncoder, probe: usize) {
        let Some(pending) = self.pending.as_ref() else { return };
        if probe as u64 >= pending.count {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("irradiance_bake_project"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.project);
        pass.set_bind_group(0, &pending.group, &[(probe as u64 * JOB_STRIDE) as u32]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    /// Records copying the finished bake into `target` — the lighting pass's probes — and ends it.
    pub(crate) fn record_finish(&mut self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Buffer) {
        if let Some(pending) = self.pending.take() {
            encoder.copy_buffer_to_buffer(&pending.probes, 0, target, 0, pending.count * PROBE_FLOATS * 4);
        }
    }
}
