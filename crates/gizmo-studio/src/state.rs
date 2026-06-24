/// Per-application runtime state for Gizmo Studio (FPS, camera entities,
/// timers and frame statistics) carried across the engine's update loop.
pub struct StudioState {
    pub current_fps: f32,
    pub actual_dt: f32,
    pub editor_camera: u32,
    pub game_camera: u32,
    pub do_raycast: bool,
    pub physics_accumulator: f32,
    pub asset_watcher: Option<gizmo::renderer::hot_reload::AssetWatcher>,
    /// Garbage Collection zamanlayıcı — soft-deleted entity'leri temizler
    pub gc_timer: f32,
    /// Auto-Save zamanlayıcı — sahneyi belirli aralıklarla yedekler
    pub autosave_timer: f32,
    /// Sahnedeki aktif (görünür) entity sayısı
    pub visible_entity_count: u32,
    /// Son frame'deki draw call sayısı
    pub draw_call_count: u32,
}

/// GPU resources used to render editor debug gizmos (primitive meshes and a
/// default white texture bind group).
pub struct DebugAssets {
    pub cube: gizmo::renderer::components::Mesh,
    pub sphere: gizmo::renderer::components::Mesh,
    pub white_tex: std::sync::Arc<gizmo::wgpu::BindGroup>,
}

/// Event fired to request a runtime shader hot-reload.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShaderReloadEvent;
