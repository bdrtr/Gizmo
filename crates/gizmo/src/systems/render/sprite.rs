//! The world-reading half of sprites, shared by both draw paths.
//!
//! In the facade for the reason [`super::text`] is: [`Sprite`] and the
//! [`SpriteRenderer`](gizmo_renderer::sprite::SpriteRenderer) live in `gizmo-renderer`, and
//! `GlobalTransform` — where a world sprite *is* — lives in `gizmo-physics-core`. Being `pub` is
//! the point: `gizmo-studio` calls this rather than keeping a copy, so the editor viewport and the
//! game cannot disagree about where a sprite sits or which one is in front.

use gizmo_math::{Mat4, Vec2};

use crate::core::World;
use crate::renderer::components::Sprite;
use crate::renderer::Renderer;

/// Places, sorts and draws this frame's [`Sprite`]s.
///
/// `view_proj` must be the **unjittered** view-projection: sprites are drawn after TAA, so the
/// jittered matrix the scene uniforms carry would make them shimmer — see `gizmo_renderer::sprite`.
///
/// Runs after TAA and **before** [`record_text`](super::record_text), into the HDR target against
/// the main depth buffer: a world sprite behind a wall is hidden by it, and a HUD label drawn over
/// a HUD image reads on top of it. An entity hidden by `IsHidden` — on itself or on an ancestor —
/// is skipped, exactly as a mesh is. A world sprite with no `GlobalTransform` is skipped too: at
/// the origin it would look placed and be wrong.
pub fn record_sprites(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    world: &World,
    view_proj: Mat4,
) {
    let sprites = world.borrow::<Sprite>();
    if sprites.iter().next().is_none() {
        if let Some(s) = renderer.sprites.as_mut() {
            // Last frame's runs must not survive into a frame that has none.
            s.begin_frame();
        }
        return;
    }
    let hidden = super::hidden::collect_hidden(
        &world.borrow::<gizmo_core::component::IsHidden>(),
        &world.borrow::<gizmo_core::component::Children>(),
    );
    let transforms = world.borrow::<crate::prelude::GlobalTransform>();
    let screen = Vec2::new(renderer.config.width as f32, renderer.config.height as f32);

    let Renderer { sprites: pass_state, queue, device, post, depth_texture_view, .. } = renderer;
    let Some(state) = pass_state.as_mut() else {
        return;
    };

    state.begin_frame();
    for (id, sprite) in sprites.iter() {
        if hidden.contains(&id) {
            continue;
        }
        let model = if sprite.space.needs_world_transform() {
            let Some(t) = transforms.get(id) else { continue };
            t.matrix
        } else {
            Mat4::IDENTITY
        };
        state.queue(sprite, model, view_proj, screen);
    }
    state.upload(device, queue);
    if state.queued() == (0, 0) {
        return;
    }

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Sprite Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &post.hdr_texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth_texture_view,
            depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    state.render(&mut pass);
}
