use criterion::Criterion;
use gizmo_core::{
    component::Component,
    world::World,
};
use super::common::*;

use gizmo_core::hierarchy::HierarchyExt;
use gizmo_core::component::{Children, Parent};

#[derive(Clone, Copy)]
struct C<const N: usize>(Mat4);
impl<const N: usize> Default for C<N> {
    fn default() -> Self {
        Self(Mat4([0.0; 16]))
    }
}
impl<const N: usize> Component for C<N> {
    fn storage_type() -> gizmo_core::component::StorageType { gizmo_core::component::StorageType::Table }
}

fn bench_clone(
    b: &mut criterion::Bencher,
    bundle_size: usize,
) {
    let mut world = World::new();

    // Spawn the first entity, which will be cloned in the benchmark routine.
    let id = world.spawn();
    world.add_component(id, C::<1>::default());
    if bundle_size > 1 {
        world.add_component(id, C::<2>::default());
        world.add_component(id, C::<3>::default());
        world.add_component(id, C::<4>::default());
        world.add_component(id, C::<5>::default());
        world.add_component(id, C::<6>::default());
        world.add_component(id, C::<7>::default());
        world.add_component(id, C::<8>::default());
        world.add_component(id, C::<9>::default());
        world.add_component(id, C::<10>::default());
    }

    let eid = id.id();

    b.iter(|| {
        world.clone_entity(eid, 1);
    });
}

fn clone_hierarchy_recursive(world: &mut World, source_id: u32) -> Option<gizmo_core::entity::Entity> {
    let cloned_entities = world.clone_entity(source_id, 1)?;
    let cloned_parent = cloned_entities[0];

    let mut children_to_clone = Vec::new();
    let source_entity = world.reconstruct_entity(source_id)?;
    if let Some(children_ptr) = world.get_component_ptr(source_entity, core::any::TypeId::of::<Children>()) {
        let children = unsafe { &*(children_ptr as *const Children) };
        children_to_clone = children.0.clone();
    }

    // Since Gizmo's clone_entity copies all components including Parent/Children,
    // the cloned parent currently points to the old children!
    // We must clear its Children component first before adding new ones.
    world.remove_component::<Children>(cloned_parent);
    // It also points to the old parent, we remove it.
    world.remove_component::<Parent>(cloned_parent);

    for child_id in children_to_clone {
        if let Some(cloned_child) = clone_hierarchy_recursive(world, child_id) {
            world.add_child(cloned_parent, cloned_child);
        }
    }

    Some(cloned_parent)
}

fn bench_clone_hierarchy(
    b: &mut criterion::Bencher,
    height: usize,
    children: usize,
    complex: bool,
) {
    let mut world = World::new();

    let root = world.spawn();
    world.add_component(root, C::<1>::default());
    if complex {
        world.add_component(root, C::<2>::default());
        world.add_component(root, C::<3>::default());
        world.add_component(root, C::<4>::default());
        world.add_component(root, C::<5>::default());
        world.add_component(root, C::<6>::default());
        world.add_component(root, C::<7>::default());
        world.add_component(root, C::<8>::default());
        world.add_component(root, C::<9>::default());
        world.add_component(root, C::<10>::default());
    }

    let mut hierarchy_level = vec![root];

    for _ in 0..height {
        let current_hierarchy_level = hierarchy_level.clone();
        hierarchy_level.clear();

        for parent in current_hierarchy_level {
            for _ in 0..children {
                let child = world.spawn();
                world.add_component(child, C::<1>::default());
                if complex {
                    world.add_component(child, C::<2>::default());
                    world.add_component(child, C::<3>::default());
                    world.add_component(child, C::<4>::default());
                    world.add_component(child, C::<5>::default());
                    world.add_component(child, C::<6>::default());
                    world.add_component(child, C::<7>::default());
                    world.add_component(child, C::<8>::default());
                    world.add_component(child, C::<9>::default());
                    world.add_component(child, C::<10>::default());
                }
                world.add_child(parent, child);
                hierarchy_level.push(child);
            }
        }
    }

    let root_id = root.id();
    b.iter(|| {
        clone_hierarchy_recursive(&mut world, root_id);
    });
}

pub fn single_clone(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_clone");
    group.throughput(criterion::Throughput::Elements(1));
    group.bench_function("complex_bundle", |b| {
        bench_clone(b, 10);
    });
    group.finish();
}

pub fn hierarchy_tall(c: &mut Criterion) {
    let mut group = c.benchmark_group("hierarchy_tall");
    group.throughput(criterion::Throughput::Elements(51));
    group.bench_function("tall", |b| {
        bench_clone_hierarchy(b, 50, 1, false);
    });
    group.finish();
}

pub fn hierarchy_wide(c: &mut Criterion) {
    let mut group = c.benchmark_group("hierarchy_wide");
    group.throughput(criterion::Throughput::Elements(51));
    group.bench_function("wide", |b| {
        bench_clone_hierarchy(b, 1, 50, false);
    });
    group.finish();
}

pub fn hierarchy_many(c: &mut Criterion) {
    let mut group = c.benchmark_group("hierarchy_many");
    group.throughput(criterion::Throughput::Elements(364));
    group.bench_function("many", |b| {
        bench_clone_hierarchy(b, 5, 3, true);
    });
    group.finish();
}
