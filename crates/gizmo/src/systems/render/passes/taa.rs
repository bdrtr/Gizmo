use super::super::*;

/// The TAA resolve, then the frame's debug lines over it when `debug_lines`.
///
/// A capture — a reflection probe's face, a mirror's view — passes `false`: debug lines are an
/// overlay on the camera's picture, not something in the scene, and drawn into a capture they would
/// stay baked into the probe's cube until its next recapture. (Before the draw read the lines
/// without clearing them, the first draw of a frame consumed them, so a capture took the lines and
/// the main view lost them; neither was meant.)
pub fn record_taa_and_overlays(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    world: &World,
    debug_lines: bool,
) {
    // ── TAA resolve: blend jittered HDR with clamped history ─────────────────
    if let Some(ref taa) = renderer.taa {
        if taa.enabled {
            let (resolve_bg, output_view) = taa.current_resolve_inputs_output();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TAA Resolve Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&taa.resolve_pipeline);
            pass.set_bind_group(0, resolve_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // ── TAA blit: copy stabilized history output back into HDR texture ───────
    if let Some(ref taa) = renderer.taa {
        if taa.enabled {
            let blit_bg = taa.current_blit_bg();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TAA Blit Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&taa.blit_pipeline);
            pass.set_bind_group(0, &taa.empty_bg, &[]);
            pass.set_bind_group(1, blit_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // Render Gizmos AFTER TAA to avoid ghosting/washing out dynamic overlays. What the frame's
    // preparation took from the game's `Gizmos` (`GizmoFrame`): the draw reads it and leaves the
    // game's lines alone — ending their frame is `prepare_frame`'s.
    if let Some(frame) = world.get_resource::<crate::renderer::GizmoFrame>().filter(|_| debug_lines) {
        let gizmos = frame.gizmos();
        if let Some(debug_renderer) = &mut renderer.debug_renderer {
            debug_renderer.update(&renderer.queue, gizmos);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Gizmo Render Pass (Post-TAA)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Keep TAA-stabilized geometry scene
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            debug_renderer.render(
                &mut pass,
                renderer.scene.view_bind_group(),
                gizmos.depth_test,
            );
        }
    }

    // Advance TAA ping-pong and frame counter
    if let Some(ref mut taa) = renderer.taa {
        if taa.enabled {
            taa.advance_frame();
        }
    }
}
