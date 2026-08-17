use super::*;

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Hash, Debug)]
#[non_exhaustive]
pub enum EditorTab {
    /// The ANIMATION timeline — tracks and keyframes for the selected entity's player.
    Animation,
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

/// What the hierarchy's `➕` menu can ask the studio to create.
///
/// # Why this is a type and not a string
///
/// It was an `Option<String>`, written in `gizmo-editor` and matched in `gizmo-studio`, and the
/// two drifted badly: the menu offered **nine** kinds and the spawner's match had **two** arms
/// plus a catch-all, so `Group`, `Plane`, `Cylinder`, `Capsule`, `PointLight`, `Camera` and
/// `ParticleEmitter` all fell through and produced an entity called "Boş Entity" carrying a
/// `MeshRenderer` and no mesh. Seven menu entries promising seven different things and making the
/// same wrong one. A string channel between two crates has no agreement in it to check, so
/// nothing could have caught it — not a test, not the compiler, not a reader of either file.
///
/// # Why it is deliberately NOT `#[non_exhaustive]`
///
/// Every other public enum here is. This one must not be, and that is the whole point: the
/// spawner matches on it exhaustively, so adding a variant without teaching the spawner about it
/// is a **compile error**. `#[non_exhaustive]` would force a `_` arm back into `gizmo-studio` and
/// hand back precisely the hole this type exists to close.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpawnKind {
    Empty,
    /// A folder: an entity that groups without drawing.
    ///
    /// There is no marker component. `Parent`/`Children` already say everything "folder" means,
    /// and a flag beside them would be a second source of truth that can disagree — an entity
    /// with the marker and a mesh, or with children and no marker. What a group actually is, is
    /// an entity with a name, children, and nothing to draw.
    Group,
    Cube,
    Sphere,
    Plane,
    Cylinder,
    Capsule,
    PointLight,
    Camera,
    ParticleEmitter,
}

impl SpawnKind {
    /// Does this kind put geometry on the screen?
    ///
    /// Only these may be given a `MeshRenderer`. It used to be added to every spawn before the
    /// match ran, which left one on every light, camera and empty in the scene.
    pub fn draws(self) -> bool {
        match self {
            Self::Cube | Self::Sphere | Self::Plane | Self::Cylinder | Self::Capsule => true,
            Self::Empty
            | Self::Group
            | Self::PointLight
            | Self::Camera
            | Self::ParticleEmitter => false,
        }
    }

    /// The name the new entity carries in the hierarchy.
    pub fn entity_name(self) -> &'static str {
        match self {
            Self::Empty => "Boş Entity",
            Self::Group => "Grup",
            Self::Cube => "Küp",
            Self::Sphere => "Küre",
            Self::Plane => "Düzlem",
            Self::Cylinder => "Silindir",
            Self::Capsule => "Kapsül",
            Self::PointLight => "Nokta Işığı",
            Self::Camera => "Kamera",
            Self::ParticleEmitter => "Particle Emitter",
        }
    }

    /// Every kind, for tests that have to walk all of them. Kept exhaustive by
    /// `every_kind_is_in_all`, which matches on a variant rather than trusting this list.
    pub const ALL: &'static [SpawnKind] = &[
        Self::Empty,
        Self::Group,
        Self::Cube,
        Self::Sphere,
        Self::Plane,
        Self::Cylinder,
        Self::Capsule,
        Self::PointLight,
        Self::Camera,
        Self::ParticleEmitter,
    ];
}

/// Which of a clip's three channel lists a track lives in.
///
/// The three are separate `Vec`s on `AnimationClip`, so a track is only addressable as
/// *(which list, which index)* — an index alone is ambiguous between them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrackChannel {
    Translation,
    Rotation,
    Scale,
}

/// One keyframe, addressed the only way a clip allows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyframeRef {
    pub channel: TrackChannel,
    pub track: usize,
    pub keyframe: usize,
}

/// The timeline's authoring state.
#[derive(Default, Debug)]
#[non_exhaustive]
pub struct AnimationEditState {
    /// The keyframe currently under the pointer's drag, if any. Updated as the drag reorders the
    /// track — a retime can move a key past its neighbours, and the index has to follow it or the
    /// next frame of the same drag grabs a different keyframe.
    pub dragging: Option<KeyframeRef>,
    /// The last keyframe clicked, so Delete has something to act on.
    pub selected: Option<KeyframeRef>,
}

// --- Alt Durum Yapilari ---
#[derive(Default, Debug)]
#[non_exhaustive]
pub struct CameraState {
    /// Whether the viewport is in fly mode: the right mouse button held down over it.
    ///
    /// Separate from `look_delta`, which is `Some` only while the pointer is actually moving. Fly
    /// movement has to work with the mouse held still, and holding the button is the whole signal.
    pub fly_active: bool,
    pub look_delta: Option<gizmo_math::Vec2>,
    pub pan_delta: Option<gizmo_math::Vec2>,
    pub orbit_delta: Option<gizmo_math::Vec2>,
    pub scroll_delta: Option<f32>,
    pub view: Option<gizmo_math::Mat4>,
    pub proj: Option<gizmo_math::Mat4>,
    pub focus_target: Option<gizmo_math::Vec3>,
    /// A world-space direction the editor camera should be pointed along, set by the viewport's
    /// axis gizmo and consumed (and cleared) by the studio's camera system.
    ///
    /// A **direction**, not a yaw/pitch pair, on purpose: yaw/pitch live on the `Camera`
    /// component, and only the consumer knows what the current yaw is — which matters, because a
    /// straight-up or straight-down look leaves yaw undetermined and has to inherit it rather
    /// than snap to zero.
    pub view_request: Option<gizmo_math::Vec3>,
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
    /// The type chip currently selected, or `None` for "All" — the prototype's asset filter row.
    pub kind_filter: Option<crate::asset_browser::AssetKind>,
    /// The file the detail pane describes, or `None` when nothing is selected.
    pub selected: Option<std::path::PathBuf>,
    /// The folder the **grid** is showing. Changes as you navigate.
    pub root: String,
    /// The folder the **tree** is rooted at. Separate from `root` on purpose: a tree rooted at
    /// wherever you happen to be standing would only ever show that folder's children, which is
    /// the grid again. See `crate::asset_browser::tree_root_for` for what happens when navigation
    /// walks above it.
    pub workspace_root: String,
    pub show: bool,
    pub workspace_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    pub cached_dir: Option<(
        String,
        Instant,
        Vec<(std::path::PathBuf, String, bool)>,
    )>,
    /// Subfolder listings for the tree, one entry per directory that has been expanded at least
    /// once: `(read at, the folders, how many there were before the cap)`.
    ///
    /// The tree only reads a directory while it is open, so an unexpanded branch costs nothing —
    /// but an open one would otherwise be re-walked every frame, and a folder tree is a panel that
    /// stays open.
    pub tree_cache: std::collections::HashMap<
        std::path::PathBuf,
        (Instant, Vec<(std::path::PathBuf, String)>, usize),
    >,
}
impl AssetBrowserState {
    /// The project directory the browser opens in, and the one whose asset identities an
    /// application should register.
    ///
    /// A single source for what used to be the string `"demo/assets"` written twice here — and, as
    /// soon as anything outside this crate needed it, would have been written a third time
    /// somewhere else. The studio scans exactly this directory at startup, so the editor cannot
    /// show one tree while the asset registry knows another.
    pub const DEFAULT_WORKSPACE_ROOT: &'static str = "demo/assets";
}

impl Default for AssetBrowserState {
    fn default() -> Self {
        Self {
            filter: String::new(),
            kind_filter: None,
            selected: None,
            root: Self::DEFAULT_WORKSPACE_ROOT.to_string(),
            workspace_root: Self::DEFAULT_WORKSPACE_ROOT.to_string(),
            show: true,
            workspace_rx: None,
            cached_dir: None,
            tree_cache: std::collections::HashMap::new(),
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
    /// Draft name in the inspector's "add a property" row. Native-only, like the row itself:
    /// `gizmo-scripting` is a `cfg(not(target_arch = "wasm32"))` dependency of this crate.
    #[cfg(not(target_arch = "wasm32"))]
    pub new_property_name: String,
    /// Which kind that draft property will be created as.
    #[cfg(not(target_arch = "wasm32"))]
    pub new_property_kind: crate::inspector::script::NewPropertyKind,
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
