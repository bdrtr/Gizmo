use gizmo::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum DragAxis { X, Y, Z }

#[derive(Clone, Copy, PartialEq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

/// Aktif bir diyalog metni
#[derive(Clone, Debug)]
pub struct DialogueEntry {
    pub speaker: String,
    pub text: String,
    pub timer: f32,    // kalan süre (saniye), 0 = süresiz
}

/// Yarış checkpoint'i
#[derive(Clone, Debug)]
pub struct Checkpoint {
    pub id: u32,
    pub position: Vec3,
    pub radius: f32,
    pub activated: bool,
}

/// Yarış durumu
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RaceStatus {
    Idle,
    Running,
    Finished,
}

// --- ECS KULLANIMI İÇİN EVENT VE RESOURCE YAPILARI ---

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AppMode {
    MainMenu,
    InGame,
    Settings,
}

#[derive(Clone, Debug)]
pub struct PlayerStats {
    pub health: f32,
    pub max_health: f32,
    pub ammo: u32,
    pub max_ammo: u32,
}


pub struct PachinkoSpawnerState {
    pub timer: f32,
    pub count: u32,
}

pub struct SpawnDominoEvent {
    pub count: u32,
}

pub struct ReleaseDominoEvent {
    pub count: u32,
}

pub struct TextureLoadEvent {
    pub entity_id: u32,
    pub path: String,
}

pub struct AssetSpawnEvent {
    pub path: String,
}

pub struct ShaderReloadEvent;

pub struct SelectionEvent {
    pub entity_id: u32,
}

pub struct DominoAppState {
    pub active_ball_id: Option<u32>,
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
    pub gizmo_mode: GizmoMode,
    pub egui_wants_pointer: bool,
    pub asset_watcher: Option<gizmo::renderer::hot_reload::AssetWatcher>,
    pub physics_accumulator: f32,
    pub target_physics_fps: f32,
    pub sphere_prefab_id: u32,
    pub cube_prefab_id: u32,
    pub free_cam: bool,

    // ── Oyun Sistemi ──────────────────────────────────────────────────
    /// Ekranda gösterilen diyalog (None ise gizli)
    pub active_dialogue: Option<DialogueEntry>,
    /// Aktif ara sahne adı (None ise cutscene yok)
    pub active_cutscene: Option<String>,
    /// Yarış checkpoint'leri
    pub checkpoints: Vec<Checkpoint>,
    /// Yarış durumu
    pub race_status: RaceStatus,
    /// Yarış süresi (saniye)
    pub race_timer: f32,
    pub camera_follow_target: Option<u32>,
    /// Toplam geçen süre (saniye) — Time resource'u için
    pub total_elapsed: f64,
    pub ps1_race: Option<crate::race::RaceState>,
}
