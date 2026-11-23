//! One view's draw (`draw_camera_view`): the camera's matrices, the post chain's uniforms, the
//! scene's lights and shadow maps, the batches, and the passes in the order they record.

use super::*;

/// One camera's picture: from its matrices to the post chain, into `region` of `view` (the whole
/// of it when `None`), with the HUD when `overlay`. `camera` is an entity id; `None` draws from
/// the origin, as a frame with no camera always has. With `simulations` it draws the GPU particles
/// and the fluid the frame's step simulated ([`step_gpu_simulations`]). With `capture`, that fixed
/// view replaces the camera and the frame ends in the HDR target.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_camera_view(
    world: &World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
    camera: Option<Entity>,
    simulations: bool,
    region: Option<[f32; 4]>,
    overlay: bool,
    capture: Option<FixedView>,
) {
    // A camera drawn with `CameraDetail::Simple`: the effects it goes without are set aside for its
    // view and put back after — the same way a probe's capture sets the temporal ones aside — and
    // its view plans no shadow pass (`collect_scene_setup` then also tells the lighting there are no
    // shadows to read, rather than leave it the last view's maps).
    let simple = camera
        .and_then(|c| world.query::<&Camera>().get(c).map(|cam| cam.detail == crate::renderer::components::CameraDetail::Simple))
        .unwrap_or(false);
    if !simple {
        return draw_camera_view_with(world, encoder, view, renderer, camera, simulations, region, overlay, capture, false);
    }
    let ssao = renderer.ssao.take();
    let volumetric = renderer.volumetric.take();
    let bloom = std::mem::replace(&mut renderer.bloom_intensity, 0.0);
    draw_camera_view_with(world, encoder, view, renderer, camera, simulations, region, overlay, capture, true);
    renderer.ssao = ssao;
    renderer.volumetric = volumetric;
    renderer.bloom_intensity = bloom;
}

#[allow(clippy::too_many_arguments)]
fn draw_camera_view_with(
    world: &World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
    camera: Option<Entity>,
    simulations: bool,
    region: Option<[f32; 4]>,
    overlay: bool,
    capture: Option<FixedView>,
    simple: bool,
) {
    let mut laps = Laps::start();
    // Without a G-buffer, nothing that reads one (`Renderer::keep_gbuffer_readers`).
    renderer.keep_gbuffer_readers();
    // The view's first GPU mark: what it times is the gap since the last view, not a part of this one.
    renderer.gpu_mark(encoder, "gpu.between");
    renderer.skip_mirrors = capture.is_some() || simple;
    renderer.skip_sky = capture.as_ref().is_some_and(|fixed| fixed.bounce);
    // Post-process params are written AFTER the active camera is resolved (below), so the
    // single exposure knob can be the camera's exposure — see the update_post_process call
    // after camera selection. Exposure is applied ONCE here, over the whole composited HDR
    // (deferred geometry + sky + unlit), instead of being baked per-geometry in the
    // deferred pass and multiplied again by a separate global knob.

    // A capture takes its own shape, a split-screen view or a texture camera its rectangle's, the
    // whole target otherwise. A mirror's capture used to be fitted (its shadow cascades) to the
    // whole target's shape while it projected with its view's — a wide view lost the shadows at the
    // sides of its reflection.
    let aspect = view_aspect(capture.as_ref(), region, (renderer.size.width, renderer.size.height));
    let ViewCamera {
        proj,
        view_mat,
        position: cam_pos,
        forward: cam_forward,
        exposure: cam_exposure,
        near: cam_near,
        far: cam_far,
        fov: cam_fov,
    } = view_camera(world, camera, capture, aspect);

    // The active camera's authored look, if it carries one. `None` means "graded by the
    // renderer's own fields", which is what every camera got before `PostProcess` existed and is
    // what a scene that has never been graded still gets. Read through `shared` so the editor's
    // viewport previews the grade by the same rule this frame applies it by.
    let grade = camera.and_then(|c| shared::camera_grade(world, c));

    // ── Su-altı atmosferi: kamera bir fluid zone içindeyse derinlik-bazlı sis uygula (W3+W4).
    // W1 `water_at` sorgusu tekrar kullanılır (aynı su hacimleri hem buoyancy hem yüzme hem bu
    // sisi sürer). Sis rengi/yoğunluğu deniz için makul sabitler — demolarda tunable yapılabilir.
    // Sis rengi/yoğunluğu artık kameranın içinde bulunduğu FluidZone'dan gelir (her su hacmi
    // kendi su-altı görünümünü tanımlar) — eskiden burada sabitti.
    // Underwater fog comes from the FluidZone the camera sits in, which lives in the physics
    // world; without the `physics` feature there are no fluid volumes, so the scene simply
    // never renders as submerged.
    // The zones are the preparation's copy (`WaterVolumes`), not the physics world's: the draw
    // reads nothing a render world cannot hold.
    #[cfg(feature = "physics")]
    let underwater = world
        .get_resource::<WaterVolumes>()
        .and_then(|w| w.water_at(cam_pos))
        .map(|s| crate::renderer::UnderwaterFog { color: s.fog_color, density: s.fog_density });
    #[cfg(not(feature = "physics"))]
    let underwater: Option<crate::renderer::UnderwaterFog> = None;


    // Save unjittered projection before applying TAA offset (needed for reprojection next frame).
    let unjittered_proj = proj;

    let (proj, frame_jitter) = taa_jitter(renderer, proj);

    let view_proj = proj * view_mat; // jittered — used for SceneUniforms
    let unjittered_view_proj = unjittered_proj * view_mat; // clean    — stored in TaaState for next frame

    // The active camera, in the one form both uniform blocks are built from. Assembled after the
    // TAA jitter so `view_proj` is the matrix this frame actually rasterises with.
    // The rendered camera's background, if it asked for one (`Camera::clear_color`); black for an
    // irradiance volume's capture, which holds the light bounced off the scene and not the sky's.
    let clear_color = if renderer.skip_sky {
        Some([0.0; 3])
    } else {
        camera.and_then(|c| world.query::<&Camera>().get(c).and_then(|cam| cam.clear_color)).map(|c| c.to_array())
    };
    let camera = crate::renderer::CameraFrame {
        view_proj,
        position: cam_pos,
        forward: cam_forward,
        near: cam_near,
        far: cam_far,
        exposure: cam_exposure,
    };

    let post = post_uniforms(renderer, grade, cam_exposure, cam_fov);
    let (reproject, velocity_read) = motion_history(renderer, unjittered_view_proj, &post);
    let baked = baked_route(world, renderer);
    let post = crate::renderer::PostProcessUniforms {
        reproject: reproject.to_cols_array_2d(),
        ..post
    };
    renderer.update_post_process(
        &renderer.queue,
        post.with_camera(&camera).with_underwater(underwater),
    );

    // Elapsed time drives fluid caustics/wave animation in fluid_composite.wgsl
    // (it reads cascade_params.z); this slot was hardcoded to 0.0 → frozen water.
    let elapsed_time = world
        .get_resource::<gizmo_core::time::Time>()
        .map(|t| t.elapsed() as f32)
        .unwrap_or(0.0);

    let (setup, rain_map) = scene_setup(world, renderer, camera, aspect, cam_fov, clear_color, elapsed_time, simple);
    upload_scene(renderer, &setup, rain_map.is_some(), cam_pos, elapsed_time);
    upload_temporal_params(renderer, frame_jitter, cam_pos, unjittered_view_proj);

    // CPU batched instancing (replaces the GPU cull): walk the world, frustum-cull, group into
    // instanced batches and upload the instance buffer. Lives in `batching.rs`.
    // An object off screen still casts into any shadow map the frame draws: the sun's cascades, and
    // every local light's layers.
    laps.draw_lap(renderer, "view.setup");
    let (draw_items, uploaded_instances) =
        {
            // What casts into the rain map is gathered as a shadow map's casters are: off screen too.
            let mut casters = setup.shadow_view_projs();
            casters.extend(rain_map.map(|(m, _)| m));
            batching::collect_draw_items(world, renderer, unjittered_view_proj, &casters, cam_pos, velocity_read)
        };
    record_render_stats(renderer, &draw_items, elapsed_time);
    laps.draw_lap(renderer, "view.batch");

    // The GPU-physics cull reads the view's camera, so it is per view; its simulation is not.
    if let Some(physics) = &renderer.gpu_physics {
        physics.cull_pass(encoder, renderer.scene.view_bind_group());
    }
    // GPU cull pass removed since we use CPU instancing

    resize_view_targets(renderer);

    // The sun's cascades on every path; the local lights' layers where they were planned (not on the
    // forward-only path).
    record_shadows(encoder, renderer, &draw_items, uploaded_instances, &setup, rain_map, cam_pos, elapsed_time);
    laps.draw_lap(renderer, "view.shadows");
    renderer.gpu_mark(encoder, "gpu.shadows");
    passes::record_deferred_geometry(
        encoder,
        renderer,
        world,
        &draw_items,
        uploaded_instances,
        cam_pos,
        baked,
    );
    // The decals and the lighting pass; the depth prepass and the G-buffer are marked inside
    // (`gpu.prepass`, `gpu.gbuffer`).
    renderer.gpu_mark(encoder, "gpu.lighting");
    passes::record_ssao(encoder, renderer);
    laps.draw_lap(renderer, "view.geometry");
    renderer.gpu_mark(encoder, "gpu.ssao");
    // CPU-computed inverse of the (unjittered) view-projection for the volumetric smoke raymarch
    // (the WGSL inverse_mat4 returns a wrong inverse for the perspective matrix).
    let inv_view_proj = unjittered_view_proj.inverse().to_cols_array_2d();
    // The forward-only path's lit shader reads the environment from a cube: an environment map's, or the
    // sky's, drawn again only when this frame's sky, presets or sun changed it.
    #[cfg(gizmo_forward_only)]
    {
        if renderer.environment_map_dirty {
            renderer.environment_map_dirty = false;
            renderer.scene.set_environment_map(&renderer.device, &renderer.queue, encoder, renderer.environment_map.as_ref());
        }
        renderer.scene.update_environment(&renderer.device, &renderer.queue, encoder, &crate::renderer::SceneUniforms::new(&setup.frame));
    }
    passes::record_forward_and_fluid(
        encoder, renderer, world, &draw_items, uploaded_instances, simulations,
        inv_view_proj, clear_color, velocity_read, baked, (cam_pos, elapsed_time),
    );
    // The opaque forward draws (`gpu.forward`) and the screen-space reflections and GI over them
    // (`gpu.surface_effects`, the seen mask with them) are marked inside; this is the rest — glass, what
    // blends, the forward surfaces' motion, the particles and the fluid.
    renderer.gpu_mark(encoder, "gpu.blended");
    passes::record_screen_space_effects(encoder, renderer);
    // Advance SSGI temporal ping-pong / frame counter after its passes have run.
    if let Some(ref mut ssgi) = renderer.ssgi {
        ssgi.advance_frame();
    }
    renderer.gpu_mark(encoder, "gpu.volumetric");
    passes::record_taa_and_overlays(encoder, renderer, world, capture.is_none());
    laps.draw_lap(renderer, "view.forward");
    renderer.gpu_mark(encoder, "gpu.taa");
    // After the overlays and before post-processing: sprites and text are overlays too, and the
    // ones whose smear a temporal resolve makes most obvious. Sprites first, so a HUD label lands
    // on its HUD image; with the UNJITTERED matrix, so a sprite drawn past TAA does not shimmer.
    record_sprites(encoder, renderer, world, unjittered_view_proj);
    record_text(encoder, renderer, world);
    laps.draw_lap(renderer, "view.ui");
    renderer.gpu_mark(encoder, "gpu.ui");

    // A probe face stops at the HDR target: what it captures is radiance, not a graded picture.
    if capture.is_some() {
        return;
    }
    // Auto exposure adapts in real time: a paused or slowed game still lets the eye adjust.
    let exposure_dt = world
        .get_resource::<gizmo_core::time::Time>()
        .map(|t| t.raw_dt())
        .unwrap_or(1.0 / 60.0);
    renderer.update_auto_exposure(exposure_dt);
    renderer.run_post_processing_in(encoder, view, region, overlay);
    laps.draw_lap(renderer, "view.post");
    renderer.gpu_mark(encoder, "gpu.post");
}

/// Where a view's camera stands and how it projects, before the TAA jitter: the drawn camera's,
/// or `capture`'s fixed view in its place.
struct ViewCamera {
    proj: Mat4,
    view_mat: Mat4,
    position: Vec3,
    forward: Vec3,
    exposure: f32,
    near: f32,
    far: f32,
    fov: f32,
}

/// The view's camera (see `ViewCamera`); from the origin, looking down −Z, with no camera.
fn view_camera(world: &World, camera: Option<Entity>, capture: Option<FixedView>, aspect: f32) -> ViewCamera {
    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut cam_pos = Vec3::ZERO;
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);

    // TODO: Bütün nesnelerin (özellikle kamera ve çizilecek objelerin) global matrix'leri
    // bu pass çağrılmadan hemen önce bir `update_transforms(world)` sistemiyle güncellenmiş olmalıdır.

    let mut cam_exposure = 1.0;
    // Shadow cascades must follow the ACTIVE camera's near/far/fov, not hardcoded values
    // (otherwise splits/cascade matrices are wrong for any non-default camera).
    let mut cam_near = 0.1f32;
    let mut cam_far = 2000.0f32;
    let mut cam_fov = std::f32::consts::FRAC_PI_4;

    // KAMERALARI BUL VE MATRIX YARAT
    //
    // `camera` is the one the caller resolved through `components::active_camera` (the camera
    // flagged `primary`, else the first) or a split-screen view's own. A second copy of the rule is
    // a second answer to "which camera is the scene's", and this file has the history to show what
    // that costs. Where it stands and how it looks come from `drawn_camera`: its synced
    // `GlobalTransform`, else its `Transform` (a camera with no `GlobalTransform` was silently
    // stuck at the origin), with this instant's shake — read, never written into the camera.
    if let Some((cam, pos, turn)) = camera.and_then(|id| drawn_camera(world, id)) {
        proj = cam.get_projection(aspect);
        view_mat = Camera::view(pos, turn);
        cam_pos = pos;
        cam_forward = Camera::front(turn);
        cam_exposure = cam.exposure;
        cam_near = cam.near;
        cam_far = cam.far;
        cam_fov = cam.fov;
    }
    if let Some(fixed) = capture {
        cam_near = fixed.near;
        cam_far = fixed.far;
        (proj, cam_fov) = fixed.projection.unwrap_or_else(|| {
            let right_angle = std::f32::consts::FRAC_PI_2;
            (Mat4::perspective_rh(right_angle, 1.0, cam_near, cam_far), right_angle)
        });
        view_mat = Mat4::look_to_rh(fixed.position, fixed.forward, fixed.up);
        cam_pos = fixed.position;
        cam_forward = fixed.forward;
        cam_exposure = 1.0;
    }
    ViewCamera {
        proj,
        view_mat,
        position: cam_pos,
        forward: cam_forward,
        exposure: cam_exposure,
        near: cam_near,
        far: cam_far,
        fov: cam_fov,
    }
}

/// `proj` with this frame's TAA sub-pixel offset, and the offset in NDC (zero with TAA off).
fn taa_jitter(renderer: &Renderer, proj: Mat4) -> (Mat4, [f32; 2]) {
    let mut proj = proj;
    // ── TAA Halton jitter: subpixel offset applied to the projection ──
    // Kept: TAA and SSGI's temporal pass both reproject from the G-buffer the jittered raster wrote,
    // and both put this back.
    let mut frame_jitter = [0.0f32; 2];
    if let Some(ref taa) = renderer.taa {
        if taa.enabled {
            let jp = crate::renderer::taa::TaaState::get_jitter(taa.frame_index);
            // Convert pixel jitter [−0.5, 0.5] to NDC offset (2 / viewport_size per axis)
            let jx = jp[0] * 2.0 / renderer.size.width as f32;
            let jy = jp[1] * 2.0 / renderer.size.height as f32;
            proj = jitter_projection(proj, jx, jy);
            frame_jitter = [jx, jy];
        }
    }
    (proj, frame_jitter)
}

/// The post chain's uniforms for the view's camera: its grade where it carries one, the
/// renderer's fields otherwise, and the renderer's curve either way. The forward-only composite's
/// pipeline constants follow them (`sync_composite`).
fn post_uniforms(
    renderer: &mut Renderer,
    grade: Option<crate::renderer::components::PostProcess>,
    cam_exposure: f32,
    cam_fov: f32,
) -> crate::renderer::PostProcessUniforms {
    // Post-process params, now that the active camera (hence its exposure and depth range) is
    // known. `exposure` is the SINGLE exposure knob: the camera's exposure, applied once in the
    // post composite over the entire HDR. (Previously the deferred pass baked cam.exposure into
    // geometry AND post multiplied by a separate 1.15, which compounded and skipped sky/unlit;
    // folding both into one post-stage exposure fixes that.) Everything not named here is the
    // renderer's neutral default.
    // **The camera's own grade wins where it has one.** `PostProcess` is the authored look and
    // the only form of it a file can carry; the renderer's fields are the fallback for a camera
    // that carries none, which is every camera in every scene written before the component
    // existed. Exposure is not in the branch because it never moved: it has always come from
    // `Camera::exposure`, which is exactly why the component does not duplicate it.
    let post = match grade {
        Some(g) => crate::renderer::PostProcessUniforms {
            bloom_intensity: g.bloom_intensity,
            bloom_threshold: g.bloom_threshold,
            exposure: cam_exposure,
            chromatic_aberration: g.chromatic_aberration,
            vignette_intensity: g.vignette,
            film_grain_intensity: g.film_grain,
            dof_focus_dist: g.dof_focus_dist,
            dof_focus_range: g.dof_focus_range,
            // No `dof_enabled` gate here: a blur size of 0 IS the off state, which is why the
            // component's default carries 0 rather than the renderer's 4.0.
            dof_blur_size: g.dof_blur_size,
            dof_lens: g.dof_lens_factor(cam_fov, renderer.size.height),
            dof_max_depth: g.dof_max_depth,
            dof_shape: g.dof_blur.uniform(),
            motion_blur: g.motion_blur,
            ..Default::default()
        },
        None => crate::renderer::PostProcessUniforms {
            bloom_intensity: renderer.bloom_intensity,
            bloom_threshold: renderer.bloom_threshold,
            exposure: cam_exposure,
            chromatic_aberration: renderer.chromatic_aberration,
            // Was missing, and its absence was not visible: leaving it out let
            // `PostProcessUniforms::default()`'s 0.25 through, so an ungraded camera had a
            // vignette it never asked for and could not switch off.
            vignette_intensity: renderer.vignette_intensity,
            film_grain_intensity: renderer.film_grain_intensity,
            dof_focus_dist: renderer.dof_focus_dist,
            dof_focus_range: renderer.dof_focus_range,
            dof_blur_size: if renderer.dof_enabled { renderer.dof_blur_size } else { 0.0 },
            dof_shape: renderer.dof_blur.uniform(),
            ..Default::default()
        },
    };
    // The curve is the renderer's in both branches: `PostProcess` is the authored *grade*, and a
    // grade that carried its own tone map would fight the camera it is composited with.
    let post = crate::renderer::PostProcessUniforms {
        tonemap_curve: renderer.tonemap_curve.as_f32(),
        tonemap_white_point: renderer.tonemap_white_point,
        deband_dither: if renderer.deband_dither { 1.0 } else { 0.0 },
        bloom_softness: renderer.bloom_softness.clamp(0.0, 1.0),
        bloom_energy_conserving: if renderer.bloom_energy_conserving { 1.0 } else { 0.0 },
        color_lut: renderer.color_lut_params(),
        ..post
    };
    // The forward-only composite takes the curve, the dither and whether there is a vignette or grain as
    // pipeline constants.
    crate::renderer::post_process::sync_composite(renderer, &post);
    post
}

/// The view's motion history: the matrix that takes this frame's clip space to last frame's
/// (the camera motion blur's input), and whether anything reads the G-buffer's velocity. The
/// G-buffer's velocity is given the same pair, and the history moves on.
fn motion_history(
    renderer: &mut Renderer,
    unjittered_view_proj: Mat4,
    post: &crate::renderer::PostProcessUniforms,
) -> (Mat4, bool) {
    // **Camera motion blur's one input**: where this frame's clip space was last frame. Both
    // matrices unjittered — TAA's sub-pixel jitter is not motion. The history is per view
    // (`Renderer::with_view_history` swaps it), and kept whether or not the blur is on, so turning
    // it on mid-game has a last frame to blur against.
    let reproject = match renderer.previous_view_proj {
        Some(previous) => Mat4::from_cols_array_2d(&previous) * unjittered_view_proj.inverse(),
        None => Mat4::IDENTITY,
    };
    // Whether this frame reads the G-buffer's velocity at all: TAA reprojects by it, the camera's
    // motion blur smears along it. Nothing else does — screen-space reflections read the target's
    // `zw`, the specular tint the G-buffer pass writes, and never the motion (they were counted until
    // 2026-11-19, for a temporal pass they do not have) — and without them the forward-drawn surfaces
    // are not drawn a second time into it, and no instance's last-frame model is looked up, kept or
    // uploaded (`collect_draw_items`).
    let velocity_read = post.motion_blur > 0.0 || renderer.taa.as_ref().is_some_and(|t| t.enabled);
    // The G-buffer's velocity needs the same pair, before the history moves on — and whether the
    // instances' last models come with it.
    if let Some(def) = renderer.deferred.as_ref() {
        let now = unjittered_view_proj.to_cols_array_2d();
        def.motion.set_view_projs(&renderer.queue, now, renderer.previous_view_proj.unwrap_or(now), velocity_read);
    }
    renderer.previous_view_proj = Some(unjittered_view_proj.to_cols_array_2d());
    (reproject, velocity_read)
}

/// Where the view draws the opaque baked-lit surfaces.
fn baked_route(world: &World, renderer: &Renderer) -> BakedRoute {
    // Where the opaque baked-lit surfaces are drawn this view (`BakedRoute`), by what reads them: a
    // decal, projected onto the G-buffer, needs them there (`gbuffer.wgsl`'s `fs_baked`, finished by
    // the lighting pass); screen-space reflections need only their normal, roughness, position and
    // reflectance, written beside the forward draw (`baked_lit.wgsl`'s `fs_reflective`); nothing else
    // reads them, and the forward draw alone is the same picture for less. In `nfsu2-gizmo`'s wet city
    // (Radeon 780M, 2026-11-19) the G-buffer's route cost about twice the forward draw.
    if renderer.deferred.is_none() {
        BakedRoute::Forward
    } else if world.query::<&crate::renderer::components::Decal>().iter().next().is_some() {
        BakedRoute::GBuffer
    } else if renderer.ssr.as_ref().is_some_and(|s| s.enabled) {
        BakedRoute::Reflective
    } else {
        BakedRoute::Forward
    }
}

/// The view's lights, shadow cascades, local shadow layers and scene block
/// (`collect_scene_setup`), with the rain map's matrix and resolution when the scene is wet.
#[allow(clippy::too_many_arguments)]
fn scene_setup(
    world: &World,
    renderer: &mut Renderer,
    camera: crate::renderer::CameraFrame,
    aspect: f32,
    cam_fov: f32,
    clear_color: Option<[f32; 3]>,
    elapsed_time: f32,
    simple: bool,
) -> (SceneSetup, Option<(Mat4, u32)>) {
    let cam_pos = camera.position;
    // Lights, cascades and the whole scene block — via the shared setup helper, so the game and
    // studio renderers can only differ in what they pass it.
    // Cookies a spot names by path are read from disk the first frame they are needed.
    #[cfg(not(target_arch = "wasm32"))]
    renderer.sync_light_cookies(world);
    // The rain map (`gizmo_renderer::rain_map`): the scene's depth from straight above, around this
    // camera, so that what is under cover stays dry. Drawn every frame the scene is wet — what covers
    // moves, and what streams in arrives — on the desktop, by a view that draws shadow maps.
    let rain_map = (!cfg!(gizmo_forward_only) && !simple && renderer.wetness.is_some_and(|w| w.level > 0.0)).then(|| {
        let res = renderer.shadow_settings.resolution.max(1);
        (crate::renderer::rain_map::RainMap::view_proj(cam_pos, res), res)
    });
    let setup = collect_scene_setup(
        world,
        &SceneSetupInputs {
            camera,
            aspect,
            cam_fov,
            environment: crate::renderer::EnvironmentFrame {
                preset: renderer.environment_preset,
                preset_2: renderer.environment_preset_2,
                blend_t: renderer.environment_blend_t,
                shading_mode: renderer.shading_mode,
                clear_color,
                wireframe_color: world.get_resource::<WireframeConfig>().and_then(|w| w.color),
                fog: renderer.fog,
                sky: renderer.sky,
                wetness: renderer.wetness,
                rainfall: renderer.rainfall,
                rain_map,
            },
            // The forward-only path draws the sun's cascade and the local lights' layers unless a game
            // says not to (`ShadowSettings::forward_only`).
            sun_shadow_maps: !simple && (!cfg!(gizmo_forward_only) || renderer.shadow_settings.forward_only),
            local_shadow_maps: !simple && (!cfg!(gizmo_forward_only) || renderer.shadow_settings.forward_only),
            elapsed_time,
            light_cookies: renderer.light_cookies.table(),
            shadow_settings: renderer.shadow_settings,
            scene_extent: renderer.scene_extent.last(),
            decals: renderer.scene.forward_decal_spheres.clone(),
        },
    );
    (setup, rain_map)
}

/// The view's scene on the GPU: the shadow maps and layers sized, the forward-only pipelines
/// rebuilt when what they read changed, the local layers' and cascades' matrices, the scene
/// block, and the light clusters.
fn upload_scene(renderer: &mut Renderer, setup: &SceneSetup, has_rain_map: bool, cam_pos: Vec3, elapsed_time: f32) {
    let light_view_projs: [[[f32; 4]; 4]; 4] =
        setup.cascade_view_projs.map(|m| m.to_cols_array_2d());

    // The local lights' shadow layers: the array grown to hold them, each layer's matrix where its
    // pass reads it.
    let local_shadow_layers = setup.local_shadows.layers();
    renderer.scene.ensure_sun_shadow_maps(&renderer.device, setup.draws_cascades(), renderer.shadow_settings.resolution, has_rain_map);
    renderer.scene.ensure_local_shadow_layers(&renderer.device, local_shadow_layers.len());
    // The forward-only path's lit pipelines read the sun's shadow only while there is one to read, and
    // the local lights' layers and cookies only while a frame draws a layer or lights a spot through a
    // cookie: either appearing or going builds them again (`SceneState::set_forward_sun_shadows`,
    // `set_forward_local_light_maps`), so a scene without them pays nothing for them per fragment.
    // And the local lights' loop only while there are some, a sized source's or a rectangle's lighting
    // only while there is one of those (`SceneState::set_forward_lights`).
    if cfg!(gizmo_forward_only) {
        let lights = &setup.frame.lights[..(setup.frame.num_lights as usize).min(setup.frame.lights.len())];
        let cookie = lights.iter().any(|light| light.params[1] == 1.0 && light.params[3] > 0.0);
        let sun = renderer.scene.set_forward_sun_shadows(setup.draws_cascades());
        let local = renderer.scene.set_forward_local_light_maps(!local_shadow_layers.is_empty() || cookie);
        let kinds = renderer.scene.set_forward_lights(crate::renderer::pipeline::LightShape::of(lights));
        if sun || local || kinds {
            crate::renderer::pipeline::rebuild_pipelines(renderer);
        }
    }
    for (i, layer) in local_shadow_layers.iter().enumerate() {
        renderer.queue.write_buffer(
            renderer.scene.view_local_shadow_buffer(i),
            0,
            bytemuck::bytes_of(&crate::renderer::gpu_types::ShadowVsUniform::new(
                layer.view_proj.to_cols_array_2d(),
                cam_pos.to_array(),
                elapsed_time,
            )),
        );
    }


    let scene_uniform_data = crate::renderer::SceneUniforms::new(&setup.frame);
    // Through the selector, not straight at `global_uniform_buffer`: with `active_view` set, the
    // passes below bind that view's group, so the camera has to land in that view's buffer.
    renderer.queue.write_buffer(
        renderer.scene.view_uniform_buffer(),
        0,
        bytemuck::cast_slice(&[scene_uniform_data]),
    );
    // The lighting shaders read their per-fragment light list from these buffers, so this upload is
    // not an optimisation — without it every cluster is empty and nothing is lit.
    renderer.scene.upload_clusters(&renderer.queue, &setup.clusters);
    for (i, light_view_proj) in light_view_projs.iter().enumerate() {
        renderer.queue.write_buffer(
            renderer.scene.view_shadow_cascade_buffer(i),
            0,
            bytemuck::bytes_of(&crate::renderer::gpu_types::ShadowVsUniform::new(
                *light_view_proj,
                cam_pos.to_array(),
                elapsed_time,
            )),
        );
    }
}

/// TAA's and SSGI's per-view parameters: this frame's jitter, the blend, SSGI's history matrix.
fn upload_temporal_params(renderer: &mut Renderer, frame_jitter: [f32; 2], cam_pos: Vec3, unjittered_view_proj: Mat4) {
    // Upload TAA params: this frame's jitter, and whether there is history to keep. Where each
    // pixel's history is comes from the G-buffer's velocity target.
    if let Some(ref mut taa) = renderer.taa {
        if taa.enabled {
            let alpha = if taa.frame_index == 0 { 1.0f32 } else { 0.1f32 };
            taa.update_params(&renderer.queue, frame_jitter, alpha);
        }
    }

    // Upload SSGI temporal-accumulation params (mirrors TAA: previous-frame unjittered
    // view-proj for reprojection + blend alpha). alpha=1.0 on the first frame / after a
    // reset so there is no stale history to reproject. Denoises the 1-spp raymarch grain.
    if let Some(ref mut ssgi) = renderer.ssgi {
        let alpha = if ssgi.frame_index == 0 { 1.0f32 } else { 0.1f32 };
        ssgi.update_params(&renderer.queue, alpha, cam_pos.to_array(), frame_jitter);
        ssgi.store_prev_vp(unjittered_view_proj.to_cols_array_2d());
    }
}

/// The G-buffer and the screen-space effects' targets at the renderer's size.
fn resize_view_targets(renderer: &mut Renderer) {
    // Resize deferred G-buffers if window changed; resize SSAO + TAA to match
    if let Some(ref mut def) = renderer.deferred {
        def.resize(&renderer.device, &renderer.depth_texture_view, renderer.size.width, renderer.size.height);
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
                    &def.velocity_view,
                    w,
                    h,
                );
            }
        }
    }
}

/// The view's shadow maps: the sun's cascades and the local lights' layers, then the rain map.
#[allow(clippy::too_many_arguments)]
fn record_shadows(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    setup: &SceneSetup,
    rain_map: Option<(Mat4, u32)>,
    cam_pos: Vec3,
    elapsed_time: f32,
) {
    let (shadow_passes, shadow_instances, cleared) = passes::record_shadow_passes(
        encoder,
        renderer,
        draw_items,
        uploaded_instances,
        setup.draws_cascades().then_some(&setup.cascade_view_projs),
        setup.local_shadows.layers(),
    );
    // The sun's cascades and the local lights' layers; the rain map is `gpu.shadows`, after.
    renderer.gpu_mark(encoder, "gpu.shadow_maps");
    let (rain_passes, rain_instances) = match rain_map {
        Some((view_proj, _)) => {
            renderer.scene.rain_map.write(&renderer.queue, view_proj, cam_pos, elapsed_time);
            (1, passes::record_rain_map(encoder, renderer, draw_items, uploaded_instances, view_proj, cleared))
        }
        None => (0, 0),
    };
    if let Some(stats) = renderer.render_stats.as_mut() {
        stats.shadow_passes = shadow_passes + rain_passes;
        stats.shadow_instances = shadow_instances + rain_instances;
    }
}
