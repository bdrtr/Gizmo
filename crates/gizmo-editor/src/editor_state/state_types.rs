use super::*;

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Hash, Debug)]
#[non_exhaustive]
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
#[non_exhaustive]
pub enum GizmoMode {
    Select,
    Translate,
    Rotate,
    Scale,
}

/// Build hedef işletim sistemi
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
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
#[non_exhaustive]
pub enum EditorMode {
    /// Düzenleme modu — fizik durur, entity'ler serbestçe manipüle edilir
    Edit,
    /// Oyun modu — fizik ve scriptler çalışır
    Play,
    /// Duraklatılmış oyun modu
    Paused,
}

// --- Alt Durum Yapilari ---
#[derive(Default, Debug)]
#[non_exhaustive]
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

#[derive(Debug)]
#[non_exhaustive]
pub struct BuildState {
    pub request: bool,
    pub target: BuildTarget,
    pub is_building: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub logs_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    pub cached_logs: Vec<(String, egui::Color32)>,
    pub start_time: Option<Instant>,
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

#[derive(Debug)]
#[non_exhaustive]
pub struct AssetBrowserState {
    pub filter: String,
    pub root: String,
    pub show: bool,
    pub workspace_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    pub cached_dir: Option<(
        String,
        Instant,
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

#[derive(Default, Debug)]
#[non_exhaustive]
pub struct SceneState {
    pub save_request: Option<String>,
    pub load_request: Option<String>,
    pub clear_request: bool,
    pub rebuild_navmesh_request: bool,
    pub request_save_dialog: bool,
    pub load_confirm_dialog: Option<String>,
    pub gizmo_original_transforms:
        std::collections::HashMap<gizmo_core::entity::Entity, gizmo_physics_core::Transform>,
}

#[derive(Default, Debug)]
#[non_exhaustive]
pub struct SelectionState {
    pub entities: std::collections::HashSet<gizmo_core::entity::Entity>,
    pub primary: Option<gizmo_core::entity::Entity>,
    pub rubber_band_start: Option<gizmo_math::Vec2>,
    pub rubber_band_current: Option<gizmo_math::Vec2>,
    pub rubber_band_request: Option<(gizmo_math::Vec2, gizmo_math::Vec2)>,
}

#[derive(Default, Debug)]
#[non_exhaustive]
pub struct ScriptEditorState {
    pub open: bool,
    pub active_path: Option<String>,
    pub active_content: Option<String>,
    pub is_dirty: bool,
    pub pending_clear_confirm: bool,
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ConsoleMode {
    EngineLogs,
    BuildOutput,
}

#[non_exhaustive]
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

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PostProcessSettings {
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub exposure: f32,
    pub vignette: f32,
    pub chromatic_aberration: f32,
    pub dof_focus_dist: f32,
    pub dof_focus_range: f32,
    pub dof_blur_size: f32,
    pub film_grain: f32,
    pub fxaa_enabled: bool,
    pub ssao_enabled: bool,
    pub ssao_strength: f32,
}

impl Default for PostProcessSettings {
    fn default() -> Self {
        Self {
            bloom_intensity: 0.8,
            bloom_threshold: 0.85,
            exposure: 1.0,
            vignette: 0.2,
            chromatic_aberration: 0.005,
            dof_focus_dist: 10.0,
            dof_focus_range: 20.0,
            dof_blur_size: 2.0,
            film_grain: 0.0,
            fxaa_enabled: true,
            ssao_enabled: true,
            ssao_strength: 0.8,
        }
    }
}

/// Dövüş oyunu HUD durumu — game_view.rs tarafından okunur,
/// simulation loop tarafından yazılır.
#[derive(Debug)]
#[non_exhaustive]
pub struct FightHudState {
    pub active: bool,
    pub p1_name: String,
    pub p2_name: String,
    pub p1_health: f32,
    pub p1_max_health: f32,
    pub p2_health: f32,
    pub p2_max_health: f32,
    pub current_round: u32,
    pub timer_seconds: f32,
    pub p1_entity: Option<u32>,
    pub p2_entity: Option<u32>,
}

impl Default for FightHudState {
    fn default() -> Self {
        Self {
            active: false,
            p1_name: "Player 1".to_string(),
            p2_name: "Player 2".to_string(),
            p1_health: 100.0,
            p1_max_health: 100.0,
            p2_health: 100.0,
            p2_max_health: 100.0,
            current_round: 1,
            timer_seconds: 99.0,
            p1_entity: None,
            p2_entity: None,
        }
    }
}
