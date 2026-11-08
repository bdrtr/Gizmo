//! A crate outside `gizmo-core` writes a primitive system parameter of its own: one that reaches into
//! the world itself, rather than grouping others as `system_param!` does. It is an
//! `unsafe impl SystemParam`, whose contract is that the access it declares covers what it touches.

use std::any::TypeId;
use std::marker::PhantomData;

use gizmo_core::prelude::*;
use gizmo_core::component_key::ComponentKey;
use gizmo_core::system::{AccessInfo, IntoSystem, SystemParamFetchError};

#[derive(Clone, Default)]
struct Pos;
gizmo_core::impl_component!(Pos);

#[derive(Clone, Default)]
struct Seen(usize);
gizmo_core::impl_component!(Seen);

/// How many entities carry `T`, counted from the world as the system starts.
struct Count<T>(usize, PhantomData<T>);

// SAFETY: `fetch` reads `T`'s column and nothing else, and `get_access_info` declares that read.
unsafe impl<T: Component> SystemParam for Count<T> {
    type Item<'w> = Count<T>;
    type State = ();

    fn fetch<'w>(world: &'w World, _dt: f32, _state: &'w mut ()) -> Result<Count<T>, SystemParamFetchError> {
        Ok(Count(world.query::<&T>().iter().count(), PhantomData))
    }

    fn get_access_info(info: &mut AccessInfo) {
        info.component_reads.push(ComponentKey::of::<T>());
    }
}

fn count_positions(count: Count<Pos>, mut seen: ResMut<Seen>) {
    seen.0 = count.0;
}

/// It runs in a schedule like any built-in parameter, next to one.
#[test]
fn a_parameter_written_downstream_runs_in_a_schedule() {
    let mut world = World::new();
    world.insert_resource(Seen(0));
    for _ in 0..3 {
        let e = world.spawn();
        world.add_component(e, Pos);
    }
    let mut schedule = Schedule::new();
    schedule.add_systems(count_positions);
    schedule.run(&mut world, 0.016);
    assert_eq!(world.get_resource::<Seen>().map(|s| s.0), Some(3));
}

/// What it declares is what the scheduler sees: a system taking it reads `Pos`, so a system writing
/// `Pos` cannot share its batch.
#[test]
fn its_declaration_reaches_the_scheduler() {
    let system = count_positions.into_system();
    let access = system.access_info();
    assert!(access.component_reads.contains(&ComponentKey::of::<Pos>()), "the read of Pos reached the system");
    assert!(access.resource_writes.contains(&TypeId::of::<Seen>()), "and the built-in parameter's write beside it");
}
