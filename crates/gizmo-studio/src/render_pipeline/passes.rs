//! Studio-editor render passes + editor-setting sync, extracted verbatim from
//! render_pipeline.rs. `record_studio_*` consume the `FlatBatchData` list produced by the
//! batching step; `sync_editor_settings` mirrors egui state into the renderer.

use super::*;
use super::batching::FlatBatchData;

/// Mirror egui state into the renderer, and return what the caller still has to place: the
/// viewport aspect, the debug shading mode, the collider toggle, and the post-process block.
///
/// The post block is **returned rather than uploaded** because two of its fields — `cam_near` and
/// `cam_far` — belong to the active camera, and the camera is not resolved until after this runs
/// (its projection needs the aspect this function computes). Uploading here is what left the
/// editor's DoF linearising depth against a hardcoded 0.1/2000 range.
/// The toolbar's fourth shading chip. A pipeline swap, not a shader uniform — see the comment at
/// the pipeline selection in [`record_studio_main_pass`].
pub(super) const SHADING_WIREFRAME: u32 = 3;

pub(super) fn sync_editor_settings(
    world: &gizmo::core::World,
    renderer: &mut gizmo::renderer::Renderer,
) -> (f32, u32, bool, gizmo::renderer::PostProcessUniforms, bool) {
    let mut aspect = if renderer.size.height > 0 {
        renderer.size.width as f32 / renderer.size.height as f32
    } else {
        1.0
    };

    let mut ed_shading_mode = 0;
    let mut ed_fxaa_enabled = true;
    let mut ed_ssao_enabled = true;
    let mut ed_ssao_strength = 0.8;
    let mut show_colliders = false;
    // Whether the Game panel is on screen this frame. In the default layout Scene and Game are
    // TABS OF THE SAME LEAF, so at most one of them is ever visible — and the game view was being
    // rendered as a full extra scene pass every frame regardless.
    let mut game_view_visible = false;
    
    // **The viewport is graded by the scene, not by the editor's taste.**
    //
    // This used to start from a set of values chosen for the editor (a mild aberration, no film
    // grain, a real DoF blur) and then be overwritten from `EditorState::post_process`, which no
    // file ever carried. So the picture here was never the picture the game would ship: different
    // starting values, and an authored look that reached this pass and nothing else.
    //
    // Now it is the active camera's `PostProcess` component when it has one — read through the
    // same `active_camera_grade` the engine's frame uses, so "which camera is graded" has
    // one answer — and the renderer's own neutral fields when it has none, which is exactly what
    // `default_render_pass` falls back to. Exposure comes from `Camera::exposure` either way,
    // filled in below from the resolved viewpoint.
    let grade = gizmo::systems::render::active_camera_grade(world);
    let mut post_params = match grade {
        Some(g) => gizmo::renderer::PostProcessUniforms {
            bloom_intensity: g.bloom_intensity,
            bloom_threshold: g.bloom_threshold,
            vignette_intensity: g.vignette,
            chromatic_aberration: g.chromatic_aberration,
            film_grain_intensity: g.film_grain,
            // The shutter carries over; the editor keeps no last-frame matrix, so `reproject` stays
            // identity and its viewport never smears — a fly-through would be unreadable otherwise.
            motion_blur: g.motion_blur,
            dof_focus_dist: g.dof_focus_dist,
            dof_focus_range: g.dof_focus_range,
            dof_blur_size: g.dof_blur_size,
            ..Default::default()
        },
        None => gizmo::renderer::PostProcessUniforms {
            bloom_intensity: renderer.bloom_intensity,
            bloom_threshold: renderer.bloom_threshold,
            chromatic_aberration: renderer.chromatic_aberration,
            film_grain_intensity: renderer.film_grain_intensity,
            dof_focus_dist: renderer.dof_focus_dist,
            dof_focus_range: renderer.dof_focus_range,
            dof_blur_size: if renderer.dof_enabled { renderer.dof_blur_size } else { 0.0 },
            ..Default::default()
        },
    };

    // The renderer's in both arms, as on the game path: a viewport that tone-mapped differently would
    // lie about every colour in it. It read the default curve until 2026-11-07.
    post_params.tonemap_curve = renderer.tonemap_curve.as_f32();
    post_params.tonemap_white_point = renderer.tonemap_white_point;
    post_params.deband_dither = if renderer.deband_dither { 1.0 } else { 0.0 };
    post_params.bloom_softness = renderer.bloom_softness.clamp(0.0, 1.0);
    post_params.bloom_energy_conserving = if renderer.bloom_energy_conserving { 1.0 } else { 0.0 };

    // **Exposure comes from the camera, in this viewport too.** It never moved into the grade
    // because it never had to: `Camera::exposure` round-trips in a scene and is the one
    // post-process value the shipped game has always read. What it did NOT do was reach this
    // pass — the editor drove exposure from its own slider, so the viewport and the exported
    // build disagreed about the single value both of them already agreed on the meaning of.
    // `viewpoint::resolve` takes the number from here, which is why it is set before that call.
    if let Some(cam_id) = gizmo::renderer::components::active_camera(world) {
        if let Some(cam) = world.query::<&gizmo::renderer::components::Camera>().get(cam_id) {
            post_params.exposure = cam.exposure;
            if let Some(g) = grade {
                post_params.dof_lens = g.dof_lens_factor(cam.fov, renderer.size.height);
                post_params.dof_max_depth = g.dof_max_depth;
            }
        }
    }

    if let Some(ed_state) = world.get_resource::<gizmo::editor::EditorState>() {
        ed_shading_mode = ed_state.shading_mode;
        ed_fxaa_enabled = ed_state.post_process.fxaa_enabled;
        ed_ssao_enabled = ed_state.post_process.ssao_enabled;
        ed_ssao_strength = ed_state.post_process.ssao_strength;
        
        show_colliders = ed_state.show_colliders;
        // Nothing about the LOOK is read from `EditorState` any more — it is the camera's, above.
        // What is still the editor's own and rightly here: the shading-mode dropdown, the collider
        // overlay, the viewport rect, and the two viewport toggles (FXAA, SSAO) that never
        // travelled with a scene.

        if let Some(rect) = ed_state.scene_view_rect {
            if rect.height() > 0.0 {
                aspect = rect.width() / rect.height();
            }
        }
        game_view_visible = ed_state.game_view_visible;
    }

    if let Some(ref mut fxaa) = renderer.fxaa {
        if fxaa.enabled != ed_fxaa_enabled {
            fxaa.enabled = ed_fxaa_enabled;
            fxaa.set_enabled(&renderer.queue, ed_fxaa_enabled);
        }
    }

    // The editor's SSAO strength used to be uploaded here, every frame, into a uniform no pass in
    // this pipeline reads: studio records no SSAO pass, and `post_process.wgsl` samples no AO
    // texture. `Ssao::new` binds `deferred.normal_roughness_view`, which the forward path does not
    // produce — so the write was not merely redundant, it could never become live by accident.
    // The widgets it served are disabled with that reason in `gizmo_editor::windows`.
    //
    // Restoring this line is not what turns SSAO on here; a depth-normal prepass is.
    let _ = (ed_ssao_enabled, ed_ssao_strength);

    (aspect, ed_shading_mode, show_colliders, post_params, game_view_visible)
}

// execute_render_pipeline'ten çıkarılan render geçişleri (Tier 3: mega-fn bölmesi).
// Yan-etki-only: encoder'a komut kaydeder, çıktı yok.
/// What a batch is drawn with in this viewport where the engine's own material would be drawn with
/// `engine`: a game's pipeline from the registry (`MaterialType::Custom`), a game's surface's lit
/// pipeline (`MaterialType::CustomSurface`: glass, blended or two-sided as the batch is), or `engine`.
/// `None` is an id with nothing behind it, which draws nothing — the game path's rule, so the two do
/// not disagree about a scene that names a material this renderer was never given.
fn batch_pipeline<'a>(
    renderer: &'a gizmo::renderer::Renderer,
    batch: &FlatBatchData,
    transparent: bool,
    engine: &'a wgpu::RenderPipeline,
) -> Option<&'a wgpu::RenderPipeline> {
    if let Some(id) = batch.custom {
        return renderer.custom_materials.get(id).map(|c| &c.pipeline);
    }
    if let Some(id) = batch.surface {
        return renderer.custom_materials.surface(id).map(|surface| {
            if batch.is_transmissive {
                surface.transmissive_pipeline(batch.is_double_sided)
            } else {
                surface.lit_pipeline(false, transparent, batch.is_double_sided, batch.blend_mode)
            }
        });
    }
    Some(engine)
}

/// This view's shadow maps, and how many depth passes they took: the sun's cascades when there are
/// any to draw (`SceneSetup::draws_cascades`), then each local light's layer (`local_layers`, the
/// frame's `SceneSetup::local_shadows`; the array must already hold them,
/// `SceneState::ensure_local_shadow_layers`). `camera_position` and `elapsed_time` are what a game's
/// surface is handed when its shadow is cut where the picture is.
pub(super) fn record_studio_shadow_passes(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &gizmo::renderer::Renderer,
    flat_batches: &[FlatBatchData],
    light_view_proj_cascades: Option<&[[[f32; 4]; 4]; 4]>,
    local_layers: &[gizmo::renderer::local_shadows::LocalShadowLayer],
    camera_position: gizmo::math::Vec3,
    elapsed_time: f32,
) -> (u32, u32) {
    let (mut passes, mut drawn) = (0, 0);
    for (cascade_i, &cascade_view_proj) in light_view_proj_cascades.into_iter().flatten().enumerate() {
        renderer.queue.write_buffer(
            renderer.scene.view_shadow_cascade_buffer(cascade_i),
            0,
            gizmo::bytemuck::bytes_of(&gizmo::renderer::ShadowVsUniform::new(
                cascade_view_proj,
                camera_position.to_array(),
                elapsed_time,
            )),
        );
        drawn += record_studio_caster_pass(
            encoder,
            renderer,
            flat_batches,
            &format!("Shadow Pass cascade {cascade_i}"),
            &renderer.scene.shadow_cascade_layer_views[cascade_i],
            renderer.scene.view_shadow_pass_bind_group(cascade_i),
            Mat4::from_cols_array_2d(&cascade_view_proj),
            false,
        );
        passes += 1;
    }
    let local_layers = &local_layers[..local_layers.len().min(renderer.scene.local_shadow_maps.layers())];
    for (layer_i, layer) in local_layers.iter().enumerate() {
        renderer.queue.write_buffer(
            renderer.scene.view_local_shadow_buffer(layer_i),
            0,
            gizmo::bytemuck::bytes_of(&gizmo::renderer::ShadowVsUniform::new(
                layer.view_proj.to_cols_array_2d(),
                camera_position.to_array(),
                elapsed_time,
            )),
        );
        drawn += record_studio_caster_pass(
            encoder,
            renderer,
            flat_batches,
            "Local Shadow Pass",
            &renderer.scene.local_shadow_maps.layer_views[layer_i],
            renderer.scene.view_local_shadow_pass_bind_group(layer_i),
            layer.view_proj,
            true,
        );
        passes += 1;
    }
    (passes, drawn)
}

/// One depth pass into `target`, through the shadow pass group `light`, drawing the casters whose
/// chunks meet `view_proj`'s frustum. Returns the instances it drew. `local`: a local light's layer,
/// where every caster is drawn by both faces, as on the game path (its penumbra is measured from
/// the depth of what blocks it, which a closed mesh's back faces would put at its far side).
#[allow(clippy::too_many_arguments)]
fn record_studio_caster_pass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &gizmo::renderer::Renderer,
    flat_batches: &[FlatBatchData],
    label: &str,
    target: &wgpu::TextureView,
    light: &wgpu::BindGroup,
    view_proj: Mat4,
    local: bool,
) -> u32 {
    let frustum = gizmo::renderer::Frustum::from_matrix(&view_proj);
    let mut drawn = 0;
    let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: target,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });

    // As on the game path: a closed mesh casts from its back faces and an open one from both
    // (`Mesh::closed`), a cut-out through the stage that cuts it, a game's surface with its
    // own; and every shadow pipeline binds the light, the material, the skeleton and the
    // instances.
    shadow_pass.set_bind_group(0, light, &[]);
    shadow_pass.set_bind_group(3, &renderer.scene.instance_bind_group, &[]);
    let mut bound: Option<*const wgpu::RenderPipeline> = None;
    for batch in flat_batches {
        // Non-casters must not write the shadow maps — matches the game path
        // (`passes.rs`: skip unlit/transparent) and the `classify_visibility`
        // caster predicate (excludes Unlit/Skybox/Grid/transparent). Their
        // CAMERA-VISIBLE instances still live in `[start_instance, end_instance)`
        // here, so without this filter the editor grid / a skybox / transparent
        // objects would cast shadows (grid → ground-coplanar self-shadow acne,
        // skybox → shadows the whole scene).
        if batch.is_transparent
            || batch.is_skybox
            || batch.is_grid
            || batch.is_unlit
            || batch.is_backdrop
            // Water draws forward with a vertex shader that displaces it; the shadow
            // pipeline's does not, so its shadow would be the flat plane's. The game
            // path skips it for the same reason (it rides `unlit` out of both shadow
            // passes there), and this viewport must agree or the editor shows a shadow
            // the build does not have.
            || batch.is_water
        {
            continue;
        }
        // Per-object opt-out. The material-derived rules above still hold — they exclude
        // things that cannot sensibly cast at all — and this is the object's own answer on
        // top of them.
        if !batch.casts_shadows {
            continue;
        }
        // A game's own pipeline decides for itself, as on the game path
        // (`CustomMaterial::casts_shadows`), and an id with nothing behind it draws
        // nothing, its shadow included. A game's surface casts as the engine's material.
        if let Some(id) = batch.custom {
            if !renderer.custom_materials.get(id).is_some_and(|c| c.casts_shadows) {
                continue;
            }
        }
        let back_faces = batch.closed && !local;
        let pipeline = match batch.surface {
            Some(id) => renderer.custom_materials.surface(id).and_then(|s| s.shadow_pipeline(back_faces)),
            None => Some(renderer.scene.shadow_pipeline_for(back_faces, batch.discards)),
        };
        // `None` is also an id with nothing behind it: it draws nothing, its shadow included.
        let Some(pipeline) = pipeline else { continue };
        // The runs of this batch — camera-visible instances and off-screen casters alike — that this
        // pass's frustum meets, clamped to what the instance buffer holds.
        let capacity = renderer.scene.instance_capacity as u32;
        let mut ranges = gizmo::renderer::shadow_culling::visible_ranges(&batch.shadow_chunks, &frustum, capacity).peekable();
        if ranges.peek().is_none() {
            continue;
        }
        if bound != Some(std::ptr::from_ref(pipeline)) {
            shadow_pass.set_pipeline(pipeline);
            bound = Some(std::ptr::from_ref(pipeline));
        }
        shadow_pass.set_bind_group(1, &*batch.bind_group, &[]);
        shadow_pass.set_bind_group(2, &*batch.skeleton_bg, &[]);
        for range in ranges {
            drawn += range.end - range.start;
            batch.record_draw(&mut shadow_pass, range);
        }
    }
    drawn
}

/// Records the scene into the HDR target.
///
/// `draw_chrome` is what separates the two pictures this pipeline produces. The editor viewport
/// wants the grid, the gizmo lines and the collider overlay; the game view is the same scene
/// without any of it, drawn from the other camera. The furniture that lives in the batch list
/// (grid, light icons) is filtered out by the caller via `EditorOnly`; the furniture drawn
/// procedurally here — grid pass, debug lines, colliders — is what this flag governs.
///
/// Play mode still suppresses the same things on its own: that rule predates the flag and stays,
/// because "playing" and "this is the game picture" are different questions with the same answer
/// here.
pub(super) fn record_studio_main_pass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut gizmo::renderer::Renderer,
    world: &gizmo::core::World,
    flat_batches: &[FlatBatchData],
    game_view_proj: Option<Mat4>,
    debug_aabbs: &[Aabb],
    show_colliders: bool,
    draw_chrome: bool,
    ed_shading_mode: u32,
    // The game camera's `Camera::clear_color` in the game view; `None` keeps the editor's grey.
    clear_color: Option<[f32; 3]>,
) {
            let no_wire_color = renderer.scene.wire_color_group(&renderer.device, None);
            let [cr, cg, cb] = clear_color.unwrap_or([0.035, 0.035, 0.035]);
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass (HDR)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view, // Artık ekran yerine HDR texture'a çiziyoruz!
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Linear space 0.035 ~= sRGB 0.22 (Blender dark grey) after Gamma Correction / HDR
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: cr as f64,
                            g: cg as f64,
                            b: cb as f64,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });

            // 0. PAINTED BACKDROPS — FIRST, before any world geometry.
            //
            // The pass ordering here is not the depth pin's job: the pin keeps a backdrop from
            // occluding depth-tested geometry, but the transparent loop further down writes no
            // depth and blends, so a backdrop drawn after it would paint over it. Drawing them
            // first puts them underneath by construction. (`gizmo_renderer::backdrop`; the game
            // path expresses the same rule as `DrawLayer::Backdrop` in its draw-order sort.)
            render_pass.set_pipeline(&renderer.scene.backdrop_pipeline);
            for batch in flat_batches {
                if !batch.is_backdrop || !batch.visible_in_camera {
                    continue;
                }
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                if let Some(def) = renderer.deferred.as_ref() {
                    render_pass.set_bind_group(5, &def.irradiance.bind_group, &[]);
                }
                batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
            }

            // "Wire" is a PIPELINE, not a shading term: `wireframe_pipeline` is built from the
            // same `shader.wgsl` with `PolygonMode::Line` and has existed, unselected, since it
            // was written — nothing in the workspace ever set it. The toolbar's fourth chip is
            // what it is for.
            //
            // The uniform stays 0 for this mode. `deferred_lighting.wgsl` reads 3 as
            // "Roughness/Metallic", and one chip labelled "Wire" must not mean two different
            // pictures depending on which path runs.
            let wireframe_view = ed_shading_mode == SHADING_WIREFRAME;
            let engine_opaque = if wireframe_view {
                &renderer.scene.wireframe_pipeline
            } else {
                &renderer.scene.render_pipeline
            };
            render_pass.set_pipeline(engine_opaque);
            let mut bound: *const wgpu::RenderPipeline = engine_opaque;
            for batch in flat_batches {
                if batch.is_transparent
                    || batch.is_double_sided
                    || batch.is_skybox
                    || batch.is_grid
                    || batch.is_backdrop
                    || batch.is_water
                {
                    continue;
                } // Şeffafları, Skybox'ı, Çift Yönlüleri, Grid'i, Backdrop'u ve Su'yu atla
                if !batch.visible_in_camera {
                    continue; // ShadowCasting::Only — casts, is not drawn
                }
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                // A game's material draws with its own pipeline, but not in the wireframe view, where
                // every batch is the same lines.
                let pipeline = if wireframe_view { Some(engine_opaque) } else { batch_pipeline(renderer, batch, false, engine_opaque) };
                let Some(pipeline) = pipeline else { continue };
                if !std::ptr::eq(bound, pipeline) {
                    render_pass.set_pipeline(pipeline);
                    bound = pipeline;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                // The "Wire" chip's pipeline has the line colour at group 5, not the probes.
                if ed_shading_mode == SHADING_WIREFRAME {
                    render_pass.set_bind_group(5, &*no_wire_color, &[]);
                } else if let Some(def) = renderer.deferred.as_ref() {
                    render_pass.set_bind_group(5, &def.irradiance.bind_group, &[]);
                }
                batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
            }

            // 2. ÇİFT YÖNLÜ OPAQUE OBJELER (Kumaşlar, cull_mode = None)
            let engine_two_sided = &renderer.scene.render_double_sided_pipeline;
            render_pass.set_pipeline(engine_two_sided);
            let mut bound: *const wgpu::RenderPipeline = engine_two_sided;
            for batch in flat_batches {
                // `ShadowCasting::Only`: casts, is not drawn. This loop, the sky's, the transparent
                // one, the glass one and the grid's drew it until 2026-11-11; the single-sided one
                // and the water's asked.
                if !batch.visible_in_camera {
                    continue;
                }
                if batch.is_transparent
                    || !batch.is_double_sided
                    || batch.is_skybox
                    || batch.is_grid
                    || batch.is_backdrop
                    || batch.is_water
                {
                    continue;
                }
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let Some(pipeline) = batch_pipeline(renderer, batch, false, engine_two_sided) else { continue };
                if !std::ptr::eq(bound, pipeline) {
                    render_pass.set_pipeline(pipeline);
                    bound = pipeline;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                if let Some(def) = renderer.deferred.as_ref() {
                    render_pass.set_bind_group(5, &def.irradiance.bind_group, &[]);
                }
                batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
            }

            // --- DRAW GPU PHYSICS SPHERES (Katı Obje olarak farz ediliyor) ---
            if let Some(physics) = &renderer.gpu_physics {
                physics.render_pass(&mut render_pass, renderer.scene.view_bind_group());
            }


            // 2b. SU YÜZEYİ — kendi boru hattı, kendi geçişi.
            //
            // Ayrı bir geçiş, çünkü `water.wgsl` VERTEX aşamasında Gerstner ötelemesi yapıyor:
            // yukarıdaki opak döngü onu `shader.wgsl` ile çizerdi, o da düzlemi düz bırakır. Ve
            // 2026-08-24'ten beri suyun örnek bayrağı 1,0 ("ertelenmiş PBR değil"), ki
            // `shader.wgsl` bunu "ışıkları atla" diye okuyor — yani orada kalsaydı su yalnız
            // farklı değil, DÜMDÜZ görünürdü.
            //
            // Oyun yolundaki `passes::forward` ile aynı seçim: `with_double_sided` burada gerçek
            // iş yapıyor, çünkü su boru hattı saydam boru hattının aksine arka yüzleri eliyor.
            for two_sided in [false, true] {
                render_pass.set_pipeline(if two_sided {
                    &renderer.scene.water_double_sided_pipeline
                } else {
                    &renderer.scene.water_pipeline
                });
                for batch in flat_batches {
                    if !batch.is_water || batch.is_double_sided != two_sided {
                        continue;
                    }
                    if !batch.visible_in_camera {
                        continue; // ShadowCasting::Only — casts, is not drawn
                    }
                    if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                        continue;
                    }
                    let safe_end =
                        std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                    render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                    render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                    render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                    render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                    render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                    if let Some(def) = renderer.deferred.as_ref() {
                        render_pass.set_bind_group(5, &def.irradiance.bind_group, &[]);
                    }
                    batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
                }
            }

            // 3. SKYBOX YAKALAMA VE ÖZEL PIPELINE İLE ÇİZİM
            render_pass.set_pipeline(&renderer.scene.sky_pipeline);
            for batch in flat_batches {
                if !batch.is_skybox || !batch.visible_in_camera {
                    continue;
                } // Sadece Skybox'u çiz
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]); // sky.wgsl içinde boş da olsa bağlı kalması gerek
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                if let Some(def) = renderer.deferred.as_ref() {
                    render_pass.set_bind_group(5, &def.irradiance.bind_group, &[]);
                }
                batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
            }

            // The image behind the scene, as the game path has it. The pass is shared, so it is drawn
            // here, after the opaque world, with the depth test keeping it behind.
            if let (true, Some(def)) = (renderer.environment_map.is_some(), renderer.deferred.as_ref()) {
                render_pass.set_pipeline(&def.environment_sky_pipeline);
                render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                render_pass.set_bind_group(1, &def.irradiance.bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            // 3a. Wireframes asked for per entity or by `WireframeConfig::global`, over the solids.
            for batch in flat_batches {
                let Some(wire) = batch.wire.as_ref().filter(|_| {
                    !batch.is_skybox && !batch.is_backdrop && !batch.is_grid && batch.visible_in_camera
                }) else {
                    continue;
                };
                render_pass.set_pipeline(&renderer.scene.wireframe_pipeline);
                render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                render_pass.set_bind_group(5, &**wire, &[]);
                batch.record_draw(&mut render_pass, batch.start_instance..batch.end_instance.min(renderer.scene.instance_capacity as u32));
            }

            // 3b. Glass reads the frame behind it, so with glass in view this pass ends here, the
            // frame is copied, and another takes over: the game path's order (`passes::forward`).
            let glass = |batch: &&FlatBatchData| batch.is_transmissive && batch.end_instance > batch.start_instance;
            let mut render_pass = match renderer.behind.as_ref().filter(|_| flat_batches.iter().any(|b| glass(&b))) {
                None => render_pass,
                Some(behind) => {
                    drop(render_pass);
                    behind.capture(encoder, &renderer.post.hdr_texture);
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Main Render Pass, glass and after (HDR)"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &renderer.post.hdr_texture_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                        })],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &renderer.depth_texture_view,
                            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
                            stencil_ops: None,
                        }),
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    for batch in flat_batches.iter().filter(glass).filter(|b| b.visible_in_camera) {
                        let engine = renderer.scene.transmissive_pipeline(batch.is_double_sided);
                        let Some(pipeline) = batch_pipeline(renderer, batch, true, engine) else { continue };
                        pass.set_pipeline(pipeline);
                        pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                        pass.set_bind_group(1, &*batch.bind_group, &[]);
                        pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                        pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                        pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                        if let Some(def) = renderer.deferred.as_ref() {
                            pass.set_bind_group(5, &def.irradiance.bind_group, &[]);
                        }
                        batch.record_draw(&mut pass, batch.start_instance..batch.end_instance.min(renderer.scene.instance_capacity as u32));
                    }
                    pass
                }
            };

            // 4. TRANSPARENT OBJELERİ ÇİZ (Depth yazması kapalı, Opaque'nin üstüne blend olur)
            for batch in flat_batches {
                if !batch.is_transparent || batch.is_transmissive || batch.is_grid || batch.is_backdrop || batch.is_water {
                    continue;
                } // Sadece saydamları çiz (backdrop ve su kendi geçişlerinde çizildi)
                if !batch.visible_in_camera {
                    continue;
                }
                // The game path's choice (`passes::forward`): the batch's blend mode picks a
                // variant, and an unlit draw stays unlit — this loop used to shade every
                // transparent batch with the lit shader, unlit ones included.
                let engine = match renderer.scene.blend_variant(batch.is_unlit, batch.blend_mode) {
                    Some(p) => p,
                    None if batch.is_unlit => &renderer.scene.unlit_blended_pipeline,
                    None => &renderer.scene.transparent_pipeline,
                };
                let Some(pipeline) = batch_pipeline(renderer, batch, true, engine) else { continue };
                render_pass.set_pipeline(pipeline);
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                if let Some(def) = renderer.deferred.as_ref() {
                    render_pass.set_bind_group(5, &def.irradiance.bind_group, &[]);
                }
                batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
            }

            // 5. GRID ÇİZİMİ (Play modunda gizle — Game View temiz görünsün)
            // `draw_chrome` folds in: the game view is never chrome, whatever the mode.
            let is_playing_mode = !draw_chrome || world.get_resource::<gizmo::editor::EditorState>()
                .map(|ed| ed.is_playing() || ed.mode == gizmo::editor::EditorMode::Paused)
                .unwrap_or(false);
            // The "Grid Çizgilerini Göster" checkbox, which until now wrote a preference to disk
            // that nothing read back.
            //
            // The default is `true`, not `false`: with no `EditorState` in the world — every
            // headless render test — falsy would blank the grid and quietly change what those
            // tests measure. It also matches `EditorPrefs::default()`.
            let show_grid = world
                .get_resource::<gizmo::editor::EditorState>()
                .map(|ed| ed.prefs.show_grid)
                .unwrap_or(true);
            if !is_playing_mode && show_grid {
                render_pass.set_pipeline(&renderer.scene.grid_pipeline);
                for batch in flat_batches {
                    if !batch.is_grid || !batch.visible_in_camera {
                        continue;
                    }
                    if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                        continue;
                    }
                    let safe_end =
                        std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                    render_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
                    render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                    render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                    render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                    render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                    if let Some(def) = renderer.deferred.as_ref() {
                        render_pass.set_bind_group(5, &def.irradiance.bind_group, &[]);
                    }
                    batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
                }
            }

            // GPU particles are NOT drawn here — see `gizmo::systems::render::record_gpu_particles`,
            // which the caller runs after this pass ends. They need a pass with no depth ATTACHMENT
            // so the depth texture can be sampled instead, and setting their pipeline in this pass
            // is a validation error that takes the editor down on its first frame.
            // --- 5. GIZMOS VE DEBUG LINES ÇİZİMİ (Play modunda gizle) ---
            if !is_playing_mode {
                // The frame's lines as the preparation ended them (`GizmoFrame`: the game's, and the
                // editor's own from its update), with this view's added to a copy. The draw used
                // to add them to the game's `Gizmos` and clear it afterwards — writes into the
                // world from the draw, and a clear that dropped the last fixed step's lines, which
                // the game's frame keeps until the next step.
                if let Some(frame) = world.get_resource::<gizmo::renderer::GizmoFrame>() {
                    let mut gizmos = frame.gizmos().clone();
                    // Game Camera Frustum'unu Yeşil çiz
                    if let Some(vp) = game_view_proj {
                        gizmos.draw_frustum(vp, [0.0, 1.0, 0.0, 1.0]); // Yeşil
                    }

                    // Ekranda kalan (Cull edilmeyen) objelerin AABB'lerini Kırmızı çiz
                    for aabb in debug_aabbs {
                        gizmos.draw_aabb(*aabb, [1.0, 0.0, 0.0, 1.0]); // Kırmızı
                    }

                    if let Some(debug_renderer) = &mut renderer.debug_renderer {
                        debug_renderer.update(&renderer.queue, &gizmos);
                        debug_renderer.render(
                            &mut render_pass,
                            renderer.scene.view_bind_group(),
                            gizmos.depth_test,
                        );
                    }
                }
            }

            if show_colliders && draw_chrome {
                if let Some(physics) = &renderer.gpu_physics {
                    physics.debug_render_pass(&mut render_pass, renderer.scene.view_bind_group());
                }
            }
}

