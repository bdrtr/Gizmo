//! Links between entities that keep their own reverse index: `Likes(entity)` on one side, and on the
//! other a `LikedBy` listing everyone who likes it, always up to date.
//!
//! [`Parent`](crate::component::Parent) and [`Children`](crate::component::Children) are the pair the
//! engine ships. This is the same idea for a game's own links, declared with [`relationship!`](crate::relationship!):
//!
//! ```
//! use gizmo_core::prelude::*;
//! gizmo_core::relationship!(pub struct Targeting -> TargetedBy;);
//!
//! let mut world = World::new();
//! world.register_relationship::<Targeting>();
//! let boss = world.spawn();
//! let archer = world.spawn_bundle((Targeting(boss),));
//! assert_eq!(world.query::<&TargetedBy>().get(boss).unwrap().sources(), [archer]);
//!
//! world.despawn(archer);
//! assert!(world.query::<&TargetedBy>().get(boss).unwrap().sources().is_empty());
//! ```
//!
//! Both components are immutable: the only way to change a link is to insert another, remove it or
//! despawn an end, and those are the writes the index hears about.
//!
//! ```compile_fail
//! # use gizmo_core::prelude::*;
//! # gizmo_core::relationship!(struct Likes -> LikedBy;);
//! # let mut world = World::new();
//! // A link is not written through, or the index would not hear of it.
//! for mut link in world.query_mut::<&mut Likes>().iter_mut() {
//!     link.0 = Entity::new(0, 0);
//! }
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use crate::component::Component;
use crate::entity::Entity;
use crate::world::World;

/// The side of a link that names its target. Declared with [`relationship!`](crate::relationship!).
pub trait Relationship: Component {
    /// The component the target gets, listing who points at it.
    type Target: RelationshipTarget;
    /// The entity this link points at.
    fn target(&self) -> Entity;
}

/// The side of a link that lists who points here. Declared with [`relationship!`](crate::relationship!), and kept by the
/// world: there is no way to write it from outside.
pub trait RelationshipTarget: Component + Default {
    /// Everything pointing here, in the order the links were made.
    fn sources(&self) -> &[Entity];
    #[doc(hidden)]
    fn sources_mut(&mut self) -> &mut Vec<Entity>;
}

/// What happens to a link's sources when its target is despawned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum OnTargetDespawn {
    /// They lose the link and live on.
    #[default]
    Unlink,
    /// They are despawned with it.
    Despawn,
}

/// Declares a relationship and its reverse index, both immutable components.
///
/// `relationship!(pub struct Likes -> LikedBy;);` makes `Likes(pub Entity)` and a `LikedBy` whose
/// [`sources`](RelationshipTarget::sources) list who likes it. Register it with
/// [`World::register_relationship`] before making links.
#[macro_export]
macro_rules! relationship {
    ($(#[$meta:meta])* $vis:vis struct $rel:ident -> $target:ident;) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        $vis struct $rel(pub $crate::entity::Entity);
        $crate::impl_component!($rel; immutable);

        #[doc = concat!("Who has a [`", stringify!($rel), "`] pointing here. Kept by the world.")]
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        $vis struct $target(::std::vec::Vec<$crate::entity::Entity>);
        $crate::impl_component!($target; immutable);

        impl $crate::relationship::Relationship for $rel {
            type Target = $target;
            fn target(&self) -> $crate::entity::Entity {
                self.0
            }
        }
        impl $crate::relationship::RelationshipTarget for $target {
            fn sources(&self) -> &[$crate::entity::Entity] {
                &self.0
            }
            fn sources_mut(&mut self) -> &mut ::std::vec::Vec<$crate::entity::Entity> {
                &mut self.0
            }
        }
        impl $target {
            /// Everything pointing here, in the order the links were made.
            #[must_use]
            #[allow(dead_code)]
            $vis fn sources(&self) -> &[$crate::entity::Entity] {
                &self.0
            }
        }
    };
}

/// Runs `f` on `entity`'s list, giving it one if it has none and `create` allows it.
fn with_list<R: Relationship>(world: &mut World, entity: Entity, create: bool, f: impl FnOnce(&mut Vec<Entity>)) {
    if !world.is_alive(entity) {
        return;
    }
    let type_id = crate::component_key::ComponentKey::of::<R::Target>();
    // Looked at, not written: the mutable pointer would stamp a change.
    if world.get_component_ptr(entity, type_id).is_none() {
        if !create {
            return;
        }
        world.add_component(entity, R::Target::default());
    }
    if let Some(ptr) = world.get_component_mut_ptr(entity, type_id) {
        // SAFETY: keyed by the `TypeId` of `R::Target`, so the type is right; exclusive by `&mut World`;
        // the reference ends with this call, before any structural change.
        f(unsafe { &mut *(ptr as *mut R::Target) }.sources_mut());
    }
}

/// Moves `source` to `target`'s list, or just off `old`'s for `None`, and records where it is.
fn relink<R: Relationship>(
    world: &mut World,
    links: &Mutex<HashMap<Entity, Entity>>,
    source: Entity,
    target: Option<Entity>,
) {
    let old = {
        let mut links = links.lock().unwrap_or_else(PoisonError::into_inner);
        match target {
            Some(t) => links.insert(source, t),
            None => links.remove(&source),
        }
    };
    if old == target {
        return;
    }
    if let Some(old) = old {
        with_list::<R>(world, old, false, |list| list.retain(|&e| e != source));
    }
    if let Some(target) = target {
        with_list::<R>(world, target, true, |list| list.push(source));
    }
}

impl World {
    /// Starts keeping `R`'s reverse index: its sources lose the link when the target is despawned. See
    /// [`register_relationship_with`](Self::register_relationship_with) to despawn them instead.
    pub fn register_relationship<R: Relationship>(&mut self) {
        self.register_relationship_with::<R>(OnTargetDespawn::Unlink);
    }

    /// [`register_relationship`](Self::register_relationship), choosing what a target's despawn does to
    /// its sources. Links made before this call are not in the index.
    pub fn register_relationship_with<R: Relationship>(&mut self, on_despawn: OnTargetDespawn) {
        // Where each source points. A hook is told the entity and not the value it replaced, and the
        // index has to find the old list to take it off.
        let links: Arc<Mutex<HashMap<Entity, Entity>>> = Arc::default();

        let set = links.clone();
        self.register_on_add::<R>(move |world, source| {
            let target = world.query::<&R>().get(source).map(R::target);
            relink::<R>(world, &set, source, target);
        });
        let set = links.clone();
        self.register_on_replace::<R>(move |world, source| {
            let target = world.query::<&R>().get(source).map(R::target);
            relink::<R>(world, &set, source, target);
        });
        let set = links.clone();
        self.register_on_remove::<R>(move |world, source| {
            relink::<R>(world, &set, source, None);
        });
        // A cloned target comes with its original's list, and nobody points at the clone.
        self.register_on_add::<R::Target>(|world, entity| {
            let Some(list) = world.query::<&R::Target>().get(entity).map(|t| t.sources().to_vec()) else {
                return;
            };
            let real: Vec<Entity> = {
                let links = world.query::<&R>();
                list.into_iter().filter(|&s| links.get(s).map(R::target) == Some(entity)).collect()
            };
            with_list::<R>(world, entity, false, |own| *own = real);
        });
        self.register_despawn_hook(move |world, entity| {
            let Some(list) = world.query::<&R::Target>().get(entity).map(|t| t.sources().to_vec()) else {
                return;
            };
            for source in list.into_iter().filter(|&s| s != entity) {
                match on_despawn {
                    OnTargetDespawn::Unlink => world.remove_component::<R>(source),
                    OnTargetDespawn::Despawn => world.despawn(source),
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::prelude::*;
    use crate::relationship::OnTargetDespawn;

    crate::relationship!(struct Likes -> LikedBy;);

    fn world() -> World {
        let mut world = World::new();
        world.register_relationship::<Likes>();
        world
    }

    fn fans(world: &World, entity: Entity) -> Vec<Entity> {
        world.query::<&LikedBy>().get(entity).map(|l| l.sources().to_vec()).unwrap_or_default()
    }

    /// The whole life of a link: made, moved, removed, in every order the index could get wrong.
    #[test]
    fn the_index_follows_a_link_through_its_life() {
        let mut world = world();
        let (a, b) = (world.spawn(), world.spawn());
        let fan = world.spawn_bundle((Likes(a),));
        let other = world.spawn_bundle((Likes(a),));
        assert_eq!(fans(&world, a), [fan, other], "in the order they were made");

        world.add_component(fan, Likes(b));
        assert_eq!(fans(&world, a), [other], "moved off the old target");
        assert_eq!(fans(&world, b), [fan], "and onto the new one");

        world.add_component(fan, Likes(b));
        assert_eq!(fans(&world, b), [fan], "the same link again adds nothing");

        world.remove_component::<Likes>(other);
        assert!(fans(&world, a).is_empty());

        world.despawn(fan);
        assert!(fans(&world, b).is_empty(), "a despawned source leaves the list");
    }

    /// **Every way of writing a component reaches the index.** These three wrote columns directly and fired
    /// nothing, which made an index kept by hooks a guess; `spawn_batch` told it about one entity in four.
    #[test]
    fn the_bulk_paths_reach_the_index() {
        let mut world = world();
        let target = world.spawn();

        let batch: Vec<Entity> = world.spawn_batch((0..4).map(|_| (Likes(target),))).collect();
        assert_eq!(fans(&world, target), batch);

        let bundled = world.spawn();
        world.add_bundle(bundled, (Likes(target),));
        assert!(fans(&world, target).contains(&bundled));

        let clones = world.clone_entity(bundled.id(), 3).expect("the source is alive");
        let now = fans(&world, target);
        assert!(clones.iter().all(|c| now.contains(c)), "{clones:?} in {now:?}");
        assert_eq!(now.len(), 4 + 1 + 3);
    }

    /// A target's despawn takes the link with it, and with `Despawn` the sources too. A recycled id is not
    /// the old target.
    #[test]
    fn despawning_a_target_settles_its_sources() {
        let mut world = world();
        let target = world.spawn();
        let fan = world.spawn_bundle((Likes(target),));
        world.despawn(target);
        assert!(world.is_alive(fan));
        assert!(world.query::<&Likes>().get(fan).is_none(), "the link went with its target");

        let reborn = (0..64).map(|_| world.spawn()).find(|e| e.id() == target.id()).expect("the id came back");
        assert!(fans(&world, reborn).is_empty(), "a new entity on the id inherits nothing");

        crate::relationship!(struct Owns -> OwnedBy;);
        let mut world = World::new();
        world.register_relationship_with::<Owns>(OnTargetDespawn::Despawn);
        let owner = world.spawn();
        let (x, y) = (world.spawn_bundle((Owns(owner),)), world.spawn_bundle((Owns(owner),)));
        let stranger = world.spawn();
        world.despawn(owner);
        assert!(!world.is_alive(x) && !world.is_alive(y));
        assert!(world.is_alive(stranger));
    }

    /// A clone of a target is not pointed at by its original's sources.
    #[test]
    fn a_cloned_target_starts_with_nobody() {
        let mut world = world();
        let target = world.spawn();
        let fan = world.spawn_bundle((Likes(target),));
        let clones = world.clone_entity(target.id(), 1).expect("alive");
        assert!(fans(&world, clones[0]).is_empty());
        assert_eq!(fans(&world, target), [fan]);
    }

    /// An entity can point at itself, and despawning it does not trip over its own link.
    #[test]
    fn a_link_to_itself_survives_a_despawn() {
        let mut world = world();
        let narcissus = world.spawn();
        world.add_component(narcissus, Likes(narcissus));
        assert_eq!(fans(&world, narcissus), [narcissus]);
        world.despawn(narcissus);
        assert!(!world.is_alive(narcissus));
    }
}
