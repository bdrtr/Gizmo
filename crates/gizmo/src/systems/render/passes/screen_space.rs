use super::super::*;

/// SSR and SSGI, added to the frame between the opaque world and what blends over it: glass,
/// transparent surfaces, particles and the fluid are drawn after, so each shows them as much as it lets
/// through. Added after everything until 2026-11-11, so a pane 98 % opaque took the floor's reflection
/// behind it at full strength. The cost is that the reflection no longer shows what blends: it is a
/// reflection of the opaque frame, which is what every screen-space reflection reads.
///
/// `renderer`'s GPU timer marks each stage; it is taken out for the while, as the stages borrow the rest.
pub fn record_surface_effects(encoder: &mut wgpu::CommandEncoder, renderer: &mut Renderer) {
    let mut timer = renderer.gpu_timer.take();
    let mut mark = |encoder: &mut wgpu::CommandEncoder, label: &'static str| {
        if let Some(timer) = timer.as_mut() {
            timer.mark(encoder, label);
        }
    };
    record_surface_effects_marked(encoder, renderer, &mut mark);
    renderer.gpu_timer = timer;
}

fn record_surface_effects_marked(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    mark: &mut dyn FnMut(&mut wgpu::CommandEncoder, &'static str),
) {
    // ── How much of the G-buffer's surface the frame shows ─────────────────────
    // SSR and SSGI belong to that surface and are added after the opaque forward draws, so in
    // proportion to it: not over an unlit or baked-lit surface, or a game's own pipeline, drawn in
    // front of it.
    if let Some(def) = renderer.deferred.as_ref().filter(|_| surface_effects_on(renderer)) {
        def.record_seen(encoder, renderer.scene.view_bind_group());
        mark(encoder, "gpu.seen");
    }

    // ── SSR: Screen Space Reflections ───────────────────────────────────────────
    if let Some(ssr) = renderer.ssr.as_ref().filter(|s| s.enabled) {
        // The shaping numbers go up every frame — 48 bytes, no rebuild.
        let frame = renderer.taa.as_ref().filter(|t| t.enabled).map_or(0, |t| t.frame_index);
        ssr.upload(&renderer.queue, frame);
        ssr.record_prepare(encoder);
        mark(encoder, "gpu.ssr_prepare");

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
            pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
            pass.set_bind_group(1, &ssr.ssr_bind_group, &[]);
            if let Some(def) = renderer.deferred.as_ref() {
                pass.set_bind_group(2, &def.irradiance.bind_group, &[]);
            }
            pass.draw(0..3, 0..1);
        }
        mark(encoder, "gpu.ssr_march");

        // Pass 2: the glossy resolve, spreading each reflection over its surface's lobe.
        ssr.record_blur(encoder, renderer.scene.view_bind_group());
        mark(encoder, "gpu.ssr_blur");

        // Pass 3: SSR Apply (Additive blend into HDR texture)
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
        mark(encoder, "gpu.ssr_apply");
    }

    // ── SSGI: Screen Space Global Illumination ────────────────────────────────
    if let Some(ssgi) = renderer.ssgi.as_ref().filter(|s| s.enabled) {
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
            pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
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
}

/// Whether SSR or SSGI draws this frame: what [`record_surface_effects`] records anything for.
pub fn surface_effects_on(renderer: &Renderer) -> bool {
    renderer.ssr.as_ref().is_some_and(|s| s.enabled) || renderer.ssgi.as_ref().is_some_and(|s| s.enabled)
}

/// Whether the volumetric light draws this frame: on, and something in the view scatters.
pub fn volumetric_on(renderer: &Renderer) -> bool {
    renderer.volumetric.as_ref().is_some_and(|s| s.draws())
}

/// The volumetric march, between the opaque world and what blends, and the air behind the nearest
/// surface that blends added there: what blends is drawn over it and shows it as much as it lets through.
/// The air in front is added over everything by [`record_screen_space_effects`]. The march splits the
/// air at the depth [`record_veil_depth`](super::forward) drew; with nothing that blends (`blends`
/// false), all of it is in front, the term behind is nothing, and its full-screen pass is not drawn.
pub fn record_volumetric_march(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer, blends: bool) {
    let Some(vol) = renderer.volumetric.as_ref().filter(|s| s.draws()) else {
        return;
    };
    // The shaping numbers go up every frame, so `vol.params.steps = 8.0` is the whole gesture — no
    // rebuild, no bind-group churn. 32 bytes.
    renderer.queue.write_buffer(&vol.params_buffer, 0, bytemuck::bytes_of(&vol.params));
    {
        let target = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Volumetric Pass"),
            color_attachments: &[target(&vol.volumetric_view), target(&vol.behind_view)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&vol.volumetric_pipeline);
        pass.set_bind_group(0, renderer.scene.view_bind_group(), &[]);
        pass.set_bind_group(1, &renderer.scene.shadow_bind_group, &[]);
        pass.set_bind_group(2, &vol.volumetric_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    if blends {
        apply_volumetric(encoder, renderer, &vol.behind_apply_bind_group, "Volumetric Apply Pass (behind)");
    }
}

/// The air in front of the nearest surface that blends, added over the whole frame, after what blends.
pub fn record_screen_space_effects(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer) {
    if let Some(vol) = renderer.volumetric.as_ref().filter(|s| s.draws()) {
        apply_volumetric(encoder, renderer, &vol.apply_bind_group, "Volumetric Apply Pass");
    }
}

/// One of the march's two terms, added into the HDR target.
fn apply_volumetric(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer, term: &wgpu::BindGroup, label: &str) {
    let Some(vol) = renderer.volumetric.as_ref() else {
        return;
    };
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &renderer.post.hdr_texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&vol.apply_pipeline);
    pass.set_bind_group(0, term, &[]);
    pass.set_bind_group(1, renderer.scene.view_bind_group(), &[]);
    pass.draw(0..3, 0..1);
}
