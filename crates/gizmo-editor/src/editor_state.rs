//! Editor State — Editörün global durumunu yönetir
use crate::prefs::EditorPrefs;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Hash, Debug)]
pub enum EditorTab {
    Hierarchy,
    Inspector,
    AssetBrowser,
    SceneView,
    GameView,
    Console,
    BuildConsole,
    Settings,
    ScriptEditor,
}

/// Gizmo aracı modu
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

/// Build hedef işletim sistemi
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum BuildTarget {
    /// Mevcut işletim sistemi (native)
    Native,
    /// Linux (x86_64-unknown-linux-gnu)
    Linux,
    /// Windows (x86_64-pc-windows-gnu — cross gerektirir)
    Windows,
    /// macOS (x86_64-apple-darwin — yalnızca Mac üzerinde)
    MacOs,
}

/// Editor çalışma modu
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum EditorMode {
    /// Düzenleme modu — fizik durur, entity'ler serbestçe manipüle edilir
    Edit,
    /// Oyun modu — fizik ve scriptler çalışır
    Play,
    /// Duraklatılmış oyun modu
    Paused,
}


// --- Alt Durum Yapilari ---
pub struct CameraState {
    pub look_delta: Option<gizmo_math::Vec2>,
    pub pan_delta: Option<gizmo_math::Vec2>,
    pub orbit_delta: Option<gizmo_math::Vec2>,
    pub scroll_delta: Option<f32>,
    pub view: Option<gizmo_math::Mat4>,
    pub proj: Option<gizmo_math::Mat4>,
    pub bookmarks: [Option<(gizmo_math::Vec3, f32, f32)>; 10],
}
impl Default for CameraState {
    fn default() -> Self {
        Self { look_delta: None, pan_delta: None, orbit_delta: None, scroll_delta: None, view: None, proj: None, bookmarks: [None; 10] }
    }
}

pub struct BuildState {
    pub request: bool,
    pub target: BuildTarget,
    pub is_building: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub logs_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    pub cached_logs: Vec<(String, egui::Color32)>,
    pub start_time: Option<std::time::Instant>,
}
impl Default for BuildState {
    fn default() -> Self {
        Self { request: false, target: BuildTarget::Native, is_building: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)), logs_rx: None, cached_logs: Vec::new(), start_time: None }
    }
}

pub struct AssetBrowserState {
    pub filter: String,
    pub root: String,
    pub show: bool,
    pub workspace_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    pub cached_dir: Option<(String, std::time::Instant, Vec<(std::path::PathBuf, String, bool)>)>,
}
impl Default for AssetBrowserState {
    fn default() -> Self {
        Self { filter: String::new(), root: "demo/assets".to_string(), show: true, workspace_rx: None, cached_dir: None }
    }
}

pub struct SceneState {
    pub save_request: Option<String>,
    pub load_request: Option<String>,
    pub clear_request: bool,
    pub load_confirm_dialog: Option<String>,
    pub gizmo_original_transforms: std::collections::HashMap<gizmo_core::entity::Entity, gizmo_physics::components::Transform>,
}
impl Default for SceneState {
    fn default() -> Self {
        Self { save_request: None, load_request: None, clear_request: false, load_confirm_dialog: None, gizmo_original_transforms: std::collections::HashMap::new() }
    }
}

pub struct SelectionState {
    pub entities: std::collections::HashSet<gizmo_core::entity::Entity>,
    pub primary: Option<gizmo_core::entity::Entity>,
    pub highlight_box: Option<gizmo_core::entity::Entity>,
    pub rubber_band_start: Option<gizmo_math::Vec2>,
    pub rubber_band_current: Option<gizmo_math::Vec2>,
    pub rubber_band_request: Option<(gizmo_math::Vec2, gizmo_math::Vec2)>,
}
impl Default for SelectionState {
    fn default() -> Self {
        Self { entities: std::collections::HashSet::new(), primary: None, highlight_box: None, rubber_band_start: None, rubber_band_current: None, rubber_band_request: None }
    }
}

pub struct ScriptEditorState {
    pub open: bool,
    pub active_path: Option<String>,
    pub active_content: Option<String>,
    pub is_dirty: bool,
    pub pending_clear_confirm: bool,
}
impl Default for ScriptEditorState {
    fn default() -> Self { Self { open: false, active_path: None, active_content: None, is_dirty: false, pending_clear_confirm: false } }
}


pub struct ConsoleState {
    pub show_info: bool,
    pub show_warn: bool,
    pub show_error: bool,
    pub filter_text: String,
    
    // Cache
    pub cached_logs: Vec<gizmo_core::logger::LogEntry>,
    pub last_version: usize,

    // İstatistikler
    pub count_info: usize,
    pub count_warn: usize,
    pub count_error: usize,
}

impl Default for ConsoleState {
    fn default() -> Self {
        Self {
            show_info: true,
            show_warn: true,
            show_error: true,
            filter_text: String::new(),
            cached_logs: Vec::new(),
            last_version: 0,
            count_info: 0,
            count_warn: 0,
            count_error: 0,
        }
    }
}



/// Editörün tüm durumunu tutan yapı
pub struct EditorState {
    pub open: bool,
    pub mode: EditorMode,
    pub gizmo_mode: GizmoMode,
    
    pub play_start_request: bool,
    pub play_stop_request: bool,

    pub do_raycast: bool,
    pub mouse_ndc: Option<gizmo_math::Vec2>,
    pub gizmo_local_space: bool,

    pub history: crate::history::History,

    // Panellerin görünürlüğü (asset browser hariç)
    pub show_hierarchy: bool,
    pub show_inspector: bool,
    pub show_toolbar: bool,
    pub settings_open: bool,

    // Diğer global UI state
    pub hierarchy_filter: String,
    pub hide_editor_entities: bool,
    pub add_component_open: bool,
    pub last_error: Option<String>,
    pub status_message: String,
    pub scene_path: String,
    pub has_unsaved_changes: bool,
    pub dragged_asset: Option<String>,

    // Nested Yapılar
    pub camera: CameraState,
    pub build: BuildState,
    pub assets: AssetBrowserState,
    pub scene: SceneState,
    pub selection: SelectionState,
    pub console: ConsoleState,
    pub script: ScriptEditorState,

    pub prefs: EditorPrefs,

    pub despawn_requests: Vec<gizmo_core::entity::Entity>,
    pub generate_terrain_requests: Vec<gizmo_core::entity::Entity>,
    pub duplicate_requests: Vec<gizmo_core::entity::Entity>,
    pub toggle_visibility_requests: Vec<gizmo_core::entity::Entity>,

    pub prefab_save_request: Option<(gizmo_core::entity::Entity, String)>,
    pub prefab_load_request: Option<(String, Option<gizmo_core::entity::Entity>, Option<gizmo_math::Vec3>)>,
    pub spawn_request: Option<String>,
    pub spawn_asset_request: Option<String>,
    pub spawn_asset_position: Option<gizmo_math::Vec3>,
    pub gltf_load_request: Option<(String, Option<gizmo_math::Vec3>)>,
    pub reparent_request: Option<(gizmo_core::entity::Entity, gizmo_core::entity::Entity)>,
    pub unparent_request: Option<gizmo_core::entity::Entity>,
    pub add_component_request: Option<(gizmo_core::entity::Entity, String)>,

    pub scene_view_visible: bool,
    pub game_view_visible: bool,
    pub scene_view_rect: Option<egui::Rect>,
    pub game_view_rect: Option<egui::Rect>,
    pub scene_view_size: Option<egui::Vec2>,
    pub game_view_size: Option<egui::Vec2>,
    pub scene_texture_id: Option<egui::TextureId>,
    pub game_texture_id: Option<egui::TextureId>,
    pub dock_state: egui_dock::DockState<EditorTab>,

    pub debug_draw_requests: Vec<(
        gizmo_math::Vec3,
        gizmo_math::Quat,
        gizmo_math::Vec3,
        gizmo_math::Vec4,
    )>,
    pub debug_spawned_entities: Vec<(f32, u32)>,

    pub pending_dialog_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<(bool, Option<String>)>>>,
}

impl EditorState {
    pub fn new() -> Self {
        let prefs = EditorPrefs::load();
        Self {
            open: false,
            mode: EditorMode::Edit,
            gizmo_mode: GizmoMode::Translate,

            play_start_request: false,
            play_stop_request: false,

            do_raycast: false,
            mouse_ndc: None,
            gizmo_local_space: false,
            
            history: crate::history::History::new(prefs.max_history),

            show_hierarchy: true,
            show_inspector: true,
            show_toolbar: true,
            settings_open: false,

            hierarchy_filter: String::new(),
            hide_editor_entities: true,
            add_component_open: false,
            last_error: None,
            status_message: "Hazır".to_string(),
            scene_path: String::new(),
            has_unsaved_changes: false,
            dragged_asset: None,

            camera: CameraState::default(),
            build: BuildState::default(),
            assets: AssetBrowserState::default(),
            scene: SceneState::default(),
            selection: SelectionState::default(),
            console: ConsoleState::default(),
            script: ScriptEditorState::default(),
            prefs: prefs,

            despawn_requests: Vec::new(),
            generate_terrain_requests: Vec::new(),
            duplicate_requests: Vec::new(),
            toggle_visibility_requests: Vec::new(),

            prefab_save_request: None,
            prefab_load_request: None,
            spawn_request: None,
            spawn_asset_request: None,
            spawn_asset_position: None,
            gltf_load_request: None,
            reparent_request: None,
            unparent_request: None,
            add_component_request: None,

            scene_view_visible: true,
            game_view_visible: false,
            scene_view_rect: None,
            game_view_rect: None,
            scene_view_size: None,
            game_view_size: None,
            scene_texture_id: None,
            game_texture_id: None,
            
            dock_state: Self::load_layout().unwrap_or_else(create_default_dock_state),

            debug_draw_requests: Vec::new(),
            debug_spawned_entities: Vec::new(),

            pending_dialog_rx: None,
        }
    }

    pub fn is_tab_open(&self, tab: &EditorTab) -> bool {
        self.dock_state.iter_all_tabs().any(|node| node.1 == tab)
    }

    pub fn toggle_tab(&mut self, tab: EditorTab) {
        if let Some(index) = self.dock_state.find_tab(&tab) {
            self.dock_state.remove_tab(index);
        } else {
            self.dock_state.push_to_first_leaf(tab);
        }
    }
    
    pub fn open_tab(&mut self, tab: EditorTab) {
        if !self.is_tab_open(&tab) {
            self.dock_state.push_to_first_leaf(tab);
        }
    }
    // --- Selection API ---
    pub fn is_selected(&self, id: gizmo_core::entity::Entity) -> bool {
        self.selection.entities.contains(&id)
    }

    pub fn select_exclusive(&mut self, id: gizmo_core::entity::Entity) {
        self.selection.entities.clear();
        self.selection.entities.insert(id);
        self.selection.primary = Some(id);
    }

    pub fn toggle_selection(&mut self, id: gizmo_core::entity::Entity) {
        if self.selection.entities.contains(&id) {
            self.selection.entities.remove(&id);
            if self.selection.primary == Some(id) {
                self.selection.primary = self.selection.entities.iter().next().copied();
            }
        } else {
            self.selection.entities.insert(id);
            self.selection.primary = Some(id);
        }
    }

    pub fn unselect_entity(&mut self, id: gizmo_core::entity::Entity) {
        if self.selection.entities.contains(&id) {
            self.selection.entities.remove(&id);
            if self.selection.primary == Some(id) {
                self.selection.primary = self.selection.entities.iter().next().copied();
            }
        }
    }

    pub fn clear_selection(&mut self) {
        self.selection.entities.clear();
        self.selection.primary = None;
        self.scene.gizmo_original_transforms.clear();
    }

    pub fn toggle_play(&mut self) {
        self.mode = match self.mode {
            EditorMode::Edit => {
                self.play_start_request = true;
                EditorMode::Play
            }
            EditorMode::Play => {
                self.play_stop_request = true;
                EditorMode::Edit
            }
            EditorMode::Paused => EditorMode::Play,
        };
    }

    pub fn toggle_pause(&mut self) {
        self.mode = match self.mode {
            EditorMode::Play => EditorMode::Paused,
            EditorMode::Paused => EditorMode::Play,
            other => other,
        };
    }

    pub fn is_playing(&self) -> bool {
        self.mode == EditorMode::Play
    }

    pub fn is_editing(&self) -> bool {
        self.mode == EditorMode::Edit
    }

    pub fn reset_layout(&mut self) {
        self.dock_state = create_default_dock_state();
    }

    pub fn log_info(&mut self, msg: &str) {
        gizmo_core::logger::log_message(gizmo_core::logger::LogLevel::Info, msg.to_string(), file!(), line!());
    }

    pub fn log_warning(&mut self, msg: &str) {
        gizmo_core::logger::log_message(gizmo_core::logger::LogLevel::Warning, msg.to_string(), file!(), line!());
    }

    pub fn log_error(&mut self, msg: &str) {
        gizmo_core::logger::log_message(gizmo_core::logger::LogLevel::Error, msg.to_string(), file!(), line!());
        self.last_error = Some(msg.to_string());
    }

    pub fn save_layout(&mut self) {
        if let Ok(json) = serde_json::to_string(&self.dock_state) {
            if let Err(e) = std::fs::write("editor_layout.json", json) {
                self.log_error(&format!("Layout kaydedilemedi: {}", e));
            } else {
                self.log_info("Pencere düzeni başarıyla kaydedildi.");
            }
        } else {
            self.log_error("Layout serialize edilemedi.");
        }
    }

    pub fn load_layout() -> Option<egui_dock::DockState<EditorTab>> {
        if let Ok(content) = std::fs::read_to_string("editor_layout.json") {
            if let Ok(dock) = serde_json::from_str(&content) {
                return Some(dock);
            } else {
                gizmo_core::logger::log_message(
                    gizmo_core::logger::LogLevel::Error, 
                    "editor_layout.json parse hatasi!".to_string(), 
                    file!(), 
                    line!()
                );
            }
        }
        None
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

fn create_default_dock_state() -> egui_dock::DockState<EditorTab> {
    use egui_dock::{DockState, NodeIndex};
    // Root tab "Scene View" and "Game View" in the same area
    let mut state = DockState::new(vec![EditorTab::SceneView, EditorTab::GameView]);
    let surface = state.main_surface_mut();

    // Right Split for Inspector & Hierarchy
    let [main, right_panel] =
        surface.split_right(NodeIndex::root(), 0.8, vec![EditorTab::Inspector]);
    let [_hierarchy, _inspector] =
        surface.split_above(right_panel, 0.4, vec![EditorTab::Hierarchy]);

    // Bottom Split for Asset Browser
    let [_main, _bottom] = surface.split_below(
        main,
        0.7,
        vec![EditorTab::AssetBrowser, EditorTab::Console],
    );

    state
}
