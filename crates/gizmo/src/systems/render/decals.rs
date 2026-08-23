//! Decals for a pipeline that has no G-buffer — and the world-reading half both pipelines share.
//!
//! # Why this file exists
//!
//! A decal is a projector: it paints the surface that is already there, so it needs that
//! surface's position. The engine's deferred pass reads it out of the G-buffer, which works for a
//! game and not at all for the editor — `gizmo-studio` draws forward and never fills a G-buffer,
//! so a decal placed in the editor was invisible until the game ran. The test that recorded that
//! (`gizmo-studio/tests/render_parity.rs`) called it "real and unfixed". This is the fix.
//!
//! [`record_forward_decals`] reconstructs the same position from the **depth** buffer, which the
//! forward pass does write, and alpha-blends into the lit HDR image. The projection maths, the
//! volume test and the fade are the deferred shader's, deliberately — see `decal_forward.wgsl`.
//!
//! # Why it is in the facade
//!
//! Same reason as [`super::particles::spawn_from_emitters`]: it needs both halves and they live
//! in different crates. `DecalState` is in `gizmo-renderer`, and `Transform` — which is where a
//! decal *is* — is in `gizmo-physics-core`, which the renderer does not depend on. This is the
//! lowest crate that can see both, and being `pub` is the point: the studio calls it rather than
//! keeping a second copy, so the two paths cannot drift apart about where a decal sits.

use crate::core::World;
use crate::renderer::decal::DecalUniforms;
use crate::renderer::Renderer;
use gizmo_math::{Mat4, Vec3};
use std::sync::Arc;

/// The uniform arena is 1024 slots of 256 bytes; beyond that a decal has nowhere to be written.
const MAX_DECALS: usize = 1024;

/// Collect what a decal pass needs from the world: one uniform block and one texture bind group
/// per decal, in draw order.
///
/// Shared by both passes, because the thing they must agree about is *where the decal is*. The
/// matrix fold below is the subtle half of that agreement: both shaders hand this matrix a
/// **camera-relative** position — the deferred one because that is what the G-buffer stores (see
/// `gbuffer.wgsl`), the forward one because it subtracts the camera from what it reconstructs —
/// so folding `T(camera)` in here is what lets one uniform serve both without either shader
/// needing a uniform it does not otherwise have.
pub fn collect_decals(
    world: &World,
    cam_pos: Vec3,
) -> (Vec<DecalUniforms>, Vec<Arc<wgpu::BindGroup>>) {
    let decals = world.borrow::<crate::renderer::components::Decal>();
    let transforms = world.borrow::<gizmo_physics_core::Transform>();

    let mut uniforms = Vec::new();
    let mut bind_groups = Vec::new();
    for (id, decal) in decals.iter() {
        let Some(trans) = transforms.get(id) else {
            continue;
        };
        let model = trans.local_matrix;
        let inv_model = model.inverse() * Mat4::from_translation(cam_pos);

        uniforms.push(DecalUniforms {
            inv_model: inv_model.to_cols_array(),
            model: model.to_cols_array(),
            albedo_color: [decal.color.x, decal.color.y, decal.color.z, decal.color.w],
            _pad: [0.0; 28],
        });
        bind_groups.push(decal.bind_group.clone());
        if uniforms.len() >= MAX_DECALS {
            break;
        }
    }
    (uniforms, bind_groups)
}

/// Draw every decal in `world` onto the lit HDR image, reading the depth buffer for the surface.
///
/// For a forward pipeline: it needs the depth buffer to be **already written** by the frame's
/// geometry pass, and it must run before post-processing reads the HDR texture. Nothing is
/// recorded when the scene has no decals — an empty pass would still cost an encoder section.
pub fn record_forward_decals(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    world: &World,
    cam_pos: Vec3,
) {
    let Some(decal_state) = renderer.decal.as_ref().filter(|d| d.enabled) else {
        return;
    };
    let (uniforms, bind_groups) = collect_decals(world, cam_pos);
    if uniforms.is_empty() {
        return;
    }

    renderer.queue.write_buffer(
        &decal_state.uniform_buffer,
        0,
        bytemuck::cast_slice(&uniforms),
    );

    // The depth view is a new object after every resize, so this is built per frame — the same
    // reason the particle pass builds its own.
    let depth_bg =
        decal_state.create_depth_bind_group(&renderer.device, &renderer.depth_texture_view);

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Forward Decal Pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &renderer.post.hdr_texture_view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        // No depth attachment on purpose: the depth texture is *sampled* here, and wgpu will not
        // let one texture be both in a single pass.
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(&decal_state.forward_pipeline);
    pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
    pass.set_bind_group(1, &depth_bg, &[]);
    pass.set_vertex_buffer(0, decal_state.vertex_buffer.slice(..));
    for (i, bind_group) in bind_groups.iter().enumerate() {
        pass.set_bind_group(2, bind_group.as_ref(), &[]);
        pass.set_bind_group(3, &decal_state.decal_uniform_bg, &[(i * 256) as u32]);
        pass.draw(0..36, 0..1);
    }
}
