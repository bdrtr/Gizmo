use super::super::*;

#[cfg(not(target_arch = "wasm32"))]
pub fn record_shadow_passes(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
) {
    // CSM shadow passes — one depth-only pass per cascade.
    for i in 0..crate::renderer::CASCADE_COUNT {
        let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Shadow Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.scene.shadow_cascade_layer_views[i],
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
        shadow_pass.set_bind_group(0, &renderer.scene.shadow_pass_bind_groups[i], &[]);
        shadow_pass.set_bind_group(2, &renderer.scene.instance_bind_group, &[]);
        for item in draw_items {
            if item.unlit || item.is_transparent {
                continue;
            }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            shadow_pass.set_bind_group(1, skel_bg.as_ref(), &[]);
            shadow_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            // Two-region instance layout (see collect_draw_items): the camera-visible
            // instances (region A) and the off-screen shadow-only casters (region B) are no
            // longer contiguous, so cast shadows from BOTH ranges with a draw each. Empty
            // ranges (e.g. a batch with no off-screen casters) draw 0 instances = no-op.
            shadow_pass.draw(
                0..item.vertex_count,
                item.camera_instance_range(uploaded_instances),
            );
            shadow_pass.draw(
                0..item.vertex_count,
                item.shadow_instance_range(uploaded_instances),
            );
        }
    }

    // Point Light Shadow Passes — 6 faces
    for i in 0..6 {
        let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Point Shadow Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.scene.point_shadow_face_views[i],
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
        // We reuse the directional shadow pipeline because it only does position transformation
        shadow_pass.set_pipeline(&renderer.scene.shadow_pipeline);
        shadow_pass.set_bind_group(0, &renderer.scene.point_shadow_pass_bind_groups[i], &[]);
        shadow_pass.set_bind_group(2, &renderer.scene.instance_bind_group, &[]);
        for item in draw_items {
            if item.unlit || item.is_transparent { continue; }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            shadow_pass.set_bind_group(1, skel_bg.as_ref(), &[]);
            shadow_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            // Both instance regions cast into the point-shadow faces (see the directional
            // pass above for why the two ranges are non-contiguous).
            shadow_pass.draw(
                0..item.vertex_count,
                item.camera_instance_range(uploaded_instances),
            );
            shadow_pass.draw(
                0..item.vertex_count,
                item.shadow_instance_range(uploaded_instances),
            );
        }
    }

}
