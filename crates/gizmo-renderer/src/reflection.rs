//! Reflection probe — the scene, captured into a cubemap, for metals and glossy surfaces to reflect.
//!
//! Until 2026-11-01 specular image-based lighting read `get_procedural_environment`: a sky preset,
//! so a chrome sphere in a red room reflected a blue sky. SSR covers what is on screen and nothing
//! else. A probe renders the scene from one point in six directions into a small HDR cube, then
//! prefilters its mips by roughness (GGX, split-sum), and the lighting pass reads it in place of
//! the procedural sky — mip `roughness × max_mip` along the reflected ray.
//!
//! This module is the GPU half: the cube, the face copy, the prefilter, and the parameters the
//! lighting pass reads. Rendering the six faces is the game path's (`gizmo`'s
//! `capture_reflection_probes`), because only it knows how to draw the world.
//!
//! Up to [`MAX_PROBES`] probes share one cube **array** (six layers per slot), because the
//! lighting pass has no bind group left to give each its own. The lighting shader blends them by
//! their influence boxes (see `ReflectionProbe::influence`).

use wgpu::util::DeviceExt;

/// Cube faces, in WebGPU's layer order: +X, −X, +Y, −Y, +Z, −Z.
pub const FACES: usize = 6;

/// How many probes the lighting pass reads at once. A scene with more uses the first four by
/// entity order.
pub const MAX_PROBES: usize = 4;

/// Rendering parameters for face `face`: the direction it looks along and the camera "up" that
/// makes a right-handed render of it the cube's face mirrored left-right (see the shader).
pub fn face_basis(face: usize) -> (gizmo_math::Vec3, gizmo_math::Vec3) {
    use gizmo_math::Vec3;
    match face {
        0 => (Vec3::X, Vec3::Y),
        1 => (Vec3::NEG_X, Vec3::Y),
        2 => (Vec3::Y, Vec3::NEG_Z),
        3 => (Vec3::NEG_Y, Vec3::Z),
        4 => (Vec3::Z, Vec3::Y),
        _ => (Vec3::NEG_Z, Vec3::Y),
    }
}

/// The captured cubes and the passes that fill them.
#[non_exhaustive]
pub struct ReflectionProbeGpu {
    /// The cube array: `size`², [`mip_count`](Self::mip_count) mips, six layers per slot,
    /// `Rgba16Float`.
    pub texture: wgpu::Texture,
    /// The whole array as a `CubeArray` view — what the lighting pass samples.
    pub cube_view: wgpu::TextureView,
    /// How many probe slots the array has.
    pub slots: u32,
    /// Which entity each slot was captured for — the game path rebuilds the array when this set
    /// changes.
    pub owners: Vec<u32>,
    /// Where each slot was captured. A probe moved without a recapture keeps reflecting from here,
    /// so its parallax stays consistent with the picture it holds.
    pub captured_at: Vec<[f32; 3]>,
    /// The parameters last uploaded to the lighting pass, so they are re-sent only when they change.
    pub uploaded: Vec<crate::irradiance::ReflectionSource>,
    /// Edge length of mip 0, in texels.
    pub size: u32,
    /// Mips, from `size` down to 4 texels.
    pub mip_count: u32,
    copy_pipeline: wgpu::RenderPipeline,
    prefilter_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    dummy_cube: wgpu::TextureView,
    dummy_frame: wgpu::TextureView,
    /// One job uniform per (mip, face): written once at build, so recording every pass into one
    /// encoder cannot leave them all reading the last write.
    jobs: Vec<wgpu::Buffer>,
}

/// GGX samples per prefiltered texel.
const PREFILTER_SAMPLES: f32 = 64.0;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl ReflectionProbeGpu {
    /// Builds an empty array of `slots` cubes (1..=[`MAX_PROBES`]) of `size` texels per edge
    /// (rounded up to a power of two, at least 8).
    pub fn new(device: &wgpu::Device, size: u32, slots: u32) -> Self {
        let size = size.clamp(8, 1024).next_power_of_two();
        let slots = slots.clamp(1, MAX_PROBES as u32);
        let mip_count = size.trailing_zeros() - 1; // down to 4 texels
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("reflection_probe_cube"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: FACES as u32 * slots },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let cube_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("reflection_probe_cube_view"),
            dimension: Some(wgpu::TextureViewDimension::CubeArray),
            ..Default::default()
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reflection_probe"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/reflection_probe.wgsl").into()),
        });
        let tex_entry = |binding, view_dimension| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension,
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reflection_probe_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                tex_entry(1, wgpu::TextureViewDimension::D2),
                tex_entry(2, wgpu::TextureViewDimension::Cube),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("reflection_probe"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let copy_pipeline = pipeline("fs_copy");
        let prefilter_pipeline = pipeline("fs_prefilter");

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reflection_probe_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let dummy = |label, layers, dimension| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: layers },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: FORMAT,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor { dimension: Some(dimension), ..Default::default() })
        };
        let dummy_cube = dummy("reflection_probe_dummy_cube", 6, wgpu::TextureViewDimension::Cube);
        let dummy_frame = dummy("reflection_probe_dummy_frame", 1, wgpu::TextureViewDimension::D2);

        let jobs = (0..mip_count)
            .flat_map(|mip| (0..FACES).map(move |face| (mip, face)))
            .map(|(mip, face)| {
                let roughness = mip as f32 / (mip_count - 1).max(1) as f32;
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("reflection_probe_job"),
                    contents: bytemuck::cast_slice(&[face as f32, roughness, PREFILTER_SAMPLES, size as f32]),
                    usage: wgpu::BufferUsages::UNIFORM,
                })
            })
            .collect();

        Self {
            texture,
            cube_view,
            slots,
            owners: Vec::new(),
            captured_at: vec![[0.0; 3]; slots as usize],
            uploaded: Vec::new(),
            size,
            mip_count,
            copy_pipeline,
            prefilter_pipeline,
            layout,
            sampler,
            dummy_cube,
            dummy_frame,
            jobs,
        }
    }

    fn face_view(&self, mip: u32, slot: u32, face: usize) -> wgpu::TextureView {
        self.texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("reflection_probe_face"),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_mip_level: mip,
            mip_level_count: Some(1),
            base_array_layer: slot * FACES as u32 + face as u32,
            array_layer_count: Some(1),
            ..Default::default()
        })
    }

    fn bind(&self, device: &wgpu::Device, job: &wgpu::Buffer, frame: &wgpu::TextureView, cube: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reflection_probe_bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: job.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(frame) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(cube) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }

    fn fullscreen(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("reflection_probe_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Records copying `frame` — the HDR target, just rendered by a 90° square camera looking
    /// along [`face_basis`]`(face)` — into mip 0 of that face of slot `slot`.
    pub fn record_face(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::TextureView,
        slot: u32,
        face: usize,
    ) {
        let group = self.bind(device, &self.jobs[face], frame, &self.dummy_cube);
        self.fullscreen(encoder, &self.face_view(0, slot, face), &self.copy_pipeline, &group);
    }

    /// Records prefiltering every mip above 0 of slot `slot` from the ones below it. Call once all
    /// six of its faces are in mip 0.
    pub fn record_prefilter(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, slot: u32) {
        for mip in 1..self.mip_count {
            // This slot's six layers, every mip below this one, as a cube: disjoint from the mip
            // being written.
            let source = self.texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("reflection_probe_source"),
                dimension: Some(wgpu::TextureViewDimension::Cube),
                base_mip_level: 0,
                mip_level_count: Some(mip),
                base_array_layer: slot * FACES as u32,
                array_layer_count: Some(FACES as u32),
                ..Default::default()
            });
            for face in 0..FACES {
                let job = &self.jobs[mip as usize * FACES + face];
                let group = self.bind(device, job, &self.dummy_frame, &source);
                self.fullscreen(encoder, &self.face_view(mip, slot, face), &self.prefilter_pipeline, &group);
            }
        }
    }
}
