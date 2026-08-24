//! The world-reading half of text, shared by both draw paths — and the bridge from `gizmo-ui`.
//!
//! # Why it is in the facade
//!
//! The same reason as [`super::decals::collect_decals`]: it needs both halves and they live in
//! different crates. [`TextRenderer`](gizmo_renderer::text::TextRenderer) and
//! [`Text`](gizmo_renderer::components::Text) are in `gizmo-renderer`, and `GlobalTransform` —
//! which is where a world label *is* — is in `gizmo-physics-core`, which the renderer does not
//! depend on. This is the lowest crate that can see both.
//!
//! `gizmo-ui` makes that sharper rather than looser. It sits **above** `gizmo-app`, so the renderer
//! cannot see a `Node` and never will; the facade is the only place that sees a laid-out UI box and
//! a glyph atlas at once. So the bridge is here, and it is small: `Node` publishes absolute
//! window-pixel rects and `TextSpace::Screen` takes absolute window pixels, so connecting them is
//! an anchor and a colour rather than a layout engine.
//!
//! Being `pub` is the point. `gizmo-studio` calls this rather than keeping a second copy, so the
//! two paths cannot drift about where a label sits — which is exactly the divergence
//! `gizmo-studio/tests/render_parity.rs` exists to catch, and exactly what happened to decals
//! before that test was written.

use gizmo_math::{Vec2, Vec3, Vec4};

use crate::core::World;
use crate::renderer::components::Text;
use crate::renderer::Renderer;

/// One laid-out UI box: `(entity, top-left, size, background)`.
///
/// Deliberately carries no `gizmo-ui` type. Everything past [`ui_boxes`] is the same code whether
/// the `ui` feature is on or off, which is what keeps the feature from forking the draw loop.
type UiBox = (u32, Vec2, Vec2, Option<Vec4>);

/// Every entity `gizmo-ui` has laid out this frame.
///
/// Empty without the feature — and empty is the honest answer rather than a `cfg` around the loop
/// below: a build with no UI crate has no UI boxes, which is the same thing a build with an empty
/// UI tree has.
#[cfg(feature = "ui")]
fn ui_boxes(world: &World) -> Vec<UiBox> {
    let nodes = world.borrow::<crate::ui::Node>();
    let backgrounds = world.borrow::<crate::ui::BackgroundColor>();
    nodes
        .iter()
        .map(|(id, node)| (id, node.position, node.size, backgrounds.get(id).map(|c| c.0)))
        .collect()
}

#[cfg(not(feature = "ui"))]
fn ui_boxes(_world: &World) -> Vec<UiBox> {
    Vec::new()
}

/// Lays this frame's [`Text`] out, rasterises any glyph the atlas has not seen, and draws it —
/// with the UI backgrounds underneath.
///
/// Runs **after** TAA, into the HDR target and against the main depth buffer — the same slot the
/// debug gizmos use, and for the same reason: a temporal resolve smears an overlay that changes
/// every frame, and text is the most legible thing in a frame and so the most obviously smeared.
///
/// World labels are depth-tested (a label behind a wall is behind the wall); screen text and UI
/// backgrounds are not. A `Text` with no `GlobalTransform` still draws if it is screen-space — a
/// HUD element has no place in the world — and is skipped if it is not.
///
/// # What a `Node` on the same entity does
///
/// It **positions the text**: a screen-space `Text` beside a `gizmo-ui` [`Node`](crate::ui::Node)
/// is placed at the box's own corner, chosen by the text's anchor — `TopLeft` at the box's
/// top-left, `Center` in the middle of it — and the position the component itself carries is
/// ignored. That is the only thing putting both on one entity can sensibly mean, and it is what
/// makes a laid-out button able to have a label. A `Node` does **not** clip the text and does not
/// resize it: a string longer than its box overflows it, because `gizmo-ui` has no clipping.
///
/// [`BackgroundColor`](crate::ui::BackgroundColor) is drawn as a solid quad over the box, under
/// every glyph in the frame. It had been written and read by nothing since the crate existed.
/// Overlapping boxes are painted in an arbitrary order, because `gizmo-ui` has no z-order to sort
/// by — its own docs say so, and this pass cannot invent one.
pub fn record_text(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    world: &World,
) {
    let texts = world.borrow::<Text>();
    let boxes = ui_boxes(world);
    if texts.iter().next().is_none() && boxes.iter().all(|(_, _, _, bg)| bg.is_none()) {
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
    // Backgrounds first in the source too, though the renderer keeps them under the glyphs
    // whatever order they arrive in — `TextRenderer::queue_rect` says so.
    for (_, position, size, background) in &boxes {
        if let Some(color) = background {
            text.queue_rect(*position, *size, *color, screen);
        }
    }

    for (id, item) in texts.iter() {
        // `TextSpace::needs_world_position` rather than a `match` here: the enum is
        // `#[non_exhaustive]`, so this file could only match it with a wildcard, and a wildcard is
        // where a third space would quietly become a HUD element. The decision lives beside the
        // enum, where it is a compile error instead.
        let (origin, anchor) = if item.space.needs_world_position() {
            let Some(t) = transforms.get(id) else {
                // A world label with no world position. Skipping is the honest answer: at the
                // origin it would sit somewhere plausible and wrong, which is harder to diagnose
                // than not being there.
                continue;
            };
            (t.matrix.w_axis.truncate(), None)
        } else {
            // A UI box on the same entity replaces the authored position. `factors()` is what
            // makes the text's own anchor pick the box's corner, so `Center` centres it in the
            // button rather than centring it on the button's top-left.
            let anchor = boxes
                .iter()
                .find(|(e, ..)| *e == id)
                .map(|(_, position, size, _)| *position + *size * item.anchor.factors());
            (Vec3::ZERO, anchor)
        };
        text.queue(queue, item, origin, anchor, screen);
    }
    text.upload(device, queue);
    if text.queued() == (0, 0, 0) {
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
