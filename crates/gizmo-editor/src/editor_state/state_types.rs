use super::*;

/// A dockable tab in the editor's layout. Serialised with the layout, so a tab that is renamed
/// is a tab that vanishes from everyone's saved dock.
#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Hash, Debug)]
#[non_exhaustive]
pub enum EditorTab {
    /// The ANIMATION timeline — tracks and keyframes for the selected entity's player.
    Animation,
    /// The scene tree.
    Hierarchy,
    /// The selected entity's components.
    Inspector,
    /// The project's files.
    AssetBrowser,
    /// The editable viewport, with the gizmo and picking.
    SceneView,
    /// What the game camera sees, as the game sees it.
    GameView,
    /// The log console.
    Console,
    /// The editor's settings window.
    Settings,
    /// The Lua script editor.
    ScriptEditor,
    /// Frame timings and the flamegraph.
    Profiler,
}

/// The gizmo tool's mode
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum GizmoMode {
    /// Picking only; the gizmo is not drawn.
    Select,
    /// Move the selection.
    Translate,
    /// Turn it.
    Rotate,
    /// Resize it.
    Scale,
}

/// The target operating system of a build
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum BuildTarget {
    /// The current operating system (native)
    Native,
    /// Linux (x86_64-unknown-linux-gnu)
    Linux,
    /// Windows (x86_64-pc-windows-gnu — needs a cross toolchain)
    Windows,
    /// macOS (x86_64-apple-darwin — on a Mac only)
    MacOs,
}

/// The editor's run mode
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum EditorMode {
    /// Edit mode — physics is stopped and entities can be manipulated freely.
    Edit,
    /// Game mode — physics and scripts run
    Play,
    /// Game mode, paused
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
    /// A bare entity: a transform and a name, nothing else.
    Empty,
    /// A folder: an entity that groups without drawing.
    ///
    /// There is no marker component. `Parent`/`Children` already say everything "folder" means,
    /// and a flag beside them would be a second source of truth that can disagree — an entity
    /// with the marker and a mesh, or with children and no marker. What a group actually is, is
    /// an entity with a name, children, and nothing to draw.
    Group,
    /// A cube, with a matching box collider.
    Cube,
    /// A sphere, with a matching sphere collider.
    Sphere,
    /// A plane quad, with a thin box collider under it.
    Plane,
    /// A cylinder, with a cylinder collider.
    Cylinder,
    /// A capsule, with a capsule collider.
    Capsule,
    /// A point light.
    PointLight,
    /// A camera.
    Camera,
    /// A particle emitter.
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
    /// The clip's translation tracks.
    Translation,
    /// Its rotation tracks.
    Rotation,
    /// Its scale tracks.
    Scale,
}

/// One keyframe, addressed the only way a clip allows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyframeRef {
    /// Which of the clip's three channel lists the track is in.
    pub channel: TrackChannel,
    /// Index of the track within that list.
    pub track: usize,
    /// Index of the keyframe within that track.
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

// --- The nested state structs ---
/// What the viewport asked the editor camera to do this frame, and what it worked out.
///
/// The `*_delta` fields are inputs the panel writes and the camera system consumes; `view` and
/// `proj` are outputs it writes back, so that picking and the gizmo work from the same matrices
/// the frame was drawn with rather than rebuilding them.
#[derive(Default, Debug)]
#[non_exhaustive]
pub struct CameraState {
    /// Whether the viewport is in fly mode: the right mouse button held down over it.
    ///
    /// Separate from `look_delta`, which is `Some` only while the pointer is actually moving. Fly
    /// movement has to work with the mouse held still, and holding the button is the whole signal.
    pub fly_active: bool,
    /// Mouse movement to look with this frame, in points. `None` while the pointer is still.
    pub look_delta: Option<gizmo_math::Vec2>,
    /// Movement to pan the camera sideways with (middle drag).
    pub pan_delta: Option<gizmo_math::Vec2>,
    /// Movement to orbit the focus point with (Alt drag).
    pub orbit_delta: Option<gizmo_math::Vec2>,
    /// Wheel movement to dolly with.
    pub scroll_delta: Option<f32>,
    /// The view matrix the frame was drawn with — written by the camera system, read by picking.
    pub view: Option<gizmo_math::Mat4>,
    /// The projection matrix that went with it.
    pub proj: Option<gizmo_math::Mat4>,
    /// A point to frame, set by `F` on the selection and cleared once the camera has moved there.
    pub focus_target: Option<gizmo_math::Vec3>,
    /// A world-space direction the editor camera should be pointed along, set by the viewport's
    /// axis gizmo and consumed (and cleared) by the studio's camera system.
    ///
    /// A **direction**, not a yaw/pitch pair, on purpose: yaw/pitch live on the `Camera`
    /// component, and only the consumer knows what the current yaw is — which matters, because a
    /// straight-up or straight-down look leaves yaw undetermined and has to inherit it rather
    /// than snap to zero.
    pub view_request: Option<gizmo_math::Vec3>,
    /// The ten camera bookmarks, each a position with its yaw and pitch. Empty slots are
    /// `None`.
    pub bookmarks: [Option<(gizmo_math::Vec3, f32, f32)>; 10],
}

#[derive(Debug)]
#[non_exhaustive]
/// Exporting a game: what was asked for, and what the build is doing about it.
///
/// The build runs `cargo` on its own thread, so everything here is either a request the UI
/// raised or a channel it reads the result from — the frame must not block on a compiler.
pub struct BuildState {
    /// Raised by the toolbar's Build button; cleared once the build has been started.
    pub request: bool,
    /// Which platform to build for.
    pub target: BuildTarget,
    /// Whether a build is running. Shared with the build thread, which clears it when it exits.
    pub is_building: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// The channel the build thread streams its output on.
    pub logs_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    /// Output already drained from that channel, with the colour it is drawn in — kept because
    /// the receiver can only be read once.
    pub cached_logs: Vec<(String, egui::Color32)>,
    /// When the running build started, for the elapsed time in the panel.
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
/// The asset browser's own state: where it is looking, what it is showing, and the caches that
/// keep it from walking the filesystem every frame.
pub struct AssetBrowserState {
    /// The search box's contents.
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
    /// Is the browser panel visible?
    pub show: bool,
    /// The channel a "choose workspace folder" dialog answers on; it runs off-thread like the
    /// other native dialogs.
    pub workspace_rx: Option<std::sync::Mutex<std::sync::mpsc::Receiver<String>>>,
    /// The grid's listing of `root`: `(which folder, when it was read, the entries)`, each entry
    /// being `(path, display name, is a directory)`. Re-read when it goes stale rather than every
    /// frame.
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
/// Scene-level requests the UI raised, and the dialogs it is waiting on.
pub struct SceneState {
    /// "Save the scene to this path."
    pub save_request: Option<String>,
    /// "Load the scene at this path."
    pub load_request: Option<String>,
    /// "Empty the scene", keeping the editor's own entities.
    pub clear_request: bool,
    /// "Rebuild the navigation mesh from what is in the scene now."
    pub rebuild_navmesh_request: bool,
    /// Open the native save dialog — the scene has no path yet, or Save As was used.
    pub request_save_dialog: bool,
    /// A load waiting on "you have unsaved changes, continue?", carrying the path it would load.
    pub load_confirm_dialog: Option<String>,
    /// The transforms as they were when the current gizmo drag started, so the whole drag is one
    /// undo entry rather than one per frame.
    pub gizmo_original_transforms:
        std::collections::HashMap<gizmo_core::entity::Entity, gizmo_physics_core::Transform>,
}

#[derive(Default, Debug)]
#[non_exhaustive]
/// What is selected, and the rubber band being dragged over the viewport.
pub struct SelectionState {
    /// Everything selected.
    pub entities: std::collections::HashSet<gizmo_core::entity::Entity>,
    /// The one the inspector shows and the gizmo sits on — the last one clicked.
    pub primary: Option<gizmo_core::entity::Entity>,
    /// Where the rubber band started, in viewport points; `None` when none is being dragged.
    pub rubber_band_start: Option<gizmo_math::Vec2>,
    /// Where the pointer is now, so the band can be drawn.
    pub rubber_band_current: Option<gizmo_math::Vec2>,
    /// A finished band waiting to be turned into a selection: the two corners it spanned.
    pub rubber_band_request: Option<(gizmo_math::Vec2, gizmo_math::Vec2)>,
}

#[derive(Default, Debug)]
#[non_exhaustive]
/// The script editor tab: which file is open, its text, and whether it has been changed.
pub struct ScriptEditorState {
    /// Is the editor tab open?
    pub open: bool,
    /// The file being edited, `None` when nothing is open.
    pub active_path: Option<String>,
    /// Its text as edited — not what is on disk until it is saved.
    pub active_content: Option<String>,
    /// Whether the text differs from the file.
    pub is_dirty: bool,
    /// A "discard your changes?" prompt is waiting for an answer.
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
/// Which stream the console panel is showing.
pub enum ConsoleMode {
    /// The engine's own log.
    EngineLogs,
    /// The output of the running build.
    BuildOutput,
}

#[non_exhaustive]
/// The console panel: which lines it shows, and the snapshot it draws from.
pub struct ConsoleState {
    /// Which console is being shown — the log, or the cvar command line.
    pub mode: ConsoleMode,
    /// Show info lines?
    pub show_info: bool,
    /// Show warnings?
    pub show_warn: bool,
    /// Show errors?
    pub show_error: bool,
    /// A substring every shown line must contain.
    pub filter_text: String,

    // The panel draws from a snapshot rather than holding the logger's lock for the frame.
    /// The lines as of the last refresh.
    pub cached_logs: Vec<gizmo_core::logger::LogEntry>,
    /// The logger's version when that snapshot was taken; a change is what triggers a refresh.
    pub last_version: usize,

    // Counts, for the level chips.
    /// How many info lines the snapshot holds.
    pub count_info: usize,
    /// How many warnings.
    pub count_warn: usize,
    /// How many errors.
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

/// The struct holding all of the editor's state

#[derive(Clone, Debug)]
#[non_exhaustive]
/// The post-process controls that belong to the **editor**, and only those.
///
/// The graded look — bloom, vignette, aberration, depth of field, film grain — used to live here
/// too, and that was the defect: this struct is editor state, nothing wrote it to a file, and the
/// engine's frame read a second unrelated copy off the `Renderer`. So a look an author tuned was
/// gone on reopen and absent from every exported build, while the panel editing it said in as many
/// words that these were the scene's settings. The look is now
/// [`PostProcess`](gizmo_renderer::components::PostProcess), a component on the camera that renders
/// it, and exposure is where it always was — `Camera::exposure`.
///
/// What is left is genuinely the tool's own: two viewport toggles that never travelled with a
/// scene and never should.
pub struct PostProcessSettings {
    /// Whether FXAA runs as the last pass **of the editor viewport**.
    pub fxaa_enabled: bool,
    /// Whether screen-space ambient occlusion is computed. Inert in the editor — both studio views
    /// draw through the forward path and SSAO needs the deferred G-buffer's normal target; the
    /// Settings tab says so and disables the control.
    pub ssao_enabled: bool,
    /// How strongly that occlusion darkens the frame.
    pub ssao_strength: f32,
}

impl Default for PostProcessSettings {
    fn default() -> Self {
        Self {
            fxaa_enabled: true,
            ssao_enabled: true,
            ssao_strength: 0.8,
        }
    }
}

/// The fighting-game HUD state — read by game_view.rs, written by the simulation loop.
#[derive(Debug)]
#[non_exhaustive]
pub struct FightHudState {
    /// Whether the HUD is drawn at all.
    pub active: bool,
    /// Player one's name plate.
    pub p1_name: String,
    /// Player two's name plate.
    pub p2_name: String,
    /// Player one's remaining health.
    pub p1_health: f32,
    /// What that health bar is full at.
    pub p1_max_health: f32,
    /// Player two's remaining health.
    pub p2_health: f32,
    /// What their bar is full at.
    pub p2_max_health: f32,
    /// Which round is being fought, counting from 1.
    pub current_round: u32,
    /// Seconds left on the round timer.
    pub timer_seconds: f32,
    /// Player one's entity, so the HUD can follow the right fighter.
    pub p1_entity: Option<u32>,
    /// Player two's entity.
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
