//! Editor State — Editörün global durumunu yönetir
use crate::prefs::EditorPrefs;
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

// god-file Tier 3 round-2 bölmesi: küçük UI-state tipleri state_types alt-modülünde
mod state_types;
pub use state_types::*;

#[non_exhaustive]
pub struct EditorState {
    pub post_process: PostProcessSettings,
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
    // Post-Processing Settings
    pub history: crate::history::History,

    // Panellerin görünürlüğü (asset browser hariç)
    pub show_hierarchy: bool,
    pub show_inspector: bool,
    pub show_toolbar: bool,
    pub settings_open: bool,
    pub show_colliders: bool,

    pub inspector_drag_original_transforms: std::collections::HashMap<gizmo_core::entity::Entity, gizmo_physics_core::Transform>,

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
    /// Spawn edilen entity'nin otomatik olarak bağlanacağı parent
    pub pending_child_parent: Option<gizmo_core::entity::Entity>,
    /// Spawn edilen entity'ye otomatik eklenecek bileşenler
    pub pending_child_components: Vec<String>,
    pub spawn_asset_request: Option<String>,
    pub spawn_asset_position: Option<gizmo_math::Vec3>,
    pub gltf_load_request: Option<(String, Option<gizmo_math::Vec3>)>,
    pub pending_async_gltfs: std::collections::HashMap<String, gizmo_math::Vec3>,
    pub reparent_request: Option<(gizmo_core::entity::Entity, gizmo_core::entity::Entity)>,
    pub unparent_request: Option<gizmo_core::entity::Entity>,
    pub add_component_request: Option<(gizmo_core::entity::Entity, String)>,
    pub remove_component_request: Option<(gizmo_core::entity::Entity, String)>,

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
    pub clipboard_entities: Vec<gizmo_core::entity::Entity>,

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

    /// Fighting game HUD durumu (health bar, round, timer)
    pub fight_hud: FightHudState,
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
            post_process: PostProcessSettings::default(),
            history: crate::history::History::new(prefs.max_history),

            show_hierarchy: true,
            show_inspector: true,
            show_toolbar: true,
            settings_open: false,
            show_colliders: false,

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

            inspector_drag_original_transforms: std::collections::HashMap::new(),

            despawn_requests: Vec::new(),
            generate_terrain_requests: Vec::new(),
            duplicate_requests: Vec::new(),
            toggle_visibility_requests: Vec::new(),

            prefab_save_request: None,
            prefab_load_request: None,
            spawn_request: None,
            pending_child_parent: None,
            pending_child_components: Vec::new(),
            spawn_asset_request: None,
            spawn_asset_position: None,
            gltf_load_request: None,
            pending_async_gltfs: std::collections::HashMap::new(),
            reparent_request: None,
            unparent_request: None,
            add_component_request: None,
            remove_component_request: None,

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
            clipboard_entities: Vec::new(),

            pending_dialog_rx: None,

            play_snapshot: None,

            pending_json_updates: Vec::new(),

            fight_hud: FightHudState::default(),
        }
    }

    // --- Post-Process Validation ---
    /// Post-process değerlerini güvenli aralıklara sıkıştırır.
    /// Render pipeline'a geçmeden önce çağrılmalıdır.
    pub fn validate_post_process(&mut self) {
        self.post_process.bloom_intensity = self.post_process.bloom_intensity.clamp(0.0, 5.0);
        self.post_process.bloom_threshold = self.post_process.bloom_threshold.clamp(0.0, 10.0);
        self.post_process.exposure = self.post_process.exposure.clamp(0.01, 20.0);
        self.post_process.vignette = self.post_process.vignette.clamp(0.0, 1.0);
        self.post_process.chromatic_aberration = self.post_process.chromatic_aberration.clamp(0.0, 0.1);
    }
}

// EditorState'in impl'i domain'lere göre bölündü (god-object → kohezyonlu modüller).
// Struct + alanlar + new() + validate_post_process burada; metodlar kardeş modüllerde.
mod console;
mod layout;
mod play_mode;
mod selection;

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

    // 1. Split right for Inspector (takes 25% of screen width)
    let [main, _inspector] =
        surface.split_right(NodeIndex::root(), 0.75, vec![EditorTab::Inspector]);

    // 2. Split left for Hierarchy (takes 20% of the remaining 75% width)
    let [_hierarchy, center] =
        surface.split_left(main, 0.20, vec![EditorTab::Hierarchy]);

    // 3. Split bottom of the center area for Asset Browser and Console
    let [_scene, _bottom] =
        surface.split_below(center, 0.65, vec![EditorTab::AssetBrowser, EditorTab::Console]);

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
        assert_eq!(state.post_process.bloom_intensity, 0.8);
        assert_eq!(state.post_process.bloom_threshold, 0.85);
        assert_eq!(state.post_process.exposure, 1.0);
        assert_eq!(state.post_process.vignette, 0.2);
        assert_eq!(state.post_process.chromatic_aberration, 0.005);
    }

    #[test]
    fn test_post_process_validation_clamps() {
        let mut state = EditorState::new();
        state.post_process.bloom_intensity = -5.0;
        state.post_process.bloom_threshold = 999.0;
        state.post_process.exposure = -1.0;
        state.post_process.vignette = 2.0;
        state.post_process.chromatic_aberration = 0.5;
        state.validate_post_process();
        assert_eq!(state.post_process.bloom_intensity, 0.0);
        assert_eq!(state.post_process.bloom_threshold, 10.0);
        assert_eq!(state.post_process.exposure, 0.01);
        assert_eq!(state.post_process.vignette, 1.0);
        assert_eq!(state.post_process.chromatic_aberration, 0.1);
    }

    #[test]
    fn test_post_process_validation_noop_on_valid() {
        let mut state = EditorState::new();
        let orig_bloom = state.post_process.bloom_intensity;
        let orig_exposure = state.post_process.exposure;
        state.validate_post_process();
        assert_eq!(state.post_process.bloom_intensity, orig_bloom);
        assert_eq!(state.post_process.exposure, orig_exposure);
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
