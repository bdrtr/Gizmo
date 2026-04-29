use wgpu::util::DeviceExt;

use crate::deferred::DeferredState;
use crate::pipeline::{load_shader, SceneState};

const KERNEL_SIZE: usize = 32;
const NOISE_SIZE:  u32   = 4;

// ── CPU data types ────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SsaoKernel {
    samples: [[f32; 4]; KERNEL_SIZE],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SsaoParams {
    pub strength: f32,
    _pad: [f32; 3],
}

impl Default for SsaoParams {
    fn default() -> Self {
        Self { strength: 0.8, _pad: [0.0; 3] }
    }
}

// ── SsaoState ────────────────────────────────────────────────────────────────

pub struct SsaoState {
    // AO render targets
    pub ao_texture:         wgpu::Texture,
    pub ao_view:            wgpu::TextureView,
    pub ao_blurred_texture: wgpu::Texture,
    pub ao_blurred_view:    wgpu::TextureView,

    // Noise + kernel resources (static — never resized)
    noise_texture: wgpu::Texture,
    noise_view:    wgpu::TextureView,
    noise_sampler: wgpu::Sampler,
    gbuf_sampler:  wgpu::Sampler,
    kernel_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,

    // SSAO pass: G-buffer inputs → raw AO
    pub ssao_pipeline:            wgpu::RenderPipeline,
    ssao_gbuf_bgl:                wgpu::BindGroupLayout,
    pub ssao_gbuf_bind_group:     wgpu::BindGroup,

    // Blur pass: raw AO → blurred AO
    pub blur_pipeline:            wgpu::RenderPipeline,
    blur_bgl:                     wgpu::BindGroupLayout,
    pub blur_bind_group:          wgpu::BindGroup,

    // Apply pass: blurred AO × HDR (multiply blend)
    pub apply_pipeline:           wgpu::RenderPipeline,
    apply_bgl:                    wgpu::BindGroupLayout,
    pub apply_bind_group:         wgpu::BindGroup,

    pub width:  u32,
    pub height: u32,
}

impl SsaoState {
    pub fn new(
        device:   &wgpu::Device,
        queue:    &wgpu::Queue,
        scene:    &SceneState,
        deferred: &DeferredState,
        width:    u32,
        height:   u32,
    ) -> Self {
        let kernel_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("ssao_kernel"),
            contents: bytemuck::bytes_of(&build_kernel()),
            usage:    wgpu::BufferUsages::UNIFORM,
        });

        let (noise_texture, noise_view) = build_noise_texture(device, queue);

        let noise_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter:     wgpu::FilterMode::Nearest,
            min_filter:     wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let gbuf_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Nearest,
            min_filter:     wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label:    Some("ssao_params"),
            contents: bytemuck::bytes_of(&SsaoParams::default()),
            usage:    wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let (ao_texture, ao_view)               = mk_ao_tex(device, width, height);
        let (ao_blurred_texture, ao_blurred_view) = mk_ao_tex(device, width, height);

        // ── Bind group layouts ──────────────────────────────────────────────
        let ssao_gbuf_bgl = mk_ssao_gbuf_bgl(device);
        let blur_bgl      = mk_blur_bgl(device);
        let apply_bgl     = mk_apply_bgl(device);

        // ── Bind groups ─────────────────────────────────────────────────────
        let ssao_gbuf_bind_group = mk_ssao_gbuf_bg(
            device, &ssao_gbuf_bgl,
            &deferred.normal_roughness_view,
            &deferred.world_position_view,
            &noise_view, &gbuf_sampler, &noise_sampler, &kernel_buffer,
        );
        let blur_bind_group  = mk_blur_bg(device, &blur_bgl, &ao_view, &gbuf_sampler);
        let apply_bind_group = mk_apply_bg(device, &apply_bgl, &ao_blurred_view, &gbuf_sampler, &params_buffer);

        // ── Pipelines ───────────────────────────────────────────────────────
        let ssao_pipeline  = mk_ssao_pipeline(device, scene, &ssao_gbuf_bgl);
        let blur_pipeline  = mk_blur_pipeline(device, &blur_bgl);
        let apply_pipeline = mk_apply_pipeline(device, &apply_bgl);

        Self {
            ao_texture, ao_view,
            ao_blurred_texture, ao_blurred_view,
            noise_texture, noise_view,
            noise_sampler, gbuf_sampler,
            kernel_buffer, params_buffer,
            ssao_pipeline, ssao_gbuf_bgl, ssao_gbuf_bind_group,
            blur_pipeline,  blur_bgl,  blur_bind_group,
            apply_pipeline, apply_bgl, apply_bind_group,
            width, height,
        }
    }

    /// Recreate AO textures and rebind G-buffer views after a resize.
    pub fn resize(
        &mut self,
        device:   &wgpu::Device,
        deferred: &DeferredState,
        width:    u32,
        height:   u32,
    ) {
        let (ao_texture, ao_view)               = mk_ao_tex(device, width, height);
        let (ao_blurred_texture, ao_blurred_view) = mk_ao_tex(device, width, height);

        self.ssao_gbuf_bind_group = mk_ssao_gbuf_bg(
            device, &self.ssao_gbuf_bgl,
            &deferred.normal_roughness_view,
            &deferred.world_position_view,
            &self.noise_view, &self.gbuf_sampler, &self.noise_sampler, &self.kernel_buffer,
        );
        self.blur_bind_group  = mk_blur_bg(device, &self.blur_bgl, &ao_view, &self.gbuf_sampler);
        self.apply_bind_group = mk_apply_bg(device, &self.apply_bgl, &ao_blurred_view, &self.gbuf_sampler, &self.params_buffer);

        self.ao_texture          = ao_texture;
        self.ao_view             = ao_view;
        self.ao_blurred_texture  = ao_blurred_texture;
        self.ao_blurred_view     = ao_blurred_view;
        self.width  = width;
        self.height = height;
    }

    /// Update the AO strength parameter at runtime.
    pub fn set_strength(&self, queue: &wgpu::Queue, strength: f32) {
        queue.write_buffer(
            &self.params_buffer,
            0,
            bytemuck::bytes_of(&SsaoParams { strength, _pad: [0.0; 3] }),
        );
    }
}

// ── Kernel & noise generation ─────────────────────────────────────────────────

fn build_kernel() -> SsaoKernel {
    let mut state: u32 = 0xDEAD_BEEF;
    let mut rng = move || -> f32 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 9) as f32 / (u32::MAX >> 9) as f32
    };

    let mut kernel = SsaoKernel { samples: [[0.0; 4]; KERNEL_SIZE] };
    for i in 0..KERNEL_SIZE {
        // Uniform hemisphere sample (z > 0)
        let xi1 = rng() * 2.0 - 1.0;
        let xi2 = rng() * 2.0 - 1.0;
        let xi3 = rng();
        let len = (xi1 * xi1 + xi2 * xi2 + xi3 * xi3).sqrt().max(1e-6);
        let (x, y, z) = (xi1 / len, xi2 / len, xi3 / len);

        // Concentrate samples closer to origin (better close-range occlusion)
        let t = i as f32 / KERNEL_SIZE as f32;
        let scale = 0.1 + t * t * 0.9;

        kernel.samples[i] = [x * scale, y * scale, z.abs() * scale, 0.0];
    }
    kernel
}

fn build_noise_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::TextureView) {
    let mut state: u32 = 0xCAFE_BABE;
    let mut rng = move || -> f32 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 9) as f32 / (u32::MAX >> 9) as f32
    };

    let n = (NOISE_SIZE * NOISE_SIZE) as usize;
    let mut pixels: Vec<[u8; 4]> = Vec::with_capacity(n);
    for _ in 0..n {
        let angle = rng() * 2.0 * std::f32::consts::PI;
        let (c, s) = (angle.cos(), angle.sin());
        let r = ((c * 0.5 + 0.5) * 255.0) as u8;
        let g = ((s * 0.5 + 0.5) * 255.0) as u8;
        pixels.push([r, g, 128, 255]); // z=0.5 → decoded as 0 (not used)
    }
    let raw: Vec<u8> = pixels.into_iter().flatten().collect();

    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label:               Some("ssao_noise"),
        size:                wgpu::Extent3d { width: NOISE_SIZE, height: NOISE_SIZE, depth_or_array_layers: 1 },
        mip_level_count:     1,
        sample_count:        1,
        dimension:           wgpu::TextureDimension::D2,
        format:              wgpu::TextureFormat::Rgba8Unorm,
        usage:               wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats:        &[],
    });
    queue.write_texture(
        tex.as_image_copy(),
        &raw,
        wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(NOISE_SIZE * 4), rows_per_image: None },
        wgpu::Extent3d { width: NOISE_SIZE, height: NOISE_SIZE, depth_or_array_layers: 1 },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

// ── Texture helpers ───────────────────────────────────────────────────────────

fn mk_ao_tex(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label:               Some("ssao_ao"),
        size:                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count:     1,
        sample_count:        1,
        dimension:           wgpu::TextureDimension::D2,
        format:              wgpu::TextureFormat::Rgba8Unorm,
        usage:               wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats:        &[],
    });
    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
    (t, v)
}

// ── Bind group layouts ────────────────────────────────────────────────────────

fn mk_ssao_gbuf_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let tex_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
        },
        count: None,
    };
    let filterable_tex = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
        },
        count: None,
    };
    let sampler_entry = |binding: u32, ty: wgpu::SamplerBindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(ty),
        count: None,
    };
    let uniform_entry = |binding: u32, size: u64| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(size),
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ssao_gbuf_bgl"),
        entries: &[
            tex_entry(0),          // t_normal
            tex_entry(1),          // t_position
            filterable_tex(2),     // t_noise (filterable for repeat sampler)
            sampler_entry(3, wgpu::SamplerBindingType::NonFiltering),
            sampler_entry(4, wgpu::SamplerBindingType::Filtering),
            uniform_entry(5, std::mem::size_of::<SsaoKernel>() as u64),
        ],
    })
}

fn mk_blur_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ssao_blur_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                count: None,
            },
        ],
    })
}

fn mk_apply_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ssao_apply_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
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
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<SsaoParams>() as u64),
                },
                count: None,
            },
        ],
    })
}

// ── Bind group constructors ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn mk_ssao_gbuf_bg(
    device:        &wgpu::Device,
    layout:        &wgpu::BindGroupLayout,
    normal_view:   &wgpu::TextureView,
    position_view: &wgpu::TextureView,
    noise_view:    &wgpu::TextureView,
    gbuf_sampler:  &wgpu::Sampler,
    noise_sampler: &wgpu::Sampler,
    kernel_buf:    &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ssao_gbuf_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(normal_view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(position_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(noise_view) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(gbuf_sampler) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(noise_sampler) },
            wgpu::BindGroupEntry { binding: 5, resource: kernel_buf.as_entire_binding() },
        ],
    })
}

fn mk_blur_bg(
    device:   &wgpu::Device,
    layout:   &wgpu::BindGroupLayout,
    ao_view:  &wgpu::TextureView,
    sampler:  &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ssao_blur_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(ao_view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

fn mk_apply_bg(
    device:      &wgpu::Device,
    layout:      &wgpu::BindGroupLayout,
    ao_view:     &wgpu::TextureView,
    sampler:     &wgpu::Sampler,
    params_buf:  &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ssao_apply_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(ao_view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: params_buf.as_entire_binding() },
        ],
    })
}

// ── Pipeline constructors ─────────────────────────────────────────────────────

fn mk_ssao_pipeline(
    device:       &wgpu::Device,
    scene:        &SceneState,
    gbuf_bgl:     &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = load_shader(device, "demo/assets/shaders/ssao.wgsl",
        include_str!("shaders/ssao.wgsl"), "SSAO Shader");
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ssao_layout"),
        bind_group_layouts: &[&scene.global_bind_group_layout, gbuf_bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("ssao_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader, entry_point: "vs_main",
            compilation_options: Default::default(), buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader, entry_point: "fs_main",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive:    wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample:  wgpu::MultisampleState::default(),
        multiview:    None,
    })
}

fn mk_blur_pipeline(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout) -> wgpu::RenderPipeline {
    let shader = load_shader(device, "demo/assets/shaders/ssao_blur.wgsl",
        include_str!("shaders/ssao_blur.wgsl"), "SSAO Blur Shader");
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ssao_blur_layout"),
        bind_group_layouts: &[bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("ssao_blur_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader, entry_point: "vs_main",
            compilation_options: Default::default(), buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader, entry_point: "fs_main",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive:    wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample:  wgpu::MultisampleState::default(),
        multiview:    None,
    })
}

fn mk_apply_pipeline(device: &wgpu::Device, bgl: &wgpu::BindGroupLayout) -> wgpu::RenderPipeline {
    let shader = load_shader(device, "demo/assets/shaders/ssao_apply.wgsl",
        include_str!("shaders/ssao_apply.wgsl"), "SSAO Apply Shader");
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ssao_apply_layout"),
        bind_group_layouts: &[bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("ssao_apply_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader, entry_point: "vs_main",
            compilation_options: Default::default(), buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader, entry_point: "fs_main",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                // Multiply blend: hdr_final = hdr_existing * ao
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Dst,
                        dst_factor: wgpu::BlendFactor::Zero,
                        operation:  wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent::REPLACE,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive:    wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: None,
        multisample:  wgpu::MultisampleState::default(),
        multiview:    None,
    })
}
