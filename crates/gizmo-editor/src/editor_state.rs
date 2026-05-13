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
    Settings,
    ScriptEditor,
    Profiler,
}

/// Gizmo aracı modu
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GizmoMode {
    Select,
    Translate,
    Rotate,
    Scale,
}

/// Build hedef işletim sistemi
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditorMode {
    /// Düzenleme modu — fizik durur, entity'ler serbestçe manipüle edilir
    Edit,
    /// Oyun modu — fizik ve scriptler çalışır
    Play,
    /// Duraklatılmış oyun modu
    Paused,
}

// --- Alt Durum Yapilari ---
#[derive(Default)]
pub struct CameraState {
    pub look_delta: Option<gizmo_math::Vec2>,
    pub pan_delta: Option<gizmo_math::Vec2>,
    pub orbit_delta: Option<gizmo_math::Vec2>,
    pub scroll_delta: Option<f32>,
    pub view: Option<gizmo_math::Mat4>,
    pub proj: Option<gizmo_math::Mat4>,
    pub focus_target: Option<gizmo_math::Vec3>,
    pub bookmarks: [Option<(gizmo_math::Vec3, f32, f32)>; 10],
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
        Self {
            request: false,
            target: BuildTarget::Native,
            is_building: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            logs_rx: None,
            cached_logs: Vec::new(),
            start_time: None,
        }
    }
}

pub struct AssetBrowserState {
    pub filter: String,
    pub root: String,
    pub show: bool,
    pub workspace_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    pub cached_dir: Option<(
        String,
        std::time::Instant,
        Vec<(std::path::PathBuf, String, bool)>,
    )>,
}
impl Default for AssetBrowserState {
    fn default() -> Self {
        Self {
            filter: String::new(),
            root: "demo/assets".to_string(),
            show: true,
            workspace_rx: None,
            cached_dir: None,
        }
    }
}

#[derive(Default)]
pub struct SceneState {
    pub save_request: Option<String>,
    pub load_request: Option<String>,
    pub clear_request: bool,
    pub rebuild_navmesh_request: bool,
    pub load_confirm_dialog: Option<String>,
    pub gizmo_original_transforms:
        std::collections::HashMap<gizmo_core::entity::Entity, gizmo_physics::components::Transform>,
}

#[derive(Default)]
pub struct SelectionState {
    pub entities: std::collections::HashSet<gizmo_core::entity::Entity>,
    pub primary: Option<gizmo_core::entity::Entity>,
    pub rubber_band_start: Option<gizmo_math::Vec2>,
    pub rubber_band_current: Option<gizmo_math::Vec2>,
    pub rubber_band_request: Option<(gizmo_math::Vec2, gizmo_math::Vec2)>,
}

#[derive(Default)]
pub struct ScriptEditorState {
    pub open: bool,
    pub active_path: Option<String>,
    pub active_content: Option<String>,
    pub is_dirty: bool,
    pub pending_clear_confirm: bool,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum ConsoleMode {
    EngineLogs,
    BuildOutput,
}

pub struct ConsoleState {
    pub mode: ConsoleMode,
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
            mode: ConsoleMode::EngineLogs,
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
    pub shading_mode: u32,
    /// FXAA Anti-Aliasing açık/kapalı durumu
    pub fxaa_enabled: bool,
    
    // Post-Processing Settings
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub exposure: f32,
    pub vignette: f32,
    pub chromatic_aberration: f32,

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
    pub transform_gizmo: transform_gizmo_egui::Gizmo,

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
    pub prefab_load_request: Option<(
        String,
        Option<gizmo_core::entity::Entity>,
        Option<gizmo_math::Vec3>,
    )>,
    pub spawn_request: Option<String>,
    pub spawn_asset_request: Option<String>,
    pub spawn_asset_position: Option<gizmo_math::Vec3>,
    pub gltf_load_request: Option<(String, Option<gizmo_math::Vec3>)>,
    pub pending_async_gltfs: std::collections::HashMap<String, gizmo_math::Vec3>,
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

    pub pending_dialog_rx:
        Option<std::sync::Mutex<std::sync::mpsc::Receiver<(bool, Option<String>)>>>,

    /// Play/Stop modu için in-memory sahne yedeği.
    /// Play'e basıldığında `Some(snapshot)`, Stop'ta `None` olur.
    pub play_snapshot: Option<gizmo_scene::SceneSnapshot>,

    pub pending_json_updates: Vec<(
        gizmo_core::entity::Entity,
        fn(
            &mut gizmo_core::World,
            gizmo_core::entity::Entity,
            serde_json::Value,
        ) -> Result<(), String>,
        serde_json::Value,
    )>,
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
            shading_mode: 0,
            fxaa_enabled: true,

            bloom_intensity: 0.8,
            bloom_threshold: 0.85,
            exposure: 1.0,
            vignette: 0.2,
            chromatic_aberration: 0.005,

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
            transform_gizmo: transform_gizmo_egui::Gizmo::default(),

            camera: CameraState::default(),
            build: BuildState::default(),
            assets: AssetBrowserState::default(),
            scene: SceneState::default(),
            selection: SelectionState::default(),
            console: ConsoleState::default(),
            script: ScriptEditorState::default(),
            prefs,

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
            pending_async_gltfs: std::collections::HashMap::new(),
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

            play_snapshot: None,

            pending_json_updates: Vec::new(),
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
        self.selection.rubber_band_start = None;
        self.selection.rubber_band_current = None;
        self.selection.rubber_band_request = None;
        self.scene.gizmo_original_transforms.clear();
    }

    /// Play/Stop geçişi yapar.
    /// Edit → Play: Sahne snapshot'ı alınması için `play_start_request` set edilir.
    /// Play veya Paused → Edit: Sahne geri yüklenmesi için `play_stop_request` set edilir.
    pub fn toggle_play(&mut self) {
        self.mode = match self.mode {
            EditorMode::Edit => {
                self.play_start_request = true;
                EditorMode::Play
            }
            EditorMode::Play | EditorMode::Paused => {
                self.play_stop_request = true;
                EditorMode::Edit
            }
        };
    }

    pub fn toggle_pause(&mut self) {
        self.mode = match self.mode {
            EditorMode::Play => EditorMode::Paused,
            EditorMode::Paused => EditorMode::Play,
            other => other,
        };
    }

    /// Oyun aktif olarak çalışıyor mu? (Sadece Play, Paused değil)
    pub fn is_playing(&self) -> bool {
        self.mode == EditorMode::Play
    }

    /// Oyun oturumu aktif mi? (Play veya Paused — snapshot hâlâ hayatta)
    pub fn is_in_play_session(&self) -> bool {
        matches!(self.mode, EditorMode::Play | EditorMode::Paused)
    }

    pub fn is_editing(&self) -> bool {
        self.mode == EditorMode::Edit
    }

    pub fn is_paused(&self) -> bool {
        self.mode == EditorMode::Paused
    }

    // --- Post-Process Validation ---
    /// Post-process değerlerini güvenli aralıklara sıkıştırır.
    /// Render pipeline'a geçmeden önce çağrılmalıdır.
    pub fn validate_post_process(&mut self) {
        self.bloom_intensity = self.bloom_intensity.clamp(0.0, 5.0);
        self.bloom_threshold = self.bloom_threshold.clamp(0.0, 10.0);
        self.exposure = self.exposure.clamp(0.01, 20.0);
        self.vignette = self.vignette.clamp(0.0, 1.0);
        self.chromatic_aberration = self.chromatic_aberration.clamp(0.0, 0.1);
    }

    pub fn reset_layout(&mut self) {
        self.dock_state = create_default_dock_state();
    }

    pub fn log_info(&mut self, msg: &str) {
        gizmo_core::logger::log_message(
            gizmo_core::logger::LogLevel::Info,
            msg.to_string(),
            file!(),
            line!(),
        );
    }

    pub fn log_warning(&mut self, msg: &str) {
        gizmo_core::logger::log_message(
            gizmo_core::logger::LogLevel::Warning,
            msg.to_string(),
            file!(),
            line!(),
        );
    }

    pub fn log_error(&mut self, msg: &str) {
        gizmo_core::logger::log_message(
            gizmo_core::logger::LogLevel::Error,
            msg.to_string(),
            file!(),
            line!(),
        );
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
                    line!(),
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
    let [_main, _bottom] =
        surface.split_below(main, 0.7, vec![EditorTab::AssetBrowser, EditorTab::Console]);

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Yardımcı ===
    fn make_entity(id: u32) -> gizmo_core::entity::Entity {
        gizmo_core::entity::Entity::new(id, 0)
    }

    // =========================================================
    //  Post-Process Defaults
    // =========================================================
    #[test]
    fn test_post_process_defaults() {
        let state = EditorState::new();
        assert_eq!(state.bloom_intensity, 0.8);
        assert_eq!(state.bloom_threshold, 0.85);
        assert_eq!(state.exposure, 1.0);
        assert_eq!(state.vignette, 0.2);
        assert_eq!(state.chromatic_aberration, 0.005);
    }

    #[test]
    fn test_post_process_validation_clamps() {
        let mut state = EditorState::new();
        state.bloom_intensity = -5.0;
        state.bloom_threshold = 999.0;
        state.exposure = -1.0;
        state.vignette = 2.0;
        state.chromatic_aberration = 0.5;
        state.validate_post_process();
        assert_eq!(state.bloom_intensity, 0.0);
        assert_eq!(state.bloom_threshold, 10.0);
        assert_eq!(state.exposure, 0.01);
        assert_eq!(state.vignette, 1.0);
        assert_eq!(state.chromatic_aberration, 0.1);
    }

    #[test]
    fn test_post_process_validation_noop_on_valid() {
        let mut state = EditorState::new();
        let orig_bloom = state.bloom_intensity;
        let orig_exposure = state.exposure;
        state.validate_post_process();
        assert_eq!(state.bloom_intensity, orig_bloom);
        assert_eq!(state.exposure, orig_exposure);
    }

    // =========================================================
    //  Selection API
    // =========================================================
    #[test]
    fn test_select_exclusive() {
        let mut state = EditorState::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        state.select_exclusive(e1);
        assert!(state.is_selected(e1));
        assert_eq!(state.selection.primary, Some(e1));
        // İkinci obje seçildiğinde birincisi çıkmalı
        state.select_exclusive(e2);
        assert!(!state.is_selected(e1));
        assert!(state.is_selected(e2));
        assert_eq!(state.selection.primary, Some(e2));
        assert_eq!(state.selection.entities.len(), 1);
    }

    #[test]
    fn test_toggle_selection_add_and_remove() {
        let mut state = EditorState::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        // Ekle
        state.toggle_selection(e1);
        assert!(state.is_selected(e1));
        assert_eq!(state.selection.primary, Some(e1));
        // İkincisini de ekle
        state.toggle_selection(e2);
        assert!(state.is_selected(e1));
        assert!(state.is_selected(e2));
        assert_eq!(state.selection.primary, Some(e2));
        assert_eq!(state.selection.entities.len(), 2);
        // Birincisini çıkar
        state.toggle_selection(e1);
        assert!(!state.is_selected(e1));
        assert!(state.is_selected(e2));
    }

    #[test]
    fn test_toggle_selection_removes_primary_reassigns() {
        let mut state = EditorState::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        state.toggle_selection(e1);
        state.toggle_selection(e2);
        // e2 primary, onu çıkar → primary e1'e düşmeli
        state.toggle_selection(e2);
        assert_eq!(state.selection.primary, Some(e1));
    }

    #[test]
    fn test_unselect_entity() {
        let mut state = EditorState::new();
        let e1 = make_entity(1);
        state.select_exclusive(e1);
        state.unselect_entity(e1);
        assert!(!state.is_selected(e1));
        assert_eq!(state.selection.primary, None);
        assert!(state.selection.entities.is_empty());
    }

    #[test]
    fn test_unselect_nonexistent_noop() {
        let mut state = EditorState::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        state.select_exclusive(e1);
        state.unselect_entity(e2); // e2 seçili değil
        assert!(state.is_selected(e1));
        assert_eq!(state.selection.primary, Some(e1));
    }

    #[test]
    fn test_clear_selection() {
        let mut state = EditorState::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        state.select_exclusive(e1);
        state.toggle_selection(e2);
        state.clear_selection();
        assert!(state.selection.entities.is_empty());
        assert_eq!(state.selection.primary, None);
        assert!(state.selection.rubber_band_start.is_none());
        assert!(state.selection.rubber_band_current.is_none());
        assert!(state.selection.rubber_band_request.is_none());
    }

    // =========================================================
    //  Play / Pause / Stop State Machine
    // =========================================================
    #[test]
    fn test_toggle_play_edit_to_play() {
        let mut state = EditorState::new();
        assert_eq!(state.mode, EditorMode::Edit);
        state.toggle_play();
        assert_eq!(state.mode, EditorMode::Play);
        assert!(state.play_start_request);
        assert!(!state.play_stop_request);
    }

    #[test]
    fn test_toggle_play_play_to_edit() {
        let mut state = EditorState::new();
        state.mode = EditorMode::Play;
        state.toggle_play();
        assert_eq!(state.mode, EditorMode::Edit);
        assert!(state.play_stop_request);
    }

    #[test]
    fn test_toggle_play_paused_to_edit() {
        let mut state = EditorState::new();
        state.mode = EditorMode::Paused;
        state.toggle_play();
        // Paused durumundan Stop'a basmak Edit'e dönmeli ve play_stop_request set etmeli
        assert_eq!(state.mode, EditorMode::Edit);
        assert!(state.play_stop_request);
    }

    #[test]
    fn test_toggle_pause() {
        let mut state = EditorState::new();
        state.mode = EditorMode::Play;
        state.toggle_pause();
        assert_eq!(state.mode, EditorMode::Paused);
        state.toggle_pause();
        assert_eq!(state.mode, EditorMode::Play);
    }

    #[test]
    fn test_toggle_pause_noop_in_edit() {
        let mut state = EditorState::new();
        state.toggle_pause();
        assert_eq!(state.mode, EditorMode::Edit);
    }

    #[test]
    fn test_full_play_cycle() {
        let mut state = EditorState::new();
        // Edit → Play
        state.toggle_play();
        assert!(state.is_playing());
        assert!(state.is_in_play_session());
        assert!(!state.is_editing());
        state.play_start_request = false; // Consume request

        // Play → Paused
        state.toggle_pause();
        assert!(!state.is_playing());
        assert!(state.is_in_play_session());
        assert!(state.is_paused());

        // Paused → Play (resume)
        state.toggle_pause();
        assert!(state.is_playing());

        // Play → Edit (stop)
        state.toggle_play();
        assert!(state.is_editing());
        assert!(!state.is_in_play_session());
        assert!(state.play_stop_request);
    }

    // =========================================================
    //  Mode Query Helpers
    // =========================================================
    #[test]
    fn test_is_playing_false_when_paused() {
        let mut state = EditorState::new();
        state.mode = EditorMode::Paused;
        assert!(!state.is_playing());
    }

    #[test]
    fn test_is_in_play_session_covers_paused() {
        let mut state = EditorState::new();
        state.mode = EditorMode::Paused;
        assert!(state.is_in_play_session());
    }

    #[test]
    fn test_is_in_play_session_false_in_edit() {
        let state = EditorState::new();
        assert!(!state.is_in_play_session());
    }

    // =========================================================
    //  GizmoMode
    // =========================================================
    #[test]
    fn test_gizmo_mode_default_translate() {
        let state = EditorState::new();
        assert_eq!(state.gizmo_mode, GizmoMode::Translate);
    }

    // =========================================================
    //  Logging
    // =========================================================
    #[test]
    fn test_log_error_sets_last_error() {
        let mut state = EditorState::new();
        assert!(state.last_error.is_none());
        state.log_error("test hata");
        assert_eq!(state.last_error.as_deref(), Some("test hata"));
    }

    #[test]
    fn test_log_info_does_not_set_last_error() {
        let mut state = EditorState::new();
        state.log_info("bilgi");
        assert!(state.last_error.is_none());
    }

    // =========================================================
    //  Dock / Tab
    // =========================================================
    #[test]
    fn test_default_dock_has_scene_view() {
        let state = EditorState::new();
        assert!(state.is_tab_open(&EditorTab::SceneView));
    }

    #[test]
    fn test_toggle_tab() {
        let mut state = EditorState::new();
        let has_profiler = state.is_tab_open(&EditorTab::Profiler);
        state.toggle_tab(EditorTab::Profiler);
        assert_ne!(has_profiler, state.is_tab_open(&EditorTab::Profiler));
        state.toggle_tab(EditorTab::Profiler);
        assert_eq!(has_profiler, state.is_tab_open(&EditorTab::Profiler));
    }

    #[test]
    fn test_open_tab_idempotent() {
        let mut state = EditorState::new();
        state.open_tab(EditorTab::Settings);
        assert!(state.is_tab_open(&EditorTab::Settings));
        // İkinci kez açmak duplicate tab yaratmamalı
        state.open_tab(EditorTab::Settings);
        let count = state.dock_state.iter_all_tabs().filter(|t| t.1 == &EditorTab::Settings).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_reset_layout() {
        let mut state = EditorState::new();
        // Bir tab kapat
        if state.is_tab_open(&EditorTab::Console) {
            state.toggle_tab(EditorTab::Console);
        }
        assert!(!state.is_tab_open(&EditorTab::Console));
        // Reset
        state.reset_layout();
        assert!(state.is_tab_open(&EditorTab::Console));
    }

    // =========================================================
    //  Default State Invariants
    // =========================================================
    #[test]
    fn test_initial_state_invariants() {
        let state = EditorState::new();
        assert_eq!(state.mode, EditorMode::Edit);
        assert!(!state.play_start_request);
        assert!(!state.play_stop_request);
        assert!(state.selection.entities.is_empty());
        assert!(state.selection.primary.is_none());
        assert!(state.despawn_requests.is_empty());
        assert!(state.duplicate_requests.is_empty());
        assert!(state.pending_async_gltfs.is_empty());
        assert!(state.play_snapshot.is_none());
        assert_eq!(state.status_message, "Hazır");
    }

    // =========================================================
    //  ConsoleState
    // =========================================================
    #[test]
    fn test_console_defaults() {
        let console = ConsoleState::default();
        assert_eq!(console.mode, ConsoleMode::EngineLogs);
        assert!(console.show_info);
        assert!(console.show_warn);
        assert!(console.show_error);
        assert!(console.filter_text.is_empty());
        assert!(console.cached_logs.is_empty());
        assert_eq!(console.last_version, 0);
    }

    // =========================================================
    //  BuildState
    // =========================================================
    #[test]
    fn test_build_state_defaults() {
        let build = BuildState::default();
        assert!(!build.request);
        assert_eq!(build.target, BuildTarget::Native);
        assert!(!build.is_building.load(std::sync::atomic::Ordering::Relaxed));
        assert!(build.logs_rx.is_none());
        assert!(build.cached_logs.is_empty());
        assert!(build.start_time.is_none());
    }

    // =========================================================
    //  CameraState
    // =========================================================
    #[test]
    fn test_camera_state_defaults() {
        let cam = CameraState::default();
        assert!(cam.look_delta.is_none());
        assert!(cam.pan_delta.is_none());
        assert!(cam.orbit_delta.is_none());
        assert!(cam.scroll_delta.is_none());
        assert!(cam.focus_target.is_none());
        assert!(cam.bookmarks.iter().all(|b| b.is_none()));
    }

    // =========================================================
    //  Enum Eq / Derive
    // =========================================================
    #[test]
    fn test_gizmo_mode_eq() {
        assert_eq!(GizmoMode::Select, GizmoMode::Select);
        assert_ne!(GizmoMode::Select, GizmoMode::Translate);
    }

    #[test]
    fn test_editor_mode_eq() {
        assert_eq!(EditorMode::Edit, EditorMode::Edit);
        assert_ne!(EditorMode::Edit, EditorMode::Play);
    }

    #[test]
    fn test_build_target_eq() {
        assert_eq!(BuildTarget::Native, BuildTarget::Native);
        assert_ne!(BuildTarget::Native, BuildTarget::Linux);
    }
}
