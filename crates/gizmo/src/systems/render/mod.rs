#[cfg(feature = "physics")]
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

/// ONE-LINE scene render setup for a manual App (`set_setup`/`set_update`/`set_ui`).
///
/// ROOT-FOOTGUN SOLUTION: a manual App DOES NOT DRAW the 3D scene if `set_render` is not
/// given (the egui HUD is visible but the scene stays BLACK — silently). `with_simple_scene`
/// does this itself; for a manual App this extension provides the same in one line (by
/// turning the heavy/optional passes — SSR/SSGI/volumetric/TAA + GPU fluid/physics — off;
/// GPU particles stay on).
///
/// ```no_run
/// use gizmo::prelude::*;
/// use gizmo::systems::AppSceneRenderExt;
///
/// App::<()>::new("Demo", 1280, 720)
///     .add_plugin(TransformPlugin)
///     .set_setup(|_world, _renderer| {})
///     .set_update(|_world, _state, _dt, _input| {})
///     .with_scene_render()   // <- bu olmadan ekran siyah
///     .run()
///     .expect("the application failed to run");
/// ```
pub trait AppSceneRenderExt {
    /// Sets `set_render` up so that the scene is drawn with [`default_render_pass`].
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

/// An out-of-the-box Render Engine that mimics Bevy's DefaultPlugins behavior, serving only
/// to light the models and put them on screen quickly.
/// It is used to avoid writing hundreds of lines of code in freshly opened, empty projects
/// like `tut`.
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

    // **Advance skeletal animation.** `collect_draw_items` below reads `Skeleton` for its skinning
    // matrices, and until this call nothing in the engine ever advanced the pose it reads: the two
    // systems live in `gizmo-renderer`, `current_time += dt · speed` appears nowhere else in the
    // workspace, and no schedule, plugin or demo invoked either of them. The engine was drawing a
    // pose it never stepped.
    //
    // They are called from the render pass rather than from a schedule because of their
    // signatures: both need a `wgpu::Queue` to upload the skin matrices, which no ordinary system
    // has, and that is very likely why they were never wired anywhere. Here is the one place that
    // holds the world, the queue, and a position before the draw path reads the result.
    //
    // Player first, state machine second: an entity carrying both is a caller error, and if one
    // has to win it should be the higher-level driver.
    let animation_dt = world
        .get_resource::<gizmo_core::time::Time>()
        .map(|t| t.dt())
        .unwrap_or(0.0);
    if animation_dt > 0.0 {
        crate::renderer::animation_update_system(world, animation_dt, &renderer.queue);
        crate::renderer::animation_state_machine_update_system(
            world,
            animation_dt,
            &renderer.queue,
        );
    }

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

    // ECS veri GPU'ya basılır ve GPU verisi ECS'ye alınır (GPU-fizik yolu — `physics` ister)
    #[cfg(feature = "physics")]
    {
        gpu_physics_submit_system(world, renderer);
        gpu_physics_readback_system(world, renderer);
    }

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
    // Underwater fog comes from the FluidZone the camera sits in, which lives in the physics
    // world; without the `physics` feature there are no fluid volumes, so the scene simply
    // never renders as submerged.
    #[cfg(feature = "physics")]
    let (uw, fog_r, fog_g, fog_b, fog_density) = match world
        .get_resource::<crate::physics::world::PhysicsWorld>()
        .and_then(|pw| pw.water_at(cam_pos))
    {
        Some(s) => (1.0, s.fog_color[0], s.fog_color[1], s.fog_color[2], s.fog_density),
        None => (0.0, 0.0, 0.0, 0.0, 0.0),
    };
    #[cfg(not(feature = "physics"))]
    let (uw, fog_r, fog_g, fog_b, fog_density) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);

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
        // Fill it from the scene. Without this the engine steps and draws a particle set that
        // nothing ever puts anything into: the emitter-to-GPU bridge existed only inside
        // `gizmo-studio`, so a `ParticleEmitter` on an entity emitted nothing for anyone using
        // this pass. See `particles.rs`.
        spawn_from_emitters(world, particles, &renderer.queue, dt);
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
    passes::record_deferred_geometry(
        encoder,
        renderer,
        world,
        &draw_items,
        uploaded_instances,
        cam_pos,
    );
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

/// Convenience methods added on top of `RenderContext`.
/// Automatically included with `use gizmo::prelude::*;`.
pub trait RenderContextExt {
    /// Runs the engine's default render pipeline.
    /// Deferred rendering, shadows, SSAO, SSR, TAA and post-processing are included.
    ///
    /// ```
    /// use gizmo::prelude::*;
    /// # struct GameState;
    /// fn render(world: &mut World, _state: &GameState, ctx: &mut RenderContext) {
    ///     ctx.disable_gpu_compute();
    ///     ctx.default_render(world);
    /// }
    /// # // The `App::set_simple_render` bound: for<'a> FnMut(&mut World, &State, &mut RenderContext<'a>)
    /// # let _: fn(&mut World, &GameState, &mut RenderContext<'_>) = render;
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

mod particles;
pub use particles::spawn_from_emitters;

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

    /// The same scene renders the same whether it sits at the world origin or two kilometres away.
    ///
    /// It did not. The G-buffer's world-position target is `Rgba16Float`, and it held **absolute**
    /// coordinates: f16 quantises to 6 cm at 100 m from the origin, 50 cm at 1 km and a full metre
    /// at 2 km, against a nearest-cascade shadow texel of 4.3 mm. A city-sized level was therefore
    /// sampling its shadows, its view vector and its fog from a position rounded to the nearest
    /// half-metre — invisible near the origin, which is where every other test in this file sits,
    /// and worse the further out you built.
    ///
    /// The target cannot be widened: the four G-buffer attachments share a 32-byte-per-sample
    /// budget and are at 28. But the budget is about the bytes, not about what goes in them, so
    /// the position is stored relative to the camera now and every reader adds it back. Same eight
    /// bytes, values at view scale, centimetres anywhere in the world.
    ///
    /// Two kilometres is chosen to be where f16 costs a whole metre. The tolerance is loose on
    /// purpose — this asks whether the picture *survives the translation*, not whether two GPU
    /// runs are bit-identical.
    #[test]
    fn a_scene_renders_the_same_two_kilometres_from_the_origin() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping a_scene_renders_the_same_two_kilometres_from_the_origin: no GPU");
            return;
        }
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_scene_renders_the_same_two_kilometres_from_the_origin: software");
            return;
        }
        pollster::block_on(async {
            let near = render_translated(Vec3::ZERO).await;
            let far = render_translated(Vec3::new(2000.0, 0.0, 2000.0)).await;
            assert_eq!(near.len(), far.len(), "same target size");

            let differing = near
                .iter()
                .zip(far.iter())
                .filter(|(a, b)| a.abs_diff(**b) > 8)
                .count();
            let ratio = differing as f32 / near.len() as f32;
            assert!(
                ratio < 0.02,
                "{:.1}% of the frame changed when the whole scene moved 2 km from the origin — \
                 the renderer is reading positions whose precision depends on where the level was \
                 built",
                100.0 * ratio
            );
        });
    }

    /// The cube scene of [`render_frame_with_mesh`], with everything — camera, cube and all —
    /// shifted by `offset`. Shifting *both* is what makes the comparison about world-origin
    /// distance and nothing else: the camera sees exactly the same thing either way.
    async fn render_translated(offset: Vec3) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;
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
        world.add_component(cube, Transform::new(offset));
        world.add_component(cube, mesh);
        world.add_component(cube, mat);
        world.add_component(cube, MeshRenderer::new());

        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-6.0, 0.0, 0.0) + offset,
            yaw: 0.0,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// Two different dark materials must not render identically.
    ///
    /// They did. The G-buffer's albedo target was `Rgba8Unorm` holding **linear** albedo, and
    /// linear 8-bit has almost no resolution where the eye has most: the whole perceptual range
    /// 0–32/255 gets **4 codes** in a linear target against 32 in an sRGB one, and the very first
    /// linear code already sits at a perceptual 12.7/255. Albedo 0.004 and 0.0045 both landed on
    /// code 1 and came out **byte-identical**, which is what this test measured before the format
    /// changed: `0 bytes differ`. It matters here more than it would in most engines — this one's
    /// flagship level is a night city whose frame medians sit at 1–14/255, i.e. entirely inside
    /// the range those four codes have to cover.
    ///
    /// The obvious version of this test does not work, and it is worth saying why: comparing two
    /// albedos the linear format *can* separate (0.004 and 0.006) makes the **linear** target look
    /// better, 2923 differing bytes against sRGB's 1649. That is not signal. Linear rounds 0.004
    /// down and 0.006 up, so it renders them two codes apart when they are one and a half apart —
    /// the extra difference is quantisation error, not detail. Only a pair the old format could
    /// not separate at all distinguishes precision from noise.
    #[test]
    fn two_different_dark_materials_do_not_render_identically() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping two_different_dark_materials_do_not_render_identically: no GPU");
            return;
        }
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping two_different_dark_materials_do_not_render_identically: software");
            return;
        }
        pollster::block_on(async {
            let a = render_dark(0.004).await;
            let b = render_dark(0.0045).await;
            let differing = a.iter().zip(b.iter()).filter(|(x, y)| x != y).count();
            assert!(
                differing > 0,
                "albedo 0.004 and 0.0045 rendered byte-identically — the G-buffer cannot hold the \
                 difference, so every dark material in this range is the same material"
            );
        });
    }

    async fn render_dark(albedo: f32) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;
        let mut renderer = Renderer::new_headless(W, H, None).await;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();
        let mesh = AssetManager::create_cube(&renderer.device);
        let tex = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );
        let mat = Material::new(tex).with_pbr(Vec4::new(albedo, albedo, albedo, 1.0), 0.0, 1.0);
        let cube = world.spawn();
        world.add_component(cube, Transform::new(Vec3::ZERO));
        world.add_component(cube, mesh);
        world.add_component(cube, mat);
        world.add_component(cube, MeshRenderer::new());
        world.spawn_bundle(CameraBundle {
            position: Vec3::new(-6.0, 0.0, 0.0),
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());
        render_world(&mut renderer, &mut world).await
    }

    /// Every pipeline this engine builds compiles on whatever backend is present.
    ///
    /// **Deliberately has no software-adapter guard, unlike every other test in this module.**
    /// That is the whole point of it. A backend compiles naga's generated target language with
    /// its own compiler — FXC on D3D12, the Metal compiler on macOS — and those reject things
    /// that are perfectly good WGSL. On 2026-08-14 the shadow PCF's `textureSampleCompare`, an
    /// implicit-derivative sample inside a conditional branch, was "gradient instruction used in
    /// a loop with varying iteration" to FXC, and the Deferred Lighting pipeline never built: the
    /// engine drew nothing at all on Windows. `gizmo-renderer`'s shader tests had been green
    /// throughout, because they type-check WGSL through naga and never reach a backend compiler.
    ///
    /// The only thing that catches this is creating the pipelines on that backend, and creating
    /// them is fast even on WARP — it is *rendering* through them that a software rasteriser
    /// cannot finish, which is why the rest of this module skips and this does not.
    #[test]
    fn every_pipeline_compiles_on_this_backend() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping every_pipeline_compiles_on_this_backend: no GPU adapter");
            return;
        }
        pollster::block_on(async {
            // Constructing the renderer is what builds them: deferred, gbuffer, both shadow
            // passes, forward, post-process, ssao/ssr/ssgi, the particle and fluid compute
            // pipelines. A backend rejecting any one of them fails here.
            let renderer = Renderer::new_headless(64, 64, None).await;
            assert!(
                renderer.surface.is_none(),
                "the headless renderer must have no surface"
            );
        });
    }

    /// The pass advances skeletal animation.
    ///
    /// It did not until 2026-08-14, and nothing noticed for a long time. `animation_update_system`
    /// and `animation_state_machine_update_system` were written, exported and documented as the
    /// thing that steps a player's clock, and no schedule, plugin, app or demo ever called either
    /// of them — `current_time += dt · speed` appears nowhere else in the workspace, so a skinned
    /// mesh rendered its bind pose for ever. The draw path *reads* `Skeleton` for its skinning
    /// matrices, which is what made the omission invisible: everything looked wired.
    ///
    /// So this asserts the wiring rather than the arithmetic. The arithmetic already has tests
    /// (`gizmo-renderer`'s `normalize_anim_time` covers looping, clamping and zero duration) and
    /// they all passed the entire time the feature was dead.
    #[test]
    fn default_render_pass_advances_skeletal_animation() {
        use crate::renderer::components::{AnimationClip, AnimationPlayer, SkeletonHierarchy};
        use gizmo_animation::skeletal::SkeletonJoint;
        use std::sync::Arc;

        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping default_render_pass_advances_skeletal_animation: no GPU adapter"
            );
            return;
        }
        // WARP and friends can create these pipelines in seconds and cannot finish
        // rendering through them: `windows-latest` spent five and a half hours on this
        // file once the D3D12 shader fix let it get this far. Pipeline compilation is
        // the coverage that matters on such a runner and it is kept by
        // `every_pipeline_compiles_on_this_backend`, which builds the whole renderer.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping default_render_pass_advances_skeletal_animation: software adapter — see the note above");
            return;
        }
        pollster::block_on(async {
            let mut renderer = Renderer::new_headless(64, 64, None).await;
            let mut world = World::new();

            // A clock with a real delta. The pass reads `Time::dt()`, and a world without the
            // resource reads 0.0 and advances nothing — which is correct, and would also make
            // this test pass for the wrong reason if the resource were left out.
            let mut time = crate::core::time::Time::new();
            time.update(1.0 / 60.0);
            world.insert_resource(time);

            // One joint, one clip, one second long, empty tracks: the pose it evaluates to does
            // not matter here, only that the clock moves.
            let hierarchy = Arc::new(SkeletonHierarchy {
                joints: vec![SkeletonJoint {
                    name: "root".into(),
                    node_index: 0,
                    inverse_bind_matrix: crate::math::Mat4::IDENTITY,
                    parent_index: None,
                    local_bind_transform: crate::math::Mat4::IDENTITY,
                    bind_translation: Vec3::ZERO,
                    bind_rotation: crate::math::Quat::IDENTITY,
                    bind_scale: Vec3::ONE,
                }],
                root_transform: crate::math::Mat4::IDENTITY,
            });
            let clip = AnimationClip {
                name: "idle".into(),
                duration: 1.0,
                translations: Vec::new(),
                rotations: Vec::new(),
                scales: Vec::new(),
            };
            let rig = world.spawn();
            world.add_component(rig, Transform::new(Vec3::ZERO));
            world.add_component(rig, renderer.create_skeleton(hierarchy));
            world.add_component(
                rig,
                AnimationPlayer {
                    current_time: 0.0,
                    active_animation: 0,
                    loop_anim: true,
                    speed: 1.0,
                    animations: Arc::from(vec![clip]),
                    blend_time: 0.0,
                    blend_duration: 0.0,
                    prev_animation: None,
                    prev_time: 0.0,
                },
            );

            world.spawn_bundle(CameraBundle {
                position: Vec3::new(-6.0, 0.0, 0.0),
                primary: true,
                ..Default::default()
            });

            let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("anim_target"),
                size: wgpu::Extent3d {
                    width: 64,
                    height: 64,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: renderer.config.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

            default_render_pass(&mut world, &mut encoder, &view, &mut renderer);
            renderer.queue.submit(Some(encoder.finish()));

            let advanced = world
                .borrow::<AnimationPlayer>()
                .get(rig.id())
                .expect("the player is still there")
                .current_time;
            assert!(
                advanced > 0.0,
                "default_render_pass left current_time at {advanced} — nothing is advancing \
                 skeletal animation, which is the state the engine shipped in until this test"
            );
        });
    }

    #[test]
    fn default_render_pass_draws_a_cube_distinct_from_background() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping default_render_pass_draws_a_cube_distinct_from_background: \
                 no GPU adapter available (headless render requires a GPU)"
            );
            return;
        }
        // WARP and friends can create these pipelines in seconds and cannot finish
        // rendering through them: `windows-latest` spent five and a half hours on this
        // file once the D3D12 shader fix let it get this far. Pipeline compilation is
        // the coverage that matters on such a runner and it is kept by
        // `every_pipeline_compiles_on_this_backend`, which builds the whole renderer.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping default_render_pass_draws_a_cube_distinct_from_background: software adapter — see the note above");
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

    /// Render the standard lit cube through the real pipeline and return the frame's bytes.
    ///
    /// `point_shadows` sets [`Renderer::point_shadows_enabled`], which gates both the six
    /// point-shadow face passes and the shader's lookup into the cubemap they write.
    async fn render_frame(exposure: f32, point_shadows: bool) -> Vec<u8> {
        render_frame_with_mesh(exposure, point_shadows, AssetManager::create_cube).await
    }

    /// As [`render_frame`], but the caller supplies the geometry.
    ///
    /// The seam exists so two frames can differ **only** in how the same triangles reach the
    /// GPU — flat vertex list versus deduplicated vertices plus an index buffer. Everything
    /// downstream (camera, light, material, passes, readback) stays byte-for-byte the same
    /// code path, which is what makes an equality assertion on the two frames meaningful.
    async fn render_frame_with_mesh(
        exposure: f32,
        point_shadows: bool,
        make_mesh: impl FnOnce(&wgpu::Device) -> crate::renderer::components::Mesh,
    ) -> Vec<u8> {
        // The offscreen target `render_world` renders into is the same size; these size the
        // renderer itself.
        const W: u32 = 128;
        const H: u32 = 128;

        let mut renderer = Renderer::new_headless(W, H, None).await;
        renderer.point_shadows_enabled = point_shadows;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let mesh = make_mesh(&renderer.device);
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

        render_world(&mut renderer, &mut world).await
    }

    /// Drive the REAL [`default_render_pass`] over `world` into a 128×128 offscreen target and
    /// read the frame back as RGBA8 bytes.
    ///
    /// Extracted so every golden test renders through byte-for-byte the same code path — the
    /// point of comparing two frames is that nothing between the scene and the bytes differs.
    async fn render_world(renderer: &mut Renderer, world: &mut World) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;
        const BPP: u32 = 4;

        let format = renderer.config.format;
        let target = renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("frame-target"),
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
        default_render_pass(world, &mut encoder, &view, renderer);

        let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame-readback"),
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

        data.to_vec()
    }

    /// A unit cube as a flat triangle list: 36 vertices, six per face, each face carrying its
    /// own normal — so the only true duplicates are the two shared corners *within* a face,
    /// and a correct dedup lands on 24.
    ///
    /// Written out here rather than reusing [`AssetManager::create_cube`] because this test
    /// needs the VERTICES and that function only hands back a finished `Mesh`.
    fn cube_vertices() -> Vec<crate::renderer::gpu_types::Vertex> {
        const CORNERS: [[f32; 3]; 8] = [
            [-1.0, -1.0, -1.0],
            [1.0, -1.0, -1.0],
            [1.0, 1.0, -1.0],
            [-1.0, 1.0, -1.0],
            [-1.0, -1.0, 1.0],
            [1.0, -1.0, 1.0],
            [1.0, 1.0, 1.0],
            [-1.0, 1.0, 1.0],
        ];
        // (two triangles as corner indices, face normal)
        const FACES: [([usize; 6], [f32; 3]); 6] = [
            ([1, 0, 3, 1, 3, 2], [0.0, 0.0, -1.0]),
            ([4, 5, 6, 4, 6, 7], [0.0, 0.0, 1.0]),
            ([0, 4, 7, 0, 7, 3], [-1.0, 0.0, 0.0]),
            ([5, 1, 2, 5, 2, 6], [1.0, 0.0, 0.0]),
            ([3, 7, 6, 3, 6, 2], [0.0, 1.0, 0.0]),
            ([0, 1, 5, 0, 5, 4], [0.0, -1.0, 0.0]),
        ];
        const UVS: [[f32; 2]; 6] = [
            [0.0, 1.0],
            [1.0, 1.0],
            [1.0, 0.0],
            [0.0, 1.0],
            [1.0, 0.0],
            [0.0, 0.0],
        ];

        let mut out = Vec::with_capacity(36);
        for (corner_indices, normal) in FACES {
            for (k, corner) in corner_indices.into_iter().enumerate() {
                out.push(crate::renderer::gpu_types::Vertex {
                    position: CORNERS[corner],
                    color: [1.0, 1.0, 1.0, 1.0],
                    normal,
                    tex_coords: UVS[k],
                    ..Default::default()
                });
            }
        }
        out
    }

    /// The indexed draw path must produce the same picture as the flat one, to the byte.
    ///
    /// This is the only test that executes `draw_indexed` at all. Until it existed, both
    /// `Mesh::new_indexed`'s deduplication and the `record_draw` index branch were code that
    /// compiled and had never once run: the engine's only producer of indexed meshes is the
    /// glTF loader, and this repository ships no `.glb` (see `assets/README.md` — the models
    /// are third-party licensed and uncommitted), so every mesh in every other test is flat.
    ///
    /// Byte equality rather than a similarity threshold is the point. A wrong index buffer
    /// does not draw *nothing* — it draws the same triangle count from the wrong corners, and
    /// at 128×128 with one lit cube that can still look plausible. Only exact equality
    /// distinguishes "the geometry survived the round trip" from "something reasonable
    /// appeared on screen".
    #[test]
    fn an_indexed_mesh_renders_byte_identically_to_the_flat_one() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping an_indexed_mesh_renders_byte_identically_to_the_flat_one: \
                 no GPU adapter available"
            );
            return;
        }
        // WARP and friends can create these pipelines in seconds and cannot finish
        // rendering through them: `windows-latest` spent five and a half hours on this
        // file once the D3D12 shader fix let it get this far. Pipeline compilation is
        // the coverage that matters on such a runner and it is kept by
        // `every_pipeline_compiles_on_this_backend`, which builds the whole renderer.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping an_indexed_mesh_renders_byte_identically_to_the_flat_one: software adapter — see the note above");
            return;
        }
        pollster::block_on(async {
            let vertices = cube_vertices();
            assert_eq!(vertices.len(), 36, "cube_vertices is meant to be a flat triangle list");

            let flat_verts = vertices.clone();
            let flat = render_frame_with_mesh(1.0, false, move |device| {
                crate::renderer::components::Mesh::from_vertices(device, &flat_verts, "flat_cube")
            })
            .await;

            let indexed_verts = vertices.clone();
            let indexed = render_frame_with_mesh(1.0, false, move |device| {
                let mesh = crate::renderer::components::Mesh::new_indexed(
                    device,
                    &indexed_verts,
                    Vec3::ZERO,
                    "indexed_cube".to_string(),
                );
                // Guard against a vacuous pass: if `new_indexed` ever stopped producing an
                // index buffer (or stopped deduplicating), the frames below would still match
                // — because both would be flat — and this test would silently stop testing
                // anything at all.
                assert!(
                    mesh.ibuf.is_some(),
                    "new_indexed produced no index buffer; the frame comparison would be vacuous"
                );
                assert_eq!(
                    mesh.index_count, 36,
                    "every original vertex must still be referenced exactly once"
                );
                assert_eq!(
                    mesh.vertex_count, 24,
                    "a cube with per-face normals has 24 distinct vertices; got {} \
                     (deduplication did not run, or merged across faces)",
                    mesh.vertex_count
                );
                // 24 unique vertices is far below the 65536 that `Uint16` can address, so this
                // mesh MUST take the narrow path — which is what makes the frame comparison
                // below a test of 16-bit indices rather than only of 32-bit ones. A buffer
                // written as u16 and bound as u32 does not crash; it draws the wrong triangles,
                // and only the byte comparison would catch it.
                assert_eq!(
                    mesh.index_format,
                    wgpu::IndexFormat::Uint16,
                    "a 24-vertex mesh must use 16-bit indices"
                );
                mesh
            })
            .await;

            assert_eq!(flat.len(), indexed.len(), "frame sizes differ");
            let differing = flat
                .iter()
                .zip(indexed.iter())
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(
                differing,
                0,
                "{differing} of {} bytes differ between the flat and indexed renders of the \
                 same cube — the index buffer, its format, or the draw_indexed range is wrong",
                flat.len()
            );
        });
    }

    // ── ITEM 7: the painted-backdrop path, rendered ────────────────────────────────────────
    //
    // These are the only tests in the tree that can see what a backdrop actually looks like.
    // Everything else about `MaterialType::Backdrop` is pinned as arithmetic or as pipeline
    // state (`gizmo_renderer::backdrop`, `batching::cmp_draw_order`); this drives the real
    // `default_render_pass` and reads the pixels back.

    /// How the full-screen test panel is materialised. The three arms are the three answers
    /// the engine can give to "draw my sky geometry", and the test compares them directly.
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum PanelKind {
        /// The new path: the mesh's own pixels, camera-locked, behind everything.
        Backdrop,
        /// What the game ships with today's engine when it wants the artwork: correct pixels,
        /// but ordinary world geometry — it writes depth and stands in front of the world.
        Unlit,
        /// The other half of the report: correct depth, but the mesh's texture and vertex
        /// colour are discarded for an invented gradient.
        Skybox,
    }

    /// A screen-filling quad in the YZ plane (its normal points down −X, at a camera looking
    /// along +X), 8 units across, carrying `colour` as its VERTEX colour and a full 0..1 UV
    /// span.
    ///
    /// The colour rides the vertex attribute rather than the material albedo on purpose: it is
    /// the channel `sky.wgsl` throws away, so a green pixel on screen is evidence the mesh's
    /// own data survived to the framebuffer.
    fn panel_vertices(colour: [f32; 4]) -> Vec<crate::renderer::gpu_types::Vertex> {
        const R: f32 = 4.0;
        let v = |y: f32, z: f32, u: f32, w: f32| crate::renderer::gpu_types::Vertex {
            position: [0.0, y, z],
            color: colour,
            normal: [-1.0, 0.0, 0.0],
            tex_coords: [u, w],
            ..Default::default()
        };
        vec![
            v(-R, -R, 0.0, 1.0),
            v(-R, R, 1.0, 1.0),
            v(R, R, 1.0, 0.0),
            v(-R, -R, 0.0, 1.0),
            v(R, R, 1.0, 0.0),
            v(R, -R, 0.0, 0.0),
        ]
    }

    /// The full-screen test panel: how it is drawn, what texture it carries, and — for the
    /// world-space kinds only — where it is nailed down.
    #[derive(Clone, Copy)]
    struct Panel {
        kind: PanelKind,
        /// `false` = the 1×1 white texture, `true` = the 256² checkerboard.
        checkered: bool,
        /// The world point the panel sits two units in front of.
        ///
        /// Ignored for [`PanelKind::Backdrop`], whose transform is camera-relative BY
        /// CONSTRUCTION — that asymmetry is the property under test, not an oversight. Pass
        /// the camera position to put a world-space panel where a backdrop would appear;
        /// pass a fixed point to leave one behind as the camera drives away.
        anchor: Vec3,
    }

    /// The standard red world cube (optional) plus a green screen-filling panel (optional),
    /// rendered from `camera_pos` looking along +X.
    async fn render_panel_scene(
        camera_pos: Vec3,
        with_cube: bool,
        panel: Option<Panel>,
    ) -> Vec<u8> {
        const W: u32 = 128;
        const H: u32 = 128;

        let mut renderer = Renderer::new_headless(W, H, None).await;
        // Screen-space filters off. Each of them re-derives world positions and ray
        // directions from the camera, so each makes the final image depend on where the
        // camera IS — which would drown out the question these tests ask, namely whether the
        // BACKDROP moved. None of them touches the backdrop's own shading: they read the
        // G-buffer, which a backdrop never writes to.
        renderer.taa = None;
        renderer.ssr = None;
        renderer.ssgi = None;
        renderer.ssao = None;
        let mut asset_manager = AssetManager::new();
        let mut world = World::new();

        let white = asset_manager.create_white_texture(
            &renderer.device,
            &renderer.queue,
            &renderer.scene.texture_bind_group_layout,
        );

        if with_cube {
            let cube = world.spawn();
            world.add_component(cube, Transform::new(Vec3::ZERO));
            world.add_component(cube, GlobalTransform::default());
            world.add_component(cube, AssetManager::create_cube(&renderer.device));
            world.add_component(
                cube,
                Material::new(white.clone()).with_pbr(Vec4::new(0.9, 0.15, 0.15, 1.0), 0.0, 1.0),
            );
            world.add_component(cube, MeshRenderer::new());
        }

        if let Some(Panel { kind, checkered, anchor }) = panel {
            let tex = if checkered {
                asset_manager.create_checkerboard_texture(
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                )
            } else {
                white.clone()
            };
            let mat = match kind {
                PanelKind::Backdrop => Material::new(tex).with_backdrop(Vec4::ONE),
                PanelKind::Unlit => Material::new(tex).with_unlit(Vec4::ONE),
                // Exactly what `Commands::spawn_skybox` builds.
                PanelKind::Skybox => Material::new(tex).with_unlit(Vec4::ONE).with_skybox(),
            };
            // Two units along the camera's forward axis. For a backdrop that offset IS the
            // transform (the shader adds the camera position); for the world-space kinds it is
            // measured from `anchor`.
            let offset = Vec3::new(2.0, 0.0, 0.0);
            let pos = match kind {
                PanelKind::Backdrop => offset,
                PanelKind::Unlit | PanelKind::Skybox => anchor + offset,
            };
            let panel_entity = world.spawn();
            world.add_component(panel_entity, Transform::new(pos));
            world.add_component(panel_entity, GlobalTransform::default());
            world.add_component(
                panel_entity,
                crate::renderer::components::Mesh::from_vertices(
                    &renderer.device,
                    &panel_vertices([0.0, 1.0, 0.0, 1.0]),
                    "backdrop_panel",
                ),
            );
            world.add_component(panel_entity, mat);
            world.add_component(panel_entity, MeshRenderer::new());
        }

        world.spawn_bundle(CameraBundle {
            position: camera_pos,
            yaw: 0.0,
            pitch: 0.0,
            primary: true,
            ..Default::default()
        });
        world.spawn_bundle(DirectionalLightBundle::default());

        render_world(&mut renderer, &mut world).await
    }

    /// The pixel at `(x, y)` of a 128×128 RGBA8 frame.
    fn px(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * 128 + x) * 4) as usize;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    }

    /// Green is the dominant channel by a clear margin — i.e. this pixel is the panel's own
    /// colour and not a tone-mapped sky gradient or a red cube.
    fn is_green(p: [u8; 4]) -> bool {
        p[1] as i32 > p[0] as i32 + 20 && p[1] as i32 > p[2] as i32 + 20
    }

    /// Both halves of the report, in one frame each: the backdrop must show the MESH's pixels
    /// (which `Skybox` discards) AND stay behind the world (which `Unlit` does not).
    ///
    /// The scene is a red PBR cube 6 units from the camera with a green screen-filling panel
    /// at 2 units — squarely between the camera and the cube. So the centre pixel answers
    /// "did the panel occlude the world?" and a corner pixel answers "did the panel reach the
    /// screen, with its own colour?".
    #[test]
    fn a_backdrop_shows_the_meshs_own_pixels_and_stays_behind_the_world() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!(
                "skipping a_backdrop_shows_the_meshs_own_pixels_and_stays_behind_the_world: \
                 no GPU adapter available"
            );
            return;
        }
        // WARP and friends can create these pipelines in seconds and cannot finish
        // rendering through them: `windows-latest` spent five and a half hours on this
        // file once the D3D12 shader fix let it get this far. Pipeline compilation is
        // the coverage that matters on such a runner and it is kept by
        // `every_pipeline_compiles_on_this_backend`, which builds the whole renderer.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_backdrop_shows_the_meshs_own_pixels_and_stays_behind_the_world: software adapter — see the note above");
            return;
        }
        pollster::block_on(async {
            let cam = Vec3::new(-6.0, 0.0, 0.0);
            // Anchored at the camera, so all three kinds put the panel on the same pixels and
            // the frames differ only in HOW it is drawn.
            let at = |kind| Panel { kind, checkered: false, anchor: cam };
            let no_panel = render_panel_scene(cam, true, None).await;
            let backdrop = render_panel_scene(cam, true, Some(at(PanelKind::Backdrop))).await;
            let unlit = render_panel_scene(cam, true, Some(at(PanelKind::Unlit))).await;
            let skybox = render_panel_scene(cam, true, Some(at(PanelKind::Skybox))).await;

            // Premise: without a panel the corner is background and the centre is the cube.
            assert!(
                !is_green(px(&no_panel, 8, 8)),
                "premise broken: the empty background is already green"
            );
            let bare_centre = px(&no_panel, 64, 64);
            assert!(
                !is_green(bare_centre),
                "premise broken: the red cube renders green ({bare_centre:?})"
            );

            // (a) The backdrop reaches the screen carrying the mesh's OWN vertex colour.
            let corner = px(&backdrop, 8, 8);
            assert!(
                is_green(corner),
                "the backdrop did not reach the screen with the mesh's own colour ({corner:?})"
            );

            // (b) …and it did NOT occlude the world 4 units behind it.
            let centre = px(&backdrop, 64, 64);
            assert!(
                !is_green(centre),
                "the backdrop painted over the cube ({centre:?}) — it is writing depth or \
                 winning the depth test"
            );

            // The `Unlit` arm is the reported symptom: same geometry, same place, and the
            // panel stands in front of the world.
            let unlit_centre = px(&unlit, 64, 64);
            assert!(
                is_green(unlit_centre),
                "premise broken: an `Unlit` panel 2 units from the camera is SUPPOSED to \
                 occlude a cube at 6 ({unlit_centre:?}); if it no longer does, this test is no \
                 longer distinguishing the two materials"
            );

            // The `Skybox` arm is the other reported half: correct depth, but the mesh's own
            // pixels are thrown away for an invented gradient.
            let sky_corner = px(&skybox, 8, 8);
            assert!(
                !is_green(sky_corner),
                "premise broken: `Skybox` is supposed to DISCARD the mesh's vertex colour and \
                 generate a gradient, but the corner came out green ({sky_corner:?})"
            );
            assert_ne!(
                sky_corner, corner,
                "the backdrop and the skybox produced the same pixel — one of them is not \
                 running its own shader"
            );
        });
    }

    /// The texture half, which the vertex-colour test above cannot see: swap the panel's
    /// texture and nothing else. `backdrop.wgsl` samples it, so the frames must differ; with
    /// `sky.wgsl` (zero `textureSample` calls) they would be byte-identical.
    #[test]
    fn a_backdrops_texture_reaches_the_screen() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping a_backdrops_texture_reaches_the_screen: no GPU adapter available");
            return;
        }
        // Software adapters build these pipelines in seconds and cannot finish rendering
        // through them — see the longer note on the first of these tests.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_backdrops_texture_reaches_the_screen: software adapter");
            return;
        }
        pollster::block_on(async {
            let cam = Vec3::new(-6.0, 0.0, 0.0);
            let with_tex = |checkered| Panel { kind: PanelKind::Backdrop, checkered, anchor: cam };
            let plain = render_panel_scene(cam, false, Some(with_tex(false))).await;
            let checkered = render_panel_scene(cam, false, Some(with_tex(true))).await;

            let differing = plain.iter().zip(checkered.iter()).filter(|(a, b)| a != b).count();
            assert!(
                differing > plain.len() / 10,
                "only {differing} of {} bytes changed when the backdrop's TEXTURE was swapped \
                 for a checkerboard — the shader is not sampling it",
                plain.len()
            );

            // The same swap under `Skybox`, which is the measurement the report made by
            // grepping (`grep -c textureSample sky.wgsl` → 0), taken here in pixels: the
            // texture makes no difference at all, because nothing ever reads it.
            let sky = |checkered| Panel { kind: PanelKind::Skybox, checkered, anchor: cam };
            let sky_plain = render_panel_scene(cam, false, Some(sky(false))).await;
            let sky_checkered = render_panel_scene(cam, false, Some(sky(true))).await;
            assert_eq!(
                sky_plain, sky_checkered,
                "premise broken: `Skybox` is supposed to ignore the mesh's texture entirely, \
                 so swapping it must change nothing — if it now does, this test is no longer \
                 measuring what distinguishes the two materials"
            );
        });
    }

    /// The largest single-channel difference between two frames.
    fn max_channel_delta(a: &[u8], b: &[u8]) -> u8 {
        a.iter().zip(b.iter()).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
    }

    /// Property (2): locked to the camera. Two cameras with the same orientation, 900 units
    /// apart, over a scene whose only content is the backdrop — so any real difference in the
    /// frame is the backdrop having moved.
    ///
    /// The bar is two 8-bit levels rather than byte equality, and the reason is arithmetic,
    /// not slack. The lock adds the camera position in world space and the view matrix
    /// subtracts it again, both in f32: at 900 units that round trip loses a few ULPs of
    /// mantissa. (The alternative — uploading a translation-free view-projection alongside
    /// `view_proj` — buys exactness for another 64 bytes in `SceneUniforms` and a field every
    /// construction site must fill; at a relative error of ~1e-7 on scenery painted at
    /// infinity, it is not worth it.)
    ///
    /// Two levels is a strong bound here, not a loose one: the panel wears a checkerboard
    /// whose own light/dark step is ~150 levels, so geometry that had genuinely SHIFTED — even
    /// by a fraction of a pixel — would move checker edges by tens of levels, and an unlocked
    /// panel leaves the frame altogether. The premise arm below measures that.
    #[test]
    fn a_backdrop_is_locked_to_the_camera() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping a_backdrop_is_locked_to_the_camera: no GPU adapter available");
            return;
        }
        // Software adapters build these pipelines in seconds and cannot finish rendering
        // through them — see the longer note on the first of these tests.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping a_backdrop_is_locked_to_the_camera: software adapter");
            return;
        }
        pollster::block_on(async {
            let here = Vec3::new(-6.0, 0.0, 0.0);
            let far_away = Vec3::new(-6.0, 0.0, 900.0);
            // `anchor` is unread for a backdrop — the transform is camera-relative, which is
            // precisely what makes the two frames comparable.
            let bd = Panel { kind: PanelKind::Backdrop, checkered: true, anchor: here };

            let a = render_panel_scene(here, false, Some(bd)).await;
            let b = render_panel_scene(far_away, false, Some(bd)).await;
            let delta = max_channel_delta(&a, &b);
            assert!(
                delta <= 2,
                "the frame changed by up to {delta} levels when the camera moved 900 units — \
                 the backdrop is not locked to it (f32 cancellation alone cannot exceed 2)"
            );

            // Premise: the SAME panel as ordinary world geometry does not survive the move. It
            // is nailed to `here` in both frames, so once the camera has driven 900 units away
            // it is nowhere near the view — which is exactly the failure a camera lock exists
            // to prevent, and the reason the game's 500-unit skybox cube runs out.
            let unlit = Panel { kind: PanelKind::Unlit, checkered: true, anchor: here };
            let ua = render_panel_scene(here, false, Some(unlit)).await;
            let ub = render_panel_scene(far_away, false, Some(unlit)).await;
            let unlocked_delta = max_channel_delta(&ua, &ub);
            assert!(
                unlocked_delta > 20,
                "premise broken: an unlocked panel is supposed to be left behind when the \
                 camera drives away from it, but the frame only moved by {unlocked_delta} \
                 levels — so the {delta}-level bar above is not distinguishing anything"
            );
        });
    }

    /// The mean of every RGB byte in a frame (alpha excluded).
    async fn render_mean_brightness(exposure: f32) -> f32 {
        let data = render_frame(exposure, false).await;
        let sum: u64 = data.chunks_exact(4).map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum();
        sum as f32 / (data.len() / 4 * 3) as f32
    }

    /// Exposure is a SINGLE post-process knob applied over the whole composited HDR (the
    /// deferred pass no longer bakes it in). This guards that rework: a higher camera exposure
    /// must brighten the frame. If exposure were detached (or the deferred→post move dropped
    /// the wiring), the two renders would match and this fails. (Tone-mapping is non-linear so
    /// we assert monotonic increase, not an exact 2x.)
    #[test]
    fn camera_exposure_brightens_the_frame() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping camera_exposure_brightens_the_frame: no GPU adapter available");
            return;
        }
        // Software adapters build these pipelines in seconds and cannot finish rendering
        // through them — see the longer note on the first of these tests.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping camera_exposure_brightens_the_frame: software adapter");
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

    /// The six point-shadow face passes are skipped when nothing samples them, and skipping
    /// them changes no pixel.
    ///
    /// `Renderer::point_shadows_enabled` defaults to false, and `deferred_lighting.wgsl` already
    /// gates its cubemap lookup on the uniform written from that same bool. Until this was fixed
    /// the passes ran anyway — six depth passes a frame, two draws per lit item each, twelve of
    /// the twenty-three draws a lit batch costs, into a cubemap nothing read.
    ///
    /// Rendering the same scene both ways and demanding byte-identical output is what proves the
    /// skipped work was unobserved. It is also what catches the dangerous version of this change:
    /// gate something the shader *does* sample — the cascades, say — and the frames diverge here
    /// rather than in someone's screenshot.
    #[test]
    fn skipping_the_point_shadow_passes_changes_no_pixel() {
        let _gpu = crate::test_gpu::gpu_lock();
        if !pollster::block_on(Renderer::headless_adapter_available()) {
            eprintln!("skipping skipping_the_point_shadow_passes_changes_no_pixel: no GPU adapter");
            return;
        }
        // Software adapters build these pipelines in seconds and cannot finish rendering
        // through them — see the longer note on the first of these tests.
        if pollster::block_on(Renderer::headless_adapter_is_software()) {
            eprintln!("skipping skipping_the_point_shadow_passes_changes_no_pixel: software adapter");
            return;
        }
        pollster::block_on(async {
            let gated = render_frame(1.0, false).await;
            let recorded = render_frame(1.0, true).await;
            assert_eq!(gated.len(), recorded.len(), "same target, same byte count");
            let differing = gated.iter().zip(&recorded).filter(|(a, b)| a != b).count();
            assert_eq!(
                differing, 0,
                "{differing} bytes differ between a frame with the point-shadow passes recorded                  and one with them skipped — this scene has no point light, so they cannot be                  observable; a gate was put on a pass something samples"
            );
            // And the frame is a real render, not two identical blank targets.
            assert!(
                gated.chunks_exact(4).any(|p| p[..3] != gated[..3]),
                "the frame is uniform — nothing was drawn, so the comparison proves nothing"
            );
        });
    }
}
