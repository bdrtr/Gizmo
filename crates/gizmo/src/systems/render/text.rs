//! The world-reading half of text, shared by both draw paths.
//!
//! # Why it is in the facade
//!
//! The same reason as [`super::decals::collect_decals`]: it needs both halves and they live in
//! different crates. [`TextRenderer`](gizmo_renderer::text::TextRenderer) and
//! [`Text`](gizmo_renderer::components::Text) are in `gizmo-renderer`, and `GlobalTransform` —
//! which is where a world label *is* — is in `gizmo-physics-core`, which the renderer does not
//! depend on. This is the lowest crate that can see both.
//!
//! Being `pub` is the point. `gizmo-studio` calls this rather than keeping a second copy, so the
//! two paths cannot drift about where a label sits — which is exactly the divergence
//! `gizmo-studio/tests/render_parity.rs` exists to catch, and exactly what happened to decals
//! before that test was written.

use gizmo_math::{Vec2, Vec3};

use crate::core::World;
use crate::renderer::components::Text;
use crate::renderer::Renderer;

/// Lays this frame's [`Text`] out, rasterises any glyph the atlas has not seen, and draws it.
///
/// Runs **after** TAA, into the HDR target and against the main depth buffer — the same slot the
/// debug gizmos use, and for the same reason: a temporal resolve smears an overlay that changes
/// every frame, and text is the most legible thing in a frame and so the most obviously smeared.
///
/// World labels are depth-tested (a label behind a wall is behind the wall); screen text is not.
/// A `Text` with no `GlobalTransform` still draws if it is screen-space — a HUD element has no
/// place in the world — and is skipped if it is not.
pub fn record_text(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    world: &World,
) {
    let texts = world.borrow::<Text>();
    if texts.iter().next().is_none() {
        return;
    }
    let transforms = world.borrow::<crate::prelude::GlobalTransform>();
    let screen = Vec2::new(renderer.config.width as f32, renderer.config.height as f32);

    // Split the borrow by hand: queueing needs `&mut` on the text renderer and `&` on the queue,
    // and both live on `Renderer`.
    let Renderer { text, queue, device, scene, post, depth_texture_view, .. } = renderer;
    let Some(text) = text.as_mut() else {
        return;
    };

    text.begin_frame();
    for (id, item) in texts.iter() {
        // `TextSpace::needs_world_position` rather than a `match` here: the enum is
        // `#[non_exhaustive]`, so this file could only match it with a wildcard, and a wildcard is
        // where a third space would quietly become a HUD element. The decision lives beside the
        // enum, where it is a compile error instead.
        let origin = if item.space.needs_world_position() {
            let Some(t) = transforms.get(id) else {
                // A world label with no world position. Skipping is the honest answer: at the
                // origin it would sit somewhere plausible and wrong, which is harder to diagnose
                // than not being there.
                continue;
            };
            t.matrix.w_axis.truncate()
        } else {
            Vec3::ZERO
        };
        text.queue(queue, item, origin, screen);
    }
    text.upload(device, queue);
    if text.queued() == (0, 0) {
        return;
    }

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Text Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &post.hdr_texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        // Loaded, not cleared: the world pipeline tests against the scene's own depth, which is
        // what puts a label behind the wall in front of it.
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_texture_view,
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
    text.render(&mut pass, scene.view_bind_group());
}
