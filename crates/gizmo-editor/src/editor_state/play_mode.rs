//! EditorState — play / pause / edit mode transitions.
use super::*;

impl EditorState {
    /// Play/Stop geçişi yapar.
    /// Edit → Play: Sahne snapshot'ı alınması için `play_start_request` set edilir.
    /// Play veya Paused → Edit: Sahne geri yüklenmesi için `play_stop_request` set edilir.
    pub fn toggle_play(&mut self) {
        self.mode = match self.mode {
            EditorMode::Edit => {
                self.play_start_request = true;
                self.open_tab(EditorTab::GameView);
                EditorMode::Play
            }
            EditorMode::Play | EditorMode::Paused => {
                self.play_stop_request = true;
                self.open_tab(EditorTab::SceneView);
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
}
