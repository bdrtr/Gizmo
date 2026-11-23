//! The world-reading half of the decal pass: where each decal is and what it paints.
//!
//! A decal is a projector box, drawn against the G-buffer's world positions by the deferred pass
//! (`passes::geometry`'s `decal_pass`); the editor's viewport draws through that same pass. It is
//! here, in the facade, because it needs both halves and they live in different crates:
//! `DecalState` is in `gizmo-renderer`, and `Transform` — which is where a decal *is* — is in
//! `gizmo-physics-core`, which the renderer does not depend on.
//!
//! Until 2026-11-23 this file also held `record_forward_decals`, a forward variant that read the
//! surface from the depth buffer and blended the decal, unlit, into the HDR image, for an editor
//! that drew forward. The editor draws through the engine's deferred path now, and nothing called
//! it; it went with its pipeline and `decal_forward.wgsl`. The forward-only path (the browser's, a
//! phone's) draws no decals — an unlit decal would glow in the dark, which is not what it needs
//! (`docs/ENGINE.md` §3).

use gizmo_core::Entity;
use crate::core::World;
use crate::renderer::decal::{DecalUniforms, MAX_DECALS};
use gizmo_math::{Mat4, Vec3};
use std::sync::Arc;

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
    let transforms = world.query::<&gizmo_physics_core::Transform>();

    let mut uniforms = Vec::new();
    let mut bind_groups = Vec::new();
    for (entity, decal) in world.query::<(Entity, &crate::renderer::components::Decal)>().iter() {
        let Some(trans) = transforms.get(entity) else {
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
