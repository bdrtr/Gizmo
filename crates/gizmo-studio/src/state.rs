

pub struct StudioState {
    pub current_fps: f32,
    pub editor_camera: u32,
    pub do_raycast: bool,
    pub physics_accumulator: f32,
    pub asset_watcher: Option<gizmo::renderer::hot_reload::AssetWatcher>,
}

pub struct DebugAssets {
    pub cube: gizmo::renderer::components::Mesh,
    pub white_tex: std::sync::Arc<gizmo::wgpu::BindGroup>,
}

pub struct ShaderReloadEvent;
