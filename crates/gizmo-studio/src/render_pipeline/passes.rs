//! Studio-editor render passes + editor-setting sync, extracted verbatim from
//! render_pipeline.rs. `record_studio_*` consume the `FlatBatchData` list produced by the
//! batching step; `sync_editor_settings` mirrors egui state into the renderer.

use super::*;
use super::batching::FlatBatchData;

pub(super) fn sync_editor_settings(world: &gizmo::core::World, renderer: &mut gizmo::renderer::Renderer) -> (f32, u32, bool) {
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
    
    let mut post_params = gizmo::renderer::renderer::PostProcessUniforms {
        bloom_intensity: 0.8,
        bloom_threshold: 0.85,
        exposure: 1.0,
        vignette_intensity: 0.2,
        chromatic_aberration: 0.005,
        film_grain_intensity: 0.0,
        dof_focus_dist: 10.0,
        dof_focus_range: 20.0,
        dof_blur_size: 2.0,
        cam_near: 0.1,
        cam_far: 2000.0,
        underwater: 0.0,
        fog_r: 0.0,
        fog_g: 0.0,
        fog_b: 0.0,
        fog_density: 0.0,
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
    }

    renderer.update_post_process(&renderer.queue, post_params);

    if let Some(ref mut fxaa) = renderer.fxaa {
        if fxaa.enabled != ed_fxaa_enabled {
            fxaa.enabled = ed_fxaa_enabled;
            fxaa.set_enabled(&renderer.queue, ed_fxaa_enabled);
        }
    }

    if let Some(ref mut ssao) = renderer.ssao {
        let actual_strength = if ed_ssao_enabled { ed_ssao_strength } else { 0.0 };
        ssao.set_strength(&renderer.queue, actual_strength);
    }

    (aspect, ed_shading_mode, show_colliders)
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
                if batch.is_transparent || batch.is_skybox || batch.is_grid || batch.is_unlit {
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
                shadow_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                shadow_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
            }
        }
}

pub(super) fn record_studio_main_pass(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut gizmo::renderer::Renderer,
    world: &gizmo::core::World,
    flat_batches: &[FlatBatchData],
    game_view_proj: Option<Mat4>,
    debug_aabbs: &[Aabb],
    show_colliders: bool,
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

            render_pass.set_pipeline(&renderer.scene.render_pipeline);
            for batch in flat_batches {
                if batch.is_transparent || batch.is_double_sided || batch.is_skybox || batch.is_grid
                {
                    continue;
                } // Şeffafları, Skybox'ı, Çift Yönlüleri ve Grid'i atla
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
                render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
            }

            // 2. ÇİFT YÖNLÜ OPAQUE OBJELER (Kumaşlar, cull_mode = None)
            render_pass.set_pipeline(&renderer.scene.render_double_sided_pipeline);
            for batch in flat_batches {
                if batch.is_transparent
                    || !batch.is_double_sided
                    || batch.is_skybox
                    || batch.is_grid
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
                render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
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
                render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
            }

            // 4. TRANSPARENT OBJELERİ ÇİZ (Depth yazması kapalı, Opaque'nin üstüne blend olur)
            render_pass.set_pipeline(&renderer.scene.transparent_pipeline);
            for batch in flat_batches {
                if !batch.is_transparent || batch.is_grid {
                    continue;
                } // Sadece saydamları çiz
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
                render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
            }

            // 5. GRID ÇİZİMİ (Play modunda gizle — Game View temiz görünsün)
            let is_playing_mode = world.get_resource::<gizmo::editor::EditorState>()
                .map(|ed| ed.is_playing() || ed.mode == gizmo::editor::EditorMode::Paused)
                .unwrap_or(false);
            if !is_playing_mode {
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
                    render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                    render_pass.draw(0..batch.vertex_count, batch.start_instance..safe_end);
                }
            }

            // --- 4. DRAW GPU PARTICLES (Billboard & Şeffaf) ---
            if let Some(gpu_particles) = &renderer.gpu_particles {
                render_pass.set_pipeline(&gpu_particles.pipelines.render_pipeline);
                render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
                render_pass.set_vertex_buffer(0, gpu_particles.quad_vertex_buffer.slice(..));
                render_pass.set_vertex_buffer(1, gpu_particles.particles_buffer.slice(..));
                render_pass.draw(0..4, 0..gpu_particles.active_particles);
            }
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

            if show_colliders {
                if let Some(physics) = &renderer.gpu_physics {
                    physics.debug_render_pass(&mut render_pass, &renderer.scene.global_bind_group);
                }
            }
}
