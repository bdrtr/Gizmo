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

/// anisotropy/clear_coat/subsurface'i tek bir f32'ye ondalık-basamak paketleme ile sıkıştırır
/// (InstanceRaw'da ayrı alan yok). gbuffer.wgsl fs_main'deki unpack ile EŞLEŞMELİDİR:
///   subsurface = floor(w/1e6)/100 · clear_coat = floor((w mod 1e6)/1e3)/1e3
///   anisotropy = (w mod 1e3)/1e3
/// anisotropy ve clear_coat 3-haneli alanlardır (0..999). `floor(1.0*1000)=1000` bir hane
/// fazladır ve komşu alana TAŞAR (yasal clamp'li `1.0` uçları için) → alanı .min(999.0) ile
/// sınırla; `1.0` artık `0.999` olarak okunur (fark edilmez) yerine komşuyu bozar.
/// (Uzun vadeli sağlam çözüm: ayrı InstanceRaw alanları — bu şema f32'de 2^24 üstünde
/// tamsayı hassasiyetini de kaybeder.)
pub(crate) fn pack_pbr_params(anisotropy: f32, clear_coat: f32, subsurface: f32) -> f32 {
    (anisotropy * 1000.0).floor().min(999.0)
        + 1000.0 * (clear_coat * 1000.0).floor().min(999.0)
        + 1_000_000.0 * (subsurface * 100.0).floor()
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
    /// Start of this batch's CAMERA-visible instances in region A of the instance buffer.
    pub(super) first_instance: u32,
    /// Number of camera-visible instances (== the old camera-culled set). Main/geometry
    /// passes draw `first_instance .. first_instance + camera_count`.
    pub(super) camera_count: u32,
    /// Start of this batch's SHADOW-ONLY casters in region B (all camera instances of all
    /// batches come first, then all shadow-only casters — see `collect_draw_items`). These
    /// are NOT contiguous with the camera range, so shadow passes draw them as a separate
    /// range. (Yalnız shadow geçitleri okur — web'de gölge yok, alanlar orada ölü.)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) shadow_first_instance: u32,
    /// Number of shadow-only casters (region B) for this batch.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) shadow_count: u32,
}

impl DrawItem {
    /// Camera-visible instance range (region A), clamped to what actually fit the GPU
    /// instance buffer (`uploaded`). `.max(start)` keeps the range non-reversed when this
    /// batch's region was entirely truncated (an empty range = a 0-instance no-op draw).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn camera_instance_range(&self, uploaded: u32) -> std::ops::Range<u32> {
        self.first_instance
            ..(self.first_instance + self.camera_count)
                .min(uploaded)
                .max(self.first_instance)
    }

    /// Shadow-only caster range (region B), clamped to what fit the GPU buffer. Because
    /// region B is appended AFTER every camera instance, capacity truncation drops these
    /// off-screen casters before it ever drops camera-visible geometry.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn shadow_instance_range(&self, uploaded: u32) -> std::ops::Range<u32> {
        self.shadow_first_instance
            ..(self.shadow_first_instance + self.shadow_count)
                .min(uploaded)
                .max(self.shadow_first_instance)
    }
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

                let packed_params = pack_pbr_params($mat.anisotropy, $mat.clear_coat, $mat.subsurface);

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

        // Two-region instance layout. Region A = EVERY batch's camera-visible instances;
        // region B (appended after A) = EVERY batch's shadow-only casters. The old layout
        // packed each batch as [camera][shadow] contiguously, so when the total exceeded
        // `instance_capacity` (8192) the tail truncation could drop a LATER batch's
        // camera-visible geometry because an EARLIER batch's shadow-only casters had already
        // eaten slots (and which mesh vanished flipped with nondeterministic HashMap order).
        // Splitting the regions means truncation drops off-screen shadow casters first and
        // never starves on-screen geometry. The two ranges are non-contiguous, so DrawItem
        // carries both (first_instance/camera_count and shadow_first_instance/shadow_count)
        // and the shadow pass draws them separately.
        let batches: Vec<&BatchData> = cache
            .batches
            .values()
            .filter(|b| !(b.instances.is_empty() && b.shadow_instances.is_empty()))
            .collect();

        // Region A — all camera-visible instances. One DrawItem per batch (shadow fields
        // filled in the region-B pass below; the batch list order is stable between passes).
        for batch in &batches {
            let first_instance = local_instances.len() as u32;
            let camera_count = batch.instances.len() as u32;
            local_instances.extend(&batch.instances);
            local_draw_items.push(DrawItem {
                vbuf: batch.vbuf.clone(),
                vertex_count: batch.vertex_count,
                bind_group: batch.bind_group.clone(),
                unlit: batch.unlit,
                is_skybox: batch.is_skybox,
                skeleton_bind_group: batch.skeleton_bind_group.clone(),
                is_transparent: batch.is_transparent,
                first_instance,
                camera_count,
                shadow_first_instance: 0,
                shadow_count: 0,
            });
        }

        // Region B — all shadow-only casters, after every camera instance. Backfill each
        // DrawItem's shadow range (draw items were pushed in the same batch order above).
        let draw_item_base = local_draw_items.len() - batches.len();
        for (i, batch) in batches.iter().enumerate() {
            let shadow_first_instance = local_instances.len() as u32;
            local_instances.extend(&batch.shadow_instances);
            let item = &mut local_draw_items[draw_item_base + i];
            item.shadow_first_instance = shadow_first_instance;
            item.shadow_count = batch.shadow_instances.len() as u32;
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

#[cfg(test)]
mod pbr_pack_tests {
    use super::pack_pbr_params;

    // Mirror gbuffer.wgsl fs_main's unpack of packed_params (in.inst_pbr.w) exactly.
    fn unpack(w: f32) -> (f32, f32, f32) {
        let subsurface = (w / 1_000_000.0).floor() / 100.0;
        let rem = w - (w / 1_000_000.0).floor() * 1_000_000.0;
        let clear_coat = (rem / 1000.0).floor() / 1000.0;
        let anisotropy = (rem - (rem / 1000.0).floor() * 1000.0) / 1000.0;
        (anisotropy, clear_coat, subsurface)
    }

    // Regression: the legal clamped endpoint 1.0 must NOT overflow its 3-digit field into
    // the neighbour. Before the .min(999.0) clamp, clear_coat=1.0 packed as floor(1000)*1000
    // which carried into the subsurface field → clear_coat read back as 0 and a phantom
    // subsurface≈0.01 appeared. Symmetric for anisotropy=1.0.
    #[test]
    fn endpoint_one_does_not_overflow_into_neighbours() {
        // clear_coat = 1.0, others 0 → clear_coat must survive (~0.999), no phantom subsurface.
        let (aniso, cc, ss) = unpack(pack_pbr_params(0.0, 1.0, 0.0));
        assert!(cc >= 0.99, "clear_coat=1.0 lost (got {cc})");
        assert_eq!(ss, 0.0, "clear_coat=1.0 leaked a phantom subsurface ({ss})");
        assert_eq!(aniso, 0.0, "clear_coat=1.0 leaked into anisotropy ({aniso})");

        // anisotropy = 1.0 → survives, no leak into clear_coat.
        let (aniso, cc, ss) = unpack(pack_pbr_params(1.0, 0.0, 0.0));
        assert!(aniso >= 0.99, "anisotropy=1.0 lost (got {aniso})");
        assert_eq!(cc, 0.0, "anisotropy=1.0 leaked into clear_coat ({cc})");
        assert_eq!(ss, 0.0, "anisotropy=1.0 leaked into subsurface ({ss})");
    }

    // Ordinary mid-range values round-trip within the decimal-packing resolution.
    #[test]
    fn mid_range_values_round_trip() {
        let (aniso, cc, ss) = unpack(pack_pbr_params(0.3, 0.7, 0.05));
        assert!((aniso - 0.3).abs() < 0.002, "aniso {aniso}");
        assert!((cc - 0.7).abs() < 0.002, "clear_coat {cc}");
        assert!((ss - 0.05).abs() < 0.02, "subsurface {ss}");
    }
}
