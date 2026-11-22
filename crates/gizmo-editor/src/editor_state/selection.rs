//! EditorState — entity selection API.
use super::*;

impl EditorState {
    /// The one entity the inspector, the transform gizmo and the animation and shader panels work
    /// on: the primary — the last one clicked — while it is still selected, else the selected entity
    /// with the lowest id. `None` with nothing selected.
    ///
    /// Asked here, not of `selection.primary`: the selection set is written directly in places (the
    /// rubber band, select-all) that leave the primary unset or pointing at an entity no longer
    /// selected, and each panel made up its own answer — an arbitrary member of the set (a
    /// `HashSet`'s first, which is no order at all), an unwrap, or the stale primary — so with
    /// several selected the inspector and the animation panel could be showing two different
    /// entities, neither of them the one last clicked.
    pub fn primary_selection(&self) -> Option<gizmo_core::entity::Entity> {
        self.selection
            .primary
            .filter(|p| self.selection.entities.contains(p))
            .or_else(|| self.selection.entities.iter().copied().min_by_key(|e| e.id()))
    }

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
                self.selection.primary = None;
                self.selection.primary = self.primary_selection();
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
                self.selection.primary = None;
                self.selection.primary = self.primary_selection();
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

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::entity::Entity;

    /// The one entity the panels work on is the last one clicked while it is selected — and when the
    /// set was written directly, as the rubber band writes it, the lowest selected id rather than a
    /// stale primary or a `HashSet`'s arbitrary first.
    #[test]
    fn the_primary_selection_is_the_last_click_or_the_lowest_id() {
        let ids: Vec<Entity> = (1..=6).map(|i| Entity::new(i, 0)).collect();
        let mut state = EditorState::default();
        assert_eq!(state.primary_selection(), None);

        state.select_exclusive(ids[0]);
        state.toggle_selection(ids[4]);
        assert_eq!(state.primary_selection(), Some(ids[4]), "the last one clicked");

        // The rubber band: the set cleared and refilled, the primary left as it was.
        state.selection.entities.clear();
        for &e in &ids[1..4] {
            state.selection.entities.insert(e);
        }
        assert_eq!(state.primary_selection(), Some(ids[1]), "not the stale primary, and not whichever the set yields first");

        // Taking the primary out of the selection hands it on by the same rule.
        state.select_exclusive(ids[5]);
        for &e in &ids[2..5] {
            state.toggle_selection(e);
        }
        state.toggle_selection(ids[4]);
        assert_eq!(state.selection.primary, Some(ids[2]), "the lowest id left, not the set's first");
    }
}
