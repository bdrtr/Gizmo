use crate::gpu_types::PostProcessUniforms;
use wgpu::util::DeviceExt;

/// Post-Processing durumu — HDR, Bloom, Blur, Composite pipeline'ları ve kaynakları
pub struct PostProcessState {
    pub post_bind_group_layout: wgpu::BindGroupLayout,
    pub blur_params_bind_group_layout: wgpu::BindGroupLayout,
    pub composite_bloom_bind_group_layout: wgpu::BindGroupLayout,
    pub post_params_buffer: wgpu::Buffer,
    pub post_params_bind_group_layout: wgpu::BindGroupLayout,
    pub post_params_bind_group: wgpu::BindGroup,
    pub bloom_extract_pipeline: wgpu::RenderPipeline,
    pub bloom_blur_pipeline: wgpu::RenderPipeline,
    pub composite_pipeline: wgpu::RenderPipeline,
    pub hdr_texture: wgpu::Texture,
    pub hdr_texture_view: wgpu::TextureView,
    pub hdr_bind_group: wgpu::BindGroup,
    pub bloom_extract_texture_view: wgpu::TextureView,
    pub bloom_extract_bind_group: wgpu::BindGroup,
    pub bloom_blur_texture_view: wgpu::TextureView,
    pub bloom_blur_bind_group: wgpu::BindGroup,
    pub composite_bloom_bind_group: wgpu::BindGroup,
    pub blur_params_buffer: wgpu::Buffer,
    pub blur_h_bind_group: wgpu::BindGroup,
    pub blur_v_bind_group: wgpu::BindGroup,
}

pub fn build_post_process_resources(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    depth_view: &wgpu::TextureView,
) -> PostProcessState {
    let post_shader = {
        let source = std::fs::read_to_string("demo/assets/shaders/post_process.wgsl")
            .unwrap_or_else(|_| include_str!("shaders/post_process.wgsl").to_string());
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Post-Processing Shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
    };

    let post_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post_bind_group_layout"),
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

    let blur_params_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("blur_params_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

    let composite_bloom_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("composite_bloom_bind_group_layout"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
            ],
        });

    let post_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Post Process Params Buffer"),
        contents: bytemuck::cast_slice(&[PostProcessUniforms {
            bloom_intensity: 0.8, // Daha belirgin ve hacimli parlama
            bloom_threshold: 0.85, // Daha çok highlight yakalamak için eşik düşürüldü
            exposure: 1.15, // ACES Tone mapping'in renkleri daha canlı sunması için pozlama artırıldı
            chromatic_aberration: 0.35, // Sinematik lens hissi için ufak renk sapması
            vignette_intensity: 0.25, // Köşelerde dramatik kararma (Vignette)
            film_grain_intensity: 0.03, // Film greni (Realistic noise)
            dof_focus_dist: 15.0,
            dof_focus_range: 25.0,
            dof_blur_size: 2.0, // Daha yumuşak odak derinliği (DoF)
            _padding: [0.0; 3],
        }]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let post_params_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post_params_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

    let post_params_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &post_params_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: post_params_buffer.as_entire_binding(),
        }],
        label: Some("post_params_bind_group"),
    });

    let (bloom_extract_pipeline, bloom_blur_pipeline, composite_pipeline) = build_post_pipelines(
        device,
        &post_shader,
        &post_bind_group_layout,
        &blur_params_bind_group_layout,
        &composite_bloom_bind_group_layout,
        &post_params_bind_group_layout,
        surface_format,
    );

    let post_sampler = create_post_sampler(device);
    let (
        hdr_texture,
        hdr_texture_view,
        hdr_bind_group,
        bloom_extract_texture_view,
        bloom_extract_bind_group,
        bloom_blur_texture_view,
        bloom_blur_bind_group,
        composite_bloom_bind_group,
    ) = create_post_textures(
        device,
        &post_bind_group_layout,
        &composite_bloom_bind_group_layout,
        &post_sampler,
        width,
        height,
        depth_view,
    );

    let (blur_params_buffer, blur_h_bind_group, blur_v_bind_group) =
        create_blur_buffers(device, &blur_params_bind_group_layout, width, height);

    PostProcessState {
        post_bind_group_layout,
        blur_params_bind_group_layout,
        composite_bloom_bind_group_layout,
        post_params_buffer,
        post_params_bind_group_layout,
        post_params_bind_group,
        bloom_extract_pipeline,
        bloom_blur_pipeline,
        composite_pipeline,
        hdr_texture,
        hdr_texture_view,
        hdr_bind_group,
        bloom_extract_texture_view,
        bloom_extract_bind_group,
        bloom_blur_texture_view,
        bloom_blur_bind_group,
        composite_bloom_bind_group,
        blur_params_buffer,
        blur_h_bind_group,
        blur_v_bind_group,
    }
}

fn build_post_pipelines(
    device: &wgpu::Device,
    post_shader: &wgpu::ShaderModule,
    post_bgl: &wgpu::BindGroupLayout,
    blur_bgl: &wgpu::BindGroupLayout,
    composite_bloom_bgl: &wgpu::BindGroupLayout,
    post_params_bgl: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
) -> (
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
    wgpu::RenderPipeline,
) {
    let extract_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Bloom Extract Pipeline Layout"),
        bind_group_layouts: &[post_bgl, blur_bgl, post_params_bgl],
        push_constant_ranges: &[],
    });
    let bloom_extract_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Bloom Extract Pipeline"),
        layout: Some(&extract_layout),
        vertex: wgpu::VertexState {
            module: post_shader,
            entry_point: "vs_fullscreen",
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: post_shader,
            entry_point: "fs_bright_extract",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    let blur_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Bloom Blur Pipeline Layout"),
        bind_group_layouts: &[post_bgl, blur_bgl],
        push_constant_ranges: &[],
    });
    let bloom_blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Bloom Blur Pipeline"),
        layout: Some(&blur_layout),
        vertex: wgpu::VertexState {
            module: post_shader,
            entry_point: "vs_fullscreen",
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: post_shader,
            entry_point: "fs_blur",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    let composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Composite Pipeline Layout"),
        bind_group_layouts: &[post_bgl, composite_bloom_bgl, post_params_bgl],
        push_constant_ranges: &[],
    });
    let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Composite Pipeline"),
        layout: Some(&composite_layout),
        vertex: wgpu::VertexState {
            module: post_shader,
            entry_point: "vs_fullscreen",
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: post_shader,
            entry_point: "fs_composite",
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    (
        bloom_extract_pipeline,
        bloom_blur_pipeline,
        composite_pipeline,
    )
}

pub fn rebuild_post_pipelines(renderer: &mut crate::Renderer, post_shader: &wgpu::ShaderModule) {
    let (e, b, c) = build_post_pipelines(
        &renderer.device,
        post_shader,
        &renderer.post.post_bind_group_layout,
        &renderer.post.blur_params_bind_group_layout,
        &renderer.post.composite_bloom_bind_group_layout,
        &renderer.post.post_params_bind_group_layout,
        renderer.config.format,
    );
    renderer.post.bloom_extract_pipeline = e;
    renderer.post.bloom_blur_pipeline = b;
    renderer.post.composite_pipeline = c;
}

fn create_post_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

pub fn create_post_textures(
    device: &wgpu::Device,
    post_bgl: &wgpu::BindGroupLayout,
    composite_bloom_bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    width: u32,
    height: u32,
    depth_view: &wgpu::TextureView,
) -> (
    wgpu::Texture,
    wgpu::TextureView,
    wgpu::BindGroup,
    wgpu::TextureView,
    wgpu::BindGroup,
    wgpu::TextureView,
    wgpu::BindGroup,
    wgpu::BindGroup,
) {
    let make = |label: &str| -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: post_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
            label: Some(&format!("{}_bind_group", label)),
        });
        (tex, view, bg)
    };

    let (hdr_t, hdr_v, hdr_bg) = make("HDR Texture");
    let (_be_t, be_v, be_bg) = make("Bloom Extract Texture");
    let (_bb_t, bb_v, bb_bg) = make("Bloom Blur Texture");

    let cb_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: composite_bloom_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&be_v),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(depth_view),
            },
        ],
        label: Some("composite_bloom_bind_group"),
    });

    (hdr_t, hdr_v, hdr_bg, be_v, be_bg, bb_v, bb_bg, cb_bg)
}

pub fn create_blur_buffers(
    device: &wgpu::Device,
    bgl: &wgpu::BindGroupLayout,
    width: u32,
    height: u32,
) -> (wgpu::Buffer, wgpu::BindGroup, wgpu::BindGroup) {
    let h_data: [f32; 4] = [1.0 / width as f32, 0.0, 0.0, 0.0];
    let v_data: [f32; 4] = [0.0, 1.0 / height as f32, 0.0, 0.0];

    let h_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Blur H Params"),
        contents: bytemuck::cast_slice(&h_data),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let v_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Blur V Params"),
        contents: bytemuck::cast_slice(&v_data),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let h_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: h_buf.as_entire_binding(),
        }],
        label: Some("blur_h_bind_group"),
    });
    let v_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: v_buf.as_entire_binding(),
        }],
        label: Some("blur_v_bind_group"),
    });
    (h_buf, h_bg, v_bg)
}

/// Post-processing render geçişlerini sırayla çalıştırır.
pub fn run_post_processing(
    renderer: &crate::Renderer,
    encoder: &mut wgpu::CommandEncoder,
    output_view: &wgpu::TextureView,
) {
    // Pass 1: Bright Extract
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Bloom Extract Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.bloom_extract_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(&renderer.post.bloom_extract_pipeline);
        pass.set_bind_group(0, &renderer.post.hdr_bind_group, &[]);
        pass.set_bind_group(1, &renderer.post.blur_h_bind_group, &[]);
        pass.set_bind_group(2, &renderer.post.post_params_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    // Pass 2a: Yatay Blur
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Bloom Blur Horizontal"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.bloom_blur_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(&renderer.post.bloom_blur_pipeline);
        pass.set_bind_group(0, &renderer.post.bloom_extract_bind_group, &[]);
        pass.set_bind_group(1, &renderer.post.blur_h_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    // Pass 2b: Dikey Blur (ping-pong)
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Bloom Blur Vertical"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.bloom_extract_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(&renderer.post.bloom_blur_pipeline);
        pass.set_bind_group(0, &renderer.post.bloom_blur_bind_group, &[]);
        pass.set_bind_group(1, &renderer.post.blur_v_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    // Pass 3: Composite + Tone Mapping
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Composite + Tone Mapping Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            ..Default::default()
        });
        pass.set_pipeline(&renderer.post.composite_pipeline);
        pass.set_bind_group(0, &renderer.post.hdr_bind_group, &[]);
        pass.set_bind_group(1, &renderer.post.composite_bloom_bind_group, &[]);
        pass.set_bind_group(2, &renderer.post.post_params_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
