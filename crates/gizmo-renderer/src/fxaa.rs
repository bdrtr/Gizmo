//! FXAA (Fast Approximate Anti-Aliasing) — Post-Processing Pass
//!
//! Timothy Lottes FXAA 3.11 Quality implementasyonu.
//! Composite + Tone Mapping sonrasında son pass olarak çalışır.
//! Kenar tespiti luma tabanlıdır — düşük GPU maliyetli kenar yumuşatma sağlar.

use wgpu::util::DeviceExt;

/// FXAA GPU kaynakları
pub struct FxaaState {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub bind_group: wgpu::BindGroup,
    pub params_buffer: wgpu::Buffer,
    pub input_texture: wgpu::Texture,
    pub input_texture_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub enabled: bool,
}

/// FXAA shader'ına gönderilen parametreler
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct FxaaParams {
    pub inv_screen_size: [f32; 2], // 1.0 / vec2(width, height)
    pub fxaa_enabled: f32,         // 1.0 = açık, 0.0 = kapalı
    pub _padding: f32,
}

impl FxaaState {
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Self {
        // Shader yükleme
        let shader = {
            #[cfg(not(target_arch = "wasm32"))]
            let source = std::fs::read_to_string("demo/assets/shaders/fxaa.wgsl")
                .unwrap_or_else(|_| include_str!("shaders/fxaa.wgsl").to_string());

            #[cfg(target_arch = "wasm32")]
            let source = include_str!("shaders/fxaa.wgsl").to_string();

            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("FXAA Shader"),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            })
        };

        // Sampler — bilinear filtering
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("FXAA Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Input texture (composite pass buraya yazar, FXAA buradan okur)
        let (input_texture, input_texture_view) =
            create_fxaa_texture(device, surface_format, width, height);

        // Parametreler
        let params = FxaaParams {
            inv_screen_size: [1.0 / width as f32, 1.0 / height as f32],
            fxaa_enabled: 1.0,
            _padding: 0.0,
        };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FXAA Params Buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Bind group layout
        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("FXAA Bind Group Layout"),
                entries: &[
                    // @binding(0) — input texture
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
                    // @binding(1) — sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // @binding(2) — params uniform
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        // Bind group
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FXAA Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&input_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        // Pipeline
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("FXAA Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("FXAA Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,

        });

        Self {
            pipeline,
            bind_group_layout,
            bind_group,
            params_buffer,
            input_texture,
            input_texture_view,
            sampler,
            enabled: true,
        }
    }

    /// Pencere boyutu değiştiğinde texture'ları yeniden oluştur
    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, surface_format: wgpu::TextureFormat, width: u32, height: u32) {
        let (tex, view) = create_fxaa_texture(device, surface_format, width, height);
        self.input_texture = tex;
        self.input_texture_view = view;

        // Params güncelle
        let params = FxaaParams {
            inv_screen_size: [1.0 / width as f32, 1.0 / height as f32],
            fxaa_enabled: if self.enabled { 1.0 } else { 0.0 },
            _padding: 0.0,
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::cast_slice(&[params]));

        // Bind group yeniden oluştur (yeni texture view)
        self.bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("FXAA Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.input_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.params_buffer.as_entire_binding(),
                },
            ],
        });
    }

    /// FXAA'yı aç/kapat
    pub fn set_enabled(&mut self, queue: &wgpu::Queue, enabled: bool) {
        self.enabled = enabled;
        let params = FxaaParams {
            inv_screen_size: [0.0, 0.0], // Resize'da güncellenir
            fxaa_enabled: if enabled { 1.0 } else { 0.0 },
            _padding: 0.0,
        };
        // Sadece enabled alanını güncelle
        queue.write_buffer(
            &self.params_buffer,
            8, // offset: inv_screen_size(8 byte) sonrası
            bytemuck::cast_slice(&[params.fxaa_enabled]),
        );
    }
}

/// FXAA render pass'ını çalıştırır
/// `input_view` → composite çıktısı, `output_view` → ekran/swapchain
pub fn run_fxaa_pass(
    fxaa: &FxaaState,
    encoder: &mut wgpu::CommandEncoder,
    output_view: &wgpu::TextureView,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("FXAA Pass"),
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
    pass.set_pipeline(&fxaa.pipeline);
    pass.set_bind_group(0, &fxaa.bind_group, &[]);
    pass.draw(0..3, 0..1);
}

/// FXAA input texture oluşturur (composite pass buraya yazar)
fn create_fxaa_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("FXAA Input Texture"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}
