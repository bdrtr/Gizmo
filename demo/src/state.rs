use gizmo::prelude::*;
use std::cell::{Cell, RefCell};

#[derive(Clone, Copy, PartialEq)]
pub enum DragAxis { X, Y, Z }

#[derive(Clone, Copy, PartialEq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

pub struct GameState {
    pub bouncing_box_id: u32,
    pub player_id: u32,
    pub skybox_id: u32,
    pub inspector_selected_entity: Option<u32>,
    #[allow(dead_code)] // AudioManager'ın OutputStream'i canlı tutulmalı
    pub audio: Option<gizmo::audio::AudioManager>,
    pub do_raycast: bool,
    pub gizmo_x: u32,
    pub gizmo_y: u32,
    pub gizmo_z: u32,
    pub dragging_axis: Option<DragAxis>,
    pub drag_start_t: f32,
    pub drag_original_pos: Vec3,
    pub drag_original_scale: Vec3,
    pub drag_original_rot: Quat,
    pub current_fps: f32,
    pub new_selection_request: Cell<Option<u32>>,
    pub spawn_domino_requests: Cell<u32>,
    pub release_domino_requests: Cell<u32>,
    pub domino_ball_id: Cell<Option<u32>>,
    pub texture_load_requests: RefCell<Vec<(u32, String)>>,
    pub asset_manager: RefCell<gizmo::renderer::asset::AssetManager>,
    pub gizmo_mode: GizmoMode,
    pub egui_wants_pointer: bool,
    pub asset_watcher: Option<gizmo::renderer::hot_reload::AssetWatcher>,
    pub script_engine: RefCell<Option<gizmo::scripting::ScriptEngine>>,
    pub physics_accumulator: f32,
    pub target_physics_fps: f32,
    pub sphere_prefab_id: u32,
    pub cube_prefab_id: u32,
    pub asset_spawn_requests: RefCell<Vec<String>>,
    pub shader_reload_request: Cell<bool>,
    pub post_process_settings: RefCell<gizmo::renderer::renderer::PostProcessUniforms>,
    pub editor_state: RefCell<gizmo::editor::EditorState>,
    pub free_cam: bool,
}
