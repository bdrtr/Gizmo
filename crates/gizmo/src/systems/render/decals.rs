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
//! phone's) projects its decals in its lit shader instead, over the surface before it is lit
//! ([`prepare_forward_decals`]).
//!
//! A decal is where its `GlobalTransform` puts it. It was where its `Transform` alone did until
//! 2026-11-23, so a decal parented to something that had moved — a sticker on a car — was drawn
//! where the car started.

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
    decals_in(world)
        .into_iter()
        .map(|(model, color, bind_group)| {
            let inv_model = model.inverse() * Mat4::from_translation(cam_pos);
            let uniforms = DecalUniforms {
                inv_model: inv_model.to_cols_array(),
                model: model.to_cols_array(),
                albedo_color: color.to_array(),
                _pad: [0.0; 28],
            };
            (uniforms, bind_group)
        })
        .unzip()
}

/// Every decal the world has, in draw order and at most [`MAX_DECALS`]: the matrix its box's unit
/// cube is mapped into the world by — its `GlobalTransform`'s, or its `Transform`'s where it has no
/// `GlobalTransform` — its tint, and its bind group.
fn decals_in(world: &World) -> Vec<(Mat4, gizmo_math::Vec4, Arc<wgpu::BindGroup>)> {
    let transforms = world.query::<&gizmo_physics_core::Transform>();
    let globals = world.query::<&gizmo_physics_core::components::GlobalTransform>();
    world
        .query::<(Entity, &crate::renderer::components::Decal)>()
        .iter()
        .filter_map(|(entity, decal)| {
            let model = match globals.get(entity) {
                Some(global) => global.matrix,
                None => transforms.get(entity)?.local_matrix,
            };
            Some((model, decal.color, decal.bind_group.clone()))
        })
        .take(MAX_DECALS)
        .collect()
}

/// The forward-only path's decals for this frame: each decal's texture placed in a layer of the
/// decal array, copied in the first frame it is shown (`ForwardDecals::place`), the decals written
/// where the lit shader reads them, the spheres round their boxes left for every view's clusters
/// (`SceneState::forward_decal_spheres`), and the lit pipelines built again when a frame first has a
/// decal or last had one (`SceneState::set_forward_decals`) — a scene with none pays nothing for them.
///
/// A decal's texture is its bind group's base colour, which the asset manager made it from
/// (`AssetManager::base_colour_view_of`); a decal over the white material is its tint. One whose
/// bind group the asset manager did not make has no texture this path can find, and is its tint,
/// said once; one past the array's [`DECAL_LAYERS`](crate::renderer::decal::DECAL_LAYERS)
/// different textures in a frame is not drawn, said once.
pub fn prepare_forward_decals(world: &World, renderer: &mut crate::renderer::Renderer) {
    use crate::renderer::decal::{GpuDecal, NO_DECAL_TEXTURE};
    use std::sync::atomic::{AtomicBool, Ordering};
    static UNKNOWN_SAID: AtomicBool = AtomicBool::new(false);
    static FULL_SAID: AtomicBool = AtomicBool::new(false);
    let decals = decals_in(world);
    if decals.is_empty() {
        renderer.scene.forward_decal_spheres.clear();
        if renderer.scene.set_forward_decals(false) {
            crate::renderer::pipeline::rebuild_pipelines(renderer);
        }
        return;
    }
    renderer.scene.ensure_forward_decals(&renderer.device);
    // Looked up once a bind group, not once a decal: a thousand tyre marks are one texture.
    let textures: Vec<Option<Arc<wgpu::TextureView>>> = {
        let assets = renderer.asset_manager.read().unwrap_or_else(|p| p.into_inner());
        let mut found: Vec<(&Arc<wgpu::BindGroup>, Option<Arc<wgpu::TextureView>>)> = Vec::new();
        decals
            .iter()
            .map(|(_, _, bind_group)| {
                if let Some((_, view)) = found.iter().find(|(seen, _)| Arc::ptr_eq(seen, bind_group)) {
                    return view.clone();
                }
                let view = assets.base_colour_view_of(bind_group);
                if view.is_none() && !assets.made(bind_group) && !UNKNOWN_SAID.swap(true, Ordering::Relaxed) {
                    tracing::warn!("[Decal] a decal's bind group was not made by the asset manager: the forward-only path cannot find its texture and draws its tint");
                }
                found.push((bind_group, view.clone()));
                view
            })
            .collect()
    };
    let mut distinct: Vec<Arc<wgpu::TextureView>> = Vec::new();
    for texture in textures.iter().flatten() {
        if !distinct.iter().any(|d| Arc::ptr_eq(d, texture)) {
            distinct.push(texture.clone());
        }
    }
    let layers = renderer.scene.forward_decals.place(&renderer.device, &renderer.queue, &distinct);
    let mut gpu = Vec::with_capacity(decals.len());
    let mut spheres = Vec::with_capacity(decals.len());
    for ((model, tint, _), texture) in decals.iter().zip(&textures) {
        let layer = match texture {
            None => Some(NO_DECAL_TEXTURE),
            Some(t) => distinct.iter().position(|d| Arc::ptr_eq(d, t)).and_then(|i| layers[i]),
        };
        let Some(layer) = layer else {
            if !FULL_SAID.swap(true, Ordering::Relaxed) {
                tracing::warn!(layers = crate::renderer::decal::DECAL_LAYERS, "[Decal] more different decal textures in a frame than the forward-only path's array holds; the rest are not drawn");
            }
            continue;
        };
        gpu.push(GpuDecal::new(*model, *tint, layer));
        spheres.push(crate::renderer::clustered::decal_sphere(*model));
    }
    renderer.scene.forward_decals.upload(&renderer.queue, &gpu);
    renderer.scene.forward_decal_spheres = spheres;
    if renderer.scene.set_forward_decals(!gpu.is_empty()) {
        crate::renderer::pipeline::rebuild_pipelines(renderer);
    }
}
