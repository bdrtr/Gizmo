//! Editor State — Editörün global durumunu yönetir

/// Gizmo aracı modu
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
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

/// Hangi eksende sürüklendiğini belirtir
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DragAxis {
    X,
    Y,
    Z,
}

/// Editörün tüm durumunu tutan yapı
pub struct EditorState {
    /// Editor is toggled on/off
    pub open: bool,
    /// Seçili entity ID'si
    pub selected_entity: Option<u32>,
    /// Gizmo aracı modu (Translate/Rotate/Scale)
    pub gizmo_mode: GizmoMode,
    
    // --- Gizmo Kimlikleri ---
    pub gizmo_x: u32,
    pub gizmo_y: u32,
    pub gizmo_z: u32,
    /// Etrafını saran seçim çizgisi (Highlight Box)
    pub highlight_box: u32,
    
    // --- Etkileşim & Sürükleme Durumları ---
    pub do_raycast: bool,
    pub dragging_axis: Option<DragAxis>,
    pub drag_original_pos: gizmo_math::Vec3,
    pub drag_original_scale: gizmo_math::Vec3,
    pub drag_original_rot: gizmo_math::Quat,
    pub drag_start_t: f32,
    pub mouse_ndc: Option<gizmo_math::Vec2>,
    pub camera_look_delta: Option<gizmo_math::Vec2>,

    /// Çalışma modu
    pub mode: EditorMode,
    /// Hiyerarşi paneli açık mı?
    pub show_hierarchy: bool,
    /// Inspector paneli açık mı?
    pub show_inspector: bool,
    /// Asset browser paneli açık mı?
    pub show_asset_browser: bool,
    /// Toolbar açık mı?
    pub show_toolbar: bool,
    /// Filtre metni (hierarchy arama)
    pub hierarchy_filter: String,
    /// Asset browser filtre metni
    pub asset_filter: String,
    /// Asset browser kök dizini
    pub asset_root: String,
    /// Inspector add component dropdown açık mı?
    pub add_component_open: bool,
    /// Son hata mesajı
    pub last_error: Option<String>,
    /// Durum çubuğu mesajı
    pub status_message: String,
    /// Sahne dosya yolu
    pub scene_path: String,
    /// Silme talebi gönderilen entity ID
    pub despawn_request: Option<u32>,
    /// Sahne kaydetme isteği (Dosya yolu)
    pub scene_save_request: Option<String>,
    /// Sahne yükleme isteği (Dosya yolu)
    pub scene_load_request: Option<String>,
    /// Prefab kaydetme isteği (Entity ID, Dosya yolu)
    pub prefab_save_request: Option<(u32, String)>,
    /// Prefab yükleme isteği (Dosya yolu, Opsiyonel Parent ID)
    pub prefab_load_request: Option<(String, Option<u32>)>,
    /// Entity kopyalama / çoğaltma isteği (Entity ID)
    pub duplicate_request: Option<u32>,
    /// Yeni entity yaratma talebi (Type adı örn: "Empty", "Cube", "Sphere")
    pub spawn_request: Option<String>,
    /// Asset üzerinden yeni model spawn etme isteği (Dosya yolu)
    pub spawn_asset_request: Option<String>,
    /// İsteğe bağlı, ekrandan (drag&drop) atılan pozisyon
    pub spawn_asset_position: Option<gizmo_math::Vec3>,
    /// Entity ebeveyn değiştirme (Dragged ID, Target Parent ID)
    pub reparent_request: Option<(u32, u32)>,
    /// Entity ebeveyni silme (Root yapma) - Drag ID
    pub unparent_request: Option<u32>,
    /// Obje görünürlüğünü aç/kapat tetiği
    pub toggle_visibility_request: Option<u32>,
    /// Seçili Obje ID'sine yeni obje tipi ekleme
    pub add_component_request: Option<(u32, String)>,
    /// Hangi kameraların çizileceğini anlamak için bayraklar
    pub scene_view_visible: bool,
    pub game_view_visible: bool,
    /// Scene View panelinin son bilinen konumu ve boyutu
    pub scene_view_rect: Option<egui::Rect>,
    /// WGPU tarafından verilen ve Egui'de çizilecek olan Doku (Texture) ID'si
    pub scene_texture_id: Option<egui::TextureId>,
    /// Console logları
    pub console_logs: Vec<(String, egui::Color32)>,
    /// Docking State (Pencere yerleşim verisi)
    pub dock_state: egui_dock::DockState<String>,

    // --- Gizmo Debug Renderer ---
    /// (Pos, Rot, Scale, Color) olarak çizim talepleri
    pub debug_draw_requests: Vec<(gizmo_math::Vec3, gizmo_math::Quat, gizmo_math::Vec3, gizmo_math::Vec4)>,
    /// Ekranda belirip silinmesi gereken Debug çizim objeleri (Timer, EntityID)
    pub debug_spawned_entities: Vec<(f32, u32)>,
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            open: false,
            selected_entity: None,
            gizmo_mode: GizmoMode::Translate,

            gizmo_x: 0,
            gizmo_y: 0,
            gizmo_z: 0,
            highlight_box: 0,
            
            do_raycast: false,
            dragging_axis: None,
            drag_original_pos: gizmo_math::Vec3::new(0.0, 0.0, 0.0),
            drag_original_scale: gizmo_math::Vec3::new(1.0, 1.0, 1.0),
            drag_original_rot: gizmo_math::Quat::default(),
            drag_start_t: 0.0,
            mouse_ndc: None,
            camera_look_delta: None,

            mode: EditorMode::Edit,
            show_hierarchy: true,
            show_inspector: true,
            show_asset_browser: true,
            show_toolbar: true,
            hierarchy_filter: String::new(),
            asset_filter: String::new(),
            asset_root: "demo/assets".to_string(),
            add_component_open: false,
            last_error: None,
            status_message: "Hazır".to_string(),
            scene_path: "scene.giz".to_string(),
            despawn_request: None,
            scene_save_request: None,
            scene_load_request: None,
            prefab_save_request: None,
            prefab_load_request: None,
            duplicate_request: None,
            spawn_request: None,
            spawn_asset_request: None,
            spawn_asset_position: None,
            reparent_request: None,
            unparent_request: None,
            toggle_visibility_request: None,
            add_component_request: None,
            scene_view_visible: true,
            game_view_visible: false,
            scene_view_rect: None,
            scene_texture_id: None,
            console_logs: Vec::new(),
            dock_state: create_default_dock_state(),
            debug_draw_requests: Vec::new(),
            debug_spawned_entities: Vec::new(),
        }
    }

    pub fn select_entity(&mut self, id: u32) {
        self.selected_entity = Some(id);
    }

    pub fn deselect(&mut self) {
        self.selected_entity = None;
    }

    pub fn toggle_play(&mut self) {
        self.mode = match self.mode {
            EditorMode::Edit => EditorMode::Play,
            EditorMode::Play => EditorMode::Edit,
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
        self.console_logs.push((format!("ℹ️ {}", msg), egui::Color32::WHITE));
    }
    
    pub fn log_warning(&mut self, msg: &str) {
        self.console_logs.push((format!("⚠️ {}", msg), egui::Color32::YELLOW));
    }

    pub fn log_error(&mut self, msg: &str) {
        self.console_logs.push((format!("❌ {}", msg), egui::Color32::RED));
    }
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}

fn create_default_dock_state() -> egui_dock::DockState<String> {
    use egui_dock::{DockState, NodeIndex};
    // Root tab "Scene View" and "Game View" in the same area
    let mut state = DockState::new(vec!["Scene View".to_string(), "Game View".to_string()]);
    let surface = state.main_surface_mut();

    // Right Split for Inspector & Hierarchy
    let [main, right_panel] = surface.split_right(NodeIndex::root(), 0.8, vec!["Inspector".to_string()]);
    let [_hierarchy, _inspector] = surface.split_above(right_panel, 0.4, vec!["Hierarchy".to_string()]);
    
    // Bottom Split for Asset Browser
    let [_main, _bottom] = surface.split_below(main, 0.7, vec!["Asset Browser".to_string(), "Console".to_string()]);
    
    state
}

