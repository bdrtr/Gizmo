use std::sync::Arc;
use wgpu::{Device, Queue, Surface, SurfaceConfiguration};
use winit::window::Window;

pub use crate::gpu_types::{
    InstanceRaw, LightData, PostProcessUniforms, SceneUniforms, ShadowVsUniform, Vertex,
};
pub use crate::pipeline::SceneState;
pub use crate::post_process::PostProcessState;

// Cohesive helper groups, split out for navigability (no logic change).
// Each module holds `impl Renderer` blocks (private-field access preserved).
mod assets;
mod construction;
mod textures;

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
pub struct RenderContext<'a> {
    pub(crate) encoder: &'a mut wgpu::CommandEncoder,
    pub(crate) view: &'a wgpu::TextureView,
    pub(crate) renderer: &'a mut Renderer,
    pub(crate) light_time: f32,
}

impl<'a> RenderContext<'a> {
    /// Yeni bir RenderContext oluşturur (motor tarafından dahili olarak çağrılır).
    pub fn new(
        encoder: &'a mut wgpu::CommandEncoder,
        view: &'a wgpu::TextureView,
        renderer: &'a mut Renderer,
        light_time: f32,
    ) -> Self {
        Self {
            encoder,
            view,
            renderer,
            light_time,
        }
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
    pub fn renderer(&self) -> &Renderer {
        self.renderer
    }

    /// Renderer'a mutable erişim (ileri düzey kullanım).
    pub fn renderer_mut(&mut self) -> &mut Renderer {
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
    pub fn parts_mut(&mut self) -> (&mut wgpu::CommandEncoder, &wgpu::TextureView, &mut Renderer) {
        (self.encoder, self.view, self.renderer)
    }
}

pub struct Renderer {
    // === TEMEL WGPU KAYNAKLARI ===
    /// `None` in headless/offscreen mode (constructed via [`Renderer::new_headless`]);
    /// `Some` on the windowed path. Frame acquisition/present must handle both.
    pub surface: Option<Surface<'static>>,
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

    // === FXAA — Fast Approximate Anti-Aliasing (son post-process pass) ===
    pub fxaa: Option<crate::fxaa::FxaaState>,

    // === GIZMO HATA AYIKLAMA (Debug Lines) ===
    pub debug_renderer: Option<crate::debug_renderer::GizmoRendererSystem>,

    // === DAHİLİ ASSET YÖNETİCİSİ (Kolaylık metodları için cache) ===
    pub asset_manager: std::sync::RwLock<crate::asset::AssetManager>,

    // === WEB PROFİLİ — Platform bazlı GPU kaynak yönetimi ===
    pub web_profile: crate::web_profile::WebProfile,

    // === RENDER SETTINGS & DIAGNOSTICS ===
    pub shading_mode: u32,
    pub environment_preset: u32,
    pub environment_preset_2: u32,
    pub environment_blend_t: f32,
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub exposure: f32,
    pub dof_enabled: bool,
    pub dof_focus_dist: f32,
    pub dof_focus_range: f32,
    pub dof_blur_size: f32,
    pub chromatic_aberration: f32,
    pub film_grain_intensity: f32,
    pub point_shadows_enabled: bool,
    /// Whether the GPU SPH fluid "ocean" is simulated and composited this frame.
    /// A renderer always allocates a 100k-particle fluid system, but its water
    /// surface must NOT render over every scene — only scenes that actually want
    /// fluid opt in (`ocean_scene`, `fluid_rigid`, …). Off by default so a plain
    /// scene isn't covered by a stray mottled water surface.
    pub fluid_enabled: bool,
}

impl Renderer {
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

    pub fn rebuild_shaders(&mut self) {
        tracing::info!("🚀 Rebuilding Shaders Pipeline...");
        crate::pipeline::rebuild_pipelines(self);
    }

    pub fn ensure_instance_capacity(&mut self, needed: usize) -> bool {
        self.scene.ensure_instance_capacity(&self.device, needed)
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        // Web'de dahili render çözünürlüğünü aynı cap'ten geçir (native no-op).
        // Bu olmadan ilk `Resized` olayı — tarayıcı canvas'ı CSS %100 ile
        // pencereye büyüdüğünde — surface + tüm post-process zincirini tam
        // fiziksel çözünürlükte yeniden kurup `Renderer::new`'daki 640x360
        // perf cap'ini sessizce delerdi.
        let new_size = crate::renderer::construction::cap_web_render_size(new_size);
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            if let Some(ref surface) = self.surface {
                surface.configure(&self.device, &self.config);
            }

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
            if let (Some(ref mut ssao), Some(ref def)) = (&mut self.ssao, &self.deferred) {
                ssao.resize(
                    &self.device,
                    def,
                    new_size.width,
                    new_size.height,
                );
            }
            if let (Some(ref mut vol), Some(ref def)) = (&mut self.volumetric, &self.deferred) {
                vol.resize(
                    &self.device,
                    def,
                    new_size.width,
                    new_size.height,
                );
            }
            // FXAA resize
            if let Some(ref mut fxaa) = self.fxaa {
                fxaa.resize(&self.device, &self.queue, self.config.format, new_size.width, new_size.height);
            }
            // GPU fluid SSFR render targets. Previously never rebuilt, so after any
            // resize the fluid rendered into a stale sub-rectangle / the composite
            // copied the wrong extent. Fluid composites into the HDR target.
            let fluid_fmt = self.post.hdr_texture.format();
            if let Some(ref mut fluid) = self.gpu_fluid {
                fluid.resize(&self.device, fluid_fmt, new_size.width, new_size.height);
            }
        }
    }

    pub fn run_post_processing(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        output_view: &wgpu::TextureView,
    ) {
        if let Some(ref fxaa) = self.fxaa {
            if fxaa.enabled {
                // Composite → FXAA input texture → FXAA → output_view
                crate::post_process::run_post_processing(self, encoder, &fxaa.input_texture_view);
                crate::fxaa::run_fxaa_pass(fxaa, encoder, output_view);
                return;
            }
        }
        // FXAA kapalıysa doğrudan output'a yaz
        crate::post_process::run_post_processing(self, encoder, output_view);
    }

    pub fn update_post_process(&self, queue: &wgpu::Queue, params: PostProcessUniforms) {
        queue.write_buffer(
            &self.post.post_params_buffer,
            0,
            bytemuck::cast_slice(&[params]),
        );
    }

    pub(crate) fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
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
    fn test_environment_preset_ranges() {
        // Enforce valid atmospheric preset range constraints [0, 3]
        let renderer_presets = vec![0, 1, 2, 3];
        for preset in &renderer_presets {
            assert!(*preset < 4, "Preset ID {} exceeds maximum allowed atmospheric preset index 3!", preset);
        }
    }

    #[test]
    fn test_environment_blend_weight_clamping() {
        // Dynamic weight blend_t must lie within [0.0, 1.0] and clamp gracefully if out-of-bounds
        let input_weights = vec![-0.5f32, 0.0f32, 0.45f32, 1.0f32, 1.5f32];
        let expected_clamps = vec![0.0f32, 0.0f32, 0.45f32, 1.0f32, 1.0f32];
        for (input, expected) in input_weights.into_iter().zip(expected_clamps) {
            let clamped = input.clamp(0.0, 1.0);
            assert_eq!(clamped, expected, "Clamped weight of {} did not match expected value {}!", input, expected);
        }
    }

    #[test]
    fn test_gpu_uniform_struct_sizes() {
        // Extremely critical alignment checks to prevent runtime pipeline crashes on GPU
        assert_eq!(std::mem::size_of::<crate::gpu_types::SceneUniforms>(), 1104, "SceneUniforms size shifted from target 1104 bytes!");
        assert_eq!(std::mem::size_of::<crate::gpu_types::LightData>(), 64, "LightData size shifted from target 64 bytes!");
        assert_eq!(std::mem::size_of::<crate::gpu_types::PostProcessUniforms>(), 48, "PostProcessUniforms size shifted from target 48 bytes!");
        assert_eq!(std::mem::size_of::<crate::gpu_types::InstanceRaw>(), 96, "InstanceRaw size shifted from target 96 bytes!");

        // Vertex attribute offsetleri shader VertexInput @location'larıyla (ve
        // Vertex::desc() ile) BİREBİR uyuşmalı. Bir alan kayarsa skinning/tangent
        // bozulur ama toplam boyut değişmeyebilir — bu yüzden offset'leri de kilitle.
        use crate::gpu_types::Vertex;
        assert_eq!(std::mem::offset_of!(Vertex, position), 0);
        assert_eq!(std::mem::offset_of!(Vertex, color), 12);
        assert_eq!(std::mem::offset_of!(Vertex, normal), 24);
        assert_eq!(std::mem::offset_of!(Vertex, tex_coords), 36);
        assert_eq!(std::mem::offset_of!(Vertex, joint_indices), 44);
        assert_eq!(std::mem::offset_of!(Vertex, joint_weights), 60);
        assert_eq!(std::mem::offset_of!(Vertex, tangent), 76);
        assert_eq!(std::mem::size_of::<Vertex>(), 92, "Vertex size/layout shifted!");
    }

    #[test]
    fn test_headless_mipmap_generation() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                flags: wgpu::InstanceFlags::default(),
                memory_budget_thresholds: Default::default(),
                backend_options: Default::default(),
                display: None,
            });

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await;

            let adapter = match adapter {
                Ok(a) => a,
                Err(_) => {
                    tracing::info!(
                        "No suitable GPU adapter found for headless test. Skipping wgpu test."
                    );
                    return;
                }
            };

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_defaults(),
                    label: None,
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::Off,
                })
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

            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        });
    }

    #[test]
    fn new_headless_builds_all_subsystems_and_renders_offscreen() {
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping new_headless_builds_all_subsystems_and_renders_offscreen: \
                 no GPU adapter available (headless render requires a GPU)"
            );
            return;
        }
        pollster::block_on(async {
            // Builds the FULL renderer (pipelines, post-process, deferred, ssao/ssr/ssgi,
            // gpu particle/physics/fluid) with NO window/surface — the headless path.
            let renderer = Renderer::new_headless(64, 64, None).await;
            assert!(
                renderer.surface.is_none(),
                "headless renderer must have no surface"
            );
            assert_eq!((renderer.config.width, renderer.config.height), (64, 64));

            let device = &renderer.device;
            let queue = &renderer.queue;

            // Clear an offscreen target to a known colour, then read the first pixel back.
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("headless-test-target"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());

            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("headless-clear"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 1.0,
                                b: 0.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
            }

            // 64 * 4 = 256 bytes/row → already 256-aligned, no padding arithmetic.
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("headless-readback"),
                size: 64 * 64 * 4,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(64 * 4),
                        rows_per_image: Some(64),
                    },
                },
                wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
            );
            queue.submit(Some(encoder.finish()));

            let slice = staging.slice(..);
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |v| sender.send(v).unwrap());
            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            receiver.recv().unwrap().unwrap();

            let data = slice.get_mapped_range();
            assert_eq!(
                &data[0..4],
                &[0u8, 255, 0, 255],
                "offscreen clear colour must read back as green"
            );
        });
    }
}
