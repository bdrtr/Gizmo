//! Which entities are hidden this frame — including the ones hidden by an ancestor.
//!
//! # What was already true, and what the docs said
//!
//! [`IsHidden`](gizmo_core::component::IsHidden) has been honoured by **both** draw loops since
//! 2026-08-19, and it is a *full* hide: the entity is dropped before it becomes a draw item, so it
//! is not drawn and casts nothing. `docs/CAPABILITY_GAPS.md` claimed the opposite until 2026-08-24
//! — *"hiding means `ShadowCasting::Only`, which still casts a shadow"* — and `infinite_grid` said
//! the same, four days after the marker had been wired. Both were measured and corrected:
//! `hiding_an_object_removes_its_shadow_and_shadow_casting_only_keeps_it` renders the two side by
//! side, and an `IsHidden` cube gives a frame **0 pixels** different from having no cube at all
//! while `ShadowCasting::Only` leaves its shadow behind.
//!
//! # What was actually missing
//!
//! Inheritance. Both loops asked `hidden.contains(entity)` per entity, so hiding a parent left its
//! children drawn — measured at **1 886 of 2 946** pixels of a parent/child pair surviving the
//! parent being hidden. Hiding a composite object (a vehicle whose wheels are children) meant
//! walking the tree by hand and getting it wrong once.
//!
//! [`collect_hidden`] is the answer, computed once per frame and shared, because two answers to
//! "is this drawn" is exactly what the 2026-08-19 fix was about.
//!
//! # Cost
//!
//! `O(hidden subtree)`, not `O(entities)`: it starts from the entities that carry the marker, so a
//! frame with nothing hidden allocates an empty set and visits nothing. The alternative — asking
//! each entity to walk its ancestors — is `O(entities × depth)` and pays on every frame whether
//! anything is hidden or not.

use std::collections::HashSet;

use gizmo_core::component::{Children, IsHidden};
use gizmo_core::StorageView;

/// Every entity that must not be drawn: those carrying `IsHidden`, and everything below them.
///
/// The two views are passed rather than a `&World` so the call site keeps naming `IsHidden` — the
/// gate `gizmo-studio/tests/render_parity.rs` asserts is in the *code* of both draw paths, on the
/// grounds that a path which deleted the check and kept the comment would otherwise pass.
///
/// A child id that no longer resolves to an entity is inserted anyway and costs nothing: a plain
/// `despawn` leaves a dangling id in its parent's `Children` (recorded in the capability list),
/// and filtering here would mean a second opinion about what a live entity is.
#[must_use]
pub fn collect_hidden(
    hidden: &StorageView<'_, IsHidden>,
    children: &StorageView<'_, Children>,
) -> HashSet<u32> {
    let mut out: HashSet<u32> = HashSet::new();
    let mut stack: Vec<u32> = hidden.iter().map(|(id, _)| id).collect();
    // The set doubles as the visited set. A `Parent`/`Children` cycle should be impossible, but
    // "should be" is what makes an infinite loop in a render pass; descending only into ids the
    // set did not already have makes the walk terminate whatever the hierarchy looks like.
    while let Some(id) = stack.pop() {
        if !out.insert(id) {
            continue;
        }
        if let Some(kids) = children.get(id) {
            stack.extend(kids.0.iter().copied());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::hierarchy::HierarchyExt;
    use gizmo_core::world::World;

    /// Builds `a -> b -> c` plus an unrelated `d`, and returns their ids in that order.
    fn chain(world: &mut World) -> [u32; 4] {
        let a = world.spawn();
        let b = world.spawn();
        let c = world.spawn();
        let d = world.spawn();
        world.add_child(a, b);
        world.add_child(b, c);
        [a.id(), b.id(), c.id(), d.id()]
    }

    fn hidden_set(world: &World) -> HashSet<u32> {
        let hidden = world.borrow::<IsHidden>();
        let children = world.borrow::<Children>();
        collect_hidden(&hidden, &children)
    }

    #[test]
    fn nothing_hidden_is_an_empty_set() {
        let mut world = World::new();
        chain(&mut world);
        assert!(hidden_set(&world).is_empty());
    }

    /// Hiding a parent hides everything under it, at any depth, and nothing else.
    ///
    /// The grandchild is the half that matters: a one-level implementation passes a
    /// parent-and-child test and leaves a wheel's bolt on screen.
    #[test]
    fn hiding_a_parent_hides_its_whole_subtree_and_no_sibling() {
        let mut world = World::new();
        let [a, b, c, d] = chain(&mut world);
        world.add_component(gizmo_core::entity::Entity::new(a, 0), IsHidden);

        let set = hidden_set(&world);
        assert!(set.contains(&a), "the hidden entity itself");
        assert!(set.contains(&b), "its child");
        assert!(set.contains(&c), "its grandchild — a one-level walk stops here");
        assert!(!set.contains(&d), "an unrelated entity was swept up");
    }

    /// Hiding a child does not hide its parent — inheritance runs one way.
    #[test]
    fn hiding_a_child_leaves_its_parent_alone() {
        let mut world = World::new();
        let [a, b, c, _] = chain(&mut world);
        world.add_component(gizmo_core::entity::Entity::new(b, 0), IsHidden);

        let set = hidden_set(&world);
        assert!(!set.contains(&a));
        assert!(set.contains(&b) && set.contains(&c));
    }

    /// A cycle in the hierarchy terminates instead of hanging the render pass.
    ///
    /// `add_child` should make one impossible; this builds it by writing `Children` directly,
    /// because the guard is worth having precisely for the state nothing is supposed to produce.
    #[test]
    fn a_cycle_terminates() {
        let mut world = World::new();
        let a = world.spawn();
        let b = world.spawn();
        world.add_component(a, Children(vec![b.id()]));
        world.add_component(b, Children(vec![a.id()]));
        world.add_component(a, IsHidden);

        let set = hidden_set(&world);
        assert_eq!(set.len(), 2, "both entities of the cycle, visited once each");
    }

    /// A dangling child id is kept rather than filtered — see [`collect_hidden`].
    #[test]
    fn a_dangling_child_id_is_harmless() {
        let mut world = World::new();
        let a = world.spawn();
        world.add_component(a, Children(vec![9_999]));
        world.add_component(a, IsHidden);
        assert!(hidden_set(&world).contains(&9_999));
    }
}
