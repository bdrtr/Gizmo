//! Studio render-pipeline batching types + the per-frame cache. Extracted verbatim from
//! render_pipeline.rs (pure move). All items are `pub(super)`: `execute_render_pipeline`
//! (parent) fills these batches and the `passes` recorders read `FlatBatchData`.

use super::*;

// The pointer triple identifies mesh + texture bind group + skeleton, but the
// texture bind group is cached and SHARED across distinct materials (default
// white texture, same file), so two materials differing only in material type
// would collide into one batch and inherit the first-iterated material's
// `is_skybox`/`is_grid`/`is_unlit`. Those flags gate shadow casting and pass
// routing (e.g. `passes::record_studio_shadow_passes` skips unlit/skybox/grid), so a real
// PBR object batched under an unlit-first batch would silently stop casting
// shadows. Keying on the routing flags too keeps them apart.
pub(super) type BatchKey = (
    *const wgpu::Buffer,
    *const wgpu::BindGroup,
    *const wgpu::BindGroup,
    bool, // is_skybox
    bool, // is_grid
    bool, // is_unlit
);

pub(super) struct BatchData {
    pub(super) vbuf: std::sync::Arc<wgpu::Buffer>,
    pub(super) vertex_count: u32,
    pub(super) bind_group: std::sync::Arc<wgpu::BindGroup>,
    pub(super) skeleton_bg: std::sync::Arc<wgpu::BindGroup>,
    pub(super) instances: Vec<gizmo::renderer::InstanceRaw>,
    /// Casters outside the camera frustum but inside a shadow cascade's light frustum —
    /// drawn into the shadow maps only so off-screen objects still cast visible shadows.
    pub(super) shadow_instances: Vec<gizmo::renderer::InstanceRaw>,
    pub(super) is_skybox: bool,
    pub(super) is_grid: bool,
    pub(super) is_unlit: bool,
}

pub(super) struct FlatBatchData {
    pub(super) vbuf: std::sync::Arc<wgpu::Buffer>,
    pub(super) vertex_count: u32,
    pub(super) bind_group: std::sync::Arc<wgpu::BindGroup>,
    pub(super) skeleton_bg: std::sync::Arc<wgpu::BindGroup>,
    pub(super) start_instance: u32,
    /// End of the CAMERA-visible range (main passes draw `start..end_instance`).
    pub(super) end_instance: u32,
    /// End of the full range incl. off-screen shadow casters (shadow pass draws
    /// `start..shadow_end_instance`). Equals `end_instance` when there are none.
    pub(super) shadow_end_instance: u32,
    pub(super) is_transparent: bool,
    pub(super) is_double_sided: bool,
    pub(super) is_skybox: bool,
    pub(super) is_grid: bool,
    pub(super) is_unlit: bool,
}

pub(super) struct PipelineCache {
    pub(super) opaque_batches: std::collections::HashMap<BatchKey, BatchData>,
    pub(super) opaque_double_sided_batches: std::collections::HashMap<BatchKey, BatchData>,
    pub(super) transparent_batches: std::collections::HashMap<BatchKey, BatchData>,
    pub(super) all_instances: Vec<gizmo::renderer::InstanceRaw>,
    pub(super) flat_batches: Vec<FlatBatchData>,
    pub(super) vec_pool: Vec<Vec<gizmo::renderer::InstanceRaw>>,
}

impl Default for PipelineCache {
    fn default() -> Self {
        Self {
            opaque_batches: std::collections::HashMap::with_capacity(256),
            opaque_double_sided_batches: std::collections::HashMap::with_capacity(256),
            transparent_batches: std::collections::HashMap::with_capacity(256),
            all_instances: Vec::with_capacity(10000),
            flat_batches: Vec::with_capacity(256),
            vec_pool: Vec::with_capacity(256),
        }
    }
}

thread_local! {
    pub(super) static CACHE: RefCell<PipelineCache> = RefCell::new(PipelineCache::default());
}

/// Representative camera distance of an instanced batch: distance from `cam_pos` to the
/// centroid of its instance world positions (the translation column of each
/// `InstanceRaw::model`). Used to order transparent batches back-to-front RELATIVE TO EACH
/// OTHER (inter-batch) — the per-instance sort inside a batch already handles intra-batch
/// order, but batches drain from a HashMap in arbitrary order, so overlapping transparents
/// of different materials (= different batches) blended wrongly without this. Returns 0 for
/// an empty batch. Mirrors the game path's `batch_sort_depth`.
pub(super) fn batch_centroid_depth(
    instances: &[gizmo::renderer::InstanceRaw],
    cam_pos: Vec3,
) -> f32 {
    if instances.is_empty() {
        return 0.0;
    }
    let mut centroid = Vec3::ZERO;
    for inst in instances {
        centroid += Vec3::new(inst.model[3][0], inst.model[3][1], inst.model[3][2]);
    }
    centroid /= instances.len() as f32;
    cam_pos.distance(centroid)
}

#[cfg(test)]
mod tests {
    use super::batch_centroid_depth;
    use gizmo::renderer::InstanceRaw;
    use gizmo::prelude::Vec3;

    fn inst_at(x: f32, y: f32, z: f32) -> InstanceRaw {
        InstanceRaw {
            model: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [x, y, z, 1.0],
            ],
            albedo_color: [1.0, 1.0, 1.0, 1.0],
            roughness: 0.5,
            metallic: 0.0,
            unlit: 0.0,
            _padding: 0.0,
        }
    }

    #[test]
    fn centroid_depth_is_distance_to_batch_center() {
        let cam = Vec3::new(0.0, 0.0, 0.0);
        // Single instance 10 units down -Z → distance 10.
        assert!((batch_centroid_depth(&[inst_at(0.0, 0.0, -10.0)], cam) - 10.0).abs() < 1e-3);
        // Two instances at x=±3, z=-4 → centroid (0,0,-4), distance 4 (not 5).
        let d = batch_centroid_depth(&[inst_at(3.0, 0.0, -4.0), inst_at(-3.0, 0.0, -4.0)], cam);
        assert!((d - 4.0).abs() < 1e-3, "centroid distance wrong: {d}");
        // Empty batch → 0.
        assert_eq!(batch_centroid_depth(&[], cam), 0.0);
    }

    // Centroid averages in all three axes, not just Z, and the distance is measured
    // from the ACTUAL camera position (not assumed origin).
    #[test]
    fn centroid_depth_is_3d_and_camera_relative() {
        // Two instances at the origin and (2,2,2) → centroid (1,1,1); from a camera at
        // the origin that is sqrt(3) away.
        let d = batch_centroid_depth(
            &[inst_at(0.0, 0.0, 0.0), inst_at(2.0, 2.0, 2.0)],
            Vec3::ZERO,
        );
        assert!((d - 3.0_f32.sqrt()).abs() < 1e-3, "3D centroid distance wrong: {d}");

        // Same single instance, camera moved to (10,0,0): distance is 10, proving the
        // measure is camera-relative rather than origin-relative.
        let d2 = batch_centroid_depth(&[inst_at(0.0, 0.0, 0.0)], Vec3::new(10.0, 0.0, 0.0));
        assert!((d2 - 10.0).abs() < 1e-3, "camera-relative distance wrong: {d2}");
    }

    // A far batch must sort ahead of a near one (back-to-front). This is the inter-batch
    // ordering the fix adds — instances within a batch were already sorted, but the batches
    // themselves drained in arbitrary HashMap order.
    #[test]
    fn far_batch_sorts_before_near_batch() {
        let cam = Vec3::ZERO;
        let near = vec![inst_at(0.0, 0.0, -5.0)];
        let far = vec![inst_at(0.0, 0.0, -50.0)];
        let mut batches = [near, far];
        batches.sort_by(|a, b| {
            batch_centroid_depth(b, cam)
                .partial_cmp(&batch_centroid_depth(a, cam))
                .unwrap()
        });
        // Farthest first.
        assert!((batch_centroid_depth(&batches[0], cam) - 50.0).abs() < 1e-3);
        assert!((batch_centroid_depth(&batches[1], cam) - 5.0).abs() < 1e-3);
    }
}
