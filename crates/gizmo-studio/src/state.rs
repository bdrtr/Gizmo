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

pub struct DebugAssets {
    pub cube: gizmo::renderer::components::Mesh,
    pub sphere: gizmo::renderer::components::Mesh,
    pub white_tex: std::sync::Arc<gizmo::wgpu::BindGroup>,
}

pub struct ShaderReloadEvent;
