use super::super::*;

pub fn record_screen_space_effects(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer) {


    // ── SSR: Screen Space Reflections ───────────────────────────────────────────
    if let Some(ref ssr) = renderer.ssr {
        // Pass 1: SSR Raymarch
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSR Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ssr.ssr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&ssr.ssr_pipeline);
            pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            pass.set_bind_group(1, &ssr.ssr_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: SSR Apply (Additive blend into HDR texture)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSR Apply Pass"),
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
            pass.set_pipeline(&ssr.apply_pipeline);
            pass.set_bind_group(0, &ssr.apply_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // ── SSGI: Screen Space Global Illumination ────────────────────────────────
    if let Some(ref ssgi) = renderer.ssgi {
        // Pass 1: SSGI Raymarch
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ssgi.ssgi_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&ssgi.ssgi_pipeline);
            pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            pass.set_bind_group(1, &ssgi.ssgi_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 1.5: SSGI Temporal Accumulation — blend the frame-varying 1-spp raymarch
        // with reprojected history to converge the Monte-Carlo grain. Writes the current
        // ping-pong accumulation buffer, which the blur then reads.
        {
            let (resolve_bg, output_view) = ssgi.current_temporal_io();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Temporal Resolve Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&ssgi.temporal_pipeline);
            pass.set_bind_group(0, resolve_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: SSGI Blur
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Blur Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ssgi.ssgi_blurred_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&ssgi.blur_pipeline);
            pass.set_bind_group(0, ssgi.current_blur_bg(), &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 3: SSGI Apply (Additive blend into HDR texture)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Apply Pass"),
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
            pass.set_pipeline(&ssgi.apply_pipeline);
            pass.set_bind_group(0, &ssgi.apply_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // ── Volumetric Lighting (God Rays) ──────────────────────────────────────────
    if let Some(ref vol) = renderer.volumetric {
        // Pass 1: Volumetric Raymarch
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Volumetric Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &vol.volumetric_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&vol.volumetric_pipeline);
            pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            pass.set_bind_group(1, &renderer.scene.shadow_bind_group, &[]);
            pass.set_bind_group(2, &vol.volumetric_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: Volumetric Apply (Additive blend into HDR texture)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Volumetric Apply Pass"),
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
            pass.set_pipeline(&vol.apply_pipeline);
            pass.set_bind_group(0, &vol.apply_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

}
