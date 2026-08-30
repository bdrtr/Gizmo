//! Parent/child links between entities, as an extension trait on `World`.
//!
//! The hierarchy is two plain components (a parent link and a child list) kept consistent by
//! [`HierarchyExt`]; there is no separate tree structure. Writing those components directly
//! bypasses the bookkeeping and can leave a parent whose children do not point back.
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

    /// Every entity reachable from `root` down the [`Children`] links, `root` itself first,
    /// then breadth-first — **cycle-safe and duplicate-free**.
    ///
    /// This exists because the same child-list descent kept being written by hand, and a sweep
    /// on 2026-08-25 found **six** copies of it in the engine crates with no visited set at all:
    /// two inside per-frame work (`gizmo-physics-rigid`'s compound-collider gather and
    /// `gizmo-animation`'s target resolve), three in `gizmo-studio` (delete cascade, GC cascade,
    /// selection highlight) and one recursive in `gizmo-editor`'s hierarchy panel. Having one
    /// guarded walker is the point: the next person to need this should not have to remember the
    /// guard.
    ///
    /// **The six did not fail the same way**, so no single symptom describes them — and the
    /// difference is not cosmetic, because it decides what a test for one of them can assert.
    /// The three `gizmo-studio` copies walked an index cursor along a vector they never drained,
    /// so a cycle grew it until the process was killed. The two per-frame walks pop before they
    /// push: on the simple cycle A→B→C→A their stack holds ONE id for the whole infinite run, so
    /// the symptom is a frame that never returns at flat memory, and memory climbs only where
    /// the loop body appends per child regardless (`gizmo-physics-rigid`'s `compound_shapes`
    /// does; `gizmo-animation`'s name map does not). The editor's panel recurses, so it
    /// overflows the stack and ABORTS — no unwind, no `catch_unwind`, no chance to save.
    ///
    /// A seventh site is not a walk of ours at all: `gizmo-ui` mirrored each `Children` list
    /// into its layout engine verbatim, and the recursion that then blew the stack was taffy's,
    /// inside `compute_layout`. It is guarded where it mirrors, not by calling this.
    ///
    /// **A cycle is reachable.** `add_child` refuses to build one, but `Children` and [`Parent`]
    /// are ordinary components that `add_component` writes directly, and scene loading writes a
    /// file's parent edges verbatim with no cycle rejection anywhere on that path.
    ///
    /// Duplicate-free matters separately from termination: a *diamond* — the same child id in
    /// two lists — terminates on its own but is visited twice, which double-counts a delete
    /// cascade and adds the same child collider to a compound body twice.
    ///
    /// Ids are returned, not [`Entity`] handles, and are not checked for liveness: a `Children`
    /// list can name an id that has been despawned. Resolve with [`World::entity`] if you need a
    /// handle, and expect `None`.
    fn descendants_inclusive(&self, root: u32) -> Vec<u32>;
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
        // (dragging a node onto its own descendant).
        //
        // This refusal is a courtesy, not a guarantee, and no walker may lean on it: `Parent`
        // and `Children` are ordinary components that `add_component` writes directly, and
        // `SceneData::instantiate_entities` writes a file's parent edges verbatim without
        // passing through here. The three walks this comment used to name as the victims —
        // transform propagation, `despawn_recursive`, scene save — are the wrong list twice
        // over: the first two carry visited sets (the one for `despawn_recursive` is eleven
        // lines above), and the six that genuinely carried none, listed as open work in
        // `docs/ENGINE.md` §3, carry one since 2026-08-30 — three by calling
        // [`HierarchyExt::descendants_inclusive`], three by an inline set of their own.
        // What a cycle costs still depends on who walks it, not on this line: the point of the
        // paragraph is that a walker may not lean on this refusal, and that survives the fix.
        if parent.id() == child.id() || self.is_ancestor(child.id(), parent.id()) {
            return;
        }

        // Remove from old parent first
        if let Some(parent_ptr) = self.get_component_ptr(child, std::any::TypeId::of::<Parent>()) {
            // SAFETY: the pointer was keyed by `TypeId::of::<Parent>()`, so it addresses a live
            // `Parent`; the id is copied out immediately, before anything can move the row.
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
            // SAFETY: keyed by `TypeId::of::<Children>()` and obtained from `&mut self`, so this
            // is the only live reference to that component; it is used and dropped inside this
            // block, with no structural change in between.
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
            // SAFETY: as in `add_child` — right type by construction, exclusive by `&mut self`,
            // consumed before any structural change.
            let children = unsafe { &mut *(children_ptr as *mut Children) };
            children.0.retain(|&id| id != child.id());
        }
    }

    fn descendants_inclusive(&self, root: u32) -> Vec<u32> {
        let children = self.borrow::<Children>();
        let mut out = vec![root];
        let mut seen = std::collections::HashSet::from([root]);
        let mut i = 0;
        // `out` doubles as the queue: everything already in it has been enqueued exactly once,
        // which is what `seen` enforces. Pushing only unseen ids is what makes a cycle
        // terminate and a diamond visit its shared child once.
        while i < out.len() {
            if let Some(list) = children.get(out[i]) {
                for &child in &list.0 {
                    if seen.insert(child) {
                        out.push(child);
                    }
                }
            }
            i += 1;
        }
        out
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
        // SAFETY: keyed by `TypeId::of::<Children>()`, so the type is right. The list is only
        // READ here (the ids are copied into `children_to_despawn`) and the despawns happen
        // after this borrow ends — reading it while despawning would be the dangling case.
        let children = unsafe { &*(children_ptr as *const Children) };
        for &child_id in &children.0 {
            if let Some(child_entity) = world.entity(child_id) {
                children_to_despawn.push(child_entity);
            }
        }
    }

    // Detach from the (surviving) parent's Children list.
    if let Some(parent_ptr) = world.get_component_ptr(entity, std::any::TypeId::of::<Parent>()) {
        // SAFETY: right type by construction; the id is copied out before anything moves the row.
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
            // SAFETY: test-local — the pointer was looked up by `TypeId::of::<Parent>()`, so the cast matches
            // the bytes, and the world outlives this read.
            let parent_id = unsafe { (*(parent_ptr as *const Parent)).0 };
            assert_eq!(parent_id, parent.id());
        } else {
            panic!("Child missing Parent component");
        }

        // Check if Children component is updated
        if let Some(children_ptr) = world.get_component_ptr(parent, std::any::TypeId::of::<Children>()) {
            // SAFETY: test-local — the pointer was looked up by `TypeId::of::<Children>()`, so the cast matches
            // the bytes, and the world outlives this read.
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
            // SAFETY: test-local — the pointer was looked up by `TypeId::of::<Children>()`, so the cast matches
            // the bytes, and the world outlives this read.
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

    /// Runs `f` on a worker thread and fails if it has not finished within `secs`.
    ///
    /// Every guard tested below protects against an UNBOUNDED walk, and an unbounded walk has
    /// no assertion to disagree with: the natural test does not fail, it hangs, and a suite that
    /// hangs covers less than one that goes red — the same argument `CLAUDE.md` makes for
    /// `--no-fail-fast`. The `World` is built inside the closure so nothing has to cross a
    /// thread boundary. A worker left spinning does not keep the harness alive: the process ends
    /// when the main thread is done.
    ///
    /// It bounds the WAIT, not the work. A timeout here means a detached thread is still running
    /// `descendants_inclusive` — which allocates, pushing an id per iteration — for the rest of
    /// the binary. So a red from this helper wants investigating rather than living in the suite
    /// under `--no-fail-fast`; on the ~13 GB machine `CLAUDE.md` describes, leaving one to run is
    /// how a clear failure turns into an allocation abort somewhere unrelated.
    ///
    /// The wall-clock deadline is safe under the Miri job, which runs `hierarchy::tests` by name
    /// with `-Zmiri-disable-isolation` — i.e. against the REAL clock, in an interpreter orders of
    /// magnitude slower than native. Measured 2026-08-30 rather than assumed: the whole filtered
    /// set, all ten tests, takes **4.76 s** under Miri, so each deadline keeps roughly a tenfold
    /// margin and no `#[cfg_attr(miri, ignore)]` is needed to buy it.
    fn within<T: Send + 'static>(secs: u64, f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        match rx.recv_timeout(std::time::Duration::from_secs(secs)) {
            Ok(value) => value,
            // Told apart on purpose. `expect` collapses both arms, and the two mean opposite
            // things: `Disconnected` arrives the instant the worker panics — its own message is
            // already on stdout — and reporting that as "looped for ten seconds" sends the next
            // reader hunting a cycle that never existed.
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("the walk did not terminate within {secs}s — a `Children` cycle looped")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                panic!("the walk panicked before answering; its own message is above")
            }
        }
    }

    /// `Children` written directly, bypassing `add_child`'s cycle refusal — which is exactly how
    /// a cycle reaches the engine in practice, since scene loading writes a file's parent edges
    /// verbatim.
    fn link(world: &mut World, parent: Entity, children: &[Entity]) {
        world.add_component(parent, Children(children.iter().map(|e| e.id()).collect()));
    }

    /// The documented order: `root` first, then breadth-first.
    ///
    /// The fixture BRANCHES on purpose. A chain cannot tell the orders apart — breadth-first,
    /// depth-first and any parent-before-child walk all return `[a, b, c]` — so a chain-shaped
    /// test asserting "breadth-first" claims more than it checks, and a rewrite of the walker
    /// into a `stack.pop()` depth-first would pass it. Here a → [b, c] and b → [d]: breadth-first
    /// is `[a, b, c, d]` and depth-first would be `[a, b, d, c]`.
    #[test]
    fn descendants_are_returned_root_first_then_breadth_first() {
        let ids = within(10, || {
            let mut world = World::new();
            let (a, b, c, d) = (world.spawn(), world.spawn(), world.spawn(), world.spawn());
            world.add_child(a, b);
            world.add_child(a, c);
            world.add_child(b, d);
            (world.descendants_inclusive(a.id()), a.id(), b.id(), c.id(), d.id())
        });
        let (out, a, b, c, d) = ids;
        assert_eq!(out, vec![a, b, c, d], "root, then both siblings, then the grandchild");
    }

    #[test]
    fn an_entity_with_no_children_is_its_own_only_descendant() {
        let (out, a) = within(10, || {
            let mut world = World::new();
            let a = world.spawn();
            (world.descendants_inclusive(a.id()), a.id())
        });
        // The identity, not just the count: `vec![0]` or somebody else's id would satisfy a
        // length check while making the name of this test false.
        assert_eq!(out, vec![a]);
    }

    /// The guard's reason for existing: without it this call never returns.
    #[test]
    fn a_children_cycle_terminates_instead_of_looping() {
        let out = within(10, || {
            let mut world = World::new();
            let (a, b, c) = (world.spawn(), world.spawn(), world.spawn());
            link(&mut world, a, &[b]);
            link(&mut world, b, &[c]);
            link(&mut world, c, &[a]); // …and back to the root.
            world.descendants_inclusive(a.id())
        });
        assert_eq!(out.len(), 3, "each entity of the cycle is visited exactly once");
    }

    /// A one-entity cycle — the shortest one, and the one `add_child` names explicitly.
    #[test]
    fn a_self_parenting_entity_terminates() {
        let out = within(10, || {
            let mut world = World::new();
            let a = world.spawn();
            link(&mut world, a, &[a]); // its own child
            world.descendants_inclusive(a.id())
        });
        assert_eq!(out.len(), 1, "an entity that is its own child is still one entity");
    }

    /// A DIAMOND terminates on its own, so this is not about hanging: it is about counting.
    /// Visiting the shared child twice double-counts a delete cascade and adds the same child
    /// collider to a compound body twice.
    #[test]
    fn a_diamond_visits_its_shared_child_once() {
        let out = within(10, || {
            let mut world = World::new();
            let (a, b, c, d) = (world.spawn(), world.spawn(), world.spawn(), world.spawn());
            link(&mut world, a, &[b, c]);
            link(&mut world, b, &[d]);
            link(&mut world, c, &[d]); // same child, reachable two ways
            (world.descendants_inclusive(a.id()), d.id())
        });
        let (out, d) = out;
        assert_eq!(out.len(), 4, "a, b, c, d — d once, not twice");
        assert_eq!(out.iter().filter(|&&id| id == d).count(), 1);
    }

    /// A `Children` list may name a despawned id; the walk reports it rather than filtering it,
    /// and must not panic on the missing storage.
    #[test]
    fn a_dangling_child_id_is_reported_not_skipped() {
        let out = within(10, || {
            let mut world = World::new();
            let (a, b) = (world.spawn(), world.spawn());
            link(&mut world, a, &[b]);
            world.despawn(b);
            (world.descendants_inclusive(a.id()), b.id())
        });
        let (out, b) = out;
        assert!(out.contains(&b), "the id is returned; liveness is the caller's to check");
    }
}
