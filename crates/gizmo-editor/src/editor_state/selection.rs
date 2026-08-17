//! EditorState — entity selection API.
use super::*;

impl EditorState {
    /// Is this entity in the selection?
    pub fn is_selected(&self, id: gizmo_core::entity::Entity) -> bool {
        self.selection.entities.contains(&id)
    }

    /// Selects just this entity, dropping whatever was selected before, and makes it primary.
    pub fn select_exclusive(&mut self, id: gizmo_core::entity::Entity) {
        self.selection.entities.clear();
        self.selection.entities.insert(id);
        self.selection.primary = Some(id);
    }

    /// Adds the entity to the selection, or removes it if it was already there — Ctrl+click.
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

    /// Removes one entity from the selection, choosing a new primary if it was the primary.
    pub fn unselect_entity(&mut self, id: gizmo_core::entity::Entity) {
        if self.selection.entities.contains(&id) {
            self.selection.entities.remove(&id);
            if self.selection.primary == Some(id) {
                self.selection.primary = self.selection.entities.iter().next().copied();
            }
        }
    }

    /// Selects nothing.
    pub fn clear_selection(&mut self) {
        self.selection.entities.clear();
        self.selection.primary = None;
        self.selection.rubber_band_start = None;
        self.selection.rubber_band_current = None;
        self.selection.rubber_band_request = None;
        self.scene.gizmo_original_transforms.clear();
    }
}
