use std::collections::HashMap;
use taffy::{TaffyTree, NodeId};
use gizmo_core::entity::Entity;
use gizmo_math::Vec2;

pub struct UiContext {
    pub taffy: TaffyTree,
    pub entity_to_node: HashMap<u32, NodeId>,
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
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
            entity_to_node: HashMap::new(),
            window_size: Vec2::new(1280.0, 720.0), // Default window size
        }
    }
}
