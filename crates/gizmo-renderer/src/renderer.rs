use bytemuck::{Pod, Zeroable};
use wgpu::{util::DeviceExt, Device, Queue, Surface, SurfaceConfiguration};
use winit::window::Window;
use std::sync::Arc;

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
    pub normal: [f32; 3],
    pub tex_coords: [f32; 2],
    pub joint_indices: [u32; 4],
    pub joint_weights: [f32; 4],
}

impl Vertex {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 6]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x3, // Normaller
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 9]>() as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x2, // Kaplama (Texture) Koordinatları (U,V)
                },
                wgpu::VertexAttribute {
                    offset: std::mem::size_of::<[f32; 11]>() as wgpu::BufferAddress,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Uint32x4, // Joint Indices (u32 array, WGPU treats as Uint32x4)
                },
                wgpu::VertexAttribute {
                    offset: (std::mem::size_of::<[f32; 11]>() + std::mem::size_of::<[u32; 4]>()) as wgpu::BufferAddress,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4, // Joint Weights
                },
            ],
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct LightData {
    pub position: [f32; 4], // xyz: pozisyon, w: şiddet (intensity)
    pub color: [f32; 4],    // xyz: renk, w: boş
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SceneUniforms {
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    pub sun_direction: [f32; 4], // xyz: yon, w: aktif mi (0 veya 1)
    pub sun_color: [f32; 4],     // xyz: renk, w: sidet
    pub lights: [LightData; 10], // Maksimum 10 ışık
    pub light_view_proj: [[f32; 4]; 4],
    pub num_lights: u32,
    pub _padding: [u32; 3], // 16 byte hezalanmak için
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct InstanceRaw {
    pub model: [[f32; 4]; 4],
    pub albedo_color: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
    pub _padding: f32,
}

impl InstanceRaw {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<InstanceRaw>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute { offset: 0, shader_location: 6, format: wgpu::VertexFormat::Float32x4 },
                wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 4]>() as wgpu::BufferAddress, shader_location: 7, format: wgpu::VertexFormat::Float32x4 },
                wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 8]>() as wgpu::BufferAddress, shader_location: 8, format: wgpu::VertexFormat::Float32x4 },
                wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 12]>() as wgpu::BufferAddress, shader_location: 9, format: wgpu::VertexFormat::Float32x4 },
                wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 16]>() as wgpu::BufferAddress, shader_location: 10, format: wgpu::VertexFormat::Float32x4 },
                wgpu::VertexAttribute { offset: std::mem::size_of::<[f32; 20]>() as wgpu::BufferAddress, shader_location: 11, format: wgpu::VertexFormat::Float32x4 },
            ],
        }
    }
}

pub struct Renderer<'a> {
    pub surface: Surface<'a>,
    pub device: Device,
    pub queue: Queue,
    pub config: SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
    pub render_pipeline: wgpu::RenderPipeline,
    pub shadow_pipeline: wgpu::RenderPipeline,
    pub shadow_texture_view: wgpu::TextureView,
    pub global_uniform_buffer: wgpu::Buffer,
    pub global_bind_group_layout: wgpu::BindGroupLayout,
    pub global_bind_group: wgpu::BindGroup,
    
    pub shadow_bind_group_layout: wgpu::BindGroupLayout,
    pub shadow_bind_group: wgpu::BindGroup,
    
    // Shaderda Texture için hazırladığımız Blueprint
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    
    // Dünyayı 3 Boyutlu olarak doğru örtüştürmek için Depth Buffer (Üçgenler birbirine geçmesin diye)
    pub depth_texture_view: wgpu::TextureView,

    // Kemik / İskelet Animasyonu İçin
    pub skeleton_bind_group_layout: wgpu::BindGroupLayout,
    pub dummy_skeleton_bind_group: Arc<wgpu::BindGroup>,
    
    // === POST-PROCESSING ===
    // HDR Offscreen Render Target (Sahne buraya çizilir)
    pub hdr_texture_view: wgpu::TextureView,
    pub hdr_bind_group: wgpu::BindGroup,
    
    // Bloom Texture'ları (Parlak piksel ayıklama ve blur)
    pub bloom_extract_texture_view: wgpu::TextureView,
    pub bloom_extract_bind_group: wgpu::BindGroup,
    pub bloom_blur_texture_view: wgpu::TextureView,
    pub bloom_blur_bind_group: wgpu::BindGroup,
    
    // Post-Processing Pipeline'ları
    pub post_bind_group_layout: wgpu::BindGroupLayout,
    pub bloom_extract_pipeline: wgpu::RenderPipeline,
    pub bloom_blur_pipeline: wgpu::RenderPipeline,
    pub composite_pipeline: wgpu::RenderPipeline,
    
    // Blur yönünü belirleyen uniform buffer
    pub blur_params_buffer: wgpu::Buffer,
    pub blur_params_bind_group_layout: wgpu::BindGroupLayout,
    pub blur_h_bind_group: wgpu::BindGroup,
    pub blur_v_bind_group: wgpu::BindGroup,
    
    // Composite pass: bloom texture'ını bağlayan bind group
    pub composite_bloom_bind_group_layout: wgpu::BindGroupLayout,
    pub composite_bloom_bind_group: wgpu::BindGroup,
}

impl<'a> Renderer<'a> {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).expect("Surface error");

        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }).await.unwrap();

        let (device, queue) = adapter.request_device(
            &wgpu::DeviceDescriptor {
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                label: None,
            },
            None,
        ).await.unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo, // Sabit ve pürüzsüz 60/144hz için VSync aktif edildi
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // -- DEPTH (Derinlik Z-Buffer) Yaratılımı --
        let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size: wgpu::Extent3d { width: config.width, height: config.height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_texture_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // -- GPU Hafızasında UNIFORMS (Kamera ve Işık) --
        let initial_scene_uniforms = SceneUniforms {
            view_proj: [[0.0; 4]; 4],
            camera_pos: [0.0; 4],
            sun_direction: [0.0, -1.0, 0.0, 0.0],
            sun_color: [1.0, 1.0, 1.0, 0.0],
            lights: [LightData { position: [0.0; 4], color: [0.0; 4] }; 10],
            light_view_proj: [[0.0; 4]; 4],
            num_lights: 0,
            _padding: [0; 3],
        };

        let global_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Global Uniform Buffer"),
            contents: bytemuck::cast_slice(&[initial_scene_uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // -- Shadow Depth Texture --
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            size: wgpu::Extent3d { width: 2048, height: 2048, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            label: Some("shadow_texture"),
            view_formats: &[],
        });
        let shadow_texture_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        // WGSL Group 0: Kamera ve Işık Uniformu (Global)
        let global_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry { // Uniform
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
            label: Some("global_bind_group_layout"),
        });

        let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &global_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: global_uniform_buffer.as_entire_binding(),
                },
            ],
            label: Some("global_bind_group"),
        });

        // WGSL Group 3: Shadow Map ve Sampler
        let shadow_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry { // Shadow Texture
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { // Shadow Sampler
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
            label: Some("shadow_bind_group_layout"),
        });

        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &shadow_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shadow_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
            label: Some("shadow_bind_group"),
        });

        // WGSL Group 1: Texture (Resim) Şablonu
        let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            label: Some("texture_bind_group_layout"),
        });

        // -- Shader ve Pipeline Kurulumu --
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // WGSL Group 3: Skeleton Data
        let skeleton_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX, // Vertex shader içinde skinning yapacağız
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }
            ],
            label: Some("skeleton_bind_group_layout"),
        });

        // Dummy iskelet (Hiç animasyonu olmayan normal Mesh'ler buraya bağlanacak - 64 tane mat4 = 4096 bytes)
        let dummy_skeleton_data = [[[0.0f32; 4]; 4]; 64]; 
        let dummy_skeleton_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Dummy Skeleton Buffer"),
            contents: bytemuck::cast_slice(&dummy_skeleton_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let dummy_skeleton_bind_group = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &skeleton_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: dummy_skeleton_buffer.as_entire_binding(),
                }
            ],
            label: Some("dummy_skeleton_bind_group"),
        }));

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[
                &global_bind_group_layout,    // @group(0)
                &texture_bind_group_layout,   // @group(1)
                &shadow_bind_group_layout,    // @group(2)
                &skeleton_bind_group_layout,  // @group(3)
            ],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[Vertex::desc(), InstanceRaw::desc()], 
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float, // HDR çıktı için 16-bit float
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // Görünmezlik hatalarını denetlemek için Culling kapatıldı
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
        });

        // -- Shadow Pipeline Kurulumu --
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shadow Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shadow.wgsl").into()),
        });

        let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shadow Pipeline Layout"),
            bind_group_layouts: &[
                &global_bind_group_layout,    // @group(0)
                &skeleton_bind_group_layout,  // @group(1)
            ],
            push_constant_ranges: &[],
        });

        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Shadow Pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: "vs_main",
                buffers: &[Vertex::desc(), InstanceRaw::desc()],
            },
            fragment: None, // Shadow pass sadece Depth'e yazar, Color target yoktur
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back), // Front-face/back-face duruma göre peter panning azaltır
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2, // Slope-scaled depth bias (Z-fighting'i engeller)
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // === POST-PROCESSING ALTYAPISI ===
        let post_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Post-Processing Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post_process.wgsl").into()),
        });

        // Texture sampling bind group layout (tüm post-processing geçişleri için ortak)
        let post_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            label: Some("post_bind_group_layout"),
        });

        // Blur parametreleri bind group layout
        let blur_params_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            ],
            label: Some("blur_params_bind_group_layout"),
        });

        // Composite bloom texture bind group layout (bloom texture sampling)
        let composite_bloom_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            label: Some("composite_bloom_bind_group_layout"),
        });

        // -- Bloom Extract Pipeline --
        let bloom_extract_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Bloom Extract Pipeline Layout"),
            bind_group_layouts: &[&post_bind_group_layout],
            push_constant_ranges: &[],
        });
        let bloom_extract_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Bloom Extract Pipeline"),
            layout: Some(&bloom_extract_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &post_shader,
                entry_point: "vs_fullscreen",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &post_shader,
                entry_point: "fs_bright_extract",
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

        // -- Bloom Blur Pipeline --
        let bloom_blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Bloom Blur Pipeline Layout"),
            bind_group_layouts: &[&post_bind_group_layout, &blur_params_bind_group_layout],
            push_constant_ranges: &[],
        });
        let bloom_blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Bloom Blur Pipeline"),
            layout: Some(&bloom_blur_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &post_shader,
                entry_point: "vs_fullscreen",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &post_shader,
                entry_point: "fs_blur",
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

        // -- Composite Pipeline (HDR + Bloom → LDR Ekran) --
        let composite_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Composite Pipeline Layout"),
            bind_group_layouts: &[&post_bind_group_layout, &composite_bloom_bind_group_layout],
            push_constant_ranges: &[],
        });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Composite Pipeline"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &post_shader,
                entry_point: "vs_fullscreen",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &post_shader,
                entry_point: "fs_composite",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format, // Son geçiş ekranın sRGB formatına yazar
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

        // Post-processing texture ve bind group'ları oluştur
        let post_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let (hdr_texture_view, hdr_bind_group,
             bloom_extract_texture_view, bloom_extract_bind_group,
             bloom_blur_texture_view, bloom_blur_bind_group,
             composite_bloom_bind_group) = Self::create_post_textures(
            &device, &post_bind_group_layout, &composite_bloom_bind_group_layout,
            &post_sampler, config.width, config.height,
        );

        // Blur parametreleri (yatay ve dikey)
        let blur_h_data: [f32; 4] = [1.0 / config.width as f32, 0.0, 0.0, 0.0];
        let blur_v_data: [f32; 4] = [0.0, 1.0 / config.height as f32, 0.0, 0.0];

        let blur_h_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blur H Params"),
            contents: bytemuck::cast_slice(&blur_h_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let blur_v_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blur V Params"),
            contents: bytemuck::cast_slice(&blur_v_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let blur_h_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &blur_params_bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: blur_h_buffer.as_entire_binding() }],
            label: Some("blur_h_bind_group"),
        });
        let blur_v_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &blur_params_bind_group_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: blur_v_buffer.as_entire_binding() }],
            label: Some("blur_v_bind_group"),
        });

        Self {
            surface,
            device,
            queue,
            config,
            size,
            render_pipeline,
            shadow_pipeline,
            shadow_texture_view,
            global_uniform_buffer,
            global_bind_group_layout,
            global_bind_group,
            shadow_bind_group_layout,
            shadow_bind_group,
            texture_bind_group_layout,
            depth_texture_view,
            skeleton_bind_group_layout,
            dummy_skeleton_bind_group,
            // Post-Processing
            hdr_texture_view,
            hdr_bind_group,
            bloom_extract_texture_view,
            bloom_extract_bind_group,
            bloom_blur_texture_view,
            bloom_blur_bind_group,
            post_bind_group_layout,
            bloom_extract_pipeline,
            bloom_blur_pipeline,
            composite_pipeline,
            blur_params_buffer: blur_h_buffer, // Sadece referans olarak ilkini tutuyoruz
            blur_params_bind_group_layout,
            blur_h_bind_group,
            blur_v_bind_group,
            composite_bloom_bind_group_layout,
            composite_bloom_bind_group,
        }
    }

    // Ekran boyutlandırılırken Depth Buffer ve Post-Processing texture'ları da güncellenmeli!
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);

            let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Depth Texture"),
                size: wgpu::Extent3d { width: self.config.width, height: self.config.height, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            self.depth_texture_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Post-processing texture'larını yeniden oluştur
            let post_sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let (hdr_tv, hdr_bg, be_tv, be_bg, bb_tv, bb_bg, cb_bg) = Self::create_post_textures(
                &self.device, &self.post_bind_group_layout, &self.composite_bloom_bind_group_layout,
                &post_sampler, new_size.width, new_size.height,
            );
            self.hdr_texture_view = hdr_tv;
            self.hdr_bind_group = hdr_bg;
            self.bloom_extract_texture_view = be_tv;
            self.bloom_extract_bind_group = be_bg;
            self.bloom_blur_texture_view = bb_tv;
            self.bloom_blur_bind_group = bb_bg;
            self.composite_bloom_bind_group = cb_bg;

            // Blur parametrelerini güncelle
            let blur_h_data: [f32; 4] = [1.0 / new_size.width as f32, 0.0, 0.0, 0.0];
            let blur_v_data: [f32; 4] = [0.0, 1.0 / new_size.height as f32, 0.0, 0.0];
            // Yeni buffer'lar oluşturup bind group'ları yenilemeliyiz
            let blur_h_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Blur H Params"),
                contents: bytemuck::cast_slice(&blur_h_data),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            let blur_v_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Blur V Params"),
                contents: bytemuck::cast_slice(&blur_v_data),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
            self.blur_h_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &self.blur_params_bind_group_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: blur_h_buffer.as_entire_binding() }],
                label: Some("blur_h_bind_group"),
            });
            self.blur_v_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &self.blur_params_bind_group_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: blur_v_buffer.as_entire_binding() }],
                label: Some("blur_v_bind_group"),
            });
        }
    }

    // Post-processing texture'larını oluşturan yardımcı fonksiyon (new ve resize'da kullanılır)
    fn create_post_textures(
        device: &Device,
        post_bind_group_layout: &wgpu::BindGroupLayout,
        composite_bloom_bind_group_layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        width: u32,
        height: u32,
    ) -> (
        wgpu::TextureView, wgpu::BindGroup,   // HDR
        wgpu::TextureView, wgpu::BindGroup,   // Bloom Extract
        wgpu::TextureView, wgpu::BindGroup,   // Bloom Blur
        wgpu::BindGroup,                       // Composite bloom bind group
    ) {
        let create_hdr_texture = |label: &str| -> (wgpu::TextureView, wgpu::BindGroup) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: post_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
                ],
                label: Some(&format!("{}_bind_group", label)),
            });
            (view, bg)
        };

        let (hdr_view, hdr_bg) = create_hdr_texture("HDR Texture");
        let (be_view, be_bg) = create_hdr_texture("Bloom Extract Texture");
        let (bb_view, bb_bg) = create_hdr_texture("Bloom Blur Texture");

        // Composite pass bloom texture bind group (bloom_blur sonucu okuyacak)
        let composite_bloom_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: composite_bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&bb_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
            label: Some("composite_bloom_bind_group"),
        });

        (hdr_view, hdr_bg, be_view, be_bg, bb_view, bb_bg, composite_bloom_bg)
    }

    /// Post-processing geçişlerini çalıştır (Bloom extract → Blur → Composite + Tone Mapping)
    pub fn run_post_processing(&self, encoder: &mut wgpu::CommandEncoder, output_view: &wgpu::TextureView) {
        // Pass 1: Bright Extract (HDR → Bloom Extract)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Extract Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_extract_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.bloom_extract_pipeline);
            pass.set_bind_group(0, &self.hdr_bind_group, &[]);
            pass.draw(0..3, 0..1); // Fullscreen üçgen
        }

        // Pass 2a: Yatay Blur (Bloom Extract → Bloom Blur)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Blur Horizontal"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_blur_texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.bloom_blur_pipeline);
            pass.set_bind_group(0, &self.bloom_extract_bind_group, &[]);
            pass.set_bind_group(1, &self.blur_h_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2b: Dikey Blur (Bloom Blur → Bloom Extract tekrar kullanılır, ping-pong)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Bloom Blur Vertical"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.bloom_extract_texture_view, // Ping-pong: extract'ı tekrar hedef al
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.bloom_blur_pipeline);
            pass.set_bind_group(0, &self.bloom_blur_bind_group, &[]); // Yatay blur sonucunu oku
            pass.set_bind_group(1, &self.blur_v_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 3: Composite + Tone Mapping (HDR + Bloom → Ekran)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Composite + Tone Mapping Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output_view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &self.hdr_bind_group, &[]);              // HDR sahne
            pass.set_bind_group(1, &self.bloom_extract_bind_group, &[]);     // Blur sonucu (ping-pong'da extract'ta)
            pass.draw(0..3, 0..1);
        }
    }

    // Oyun içinden Mesh (Vertex Buffer) yaratmak için fonksiyon
    pub fn create_mesh(&self, vertices: &[Vertex]) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mesh Vertex Buffer"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    // Resim Bayt dizisini okur ve Ekran Kartında doku (Texture) oluşturur!
    pub fn create_texture(&self, rgba_bytes: &[u8], width: u32, height: u32) -> wgpu::BindGroup {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Game Texture"),
            size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::ImageCopyTexture { texture: &texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            rgba_bytes,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: Some(height) },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest, // Piksel dokunuşu
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
            label: Some("texture_bind_group"),
        })
    }
}
