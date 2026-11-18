use super::super::*;

pub fn record_deferred_geometry(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    world: &World,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    cam_pos: gizmo_math::Vec3,
    // Whether the opaque baked-lit surfaces go into the G-buffer this view (`DrawItem::in_gbuffer`).
    baked_in_gbuffer: bool,
) {

    // ── Z-Prepass (Depth Only) ────────────────────────────────────────────────
    if let Some(ref def) = renderer.deferred {
        let mut z_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Z-Prepass"),
            color_attachments: &[], // No color targets
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
        z_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
        z_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        z_pass.set_bind_group(4, renderer.scene.view_instance_bind_group(), &[]);
        z_pass.set_bind_group(5, &def.motion.bind_group, &[]);
        // The cull mode is baked into the pipeline, so a double-sided material needs its own.
        // Tracked and set only on change rather than per item: the batches are not sorted by this
        // flag and a scene is overwhelmingly one-sided, so this is a handful of switches at most.
        // Through a plan: it binds what changes only, and draws a run sharing a pipeline, a material
        // and a buffer as one multi-draw.
        let mut plan = super::plan::PassPlan::default();
        let mut bound: Option<(Option<gizmo_renderer::custom_material::SurfaceId>, bool, bool)> = None;
        for item in draw_items {
            // `ShadowCasting::Only`: casts, is not drawn. Checked in the z-prepass as well as the
            // G-buffer below, or the object would still write depth and punch a hole in whatever
            // stands behind it.
            if !item.visible_in_camera {
                continue;
            }
            // Not a baked-lit surface either: it writes its own depth in the G-buffer pass (`fs_baked`'s
            // pipelines), where shading it again on an overlap costs less than drawing it twice.
            if !item.in_gbuffer(baked_in_gbuffer) || item.baked_lit || item.is_skybox || item.is_transparent {
                continue;
            }
            let key = (item.surface, item.is_double_sided, item.discards);
            if bound != Some(key) {
                let pipeline = match item.surface {
                    // A game's surface cuts where its own code says, so its prepass always has the
                    // fragment stage; an id with nothing behind it draws nothing, here as everywhere.
                    Some(id) => match renderer.custom_materials.surface(id) {
                        // A surface that never cuts leaves the depth a plain solid does.
                        Some(s) if s.is_solid() => match (item.discards, item.is_double_sided) {
                            (true, two_sided) => &def.z_prepass_discard_pipelines[usize::from(two_sided)],
                            (false, true) => &def.z_prepass_double_sided_pipeline,
                            (false, false) => &def.z_prepass_pipeline,
                        },
                        Some(s) => match s.prepass_pipeline(item.is_double_sided) {
                            Some(pipeline) => pipeline,
                            None => continue,
                        },
                        None => continue,
                    },
                    None => match (item.discards, item.is_double_sided) {
                        (true, two_sided) => &def.z_prepass_discard_pipelines[usize::from(two_sided)],
                        (false, true) => &def.z_prepass_double_sided_pipeline,
                        (false, false) => &def.z_prepass_pipeline,
                    },
                };
                plan.set_pipeline(pipeline);
                bound = Some(key);
            }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            plan.set_bind_group(3, skel_bg.as_ref());
            plan.set_bind_group(1, &item.bind_group);
            // Main pass: camera-visible instances only.
            item.plan_draw(&mut plan, item.camera_instance_range(uploaded_instances));
        }
        plan.replay(&mut z_pass, &renderer.device, renderer.multi_draw);
    }

    // ── G-buffer pass (PBR geometry → albedo / normal / world-position) ─────
    if let Some(ref def) = renderer.deferred {
        let mut gbuf_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("G-Buffer Pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.albedo_metallic_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.normal_roughness_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.world_position_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // TRANSPARENT, not BLACK: `.w` is the written flag every reader tests
                        // (`pos_sample.w < 0.5` = no geometry). BLACK's alpha is 1, so every empty
                        // pixel read as a surface at the camera — lit as one, and counted as an
                        // occluder at distance 0 by SSAO.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.world_tangent_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                // RT4: velocity — zero wherever no G-buffer geometry lands (the sky), so the
                // post pass falls back to the camera's own motion there.
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.velocity_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                // RT5: emissive — black where nothing glows.
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.emissive_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.depth_texture_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load, // Z-Prepass populated this!
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        gbuf_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
        gbuf_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        gbuf_pass.set_bind_group(4, renderer.scene.view_instance_bind_group(), &[]);
        gbuf_pass.set_bind_group(5, &def.motion.bind_group, &[]);
        let mut plan = super::plan::PassPlan::default();
        let mut bound: Option<(Option<gizmo_renderer::custom_material::SurfaceId>, bool, bool)> = None;
        for item in draw_items {
            if !item.visible_in_camera {
                continue; // ShadowCasting::Only
            }
            if !item.in_gbuffer(baked_in_gbuffer) || item.is_transparent {
                continue;
            }
            let key = (item.surface, item.is_double_sided, item.baked_lit);
            if bound != Some(key) {
                let pipeline = match item.surface {
                    // The engine's G-buffer shader around the game's surface: written as `Pbr` is.
                    Some(id) => match renderer.custom_materials.surface(id).and_then(|s| s.gbuffer_pipeline(item.is_double_sided)) {
                        Some(pipeline) => pipeline,
                        None => continue,
                    },
                    // A baked-lit surface, finished by the lighting pass as `baked_lit.wgsl` draws it.
                    None if item.baked_lit => &def.gbuffer_baked_pipelines[usize::from(item.is_double_sided)],
                    None if item.is_double_sided => &def.gbuffer_double_sided_pipeline,
                    None => &def.gbuffer_pipeline,
                };
                plan.set_pipeline(pipeline);
                bound = Some(key);
            }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            plan.set_bind_group(3, skel_bg.as_ref());
            plan.set_bind_group(1, &item.bind_group);
            // Main pass: camera-visible instances only.
            item.plan_draw(&mut plan, item.camera_instance_range(uploaded_instances));
        }
        plan.replay(&mut gbuf_pass, &renderer.device, renderer.multi_draw);
    }

    // ── Decal Pass (Blend into G-buffer) ──────────────────────────
    // What to draw comes from the shared collector, so this pass and the editor's forward one
    // cannot disagree about where a decal is — only about which target it lands in.
    let (decal_uniforms, decal_draws) = super::super::collect_decals(world, cam_pos);
    if let Some(ref decal_state) = renderer.decal {
        if !decal_uniforms.is_empty() {
            renderer.queue.write_buffer(
                &decal_state.uniform_buffer,
                0,
                bytemuck::cast_slice(&decal_uniforms),
            );
        }
    }

    if !decal_draws.is_empty() {
        if let (Some(decal_state), Some(ref def)) =
            (renderer.decal.as_ref().filter(|d| d.enabled), &renderer.deferred)
        {
            let mut decal_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Decal Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &def.albedo_metallic_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None, // No depth testing needed
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });

            decal_pass.set_pipeline(&decal_state.pipeline);
            decal_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
            decal_pass.set_bind_group(1, &decal_state.world_pos_bg, &[]);
            decal_pass.set_vertex_buffer(0, decal_state.vertex_buffer.slice(..));

            for (i, bind_group) in decal_draws.iter().enumerate() {
                let offset = (i * 256) as u32;
                decal_pass.set_bind_group(2, bind_group.as_ref(), &[]);
                decal_pass.set_bind_group(3, &decal_state.decal_uniform_bg, &[offset]);
                decal_pass.draw(0..36, 0..1);
            }
        }
    }

    // ── Deferred lighting pass (G-buffers → HDR) ──────────────────────────
    if let Some(ref def) = renderer.deferred {
        let mut light_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Deferred Lighting Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.4, g: 0.6, b: 0.9, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        light_pass.set_pipeline(&def.lighting_pipeline);
        light_pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
        light_pass.set_bind_group(1, &renderer.scene.shadow_bind_group, &[]);
        light_pass.set_bind_group(2, &def.gbuffer_bind_group, &[]);
        // Group 3 is always bound because the pipeline layout always declares it; an empty grid
        // is a one-probe buffer of zeros and `gi_sample` returns black for it.
        light_pass.set_bind_group(3, &def.irradiance.bind_group, &[]);
        light_pass.draw(0..3, 0..1);
    }

}
