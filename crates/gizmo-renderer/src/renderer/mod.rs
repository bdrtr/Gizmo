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

/// The context object that lets user code render without ever seeing a `wgpu::CommandEncoder` or
/// a `wgpu::TextureView` directly.
///
/// This is the signature `gizmo_app`'s `set_simple_render` expects:
///
/// ```
/// # use gizmo_core::World;
/// # use gizmo_renderer::RenderContext;
/// # struct GameState;
/// fn render(world: &mut World, _state: &GameState, ctx: &mut RenderContext) {
///     ctx.disable_gpu_compute();           // GPU compute off
///     let _light_time = ctx.light_time();  // scene data — no raw wgpu type in sight
/// #   let _ = world;
/// }
/// # // The `set_simple_render` bound: for<'a> FnMut(&mut World, &State, &mut RenderContext<'a>)
/// # let _: fn(&mut World, &GameState, &mut RenderContext<'_>) = render;
/// ```
///
/// To drive the default render pipeline in one line, use the facade's
/// `RenderContextExt::default_render` extension (the `gizmo` crate, `use gizmo::prelude::*`);
/// this crate does not define that extension itself.
pub struct RenderContext<'a> {
    pub(crate) encoder: &'a mut wgpu::CommandEncoder,
    pub(crate) view: &'a wgpu::TextureView,
    pub(crate) renderer: &'a mut Renderer,
    pub(crate) light_time: f32,
}

impl<'a> RenderContext<'a> {
    /// Creates a new RenderContext (called internally by the engine).
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

    /// Disables the GPU compute subsystems (fluid, particles, physics).
    /// Removes needless GPU work in simple scenes.
    pub fn disable_gpu_compute(&mut self) {
        self.renderer.gpu_fluid = None;
        self.renderer.gpu_particles = None;
        self.renderer.gpu_physics = None;
    }

    /// Returns the current scene light time, in seconds.
    pub fn light_time(&self) -> f32 {
        self.light_time
    }

    /// Direct access to the Renderer (advanced use).
    pub fn renderer(&self) -> &Renderer {
        self.renderer
    }

    /// Mutable access to the Renderer (advanced use).
    pub fn renderer_mut(&mut self) -> &mut Renderer {
        self.renderer
    }

    /// Advanced use: access to the raw wgpu encoder.
    pub fn encoder(&mut self) -> &mut wgpu::CommandEncoder {
        self.encoder
    }

    /// Advanced use: access to the output texture view.
    pub fn output_view(&self) -> &wgpu::TextureView {
        self.view
    }

    /// Simultaneous access to the internals — for passing to functions like
    /// `default_render_pass`.
    pub fn parts_mut(&mut self) -> (&mut wgpu::CommandEncoder, &wgpu::TextureView, &mut Renderer) {
        (self.encoder, self.view, self.renderer)
    }
}

/// The renderer: every GPU resource the engine owns, and the entry points that drive a frame.
///
/// One object rather than a tree of subsystems, because almost every pass needs the device, the
/// queue and the surface configuration, and the optional passes need each other's textures (SSAO
/// reads the G-buffer, TAA reads the composite, FXAA reads TAA's output). The `Option` fields are
/// the passes that can be absent: each is `None` until something enables it, and every one of them
/// is off on the web profile.
///
/// Construction lives in [`construction`](crate::renderer::construction) — [`Renderer::new`] for
/// the windowed path and `new_headless` for a renderer with no surface, which is what the tests
/// and the screenshot path use.
pub struct Renderer {
    // === TEMEL WGPU KAYNAKLARI ===
    /// `None` in headless/offscreen mode (constructed via [`Renderer::new_headless`]);
    /// `Some` on the windowed path. Frame acquisition/present must handle both.
    pub surface: Option<Surface<'static>>,
    /// The wgpu device — everything is allocated from it.
    pub device: Device,
    /// The command queue everything is submitted to.
    pub queue: Queue,
    /// The surface's configuration: format, size and present mode. Kept truthful even headless, so
    /// that attaching a surface later needs no reconciliation.
    pub config: SurfaceConfiguration,
    /// The render resolution in physical pixels. On the web this is capped below the window size —
    /// see `cap_web_render_size`.
    pub size: winit::dpi::PhysicalSize<u32>,
    /// The depth buffer for the main pass, rebuilt on every resize.
    pub depth_texture_view: wgpu::TextureView,

    // === SAHNE (Scene) — Pipeline'lar, Shadow, Skeleton ===
    /// Pipelines, bind-group layouts, the instance buffer, shadow maps and skinning — everything
    /// needed to draw the world itself.
    pub scene: SceneState,

    // === POST-PROCESSING — HDR, Bloom, Blur, Composite ===
    /// The HDR chain: bloom, blur, tone map and composite targets.
    pub post: PostProcessState,

    // === PARTİKÜL SİSTEMİ ===
    /// The GPU particle system, if one was created.
    pub gpu_particles: Option<crate::gpu_particles::GpuParticleSystem>,

    /// The renderer's own GPU rigid-body solver. `None` unless
    /// [`enable_gpu_physics`](Renderer::enable_gpu_physics) was called — it must stay off for a
    /// game using the CPU physics plugin, or the two simulations fight over the same transforms.
    pub gpu_physics: Option<crate::gpu_physics::GpuPhysicsSystem>,

    // === GPU SIVI SİSTEMİ ===
    /// The SPH fluid solver and its surface. Allocated by the windowed path, but only composited
    /// when [`fluid_enabled`](Renderer::fluid_enabled) is set.
    pub gpu_fluid: Option<crate::gpu_fluid::GpuFluidSystem>,
    /// Volumetrik duman (T6, raymarch). Default None; demo `Some(SmokeVolume::new(..))` verir.
    pub smoke: Option<crate::gpu_smoke::SmokeVolume>,

    // === DEFERRED RENDERING — G-Buffer + Lighting pass ===
    /// The G-buffer and the deferred lighting pass. `None` on the web profile, which renders
    /// forward-only.
    pub deferred: Option<crate::deferred::DeferredState>,

    // === SSAO — Screen-Space Ambient Occlusion ===
    /// Screen-space ambient occlusion. Requires [`deferred`](Renderer::deferred).
    pub ssao: Option<crate::ssao::SsaoState>,

    // === Custom materials ===
    /// Materials the game registered — see [`crate::custom_material`].
    ///
    /// Empty by default. A `MaterialType::Custom(id)` whose id is not in here draws nothing rather
    /// than falling back to PBR: a silent fallback is how `routing.rs`'s own module docs describe
    /// two capabilities dying, and an object that vanishes is a question, while an object shaded
    /// as something else is a wrong answer.
    pub custom_materials: crate::custom_material::MaterialRegistry,

    // === SSR — Screen-Space Reflections ===
    /// Screen-space reflections. Requires [`deferred`](Renderer::deferred).
    pub ssr: Option<crate::ssr::SsrState>,

    // === SSGI — Screen-Space Global Illumination ===
    /// Screen-space global illumination. Requires [`deferred`](Renderer::deferred).
    pub ssgi: Option<crate::ssgi::SsgiState>,

    // === Volumetric Lighting (God Rays) ===
    /// Volumetric lighting — the god rays raymarched through the shadow cascades.
    pub volumetric: Option<crate::volumetric::VolumetricState>,

    // === DEFERRED DECALS ===
    /// The deferred decal pass, which projects decals onto the G-buffer.
    pub decal: Option<crate::decal::DecalState>,

    // === TAA — Temporal Anti-Aliasing (ping-pong history + Halton jitter) ===
    /// Temporal anti-aliasing: the history buffers and the Halton jitter sequence.
    pub taa: Option<crate::taa::TaaState>,

    // === FXAA — Fast Approximate Anti-Aliasing (son post-process pass) ===
    /// FXAA, the last pass in the chain. When present and enabled, post-processing writes into
    /// its input texture rather than straight to the swapchain.
    pub fxaa: Option<crate::fxaa::FxaaState>,

    // === GIZMO HATA AYIKLAMA (Debug Lines) ===
    /// The debug line renderer — gizmos, wireframes, physics overlays.
    pub debug_renderer: Option<crate::debug_renderer::GizmoRendererSystem>,

    // === DAHİLİ ASSET YÖNETİCİSİ (Kolaylık metodları için cache) ===
    /// The texture/mesh cache behind the convenience loaders. Behind a lock because the
    /// convenience methods take `&self`, and loading mutates the cache.
    pub asset_manager: std::sync::RwLock<crate::asset::AssetManager>,

    // === WEB PROFİLİ — Platform bazlı GPU kaynak yönetimi ===
    /// Which resource budget this renderer was built for — it is what decides whether the heavy
    /// passes exist at all.
    pub web_profile: crate::web_profile::WebProfile,

    // === RENDER SETTINGS & DIAGNOSTICS ===
    /// Debug shading mode: 0 = shade normally, 1 = world normals, 2 = albedo. Both the forward
    /// and deferred shaders must number these identically — see
    /// [`SceneUniforms::shading_mode`](crate::gpu_types::SceneUniforms::shading_mode).
    pub shading_mode: u32,
    /// The active sky/environment preset.
    pub environment_preset: u32,
    /// The preset being blended towards.
    pub environment_preset_2: u32,
    /// How far between the two: 0 = fully the first, 1 = fully the second.
    pub environment_blend_t: f32,
    /// Which tone-mapping curve the post-process chain applies.
    ///
    /// [`TonemapCurve::Aces`](crate::gpu_types::TonemapCurve::Aces) by default, which is what the
    /// shader hard-coded before this existed — so changing the default would restyle every scene
    /// built on the engine, and does not happen by upgrading.
    pub tonemap_curve: crate::gpu_types::TonemapCurve,
    /// White point for [`TonemapCurve::ReinhardExtended`](crate::gpu_types::TonemapCurve::ReinhardExtended);
    /// ignored by the other curves.
    pub tonemap_white_point: f32,
    /// How much bloom is added back over the frame.
    pub bloom_intensity: f32,
    /// The luminance above which a texel blooms.
    pub bloom_threshold: f32,
    /// Linear exposure multiplier applied before tone mapping.
    pub exposure: f32,
    /// Whether depth of field runs.
    pub dof_enabled: bool,
    /// The distance that is perfectly in focus, in metres.
    pub dof_focus_dist: f32,
    /// How far either side of that stays sharp.
    pub dof_focus_range: f32,
    /// The maximum blur radius outside that range, in texels.
    pub dof_blur_size: f32,
    /// Per-channel radial offset, in screen fractions. 0 = off.
    pub chromatic_aberration: f32,
    /// How dark the frame's corners get, for a camera that carries no
    /// [`PostProcess`](crate::components::PostProcess).
    ///
    /// The component's `vignette` is the authored value and wins where it exists; this is the
    /// fallback, and it exists because there was none: the fallback branch simply did not write
    /// the field, so every ungraded camera inherited `PostProcessUniforms::default()`'s `0.25`
    /// and had no way to change it. Measured before the field existed
    /// (`demo/src/bin/color_grading.rs`): corner/centre sat at 1.206 with no component,
    /// between the 1.344 of an explicit `vignette = 0` and the 0.635 of `vignette = 0.9`.
    ///
    /// Defaults to that same `0.25`, so adding the field changed no pixel — it only made the
    /// value reachable.
    pub vignette_intensity: f32,
    /// Strength of the animated film grain. 0 = off.
    pub film_grain_intensity: f32,
    /// Whether point lights cast shadows this frame.
    pub point_shadows_enabled: bool,
    /// Whether the GPU SPH fluid "ocean" is simulated and composited this frame.
    /// A renderer always allocates a 100k-particle fluid system, but its water
    /// surface must NOT render over every scene — only scenes that actually want
    /// fluid opt in (`ocean_scene`, `fluid_rigid`, …). Off by default so a plain
    /// scene isn't covered by a stray mottled water surface.
    pub fluid_enabled: bool,
}

impl Renderer {
    /// OPT-IN: enable the renderer's own GPU rigid-body physics (default OFF). Idempotent —
    /// safe to call every frame. Only games that drive massive body counts / GPU cloth via
    /// [`GpuPhysicsLink`](gizmo_physics_rigid::components) need this; a normal CPU-physics game
    /// (`PhysicsPlugin`) must NOT enable it — the two sims would fight and blank the scene.
    /// Allocates ~50 000 GPU spheres, so it is off unless explicitly requested.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn enable_gpu_physics(&mut self) {
        if self.gpu_physics.is_some() {
            return;
        }
        let mut physics = crate::gpu_physics::GpuPhysicsSystem::new(
            &self.device,
            50_000,
            &self.scene.global_bind_group_layout,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
        );
        physics.enable_debug(&self.device, 0);
        self.gpu_physics = Some(physics);
    }

    /// Compiles a shader, preferring a copy on disk over the one compiled in.
    ///
    /// `file_path` is tried first and `fallback_src` — normally an `include_str!` — is used when
    /// it is absent. That is what makes shader hot-reload work: drop a copy of a shader into
    /// `demo/assets/shaders/`, edit it, and the studio rebuilds its pipelines on the change. The
    /// repository deliberately ships no such copies, because two versions of a shader free to
    /// drift, with the disk one silently winning, is worse than no hot reload.
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

    /// Recompiles every shader and rebuilds every pipeline — what the studio's file watcher calls
    /// when a `.wgsl` under `demo/assets` changes.
    pub fn rebuild_shaders(&mut self) {
        tracing::info!("🚀 Rebuilding Shaders Pipeline...");
        crate::pipeline::rebuild_pipelines(self);
    }

    /// Grows the instance buffer to hold at least `needed` instances, returning whether it was
    /// reallocated (in which case any bind group referring to the old buffer is stale).
    pub fn ensure_instance_capacity(&mut self, needed: usize) -> bool {
        self.scene.ensure_instance_capacity(&self.device, needed)
    }

    /// Reconfigure the surface with the current config — the first-line recovery when
    /// `Surface::get_current_texture()` returns `Outdated` or `Lost` (wgpu recommends calling
    /// `configure()` again before any heavier recreation). Cheap and idempotent, so it is safe
    /// to call every frame the acquire keeps failing. Unlike [`resize`](Self::resize) it does
    /// NOT rebuild the depth/deferred targets (those are device resources, still valid on a
    /// mere surface loss); it only re-establishes the swapchain so the next frame can present
    /// instead of freezing on a black screen.
    pub fn reconfigure_surface(&self) {
        if let Some(ref surface) = self.surface {
            surface.configure(&self.device, &self.config);
        }
    }

    /// Switches the swapchain's present mode — vsync on or off — at runtime.
    ///
    /// The engine builds its surface with [`wgpu::PresentMode::AutoNoVsync`], i.e. frames are
    /// presented as fast as they are produced. That is the right default for a benchmark and the
    /// wrong one for a laptop on battery, and until now it was not reachable at all: the value
    /// was hard-coded at construction (the campaign record, now ENGINE.md §3).
    ///
    /// `AutoVsync` is the usual opposite choice. Anything the surface does not support falls back
    /// inside wgpu rather than failing here, so this cannot make the swapchain invalid.
    ///
    /// Headless renderers have no surface; the call updates the stored config and returns without
    /// doing anything else, which keeps `config` truthful if a surface is ever attached later.
    ///
    /// Cheap but NOT free: it rebuilds the swapchain. Call it when the user changes a setting,
    /// not every frame.
    pub fn set_present_mode(&mut self, mode: wgpu::PresentMode) {
        if self.config.present_mode == mode {
            return;
        }
        self.config.present_mode = mode;
        if let Some(ref surface) = self.surface {
            surface.configure(&self.device, &self.config);
        }
    }

    /// The swapchain's current present mode. See [`set_present_mode`](Self::set_present_mode).
    pub fn present_mode(&self) -> wgpu::PresentMode {
        self.config.present_mode
    }

    /// Resizes the swapchain and every size-dependent target: depth, G-buffer, decals, fluid.
    ///
    /// The requested size is first put through the web render cap, so a browser canvas growing to
    /// fill its window does not silently rebuild the whole chain at full physical resolution — that
    /// is precisely how the first `Resized` event used to defeat the initial cap.
    ///
    /// A zero width or height is ignored (a minimised window reports one).
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

    /// Runs the post-process chain into `output_view`.
    ///
    /// When FXAA is present and enabled the chain writes into FXAA's input texture and FXAA writes
    /// the result out, so the anti-aliasing sees the final tone-mapped image rather than the HDR
    /// one.
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

    /// Uploads the post-process parameters for this frame.
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
        let expect_scene = 560 + crate::frame_uniforms::MAX_LIGHTS * 64;
        assert_eq!(
            std::mem::size_of::<crate::gpu_types::SceneUniforms>(),
            expect_scene,
            "SceneUniforms size shifted from 560 + 64*MAX_LIGHTS bytes!"
        );
        assert_eq!(std::mem::size_of::<crate::gpu_types::LightData>(), 64, "LightData size shifted from target 64 bytes!");
        // 80 since 2026-08-24: the tone-mapping pair (curve + white point) is a fifth vec4. It was
        // 64 when `underwater` + `fog` were added, and the number is asserted rather than derived
        // so that the *next* field also has to be a deliberate change to this line.
        assert_eq!(std::mem::size_of::<crate::gpu_types::PostProcessUniforms>(), 80, "PostProcessUniforms size shifted from target 80 bytes (tonemap vec4 eklendi)!");
        assert_eq!(std::mem::size_of::<crate::gpu_types::InstanceRaw>(), 128, "InstanceRaw size shifted from target 128 bytes!");
        // Textured-PBR per-material params: three std140 vec4 slots.
        assert_eq!(std::mem::size_of::<crate::gpu_types::MaterialParams>(), 48, "MaterialParams size shifted from target 48 bytes!");
        assert_eq!(std::mem::align_of::<crate::gpu_types::MaterialParams>(), 4, "MaterialParams alignment unexpected!");
        // Field offsets must match the WGSL `MaterialParams` layout in gbuffer.wgsl.
        assert_eq!(std::mem::offset_of!(crate::gpu_types::MaterialParams, emissive_and_normal_scale), 0, "emissive_and_normal_scale must be at offset 0");
        assert_eq!(std::mem::offset_of!(crate::gpu_types::MaterialParams, occlusion_uv_rot_offset), 16, "occlusion_uv_rot_offset must be at offset 16");
        assert_eq!(std::mem::offset_of!(crate::gpu_types::MaterialParams, uv_scale), 32, "uv_scale must be at offset 32");

        // Vertex attribute offsetleri shader VertexInput @location'larıyla (ve
        // Vertex::desc() ile) BİREBİR uyuşmalı. Bir alan kayarsa skinning/tangent
        // bozulur ama toplam boyut değişmeyebilir — bu yüzden offset'leri de kilitle.
        use crate::gpu_types::Vertex;
        assert_eq!(std::mem::offset_of!(Vertex, position), 0);
        // color is RGBA (16 bytes), so everything after it sits 4 bytes later than it used to.
        assert_eq!(std::mem::offset_of!(Vertex, color), 12);
        assert_eq!(std::mem::offset_of!(Vertex, normal), 28);
        assert_eq!(std::mem::offset_of!(Vertex, tex_coords), 40);
        assert_eq!(std::mem::offset_of!(Vertex, joint_indices), 48);
        assert_eq!(std::mem::offset_of!(Vertex, joint_weights), 64);
        assert_eq!(std::mem::offset_of!(Vertex, tangent), 80);
        assert_eq!(std::mem::size_of::<Vertex>(), 96, "Vertex size/layout shifted!");
    }

    /// Regression for M7.0a: the SSGI-apply pass runs at full-res but samples a
    /// half-res buffer. The old shader derived UV as `frag_coord / texture_dims`,
    /// which reaches ~2.0 at the far edge (squeezing GI into the top-left
    /// quarter). The current shader emits a vertex UV that spans exactly [0,1]
    /// across the visible frame (NDC x∈[-1,1] → UV∈[0,1]), independent of the
    /// half-res texture size.
    #[test]
    fn ssgi_apply_uv_covers_full_frame() {
        let full_w = 1920.0f32;
        let half_w = (full_w / 2.0).max(1.0); // matches SsgiState half-res buffer

        // Old (buggy) mapping at the right-most full-res fragment centre.
        let old_uv_x = (full_w - 0.5) / half_w;
        assert!(
            old_uv_x > 1.5,
            "old UV should overshoot [0,1] (was {old_uv_x})"
        );

        // New mapping: fullscreen-triangle NDC x in {-1, 3} → UV via x*0.5+0.5.
        // The screen spans NDC x∈[-1,1] → UV∈[0,1] independent of texture size.
        let ndc_left = -1.0f32;
        let ndc_right = 1.0f32;
        let new_uv_left = ndc_left * 0.5 + 0.5;
        let new_uv_right = ndc_right * 0.5 + 0.5;
        assert_eq!(new_uv_left, 0.0);
        assert_eq!(new_uv_right, 1.0);
    }

    #[test]
    fn test_headless_mipmap_generation() {
        // Bu test kendi wgpu cihazını kuruyor. Guard testin TAMAMI boyunca tutulur —
        // yalnız yaratımı serileştirmek ölçüldü ve yetmedi (bkz. `crate::test_gpu`).
        let _gpu = crate::test_gpu::gpu_lock();
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
                    // wgpu 30. Limit bucketing exists so a browser can stop untrusted content
                    // fingerprinting the GPU through its exact limits. This is a native engine talking to
                    // its own hardware, and bucketing would cost real limits for a threat it does not have.
                    apply_limit_buckets: false,
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
            crate::texture_quality::MipmapBlitter::new(
                &device,
                wgpu::TextureFormat::Rgba8UnormSrgb,
            )
            .generate(&device, &queue, &texture, mip_level_count);

            let _ = device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
        });
    }

    #[test]
    fn new_headless_builds_all_subsystems_and_renders_offscreen() {
        // Bu test kendi wgpu cihazını kuruyor. Guard testin TAMAMI boyunca tutulur —
        // yalnız yaratımı serileştirmek ölçüldü ve yetmedi (bkz. `crate::test_gpu`).
        let _gpu = crate::test_gpu::gpu_lock();
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

            let data = slice.get_mapped_range()
                // wgpu 30 made this fallible; the range is the whole buffer we just mapped, so a
                // failure here is a programming error rather than a runtime condition.
                .expect("a just-mapped buffer's full range is always valid");
            assert_eq!(
                &data[0..4],
                &[0u8, 255, 0, 255],
                "offscreen clear colour must read back as green"
            );
        });
    }
}
