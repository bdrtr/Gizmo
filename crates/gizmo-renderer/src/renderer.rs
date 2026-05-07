use std::sync::Arc;
use wgpu::{util::DeviceExt, Device, Queue, Surface, SurfaceConfiguration};
use winit::window::Window;

pub use crate::gpu_types::{
    InstanceRaw, LightData, PostProcessUniforms, SceneUniforms, ShadowVsUniform, Vertex,
};
pub use crate::pipeline::SceneState;
pub use crate::post_process::PostProcessState;

// ============================================================
//  RenderContext — wgpu detaylarını kullanıcıdan gizler
// ============================================================

/// Kullanıcı kodunun doğrudan `wgpu::CommandEncoder` veya `wgpu::TextureView`
/// görmesine gerek kalmadan render işlemi yapmasını sağlayan bağlam nesnesi.
///
/// ```ignore
/// fn render(world: &mut World, _state: &GameState, ctx: &mut RenderContext) {
///     ctx.disable_gpu_compute();           // GPU Compute kapalı
///     ctx.default_render(world);           // Varsayılan render pipeline
/// }
/// ```
pub struct RenderContext<'a, 'r> {
    pub(crate) encoder: &'a mut wgpu::CommandEncoder,
    pub(crate) view: &'a wgpu::TextureView,
    pub(crate) renderer: &'a mut Renderer<'r>,
    pub(crate) light_time: f32,
}

impl<'a, 'r> RenderContext<'a, 'r> {
    /// Yeni bir RenderContext oluşturur (motor tarafından dahili olarak çağrılır).
    pub fn new(
        encoder: &'a mut wgpu::CommandEncoder,
        view: &'a wgpu::TextureView,
        renderer: &'a mut Renderer<'r>,
        light_time: f32,
    ) -> Self {
        Self { encoder, view, renderer, light_time }
    }

    /// GPU Compute alt sistemlerini devre dışı bırakır (fluid, particles, physics).
    /// Basit sahnelerde gereksiz GPU iş yükünü sıfırlar.
    pub fn disable_gpu_compute(&mut self) {
        self.renderer.gpu_fluid = None;
        self.renderer.gpu_particles = None;
        self.renderer.gpu_physics = None;
    }

    /// Mevcut sahne ışık zamanını döndürür (saniye).
    pub fn light_time(&self) -> f32 {
        self.light_time
    }

    /// Renderer'a doğrudan erişim (ileri düzey kullanım).
    pub fn renderer(&self) -> &Renderer<'r> {
        self.renderer
    }

    /// Renderer'a mutable erişim (ileri düzey kullanım).
    pub fn renderer_mut(&mut self) -> &mut Renderer<'r> {
        self.renderer
    }

    /// İleri düzey kullanım: ham wgpu encoder'a erişim.
    pub fn encoder(&mut self) -> &mut wgpu::CommandEncoder {
        self.encoder
    }

    /// İleri düzey kullanım: çıkış texture view'ına erişim.
    pub fn output_view(&self) -> &wgpu::TextureView {
        self.view
    }

    /// Dahili bileşenlere eşzamanlı erişim — `default_render_pass` gibi
    /// fonksiyonlara geçirmek için kullanılır.
    pub fn parts_mut(&mut self) -> (&mut wgpu::CommandEncoder, &wgpu::TextureView, &mut Renderer<'r>) {
        (self.encoder, self.view, self.renderer)
    }
}


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
    pub gpu_particles: Option<crate::gpu_particles::GpuParticleSystem>,

    pub gpu_physics: Option<crate::gpu_physics::GpuPhysicsSystem>,

    // === GPU SIVI SİSTEMİ ===
    pub gpu_fluid: Option<crate::gpu_fluid::GpuFluidSystem>,

    // === DEFERRED RENDERING — G-Buffer + Lighting pass ===
    pub deferred: Option<crate::deferred::DeferredState>,

    // === GPU-DRIVEN MESH CULLING — Compute frustum cull + indirect draw ===
    pub gpu_cull: Option<crate::gpu_cull::GpuCullState>,

    // === SSAO — Screen-Space Ambient Occlusion ===
    pub ssao: Option<crate::ssao::SsaoState>,

    // === SSR — Screen-Space Reflections ===
    pub ssr: Option<crate::ssr::SsrState>,

    // === SSGI — Screen-Space Global Illumination ===
    pub ssgi: Option<crate::ssgi::SsgiState>,

    // === Volumetric Lighting (God Rays) ===
    pub volumetric: Option<crate::volumetric::VolumetricState>,

    // === DEFERRED DECALS ===
    pub decal: Option<crate::decal::DecalState>,

    // === TAA — Temporal Anti-Aliasing (ping-pong history + Halton jitter) ===
    pub taa: Option<crate::taa::TaaState>,

    // === GIZMO HATA AYIKLAMA (Debug Lines) ===
    pub debug_renderer: Option<crate::debug_renderer::GizmoRendererSystem>,

    // === DAHİLİ ASSET YÖNETİCİSİ (Kolaylık metodları için cache) ===
    asset_manager: std::cell::RefCell<crate::asset::AssetManager>,
}

impl<'a> Renderer<'a> {
    pub fn load_shader(
        device: &wgpu::Device,
        file_path: &str,
        fallback_src: &str,
        label: &str,
    ) -> wgpu::ShaderModule {
        let source =
            std::fs::read_to_string(file_path).unwrap_or_else(|_| fallback_src.to_string());
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
        let surface = instance
            .create_surface(window.clone())
            .expect("Surface error");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::POLYGON_MODE_LINE | wgpu::Features::TEXTURE_COMPRESSION_BC,
                    required_limits: wgpu::Limits {
                        max_bind_groups: 6,
                        max_storage_buffers_per_shader_stage: 8,
                        max_storage_buffer_binding_size: 256 << 20, // 256 MB buffer limit
                        ..wgpu::Limits::default()
                    },
                    label: None,
                },
                None,
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            // VSync tercihi: Mailbox (uncapped FPS) varsa kullan, yoksa Fifo (VSync)
            present_mode: wgpu::PresentMode::AutoNoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth_texture_view = Self::create_depth_texture(&device, config.width, config.height);

        let scene = crate::pipeline::build_scene_pipelines(&device);
        let post_res = crate::post_process::build_post_process_resources(
            &device,
            surface_format,
            config.width,
            config.height,
            &depth_texture_view,
        );

        // GPU particle buffer boyutu — ihtiyaca göre ayarlanabilir
        let max_particles: u32 = 100_000;
        let gpu_particles = Some(crate::gpu_particles::GpuParticleSystem::new(
            &device,
            max_particles,
            &scene.global_bind_group_layout,
            wgpu::TextureFormat::Rgba16Float,
        ));

        // GPU Physics buffer boyutu -- 1 Milyon tam OBB fizik iterasyonu GPU'yu kitler, 50k ile 60+ FPS alalım!
        let max_physics_spheres: u32 = 50_000;
        let gpu_physics = Some(crate::gpu_physics::GpuPhysicsSystem::new(
            &device,
            max_physics_spheres,
            &scene.global_bind_group_layout,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
        ));

        let gpu_fluid = Some(crate::gpu_fluid::GpuFluidSystem::new(
            &device,
            &queue,
            100_000,
            &scene.global_bind_group_layout,
            post_res.hdr_texture.format(),
            config.width,
            config.height,
        ));        
        let debug_renderer = Some(crate::debug_renderer::GizmoRendererSystem::new(
            &device,
            &scene.global_bind_group_layout,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
        ));


        let scene_state = SceneState {
            render_pipeline: scene.render_pipeline,
            render_double_sided_pipeline: scene.render_double_sided_pipeline,
            wireframe_pipeline: scene.wireframe_pipeline,
            unlit_pipeline: scene.unlit_pipeline,
            sky_pipeline: scene.sky_pipeline,
            water_pipeline: scene.water_pipeline,
            shadow_pipeline: scene.shadow_pipeline,
            transparent_pipeline: scene.transparent_pipeline,
            grid_pipeline: scene.grid_pipeline,
            shadow_texture_view: scene.shadow_texture_view,
            shadow_cascade_layer_views: scene.shadow_cascade_layer_views,
            shadow_depth_texture: scene.shadow_depth_texture,
            shadow_pass_bind_group_layout: scene.shadow_pass_bind_group_layout,
            shadow_cascade_uniform_buffers: scene.shadow_cascade_uniform_buffers,
            shadow_pass_bind_groups: scene.shadow_pass_bind_groups,
            global_uniform_buffer: scene.global_uniform_buffer,
            global_bind_group_layout: scene.global_bind_group_layout,
            global_bind_group: scene.global_bind_group,
            shadow_bind_group_layout: scene.shadow_bind_group_layout,
            shadow_bind_group: scene.shadow_bind_group,
            texture_bind_group_layout: scene.texture_bind_group_layout,
            skeleton_bind_group_layout: scene.skeleton_bind_group_layout,
            dummy_skeleton_bind_group: scene.dummy_skeleton_bind_group,
            instance_bind_group_layout: scene.instance_bind_group_layout,
            instance_buffer: scene.instance_buffer,
            instance_bind_group: scene.instance_bind_group,
            instance_capacity: scene.instance_capacity,
        };

        let deferred = Some(crate::deferred::DeferredState::new(
            &device,
            &scene_state,
            size.width,
            size.height,
        ));

        let gpu_cull = Some(crate::gpu_cull::GpuCullState::new(
            &device,
            &scene_state,
            scene_state.instance_capacity as u32,
        ));

        let ssao = deferred.as_ref().map(|def| {
            crate::ssao::SsaoState::new(&device, &queue, &scene_state, def, size.width, size.height)
        });

        let ssr = deferred.as_ref().map(|def| {
            crate::ssr::SsrState::new(&device, &scene_state, def, &post_res.hdr_texture_view, size.width, size.height)
        });

        let ssgi = deferred.as_ref().map(|def| {
            crate::ssgi::SsgiState::new(&device, &scene_state, def, &post_res.hdr_texture_view, size.width, size.height)
        });

        let volumetric = deferred.as_ref().map(|def| {
            crate::volumetric::VolumetricState::new(&device, &scene_state, def, size.width, size.height)
        });

        let decal = deferred.as_ref().map(|def| {
            crate::decal::DecalState::new(&device, &scene_state, def)
        });

        let taa = if let Some(ref def) = deferred {
            Some(crate::taa::TaaState::new(
                &device,
                &post_res.hdr_texture_view,
                &def.world_position_view,
                size.width,
                size.height,
            ))
        } else {
            None
        };

        let post_state = PostProcessState {
            hdr_texture: post_res.hdr_texture,
            hdr_texture_view: post_res.hdr_texture_view,
            hdr_bind_group: post_res.hdr_bind_group,
            bloom_extract_texture_view: post_res.bloom_extract_texture_view,
            bloom_extract_bind_group: post_res.bloom_extract_bind_group,
            bloom_blur_texture_view: post_res.bloom_blur_texture_view,
            bloom_blur_bind_group: post_res.bloom_blur_bind_group,
            post_bind_group_layout: post_res.post_bind_group_layout,
            bloom_extract_pipeline: post_res.bloom_extract_pipeline,
            bloom_blur_pipeline: post_res.bloom_blur_pipeline,
            composite_pipeline: post_res.composite_pipeline,
            blur_params_buffer: post_res.blur_params_buffer,
            blur_params_bind_group_layout: post_res.blur_params_bind_group_layout,
            blur_h_bind_group: post_res.blur_h_bind_group,
            blur_v_bind_group: post_res.blur_v_bind_group,
            composite_bloom_bind_group_layout: post_res.composite_bloom_bind_group_layout,
            composite_bloom_bind_group: post_res.composite_bloom_bind_group,
            post_params_buffer: post_res.post_params_buffer,
            post_params_bind_group_layout: post_res.post_params_bind_group_layout,
            post_params_bind_group: post_res.post_params_bind_group,
        };

        Self {
            surface,
            device,
            queue,
            config,
            size,
            depth_texture_view,
            scene: scene_state,
            post: post_state,
            deferred,
            gpu_cull,
            ssao,
            ssr,
            ssgi,
            volumetric,
            decal,
            taa,
            gpu_particles,
            gpu_physics,
            gpu_fluid,
            debug_renderer,
            asset_manager: std::cell::RefCell::new(crate::asset::AssetManager::new()),
        }
    }

    pub fn rebuild_shaders(&mut self) {
        println!("🚀 Rebuilding Shaders Pipeline...");
        crate::pipeline::rebuild_pipelines(self);
    }

    pub fn ensure_instance_capacity(&mut self, needed: usize) -> bool {
        self.scene.ensure_instance_capacity(&self.device, needed)
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(&self.device, &self.config);

            self.depth_texture_view =
                Self::create_depth_texture(&self.device, new_size.width, new_size.height);

            if let Some(ref mut def) = self.deferred {
                def.resize(&self.device, new_size.width, new_size.height);
                if let Some(ref mut decal) = self.decal {
                    decal.resize(&self.device, def);
                }
            }

            let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            let (hdr_t, hdr_tv, hdr_bg, be_tv, be_bg, bb_tv, bb_bg, cb_bg) =
                crate::post_process::create_post_textures(
                    &self.device,
                    &self.post.post_bind_group_layout,
                    &self.post.composite_bloom_bind_group_layout,
                    &sampler,
                    new_size.width,
                    new_size.height,
                    &self.depth_texture_view,
                );
            self.post.hdr_texture = hdr_t;
            self.post.hdr_texture_view = hdr_tv;
            self.post.hdr_bind_group = hdr_bg;
            self.post.bloom_extract_texture_view = be_tv;
            self.post.bloom_extract_bind_group = be_bg;
            self.post.bloom_blur_texture_view = bb_tv;
            self.post.bloom_blur_bind_group = bb_bg;
            self.post.composite_bloom_bind_group = cb_bg;

            let (buf, h_bg, v_bg) = crate::post_process::create_blur_buffers(
                &self.device,
                &self.post.blur_params_bind_group_layout,
                new_size.width,
                new_size.height,
            );
            self.post.blur_params_buffer = buf;
            self.post.blur_h_bind_group = h_bg;
            self.post.blur_v_bind_group = v_bg;

            // TAA history textures + bind groups (needs fresh hdr_view + position_view)
            if let (Some(ref mut taa), Some(ref def)) = (&mut self.taa, &self.deferred) {
                taa.resize(
                    &self.device,
                    &self.post.hdr_texture_view,
                    &def.world_position_view,
                    new_size.width,
                    new_size.height,
                );
            }
            if let (Some(ref mut ssgi), Some(ref def)) = (&mut self.ssgi, &self.deferred) {
                ssgi.resize(
                    &self.device,
                    def,
                    &self.post.hdr_texture_view,
                    new_size.width,
                    new_size.height,
                );
            }
        }
    }

    // ==========================================================
    //  Kolaylık Metodları — Asset Oluşturma
    //  Kullanıcı `AssetManager` oluşturmak zorunda kalmadan
    //  doğrudan `renderer.create_cube()` gibi çağırabilir.
    // ==========================================================

    /// Küp mesh oluşturur.
    pub fn create_cube(&self) -> crate::components::Mesh {
        crate::asset::AssetManager::create_cube(&self.device)
    }

    /// Küre mesh oluşturur.
    pub fn create_sphere(&self, radius: f32, stacks: u32, slices: u32) -> crate::components::Mesh {
        crate::asset::AssetManager::create_sphere(&self.device, radius, stacks, slices)
    }

    /// Düzlem mesh oluşturur.
    pub fn create_plane(&self, size: f32) -> crate::components::Mesh {
        crate::asset::AssetManager::create_plane(&self.device, size)
    }

    /// Dama dokusu (checkerboard) oluşturur — test materyalleri için idealdir.
    /// Cache'lenir: aynı doku tekrar oluşturulmaz.
    pub fn create_checkerboard_texture(&self) -> Arc<wgpu::BindGroup> {
        self.asset_manager.borrow_mut()
            .create_checkerboard_texture(&self.device, &self.queue, &self.scene.texture_bind_group_layout)
    }

    /// Düz beyaz doku — varsayılan materyal için.
    /// Cache'lenir: aynı doku tekrar oluşturulmaz.
    pub fn create_white_texture(&self) -> Arc<wgpu::BindGroup> {
        self.asset_manager.borrow_mut()
            .create_white_texture(&self.device, &self.queue, &self.scene.texture_bind_group_layout)
    }

    /// Diskten doku yükler (BC7 pipeline dahil).
    /// Cache'lenir: aynı dosya yolu tekrar yüklenmez.
    pub fn load_texture(&self, path: &str) -> Result<Arc<wgpu::BindGroup>, String> {
        self.asset_manager.borrow_mut()
            .load_material_texture(&self.device, &self.queue, &self.scene.texture_bind_group_layout, path)
    }

    pub fn run_post_processing(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        output_view: &wgpu::TextureView,
    ) {
        crate::post_process::run_post_processing(self, encoder, output_view);
    }

    pub fn update_post_process(&self, queue: &wgpu::Queue, params: PostProcessUniforms) {
        queue.write_buffer(
            &self.post.post_params_buffer,
            0,
            bytemuck::cast_slice(&[params]),
        );
    }

    pub fn create_mesh(&self, vertices: &[Vertex]) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Mesh Vertex Buffer"),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            })
    }

    pub fn create_texture(&self, rgba_bytes: &[u8], width: u32, height: u32) -> wgpu::BindGroup {
        let mip_level_count = width.max(height).ilog2() + 1;
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Game Texture"),
            size,
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba_bytes,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        Self::generate_mipmaps(
            &self.device,
            &self.queue,
            &texture,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            mip_level_count,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.scene.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
            label: Some("texture_bind_group"),
        })
    }

    fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        tex.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn generate_mipmaps(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        format: wgpu::TextureFormat,
        mip_level_count: u32,
    ) {
        if mip_level_count <= 1 {
            return;
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Mipmap Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/mipmap.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Mipmap Blit Pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
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

        let bind_group_layout = pipeline.get_bind_group_layout(0);
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mipmap Encoder"),
        });

        let views: Vec<wgpu::TextureView> = (0..mip_level_count)
            .map(|mip| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some(&format!("Mip {}", mip)),
                    format: None,
                    dimension: None,
                    aspect: wgpu::TextureAspect::All,
                    base_mip_level: mip,
                    mip_level_count: Some(1),
                    base_array_layer: 0,
                    array_layer_count: None,
                })
            })
            .collect();

        for target_mip in 1..mip_level_count as usize {
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[target_mip - 1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
                label: None,
            });

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &views[target_mip],
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        queue.submit(Some(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mipmap_level_calculation() {
        let width = 4096u32;
        let height = 2048u32;
        let mip_level_count = width.max(height).ilog2() + 1;
        assert_eq!(mip_level_count, 13); // 4096 -> 2^12. Level count is 13 (with level 0)

        let width2 = 512u32;
        let height2 = 512u32;
        assert_eq!(width2.max(height2).ilog2() + 1, 10);
    }

    #[test]
    fn test_headless_mipmap_generation() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await;

            let adapter = match adapter {
                Some(a) => a,
                None => {
                    println!(
                        "No suitable GPU adapter found for headless test. Skipping wgpu test."
                    );
                    return;
                }
            };

            let (device, queue) = adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits::downlevel_defaults(),
                        label: None,
                    },
                    None,
                )
                .await
                .unwrap();

            let width = 256u32;
            let height = 256u32;
            let mip_level_count = width.max(height).ilog2() + 1;

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Test Texture"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });

            // This should compile the WGSL and execute without panicking or creating wgpu validation errors
            Renderer::generate_mipmaps(
                &device,
                &queue,
                &texture,
                wgpu::TextureFormat::Rgba8UnormSrgb,
                mip_level_count,
            );

            device.poll(wgpu::Maintain::Wait);
        });
    }
}
