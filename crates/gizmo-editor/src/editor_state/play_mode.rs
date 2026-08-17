//! EditorState — play / pause / edit mode transitions.
use super::*;

impl EditorState {
    /// Toggles between play and stop.
    /// Edit → Play: sets `play_start_request`, so a scene snapshot gets taken.
    /// Play or Paused → Edit: sets `play_stop_request`, so the scene gets restored.
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

    /// Is the game actually running? (Play only — not Paused.)
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
