use std::collections::HashMap;
use taffy::{TaffyTree, NodeId};
use gizmo_math::Vec2;

/// Shared layout state for the UI, stored as a resource.
///
/// Holds the [`TaffyTree`] used to compute layouts, the mapping from entity ids
/// to taffy nodes, and the current window size used as the available space for
/// root nodes.
pub struct UiContext {
    /// The taffy layout tree backing all UI nodes.
    pub taffy: TaffyTree,
    /// Mapping from entity id to its corresponding taffy node.
    pub entity_to_node: HashMap<u32, NodeId>,
    /// Size of the window, used as available space for root layout.
    pub window_size: Vec2,
}

unsafe impl Send for UiContext {}
unsafe impl Sync for UiContext {}

impl Default for UiContext {
    fn default() -> Self {
        Self::new()
    }
}

impl UiContext {
    /// Creates an empty [`UiContext`] with a default window size.
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
            entity_to_node: HashMap::new(),
            window_size: Vec2::new(1280.0, 720.0), // Default window size
        }
    }
}
