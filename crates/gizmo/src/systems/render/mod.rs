use super::physics::*;
use crate::core::World;
use crate::math::{Mat4, Vec3};
use crate::renderer::{
    components::{Camera, Material, Mesh, MeshRenderer},
    Renderer,
};
use bytemuck;
use wgpu;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WireframeConfig {
    pub global: bool,
}


/// Guarantee every renderable mesh has a current `GlobalTransform` before the
/// draw query runs.
///
/// The draw query below requires `(&Mesh, &GlobalTransform, &Material)` and reads
/// the world matrix from `GlobalTransform`, but physics/gameplay write only the
/// local `Transform`. Without this step a plain `spawn((Transform, Mesh, Material))`
/// renders nothing (the "empty screen" footgun) and callers had to hand-run the
/// transform systems each frame. Here we (1) backfill a `GlobalTransform` onto any
/// mesh that lacks one, then (2) refresh local matrices and propagate them to
/// `GlobalTransform` — the "update transforms right before the pass" TODO.
fn ensure_global_transforms(world: &mut World) {
    use crate::core::query::Without;
    use crate::core::system::System;
    use gizmo_physics_core::components::{GlobalTransform, Transform};

    // Collect first: `add_component` is a structural change and can't run while a
    // query borrow is live.
    let mut missing = Vec::new();
    if let Some(q) = world.query::<(&Mesh, &Transform, Without<GlobalTransform>)>() {
        for (id, _) in q.iter() {
            missing.push(id);
        }
    }
    for id in missing {
        if let Some(e) = world.get_entity(id) {
            world.add_component(e, GlobalTransform::default());
        }
    }

    let mut sync = crate::systems::transform::TransformSyncSystem;
    let mut propagate = crate::systems::transform::TransformPropagateSystem;
    sync.run(world, 0.0);
    propagate.run(world, 0.0);
}

/// Manuel App (`set_setup`/`set_update`/`set_ui`) için TEK-SATIR sahne render kurulumu.
///
/// KÖK-TUZAK ÇÖZÜMÜ: manuel App, `set_render` verilmezse 3B sahneyi ÇİZMEZ (egui HUD
/// görünür ama sahne SİYAH kalır — sessizce). `with_simple_scene` bunu kendi yapar;
/// manuel App için bu uzantı aynısını tek satırda sağlar (ağır/opsiyonel pass'leri —
/// SSR/SSGI/volumetric/TAA + GPU sıvı/fizik — kapatarak; GPU parçacık açık kalır).
///
/// ```ignore
/// use gizmo::systems::AppSceneRenderExt;
/// App::<S>::new(..).add_plugin(TransformPlugin).set_setup(..).set_update(..)
///     .with_scene_render()   // <- bu olmadan ekran siyah
///     .run()
/// ```
pub trait AppSceneRenderExt {
    /// Sahneyi [`default_render_pass`] ile çizecek şekilde `set_render`'ı kurar.
    fn with_scene_render(self) -> Self;
}

impl<State: 'static> AppSceneRenderExt for gizmo_app::App<State> {
    fn with_scene_render(self) -> Self {
        self.set_render(|world, _state, encoder, view, renderer, _light_time| {
            renderer.gpu_fluid = None;
            renderer.gpu_physics = None;
            renderer.ssr = None;
            renderer.ssgi = None;
            renderer.volumetric = None;
            renderer.taa = None;
            default_render_pass(world, encoder, view, renderer);
        })
    }
}

/// Bevy'nin DefaultPlugins davranisini taklit eden, sadece modelleri
/// isiklandirip hizlica ekrana basmaya yarayan kutudan cikmis Render Motoru.
/// Yeni acilan `tut` gibi bos projelerde yuzlerce satir kod yazmamak icin kullanilir.
#[tracing::instrument(skip_all, name = "render_system")]
pub fn default_render_pass(
    world: &mut World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
) {
    // Every renderable object needs an up-to-date `GlobalTransform` (the draw query
    // below requires it, and physics/gameplay only write the local `Transform`).
    // Realize the long-standing "update_transforms right before the pass" TODO here
    // so a caller that just spawned `Transform + Mesh + Material` is not silently
    // culled (the classic "empty screen" footgun) and doesn't have to hand-run the
    // transform systems every frame.
    ensure_global_transforms(world);

    // Post-process params are written AFTER the active camera is resolved (below), so the
    // single exposure knob can be the camera's exposure — see the update_post_process call
    // after camera selection. Exposure is applied ONCE here, over the whole composited HDR
    // (deferred geometry + sky + unlit), instead of being baked per-geometry in the
    // deferred pass and multiplied again by a separate global knob.

    let aspect = if renderer.size.height > 0 {
        renderer.size.width as f32 / renderer.size.height as f32
    } else {
        1.0
    };
    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut cam_pos = Vec3::ZERO;
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);

    // TODO: Bütün nesnelerin (özellikle kamera ve çizilecek objelerin) global matrix'leri
    // bu pass çağrılmadan hemen önce bir `update_transforms(world)` sistemiyle güncellenmiş olmalıdır.

    // ECS veri GPU'ya basılır ve GPU verisi ECS'ye alınır
    gpu_physics_submit_system(world, renderer);
    gpu_physics_readback_system(world, renderer);

    let mut cam_exposure = 1.0;
    // Shadow cascades must follow the ACTIVE camera's near/far/fov, not hardcoded values
    // (otherwise splits/cascade matrices are wrong for any non-default camera).
    let mut cam_near = 0.1f32;
    let mut cam_far = 2000.0f32;
    let mut cam_fov = std::f32::consts::FRAC_PI_4;

    // KAMERALARI BUL VE MATRIX YARAT
    let cameras = world.borrow::<Camera>();
    let global_transforms = world.borrow::<gizmo_physics_core::components::GlobalTransform>();
    let local_transforms = world.borrow::<gizmo_physics_core::components::Transform>();
    {
        // Pick the camera flagged `primary` — the convention maintained by
        // `spawn_camera`/`CameraBundle` (which keep a single primary) and used by
        // the audio listener. Fall back to the first camera if none is marked.
        // This makes selection deterministic instead of depending on the
        // (unstable) ECS iteration order.
        let active_cam = cameras
            .iter()
            .find(|(_, c)| c.primary)
            .or_else(|| cameras.iter().next())
            .map(|(id, _)| id);
        if let Some(active_cam) = active_cam {
            if let Some(cam) = cameras.get(active_cam) {
                // Camera world position: prefer a synced GlobalTransform (needed when the
                // camera is parented), but fall back to the camera's own Transform.position
                // when it has none. Without the fallback a hand-built camera that only got
                // a Transform + Camera (no GlobalTransform) was silently skipped and the
                // view stuck at the origin — nothing read the Transform that gameplay/WASD
                // moved. The transform-propagate system runs in the fixed-step schedule
                // BEFORE the user update, and a custom App may not register it at all, so
                // a camera's GlobalTransform is easily missing or a frame stale; the
                // Transform is written right before render and is always current.
                let pos = global_transforms
                    .get(active_cam)
                    .map(|g| g.matrix.to_scale_rotation_translation().2)
                    .or_else(|| local_transforms.get(active_cam).map(|t| t.position))
                    .unwrap_or(Vec3::ZERO);
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(pos);
                cam_pos = pos;
                cam_forward = cam.get_front();
                cam_exposure = cam.exposure;
                cam_near = cam.near;
                cam_far = cam.far;
                cam_fov = cam.fov;
            }
        }
    }

    // Update post-process params now that the active camera (hence its exposure) is known.
    // `exposure` is the SINGLE exposure knob: the camera's exposure, applied once in the
    // post composite over the entire HDR. (Previously the deferred pass baked cam.exposure
    // into geometry AND post multiplied by a separate 1.15, which compounded and skipped
    // sky/unlit; folding both into one post-stage exposure fixes that.)
    // ── Su-altı atmosferi: kamera bir fluid zone içindeyse derinlik-bazlı sis uygula (W3+W4).
    // W1 `water_at` sorgusu tekrar kullanılır (aynı su hacimleri hem buoyancy hem yüzme hem bu
    // sisi sürer). Sis rengi/yoğunluğu deniz için makul sabitler — demolarda tunable yapılabilir.
    // Sis rengi/yoğunluğu artık kameranın içinde bulunduğu FluidZone'dan gelir (her su hacmi
    // kendi su-altı görünümünü tanımlar) — eskiden burada sabitti.
    let water_sample = world
        .get_resource::<crate::physics::world::PhysicsWorld>()
        .and_then(|pw| pw.water_at(cam_pos));
    let (uw, fog_r, fog_g, fog_b, fog_density) = match water_sample {
        Some(s) => (1.0, s.fog_color[0], s.fog_color[1], s.fog_color[2], s.fog_density),
        None => (0.0, 0.0, 0.0, 0.0, 0.0),
    };

    renderer.update_post_process(
        &renderer.queue,
        crate::renderer::gpu_types::PostProcessUniforms {
            bloom_intensity: renderer.bloom_intensity,
            bloom_threshold: renderer.bloom_threshold,
            exposure: cam_exposure,
            chromatic_aberration: renderer.chromatic_aberration,
            vignette_intensity: 0.25,
            film_grain_intensity: renderer.film_grain_intensity,
            dof_focus_dist: renderer.dof_focus_dist,
            dof_focus_range: renderer.dof_focus_range,
            dof_blur_size: if renderer.dof_enabled { renderer.dof_blur_size } else { 0.0 },
            cam_near,
            cam_far,
            underwater: uw,
            fog_r,
            fog_g,
            fog_b,
            fog_density,
        },
    );

    // Save unjittered projection before applying TAA offset (needed for reprojection next frame).
    let unjittered_proj = proj;

    // ── TAA Halton jitter: subpixel offset applied via z-column of projection ──
    if let Some(ref taa) = renderer.taa {
        if taa.enabled {
            let jp = crate::renderer::taa::TaaState::get_jitter(taa.frame_index);
            // Convert pixel jitter [−0.5, 0.5] to NDC offset (2 / viewport_size per axis)
            let jx = jp[0] * 2.0 / renderer.size.width as f32;
            let jy = jp[1] * 2.0 / renderer.size.height as f32;
            // Adding jitter to NDC.x requires: new_clip.x = clip.x - jx*vz
            // ↔ subtract jx from proj.z_axis.x (the M[0][2] element, row0·col2)
            proj.z_axis.x -= jx;
            proj.z_axis.y -= jy;
        }
    }

    let view_proj = proj * view_mat; // jittered — used for SceneUniforms
    let unjittered_view_proj = unjittered_proj * view_mat; // clean    — stored in TaaState for next frame

    // Lights (point + spot + sun) — collected via the shared setup helper so the
    // game and studio renderers can never drift apart on light handling again.
    let scene_lights = collect_scene_lights(world);
    let sun_dir = scene_lights.sun_dir;
    let sun_col = scene_lights.sun_col;

    // Directional shadow cascades via the shared orchestration helper (SHADOW_DISTANCE
    // cap + CASCADE_LAMBDA + cascade math), so the game and studio paths can't drift on
    // shadow setup. The game always casts from the sun; the studio has its own fallback.
    let cascades =
        crate::renderer::compute_directional_cascades(cam_pos, cam_forward, aspect, cam_fov, cam_near, cam_far, sun_dir);
    let cascade_splits = cascades.splits;
    let cascade_vp = cascades.view_projs;
    let light_view_projs: [[[f32; 4]; 4]; 4] = cascade_vp.map(|m| m.to_cols_array_2d());

    // Dinamik ışıklar (point + spot) shared helper'dan geldi.
    let lights_data = scene_lights.lights;
    let num_lights = scene_lights.num_lights;

    #[allow(unused_assignments)]
    let mut point_light_view_projs = [gizmo_math::Mat4::IDENTITY; 6];
    // Build the point-shadow cube for the ONE designated caster (shared.rs picks the
    // first point light). Take its position/radius from the collected light array so the
    // CPU and the shader agree on which light owns the cube, and so a light with only a
    // Transform (no GlobalTransform) still casts — matching how it is lit.
    if renderer.point_shadows_enabled && scene_lights.shadow_point_index >= 0 {
        let idx = scene_lights.shadow_point_index as usize;
        let lp = lights_data[idx].position;
        let pos = gizmo_math::Vec3::new(lp[0], lp[1], lp[2]);
        // Far plane tracks the light radius (the shader decodes depth with the same far).
        let radius = lights_data[idx].color[3].max(1.0);
        let proj = gizmo_math::Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, radius);
        point_light_view_projs = [
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::X, -gizmo_math::Vec3::Y),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::NEG_X, -gizmo_math::Vec3::Y),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::Y, gizmo_math::Vec3::Z),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::NEG_Y, gizmo_math::Vec3::NEG_Z),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::Z, -gizmo_math::Vec3::Y),
            proj * gizmo_math::Mat4::look_to_rh(pos, gizmo_math::Vec3::NEG_Z, -gizmo_math::Vec3::Y),
        ];

        for (i, view_proj) in point_light_view_projs.iter().enumerate() {
            renderer.queue.write_buffer(
                &renderer.scene.point_shadow_uniform_buffers[i],
                0,
                bytemuck::bytes_of(&crate::renderer::gpu_types::ShadowVsUniform {
                    light_view_proj: view_proj.to_cols_array_2d(),
                }),
            );
        }
    }


    // Elapsed time drives fluid caustics/wave animation in fluid_composite.wgsl
    // (it reads cascade_params.z); this slot was hardcoded to 0.0 → frozen water.
    let elapsed_time = world
        .get_resource::<gizmo_core::time::Time>()
        .map(|t| t.elapsed() as f32)
        .unwrap_or(0.0);
    let scene_uniform_data = crate::renderer::gpu_types::SceneUniforms {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
        // w = "sun present" flag (1.0 / 0.0). Was hardcoded 1.0, which left the deferred
        // shader evaluating the sun branch + a full CSM shadow lookup (against cascades
        // built for a bogus down-vector) even in a scene with no sun. Gate it on has_sun,
        // exactly like the studio path already does.
        sun_direction: [sun_dir.x, sun_dir.y, sun_dir.z, if scene_lights.has_sun { 1.0 } else { 0.0 }],
        sun_color: [sun_col.x, sun_col.y, sun_col.z, sun_col.w],
        lights: lights_data,
        light_view_proj: light_view_projs,
        cascade_splits,
        camera_forward: [cam_forward.x, cam_forward.y, cam_forward.z, 0.0],
        // w = point-shadow caster index + 1 (0 = none); the deferred shader samples the
        // single point-shadow cube only for this light.
        cascade_params: [
            0.1,
            1.0 / crate::renderer::SHADOW_MAP_RES as f32,
            elapsed_time,
            (scene_lights.shadow_point_index + 1).max(0) as f32,
        ],
        num_lights,
        exposure: cam_exposure,
        _pre_align_pad: [0; 2],
        _align_pad: [0; 3],
        environment_blend_t: renderer.environment_blend_t,
        environment_preset: renderer.environment_preset,
        point_shadows_enabled: renderer.point_shadows_enabled as u32,
        environment_preset_2: renderer.environment_preset_2,
        shading_mode: renderer.shading_mode,
        // inverse of the same view_proj written above; hoists the per-fragment 4×4 inverse
        // out of the volumetric/particle fullscreen passes into one CPU compute per frame.
        inv_view_proj: view_proj.inverse().to_cols_array_2d(),
    };
    renderer.queue.write_buffer(
        &renderer.scene.global_uniform_buffer,
        0,
        bytemuck::cast_slice(&[scene_uniform_data]),
    );
    for (i, light_view_proj) in light_view_projs.iter().enumerate() {
        renderer.queue.write_buffer(
            &renderer.scene.shadow_cascade_uniform_buffers[i],
            0,
            bytemuck::bytes_of(&crate::renderer::gpu_types::ShadowVsUniform {
                light_view_proj: *light_view_proj,
            }),
        );
    }

    // Upload TAA params (prev_vp from last frame, current jitter, blend alpha)
    if let Some(ref mut taa) = renderer.taa {
        if taa.enabled {
            let jp = crate::renderer::taa::TaaState::get_jitter(taa.frame_index);
            let jx = jp[0] * 2.0 / renderer.size.width as f32;
            let jy = jp[1] * 2.0 / renderer.size.height as f32;
            let alpha = if taa.frame_index == 0 { 1.0f32 } else { 0.1f32 };
            taa.update_params(&renderer.queue, [jx, jy], alpha);
            taa.store_prev_vp(unjittered_view_proj.to_cols_array_2d());
        }
    }

    // Upload SSGI temporal-accumulation params (mirrors TAA: previous-frame unjittered
    // view-proj for reprojection + blend alpha). alpha=1.0 on the first frame / after a
    // reset so there is no stale history to reproject. Denoises the 1-spp raymarch grain.
    if let Some(ref mut ssgi) = renderer.ssgi {
        let alpha = if ssgi.frame_index == 0 { 1.0f32 } else { 0.1f32 };
        ssgi.update_params(&renderer.queue, alpha);
        ssgi.store_prev_vp(unjittered_view_proj.to_cols_array_2d());
    }

    // CPU batched instancing (replaces the GPU cull): walk the world, frustum-cull, group into
    // instanced batches and upload the instance buffer. Lives in `batching.rs`.
    let (draw_items, uploaded_instances) =
        batching::collect_draw_items(world, renderer, unjittered_view_proj, cascade_vp, cam_pos);

    if let Some(physics) = &renderer.gpu_physics {
        // Her frame başında sıradaki state'i çekmek için WGPU CommandEncoder'a asenkron mapping iste.
        physics.request_readback(encoder);

        physics.compute_pass(encoder);
        physics.debug_compute_pass(encoder);
        physics.cull_pass(encoder, &renderer.scene.global_bind_group);
    }

    // Compute LOD (Level of Detail) Scaling.
    // `fluid_lod == 0` disables the fluid entirely (both `compute_pass` and
    // `render_ssfr` early-return on a zero active count), so a scene that hasn't
    // opted into fluid never simulates or composites the default 100k-particle
    // ocean — previously its SSFR water surface rendered over every scene as a
    // mottled overlay that read like broken shadows.
    let fluid_pos = Vec3::new(0.0, 5.0, 0.0);
    let dist_to_fluid = (cam_pos - fluid_pos).length();
    let fluid_lod = if !renderer.fluid_enabled {
        0.0
    } else if dist_to_fluid < 40.0 {
        1.0
    } else if dist_to_fluid < 80.0 {
        0.5
    } else if dist_to_fluid < 150.0 {
        0.1
    } else {
        0.0
    };

    let dist_to_origin = cam_pos.length();
    let particle_lod = if dist_to_origin < 50.0 {
        1.0
    } else if dist_to_origin < 100.0 {
        0.5
    } else if dist_to_origin < 200.0 {
        0.1
    } else {
        0.0
    };

    // Gpu Fluid Processing
    if let Some(fluid) = &renderer.gpu_fluid {
        let active_fluid = (fluid.num_particles as f32 * fluid_lod) as u32;
        fluid.compute_pass(encoder, &renderer.queue, true, active_fluid);
    }

    // Gpu Particles Processing
    if let Some(particles) = &renderer.gpu_particles {
        let active_parts = (particles.max_particles as f32 * particle_lod) as u32;
        let (dt, time) = world
            .get_resource::<gizmo_core::time::Time>()
            .map(|t| (t.dt(), t.elapsed() as f32))
            .unwrap_or((0.016, 0.0));
        particles.update_params(&renderer.queue, dt, time); // time → curl-noise evrimi
        particles.compute_pass(encoder, active_parts);
    }

    // GPU cull pass removed since we use CPU instancing

    // Resize deferred G-buffers if window changed; resize SSAO + TAA to match
    if let Some(ref mut def) = renderer.deferred {
        def.resize(&renderer.device, renderer.size.width, renderer.size.height);
    }
    {
        let w = renderer.size.width;
        let h = renderer.size.height;
        if let (Some(ssao), Some(def)) = (&mut renderer.ssao, &renderer.deferred) {
            if ssao.width != w || ssao.height != h {
                ssao.resize(&renderer.device, def, w, h);
            }
        }
        if let (Some(ssr), Some(def)) = (&mut renderer.ssr, &renderer.deferred) {
            if ssr.width != w || ssr.height != h {
                ssr.resize(&renderer.device, def, &renderer.post.hdr_texture_view, w, h);
            }
        }
        if let (Some(volumetric), Some(def)) = (&mut renderer.volumetric, &renderer.deferred) {
            if volumetric.width != w || volumetric.height != h {
                volumetric.resize(&renderer.device, def, w, h);
            }
        }
    }
    {
        let w = renderer.size.width;
        let h = renderer.size.height;
        if let (Some(taa), Some(def)) = (&mut renderer.taa, &renderer.deferred) {
            if taa.width != w || taa.height != h {
                taa.resize(
                    &renderer.device,
                    &renderer.post.hdr_texture_view,
                    &def.world_position_view,
                    w,
                    h,
                );
            }
        }
    }

    // Web şemasında gölge yok (4-grup limiti, forward shader'dan shadow örneklemesi
    // `load_shader_web` ile sökülür) — depth-only CSM/point geçitleri boşa GPU olur.
    #[cfg(not(target_arch = "wasm32"))]
    passes::record_shadow_passes(encoder, renderer, &draw_items, uploaded_instances);
    passes::record_deferred_geometry(encoder, renderer, world, &draw_items, uploaded_instances);
    passes::record_ssao(encoder, renderer);
    // CPU-computed inverse of the (unjittered) view-projection for the volumetric smoke raymarch
    // (the WGSL inverse_mat4 returns a wrong inverse for the perspective matrix).
    let inv_view_proj = unjittered_view_proj.inverse().to_cols_array_2d();
    passes::record_forward_and_fluid(
        encoder, renderer, world, &draw_items, uploaded_instances, particle_lod, fluid_lod,
        inv_view_proj,
    );
    passes::record_screen_space_effects(encoder, renderer);
    // Advance SSGI temporal ping-pong / frame counter after its passes have run.
    if let Some(ref mut ssgi) = renderer.ssgi {
        ssgi.advance_frame();
    }
    passes::record_taa_and_overlays(encoder, renderer, world);

    renderer.run_post_processing(encoder, view);
}

// ============================================================
//  RenderContext Kolaylık Metodu
//  `ctx.default_render(world)` ile varsayılan pipeline çalışır.
// ============================================================

/// `RenderContext` üzerine eklenen kolaylık metodları.
/// `use gizmo::prelude::*;` ile otomatik olarak dahil edilir.
pub trait RenderContextExt {
    /// Motorun varsayılan render pipeline'ını çalıştırır.
    /// Deferred rendering, gölgeler, SSAO, SSR, TAA ve post-processing dahildir.
    ///
    /// ```ignore
    /// fn render(world: &mut World, _state: &GameState, ctx: &mut RenderContext) {
    ///     ctx.disable_gpu_compute();
    ///     ctx.default_render(world);
    /// }
    /// ```
    fn default_render(&mut self, world: &mut crate::core::World);
}

impl<'a> RenderContextExt for crate::renderer::RenderContext<'a> {
    fn default_render(&mut self, world: &mut crate::core::World) {
        let (encoder, view, renderer) = self.parts_mut();
        default_render_pass(world, encoder, view, renderer);
    }
}

mod batching;
pub use batching::{clear_render_cache, DrawItem, RenderCache};

mod passes;

mod shared;
pub use shared::{collect_scene_lights, SceneLights};

/// Golden render test: drive the REAL [`default_render_pass`] over a minimal scene
/// (one lit cube + a camera + a sun) into an offscreen target and assert that geometry
/// actually reaches the framebuffer — a sizeable central region must differ from the
/// background. Unlike the renderer's clear-colour readback test, this exercises the full
/// pipeline (cull → batch → shadow/deferred/forward → post), so a regression in the
/// pass-recording split (or any pass) that drops geometry fails here instead of slipping
/// past CI. Needs a GPU adapter; runs in GPU-backed CI/dev.
#[cfg(test)]
mod golden_render_tests {
    use super::default_render_pass;
    use crate::bundles::{CameraBundle, DirectionalLightBundle};
    use crate::core::World;
    use crate::math::{Vec3, Vec4};
    use crate::physics::components::{GlobalTransform, Transform};
    use crate::renderer::asset::AssetManager;
    use crate::renderer::components::{Material, MeshRenderer};
    use crate::renderer::Renderer;

    #[test]
    fn default_render_pass_draws_a_cube_distinct_from_background() {
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping default_render_pass_draws_a_cube_distinct_from_background: \
                 no GPU adapter available (headless render requires a GPU)"
            );
            return;
        }
        pollster::block_on(async {
            const W: u32 = 128;
            const H: u32 = 128;
            const BPP: u32 = 4; // every surface format used here is 4 bytes/pixel

            let mut renderer = Renderer::new_headless(W, H, None).await;
            let mut asset_manager = AssetManager::new();
            let mut world = World::new();

            // --- one cube at the origin (create_cube spans -1..1 → size 2) ---
            let mesh = AssetManager::create_cube(&renderer.device);
            let tex = asset_manager.create_white_texture(
                &renderer.device,
                &renderer.queue,
                &renderer.scene.texture_bind_group_layout,
            );
            let mat = Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0);
            // Deliberately NO GlobalTransform: `default_render_pass` must backfill and
            // sync it from the Transform (the "spawn Transform+Mesh+Material and it just
            // renders" contract — regression guard for the empty-screen footgun).
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::ZERO));
            world.add_component(cube, mesh);
            world.add_component(cube, mat);
            world.add_component(cube, MeshRenderer::new());

            // --- camera on -X looking toward +X (yaw 0 → front = +X), framing the cube ---
            world.spawn_bundle(CameraBundle {
                position: Vec3::new(-6.0, 0.0, 0.0),
                yaw: 0.0,
                pitch: 0.0,
                primary: true,
                ..Default::default()
            });
            // --- a sun so the cube is lit (role = Sun by default) ---
            world.spawn_bundle(DirectionalLightBundle::default());

            // --- run the REAL pipeline into an offscreen target ---
            let format = renderer.config.format;
            let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("golden-target"),
                size: wgpu::Extent3d {
                    width: W,
                    height: H,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

            default_render_pass(&mut world, &mut encoder, &view, &mut renderer);

            // --- copy the result out (W*BPP = 512 → already 256-aligned) ---
            let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("golden-readback"),
                size: (W * H * BPP) as u64,
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
                        bytes_per_row: Some(W * BPP),
                        rows_per_image: Some(H),
                    },
                },
                wgpu::Extent3d {
                    width: W,
                    height: H,
                    depth_or_array_layers: 1,
                },
            );
            renderer.queue.submit(Some(encoder.finish()));

            let slice = staging.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
            let _ = renderer.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            });
            rx.recv().unwrap().unwrap();
            let data = slice.get_mapped_range();

            let px = |x: u32, y: u32| -> [u8; 4] {
                let i = ((y * W + x) * BPP) as usize;
                [data[i], data[i + 1], data[i + 2], data[i + 3]]
            };
            let background = px(2, 2); // a corner — the cube never reaches here
            let centre = px(W / 2, H / 2);
            assert_ne!(
                centre, background,
                "centre pixel equals the corner/background — default_render_pass drew no geometry"
            );

            // the cube should cover a sizeable central region, not a stray pixel
            let mut differing = 0u32;
            for y in 0..H {
                for x in 0..W {
                    if px(x, y) != background {
                        differing += 1;
                    }
                }
            }
            let frac = differing as f32 / (W * H) as f32;
            assert!(
                frac > 0.05,
                "only {:.1}% of pixels differ from the background; the lit cube should fill a \
                 sizeable central region (regression dropping geometry?)",
                frac * 100.0
            );
        });
    }

    /// Render the standard lit cube at a given camera `exposure` and return the mean of all
    /// RGB bytes in the frame. Shared by the exposure invariant test below.
    async fn render_mean_brightness(exposure: f32) -> f32 {
        const W: u32 = 128;
        const H: u32 = 128;
        const BPP: u32 = 4;

        let mut renderer = Renderer::new_headless(W, H, None).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let mesh = AssetManager::create_cube(&renderer.device);
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0);
        let cube = world.spawn();
        world.add_component(cube, Transform::new(Vec3::ZERO));
        world.add_component(cube, GlobalTransform::default());
        world.add_component(cube, mesh);
        world.add_component(cube, mat);
        world.add_component(cube, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-6.0, 0.0, 0.0),
            yaw: 0.0,
            pitch: 0.0,
            primary: true,
            exposure,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        let format = renderer.config.format;
        let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("exposure-target"),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = renderer
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        default_render_pass(&mut world, &mut encoder, &view, &mut renderer);

        let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("exposure-readback"),
            size: (W * H * BPP) as u64,
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
                    bytes_per_row: Some(W * BPP),
                    rows_per_image: Some(H),
                },
            },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );
        renderer.queue.submit(Some(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| tx.send(v).unwrap());
        let _ = renderer.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();

        // Mean of R,G,B over the whole frame (alpha excluded).
        let mut sum = 0u64;
        for i in (0..(W * H * BPP) as usize).step_by(BPP as usize) {
            sum += data[i] as u64 + data[i + 1] as u64 + data[i + 2] as u64;
        }
        sum as f32 / (W * H * 3) as f32
    }

    /// Exposure is a SINGLE post-process knob applied over the whole composited HDR (the
    /// deferred pass no longer bakes it in). This guards that rework: a higher camera exposure
    /// must brighten the frame. If exposure were detached (or the deferred→post move dropped
    /// the wiring), the two renders would match and this fails. (Tone-mapping is non-linear so
    /// we assert monotonic increase, not an exact 2x.)
    #[test]
    fn camera_exposure_brightens_the_frame() {
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping camera_exposure_brightens_the_frame: no GPU adapter available");
            return;
        }
        pollster::block_on(async {
            let dim = render_mean_brightness(1.0).await;
            let bright = render_mean_brightness(2.0).await;
            assert!(
                bright > dim + 1.0,
                "higher camera exposure must brighten the scene, but exp=1.0 mean={dim:.2} \
                 vs exp=2.0 mean={bright:.2} (exposure not applied / detached from post?)"
            );
        });
    }
}
