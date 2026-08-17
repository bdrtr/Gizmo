//! Editor state — the editor's global state lives here.
use crate::prefs::EditorPrefs;
use serde::{Deserialize, Serialize};

#[cfg(target_arch = "wasm32")]
use web_time::Instant;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

// god-file Tier 3 round-2 bölmesi: küçük UI-state tipleri state_types alt-modülünde
mod state_types;
pub use state_types::*;

/// Everything the editor knows between frames.
///
/// One struct rather than a dozen resources, because egui draws the whole UI in one pass and
/// every panel needs to see what the others decided. It is `#[non_exhaustive]`: build one with
/// [`EditorState::new`] and adjust fields, so that adding state here is not a breaking change.
///
/// The **request** fields are the pattern worth knowing. A panel cannot mutate the world while
/// egui is drawing it, so an action — delete, duplicate, reparent, add component — is recorded
/// here and carried out afterwards by the host's systems, which drain it. A request that nobody
/// drains is a menu item that silently does nothing, which is exactly the class of bug the
/// exhaustive `SpawnKind` match in `gizmo-studio` was added to prevent.
#[non_exhaustive]
pub struct EditorState {
    /// Post-process settings the viewport renders with — bloom, exposure, FXAA and the rest.
    pub post_process: PostProcessSettings,
    /// Whether the editor UI is drawn at all. `false` leaves the game rendering on its own.
    pub open: bool,
    /// Edit, play or paused.
    pub mode: EditorMode,
    /// Which transform gizmo is active: translate, rotate or scale.
    pub gizmo_mode: GizmoMode,

    /// Raised by the toolbar's ▶; the host consumes it, takes the scene snapshot and starts
    /// playing.
    pub play_start_request: bool,
    /// Raised by ■; the host restores the snapshot and stops.
    pub play_stop_request: bool,

    /// Set when a viewport click needs picking; cleared once the raycast has run.
    pub do_raycast: bool,
    /// The mouse in normalised device coordinates for that pick, `None` outside the viewport.
    pub mouse_ndc: Option<gizmo_math::Vec2>,
    /// Whether the gizmo works in the entity's local space rather than the world's.
    pub gizmo_local_space: bool,
    /// Which shading mode the viewport draws in — lit, unlit, wireframe and the debug channels.
    pub shading_mode: u32,
    /// Undo/redo. Every editor action that changes the world is pushed here rather than applied
    /// directly, which is what makes Ctrl+Z whole.
    pub history: crate::history::History,

    // Panel visibility (the asset browser keeps its own, in `assets`).
    /// Is the hierarchy panel shown?
    pub show_hierarchy: bool,
    /// Is the inspector shown?
    pub show_inspector: bool,
    /// Is the toolbar shown?
    pub show_toolbar: bool,
    /// Is the settings window open?
    pub settings_open: bool,
    /// Draw collider outlines over the scene?
    pub show_colliders: bool,

    /// The transforms as they were when the current inspector drag started, so a whole drag
    /// becomes ONE undo entry rather than one per frame.
    pub inspector_drag_original_transforms: std::collections::HashMap<gizmo_core::entity::Entity, gizmo_physics_core::Transform>,

    // Other global UI state.
    /// The hierarchy panel's search box.
    pub hierarchy_filter: String,
    /// Hide the editor's own entities (camera, grid, lights) from the hierarchy.
    pub hide_editor_entities: bool,
    /// Is the "add component" popup open?
    pub add_component_open: bool,
    /// The last error to show in the status bar, if any.
    pub last_error: Option<String>,
    /// The status bar's current message.
    pub status_message: String,
    /// Path of the scene being edited; empty until it has been saved once.
    pub scene_path: String,
    /// Whether the scene has changes that are not on disk.
    pub has_unsaved_changes: bool,
    /// The asset currently being dragged out of the browser, if any.
    pub dragged_asset: Option<String>,
    /// The transform gizmo widget itself, which owns its own interaction state.
    pub transform_gizmo: transform_gizmo_egui::Gizmo,

    // Nested state, one struct per area.
    /// The editor camera's own state.
    pub camera: CameraState,
    /// The animation timeline's authoring state (drag + selection).
    pub anim_edit: AnimationEditState,
    /// The entity's clips as they were when the current keyframe drag started, so a whole drag
    /// becomes ONE undo entry instead of one per frame — the same shape the transform gizmo uses
    /// with `gizmo_original_transforms`.
    pub anim_drag_original: Option<std::sync::Arc<[gizmo_renderer::AnimationClip]>>,
    /// Export/build state: the target, the progress and the last result.
    pub build: BuildState,
    /// The asset browser's own state: where it is looking and what is filtered.
    pub assets: AssetBrowserState,
    /// Scene-level UI state — the save/load prompts and what they are waiting on.
    pub scene: SceneState,
    /// What is selected, and the outline drawn around it.
    pub selection: SelectionState,
    /// The console panel: its buffer, its filters and its input line.
    pub console: ConsoleState,
    /// The script editor: the open file, its text and whether it is dirty.
    pub script: ScriptEditorState,

    /// Preferences that persist between sessions, on disk.
    pub prefs: EditorPrefs,

    // Requests. The UI cannot mutate the world while it is drawing, so every action it offers
    // becomes an entry here and a system carries it out afterwards. Each is drained by its
    // handler, which is why they are plain vectors rather than events.
    /// Entities the UI asked to delete.
    pub despawn_requests: Vec<gizmo_core::entity::Entity>,
    /// Entities the UI asked to generate terrain for.
    pub generate_terrain_requests: Vec<gizmo_core::entity::Entity>,
    /// Entities the UI asked to duplicate.
    pub duplicate_requests: Vec<gizmo_core::entity::Entity>,
    /// Entities whose visibility the UI asked to toggle.
    pub toggle_visibility_requests: Vec<gizmo_core::entity::Entity>,

    /// "Save this entity as a prefab at this path."
    pub prefab_save_request: Option<(gizmo_core::entity::Entity, String)>,
    /// "Load this prefab" — the path, an optional parent to attach it to, and an optional
    /// position to place it at.
    pub prefab_load_request: Option<(
        String,
        Option<gizmo_core::entity::Entity>,
        Option<gizmo_math::Vec3>,
    )>,
    /// Which primitive the `➕ Add` menu asked to spawn.
    pub spawn_request: Option<SpawnKind>,
    /// The parent a spawned entity is automatically attached to
    pub pending_child_parent: Option<gizmo_core::entity::Entity>,
    /// The components automatically added to a spawned entity
    pub pending_child_components: Vec<String>,
    /// Entities that should become children of the entity the next spawn creates — the other half
    /// of "group the selection", which used to create the group and then leave the selection
    /// exactly where it was.
    pub pending_group_members: Vec<gizmo_core::entity::Entity>,
    /// An asset dragged into the viewport, waiting to be spawned.
    pub spawn_asset_request: Option<String>,
    /// Where that asset was dropped, in world space.
    pub spawn_asset_position: Option<gizmo_math::Vec3>,
    /// "Load this glTF", with an optional position to place it at.
    pub gltf_load_request: Option<(String, Option<gizmo_math::Vec3>)>,
    /// glTF imports handed to the background loader, and where each should land when it arrives.
    pub pending_async_gltfs: std::collections::HashMap<String, gizmo_math::Vec3>,
    /// "Make the first entity a child of the second."
    pub reparent_request: Option<(gizmo_core::entity::Entity, gizmo_core::entity::Entity)>,
    /// "Detach this entity from its parent."
    pub unparent_request: Option<gizmo_core::entity::Entity>,
    /// "Add this component, by name, to this entity."
    pub add_component_request: Option<(gizmo_core::entity::Entity, String)>,
    /// "Remove this component, by name, from this entity."
    pub remove_component_request: Option<(gizmo_core::entity::Entity, String)>,

    // The two viewports. Each panel reports where and how big it is, and the host renders into a
    // texture of that size — which is why a size of `None` means "not laid out yet", not "zero".
    /// Is the scene view tab visible this frame?
    pub scene_view_visible: bool,
    /// Is the game view tab visible this frame?
    pub game_view_visible: bool,
    /// Where the scene view sits on screen, for turning a click into a ray.
    pub scene_view_rect: Option<egui::Rect>,
    /// Where the game view sits on screen.
    pub game_view_rect: Option<egui::Rect>,
    /// The scene view's size in points, which the render target follows.
    pub scene_view_size: Option<egui::Vec2>,
    /// The game view's size in points.
    pub game_view_size: Option<egui::Vec2>,
    /// The egui texture the scene view paints — the render target's other end.
    pub scene_texture_id: Option<egui::TextureId>,
    /// The egui texture the game view paints.
    pub game_texture_id: Option<egui::TextureId>,
    /// The dock layout: which tabs exist and how they are arranged. Persisted between sessions.
    pub dock_state: egui_dock::DockState<EditorTab>,

    /// Debug boxes to draw this frame: position, rotation, half-extents and colour.
    pub debug_draw_requests: Vec<(
        gizmo_math::Vec3,
        gizmo_math::Quat,
        gizmo_math::Vec3,
        gizmo_math::Vec4,
    )>,
    /// Debug entities with a lifetime: seconds remaining, and the entity id to despawn.
    pub debug_spawned_entities: Vec<(f32, u32)>,
    /// What Ctrl+C copied — the entities a paste will duplicate.
    pub clipboard_entities: Vec<gizmo_core::entity::Entity>,

    /// The channel a native file dialog answers on. It runs on its own thread, so the frame that
    /// opened it cannot block: the result is picked up whenever it arrives.
    pub pending_dialog_rx:
        Option<std::sync::Mutex<std::sync::mpsc::Receiver<(bool, Option<String>)>>>,

    /// The in-memory scene backup behind play/stop.
    /// `Some(snapshot)` once play is pressed, `None` on stop.
    pub play_snapshot: Option<gizmo_scene::SceneSnapshot>,

    /// Component edits the inspector made as JSON, each with the function that applies it.
    ///
    /// The inspector edits a component it does not know the type of by round-tripping it through
    /// `serde_json`; the function pointer is what turns that value back into a typed write.
    pub pending_json_updates: Vec<(
        gizmo_core::entity::Entity,
        fn(
            &mut gizmo_core::World,
            gizmo_core::entity::Entity,
            serde_json::Value,
        ) -> Result<(), String>,
        serde_json::Value,
    )>,

    /// The fighting-game HUD's state: health bars, round counter and timer.
    pub fight_hud: FightHudState,

    /// When the status bar last read this process's resident set size, and what it read.
    ///
    /// Cached because the reading is a file read and a parse; the bar samples it once a second
    /// rather than sixty times. `rss_bytes` stays `None` where RSS cannot be measured at all, and
    /// the row is then absent rather than showing a zero it did not measure.
    pub rss_sampled_at: Option<Instant>,
    /// The resident set size last read, in bytes. `None` where RSS cannot be measured.
    pub rss_bytes: Option<u64>,
}

impl EditorState {
    /// The editor's starting state, with preferences and the dock layout read from disk.
    ///
    /// Both reads touch the filesystem — `editor_prefs.toml` from the config directory and
    /// `editor_layout.json` from the working directory — so a test that cares about the dock
    /// should overwrite `dock_state` with `create_default_dock_state()` rather than inherit
    /// whatever the developer's machine has.
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
            anim_edit: AnimationEditState::default(),
            anim_drag_original: None,
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
            pending_group_members: Vec::new(),
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

            rss_sampled_at: None,
            rss_bytes: None,
        }
    }

    // --- Post-Process Validation ---
    /// Clamps the post-process values into safe ranges.
    /// Must be called before they reach the render pipeline.
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

pub(crate) fn create_default_dock_state() -> egui_dock::DockState<EditorTab> {
    use egui_dock::{DockState, NodeIndex};
    
    // Root tab "Scene View" and "Game View" in the same area
    let mut state = DockState::new(vec![EditorTab::SceneView, EditorTab::GameView]);
    let surface = state.main_surface_mut();

    // 1. Split right for Inspector (takes 25% of screen width)
    let [main, _inspector] =
        surface.split_right(NodeIndex::root(), 0.75, vec![EditorTab::Inspector]);

    // 2. Hierarchy takes the left 20%; `center` is what remains.
    //
    // The binding order is load-bearing and was wrong here for a long time. `Tree::split_*` returns
    // `[old_node, new_node]` (egui_dock `tree/mod.rs`), NOT `[left, right]` — so the node that
    // keeps the existing tabs comes first regardless of which side it ends up on. Written as
    // `[_hierarchy, center]`, `center` was bound to the freshly-created Hierarchy leaf, and the
    // split below then hung the asset browser under the hierarchy: a 175 px column in the bottom
    // left, where a three-column asset browser cannot be read at all.
    let [center, _hierarchy] = surface.split_left(main, 0.20, vec![EditorTab::Hierarchy]);

    // 3. The bottom dock, under the VIEWPORT: assets, console and profiler as tabs, the way the
    //    prototype arranges them. 0.78 leaves the strip roughly the prototype's 218 px.
    //
    //    The profiler was previously in no default layout at all — `Window ▸ Profiler` calls
    //    `push_to_first_leaf`, which walks nodes in index order and lands on the Inspector, so it
    //    opened on the right next to the entity fields.
    let [_scene, _bottom] = surface.split_below(
        center,
        0.78,
        vec![
            EditorTab::AssetBrowser,
            EditorTab::Animation,
            EditorTab::Console,
            EditorTab::Profiler,
        ],
    );

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
    /// The bottom dock sits under the VIEWPORT, not under the hierarchy.
    ///
    /// This is the assertion the four existing dock tests were missing. Every one of them asks
    /// only "is this tab open somewhere" — which stayed true the entire time the asset browser was
    /// crammed into a 175 px column in the bottom-left corner, because a reversed destructuring of
    /// `split_left`'s `[old, new]` return had hung it off the hierarchy node. Placement is the
    /// thing that was wrong, so placement is the thing to assert.
    ///
    /// Calls `create_default_dock_state()` directly rather than `EditorState::new()`: `new()` goes
    /// through `load_layout()`, which reads a cwd-relative `editor_layout.json` that a test cannot
    /// control and a developer might have.
    #[test]
    fn the_bottom_dock_hangs_off_the_viewport_not_the_hierarchy() {
        let state = create_default_dock_state();
        let assets = state.find_tab(&EditorTab::AssetBrowser).expect("asset browser");
        let scene = state.find_tab(&EditorTab::SceneView).expect("scene view");
        let hierarchy = state.find_tab(&EditorTab::Hierarchy).expect("hierarchy");

        assert_eq!(
            assets.node.parent(),
            scene.node.parent(),
            "the asset browser must share a parent with the viewport — that is what puts it \
             underneath it"
        );
        assert_ne!(
            assets.node.parent(),
            hierarchy.node.parent(),
            "the asset browser is hanging off the hierarchy again: `split_*` returns [old, new], \
             so binding the new node to `center` puts the whole bottom dock in the left column"
        );
    }

    /// The profiler is in the default layout, in the bottom dock with the other two.
    ///
    /// It used to be in no layout at all: `Window ▸ Profiler` calls `push_to_first_leaf`, which
    /// takes the first leaf in index order — the Inspector — so a performance panel opened on top
    /// of the entity fields.
    #[test]
    fn the_profiler_ships_in_the_bottom_dock() {
        let state = create_default_dock_state();
        let profiler = state.find_tab(&EditorTab::Profiler).expect("profiler is in the layout");
        let assets = state.find_tab(&EditorTab::AssetBrowser).expect("asset browser");
        assert_eq!(profiler.node, assets.node, "the profiler belongs in the same leaf as the assets");
    }

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
        assert!(cam.view_request.is_none());
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
