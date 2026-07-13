use super::super::*;

pub fn record_ssao(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer) {
    // ── SSAO: hemisphere sampling → raw AO texture ────────────────────────────
    if let Some(ref ssao) = renderer.ssao {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ssao.ao_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&ssao.ssao_pipeline);
        pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        pass.set_bind_group(1, &ssao.ssao_gbuf_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // ── SSAO blur: 5×5 box filter → blurred AO texture ───────────────────────
    if let Some(ref ssao) = renderer.ssao {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Blur Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ssao.ao_blurred_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&ssao.blur_pipeline);
        pass.set_bind_group(0, &ssao.blur_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // ── SSAO apply: multiply AO into HDR (multiply blend) ─────────────────────
    if let Some(ref ssao) = renderer.ssao {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Apply Pass"),
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
        pass.set_pipeline(&ssao.apply_pipeline);
        pass.set_bind_group(0, &ssao.apply_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

}
