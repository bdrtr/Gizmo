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
    
    // The editor's look before `EditorState` overrides it: the renderer's neutral default, minus
    // the film grain (it reads as noise on a static viewport) and with a mild aberration and a
    // real DoF blur, which are the editor's own taste. Everything unnamed is the default.
    let mut post_params = gizmo::renderer::PostProcessUniforms {
        exposure: 1.0,
        vignette_intensity: 0.2,
        chromatic_aberration: 0.005,
        film_grain_intensity: 0.0,
        dof_focus_dist: 10.0,
        dof_focus_range: 20.0,
        dof_blur_size: 2.0,
        ..Default::default()
    };

    if let Some(ed_state) = world.get_resource::<gizmo::editor::EditorState>() {
        ed_shading_mode = ed_state.shading_mode;
        ed_fxaa_enabled = ed_state.post_process.fxaa_enabled;
        ed_ssao_enabled = ed_state.post_process.ssao_enabled;
        ed_ssao_strength = ed_state.post_process.ssao_strength;
        
        show_colliders = ed_state.show_colliders;
        post_params.bloom_intensity = ed_state.post_process.bloom_intensity;
        post_params.bloom_threshold = ed_state.post_process.bloom_threshold;
        post_params.exposure = ed_state.post_process.exposure;
        post_params.vignette_intensity = ed_state.post_process.vignette;
        post_params.chromatic_aberration = ed_state.post_process.chromatic_aberration;
        post_params.dof_focus_dist = ed_state.post_process.dof_focus_dist;
        post_params.dof_focus_range = ed_state.post_process.dof_focus_range;
        post_params.dof_blur_size = ed_state.post_process.dof_blur_size;
        post_params.film_grain_intensity = ed_state.post_process.film_grain;

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
pub(super) fn record_studio_shadow_passes(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &gizmo::renderer::Renderer,
    flat_batches: &[FlatBatchData],
    light_view_proj_cascades: &[[[f32; 4]; 4]; 4],
) {
        for (cascade_i, &cascade_view_proj) in light_view_proj_cascades.iter().enumerate() {
            renderer.queue.write_buffer(
                &renderer.scene.shadow_cascade_uniform_buffers[cascade_i],
                0,
                gizmo::bytemuck::bytes_of(&gizmo::renderer::ShadowVsUniform {
                    light_view_proj: cascade_view_proj,
                }),
            );

            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&format!("Shadow Pass cascade {cascade_i}")),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.scene.shadow_cascade_layer_views[cascade_i],
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

            shadow_pass.set_pipeline(&renderer.scene.shadow_pipeline);

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
                {
                    continue;
                }
                // Per-object opt-out. The material-derived rules above still hold — they exclude
                // things that cannot sensibly cast at all — and this is the object's own answer on
                // top of them.
                if !batch.casts_shadows {
                    continue;
                }
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                // Shadow pass draws the FULL range (camera-visible + off-screen casters).
                let safe_end = std::cmp::min(
                    batch.shadow_end_instance,
                    renderer.scene.instance_capacity as u32,
                );

                shadow_pass.set_bind_group(
                    0,
                    &renderer.scene.shadow_pass_bind_groups[cascade_i],
                    &[],
                );
                shadow_pass.set_bind_group(1, &*batch.skeleton_bg, &[]);
                shadow_pass.set_bind_group(2, &renderer.scene.instance_bind_group, &[]);
                batch.record_draw(&mut shadow_pass, batch.start_instance..safe_end);
            }
        }
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
) {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass (HDR)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view, // Artık ekran yerine HDR texture'a çiziyoruz!
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Linear space 0.035 ~= sRGB 0.22 (Blender dark grey) after Gamma Correction / HDR
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.035,
                            g: 0.035,
                            b: 0.035,
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
                if !batch.is_backdrop {
                    continue;
                }
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
            }

            render_pass.set_pipeline(&renderer.scene.render_pipeline);
            for batch in flat_batches {
                if batch.is_transparent
                    || batch.is_double_sided
                    || batch.is_skybox
                    || batch.is_grid
                    || batch.is_backdrop
                {
                    continue;
                } // Şeffafları, Skybox'ı, Çift Yönlüleri, Grid'i ve Backdrop'u atla
                if !batch.visible_in_camera {
                    continue; // ShadowCasting::Only — casts, is not drawn
                }
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
            }

            // 2. ÇİFT YÖNLÜ OPAQUE OBJELER (Kumaşlar, cull_mode = None)
            render_pass.set_pipeline(&renderer.scene.render_double_sided_pipeline);
            for batch in flat_batches {
                if batch.is_transparent
                    || !batch.is_double_sided
                    || batch.is_skybox
                    || batch.is_grid
                    || batch.is_backdrop
                {
                    continue;
                }
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
            }

            // --- DRAW GPU PHYSICS SPHERES (Katı Obje olarak farz ediliyor) ---
            if let Some(physics) = &renderer.gpu_physics {
                physics.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
            }

            // 3. SKYBOX YAKALAMA VE ÖZEL PIPELINE İLE ÇİZİM
            render_pass.set_pipeline(&renderer.scene.sky_pipeline);
            for batch in flat_batches {
                if !batch.is_skybox {
                    continue;
                } // Sadece Skybox'u çiz
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]); // sky.wgsl içinde boş da olsa bağlı kalması gerek
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
            }

            // 4. TRANSPARENT OBJELERİ ÇİZ (Depth yazması kapalı, Opaque'nin üstüne blend olur)
            render_pass.set_pipeline(&renderer.scene.transparent_pipeline);
            for batch in flat_batches {
                if !batch.is_transparent || batch.is_grid || batch.is_backdrop {
                    continue;
                } // Sadece saydamları çiz (backdrop kendi geçişinde, en başta çizildi)
                if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                    continue;
                }
                let safe_end =
                    std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
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
                    if !batch.is_grid {
                        continue;
                    }
                    if batch.start_instance >= renderer.scene.instance_capacity as u32 {
                        continue;
                    }
                    let safe_end =
                        std::cmp::min(batch.end_instance, renderer.scene.instance_capacity as u32);

                    render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                    render_pass.set_bind_group(1, &*batch.bind_group, &[]);
                    render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
                    render_pass.set_bind_group(3, &*batch.skeleton_bg, &[]);
                    render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
                    batch.record_draw(&mut render_pass, batch.start_instance..safe_end);
                }
            }

            // GPU particles are NOT drawn here — see `record_studio_particle_pass`, which the
            // caller runs after this pass ends. They need a pass with no depth ATTACHMENT so the
            // depth texture can be sampled instead, and setting their pipeline in this pass is a
            // validation error that takes the editor down on its first frame.
            // --- 5. GIZMOS VE DEBUG LINES ÇİZİMİ (Play modunda gizle) ---
            if !is_playing_mode {
                if let Some(mut gizmos) = world.get_resource_mut::<gizmo::renderer::Gizmos>() {
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
                            &renderer.scene.global_bind_group,
                            gizmos.depth_test,
                        );
                    }
                }
            }

            if show_colliders && draw_chrome {
                if let Some(physics) = &renderer.gpu_physics {
                    physics.debug_render_pass(&mut render_pass, &renderer.scene.global_bind_group);
                }
            }
}

/// GPU particles, in their own pass — the way the game path has drawn them since the soft-particle
/// work landed.
///
/// # Why a separate pass
///
/// The particle fragment shader SAMPLES the scene depth, so particles fade out against geometry
/// instead of cutting into it. A texture cannot be a depth attachment and a sampled texture in the
/// same pass, so this pass has no depth attachment and binds the depth texture instead; occlusion
/// is done in the shader.
///
/// # Why this function exists
///
/// The editor did not do that. It set the particle pipeline inside the main pass — which has a
/// depth attachment — and left the depth bind group unbound, because it was still making the call
/// the way it looked before 2026-07-12, when the pipeline gained its depth binding and lost its
/// depth-stencil state. The game path was updated that day; this copy was not.
///
/// The result was not a subtle difference in the picture. `set_pipeline` with a pipeline whose
/// depth-stencil state does not match the pass is a wgpu validation error, so the studio panicked
/// on its first frame and had been unable to start since. Nothing caught it: the editor's render
/// path has no automated coverage, and the crash needs a window.
pub(super) fn record_studio_particle_pass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &gizmo::renderer::Renderer,
) {
    let Some(particles) = &renderer.gpu_particles else { return };
    if particles.active_particles == 0 {
        return;
    }

    let depth_bg = particles.create_depth_bind_group(&renderer.device, &renderer.depth_texture_view);
    let mut ppass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Studio Particle Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &renderer.post.hdr_texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    particles.render_pass(
        &mut ppass,
        &renderer.scene.global_bind_group,
        &depth_bg,
        particles.active_particles,
    );
}
