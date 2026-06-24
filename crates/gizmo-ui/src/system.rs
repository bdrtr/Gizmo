use gizmo_core::query::{Query, Mut};
use gizmo_core::system::ResMut;
use gizmo_core::component::{Children, Parent};
use crate::components::{Style, Node};
use crate::layout::UiContext;
use taffy::{AvailableSpace, Size};

/// System that syncs UI entities into the taffy tree, computes layout for each
/// root, and writes the results back into the [`Node`] components.
pub fn ui_layout_system(
    mut ctx: ResMut<UiContext>,
    styles: Query<&Style>,
    parents: Query<&Parent>,
    children: Query<&Children>,
    nodes: Query<Mut<Node>>,
) {
    let mut current_entities = std::collections::HashSet::new();

    // 1. Ensure all entities with a Style have a Taffy node
    for (entity, style) in styles.iter() {
        current_entities.insert(entity);

        if !ctx.entity_to_node.contains_key(&entity) {
            // `new_leaf` returns a TaffyResult; on the rare allocation/insertion
            // failure we skip this entity for this frame instead of panicking the
            // whole frame loop. The entity will simply be retried next frame.
            // This keeps the policy consistent with the other taffy calls below
            // (`set_style`, `set_children`, `remove`, `compute_layout`) which all
            // swallow their errors.
            match ctx.taffy.new_leaf(style.0.clone()) {
                Ok(node_id) => {
                    ctx.entity_to_node.insert(entity, node_id);
                }
                Err(_) => {
                    // Skip this entity this frame; layout write-back below is
                    // guarded by `entity_to_node.get`, so a missing node is safe.
                    continue;
                }
            }
        } else {
            // Update style if it changed (always updating for simplicity here)
            let node_id = ctx.entity_to_node[&entity];
            let _ = ctx.taffy.set_style(node_id, style.0.clone());
        }
    }

    // 2. Remove deleted entities from Taffy
    let mut to_remove = Vec::new();
    for (&entity, &node_id) in ctx.entity_to_node.iter() {
        if !current_entities.contains(&entity) {
            to_remove.push((entity, node_id));
        }
    }
    for (entity, node_id) in to_remove {
        let _ = ctx.taffy.remove(node_id);
        ctx.entity_to_node.remove(&entity);
    }

    // 3. Update parent-child hierarchy in Taffy
    for (entity, _) in styles.iter() {
        if let Some(&node_id) = ctx.entity_to_node.get(&entity) {
            if let Some(children_comp) = children.get(entity) {
                let mut taffy_children = Vec::new();
                for &child_id in &children_comp.0 {
                    if let Some(&v) = ctx.entity_to_node.get(&child_id) {
                        taffy_children.push(v);
                    }
                }
                let _ = ctx.taffy.set_children(node_id, &taffy_children);
            } else {
                let _ = ctx.taffy.set_children(node_id, &[]);
            }
        }
    }

    // 4. Compute layout for roots
    // A root is any node without a Parent, or without a parent that has a Style
    let roots: Vec<_> = current_entities.iter()
        .filter_map(|&entity| {
            if parents.get(entity).is_none() {
                // Use `get` rather than indexing: an entity may be present in
                // `current_entities` but missing from `entity_to_node` if its
                // `new_leaf` allocation failed earlier this frame.
                ctx.entity_to_node.get(&entity).copied()
            } else {
                // Technically we should check if the parent also has a Style.
                // For simplicity, we assume the ECS hierarchy accurately represents the UI tree.
                None
            }
        }).collect();

    let available_space = Size {
        width: AvailableSpace::Definite(ctx.window_size.x),
        height: AvailableSpace::Definite(ctx.window_size.y),
    };

    for &root_node in &roots {
        let _ = ctx.taffy.compute_layout(root_node, available_space);
    }

    // 5. Write back calculated layout to Node components
    for (entity, _) in styles.iter() {
        if let Some(&node_id) = ctx.entity_to_node.get(&entity) {
            if let Ok(layout) = ctx.taffy.layout(node_id) {
                if let Some(mut node) = nodes.get_mut(entity) {
                    node.size = gizmo_math::Vec2::new(layout.size.width, layout.size.height);
                    node.position = gizmo_math::Vec2::new(layout.location.x, layout.location.y);
                }
            }
        }
    }
}
