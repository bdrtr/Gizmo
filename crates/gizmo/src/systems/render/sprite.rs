//! The world-reading half of sprites, shared by both draw paths.
//!
//! In the facade for the reason [`super::text`] is: [`Sprite`] and the
//! [`SpriteRenderer`](gizmo_renderer::sprite::SpriteRenderer) live in `gizmo-renderer`, and
//! `GlobalTransform` — where a world sprite *is* — lives in `gizmo-physics-core`. Being `pub` is
//! the point: `gizmo-studio` calls this rather than keeping a copy, so the editor viewport and the
//! game cannot disagree about where a sprite sits or which one is in front.

use gizmo_math::{Mat4, Vec2};

use crate::core::World;
use crate::renderer::components::{Sprite, Tilemap};
use crate::renderer::Renderer;

/// Places, sorts and draws this frame's [`Sprite`]s.
///
/// `view_proj` must be the **unjittered** view-projection: sprites are drawn after TAA, so the
/// jittered matrix the scene uniforms carry would make them shimmer — see `gizmo_renderer::sprite`.
///
/// World sprites are drawn here, after TAA and before [`record_text`](super::record_text), into
/// the HDR target against the main depth buffer: a sprite behind a wall is hidden by it. Screen
/// sprites are only queued here; `Renderer::run_post_processing` draws them onto the final image
/// after tone mapping, under screen text, so a HUD keeps its authored colours.
///
/// [`Tilemap`]s are queued here too, one world sprite per tile, so a map sorts on its layer and
/// batches into one draw call per run like any other sprites of one image.
///
/// An entity hidden by `IsHidden` — on itself or on an ancestor — is skipped, exactly as a mesh
/// is. A world sprite with no `GlobalTransform` is skipped too: at the origin it would look placed
/// and be wrong.
pub fn record_sprites(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    world: &World,
    view_proj: Mat4,
) {
    let sprites = world.borrow::<Sprite>();
    let tilemaps = world.borrow::<Tilemap>();
    if sprites.iter().next().is_none() && tilemaps.iter().next().is_none() {
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
    // A tilemap is a run of world sprites sharing one image: queued here, it is sorted and
    // batched with them, so a whole map is one draw call on both draw paths.
    for (id, map) in tilemaps.iter() {
        if hidden.contains(&id) {
            continue;
        }
        let Some(t) = transforms.get(id) else { continue };
        for (offset, tile) in map.tile_sprites() {
            let model = t.matrix * Mat4::from_translation(offset.extend(0.0));
            state.queue(&tile, model, view_proj, screen);
        }
    }
    state.upload(device, queue);
    // Screen sprites are drawn later, onto the final image, by the overlay in
    // `Renderer::run_post_processing`. This pass is only for world sprites.
    if state.queued().0 == 0 {
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
