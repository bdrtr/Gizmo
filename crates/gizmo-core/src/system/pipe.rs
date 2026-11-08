//! `a.pipe(b)`: the value one system returns becomes the first argument of the next.
//!
//! ```
//! # use gizmo_core::prelude::*;
//! # use gizmo_core::system::{In, SystemPipe};
//! # #[derive(Clone)] struct Hp(i32);
//! # gizmo_core::impl_component!(Hp);
//! fn lowest(q: Query<&Hp>) -> Option<i32> {
//!     q.iter().map(|h| h.0).min()
//! }
//! fn warn(In(lowest): In<Option<i32>>) {
//!     if let Some(hp) = lowest {
//!         assert_eq!(hp, 3);
//!     }
//! }
//! let mut world = World::new();
//! world.spawn_bundle((Hp(3),));
//! world.run_system(lowest.pipe(warn));
//! ```
//!
//! The pair is one system to the scheduler: both halves run on the same frame, one after the other, with
//! the union of what they touch. Each half keeps its own [`Local`](super::Local)s. Commands are not applied
//! between them. The last half may return `()` or a `Result`, handled as any system's would be.

use std::marker::PhantomData;

use super::{AccessInfo, IntoSystem, System, SystemOutput, SystemParam};
use crate::world::World;

/// What the previous system returned, as the first parameter of a [`pipe`](SystemPipe::pipe)d one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct In<T>(pub T);

/// One half of a pipe: takes `I`, gives `O`.
pub trait PipeStage<I, O>: Send + Sync + 'static {
    /// Fetches this stage's parameters and runs it.
    fn run_with(&mut self, input: I, world: &World, dt: f32) -> O;
    /// What it touches.
    fn access_info(&self) -> AccessInfo;
}

/// A function that can be a [`PipeStage`]: parameters only for the first, [`In`] then parameters after that.
pub trait IntoPipeStage<I, O, Marker> {
    /// The stage it becomes.
    type Stage: PipeStage<I, O>;
    /// Builds the stage, with its parameters' state.
    fn into_stage(self) -> Self::Stage;
}

/// Pipes one system into another.
pub trait SystemPipe<I, T, Marker>: IntoPipeStage<I, T, Marker> + Sized {
    /// Hands what `self` returns to `next`, whose first parameter is [`In<T>`](In).
    fn pipe<O, NextMarker, Next: IntoPipeStage<T, O, NextMarker>>(self, next: Next) -> Piped<Self::Stage, Next::Stage, T> {
        Piped { first: self.into_stage(), next: next.into_stage(), _value: PhantomData }
    }
}

impl<F: IntoPipeStage<I, T, Marker>, I, T, Marker> SystemPipe<I, T, Marker> for F {}

/// Two stages, the first's output the second's input. A system when it starts from nothing and ends in
/// something a system may return, and a stage in its own right for a longer pipe.
pub struct Piped<A, B, T> {
    first: A,
    next: B,
    _value: PhantomData<fn() -> T>,
}

impl<A, B, I, T, O> PipeStage<I, O> for Piped<A, B, T>
where
    A: PipeStage<I, T>,
    B: PipeStage<T, O>,
    T: 'static,
{
    fn run_with(&mut self, input: I, world: &World, dt: f32) -> O {
        let value = self.first.run_with(input, world, dt);
        self.next.run_with(value, world, dt)
    }

    fn access_info(&self) -> AccessInfo {
        let mut info = self.first.access_info();
        info.merge(self.next.access_info());
        info
    }
}

#[doc(hidden)]
pub struct AlreadyPiped;

impl<A, B, I, T, O> IntoPipeStage<I, O, AlreadyPiped> for Piped<A, B, T>
where
    Piped<A, B, T>: PipeStage<I, O>,
{
    type Stage = Self;
    fn into_stage(self) -> Self {
        self
    }
}

#[doc(hidden)]
pub struct PipedSystem<T, O>(PhantomData<fn() -> (T, O)>);

impl<A, B, T, O> IntoSystem<PipedSystem<T, O>> for Piped<A, B, T>
where
    A: PipeStage<(), T>,
    B: PipeStage<T, O>,
    T: 'static,
    O: SystemOutput + 'static,
{
    fn into_system(self) -> Box<dyn System> {
        struct Run<P, O>(P, PhantomData<fn() -> O>);
        impl<P: PipeStage<(), O>, O: SystemOutput + 'static> System for Run<P, O> {
            fn run(&mut self, world: &World, dt: f32) {
                self.0.run_with((), world, dt).handle(world, std::any::type_name::<P>());
            }
            fn access_info(&self) -> AccessInfo {
                self.0.access_info()
            }
        }
        Box::new(Run::<Self, O>(self, PhantomData))
    }
}

#[doc(hidden)]
pub struct Source<Params>(PhantomData<fn() -> Params>);
#[doc(hidden)]
pub struct Sink<Input, Params>(PhantomData<fn() -> (Input, Params)>);

#[doc(hidden)]
pub struct FnStage<F, Marker, State> {
    func: F,
    state: State,
    _marker: PhantomData<fn() -> Marker>,
}

fn fetch<'w, P: SystemParam>(world: &'w World, dt: f32, state: &'w mut P::State) -> P::Item<'w> {
    match P::fetch(world, dt, state) {
        Ok(value) => value,
        Err(e) => panic!("system parameter '{}' could not be fetched: {e}", std::any::type_name::<P>()),
    }
}

macro_rules! impl_stage {
    ($($P:ident $s:ident),*) => {
        #[allow(non_snake_case, unused_variables, unused_mut, clippy::unused_unit)]
        impl<F, O, $($P),*> PipeStage<(), O> for FnStage<F, Source<($($P,)*)>, ($($P::State,)*)>
        where
            F: FnMut($($P::Item<'_>),*) -> O + FnMut($($P),*) -> O + Send + Sync + 'static,
            O: 'static,
            $($P: SystemParam + 'static,)*
        {
            fn run_with(&mut self, _input: (), world: &World, dt: f32) -> O {
                let Self { func, state, .. } = self;
                let ($($s,)*) = state;
                fn body<F, O, $($P: SystemParam),*>(func: &mut F, params: ($($P::Item<'_>,)*)) -> O
                where
                    F: FnMut($($P::Item<'_>),*) -> O,
                {
                    let ($($P,)*) = params;
                    func($($P),*)
                }
                crate::hotpatch::hot_call!(body::<F, O, $($P),*>, func, ($(fetch::<$P>(world, dt, $s),)*))
            }
            fn access_info(&self) -> AccessInfo {
                let mut info = AccessInfo::new();
                $($P::get_access_info(&mut info);)*
                info
            }
        }

        #[allow(non_snake_case, unused_variables, unused_mut, clippy::unused_unit)]
        impl<F, O, $($P),*> IntoPipeStage<(), O, Source<($($P,)*)>> for F
        where
            F: FnMut($($P::Item<'_>),*) -> O + FnMut($($P),*) -> O + Send + Sync + 'static,
            O: 'static,
            $($P: SystemParam + 'static,)*
        {
            type Stage = FnStage<F, Source<($($P,)*)>, ($($P::State,)*)>;
            fn into_stage(self) -> Self::Stage {
                FnStage { func: self, state: Default::default(), _marker: PhantomData }
            }
        }

        #[allow(non_snake_case, unused_variables, unused_mut, clippy::unused_unit)]
        impl<F, I, O, $($P),*> PipeStage<I, O> for FnStage<F, Sink<I, ($($P,)*)>, ($($P::State,)*)>
        where
            F: FnMut(In<I>, $($P::Item<'_>),*) -> O + FnMut(In<I>, $($P),*) -> O + Send + Sync + 'static,
            I: 'static,
            O: 'static,
            $($P: SystemParam + 'static,)*
        {
            fn run_with(&mut self, input: I, world: &World, dt: f32) -> O {
                let Self { func, state, .. } = self;
                let ($($s,)*) = state;
                fn body<F, I, O, $($P: SystemParam),*>(func: &mut F, input: In<I>, params: ($($P::Item<'_>,)*)) -> O
                where
                    F: FnMut(In<I>, $($P::Item<'_>),*) -> O,
                {
                    let ($($P,)*) = params;
                    func(input, $($P),*)
                }
                crate::hotpatch::hot_call!(body::<F, I, O, $($P),*>, func, In(input), ($(fetch::<$P>(world, dt, $s),)*))
            }
            fn access_info(&self) -> AccessInfo {
                let mut info = AccessInfo::new();
                $($P::get_access_info(&mut info);)*
                info
            }
        }

        #[allow(non_snake_case, unused_variables, unused_mut, clippy::unused_unit)]
        impl<F, I, O, $($P),*> IntoPipeStage<I, O, Sink<I, ($($P,)*)>> for F
        where
            F: FnMut(In<I>, $($P::Item<'_>),*) -> O + FnMut(In<I>, $($P),*) -> O + Send + Sync + 'static,
            I: 'static,
            O: 'static,
            $($P: SystemParam + 'static,)*
        {
            type Stage = FnStage<F, Sink<I, ($($P,)*)>, ($($P::State,)*)>;
            fn into_stage(self) -> Self::Stage {
                FnStage { func: self, state: Default::default(), _marker: PhantomData }
            }
        }
    };
}

impl_stage!();
impl_stage!(P1 s1);
impl_stage!(P1 s1, P2 s2);
impl_stage!(P1 s1, P2 s2, P3 s3);
impl_stage!(P1 s1, P2 s2, P3 s3, P4 s4);
impl_stage!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5);
impl_stage!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5, P6 s6);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prelude::*;
    use std::any::TypeId;

    #[derive(Clone)]
    struct Hp(i32);
    crate::impl_component!(Hp);
    #[derive(Default)]
    struct Seen(Vec<i32>);
    #[derive(Default)]
    struct Bonus(i32);

    fn world() -> World {
        let mut world = World::new();
        world.spawn_bundle((Hp(3),));
        world.spawn_bundle((Hp(7),));
        world.insert_resource(Seen::default());
        world.insert_resource(Bonus(100));
        world
    }

    fn total(q: Query<&Hp>) -> i32 {
        q.iter().map(|h| h.0).sum()
    }
    fn record(In(value): In<i32>, mut seen: ResMut<Seen>) {
        seen.0.push(value);
    }
    fn add_bonus(In(value): In<i32>, bonus: Res<Bonus>) -> i32 {
        value + bonus.0
    }

    /// **The value arrives,** and the second system still has parameters of its own.
    #[test]
    fn what_one_system_returns_the_next_receives() {
        let mut world = world();
        world.run_system(total.pipe(record));
        assert_eq!(world.get_resource::<Seen>().unwrap().0, [10]);
    }

    /// A pipe is a stage, so it goes on: three systems, each given the last's value.
    #[test]
    fn a_pipe_can_be_piped() {
        let mut world = world();
        world.run_system(total.pipe(add_bonus).pipe(add_bonus).pipe(record));
        assert_eq!(world.get_resource::<Seen>().unwrap().0, [210]);
    }

    /// In a schedule it is one system that runs each frame, with the union of what its halves touch, so
    /// it is kept apart from whatever conflicts with either half.
    #[test]
    fn the_pair_declares_what_both_halves_touch() {
        let system = total.pipe(record).into_system();
        let info = system.access_info();
        assert_eq!(info.component_reads, [TypeId::of::<Hp>()]);
        assert_eq!(info.resource_writes, [TypeId::of::<Seen>()]);

        let mut world = world();
        let mut schedule = Schedule::new();
        schedule.add_di_system(total.pipe(record));
        schedule.run(&mut world, 0.016);
        schedule.run(&mut world, 0.016);
        assert_eq!(world.get_resource::<Seen>().unwrap().0, [10, 10]);
    }

    /// Each half keeps its own `Local`, across runs.
    #[test]
    fn each_half_keeps_its_own_state() {
        fn count(mut n: Local<i32>) -> i32 {
            *n += 1;
            *n
        }
        fn add(In(value): In<i32>, mut n: Local<i32>, mut seen: ResMut<Seen>) {
            *n += 10;
            seen.0.push(value + *n);
        }
        let mut world = world();
        let mut schedule = Schedule::new();
        schedule.add_di_system(count.pipe(add));
        schedule.run(&mut world, 0.016);
        schedule.run(&mut world, 0.016);
        assert_eq!(world.get_resource::<Seen>().unwrap().0, [11, 22]);
    }

    /// The last half may fail like any system: its `Err` goes where a system's does.
    #[test]
    fn the_last_half_may_return_a_result() {
        fn check(In(value): In<i32>) -> Result<(), String> {
            if value > 5 { Err(format!("{value} is too many")) } else { Ok(()) }
        }
        fn remember(_system: &str, error: &dyn std::fmt::Display) {
            assert!(error.to_string().contains("10 is too many"), "{error}");
            REPORTED.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let mut world = world();
        world.insert_resource(crate::system::SystemErrorHandler(remember));
        world.run_system(total.pipe(check));
        assert!(REPORTED.load(std::sync::atomic::Ordering::SeqCst), "the handler never heard of it");
    }
}
