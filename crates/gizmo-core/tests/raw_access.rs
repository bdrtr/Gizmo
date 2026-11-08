//! The type-erased component access the hierarchy, relationships, reflection and the editor go through:
//! `World::get_component_ptr` and `World::get_component_mut_ptr`.

use std::any::TypeId;

use gizmo_core::component::Children;
use gizmo_core::hierarchy::HierarchyExt;
use gizmo_core::prelude::*;

#[derive(Clone, Default)]
struct Score(u32);
gizmo_core::impl_component!(Score);

#[derive(Clone, Default)]
struct Seen(Vec<Entity>);
gizmo_core::impl_component!(Seen);

fn record_changed_children(q: Query<Entity, Changed<Children>>, mut seen: ResMut<Seen>) {
    seen.0 = q.iter().collect();
}

/// **A write through the raw mutable pointer is a write.** `add_child` on a parent that already has a
/// list pushes into it through `get_component_mut_ptr`, and that pointer stamped no change tick, so a
/// `Changed<Children>` system never saw the second child arrive.
#[test]
fn adding_a_second_child_marks_the_list_changed() {
    let mut world = World::new();
    world.insert_resource(Seen::default());
    let (parent, first, second) = (world.spawn(), world.spawn(), world.spawn());
    world.add_child(parent, first);
    let mut schedule = Schedule::new();
    schedule.add_systems(record_changed_children);
    schedule.run(&mut world, 0.016);
    assert_eq!(world.get_resource::<Seen>().map(|s| s.0.clone()), Some(vec![parent]), "the list arriving is a change");
    schedule.run(&mut world, 0.016);
    assert_eq!(world.get_resource::<Seen>().map(|s| s.0.clone()), Some(vec![]), "and is seen once");

    world.add_child(parent, second);
    schedule.run(&mut world, 0.016);
    assert_eq!(world.get_resource::<Seen>().map(|s| s.0.clone()), Some(vec![parent]), "the second child is a change too");
}

/// **A dead entity's handle reads nothing.** Its id goes back to the allocator and the next spawn gets it
/// with a new generation; the raw pointer looked the id up without asking which generation it was, and
/// handed back the new entity's component.
#[test]
fn a_stale_handle_reads_nothing_through_the_raw_pointer() {
    let mut world = World::new();
    let old = world.spawn();
    world.add_component(old, Score(1));
    world.despawn(old);
    let new = world.spawn();
    world.add_component(new, Score(2));
    assert_eq!(new.id(), old.id(), "the id was reused, which is what makes the stale handle dangerous");
    assert!(world.get_component_ptr(old, TypeId::of::<Score>()).is_none(), "a stale handle read the new entity");
    assert!(world.get_component_mut_ptr(old, TypeId::of::<Score>()).is_none(), "and could write it");
    let ptr = world.get_component_ptr(new, TypeId::of::<Score>()).expect("the live entity's own");
    // SAFETY: keyed by `Score`'s `TypeId` on a live entity, read before any structural change.
    assert_eq!(unsafe { (*(ptr as *const Score)).0 }, 2);
}
