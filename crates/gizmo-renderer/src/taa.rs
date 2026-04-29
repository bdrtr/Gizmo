use crate::pipeline::load_shader;

// ── GPU-side uniform ──────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct TaaParamsGpu {
    prev_view_proj: [[f32; 4]; 4],  // 64 bytes
    jitter:         [f32; 2],       // 8 bytes  (NDC-space subpixel offset)
    alpha:          f32,            // 4 bytes  (blend: 0=full history, 1=full current)
    _pad:           f32,            // 4 bytes
}

// ── TaaState ──────────────────────────────────────────────────────────────────

pub struct TaaState {
    // Ping-pong history buffers (Rgba16Float)
    history_a:      wgpu::Texture,
    history_a_view: wgpu::TextureView,
    history_b:      wgpu::Texture,
    history_b_view: wgpu::TextureView,

    // false → read A (history), write B (output)
    // true  → read B (history), write A (output)
    frame_parity: bool,
    pub frame_index: u32,

    // Unjittered view-proj from the previous frame (for reprojection)
    pub prev_vp: [[f32; 4]; 4],

    // Samplers and uniform buffer (never recreated on resize)
    params_buffer:  wgpu::Buffer,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,

    // Resolve pipeline: group 0 → { params, t_current, t_history, t_position, samplers }
    pub resolve_pipeline:   wgpu::RenderPipeline,
    resolve_bgl:            wgpu::BindGroupLayout,
    resolve_bg_read_a:      wgpu::BindGroup, // history=A → output=B
    resolve_bg_read_b:      wgpu::BindGroup, // history=B → output=A

    // Blit pipeline: group 0 empty, group 1 → { t_taa_out, s_blit }
    pub blit_pipeline:      wgpu::RenderPipeline,
    blit_bgl:               wgpu::BindGroupLayout,
    blit_bg_a:              wgpu::BindGroup, // reads A (used when parity=true, A is output)
    blit_bg_b:              wgpu::BindGroup, // reads B (used when parity=false, B is output)
    empty_bgl:              wgpu::BindGroupLayout,
    pub empty_bg:           wgpu::BindGroup,

    pub width:  u32,
    pub height: u32,
}

impl TaaState {
    pub fn new(
        device:        &wgpu::Device,
        hdr_view:      &wgpu::TextureView,
        position_view: &wgpu::TextureView,
        width:         u32,
        height:        u32,
    ) -> Self {
        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label:              Some("taa_params"),
            size:               std::mem::size_of::<TaaParamsGpu>() as u64,
            usage:              wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Linear,
            min_filter:     wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter:     wgpu::FilterMode::Nearest,
            min_filter:     wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let (history_a, history_a_view) = mk_history_tex(device, width, height);
        let (history_b, history_b_view) = mk_history_tex(device, width, height);

        let resolve_bgl = mk_resolve_bgl(device);
        let blit_bgl    = mk_blit_bgl(device);
        let empty_bgl   = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label:   Some("taa_empty_bgl"),
            entries: &[],
        });
        let empty_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label:   Some("taa_empty_bg"),
            layout:  &empty_bgl,
            entries: &[],
        });

        let resolve_bg_read_a = mk_resolve_bg(
            device, &resolve_bgl, &params_buffer,
            hdr_view, &history_a_view, position_view,
            &linear_sampler, &nearest_sampler,
        );
        let resolve_bg_read_b = mk_resolve_bg(
            device, &resolve_bgl, &params_buffer,
            hdr_view, &history_b_view, position_view,
            &linear_sampler, &nearest_sampler,
        );
        let blit_bg_a = mk_blit_bg(device, &blit_bgl, &history_a_view, &nearest_sampler);
        let blit_bg_b = mk_blit_bg(device, &blit_bgl, &history_b_view, &nearest_sampler);

        let resolve_pipeline = mk_resolve_pipeline(device, &resolve_bgl);
        let blit_pipeline    = mk_blit_pipeline(device, &empty_bgl, &blit_bgl);

        let identity: [[f32; 4]; 4] = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];

        Self {
            history_a, history_a_view,
            history_b, history_b_view,
            frame_parity: false,
            frame_index:  0,
            prev_vp:      identity,
            params_buffer,
            linear_sampler, nearest_sampler,
            resolve_pipeline, resolve_bgl,
            resolve_bg_read_a, resolve_bg_read_b,
            blit_pipeline, blit_bgl,
            blit_bg_a, blit_bg_b,
            empty_bgl, empty_bg,
            width, height,
        }
    }

    /// Recreate history textures and rebind after a window resize.
    pub fn resize(
        &mut self,
        device:        &wgpu::Device,
        hdr_view:      &wgpu::TextureView,
        position_view: &wgpu::TextureView,
        width:         u32,
        height:        u32,
    ) {
        let (history_a, history_a_view) = mk_history_tex(device, width, height);
        let (history_b, history_b_view) = mk_history_tex(device, width, height);

        self.resolve_bg_read_a = mk_resolve_bg(
            device, &self.resolve_bgl, &self.params_buffer,
            hdr_view, &history_a_view, position_view,
            &self.linear_sampler, &self.nearest_sampler,
        );
        self.resolve_bg_read_b = mk_resolve_bg(
            device, &self.resolve_bgl, &self.params_buffer,
            hdr_view, &history_b_view, position_view,
            &self.linear_sampler, &self.nearest_sampler,
        );
        self.blit_bg_a = mk_blit_bg(device, &self.blit_bgl, &history_a_view, &self.nearest_sampler);
        self.blit_bg_b = mk_blit_bg(device, &self.blit_bgl, &history_b_view, &self.nearest_sampler);

        self.history_a      = history_a;
        self.history_a_view = history_a_view;
        self.history_b      = history_b;
        self.history_b_view = history_b_view;
        self.width  = width;
        self.height = height;
        // Reset parity so stale history doesn't ghost after resize
        self.frame_parity = false;
        self.frame_index  = 0;
    }

    /// Halton sequence jitter: pixel offsets in [−0.5, 0.5] range.
    /// 8-frame repeating sequence using base-2 (x) and base-3 (y).
    pub fn get_jitter(frame: u32) -> [f32; 2] {
        let i = (frame % 8) + 1;
        [halton(i, 2) - 0.5, halton(i, 3) - 0.5]
    }

    /// Upload TaaParams uniform: uses self.prev_vp for reprojection.
    pub fn update_params(&self, queue: &wgpu::Queue, jitter: [f32; 2], alpha: f32) {
        let data = TaaParamsGpu {
            prev_view_proj: self.prev_vp,
            jitter,
            alpha,
            _pad: 0.0,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&data));
    }

    /// Store the current frame's unjittered view_proj for use next frame.
    pub fn store_prev_vp(&mut self, vp: [[f32; 4]; 4]) {
        self.prev_vp = vp;
    }

    /// Advance ping-pong and frame counter (call after both resolve and blit passes).
    pub fn advance_frame(&mut self) {
        self.frame_parity = !self.frame_parity;
        self.frame_index  = self.frame_index.wrapping_add(1);
    }

    /// Returns (resolve_bind_group, output_history_view) for the current frame.
    pub fn current_resolve_inputs_output(&self) -> (&wgpu::BindGroup, &wgpu::TextureView) {
        if !self.frame_parity {
            (&self.resolve_bg_read_a, &self.history_b_view)
        } else {
            (&self.resolve_bg_read_b, &self.history_a_view)
        }
    }

    /// Returns the blit bind group that reads this frame's resolve output.
    pub fn current_blit_bg(&self) -> &wgpu::BindGroup {
        if !self.frame_parity {
            &self.blit_bg_b  // B was just written
        } else {
            &self.blit_bg_a  // A was just written
        }
    }
}

// ── Halton sequence ───────────────────────────────────────────────────────────

fn halton(mut i: u32, base: u32) -> f32 {
    let mut result = 0.0f32;
    let mut f      = 1.0f32 / base as f32;
    while i > 0 {
        result += f * (i % base) as f32;
        i      /= base;
        f      /= base as f32;
    }
    result
}

// ── Texture helpers ───────────────────────────────────────────────────────────

fn mk_history_tex(device: &wgpu::Device, w: u32, h: u32) -> (wgpu::Texture, wgpu::TextureView) {
    let t = device.create_texture(&wgpu::TextureDescriptor {
        label:               Some("taa_history"),
        size:                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count:     1,
        sample_count:        1,
        dimension:           wgpu::TextureDimension::D2,
        format:              wgpu::TextureFormat::Rgba16Float,
        usage:               wgpu::TextureUsages::RENDER_ATTACHMENT
                           | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats:        &[],
    });
    let v = t.create_view(&wgpu::TextureViewDescriptor::default());
    (t, v)
}

// ── Bind group layouts ────────────────────────────────────────────────────────

fn mk_resolve_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let tex_nf = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled:  false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type:   wgpu::TextureSampleType::Float { filterable: false },
        },
        count: None,
    };
    let tex_f = |binding: u32| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled:  false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type:   wgpu::TextureSampleType::Float { filterable: true },
        },
        count: None,
    };
    let sampler = |binding: u32, ty: wgpu::SamplerBindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty:         wgpu::BindingType::Sampler(ty),
        count:      None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("taa_resolve_bgl"),
        entries: &[
            // binding 0: TaaParams uniform
            wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Buffer {
                    ty:                 wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size:   wgpu::BufferSize::new(
                        std::mem::size_of::<TaaParamsGpu>() as u64,
                    ),
                },
                count: None,
            },
            tex_nf(1),  // t_current  (HDR, textureLoad)
            tex_f(2),   // t_history  (history, textureSample bilinear)
            tex_nf(3),  // t_position (G-buffer, textureLoad)
            sampler(4, wgpu::SamplerBindingType::Filtering),    // s_linear
            sampler(5, wgpu::SamplerBindingType::NonFiltering), // s_nearest
        ],
    })
}

fn mk_blit_bgl(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("taa_blit_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding:    0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Texture {
                    multisampled:  false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type:   wgpu::TextureSampleType::Float { filterable: false },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding:    1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty:         wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                count:      None,
            },
        ],
    })
}

// ── Bind group constructors ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn mk_resolve_bg(
    device:         &wgpu::Device,
    layout:         &wgpu::BindGroupLayout,
    params_buf:     &wgpu::Buffer,
    current_view:   &wgpu::TextureView,
    history_view:   &wgpu::TextureView,
    position_view:  &wgpu::TextureView,
    linear_sampler: &wgpu::Sampler,
    nearest_sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("taa_resolve_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: params_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(current_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(history_view) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(position_view) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(linear_sampler) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(nearest_sampler) },
        ],
    })
}

fn mk_blit_bg(
    device:  &wgpu::Device,
    layout:  &wgpu::BindGroupLayout,
    taa_out: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("taa_blit_bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(taa_out) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
        ],
    })
}

// ── Pipeline constructors ─────────────────────────────────────────────────────

fn mk_resolve_pipeline(
    device:      &wgpu::Device,
    resolve_bgl: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = load_shader(
        device,
        "demo/assets/shaders/taa.wgsl",
        include_str!("shaders/taa.wgsl"),
        "TAA Shader",
    );
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:                Some("taa_resolve_layout"),
        bind_group_layouts:   &[resolve_bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("taa_resolve_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module:               &shader,
            entry_point:          "vs_main",
            compilation_options:  Default::default(),
            buffers:              &[],
        },
        fragment: Some(wgpu::FragmentState {
            module:              &shader,
            entry_point:         "fs_resolve",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format:     wgpu::TextureFormat::Rgba16Float,
                blend:      None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive:     wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample:   wgpu::MultisampleState::default(),
        multiview:     None,
    })
}

fn mk_blit_pipeline(
    device:    &wgpu::Device,
    empty_bgl: &wgpu::BindGroupLayout,
    blit_bgl:  &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = load_shader(
        device,
        "demo/assets/shaders/taa.wgsl",
        include_str!("shaders/taa.wgsl"),
        "TAA Blit Shader",
    );
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label:                Some("taa_blit_layout"),
        bind_group_layouts:   &[empty_bgl, blit_bgl],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label:  Some("taa_blit_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module:              &shader,
            entry_point:         "vs_main",
            compilation_options: Default::default(),
            buffers:             &[],
        },
        fragment: Some(wgpu::FragmentState {
            module:              &shader,
            entry_point:         "fs_blit",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format:     wgpu::TextureFormat::Rgba16Float,
                blend:      None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive:     wgpu::PrimitiveState {
            topology:  wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample:   wgpu::MultisampleState::default(),
        multiview:     None,
    })
}
