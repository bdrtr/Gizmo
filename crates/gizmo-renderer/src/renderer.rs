use wgpu::{util::DeviceExt, Device, Queue, Surface, SurfaceConfiguration};
use winit::window::Window;
use std::sync::Arc;

pub use crate::gpu_types::{Vertex, InstanceRaw, LightData, PostProcessUniforms, SceneUniforms};
pub use crate::pipeline::SceneState;
pub use crate::post_process::PostProcessState;

pub struct Renderer<'a> {
    // === TEMEL WGPU KAYNAKLARI ===
    pub surface: Surface<'a>,
    pub device: Device,
    pub queue: Queue,
    pub config: SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
    pub depth_texture_view: wgpu::TextureView,

    // === SAHNE (Scene) — Pipeline'lar, Shadow, Skeleton ===
    pub scene: SceneState,

    // === POST-PROCESSING — HDR, Bloom, Blur, Composite ===
    pub post: PostProcessState,

    // === PARTİKÜL SİSTEMİ ===
    pub gpu_particles: Option<crate::particle_renderer::GpuParticleSystem>,
    
    // === GPU FİZİK SİSTEMİ ===
    pub gpu_physics: Option<crate::physics_renderer::GpuPhysicsSystem>,
}

impl<'a> Renderer<'a> {
    pub fn load_shader(device: &wgpu::Device, file_path: &str, fallback_src: &str, label: &str) -> wgpu::ShaderModule {
        let source = std::fs::read_to_string(file_path).unwrap_or_else(|_| fallback_src.to_string());
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
    }

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
                required_limits: wgpu::Limits {
                    max_bind_groups: 6,
                    max_storage_buffers_per_shader_stage: 8,
                    ..wgpu::Limits::default()
                },
                label: None,
            },
            None,
        ).await.unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            // VSync tercihi: Mailbox (uncapped FPS) varsa kullan, yoksa Fifo (VSync)
            present_mode: if surface_caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
                wgpu::PresentMode::Mailbox
            } else {
                wgpu::PresentMode::Fifo
            },
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth_texture_view = Self::create_depth_texture(&device, config.width, config.height);

        let scene = crate::pipeline::build_scene_pipelines(&device);
        let post_res = crate::post_process::build_post_process_resources(&device, surface_format, config.width, config.height);

        // GPU particle buffer boyutu — ihtiyaca göre ayarlanabilir
        let max_particles: u32 = 100_000;
        let gpu_particles = Some(crate::particle_renderer::GpuParticleSystem::new(
            &device, max_particles, &scene.global_bind_group_layout, wgpu::TextureFormat::Rgba16Float,
        ));

        // GPU Physics buffer boyutu -- Pachinko simülasyonu
        let max_physics_spheres: u32 = 50_000;
        let gpu_physics = Some(crate::physics_renderer::GpuPhysicsSystem::new(
            &device, max_physics_spheres, &scene.global_bind_group_layout, wgpu::TextureFormat::Rgba16Float, wgpu::TextureFormat::Depth32Float
        ));

        let scene_state = SceneState {
            render_pipeline: scene.render_pipeline,
            render_double_sided_pipeline: scene.render_double_sided_pipeline,
            unlit_pipeline:  scene.unlit_pipeline,
            water_pipeline:  scene.water_pipeline,
            shadow_pipeline: scene.shadow_pipeline,
            transparent_pipeline: scene.transparent_pipeline,
            shadow_texture_view:       scene.shadow_texture_view,
            global_uniform_buffer:     scene.global_uniform_buffer,
            global_bind_group_layout:  scene.global_bind_group_layout,
            global_bind_group:         scene.global_bind_group,
            shadow_bind_group_layout:  scene.shadow_bind_group_layout,
            shadow_bind_group:         scene.shadow_bind_group,
            texture_bind_group_layout: scene.texture_bind_group_layout,
            skeleton_bind_group_layout: scene.skeleton_bind_group_layout,
            dummy_skeleton_bind_group:  scene.dummy_skeleton_bind_group,
            instance_bind_group_layout: scene.instance_bind_group_layout,
            instance_buffer: scene.instance_buffer,
            instance_bind_group: scene.instance_bind_group,
        };

        let post_state = PostProcessState {
            hdr_texture_view:             post_res.hdr_texture_view,
            hdr_bind_group:               post_res.hdr_bind_group,
            bloom_extract_texture_view:   post_res.bloom_extract_texture_view,
            bloom_extract_bind_group:     post_res.bloom_extract_bind_group,
            bloom_blur_texture_view:      post_res.bloom_blur_texture_view,
            bloom_blur_bind_group:        post_res.bloom_blur_bind_group,
            post_bind_group_layout:       post_res.post_bind_group_layout,
            bloom_extract_pipeline:       post_res.bloom_extract_pipeline,
            bloom_blur_pipeline:          post_res.bloom_blur_pipeline,
            composite_pipeline:           post_res.composite_pipeline,
            blur_params_buffer:           post_res.blur_params_buffer,
            blur_params_bind_group_layout:post_res.blur_params_bind_group_layout,
            blur_h_bind_group:            post_res.blur_h_bind_group,
            blur_v_bind_group:            post_res.blur_v_bind_group,
            composite_bloom_bind_group_layout: post_res.composite_bloom_bind_group_layout,
            composite_bloom_bind_group:   post_res.composite_bloom_bind_group,
            post_params_buffer:           post_res.post_params_buffer,
            post_params_bind_group_layout:post_res.post_params_bind_group_layout,
            post_params_bind_group:       post_res.post_params_bind_group,
        };

        Self {
            surface, device, queue, config, size, depth_texture_view,
            scene: scene_state,
            post: post_state,
            gpu_particles,
            gpu_physics,
        }
    }

    pub fn rebuild_shaders(&mut self) {
        println!("🚀 Rebuilding Shaders Pipeline...");
        crate::pipeline::rebuild_pipelines(self);
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);

            self.depth_texture_view = Self::create_depth_texture(&self.device, new_size.width, new_size.height);

            let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let (hdr_tv, hdr_bg, be_tv, be_bg, bb_tv, bb_bg, cb_bg) =
                crate::post_process::create_post_textures(
                    &self.device, &self.post.post_bind_group_layout,
                    &self.post.composite_bloom_bind_group_layout, &sampler,
                    new_size.width, new_size.height,
                );
            self.post.hdr_texture_view           = hdr_tv;
            self.post.hdr_bind_group             = hdr_bg;
            self.post.bloom_extract_texture_view = be_tv;
            self.post.bloom_extract_bind_group   = be_bg;
            self.post.bloom_blur_texture_view    = bb_tv;
            self.post.bloom_blur_bind_group      = bb_bg;
            self.post.composite_bloom_bind_group = cb_bg;

            let (buf, h_bg, v_bg) = crate::post_process::create_blur_buffers(
                &self.device, &self.post.blur_params_bind_group_layout, new_size.width, new_size.height,
            );
            self.post.blur_params_buffer = buf;
            self.post.blur_h_bind_group  = h_bg;
            self.post.blur_v_bind_group  = v_bg;
        }
    }

    pub fn run_post_processing(&self, encoder: &mut wgpu::CommandEncoder, output_view: &wgpu::TextureView) {
        crate::post_process::run_post_processing(self, encoder, output_view);
    }

    pub fn update_post_process(&self, queue: &wgpu::Queue, params: PostProcessUniforms) {
        queue.write_buffer(&self.post.post_params_buffer, 0, bytemuck::cast_slice(&[params]));
    }

    pub fn create_mesh(&self, vertices: &[Vertex]) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Mesh Vertex Buffer"),
            contents: bytemuck::cast_slice(vertices),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    pub fn create_texture(&self, rgba_bytes: &[u8], width: u32, height: u32) -> wgpu::BindGroup {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Game Texture"), size,
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
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
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.scene.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
            label: Some("texture_bind_group"),
        })
    }

    fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }
}
