use crate::component::{Children, Parent};
use crate::entity::Entity;
use crate::world::World;

/// Extends `World` with hierarchy manipulation methods.
pub trait HierarchyExt {
    /// Despawns an entity and all of its descendants recursively.
    fn despawn_recursive(&mut self, entity: Entity);
    
    /// Adds a child to a parent entity, updating both `Parent` and `Children` components.
    fn add_child(&mut self, parent: Entity, child: Entity);
    
    /// Removes a child from a parent entity.
    fn remove_child(&mut self, parent: Entity, child: Entity);

    /// Returns `true` if `ancestor` is an ancestor of `descendant` (walking up the
    /// `Parent` chain). Cycle-safe: stops if the existing chain already loops.
    /// Use before reparenting to reject links that would create a hierarchy cycle.
    fn is_ancestor(&self, ancestor: u32, descendant: u32) -> bool;
}

impl HierarchyExt for World {
    fn despawn_recursive(&mut self, entity: Entity) {
        // A `visited` set breaks `Children` cycles (e.g. reparenting an entity onto
        // its own descendant): without it a cycle recurses forever → stack overflow.
        let mut visited = std::collections::HashSet::new();
        despawn_recursive_inner(self, entity, &mut visited);
    }

    fn add_child(&mut self, parent: Entity, child: Entity) {
        // Refuse a link that would create a `Children` cycle: an entity can't be its
        // own parent, and `child` must not already be an ancestor of `parent`
        // (dragging a node onto its own descendant). A cycle hangs transform
        // propagation / despawn_recursive / scene save.
        if parent.id() == child.id() || self.is_ancestor(child.id(), parent.id()) {
            return;
        }

        // Remove from old parent first
        if let Some(parent_ptr) = self.get_component_ptr(child, std::any::TypeId::of::<Parent>()) {
            let old_parent_id = unsafe { (*(parent_ptr as *const Parent)).0 };
            if old_parent_id != parent.id() {
                if let Some(old_parent) = self.entity(old_parent_id) {
                    self.remove_child(old_parent, child);
                }
            }
        }

        // Add Parent component to child
        self.add_component(child, Parent(parent.id()));

        // Add to new parent's Children list
        if let Some(children_ptr) = self.get_component_mut_ptr(parent, std::any::TypeId::of::<Children>()) {
            let children = unsafe { &mut *(children_ptr as *mut Children) };
            if !children.0.contains(&child.id()) {
                children.0.push(child.id());
            }
        } else {
            self.add_component(parent, Children(vec![child.id()]));
        }
    }

    fn remove_child(&mut self, parent: Entity, child: Entity) {
        self.remove_component::<Parent>(child);

        if let Some(children_ptr) = self.get_component_mut_ptr(parent, std::any::TypeId::of::<Children>()) {
            let children = unsafe { &mut *(children_ptr as *mut Children) };
            children.0.retain(|&id| id != child.id());
        }
    }

    fn is_ancestor(&self, ancestor: u32, descendant: u32) -> bool {
        let parents = self.borrow::<Parent>();
        let mut visited = std::collections::HashSet::new();
        let mut current = descendant;
        // Walk up the Parent chain. `visited` also makes this safe if the existing
        // hierarchy already contains a cycle (stop instead of looping forever).
        while visited.insert(current) {
            match parents.get(current).map(|p| p.0) {
                Some(pid) if pid == ancestor => return true,
                Some(pid) => current = pid,
                None => return false,
            }
        }
        false
    }
}

/// Recursive worker for [`HierarchyExt::despawn_recursive`]. `visited` tracks
/// entity ids already handled so a `Children` cycle can't recurse forever.
fn despawn_recursive_inner(
    world: &mut World,
    entity: Entity,
    visited: &mut std::collections::HashSet<u32>,
) {
    if !visited.insert(entity.id()) {
        return; // already in-flight — a cycle led back here; stop.
    }

    let mut children_to_despawn = Vec::new();
    if let Some(children_ptr) = world.get_component_ptr(entity, std::any::TypeId::of::<Children>()) {
        let children = unsafe { &*(children_ptr as *const Children) };
        for &child_id in &children.0 {
            if let Some(child_entity) = world.entity(child_id) {
                children_to_despawn.push(child_entity);
            }
        }
    }

    // Detach from the (surviving) parent's Children list.
    if let Some(parent_ptr) = world.get_component_ptr(entity, std::any::TypeId::of::<Parent>()) {
        let parent_id = unsafe { (*(parent_ptr as *const Parent)).0 };
        if let Some(parent_entity) = world.entity(parent_id) {
            world.remove_child(parent_entity, entity);
        }
    }

    for child in children_to_despawn {
        despawn_recursive_inner(world, child, visited);
    }

    world.despawn(entity);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;

    #[test]
    fn test_hierarchy_add_remove() {
        let mut world = World::new();
        let parent = world.spawn();
        let child = world.spawn();

        world.add_child(parent, child);

        // Check if Parent component is added to child
        if let Some(parent_ptr) = world.get_component_ptr(child, std::any::TypeId::of::<Parent>()) {
            let parent_id = unsafe { (*(parent_ptr as *const Parent)).0 };
            assert_eq!(parent_id, parent.id());
        } else {
            panic!("Child missing Parent component");
        }

        // Check if Children component is updated
        if let Some(children_ptr) = world.get_component_ptr(parent, std::any::TypeId::of::<Children>()) {
            let children = unsafe { &*(children_ptr as *const Children) };
            assert_eq!(children.0.len(), 1);
            assert_eq!(children.0[0], child.id());
        } else {
            panic!("Parent missing Children component");
        }

        // Remove child
        world.remove_child(parent, child);

        // Child should not have Parent component anymore
        assert!(world.get_component_ptr(child, std::any::TypeId::of::<Parent>()).is_none());

        // Parent should have empty Children list
        if let Some(children_ptr) = world.get_component_ptr(parent, std::any::TypeId::of::<Children>()) {
            let children = unsafe { &*(children_ptr as *const Children) };
            assert_eq!(children.0.len(), 0);
        }
    }

    #[test]
    fn test_despawn_recursive() {
        let mut world = World::new();
        let p1 = world.spawn();
        let c1 = world.spawn();
        let c2 = world.spawn();
        let gc1 = world.spawn();

        world.add_child(p1, c1);
        world.add_child(p1, c2);
        world.add_child(c1, gc1);

        assert_eq!(world.entity_count(), 4);

        // Despawn root
        world.despawn_recursive(p1);

        // Entities should be marked for despawn, process them by calling despawn queue?
        // Wait, despawn is immediate through `entities_to_despawn` loop
        assert_eq!(world.entity_count(), 0);
    }

    #[test]
    fn despawn_recursive_survives_children_cycle() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        // A `Children` cycle with no matching `Parent` back-edges — reachable from a
        // loaded scene file or direct component edits, where the per-node parent
        // detach can't break the loop. The old recursive walk had no visited set and
        // recursed forever (stack overflow); with the guard it terminates.
        world.add_component(a, Children(vec![b.id()]));
        world.add_component(b, Children(vec![a.id()]));
        assert_eq!(world.entity_count(), 2);

        world.despawn_recursive(a);

        assert_eq!(world.entity_count(), 0, "both nodes despawn; no infinite recursion");
    }

    #[test]
    fn add_child_refuses_cycle_creating_reparent() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        world.add_child(a, b); // a -> b
        world.add_child(b, c); // b -> c  (chain a -> b -> c)

        assert!(world.is_ancestor(a.id(), c.id()), "a is an ancestor of c");
        assert!(!world.is_ancestor(c.id(), a.id()));

        // Dragging a under its own descendant c would create a cycle → refused.
        world.add_child(c, a);
        assert!(
            world.get_component_ptr(a, std::any::TypeId::of::<Parent>()).is_none(),
            "cyclic reparent must be refused: a stays a root"
        );
        // Self-parenting is also refused.
        world.add_child(a, a);
        assert!(
            world.get_component_ptr(a, std::any::TypeId::of::<Parent>()).is_none(),
            "self-parent must be refused"
        );

        // A legitimate reparent still works: d -> a -> b -> c.
        let d = world.spawn();
        world.add_child(d, a);
        assert!(world.is_ancestor(d.id(), c.id()), "valid reparent kept the chain intact");
    }
}
