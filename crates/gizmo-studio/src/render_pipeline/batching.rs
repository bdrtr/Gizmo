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
