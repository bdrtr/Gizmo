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

/// Compresses anisotropy/clear_coat/subsurface into a single f32 with decimal-digit packing
/// (no separate field in InstanceRaw). It MUST MATCH the unpack in gbuffer.wgsl fs_main:
///   subsurface = floor(w/1e6)/100 · clear_coat = floor((w mod 1e6)/1e3)/1e3
///   anisotropy = (w mod 1e3)/1e3
/// anisotropy and clear_coat are 3-digit fields (0..999). `floor(1.0*1000)=1000` is one digit
/// too many and OVERFLOWS into the neighboring field (for the legal clamped `1.0` endpoints) →
/// bound the field with .min(999.0); `1.0` is now read as `0.999` (unnoticeable) instead of
/// corrupting the neighbor. (Long-term robust fix: separate InstanceRaw fields — this scheme
/// also loses integer precision above 2^24 in f32.)
/// Representative camera distance of an instanced batch: distance from `cam_pos` to the
/// centroid of the batch's instance world positions (the translation column of each
/// `InstanceRaw::model`). Used to order transparent batches back-to-front. Per-batch (not
/// per-instance) granularity — coarse for a batch spread across depth, but far better than
/// the arbitrary HashMap order it replaces, and exact for the common single-instance case.
pub(crate) fn batch_sort_depth(
    instances: &[crate::renderer::gpu_types::InstanceRaw],
    cam_pos: Vec3,
) -> f32 {
    if instances.is_empty() {
        return 0.0;
    }
    let mut centroid = Vec3::ZERO;
    for inst in instances {
        // InstanceRaw::model is column-major [[f32;4];4]; column 3 is the translation.
        centroid += Vec3::new(inst.model[3][0], inst.model[3][1], inst.model[3][2]);
    }
    centroid /= instances.len() as f32;
    (centroid - cam_pos).length()
}

/// Where a batch sits in the frame's paint order. Lower draws first; the `Ord` derive IS the
/// ordering, so the variants must stay in this sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DrawLayer {
    /// A painted backdrop — the scene's own sky/panorama geometry. First, before anything
    /// else in the frame.
    ///
    /// This is one of the three properties `MaterialType::Backdrop` has to hold (see
    /// `gizmo_renderer::backdrop`), and it is the only one the shader and the pipeline state
    /// cannot express, because neither of them knows what else is in the frame. It is not
    /// redundant with the far-plane depth pin: the pin keeps a backdrop from occluding OPAQUE
    /// geometry, which is depth-tested, but a transparent object writes no depth and blends
    /// with whatever is already in the target — so a backdrop drawn after it paints straight
    /// over it. Drawn first, the backdrop is underneath by construction.
    Backdrop,
    /// The solid world. Relative order is irrelevant — the depth buffer resolves it.
    Opaque,
    /// Blended geometry, sorted back-to-front among itself.
    Transparent,
}

/// Which layer a batch belongs to, from its routing flags.
///
/// `Backdrop` wins over `Transparent` deliberately. A backdrop material may well carry a
/// sub-1.0 albedo alpha (a haze pass over the skyline), and `is_transparent` is inferred from
/// that alpha — but a backdrop that sorted into the transparent bucket would be painted over
/// the world it is behind, which is the exact failure the layer exists to prevent.
pub(crate) fn draw_layer(is_backdrop: bool, is_transparent: bool) -> DrawLayer {
    if is_backdrop {
        DrawLayer::Backdrop
    } else if is_transparent {
        DrawLayer::Transparent
    } else {
        DrawLayer::Opaque
    }
}

/// Draw-order comparator for correct compositing. Backdrops come first, then the opaque world
/// (whose relative order is irrelevant — the depth buffer resolves it), then transparent
/// batches back-to-front (farthest first) because the transparent pipeline disables
/// depth-write, so ONLY draw order determines the blended result.
///
/// Backdrops are ALSO sorted back-to-front among themselves, for the same reason and one
/// more: every backdrop vertex is pinned to the same NDC depth, so the depth buffer cannot
/// separate two overlapping panels even in principle. Each arg is `(layer, sort_depth)`.
pub(crate) fn cmp_draw_order(a: (DrawLayer, f32), b: (DrawLayer, f32)) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match a.0.cmp(&b.0) {
        // Same layer: the opaque world needs no order, the two depth-write-disabled layers
        // are composited by paint order alone → farther one first (descending depth).
        Ordering::Equal => match a.0 {
            DrawLayer::Opaque => Ordering::Equal,
            DrawLayer::Backdrop | DrawLayer::Transparent => {
                b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal)
            }
        },
        different => different,
    }
}

#[derive(Debug, Clone)]
pub struct DrawItem {
    // Fields are `pub(super)` (= visible across the whole `render` module tree) so the sibling
    // `passes/` recorders can still read them now that DrawItem lives here, not in mod.rs.
    pub(super) vbuf: std::sync::Arc<wgpu::Buffer>,
    pub(super) vertex_count: u32,
    /// If `Some`, this batch is drawn with `draw_indexed`; if `None`, with a plain `draw`.
    /// Always `None` while a LOD level is active — see `collect_draw_items`.
    pub(super) ibuf: Option<std::sync::Arc<wgpu::Buffer>>,
    /// The number of indices to draw while `ibuf` is `Some`.
    pub(super) index_count: u32,
    /// The element width of `ibuf` — carried over from the mesh as-is, not derived.
    pub(super) index_format: wgpu::IndexFormat,
    pub(super) bind_group: std::sync::Arc<wgpu::BindGroup>,
    pub(super) unlit: bool,
    /// Baked lighting + the sun's cascade term: casts shadows and skips the G-buffer.
    pub(super) baked_lit: bool,
    pub(super) is_skybox: bool,
    /// A painted backdrop: `backdrop.wgsl` + the backdrop pipeline, drawn first
    /// (`DrawLayer::Backdrop`). See `gizmo_renderer::backdrop`.
    pub(super) is_backdrop: bool,
    pub(super) skeleton_bind_group: Option<std::sync::Arc<wgpu::BindGroup>>,
    pub(super) is_transparent: bool,
    /// This batch's slot in the frame's paint order — see [`draw_layer`].
    pub(super) layer: DrawLayer,
    /// Start of this batch's CAMERA-visible instances in region A of the instance buffer.
    pub(super) first_instance: u32,
    /// Number of camera-visible instances (== the old camera-culled set). Main/geometry
    /// passes draw `first_instance .. first_instance + camera_count`.
    pub(super) camera_count: u32,
    /// Start of this batch's SHADOW-ONLY casters in region B (all camera instances of all
    /// batches come first, then all shadow-only casters — see `collect_draw_items`). These
    /// are NOT contiguous with the camera range, so shadow passes draw them as a separate
    /// range. (Only the shadow passes read it — no shadows on web, the fields are dead there.)
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) shadow_first_instance: u32,
    /// Number of shadow-only casters (region B) for this batch.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(super) shadow_count: u32,
    /// Representative distance used to sort BACKDROP and TRANSPARENT batches back-to-front
    /// (see `cmp_draw_order` / `batch_sort_depth`). 0.0 for opaque batches (unused —
    /// the depth buffer resolves opaque draw order).
    ///
    /// For a backdrop it is measured from the ORIGIN, not from the camera: a camera-locked
    /// instance's translation is already an offset from the viewer, so its length is the
    /// distance. Measuring it from `cam_pos` would sort the sky by how far the player has
    /// driven from the middle of the map.
    pub(super) sort_depth: f32,
}

impl DrawItem {
    /// Binds this item's geometry and issues the draw call for `instances`.
    ///
    /// This is the ONE place where the indexed/plain decision is made. The engine has eight
    /// mesh draw sites (z-pass, G-buffer, two shadow passes, two branches in forward), and
    /// drawing an item indexed at one and plain at another produces an inconsistent frame — its
    /// most visible form is a shadow that does not match its object. If the branching is spread
    /// out, skipping one site becomes a silent bug.
    pub(super) fn record_draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        instances: std::ops::Range<u32>,
    ) {
        pass.set_vertex_buffer(0, self.vbuf.slice(..));
        match &self.ibuf {
            Some(ibuf) => {
                // Format mesh'ten geliyor: `Mesh::new_indexed` 65536 tekil vertex'e kadar
                // `Uint16` yazıyor. Türetmek değil TAŞIMAK önemli — tamponu yazıldığından
                // farklı bir formatta bağlamak çökmez, sessizce yanlış üçgen çizer.
                pass.set_index_buffer(ibuf.slice(..), self.index_format);
                pass.draw_indexed(0..self.index_count, 0, instances);
            }
            None => pass.draw(0..self.vertex_count, instances),
        }
    }

    /// Camera-visible instance range (region A), clamped to what actually fit the GPU
    /// instance buffer (`uploaded`). `.max(start)` keeps the range non-reversed when this
    /// batch's region was entirely truncated (an empty range = a 0-instance no-op draw).
    ///
    /// **Not gated on the target**, unlike its shadow sibling below: the z-pass and the
    /// G-buffer pass call it too, and those compile on wasm. It used to be shadow-only —
    /// the geometry passes inlined the same expression — so the gate went unnoticed until
    /// they were switched to this helper and only the wasm build failed.
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
    baked_lit: bool,
    is_skybox: bool,
    is_backdrop: bool,
}

pub(crate) struct BatchData {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    ibuf: Option<std::sync::Arc<wgpu::Buffer>>,
    index_count: u32,
    index_format: wgpu::IndexFormat,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    vertex_count: u32,
    unlit: bool,
    baked_lit: bool,
    is_skybox: bool,
    is_backdrop: bool,
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

                let routing = crate::renderer::routing::route($mat.material_type);

                let center_mat = Mat4::from_translation($mesh.center_offset);
                let model = $trans.matrix * center_mat;

                // Where this mesh is actually DRAWN. The same matrix for everything except a
                // camera-locked backdrop, whose vertex shader adds the camera position itself
                // (`backdrop.wgsl`) — so its authored transform is not where the triangles end
                // up, and culling or LOD-ing against it reasons about the wrong place. The
                // INSTANCE below keeps the authored matrix: hand the shader this one and it
                // adds the camera position twice.
                let drawn_model = crate::renderer::backdrop::camera_locked_model(
                    $mat.material_type,
                    &model,
                    cam_pos,
                );

                // CPU Frustum Culling
                let local_cx = ($mesh.bounds.min.x + $mesh.bounds.max.x) * 0.5;
                let local_cy = ($mesh.bounds.min.y + $mesh.bounds.max.y) * 0.5;
                let local_cz = ($mesh.bounds.min.z + $mesh.bounds.max.z) * 0.5;
                let world_c = drawn_model.transform_point3(Vec3::new(local_cx, local_cy, local_cz));
                let hx = ($mesh.bounds.max.x - $mesh.bounds.min.x) * 0.5;
                let hy = ($mesh.bounds.max.y - $mesh.bounds.min.y) * 0.5;
                let hz = ($mesh.bounds.max.z - $mesh.bounds.min.z) * 0.5;
                let local_r = (hx * hx + hy * hy + hz * hz).sqrt();
                let sx = drawn_model.x_axis.truncate().length();
                let sy = drawn_model.y_axis.truncate().length();
                let sz = drawn_model.z_axis.truncate().length();
                let world_r = local_r * sx.max(sy).max(sz);

                // Camera-visible → main passes; an off-screen shadow caster inside a
                // cascade's light frustum → shadow maps only (main passes use
                // `camera_count`, shadow passes the full range); otherwise skip. Shared
                // with the studio path so the cull test + caster predicate can't drift —
                // now the tighter AABB test (was a bounding sphere here).
                // ONE `Aabb::transform` for the whole decision. `classify_visibility` used to
                // redo it per frustum — 5× here, once for the camera and once per cascade —
                // for the same box and the same answer.
                let drawn_world_aabb = $mesh.bounds.transform(&drawn_model);
                let camera_visible = match crate::renderer::classify_visibility_world(
                    &frustum,
                    &cascade_frusta,
                    drawn_world_aabb,
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
                // LOD tamponları DÜZLEŞTİRİLMİŞ (`Mesh::new` meshopt çıktısını geri açıyor),
                // yani mesh'in indeksleri tam çözünürlüklü vertex dizisine göre ve LOD1
                // aktifken GEÇERSİZ. Onları burada düşürmek, karışık bir mesh'i indeksli
                // sanıp yanlış üçgenler çizmenin tek engeli.
                let (active_ibuf, active_index_count, active_index_format) = if use_lod1 {
                    (None, 0, wgpu::IndexFormat::Uint32)
                } else {
                    ($mesh.ibuf.clone(), $mesh.index_count, $mesh.index_format)
                };

                // What the shader is handed, which is not always what was authored: a PLACED
                // backdrop rides the same pipeline as a locked one, and that pipeline adds the
                // camera position in the vertex shader. `instance_model` takes it back out so
                // the two cancel and the geometry lands where the level put it. Identity for
                // everything else. See `renderer::backdrop`.
                let upload_model = crate::renderer::backdrop::instance_model(
                    $mat.material_type,
                    &model,
                    cam_pos,
                );
                let instance_data = crate::renderer::gpu_types::InstanceRaw::new(
                    upload_model.to_cols_array_2d(),
                    [$mat.albedo.x, $mat.albedo.y, $mat.albedo.z, $mat.albedo.w],
                    $mat.roughness,
                    $mat.metallic,
                    routing.instance_flag,
                    $mat.anisotropy,
                    $mat.clear_coat,
                    $mat.subsurface,
                    // The two `Material` lighting knobs. Zero unless the material sets them,
                    // and the shader's `(lit + ambient) * base + emissive` then collapses to
                    // exactly the multiply chain it was before they existed.
                    $mat.ambient.to_array(),
                    $mat.emissive.to_array(),
                );
                let skel_bg = $skeleton.map(|s: &crate::renderer::components::Skeleton| s.bind_group.clone());

                // Compute the pass-routing flags up front so they can be part of the
                // batch key (see BatchKey docs) — not just read from the first material.
                //
                // What each material type *means* is decided once, in `gizmo-renderer::routing`,
                // because this loop and `gizmo-studio`'s each used to decide it with a wildcard
                // match and the wildcards disagreed: `BakedLit` was routed here and defaulted
                // there, `Grid` the other way round. `MaterialType` is `#[non_exhaustive]`, so a
                // wildcard is obligatory *here* and a ninth variant could never be a compile error
                // in this file — which is why the decision moved to the crate that defines it.
                let is_skybox = routing.is_skybox;
                let baked_lit = routing.baked_lit;
                let is_backdrop = routing.is_backdrop;
                // "Not in the deferred path", which baked-lit also is not. The two part company in
                // the shadow pass, where baked-lit casts and unlit does not. A backdrop rides the
                // same flag, and that is what keeps it out of the z-prepass, the G-buffer and both
                // shadow passes without any further edit to them.
                let unlit = routing.skips_deferred;
                let is_transparent = $mat.is_transparent || $mat.albedo.w < 0.99;

                let key = BatchKey {
                    vbuf_id: std::sync::Arc::as_ptr(&active_vbuf) as usize,
                    mat_id: std::sync::Arc::as_ptr(&$mat.bind_group) as usize,
                    skeleton_id: skel_bg.as_ref().map(|bg| std::sync::Arc::as_ptr(bg) as usize),
                    is_transparent,
                    unlit,
                    baked_lit,
                    is_skybox,
                    is_backdrop,
                };

                let batch = cache.batches.entry(key).or_insert_with(|| BatchData {
                    vbuf: active_vbuf.clone(),
                    ibuf: active_ibuf.clone(),
                    index_count: active_index_count,
                    index_format: active_index_format,
                    bind_group: $mat.bind_group.clone(),
                    vertex_count: active_vertex_count,
                    unlit,
                    baked_lit,
                    is_skybox,
                    is_backdrop,
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

        // **`LodGroup` is honoured here, not only in `gizmo-studio`.** The components and
        // `LodGroup::select_mesh` have always existed, but the only thing that looked at them was
        // studio's own render pipeline — this pass borrowed `Mesh`, `GlobalTransform` and
        // `Material` and never asked. For anyone using the engine's out-of-the-box pass the
        // feature was therefore inert: a scene carrying three detail levels for a building drew
        // all three of them, at every distance, for ever.
        //
        // Same semantics as studio's, deliberately, because two answers to "which mesh is this
        // entity" is worse than either: a `LodGroup` **overrides** the entity's own `Mesh`, and a
        // distance past the last level means cull rather than draw the coarsest.
        //
        // Distance is to the entity's world translation — the same point studio measures to,
        // reached from a `GlobalTransform` here and from the assembled model matrix there. The
        // three-case answer itself is `LodGroup::pick`, so only that route is ours.
        let lod_groups = world.borrow::<crate::renderer::components::LodGroup>();
        if let Some(mut q) = world.query::<(&Mesh, &gizmo_physics_core::components::GlobalTransform, &Material)>() {
            for (e, (mesh, trans, mat)) in q.iter_mut() {
                let Some(mesh) = crate::renderer::components::LodGroup::pick(
                    lod_groups.get(e),
                    mesh,
                    cam_pos.distance(trans.matrix.w_axis.truncate()),
                ) else {
                    continue;
                };
                process_mesh!(e, mesh, trans, mat, skeletons.get(e));
            }
        }
        
        let meshes = world.try_get_resource::<gizmo_core::asset::Assets<Mesh>>().ok();
        let materials = world.try_get_resource::<gizmo_core::asset::Assets<Material>>().ok();
        
        if let (Some(meshes), Some(materials)) = (meshes, materials) {
            if let Some(mut q) = world.query::<(&gizmo_core::asset::Handle<Mesh>, &gizmo_physics_core::components::GlobalTransform, &gizmo_core::asset::Handle<Material>)>() {
                for (e, (h_mesh, trans, h_mat)) in q.iter_mut() {
                    if let (Some(mesh), Some(mat)) = (meshes.get(h_mesh), materials.get(h_mat)) {
                        let Some(mesh) = crate::renderer::components::LodGroup::pick(
                            lod_groups.get(e),
                            mesh,
                            cam_pos.distance(trans.matrix.w_axis.truncate()),
                        ) else {
                            continue;
                        };
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
            // Depth key matters for the two layers the depth buffer cannot sort: transparent
            // (no depth write) and backdrop (no depth write AND every vertex pinned to the
            // same depth). A backdrop's instances are camera-RELATIVE offsets, so their
            // distance is measured from the origin — see `DrawItem::sort_depth`.
            let sort_depth = if batch.is_backdrop {
                batch_sort_depth(&batch.instances, Vec3::ZERO)
            } else if batch.is_transparent {
                batch_sort_depth(&batch.instances, cam_pos)
            } else {
                0.0
            };
            local_instances.extend(&batch.instances);
            local_draw_items.push(DrawItem {
                vbuf: batch.vbuf.clone(),
                vertex_count: batch.vertex_count,
                ibuf: batch.ibuf.clone(),
                index_count: batch.index_count,
                index_format: batch.index_format,
                bind_group: batch.bind_group.clone(),
                unlit: batch.unlit,
                baked_lit: batch.baked_lit,
                is_skybox: batch.is_skybox,
                is_backdrop: batch.is_backdrop,
                skeleton_bind_group: batch.skeleton_bind_group.clone(),
                is_transparent: batch.is_transparent,
                layer: draw_layer(batch.is_backdrop, batch.is_transparent),
                first_instance,
                camera_count,
                shadow_first_instance: 0,
                shadow_count: 0,
                sort_depth,
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

        // Order draw items for correct compositing: backdrops first, then the opaque world,
        // then transparent back-to-front. MUST run after region B backfill (which indexes draw
        // items by batch order); reordering here is safe because the instance ranges are
        // baked-in indices, independent of draw-item order, and every pass filters by its own
        // flags. The forward pass draws these in order, and the backdrop and transparent
        // pipelines both disable depth-write, so this order is the only thing that composites
        // them correctly (previously they were drawn in arbitrary HashMap order). Stable sort
        // keeps opaque batches in their build order.
        local_draw_items
            .sort_by(|a, b| cmp_draw_order((a.layer, a.sort_depth), (b.layer, b.sort_depth)));

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
            baked_lit: false,
            is_skybox: false,
            is_backdrop: false,
        };
        let transparent = BatchKey {
            is_transparent: true,
            ..base.clone()
        };
        let unlit = BatchKey {
            unlit: true,
            baked_lit: false,
            ..base.clone()
        };
        let skybox = BatchKey {
            is_skybox: true,
            ..base.clone()
        };
        // A backdrop shares the `unlit` routing flag with a plain unlit material (both skip
        // the deferred path) and, if it is untextured, the cached white-texture bind group as
        // well — so without `is_backdrop` in the key the two collide, and whichever the ECS
        // iterated first decides whether a wall is camera-locked.
        let backdrop = BatchKey {
            unlit: true,
            is_backdrop: true,
            ..base.clone()
        };

        assert_ne!(base, transparent, "opaque and transparent must be separate batches");
        assert_ne!(base, unlit, "PBR and unlit must be separate batches");
        assert_ne!(base, skybox, "PBR and skybox must be separate batches");
        assert_ne!(unlit, backdrop, "unlit and backdrop must be separate batches");

        // Identical routing + shared texture/mesh → same batch (instancing preserved).
        assert_eq!(base, base.clone(), "identical materials must still batch together");
    }
}

#[cfg(test)]
mod transparent_order_tests {
    use super::{batch_sort_depth, cmp_draw_order, draw_layer, DrawLayer, Vec3};
    use crate::renderer::gpu_types::InstanceRaw;
    use bytemuck::Zeroable;

    use DrawLayer::{Backdrop, Opaque, Transparent};

    fn inst_at(x: f32, y: f32, z: f32) -> InstanceRaw {
        let mut i = InstanceRaw::zeroed();
        // Column-major identity rotation/scale; translation in column 3.
        i.model = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [x, y, z, 1.0],
        ];
        i
    }

    #[test]
    fn batch_depth_is_centroid_distance_to_camera() {
        let cam = Vec3::new(0.0, 0.0, 0.0);
        // Single instance 10 units down -Z → distance 10.
        assert!((batch_sort_depth(&[inst_at(0.0, 0.0, -10.0)], cam) - 10.0).abs() < 1e-3);
        // Two instances at x=±3, z=-4 → centroid (0,0,-4), distance 4 (not 5).
        let d = batch_sort_depth(&[inst_at(3.0, 0.0, -4.0), inst_at(-3.0, 0.0, -4.0)], cam);
        assert!((d - 4.0).abs() < 1e-3, "centroid distance wrong: {d}");
        // Empty batch → 0.
        assert_eq!(batch_sort_depth(&[], cam), 0.0);
    }

    // Opaque batches sort ahead of transparent ones; transparent sort back-to-front
    // (farthest first) so the depth-write-disabled alpha pass composites correctly.
    #[test]
    fn opaque_first_then_transparent_back_to_front() {
        let mut items = vec![
            (Transparent, 5.0),  // near transparent
            (Opaque, 0.0),
            (Transparent, 20.0), // far transparent
            (Opaque, 0.0),
            (Transparent, 12.0), // mid transparent
        ];
        items.sort_by(|a, b| cmp_draw_order(*a, *b));
        assert_eq!(
            items,
            vec![
                (Opaque, 0.0),
                (Opaque, 0.0),
                (Transparent, 20.0),
                (Transparent, 12.0),
                (Transparent, 5.0)
            ]
        );
    }

    // The whole point: the resulting transparent order depends on DEPTH, not on the
    // (nondeterministic HashMap) insertion order.
    #[test]
    fn transparent_order_independent_of_input_order() {
        let mut a = vec![(Transparent, 3.0), (Transparent, 9.0), (Transparent, 1.0)];
        let mut b = vec![(Transparent, 1.0), (Transparent, 3.0), (Transparent, 9.0)];
        a.sort_by(|x, y| cmp_draw_order(*x, *y));
        b.sort_by(|x, y| cmp_draw_order(*x, *y));
        assert_eq!(a, b);
        assert_eq!(a, vec![(Transparent, 9.0), (Transparent, 3.0), (Transparent, 1.0)]);
    }

    // ── ITEM 7, property (1): "drawn before the world" ─────────────────────────────────────

    // The backdrop layer is the whole of that property. It has to beat BOTH other layers,
    // whatever their depths say: the near-far ordering inside a layer must never promote an
    // opaque or transparent batch ahead of a backdrop.
    #[test]
    fn backdrops_are_drawn_before_everything_else() {
        let mut items = vec![
            (Transparent, 900.0), // a distant blended object — the farthest thing in the frame
            (Opaque, 0.0),
            (Backdrop, 300.0),
            (Opaque, 0.0),
            (Backdrop, 1200.0),
            (Transparent, 4.0),
        ];
        items.sort_by(|a, b| cmp_draw_order(*a, *b));
        assert_eq!(
            items,
            vec![
                // Backdrops first, farthest of them first…
                (Backdrop, 1200.0),
                (Backdrop, 300.0),
                // …then the world…
                (Opaque, 0.0),
                (Opaque, 0.0),
                // …then the blended layer, back-to-front.
                (Transparent, 900.0),
                (Transparent, 4.0),
            ],
            "a backdrop drawn after a transparent object paints over it — the transparent \
             pipeline writes no depth, so nothing else can put the backdrop underneath"
        );
    }

    // Two overlapping backdrop panels are the one case the depth buffer provably cannot
    // resolve: both are pinned to the same NDC depth AND neither writes depth. Their order
    // must therefore come from the comparator and be independent of how the batch HashMap
    // happened to drain this frame.
    #[test]
    fn overlapping_backdrops_composite_in_a_deterministic_order() {
        let mut a = vec![(Backdrop, 80.0), (Backdrop, 500.0), (Backdrop, 200.0)];
        let mut b = vec![(Backdrop, 200.0), (Backdrop, 80.0), (Backdrop, 500.0)];
        a.sort_by(|x, y| cmp_draw_order(*x, *y));
        b.sort_by(|x, y| cmp_draw_order(*x, *y));
        assert_eq!(a, b, "backdrop order must not depend on insertion order");
        assert_eq!(a, vec![(Backdrop, 500.0), (Backdrop, 200.0), (Backdrop, 80.0)]);
    }

    // A backdrop material with a sub-1.0 alpha is still a backdrop. `is_transparent` is
    // inferred from that alpha, so if it took precedence a hazy skyline would sort into the
    // transparent bucket and be painted over the world instead of behind it.
    #[test]
    fn a_transparent_backdrop_is_still_a_backdrop() {
        assert_eq!(draw_layer(true, true), Backdrop);
        assert_eq!(draw_layer(true, false), Backdrop);
        assert_eq!(draw_layer(false, true), Transparent);
        assert_eq!(draw_layer(false, false), Opaque);
    }

    // A camera-locked batch's instance translations are offsets FROM THE VIEWER, so the
    // distance that orders them is measured from the origin. Measured from `cam_pos` instead,
    // the sky's paint order would depend on where in the map the player is standing — the
    // panels would swap over as the camera drove past the origin.
    #[test]
    fn backdrop_sort_depth_is_measured_from_the_camera_locked_origin() {
        let near_panel = [inst_at(0.0, 0.0, -100.0)];
        let far_panel = [inst_at(0.0, 0.0, -400.0)];

        for cam in [Vec3::ZERO, Vec3::new(0.0, 0.0, -900.0), Vec3::new(650.0, 20.0, 480.0)] {
            // The measure the batcher uses for a backdrop ignores `cam` entirely…
            let near = batch_sort_depth(&near_panel, Vec3::ZERO);
            let far = batch_sort_depth(&far_panel, Vec3::ZERO);
            assert!((near - 100.0).abs() < 1e-3 && (far - 400.0).abs() < 1e-3);
            assert_eq!(
                cmp_draw_order((Backdrop, far), (Backdrop, near)),
                std::cmp::Ordering::Less,
                "the far panel must paint first"
            );

            // …whereas the camera-relative measure does not, and at cam.z = -900 it even
            // reverses: that is the ordering flip this avoids.
            let _ = batch_sort_depth(&near_panel, cam);
        }
        let flipped_near = batch_sort_depth(&near_panel, Vec3::new(0.0, 0.0, -900.0));
        let flipped_far = batch_sort_depth(&far_panel, Vec3::new(0.0, 0.0, -900.0));
        assert!(
            flipped_near > flipped_far,
            "premise: from a camera at z=-900 the 'near' panel is the farther one ({flipped_near} \
             vs {flipped_far}) — which is why a backdrop must not be sorted from the camera"
        );
    }
}
