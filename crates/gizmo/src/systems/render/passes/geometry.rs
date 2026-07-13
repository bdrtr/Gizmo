use super::super::*;

pub fn record_deferred_geometry(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    world: &World,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
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
        z_pass.set_pipeline(&def.z_prepass_pipeline);
        z_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        z_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        z_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
        for item in draw_items {
            if item.unlit || item.is_skybox || item.is_transparent {
                continue;
            }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            z_pass.set_bind_group(3, skel_bg.as_ref(), &[]);
            z_pass.set_bind_group(1, &*item.bind_group, &[]);
            z_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            z_pass.draw(
                0..item.vertex_count,
                // Main pass: camera-visible instances only.
                item.first_instance
                    ..(item.first_instance + item.camera_count)
                        .min(uploaded_instances)
                        .max(item.first_instance),
            );
        }
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
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
        gbuf_pass.set_pipeline(&def.gbuffer_pipeline);
        gbuf_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        gbuf_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        gbuf_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
        for item in draw_items {
            if item.unlit || item.is_transparent {
                continue;
            }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            gbuf_pass.set_bind_group(3, skel_bg.as_ref(), &[]);
            gbuf_pass.set_bind_group(1, &*item.bind_group, &[]);
            gbuf_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            gbuf_pass.draw(
                0..item.vertex_count,
                // Main pass: camera-visible instances only.
                item.first_instance
                    ..(item.first_instance + item.camera_count)
                        .min(uploaded_instances)
                        .max(item.first_instance),
            );
        }
    }

    // ── Decal Pass (Blend into G-buffer) ──────────────────────────
    let mut decal_draws = Vec::new();
    if let Some(ref decal_state) = renderer.decal {
        let decals = world.borrow::<crate::renderer::components::Decal>();
        let transforms = world.borrow::<gizmo_physics_core::Transform>();
        let mut uniform_data = Vec::new();

        for (id, decal) in decals.iter() {
            if let Some(trans) = transforms.get(id) {
                let model = trans.local_matrix;
                let inv_model = model.inverse();

                uniform_data.push(crate::renderer::decal::DecalUniforms {
                    inv_model: inv_model.to_cols_array(),
                    model: model.to_cols_array(),
                    albedo_color: [decal.color.x, decal.color.y, decal.color.z, decal.color.w],
                    _pad: [0.0; 28],
                });

                decal_draws.push(decal.bind_group.clone());
                if uniform_data.len() >= 1024 {
                    break;
                } // Max 1024 decals limit
            }
        }

        if !uniform_data.is_empty() {
            renderer.queue.write_buffer(
                &decal_state.uniform_buffer,
                0,
                bytemuck::cast_slice(&uniform_data),
            );
        }
    }

    if !decal_draws.is_empty() {
        if let (Some(ref decal_state), Some(ref def)) = (&renderer.decal, &renderer.deferred) {
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
            decal_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
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
        light_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        light_pass.set_bind_group(1, &renderer.scene.shadow_bind_group, &[]);
        light_pass.set_bind_group(2, &def.gbuffer_bind_group, &[]);
        light_pass.draw(0..3, 0..1);
    }

}
