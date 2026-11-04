use super::super::*;

pub fn record_taa_and_overlays(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    world: &mut World,
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

    // Render Gizmos AFTER TAA to avoid ghosting/washing out dynamic overlays
    if let Some(gizmos) = world.get_resource::<crate::renderer::Gizmos>() {
        if let Some(debug_renderer) = &mut renderer.debug_renderer {
            debug_renderer.update(&renderer.queue, &gizmos);
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

    end_gizmo_frame(world);

    // Advance TAA ping-pong and frame counter
    if let Some(ref mut taa) = renderer.taa {
        if taa.enabled {
            taa.advance_frame();
        }
    }
}

/// This frame's debug lines are drawn; the last physics step's stay (`Gizmos::fixed_lines`)
/// until the next step. This cleared everything, and only on frames that had stepped physics —
/// so a line drawn every frame was drawn twice on every frame that had not.
fn end_gizmo_frame(world: &crate::core::World) {
    if let Some(mut gizmos) = world.get_resource_mut::<crate::renderer::Gizmos>() {
        gizmos.clear();
    }
}

#[cfg(test)]
mod tests {
    /// A frame that stepped no physics still ends its own lines.
    #[test]
    fn a_frame_without_a_physics_step_still_ends_its_lines() {
        let mut world = crate::core::World::new();
        world.insert_resource(gizmo_core::time::PhysicsTime::new(60));
        world.insert_resource(crate::renderer::Gizmos::default());
        for _ in 0..3 {
            world
                .get_resource_mut::<crate::renderer::Gizmos>()
                .unwrap()
                .draw_line(crate::math::Vec3::ZERO, crate::math::Vec3::X, [1.0; 4]);
            super::end_gizmo_frame(&world);
        }
        assert_eq!(world.get_resource::<crate::renderer::Gizmos>().unwrap().vertex_count(), 0);
    }
}
