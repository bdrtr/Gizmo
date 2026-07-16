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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_context_is_empty_with_documented_fallback_size() {
        // A brand-new context must carry no nodes and no entity mapping — the
        // layout system relies on this to (re)build the taffy tree from scratch.
        let ctx = UiContext::new();
        assert!(ctx.entity_to_node.is_empty(), "no entity mapping yet");
        assert_eq!(ctx.taffy.total_node_count(), 0, "no taffy nodes yet");
        // The 1280x720 fallback is load-bearing: it is the available space used
        // for root layout until a real WindowInfo resize updates window_size.
        assert_eq!(ctx.window_size, Vec2::new(1280.0, 720.0));
    }

    #[test]
    fn default_matches_new() {
        // `Default` is documented as delegating to `new`; the observable empty
        // state and fallback size must be identical.
        let d = UiContext::default();
        let n = UiContext::new();
        assert_eq!(d.window_size, n.window_size);
        assert_eq!(d.entity_to_node.len(), n.entity_to_node.len());
        assert_eq!(d.taffy.total_node_count(), n.taffy.total_node_count());
    }
}
