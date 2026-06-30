use super::*;

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Self {
        let mut size = window.inner_size();
        // WASM'da canvas boyutu 0x0 olabilir, en az 1x1 garanti et
        if size.width == 0 || size.height == 0 {
            size = winit::dpi::PhysicalSize::new(1280, 720);
        }

        #[cfg(target_arch = "wasm32")]
        {
            // Web'de 4K/Retina ekranlarda devasa çözünürlükler performansı katleder.
            // Internal rendering çözünürlüğünü 1280x720'ye (veya aspect ratio'ya göre) caple.
            if size.width > 640 || size.height > 360 {
                let aspect = size.width as f32 / size.height as f32;
                if aspect > 1.0 {
                    size.width = 640;
                    size.height = (640.0 / aspect) as u32;
                } else {
                    size.height = 360;
                    size.width = (360.0 * aspect) as u32;
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        let backends = wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL;
        #[cfg(not(target_arch = "wasm32"))]
        let backends = wgpu::Backends::all();

        log::info!("[Renderer] Window size: {}x{}", size.width, size.height);
        log::info!("[Renderer] Backends: {:?}", backends);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });

        // Enumerate available adapters for diagnostic info
        #[cfg(not(target_arch = "wasm32"))]
        {
            let adapters = instance.enumerate_adapters(backends).await;
            log::info!("[Renderer] {} adapter bulundu", adapters.len());
            for (i, a) in adapters.iter().enumerate() {
                let info = a.get_info();
                log::info!(
                    "[Renderer]   Adapter {}: {} ({:?}, {:?})",
                    i,
                    info.name,
                    info.backend,
                    info.device_type
                );
            }
        }

        let surface = instance
            .create_surface(window.clone())
            .expect("Surface oluşturulamadı!");

        log::info!("[Renderer] Surface oluşturuldu, adapter aranıyor...");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await;

        let adapter = match adapter {
            Ok(a) => {
                let info = a.get_info();
                log::info!(
                    "[Renderer] Adapter bulundu: {} ({:?})",
                    info.name,
                    info.backend
                );
                a
            }
            Err(_) => {
                log::warn!(
                    "[Renderer] Surface uyumlu adapter bulunamadı, surface'siz deneniyor..."
                );
                // Surface'siz adapter dene
                match instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::default(),
                        compatible_surface: None,
                        force_fallback_adapter: false,
                    })
                    .await
                {
                    Ok(a) => {
                        let info = a.get_info();
                        log::info!(
                            "[Renderer] Surface'siz adapter bulundu: {} ({:?})",
                            info.name,
                            info.backend
                        );
                        a
                    }
                    Err(_) => {
                        log::error!(
                            "[Renderer] Hiçbir adapter bulunamadı! Backends: {:?}",
                            backends
                        );
                        panic!(
                            "GPU adapter bulunamadı! Backends: {:?}, Window size: {}x{}",
                            backends, size.width, size.height
                        );
                    }
                }
            }
        };

        #[cfg(not(target_arch = "wasm32"))]
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::POLYGON_MODE_LINE,
                    required_limits: wgpu::Limits {
                        max_bind_groups: 6,
                        max_storage_buffers_per_shader_stage: 8,
                        max_storage_buffer_binding_size: 256 << 20, // 256 MB buffer limit
                        ..wgpu::Limits::default()
                    },
                    label: None,
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::Off,
                },
            )
            .await
            .unwrap();

        #[cfg(target_arch = "wasm32")]
        let (device, queue) = {
            let mut limits = adapter.limits();
            limits.max_bind_groups = limits.max_bind_groups.max(4);
            limits.max_storage_buffers_per_shader_stage = limits.max_storage_buffers_per_shader_stage.max(8);
            limits.max_storage_buffer_binding_size = limits.max_storage_buffer_binding_size.max(128 << 20);

            match adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        required_features: wgpu::Features::empty(),
                        required_limits: limits.clone(),
                        label: None,
                        experimental_features: wgpu::ExperimentalFeatures::default(),
                        memory_hints: wgpu::MemoryHints::default(),
                        trace: wgpu::Trace::Off,
                    },
                )
                .await
            {
                Ok(dq) => dq,
                Err(e) => {
                    log::warn!("[Renderer] request_device with custom adapter.limits() failed: {:?}", e);
                    match adapter
                        .request_device(
                            &wgpu::DeviceDescriptor {
                                required_features: wgpu::Features::empty(),
                                required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                                label: None,
                                experimental_features: wgpu::ExperimentalFeatures::default(),
                                memory_hints: wgpu::MemoryHints::default(),
                                trace: wgpu::Trace::Off,
                            },
                        )
                        .await
                    {
                        Ok(dq) => dq,
                        Err(e2) => {
                            log::warn!("[Renderer] request_device with downlevel_webgl2_defaults failed: {:?}", e2);
                            adapter
                                .request_device(
                                    &wgpu::DeviceDescriptor {
                                        required_features: wgpu::Features::empty(),
                                        required_limits: wgpu::Limits::default(),
                                        label: None,
                                        experimental_features: wgpu::ExperimentalFeatures::default(),
                                        memory_hints: wgpu::MemoryHints::default(),
                                        trace: wgpu::Trace::Off,
                                    },
                                )
                                .await
                                .unwrap_or_else(|e3| {
                                    panic!("Fatal: Failed to request wgpu device on WASM: {:?}", e3);
                                })
                        }
                    }
                }
            }
        };

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

        Self::finish_construction(device, queue, Some(surface), config, size)
    }

    /// Constructs a Renderer with **no window/surface** — every render target is an
    /// offscreen texture. Enables headless GPU servers, CI rendering and
    /// deterministic render harnesses. Shares [`Renderer::finish_construction`] with
    /// the windowed [`Renderer::new`], so every GPU subsystem initialises identically.
    ///
    /// `format` defaults to `Rgba8UnormSrgb` when `None`.
    pub async fn new_headless(width: u32, height: u32, format: Option<wgpu::TextureFormat>) -> Self {
        let width = width.max(1);
        let height = height.max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .expect("Headless Renderer: hiçbir GPU adapter bulunamadı");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: wgpu::Features::POLYGON_MODE_LINE,
                required_limits: wgpu::Limits {
                    max_bind_groups: 6,
                    max_storage_buffers_per_shader_stage: 8,
                    max_storage_buffer_binding_size: 256 << 20,
                    ..wgpu::Limits::default()
                },
                label: None,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("Headless Renderer: request_device başarısız");

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            format: format.unwrap_or(wgpu::TextureFormat::Rgba8UnormSrgb),
            width,
            height,
            present_mode: wgpu::PresentMode::AutoNoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        Self::finish_construction(
            device,
            queue,
            None,
            config,
            winit::dpi::PhysicalSize::new(width, height),
        )
    }

    /// Surface-agnostic tail of construction: configures the surface, builds the
    /// depth texture, all pipelines, post-process and GPU subsystems, then
    /// assembles the `Renderer`. Shared by `new` (windowed) and the forthcoming
    /// headless path so subsystem init order is identical regardless of how the
    /// surface/device were acquired.
    fn finish_construction(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface: Option<wgpu::Surface<'static>>,
        config: wgpu::SurfaceConfiguration,
        size: winit::dpi::PhysicalSize<u32>,
    ) -> Self {
        if let Some(ref surface) = surface {
            surface.configure(&device, &config);
        }

        let depth_texture_view = Self::create_depth_texture(&device, config.width, config.height);

        let scene = crate::pipeline::build_scene_pipelines(&device);
        let post_res = crate::post_process::build_post_process_resources(
            &device,
            config.format,
            config.width,
            config.height,
            &depth_texture_view,
        );

        // GPU particle buffer boyutu — ihtiyaca göre ayarlanabilir
        #[cfg(not(target_arch = "wasm32"))]
        let gpu_particles = {
            let max_particles: u32 = 100_000;
            Some(crate::gpu_particles::GpuParticleSystem::new(
                &device,
                max_particles,
                &scene.global_bind_group_layout,
                wgpu::TextureFormat::Rgba16Float,
            ))
        };
        #[cfg(target_arch = "wasm32")]
        let gpu_particles: Option<crate::gpu_particles::GpuParticleSystem> = None;

        #[cfg(not(target_arch = "wasm32"))]
        let gpu_physics = {
            let max_physics_spheres: u32 = 50_000;
            let mut physics = crate::gpu_physics::GpuPhysicsSystem::new(
                &device,
                max_physics_spheres,
                &scene.global_bind_group_layout,
                wgpu::TextureFormat::Rgba16Float,
                wgpu::TextureFormat::Depth32Float,
            );
            physics.enable_debug(&device, 0);
            Some(physics)
        };
        #[cfg(target_arch = "wasm32")]
        let gpu_physics: Option<crate::gpu_physics::GpuPhysicsSystem> = None;

        #[cfg(not(target_arch = "wasm32"))]
        let gpu_fluid = Some(crate::gpu_fluid::GpuFluidSystem::new(
            &device,
            &queue,
            100_000,
            &scene.global_bind_group_layout,
            post_res.hdr_texture.format(),
            config.width,
            config.height,
        ));
        #[cfg(target_arch = "wasm32")]
        let gpu_fluid: Option<crate::gpu_fluid::GpuFluidSystem> = None;
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
            point_shadow_depth_texture: scene.point_shadow_depth_texture,
            point_shadow_cube_view: scene.point_shadow_cube_view,
            point_shadow_face_views: scene.point_shadow_face_views,
            shadow_pass_bind_group_layout: scene.shadow_pass_bind_group_layout,
            shadow_cascade_uniform_buffers: scene.shadow_cascade_uniform_buffers,
            shadow_pass_bind_groups: scene.shadow_pass_bind_groups,
            point_shadow_uniform_buffers: scene.point_shadow_uniform_buffers,
            point_shadow_pass_bind_groups: scene.point_shadow_pass_bind_groups,
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

        #[cfg(not(target_arch = "wasm32"))]
        let deferred = Some(crate::deferred::DeferredState::new(
            &device,
            &scene_state,
            size.width,
            size.height,
        ));
        #[cfg(target_arch = "wasm32")]
        let deferred: Option<crate::deferred::DeferredState> = None;

        #[cfg(not(target_arch = "wasm32"))]
        let gpu_cull = Some(crate::gpu_cull::GpuCullState::new(
            &device,
            &scene_state,
            scene_state.instance_capacity as u32,
        ));
        #[cfg(target_arch = "wasm32")]
        let gpu_cull: Option<crate::gpu_cull::GpuCullState> = None;

        let ssao = deferred.as_ref().map(|def| {
            crate::ssao::SsaoState::new(&device, &queue, &scene_state, def, size.width, size.height)
        });

        let ssr = deferred.as_ref().map(|def| {
            crate::ssr::SsrState::new(
                &device,
                &scene_state,
                def,
                &post_res.hdr_texture_view,
                size.width,
                size.height,
            )
        });

        let ssgi = deferred.as_ref().map(|def| {
            crate::ssgi::SsgiState::new(
                &device,
                &scene_state,
                def,
                &post_res.hdr_texture_view,
                size.width,
                size.height,
            )
        });

        let volumetric = deferred.as_ref().map(|def| {
            crate::volumetric::VolumetricState::new(
                &device,
                &scene_state,
                def,
                size.width,
                size.height,
            )
        });

        let decal = deferred
            .as_ref()
            .map(|def| crate::decal::DecalState::new(&device, &scene_state, def));

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

        // === FXAA Başlatma ===
        let fxaa = Some(crate::fxaa::FxaaState::new(
            &device,
            config.format,
            size.width,
            size.height,
        ));

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
            fxaa,
            gpu_particles,
            gpu_physics,
            gpu_fluid,
            debug_renderer,
            asset_manager: std::sync::RwLock::new(crate::asset::AssetManager::new()),
            web_profile: crate::web_profile::WebProfile::auto(),
            shading_mode: 0,
            environment_preset: 0,
            environment_preset_2: 0,
            environment_blend_t: 0.0,
            bloom_intensity: 0.8,
            bloom_threshold: 0.85,
            exposure: 1.15,
            dof_enabled: true,
            dof_focus_dist: 4.5, // 4.5 meters (fits our lamp setup)
            dof_focus_range: 2.0, // 2.0 meters focus range
            dof_blur_size: 4.0, // Beautiful smooth blur
            chromatic_aberration: 0.15, // Cinematic soft fringe
            film_grain_intensity: 0.03, // Photographic film grain
            point_shadows_enabled: false,
        }
    }
}
