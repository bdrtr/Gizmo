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

/// Editörün tüm durumunu tutan yapı
pub struct EditorState {
    /// Seçili entity ID'si
    pub selected_entity: Option<u32>,
    /// Gizmo aracı modu (Translate/Rotate/Scale)
    pub gizmo_mode: GizmoMode,
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
}

impl EditorState {
    pub fn new() -> Self {
        Self {
            selected_entity: None,
            gizmo_mode: GizmoMode::Translate,
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
            scene_path: "scene.json".to_string(),
            despawn_request: None,
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
}

impl Default for EditorState {
    fn default() -> Self {
        Self::new()
    }
}
