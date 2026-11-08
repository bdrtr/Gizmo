//! Components defined while the program runs: `World::register_component`, the by-key insert and remove,
//! `DynamicQuery` and `DynamicSystem`.

use std::alloc::Layout;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gizmo_core::component::StorageType;
use gizmo_core::component_key::ComponentKey;
use gizmo_core::dynamic::{ComponentDescriptor, DynamicComponentError, DynamicQuery, DynamicSystem};
use gizmo_core::prelude::*;
use gizmo_core::system::{IntoSystem, System};
use gizmo_core::world::CloneEntityError;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Position(f32);
gizmo_core::impl_component!(Position);

fn f32_component(world: &mut World, name: &str) -> ComponentKey {
    world.register_component(ComponentDescriptor::new(name, Layout::new::<f32>())).expect("a new name")
}

fn insert_f32(world: &mut World, entity: Entity, key: ComponentKey, value: f32) {
    let mut value = value;
    // SAFETY: an aligned `f32`, the layout every component here is registered with; plain data.
    unsafe { world.insert_by_key(entity, key, (&raw mut value).cast()) }.expect("alive and registered");
}

fn read_f32(world: &World, entity: Entity, key: ComponentKey) -> Option<f32> {
    // SAFETY: every component read here was registered as an `f32`.
    world.get_component_ptr(entity, key).map(|p| unsafe { *p.cast::<f32>() })
}

/// The whole life of a runtime component: stored, found by a query, read back, replaced, removed, and
/// gone with its entity.
#[test]
fn a_runtime_component_is_stored_queried_replaced_and_removed() {
    let mut world = World::new();
    let health = f32_component(&mut world, "Health");
    let [a, b, c] = [world.spawn(), world.spawn(), world.spawn()];
    for (e, v) in [(a, 10.0), (b, 20.0), (c, 30.0)] {
        insert_f32(&mut world, e, health, v);
    }
    world.add_component(b, Position(1.0));

    let all = DynamicQuery::builder().read(health).build(&world).unwrap();
    let mut sum = 0.0;
    all.iter(&world).for_each(|row| sum += unsafe { *row.ptr(0).cast::<f32>() });
    assert_eq!(sum, 60.0, "every entity with Health, across the two archetypes it is in");

    insert_f32(&mut world, a, health, 11.0);
    assert_eq!(read_f32(&world, a, health), Some(11.0), "an insert over an existing value replaces it");

    assert!(world.remove_by_key(a, health));
    assert!(!world.remove_by_key(a, health), "nothing left to remove");
    assert_eq!(read_f32(&world, a, health), None);
    world.despawn(c);
    assert_eq!(all.iter(&world).count(), 1, "only b keeps one");
    assert_eq!(read_f32(&world, b, health), Some(20.0), "and the migrations did not disturb its value");
    assert_eq!(world.get::<Position>(b), Some(&Position(1.0)));
}

static DROPS: AtomicUsize = AtomicUsize::new(0);
static CLONES: AtomicUsize = AtomicUsize::new(0);

/// A value that owns something: a heap `u64` behind a pointer, as a scripting runtime's handle would be.
unsafe fn drop_boxed(ptr: *mut u8) {
    DROPS.fetch_add(1, Ordering::SeqCst);
    // SAFETY: the component's value is a `*mut u64` from `Box::into_raw`, dropped once.
    drop(unsafe { Box::from_raw(*ptr.cast::<*mut u64>()) });
}

unsafe fn clone_boxed(src: *const u8, dst: *mut u8, count: usize) {
    for i in 0..count {
        CLONES.fetch_add(1, Ordering::SeqCst);
        // SAFETY: `src` holds a live `*mut u64`; `dst` has room for `count` pointers.
        unsafe {
            let value = **src.cast::<*mut u64>();
            dst.cast::<*mut u64>().add(i).write(Box::into_raw(Box::new(value)));
        }
    }
}

fn insert_boxed(world: &mut World, entity: Entity, key: ComponentKey, value: u64) {
    let mut ptr = Box::into_raw(Box::new(value));
    // SAFETY: the component's value is a `*mut u64` from `Box::into_raw`, now the world's.
    unsafe { world.insert_by_key(entity, key, (&raw mut ptr).cast()) }.unwrap();
}

fn read_boxed(world: &World, entity: Entity, key: ComponentKey) -> Option<u64> {
    // SAFETY: the component's value is a live `*mut u64`.
    world.get_component_ptr(entity, key).map(|p| unsafe { **p.cast::<*mut u64>() })
}

/// Drop glue runs once for every value the world lets go of — replaced, removed, despawned, or dropped
/// with the world — and clone glue makes values the world drops on their own. Run under Miri or a leak
/// checker, a missed or doubled drop is a failure; here it is counted.
#[test]
fn drop_and_clone_glue_run_once_per_value() {
    let layout = Layout::new::<*mut u64>();
    for storage in [StorageType::Table, StorageType::SparseSet] {
        DROPS.store(0, Ordering::SeqCst);
        CLONES.store(0, Ordering::SeqCst);
        {
            let mut world = World::new();
            // SAFETY: `drop_boxed` and `clone_boxed` are this component's glue: a `*mut u64` from a `Box`.
            let descriptor = unsafe {
                ComponentDescriptor::new("Boxed", layout).with_drop(drop_boxed).with_clone(clone_boxed)
            }
            .with_storage(storage);
            let boxed = world.register_component(descriptor).unwrap();
            let [a, b, c] = [world.spawn(), world.spawn(), world.spawn()];
            insert_boxed(&mut world, a, boxed, 1);
            insert_boxed(&mut world, b, boxed, 2);
            insert_boxed(&mut world, c, boxed, 3);
            insert_boxed(&mut world, a, boxed, 4); // replaces 1
            assert_eq!(DROPS.load(Ordering::SeqCst), 1, "{storage:?}: the replaced value");
            assert_eq!(read_boxed(&world, a, boxed), Some(4));
            world.add_component(b, Position(0.0)); // a migration moves the value, does not drop it
            assert_eq!(DROPS.load(Ordering::SeqCst), 1, "{storage:?}: a move is not a drop");
            assert!(world.remove_by_key(b, boxed));
            assert_eq!(DROPS.load(Ordering::SeqCst), 2, "{storage:?}: the removed value");
            world.despawn(c);
            assert_eq!(DROPS.load(Ordering::SeqCst), 3, "{storage:?}: the despawned value");

            let clones = world.clone_entity(a.id(), 2).expect("it has clone glue");
            assert_eq!(CLONES.load(Ordering::SeqCst), 2, "{storage:?}");
            for clone in clones {
                assert_eq!(read_boxed(&world, clone, boxed), Some(4), "{storage:?}: the clone's own copy");
            }
        }
        // a and its two clones went with the world.
        assert_eq!(DROPS.load(Ordering::SeqCst), 6, "{storage:?}: every value, once");
    }
}

/// Plain bytes clone by copying; a component with drop glue and none to clone with is refused, and
/// `clone_entity` says which — before it has made anything.
#[test]
fn clone_entity_copies_plain_bytes_and_refuses_what_it_cannot_clone() {
    let mut world = World::new();
    let speed = f32_component(&mut world, "Speed");
    let plain = world.spawn();
    insert_f32(&mut world, plain, speed, 3.5);
    let clones = world.clone_entity(plain.id(), 2).unwrap();
    assert!(clones.iter().all(|&e| read_f32(&world, e, speed) == Some(3.5)));

    // Its own drop glue, not `drop_boxed`: tests run in parallel, and that one's counter belongs to
    // `drop_and_clone_glue_run_once_per_value`.
    unsafe fn free_boxed(ptr: *mut u8) {
        // SAFETY: the component's value is a `*mut u64` from `Box::into_raw`, dropped once.
        drop(unsafe { Box::from_raw(*ptr.cast::<*mut u64>()) });
    }
    // SAFETY: `free_boxed` is the glue of a `*mut u64` from a `Box`.
    let owning = world
        .register_component(unsafe { ComponentDescriptor::new("Owning", Layout::new::<*mut u64>()).with_drop(free_boxed) })
        .unwrap();
    let holder = world.spawn();
    insert_boxed(&mut world, holder, owning, 9);
    let before = world.entity_count();
    assert_eq!(world.clone_entity(holder.id(), 1), Err(CloneEntityError::NotCloneable(owning)));
    assert_eq!(world.entity_count(), before, "nothing was made");
    assert_eq!(world.clone_entity(u32::MAX, 1), Err(CloneEntityError::NoSuchEntity(u32::MAX)));
}

/// A name is a runtime component's identity within a world, and is found again by it; another world
/// has its own.
#[test]
fn a_name_is_taken_once_per_world() {
    let mut world = World::new();
    let first = f32_component(&mut world, "Mana");
    assert_eq!(
        world.register_component(ComponentDescriptor::new("Mana", Layout::new::<u8>())),
        Err(DynamicComponentError::NameTaken(first))
    );
    assert_eq!(world.component_key_by_name("Mana"), Some(first));
    assert_eq!(world.component_key_by_name("Stamina"), None);
    assert_eq!(world.component_type_name(first), Some("Mana"));
    assert!(first.type_id().is_none() && first.dynamic().is_some());

    let mut other = World::new();
    let theirs = f32_component(&mut other, "Mana");
    assert_eq!(other.component_key_by_name("Mana"), Some(theirs));
    let e = world.spawn();
    let mut v = 1.0f32;
    // A key another world minted is not one this world knows — unless it happens to have minted the
    // same index, which `first` and `theirs` show is possible; so this asks with an unused one.
    let unknown = f32_component(&mut other, "Unknown to world");
    assert_eq!(
        // SAFETY: refused before anything is read.
        unsafe { world.insert_by_key(e, unknown, (&raw mut v).cast()) },
        Err(DynamicComponentError::NotRegistered(unknown))
    );
    world.despawn(e);
    assert_eq!(
        // SAFETY: refused before anything is read.
        unsafe { world.insert_by_key(e, first, (&raw mut v).cast()) },
        Err(DynamicComponentError::NoSuchEntity(e))
    );
}

#[repr(align(16))]
struct Aligned([u8; 16]);

/// A layout's size is rounded up to its alignment, so every value in a column is aligned.
#[test]
fn values_are_aligned_to_the_layout() {
    let mut world = World::new();
    let wide = world.register_component(ComponentDescriptor::new("Wide", Layout::from_size_align(4, 16).unwrap())).unwrap();
    assert_eq!(world.component_info(wide).unwrap().layout.size(), 16);
    for i in 0..5 {
        let e = world.spawn();
        let mut value = Aligned([i; 16]);
        // SAFETY: 16 aligned bytes, the padded layout; plain data.
        unsafe { world.insert_by_key(e, wide, (&raw mut value).cast()) }.unwrap();
    }
    let q = DynamicQuery::builder().read(wide).build(&world).unwrap();
    let mut seen = Vec::new();
    q.iter(&world).for_each(|row| {
        assert_eq!(row.ptr(0) as usize % 16, 0);
        // SAFETY: `Wide` values are `Aligned`s.
        seen.push(unsafe { (*row.ptr(0).cast::<Aligned>()).0[15] });
    });
    assert_eq!(seen, vec![0, 1, 2, 3, 4]);
}

/// A builder refuses two views of one row with one of them mutable, a write of an immutable component,
/// and a key the world does not know.
#[test]
fn the_builder_refuses_what_a_typed_query_cannot_express() {
    let mut world = World::new();
    let hp = f32_component(&mut world, "Hp");
    let id = world.register_component(ComponentDescriptor::new("Id", Layout::new::<u32>()).immutable()).unwrap();
    let conflict = Err(DynamicComponentError::Conflict(hp));
    assert_eq!(DynamicQuery::builder().write(hp).read(hp).build(&world).map(|_| ()), conflict);
    assert_eq!(DynamicQuery::builder().write(hp).write(hp).build(&world).map(|_| ()), conflict);
    assert_eq!(DynamicQuery::builder().write(hp).changed(hp).build(&world).map(|_| ()), conflict);
    assert!(DynamicQuery::builder().read(hp).read(hp).changed(hp).build(&world).is_ok(), "reads share");
    assert!(DynamicQuery::builder().write(hp).with(hp).build(&world).is_ok(), "`with` touches no memory");
    assert_eq!(
        DynamicQuery::builder().write(id).build(&world).map(|_| ()),
        Err(DynamicComponentError::Immutable(id))
    );
    assert!(DynamicQuery::builder().read(id).build(&world).is_ok());
    let position = ComponentKey::of::<Position>();
    assert_eq!(
        DynamicQuery::builder().read(position).build(&world).map(|_| ()),
        Err(DynamicComponentError::NotRegistered(position)),
        "a type is known once registered"
    );
    world.register_component_type::<Position>();
    assert!(DynamicQuery::builder().read(position).build(&world).is_ok());
}

/// Typed and runtime components meet in one query, filters included, and in both storages.
#[test]
fn one_query_mixes_types_and_runtime_components_in_both_storages() {
    for storage in [StorageType::Table, StorageType::SparseSet] {
        let mut world = World::new();
        let tag = world
            .register_component(ComponentDescriptor::new("Tag", Layout::new::<()>()).with_storage(storage))
            .unwrap();
        let weight = f32_component(&mut world, "Weight");
        let mut expected = Vec::new();
        for i in 0..6 {
            let e = world.spawn();
            world.add_component(e, Position(i as f32));
            insert_f32(&mut world, e, weight, 2.0 * i as f32);
            if i % 2 == 0 {
                // SAFETY: a zero-sized component; the pointer is aligned and nothing is read.
                unsafe { world.insert_by_key(e, tag, std::ptr::NonNull::<u8>::dangling().as_ptr()) }.unwrap();
            } else {
                expected.push(e);
            }
        }
        let q = DynamicQuery::builder()
            .read(ComponentKey::of::<Position>())
            .write(weight)
            .without(tag)
            .build(&world)
            .unwrap();
        let mut found = Vec::new();
        let mut rows = q.iter_mut(&mut world);
        while let Some(mut row) = rows.fetch_next() {
            // SAFETY: term 0 is a `Position`, term 1 an `f32`.
            let x = unsafe { (*row.ptr(0).cast::<Position>()).0 };
            unsafe { *row.ptr_mut(1).cast::<f32>() += x };
            found.push(row.entity());
        }
        assert_eq!(found, expected, "{storage:?}: the untagged ones");
        for &e in &expected {
            let i = world.get::<Position>(e).unwrap().0;
            assert_eq!(read_f32(&world, e, weight), Some(3.0 * i), "{storage:?}");
        }
        let tagged = DynamicQuery::builder().with(tag).build(&world).unwrap();
        assert_eq!(tagged.iter(&world).count(), 3, "{storage:?}");
    }
}

/// A dynamic system's writes are writes: a `changed` filter in another system sees each one once, and
/// the scheduler keeps a typed writer of the same component out of its batch.
#[test]
fn a_dynamic_system_is_scheduled_and_change_tracked_like_a_typed_one() {
    let mut world = World::new();
    let fuel = f32_component(&mut world, "Fuel");
    let tanks: Vec<Entity> = (0..3).map(|_| world.spawn()).collect();
    for &t in &tanks {
        insert_f32(&mut world, t, fuel, 10.0);
    }
    let burns = Arc::new(Mutex::new(true));
    let burn = DynamicQuery::builder().write(fuel).build(&world).unwrap();
    let burning = burns.clone();
    let burner = DynamicSystem::new(vec![burn], move |queries, dt| {
        if !*burning.lock().unwrap() {
            return;
        }
        let mut rows = queries.rows(0);
        while let Some(mut row) = rows.fetch_next() {
            // SAFETY: term 0 is `Fuel`, an `f32`.
            unsafe { *row.ptr_mut(0).cast::<f32>() -= dt };
        }
    });

    let seen = Arc::new(AtomicUsize::new(0));
    let changed = DynamicQuery::builder().read(fuel).changed(fuel).build(&world).unwrap();
    let counter = seen.clone();
    let watcher = DynamicSystem::new(vec![changed], move |queries, _| {
        counter.fetch_add(queries.rows(0).count(), Ordering::SeqCst);
    });

    let watch_access = watcher.access_info();
    let burn_access = burner.access_info();
    assert!(!burn_access.is_compatible_with(&watch_access), "a write and a tick read of Fuel");
    assert_eq!(burn_access.component_writes, vec![fuel]);

    let mut schedule = Schedule::new();
    schedule.add_systems(burner.label("burn"));
    schedule.add_systems(watcher.after("burn"));
    schedule.run(&mut world, 1.0);
    assert_eq!(seen.load(Ordering::SeqCst), 3, "the first run sees every insert and write");
    schedule.run(&mut world, 1.0);
    assert_eq!(seen.load(Ordering::SeqCst), 6, "each write once more");
    *burns.lock().unwrap() = false;
    schedule.run(&mut world, 1.0);
    assert_eq!(seen.load(Ordering::SeqCst), 6, "nothing written, nothing seen");
    assert!(tanks.iter().all(|&t| read_f32(&world, t, fuel) == Some(8.0)));

    // A typed writer of a type and a dynamic write of the same type are one write.
    world.register_component_type::<Position>();
    let dynamic = DynamicSystem::new(
        vec![DynamicQuery::builder().write(ComponentKey::of::<Position>()).build(&world).unwrap()],
        |_, _| {},
    );
    fn typed(mut q: Query<&mut Position>) {
        for mut p in q.iter_mut() {
            p.0 += 1.0;
        }
    }
    let typed_access = IntoSystem::into_system(typed).access_info();
    assert!(!dynamic.access_info().is_compatible_with(&typed_access));
}

/// Hooks hear a runtime component as they hear a type: add, set, replace and remove.
#[test]
fn hooks_hear_a_runtime_component() {
    let mut world = World::new();
    let hp = f32_component(&mut world, "Hp");
    let log = Arc::new(Mutex::new(Vec::new()));
    let hooks = world.component_hooks_mut(hp);
    for (list, name) in [(&mut hooks.on_add, "add"), (&mut hooks.on_set, "set"), (&mut hooks.on_replace, "replace"), (&mut hooks.on_remove, "remove")] {
        let log = log.clone();
        list.push(Box::new(move |_, _| log.lock().unwrap().push(name)));
    }
    let e = world.spawn();
    insert_f32(&mut world, e, hp, 1.0);
    insert_f32(&mut world, e, hp, 2.0);
    world.remove_by_key(e, hp);
    assert_eq!(*log.lock().unwrap(), ["add", "set", "set", "replace", "remove"]);
}
