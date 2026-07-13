//! Draw-item batching + the per-frame render cache — extracted from `default_render_pass`.
//!
//! `collect_draw_items` walks the world's meshes (both the direct component form and the
//! asset-handle form), frustum-culls against the camera and the shadow cascades, groups the
//! survivors into instanced batches keyed by (mesh, material, skeleton, routing flags),
//! uploads the instance buffer, and returns the `DrawItem` list plus how many instances
//! actually fit the GPU buffer. Pure move out of mod.rs — no behaviour change.

use super::*;

#[derive(Default)]
pub struct RenderCache {
    pub(crate) batches: std::collections::HashMap<BatchKey, BatchData>,
    pub instances: Vec<crate::renderer::gpu_types::InstanceRaw>,
    pub draw_items: Vec<DrawItem>,
}

thread_local! {
    static RENDER_CACHE: std::cell::RefCell<RenderCache> = std::cell::RefCell::new(RenderCache::default());
}

pub fn clear_render_cache() {
    RENDER_CACHE.with(|rc| {
        let mut cache = rc.borrow_mut();
        cache.batches.clear();
        cache.instances.clear();
        cache.draw_items.clear();
    });
}

#[derive(Debug, Clone)]
pub struct DrawItem {
    // Fields are `pub(super)` (= visible across the whole `render` module tree) so the sibling
    // `passes/` recorders can still read them now that DrawItem lives here, not in mod.rs.
    pub(super) vbuf: std::sync::Arc<wgpu::Buffer>,
    pub(super) vertex_count: u32,
    pub(super) bind_group: std::sync::Arc<wgpu::BindGroup>,
    pub(super) unlit: bool,
    pub(super) is_skybox: bool,
    pub(super) skeleton_bind_group: Option<std::sync::Arc<wgpu::BindGroup>>,
    pub(super) is_transparent: bool,
    pub(super) first_instance: u32,
    /// Total instances in this batch's contiguous range: camera-visible ones FIRST,
    /// then shadow-only casters (outside the camera frustum but inside a cascade's light
    /// frustum). Shadow passes draw the whole range; main passes draw only `camera_count`.
    /// (Yalnız shadow geçitleri okur — web'de gölge yok, alan orada ölü.)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) instance_count: u32,
    /// Number of leading instances visible to the CAMERA (== the old camera-culled set).
    pub(super) camera_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct BatchKey {
    vbuf_id: usize,
    mat_id: usize,
    skeleton_id: Option<usize>,
    // Pass-routing flags MUST be part of the key. `mat_id` is the material's
    // *texture* bind-group pointer, which the asset manager caches and shares
    // across distinct materials (e.g. the default white texture, or the same
    // file). Two materials that differ only in transparency / material type would
    // otherwise collide into one batch, and the batch would inherit whichever
    // entity the (unordered) ECS iteration hit first — so a transparent object
    // could render opaque, or a PBR object route through the unlit path, and
    // *which* one corrupts flips between frames. Keying on the routing flags keeps
    // same-routing instances batched while separating ones that render differently.
    is_transparent: bool,
    unlit: bool,
    is_skybox: bool,
}

pub(crate) struct BatchData {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    vertex_count: u32,
    unlit: bool,
    is_skybox: bool,
    skeleton_bind_group: Option<std::sync::Arc<wgpu::BindGroup>>,
    is_transparent: bool,
    instances: Vec<crate::renderer::gpu_types::InstanceRaw>,
    /// Casters outside the camera frustum but inside a shadow cascade's light frustum —
    /// must be drawn into the shadow maps so off-screen objects still cast visible shadows.
    shadow_instances: Vec<crate::renderer::gpu_types::InstanceRaw>,
}

/// Collect visible + shadow-casting meshes into instanced draw batches for one frame.
///
/// The caller passes the UNJITTERED view-proj and the cascade view-projs so culling uses
/// clean (non-TAA-jittered) frusta — camera-visible instances feed the main passes, off-screen
/// casters inside a cascade's light frustum feed the shadow maps only. Returns the draw list and
/// the number of instances that actually fit `instance_capacity` (draw ranges clamp to it).
pub(super) fn collect_draw_items(
    world: &World,
    renderer: &Renderer,
    unjittered_view_proj: Mat4,
    cascade_vp: [Mat4; 4],
    cam_pos: Vec3,
) -> (Vec<DrawItem>, u32) {
    let renderers = world.borrow::<MeshRenderer>();

    let frustum = crate::math::Frustum::from_matrix(&unjittered_view_proj);
    // Per-cascade LIGHT frusta — shadow casters are culled against these, NOT the camera
    // frustum, so objects outside the view that cast shadows INTO it aren't dropped.
    let cascade_frusta: [crate::math::Frustum; 4] =
        cascade_vp.map(|m| crate::math::Frustum::from_matrix(&m));

    RENDER_CACHE.with(|rc| {
        let mut cache = rc.borrow_mut();
        
        // Clear instances but keep allocations.
        // `shadow_instances` MUST be cleared too: it is appended to every frame for
        // off-screen shadow casters (line ~444) but the batches HashMap persists across
        // frames, so leaving it uncleared made it grow without bound. Once the total
        // instance count crossed `instance_capacity` (8192) the buffer upload truncated
        // the tail, so batches past the cap silently stopped drawing — meshes vanished
        // one by one as more frames accumulated ("araç giderek kayboluyor"). Which mesh
        // dropped first depended on nondeterministic HashMap batch order.
        for batch in cache.batches.values_mut() {
            batch.instances.clear();
            batch.shadow_instances.clear();
        }
        cache.instances.clear();
        cache.draw_items.clear();

        let pooled_storage = world.borrow::<gizmo_core::pool::Pooled>();
        
        macro_rules! process_mesh {
            ($e:expr, $mesh:expr, $trans:expr, $mat:expr, $skeleton:expr) => {
                if renderers.get($e).is_none() {
                    continue;
                }
                
                // Pooled (havuzda pasif) nesneleri render etme
                if pooled_storage.get($e).is_some() {
                    continue;
                }

                let center_mat = Mat4::from_translation($mesh.center_offset);
                let model = $trans.matrix * center_mat;

                // CPU Frustum Culling
                let local_cx = ($mesh.bounds.min.x + $mesh.bounds.max.x) * 0.5;
                let local_cy = ($mesh.bounds.min.y + $mesh.bounds.max.y) * 0.5;
                let local_cz = ($mesh.bounds.min.z + $mesh.bounds.max.z) * 0.5;
                let world_c = model.transform_point3(Vec3::new(local_cx, local_cy, local_cz));
                let hx = ($mesh.bounds.max.x - $mesh.bounds.min.x) * 0.5;
                let hy = ($mesh.bounds.max.y - $mesh.bounds.min.y) * 0.5;
                let hz = ($mesh.bounds.max.z - $mesh.bounds.min.z) * 0.5;
                let local_r = (hx * hx + hy * hy + hz * hz).sqrt();
                let sx = model.x_axis.truncate().length();
                let sy = model.y_axis.truncate().length();
                let sz = model.z_axis.truncate().length();
                let world_r = local_r * sx.max(sy).max(sz);

                // Camera-visible → main passes; an off-screen shadow caster inside a
                // cascade's light frustum → shadow maps only (main passes use
                // `camera_count`, shadow passes the full range); otherwise skip. Shared
                // with the studio path so the cull test + caster predicate can't drift —
                // now the tighter AABB test (was a bounding sphere here).
                let camera_visible = match crate::renderer::classify_visibility(
                    &frustum,
                    &cascade_frusta,
                    &model,
                    $mesh.bounds,
                    $mat.material_type,
                    $mat.is_transparent,
                    $mat.albedo.w,
                ) {
                    crate::renderer::Visibility::Culled => continue,
                    crate::renderer::Visibility::Camera => true,
                    crate::renderer::Visibility::ShadowOnly => false,
                };

                // Auto-LOD (Level of Detail) Seçimi
                let dist_to_cam = (world_c - cam_pos).length();
                let use_lod1 = if !$mesh.lod_vbufs.is_empty() {
                    dist_to_cam > world_r * 15.0 // Nesne boyutuna göre uzaklaştıkça LOD1'e geç (örneğin 2m çapında bir nesne 30m uzaktayken geç)
                } else {
                    false
                };

                let active_vbuf = if use_lod1 {
                    $mesh.lod_vbufs[0].clone()
                } else {
                    $mesh.vbuf.clone()
                };
                let active_vertex_count = if use_lod1 {
                    $mesh.lod_vertex_counts[0]
                } else {
                    $mesh.vertex_count
                };

                let packed_params = (($mat.anisotropy * 1000.0).floor() + 1000.0 * ($mat.clear_coat * 1000.0).floor() + 1000000.0 * ($mat.subsurface * 100.0).floor()) as f32;

                let instance_data = crate::renderer::gpu_types::InstanceRaw {
                    model: model.to_cols_array_2d(),
                    albedo_color: [$mat.albedo.x, $mat.albedo.y, $mat.albedo.z, $mat.albedo.w],
                    roughness: $mat.roughness,
                    metallic: $mat.metallic,
                    unlit: match $mat.material_type {
                        crate::renderer::components::MaterialType::Skybox => 2.0,
                        crate::renderer::components::MaterialType::Unlit => 1.0,
                        _ => 0.0,
                    },
                    _padding: packed_params,
                };
                let skel_bg = $skeleton.map(|s: &crate::renderer::components::Skeleton| s.bind_group.clone());

                // Compute the pass-routing flags up front so they can be part of the
                // batch key (see BatchKey docs) — not just read from the first material.
                let is_skybox = $mat.material_type == crate::renderer::components::MaterialType::Skybox;
                let unlit = is_skybox
                    || $mat.material_type == crate::renderer::components::MaterialType::Unlit;
                let is_transparent = $mat.is_transparent || $mat.albedo.w < 0.99;

                let key = BatchKey {
                    vbuf_id: std::sync::Arc::as_ptr(&active_vbuf) as usize,
                    mat_id: std::sync::Arc::as_ptr(&$mat.bind_group) as usize,
                    skeleton_id: skel_bg.as_ref().map(|bg| std::sync::Arc::as_ptr(bg) as usize),
                    is_transparent,
                    unlit,
                    is_skybox,
                };

                let batch = cache.batches.entry(key).or_insert_with(|| BatchData {
                    vbuf: active_vbuf.clone(),
                    bind_group: $mat.bind_group.clone(),
                    vertex_count: active_vertex_count,
                    unlit,
                    is_skybox,
                    skeleton_bind_group: skel_bg,
                    is_transparent,
                    instances: Vec::new(),
                    shadow_instances: Vec::new(),
                });
                if camera_visible {
                    batch.instances.push(instance_data);
                } else {
                    // Off-screen caster kept above for shadow maps only.
                    batch.shadow_instances.push(instance_data);
                }
            };
        }

        let skeletons = world.borrow::<crate::renderer::components::Skeleton>();

        if let Some(mut q) = world.query::<(&Mesh, &gizmo_physics_core::components::GlobalTransform, &Material)>() {
            for (e, (mesh, trans, mat)) in q.iter_mut() {
                process_mesh!(e, mesh, trans, mat, skeletons.get(e));
            }
        }
        
        let meshes = world.try_get_resource::<gizmo_core::asset::Assets<Mesh>>().ok();
        let materials = world.try_get_resource::<gizmo_core::asset::Assets<Material>>().ok();
        
        if let (Some(meshes), Some(materials)) = (meshes, materials) {
            if let Some(mut q) = world.query::<(&gizmo_core::asset::Handle<Mesh>, &gizmo_physics_core::components::GlobalTransform, &gizmo_core::asset::Handle<Material>)>() {
                for (e, (h_mesh, trans, h_mat)) in q.iter_mut() {
                    if let (Some(mesh), Some(mat)) = (meshes.get(h_mesh), materials.get(h_mat)) {
                        process_mesh!(e, mesh, trans, mat, skeletons.get(e));
                    }
                }
            }
        }
        
        let mut local_instances: Vec<crate::renderer::gpu_types::InstanceRaw> = std::mem::take(&mut cache.instances);
        let mut local_draw_items: Vec<DrawItem> = std::mem::take(&mut cache.draw_items);

        for batch in cache.batches.values() {
            if batch.instances.is_empty() && batch.shadow_instances.is_empty() {
                continue;
            }
            let first_instance = local_instances.len() as u32;
            // Camera-visible instances FIRST (so `camera_count` == the old culled set),
            // then shadow-only casters — both contiguous under one DrawItem range.
            let camera_count = batch.instances.len() as u32;
            local_instances.extend(&batch.instances);
            local_instances.extend(&batch.shadow_instances);
            let instance_count = camera_count + batch.shadow_instances.len() as u32;

            local_draw_items.push(DrawItem {
                vbuf: batch.vbuf.clone(),
                vertex_count: batch.vertex_count,
                bind_group: batch.bind_group.clone(),
                unlit: batch.unlit,
                is_skybox: batch.is_skybox,
                skeleton_bind_group: batch.skeleton_bind_group.clone(),
                is_transparent: batch.is_transparent,
                first_instance,
                instance_count,
                camera_count,
            });
        }
        
        cache.instances = local_instances;
        cache.draw_items = local_draw_items;

        // Instance limiti kontrolü (Taşmaları önlemek için capaciteyi zorla)
        let max_instances = renderer.scene.instance_capacity;
        let instances_slice = if cache.instances.len() > max_instances {
            &cache.instances[..max_instances]
        } else {
            &cache.instances
        };

        if !instances_slice.is_empty() {
            renderer.queue.write_buffer(
                &renderer.scene.instance_buffer,
                0,
                bytemuck::cast_slice(instances_slice),
            );
        }
        
        // Pass draw_items to rendering logic by cloning the small struct (Arc clones are cheap).
        // Also return how many instances actually made it into the GPU buffer so draw ranges
        // can be clamped (shadow casters increase the count → guard against capacity truncation).
        (cache.draw_items.clone(), instances_slice.len() as u32)
    })
}

#[cfg(test)]
mod batch_key_tests {
    use super::BatchKey;

    // Regression: two materials that share a cached texture bind group (same
    // `mat_id`) and mesh (same `vbuf_id`) but route differently must NOT collide
    // into one batch — otherwise the batch inherits the first-iterated material's
    // transparency / lighting classification (a transparent object rendering
    // opaque, or a PBR object routed through the unlit path). The routing flags
    // are part of the key precisely to keep these apart while still batching
    // identical materials together.
    #[test]
    fn routing_flags_distinguish_batches_sharing_a_texture() {
        let base = BatchKey {
            vbuf_id: 1,
            mat_id: 42, // same cached texture bind group as the variants below
            skeleton_id: None,
            is_transparent: false,
            unlit: false,
            is_skybox: false,
        };
        let transparent = BatchKey {
            is_transparent: true,
            ..base.clone()
        };
        let unlit = BatchKey {
            unlit: true,
            ..base.clone()
        };
        let skybox = BatchKey {
            is_skybox: true,
            ..base.clone()
        };

        assert_ne!(base, transparent, "opaque and transparent must be separate batches");
        assert_ne!(base, unlit, "PBR and unlit must be separate batches");
        assert_ne!(base, skybox, "PBR and skybox must be separate batches");

        // Identical routing + shared texture/mesh → same batch (instancing preserved).
        assert_eq!(base, base.clone(), "identical materials must still batch together");
    }
}
