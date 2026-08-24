use super::*;
use crate::world::World;
use std::any::TypeId;

// ==============================================================
// DEPENDENCY INJECTION SİSTEMİ
// ==============================================================

use crate::world::{ResourceReadGuard, ResourceWriteGuard};

/// Why a [`SystemParam`] could not be produced for a run.
///
/// Systems built through [`IntoSystem`] never hand this to user code: the generated runner
/// panics with a diagnostic instead, and the typed [`IntoCondition`] path unwraps it. A
/// parameter that cannot be fetched is treated as a setup mistake, not a recoverable state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SystemParamFetchError {
    /// A [`Res`]/[`ResMut`] — or a parameter built on top of one, such as `Commands` — could
    /// not be borrowed.
    ///
    /// Either no value of that type was inserted into the world, or its lock was taken in an
    /// incompatible way: a live `ResMut<T>` blocks every other borrow of `T`, including a
    /// second `ResMut<T>` in the *same* parameter list, and a lock poisoned by an earlier
    /// panic reports the same way. The borrow is attempted, never waited on, so a
    /// self-conflict surfaces as this error rather than as a deadlock.
    Resource(crate::world::ResourceFetchError),
    /// Query construction returned nothing.
    ///
    /// Unreachable through the built-in `Query` parameter: that constructor never returns
    /// `None`. It is not *infallible*, though — asking for the same component mutably twice
    /// in one query (`Query<(Mut<T>, Mut<T>)>`) panics inside the aliasing check rather than
    /// surfacing here. The variant exists as the failure channel should construction start
    /// reporting failures instead.
    QueryError,
}

impl SystemParamFetchError {
    /// Whether this is "the thing was never there", as opposed to "it is there and I could not
    /// have it".
    ///
    /// The distinction is what makes [`Option<P>`](SystemParam) safe to offer: an absent resource
    /// is a state a game can reasonably be in and recover from, while a borrow conflict is a
    /// scheduling bug — two `ResMut<T>` in one parameter list, or a `get_resource` call made from
    /// inside a system that already holds the write lock. Turning the second into `None` would
    /// hide it behind a branch that looks like ordinary absence handling.
    ///
    /// `QueryError` is **not** an absence: query construction returning nothing means the world
    /// could not produce the view at all.
    #[must_use]
    pub fn is_absence(&self) -> bool {
        matches!(
            self,
            SystemParamFetchError::Resource(crate::world::ResourceFetchError::NotFound(_))
        )
    }
}

impl From<crate::world::ResourceFetchError> for SystemParamFetchError {
    fn from(value: crate::world::ResourceFetchError) -> Self {
        Self::Resource(value)
    }
}

impl std::fmt::Display for SystemParamFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemParamFetchError::Resource(e) => {
                write!(f, "system parameter resource fetch failed: {e}")
            }
            SystemParamFetchError::QueryError => {
                write!(f, "system parameter query construction failed")
            }
        }
    }
}

impl std::error::Error for SystemParamFetchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SystemParamFetchError::Resource(e) => Some(e),
            SystemParamFetchError::QueryError => None,
        }
    }
}

// SystemParam tamamen içsel bir DI trait'idir; yanlış bir impl scheduler'ın
// aliasing garantilerini bozar. Tüm impl'ler bu crate içindedir (Res/ResMut/
// f32/Query) ve cross-crate impl yoktur, bu yüzden sealed yapılır.
// `pub(crate)` çünkü SystemParam'ı implemente eden tipler bu crate'in başka
// modüllerinde de var (EventReader/EventWriter @ event.rs, Commands @ commands.rs);
// Sealed'a yalnızca crate içinden erişilebilir, dolayısıyla dış crate'ler hâlâ
// SystemParam impl edemez.
/// The seal on [`SystemParam`], and the one documented way through it.
///
/// `pub` and `#[doc(hidden)]` rather than `pub(crate)`, because
/// [`system_param!`](crate::system_param) has to name it from the calling crate. That is the
/// *only* intended use: implementing `Sealed` by hand still compiles, and still means writing
/// `get_access_info` by hand, which is the thing the seal exists to prevent — an access
/// declaration that under-reports is a data race the scheduler cannot see.
#[doc(hidden)]
pub mod sealed {
    /// See the module docs. Implement it through [`system_param!`](crate::system_param), not
    /// directly.
    pub trait Sealed {}
}

/// A value that a system can request as a parameter (e.g. [`Query`](crate::Query),
/// [`Res`], [`ResMut`]).
///
/// Implementors describe how to fetch their value from the [`World`] and which
/// component/resource accesses they require, allowing the scheduler to run
/// non-conflicting systems in parallel.
pub trait SystemParam: sealed::Sealed {
    /// The value actually handed to the system body, borrowed from the world for `'w`.
    ///
    /// The implementing type is the `'static` spelling of the parameter (`Res<'static, T>`,
    /// `Query<'static, Q>`) and `Item<'w>` is the same type re-borrowed for one run; a system
    /// signature writes plain `Res<Foo>` and lets elision supply `'w`.
    type Item<'w>;

    /// State this parameter keeps **between runs, per system**.
    ///
    /// `()` for everything that keeps none, which is every parameter but [`Local`]. It is the
    /// system that owns it — one `Local<u32>` per system, created with [`Default`] the first time
    /// the system is built and handed back on every run — so two systems asking for the same type
    /// get two independent values, which is exactly the difference from a resource.
    ///
    /// `Send + Sync` because a system may run on any thread of the pool; `Default` because the
    /// state has to exist before the world does anything, and a parameter that needed the world to
    /// build its state would be a resource.
    type State: Default + Send + Sync + 'static;

    /// Produces the parameter for a single run.
    ///
    /// Called once per parameter, in declaration order, immediately before the system body,
    /// and whatever it returns (a lock guard, for the resource parameters) stays alive until
    /// that body returns. `dt` is the delta time the schedule was stepped with, passed
    /// straight through — the `f32` parameter is exactly this value, and run conditions are
    /// always evaluated with `dt = 0.0`.
    ///
    /// Fails rather than blocks; see [`SystemParamFetchError`] for what the callers do with
    /// the error (they panic).
    fn fetch<'w>(
        world: &'w World,
        dt: f32,
        state: &'w mut Self::State,
    ) -> Result<Self::Item<'w>, SystemParamFetchError>;

    /// [`fetch`](Self::fetch) for a parameter that keeps no state, so a caller with none to hand
    /// it need not invent a borrow that outlives the call.
    ///
    /// `fetch` takes `&'w mut Self::State` because [`Local`] hands that reference straight to the
    /// system body — so the state has to live as long as the fetched value. A caller that has no
    /// state cannot satisfy that with a local `let mut () = ();`: the temporary dies at the end of
    /// the statement and `'w` does not. This is the way out, and it costs nothing at run time:
    /// `()` is zero-sized, so `Box::new(())` allocates nothing and leaking it leaks nothing.
    ///
    /// Only for `State = ()`, which the bound enforces — there is no way to reach it from a
    /// parameter that would actually lose its state.
    fn fetch_stateless<'w>(
        world: &'w World,
        dt: f32,
    ) -> Result<Self::Item<'w>, SystemParamFetchError>
    where
        Self: SystemParam<State = ()>,
    {
        Self::fetch(world, dt, Box::leak(Box::new(())))
    }

    /// Appends this parameter's accesses to `info`.
    ///
    /// Append-only: it must not clear or deduplicate what is already there, since all the
    /// parameters of one system accumulate into a single [`AccessInfo`]. Called while the
    /// schedule is built, not per frame.
    ///
    /// The declaration has to cover everything the fetched value can touch, including memory
    /// only touched incidentally — `Changed<T>`/`Added<T>` declare a *read* of `T` because
    /// they inspect the change ticks a `Mut<T>` writer stamps. Whatever is omitted here may
    /// be co-scheduled with a conflicting writer and raced on.
    fn get_access_info(info: &mut AccessInfo);
}

/// Shared access to a world resource, requested as a system parameter
/// (`fn sys(cfg: Res<Config>)`).
///
/// Holds a read lock on that resource's slot for as long as the value lives, i.e. for the
/// rest of the system body. Any number of `Res<T>` can be live at once; a concurrently live
/// [`ResMut`] of the same type makes the borrow fail, and so does a `T` that was never
/// inserted into the world. Those are not the only failures — see
/// [`SystemParamFetchError::Resource`] for the rest. Every one of them panics the system
/// rather than skipping it.
///
/// Read-only by construction: there is no `DerefMut`.
pub struct Res<'w, T: 'static> {
    value: ResourceReadGuard<'w, T>,
}

impl<'w, T: 'static> std::ops::Deref for Res<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: 'static> sealed::Sealed for Res<'static, T> {}
impl<T: 'static> SystemParam for Res<'static, T> {
    type Item<'w> = Res<'w, T>;
    type State = ();
    fn fetch<'w>(
        world: &'w World,
        _dt: f32,
        _state: &'w mut (),
    ) -> Result<Self::Item<'w>, SystemParamFetchError> {
        let value = world.try_get_resource::<T>()?;
        Ok(Res::<T> { value })
    }
    fn get_access_info(info: &mut AccessInfo) {
        info.resource_reads.push(TypeId::of::<T>());
    }
}

/// Exclusive access to a world resource, requested as a system parameter
/// (`fn sys(mut score: ResMut<Score>)`).
///
/// Holds a write lock on that resource's slot until the system body returns. While it is
/// alive no other borrow of `T` succeeds — not a second `ResMut<T>` in the same parameter
/// list, and not a `world.get_resource::<T>()` call made from inside this very system, which
/// comes back empty instead of blocking.
///
/// Mutation through this guard is invisible to change detection: resources carry no change
/// ticks, so nothing observes a resource write the way `Changed<T>` observes a component
/// write.
pub struct ResMut<'w, T: 'static> {
    value: ResourceWriteGuard<'w, T>,
}

impl<'w, T: 'static> std::ops::Deref for ResMut<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<'w, T: 'static> std::ops::DerefMut for ResMut<'w, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T: 'static> sealed::Sealed for ResMut<'static, T> {}
impl<T: 'static> SystemParam for ResMut<'static, T> {
    type Item<'w> = ResMut<'w, T>;
    type State = ();
    fn fetch<'w>(
        world: &'w World,
        _dt: f32,
        _state: &'w mut (),
    ) -> Result<Self::Item<'w>, SystemParamFetchError> {
        let value = world.try_get_resource_mut::<T>()?;
        Ok(ResMut::<T> { value })
    }
    fn get_access_info(info: &mut AccessInfo) {
        info.resource_writes.push(TypeId::of::<T>());
    }
}

impl<P: SystemParam> sealed::Sealed for Option<P> {}

// The block above documents THIS impl. It used to sit over the `Sealed` one, where rustdoc never
// renders it: `sealed::Sealed` is private, so forty lines explaining the only optional parameter
// in the engine were invisible on docs.rs. Found by `hatch_docs.rs`, which reads the doc block
// above each hatch's own declaration and could not find `run_if` in this one.
/// An optional parameter: `None` when the underlying one is not there yet.
///
/// # Why this exists
///
/// Every other parameter treats a failed fetch as a setup mistake and **panics** the system with a
/// diagnostic — deliberately, so a missing resource is never silently skipped. That is the right
/// default and it does not change. But it leaves no way to write "use it if it is there", and the
/// only guard available otherwise is `run_if`, which skips the whole system: there is nowhere to
/// put the fallback branch.
///
/// ```no_run
/// # use gizmo_core::prelude::*;
/// # use gizmo_core::system::Res;
/// # #[derive(Clone)]
/// # struct Analytics;
/// # gizmo_core::impl_component!(Analytics);
/// // Runs every frame whether or not the resource was inserted.
/// fn report(analytics: Option<Res<Analytics>>) {
///     match analytics {
///         Some(_a) => { /* send it */ }
///         None => { /* the game was built without analytics — carry on */ }
///     }
/// }
/// ```
///
/// # It tolerates absence, not conflict
///
/// Only [`SystemParamFetchError::is_absence`] becomes `None`. A borrow conflict still panics,
/// because it is a bug rather than a state: two `ResMut<T>` in one parameter list, or a
/// `get_resource` from inside a system already holding the write lock. Mapping that to `None`
/// would disguise a scheduling error as ordinary absence.
///
/// # The access is still declared
///
/// `get_access_info` forwards to the inner parameter **unconditionally**. If the resource is
/// present the system really does touch it, so the scheduler has to know — an `Option` that
/// declared nothing would be co-scheduled with a conflicting writer and race the moment the
/// resource appeared.
impl<P: SystemParam> SystemParam for Option<P> {
    type Item<'w> = Option<P::Item<'w>>;
    /// The inner parameter's, forwarded: `Option<Local<u32>>` is still one `u32` per system, and
    /// absence is about the *world*, not about the state.
    type State = P::State;
    fn fetch<'w>(
        world: &'w World,
        dt: f32,
        state: &'w mut Self::State,
    ) -> Result<Self::Item<'w>, SystemParamFetchError> {
        match P::fetch(world, dt, state) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.is_absence() => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn get_access_info(info: &mut AccessInfo) {
        P::get_access_info(info);
    }
}

/// Per-system state that lives across runs: `fn sys(mut count: Local<u32>)`.
///
/// # Why this is not a resource
///
/// It could be one, and that is exactly what it cost. A counter, a "did I already do this", a
/// scratch `Vec` — all of them had to become world resources, which makes them **visible to every
/// other system** and, worse, visible to the *scheduler*: two systems each keeping their own tally
/// in a `ResMut<Tally>` declare a write of the same type, so the batcher must keep them apart and
/// they never run in parallel. A `Local` declares nothing at all
/// ([`get_access_info`](SystemParam::get_access_info) appends nothing), so two systems holding
/// `Local<u32>` are as independent as if neither existed.
///
/// # One per system, and "system" means the built one
///
/// The value is created with [`Default`] when the system is turned into a [`System`](super::System)
/// and belongs to that instance. Registering the same function twice gives **two** independent
/// values — which is the same rule `distributive_run_if` already documents for a stateful
/// condition, and the answer to "why did my counter reset" is almost always that the system was
/// rebuilt.
///
/// A `Local` inside a [`system_param!`](crate::system_param) composite works and keeps its own
/// value; the composite's state is the tuple of its fields'.
pub struct Local<'w, T: 'static> {
    value: &'w mut T,
}

impl<T: 'static> std::ops::Deref for Local<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value
    }
}

impl<T: 'static> std::ops::DerefMut for Local<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value
    }
}

impl<T: std::fmt::Debug + 'static> std::fmt::Debug for Local<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.value, f)
    }
}

impl<T: Default + Send + Sync + 'static> sealed::Sealed for Local<'static, T> {}
impl<T: Default + Send + Sync + 'static> SystemParam for Local<'static, T> {
    type Item<'w> = Local<'w, T>;
    type State = T;
    fn fetch<'w>(
        _world: &'w World,
        _dt: f32,
        state: &'w mut Self::State,
    ) -> Result<Self::Item<'w>, SystemParamFetchError> {
        Ok(Local { value: state })
    }
    /// **Nothing.** That is the whole point: a `Local` touches no world state, so it puts no
    /// constraint on the batcher. Declaring a phantom access here would silently undo the one
    /// advantage this parameter has over a resource.
    fn get_access_info(_info: &mut AccessInfo) {}
}

impl sealed::Sealed for f32 {}
impl SystemParam for f32 {
    type Item<'w> = f32;
    type State = ();
    fn fetch<'w>(
        _world: &'w World,
        dt: f32,
        _state: &'w mut (),
    ) -> Result<Self::Item<'w>, SystemParamFetchError> {
        Ok(dt)
    }
    fn get_access_info(_info: &mut AccessInfo) {}
}

impl<Q: crate::query::WorldQuery + 'static> sealed::Sealed for crate::query::Query<'static, Q> {}
impl<Q: crate::query::WorldQuery + 'static> SystemParam for crate::query::Query<'static, Q> {
    type Item<'w> = crate::query::Query<'w, Q>;
    type State = ();
    fn fetch<'w>(
        world: &'w World,
        _dt: f32,
        _state: &'w mut (),
    ) -> Result<Self::Item<'w>, SystemParamFetchError> {
        // SAFETY: the scheduler validates that co-batched systems have disjoint component
        // access (`AccessInfo`/`is_compatible_with`) before running them in parallel, and
        // runs `is_exclusive` systems alone. So while this system's `Query` is live, no
        // other query mutably aliases the same components. This is the documented contract
        // of `query_unchecked` — the safe `query`/`query_mut` split can't express it because
        // a system only ever holds a shared `&World`.
        // `Query::new_for_system`, not `world.query_unchecked` — and the difference is the whole
        // boundary of `DefaultQueryFilters`. `query_unchecked` is `pub unsafe` and is ALSO the
        // engine's mutable-from-shared hatch: 98 call sites use it from editor panels, physics,
        // netcode and audio, none of which is a system parameter. Filtering there filtered them
        // too, which blanked the inspector, froze transform subtrees and made `sync_bodies`
        // *destroy* a disabled rigid body. This function is the one place a query is built as a
        // PARAMETER, so it is the one place the filter belongs.
        if let Some(query) = crate::query::Query::<Q>::new_for_system(world) {
            Ok(query)
        } else {
            Err(SystemParamFetchError::QueryError)
        }
    }
    fn get_access_info(info: &mut AccessInfo) {
        let mut types = Vec::new();
        Q::check_aliasing(&mut types);
        for (tid, is_mut) in types {
            if is_mut {
                info.component_writes.push(tid);
            } else {
                info.component_reads.push(tid);
            }
        }
    }
}


#[cfg(test)]
mod system_param_macro_tests {
    use super::*;
    use crate::world::World;

    #[derive(Clone, Default, Debug, PartialEq)]
    struct Clock(f32);
    crate::impl_component!(Clock);
    #[derive(Clone, Default, Debug, PartialEq)]
    struct Score(u32);
    crate::impl_component!(Score);
    #[derive(Clone, Default, Debug, PartialEq)]
    struct Level(u8);
    crate::impl_component!(Level);

    crate::system_param! {
        /// Two reads and a write, grouped.
        struct Ctx<'w> {
            clock: Res<Clock>,
            level: Res<Level>,
            score: ResMut<Score>,
        }
    }

    fn world() -> World {
        let mut w = World::new();
        w.insert_resource(Clock(1.5));
        w.insert_resource(Level(3));
        w.insert_resource(Score(10));
        w
    }

    #[test]
    fn a_composite_param_fetches_every_field() {
        let w = world();
        let mut state = Default::default();
        let ctx = Ctx::fetch(&w, 0.016, &mut state).expect("all three resources are present");
        assert_eq!(ctx.clock.0, 1.5);
        assert_eq!(ctx.level.0, 3);
        assert_eq!(ctx.score.0, 10);
    }

    #[test]
    fn writing_through_a_composite_param_reaches_the_world() {
        let w = world();
        {
            let mut state = Default::default();
            let mut ctx = Ctx::fetch(&w, 0.016, &mut state).expect("present");
            ctx.score.0 = 99;
        }
        assert_eq!(w.get_resource::<Score>().map(|s| s.0), Some(99));
    }

    /// **The reason the macro exists.** A hand-written `SystemParam` can under-report its access
    /// and nothing fails — the scheduler simply co-runs it with a conflicting writer. So the
    /// declaration is compared against the sum of the fields' own declarations, which is what the
    /// macro forwards to.
    #[test]
    fn the_access_declaration_is_exactly_the_sum_of_the_fields() {
        let mut composite = AccessInfo::new();
        Ctx::get_access_info(&mut composite);

        let mut separate = AccessInfo::new();
        <Res<'static, Clock> as SystemParam>::get_access_info(&mut separate);
        <Res<'static, Level> as SystemParam>::get_access_info(&mut separate);
        <ResMut<'static, Score> as SystemParam>::get_access_info(&mut separate);

        let mut a = composite.resource_reads.clone();
        let mut b = separate.resource_reads.clone();
        a.sort();
        b.sort();
        assert_eq!(a, b, "the composite under- or over-reports its reads");

        let mut a = composite.resource_writes.clone();
        let mut b = separate.resource_writes.clone();
        a.sort();
        b.sort();
        assert_eq!(a, b, "the composite under- or over-reports its writes");

        // And it is not empty, which an all-forwarding macro that forwarded nothing would also
        // satisfy above.
        assert_eq!(composite.resource_reads.len(), 2);
        assert_eq!(composite.resource_writes.len(), 1);
    }

    /// The declaration has to reach the **scheduler**, not just be correct in isolation.
    ///
    /// Two systems that both write `Score` must not share a batch. One takes it as a plain
    /// `ResMut`, the other through the composite — so this fails if the macro's forwarding stops
    /// somewhere between `get_access_info` and the batch builder.
    #[test]
    fn the_scheduler_separates_a_composite_from_a_conflicting_writer() {
        use crate::system::{IntoSystemConfig, Schedule};

        fn through_composite(mut ctx: Ctx) {
            ctx.score.0 += 1;
        }
        fn plain_writer(mut score: ResMut<Score>) {
            score.0 += 1;
        }

        let mut schedule = Schedule::new();
        schedule.add_di_system(through_composite.into_config());
        schedule.add_di_system(plain_writer.into_config());
        schedule.build();

        assert_eq!(
            schedule.legacy_batches.len(),
            2,
            "two writers of the same resource were put in one batch — the composite's write was \
             not seen"
        );

        // And they really do both run: a separation that dropped one would also give 2 above if
        // some third batch existed.
        let mut world = World::new();
        world.insert_resource(Clock(0.0));
        world.insert_resource(Level(0));
        world.insert_resource(Score(0));
        schedule.run(&mut world, 0.016);
        assert_eq!(
            world.get_resource::<Score>().map(|s| s.0),
            Some(2),
            "both writers should have run exactly once"
        );
    }

    #[test]
    fn a_missing_resource_fails_the_whole_fetch() {
        // Not `None` — that is `Option<P>`'s job, and a composite silently missing a field would
        // be worse than a panic, because the system would run against a half-built context.
        let mut w = World::new();
        w.insert_resource(Clock(1.0));
        w.insert_resource(Score(0));
        // `Level` absent.
        let Err(e) = Ctx::fetch(&w, 0.016, &mut Default::default()) else {
            panic!("Level is missing, so the fetch must fail")
        };
        assert!(e.is_absence(), "a missing resource should read as absence");
    }
}

#[cfg(test)]
mod optional_param_tests {
    use super::*;
    use crate::system::IntoSystem;

    #[derive(Clone, Copy, PartialEq, Debug)]
    struct Present(u32);
    #[derive(Clone, Copy)]
    struct Absent;

    /// The whole point: a system whose resource was never inserted still runs.
    #[test]
    fn an_absent_resource_becomes_none_instead_of_a_panic() {
        let world = World::new();
        let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let s = seen.clone();
        let mut system = (move |a: Option<Res<Absent>>| {
            assert!(a.is_none(), "nothing was inserted, so it cannot be Some");
            s.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })
        .into_system();

        system.run(&world, 0.016);
        assert_eq!(
            seen.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "the body must have run — without `Option` this fetch panics the system"
        );
    }

    /// And a present one still arrives, with its value intact.
    #[test]
    fn a_present_resource_still_arrives() {
        let mut world = World::new();
        world.insert_resource(Present(7));
        let got = std::sync::Arc::new(std::sync::Mutex::new(None));
        let g = got.clone();
        let mut system = (move |p: Option<Res<Present>>| {
            *g.lock().unwrap() = p.map(|v| *v);
        })
        .into_system();

        system.run(&world, 0.016);
        assert_eq!(*got.lock().unwrap(), Some(Present(7)));
    }

    /// `Option<ResMut<T>>` writes through, so it is not read-only by accident.
    #[test]
    fn the_mutable_form_writes_through() {
        let mut world = World::new();
        world.insert_resource(Present(1));
        let mut system = (|p: Option<ResMut<Present>>| {
            if let Some(mut p) = p {
                p.0 = 42;
            }
        })
        .into_system();

        system.run(&world, 0.016);
        assert_eq!(world.get_resource::<Present>().map(|r| *r), Some(Present(42)));
    }

    /// **The access must still be declared.** An `Option` that reported nothing would be
    /// co-scheduled with a conflicting writer and race the moment the resource appeared.
    #[test]
    fn the_access_is_declared_even_though_the_value_may_be_absent() {
        let mut bare = AccessInfo::new();
        <Res<'static, Present> as SystemParam>::get_access_info(&mut bare);
        let mut wrapped = AccessInfo::new();
        <Option<Res<'static, Present>> as SystemParam>::get_access_info(&mut wrapped);
        assert_eq!(
            bare.resource_reads, wrapped.resource_reads,
            "Option must declare exactly what the inner parameter declares"
        );

        let mut w = AccessInfo::new();
        <Option<ResMut<'static, Present>> as SystemParam>::get_access_info(&mut w);
        assert_eq!(w.resource_writes.len(), 1, "the write must be declared too");
    }

    /// Absence is tolerated; a borrow conflict is not.
    #[test]
    fn a_borrow_conflict_is_not_an_absence() {
        let missing = SystemParamFetchError::Resource(
            crate::world::ResourceFetchError::NotFound(std::any::TypeId::of::<Present>()),
        );
        let conflict = SystemParamFetchError::Resource(
            crate::world::ResourceFetchError::BorrowConflict(std::any::TypeId::of::<Present>()),
        );
        assert!(missing.is_absence(), "a never-inserted resource is an absence");
        assert!(
            !conflict.is_absence(),
            "a borrow conflict is a scheduling bug — turning it into None would hide it"
        );
        assert!(
            !SystemParamFetchError::QueryError.is_absence(),
            "a query that could not be built is not an absence"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// system_param! — a composite parameter, without opening the seal by hand
// ─────────────────────────────────────────────────────────────────────────────

/// Declares a struct that a system can take as a single parameter.
///
/// # What this is for
///
/// A system that needs six resources takes six parameters, and every system needing the same six
/// repeats them. Grouping them in a struct is the obvious move, and it was not possible:
/// [`SystemParam`] is sealed, so a game could not implement it, and implementing it is not the
/// hard part — [`get_access_info`](SystemParam::get_access_info) is. An access declaration that
/// under-reports does not fail; it lets the scheduler co-run two systems that write the same
/// resource, and the corruption depends on which one rayon reached first.
///
/// So the macro writes it. Each field's declaration is forwarded to that field's own type, which
/// is mechanical and cannot drift from what the struct actually fetches — which is exactly the
/// information the seal exists to protect, produced by construction rather than by care.
///
/// # Use
///
/// Field types are written **without** their lifetime: `Res<Time>`, not `Res<'w, Time>`. The macro
/// puts `'w` in the struct and `'static` in the impl, which is the same shape every built-in
/// parameter already has (`impl SystemParam for Res<'static, T>`).
///
/// A `pub` composite needs its field types to be **at least as public as itself**: the impl's
/// `State` is the tuple of the fields' states, so it names them, and a `pub` struct over a private
/// resource is `E0446`. Most composites are private and never meet this; the ones that are not
/// were already leaking those types through their `pub` fields.
///
/// ```
/// # use gizmo_core::prelude::*;
/// # use gizmo_core::system::{Res, ResMut};
/// # #[derive(Clone, Default)] pub struct Clock(f32);
/// # gizmo_core::impl_component!(Clock);
/// # #[derive(Clone, Default)] pub struct Score(u32);
/// # gizmo_core::impl_component!(Score);
/// gizmo_core::system_param! {
///     /// Everything the scoring systems read together.
///     pub struct Scoring<'w> {
///         pub clock: Res<Clock>,
///         pub score: ResMut<Score>,
///     }
/// }
///
/// fn award(mut s: Scoring) {
///     s.score.0 += 1;
///     let _ = s.clock.0;
/// }
/// ```
///
/// # What it does not do
///
/// It does not make [`SystemParam`] safe to implement by hand — the seal is still there and still
/// means what it meant. It also does not check that two fields do not conflict with *each other*:
/// a struct holding `ResMut<T>` twice declares two writes of `T` and panics on fetch, exactly as
/// two `ResMut<T>` parameters would.
#[macro_export]
macro_rules! system_param {
    (
        $(#[$meta:meta])*
        $vis:vis struct $name:ident<'w> {
            $(
                $(#[$fmeta:meta])*
                $fvis:vis $field:ident : $outer:ident<$($arg:ty),+ $(,)?>
            ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis struct $name<'w> {
            $(
                $(#[$fmeta])*
                $fvis $field: $outer<'w, $($arg),+>,
            )+
        }

        impl<'w> $crate::system::params::sealed::Sealed for $name<'w> {}

        impl<'a> $crate::system::SystemParam for $name<'a> {
            type Item<'w> = $name<'w>;

            // The tuple of the fields' own states, in declaration order — so a composite may hold
            // a `Local` and it keeps its own value, per system, like a bare one would.
            type State = (
                $(
                    <$outer<'static, $($arg),+> as $crate::system::SystemParam>::State,
                )+
            );

            fn fetch<'w>(
                world: &'w $crate::world::World,
                dt: f32,
                state: &'w mut Self::State,
            ) -> ::std::result::Result<Self::Item<'w>, $crate::system::SystemParamFetchError> {
                // Destructured so each field gets its OWN `&mut` — one borrow of the whole tuple
                // would be one borrow for all of them.
                let ( $($field,)+ ) = state;
                ::std::result::Result::Ok($name {
                    $(
                        $field: <$outer<'static, $($arg),+> as $crate::system::SystemParam>
                            ::fetch(world, dt, $field)?,
                    )+
                })
            }

            fn get_access_info(info: &mut $crate::system::AccessInfo) {
                // Forwarded field by field. This is the line that makes the macro the honest way
                // through the seal: it cannot report less than the struct fetches.
                $(
                    <$outer<'static, $($arg),+> as $crate::system::SystemParam>
                        ::get_access_info(info);
                )+
            }
        }
    };
}


#[cfg(test)]
mod local_tests {
    use super::*;
    use crate::system::{IntoSystemConfig, Schedule};
    use crate::world::World;

    #[derive(Default)]
    struct Reported(Vec<u32>);

    /// Counts its own runs in a `Local` and reports each value into a resource, so the test can
    /// see the sequence rather than only the last value.
    fn counting(mut runs: Local<u32>, mut out: ResMut<Reported>) {
        *runs += 1;
        out.0.push(*runs);
    }

    fn world() -> World {
        let mut w = World::new();
        w.insert_resource(Reported::default());
        w
    }

    /// The value survives from one run to the next — the whole reason the parameter exists.
    #[test]
    fn a_local_keeps_its_value_between_runs() {
        let mut w = world();
        let mut schedule = Schedule::new();
        schedule.add_di_system(counting.into_config());
        schedule.build();
        for _ in 0..4 {
            schedule.run(&mut w, 0.0);
        }
        assert_eq!(
            w.get_resource::<Reported>().map(|r| r.0.clone()),
            Some(vec![1, 2, 3, 4]),
            "the local reset between runs — it is being rebuilt rather than kept",
        );
    }

    /// Two systems asking for `Local<u32>` get **two** values, which is the difference from a
    /// resource in one sentence.
    ///
    /// The second system runs twice as often as the first here, so a shared value would show up as
    /// interleaving rather than as two independent counts.
    #[test]
    fn two_systems_each_get_their_own() {
        fn first(mut runs: Local<u32>, mut out: ResMut<Reported>) {
            *runs += 1;
            out.0.push(*runs);
        }
        fn second(mut runs: Local<u32>, mut out: ResMut<Reported>) {
            *runs += 100;
            out.0.push(*runs);
        }

        let mut w = world();
        let mut schedule = Schedule::new();
        schedule.add_di_system(first.into_config());
        schedule.add_di_system(second.into_config());
        schedule.build();
        schedule.run(&mut w, 0.0);
        schedule.run(&mut w, 0.0);

        let mut reported = w.get_resource::<Reported>().map(|r| r.0.clone()).unwrap_or_default();
        reported.sort_unstable();
        assert_eq!(
            reported,
            vec![1, 2, 100, 200],
            "the two systems shared one value — a `Local` is per system, not per type",
        );
    }

    /// Registering the same function twice gives two independent values.
    ///
    /// Worth pinning separately: "per system" could plausibly mean "per function", and the answer
    /// is that it means per *built* system — which is also the answer to "why did my counter
    /// reset".
    #[test]
    fn the_same_function_registered_twice_has_two_locals() {
        let mut w = world();
        let mut schedule = Schedule::new();
        schedule.add_di_system(counting.into_config());
        schedule.add_di_system(counting.into_config());
        schedule.build();
        schedule.run(&mut w, 0.0);

        assert_eq!(
            w.get_resource::<Reported>().map(|r| r.0.clone()),
            Some(vec![1, 1]),
            "both registrations reported 1 only if each has its own local; a shared one gives 1, 2",
        );
    }

    /// **The measurable win: a `Local` declares nothing, so it constrains nothing.**
    ///
    /// This is what the parameter is *for*. Before it, per-system state had to be a world
    /// resource — and two systems each keeping their own tally in a `ResMut<T>` declare a write of
    /// the same type, so the batcher must keep them apart and they never run in parallel. Two
    /// systems with a `Local` land in one batch; the same two written with a resource land in two.
    #[test]
    fn a_local_does_not_separate_two_systems_the_way_a_resource_does() {
        #[derive(Default)]
        struct Tally(u32);

        fn with_local_a(mut n: Local<u32>) {
            *n += 1;
        }
        fn with_local_b(mut n: Local<u32>) {
            *n += 1;
        }
        fn with_resource_a(mut n: ResMut<Tally>) {
            n.0 += 1;
        }
        fn with_resource_b(mut n: ResMut<Tally>) {
            n.0 += 1;
        }

        let mut schedule = Schedule::new();
        schedule.add_di_system(with_local_a.into_config());
        schedule.add_di_system(with_local_b.into_config());
        schedule.build();
        assert_eq!(
            schedule.legacy_batches.len(),
            1,
            "two systems holding a `Local` were put in separate batches — the parameter is \
             declaring an access it does not perform, which throws away its only advantage over a \
             resource",
        );

        let mut schedule = Schedule::new();
        schedule.add_di_system(with_resource_a.into_config());
        schedule.add_di_system(with_resource_b.into_config());
        schedule.build();
        assert_eq!(
            schedule.legacy_batches.len(),
            2,
            "the resource form is what the `Local` is being compared against; if it does not \
             separate them, this whole test proves nothing",
        );
    }

    /// It composes: `Option<Local<T>>` is still one value per system, and a `Local` beside a real
    /// parameter does not disturb it.
    #[test]
    fn a_local_sits_beside_other_parameters() {
        #[derive(Default)]
        struct Score(u32);

        fn mixed(mut runs: Local<u32>, score: Res<Score>, mut out: ResMut<Reported>) {
            *runs += 1;
            out.0.push(*runs + score.0);
        }

        let mut w = world();
        w.insert_resource(Score(10));
        let mut schedule = Schedule::new();
        schedule.add_di_system(mixed.into_config());
        schedule.build();
        schedule.run(&mut w, 0.0);
        schedule.run(&mut w, 0.0);
        assert_eq!(w.get_resource::<Reported>().map(|r| r.0.clone()), Some(vec![11, 12]));
    }

    /// A `Local` inside a `system_param!` composite keeps its own value too.
    ///
    /// The composite's state is the tuple of its fields', and that is the line that would silently
    /// be `()` if the macro had been left alone — the composite would compile and its counter
    /// would reset every frame.
    #[test]
    fn a_composite_can_hold_a_local() {
        #[derive(Default)]
        struct Score(u32);

        crate::system_param! {
            /// A composite with state in it.
            struct Ctx<'w> {
                runs: Local<u32>,
                score: Res<Score>,
            }
        }

        fn through_composite(mut ctx: Ctx, mut out: ResMut<Reported>) {
            *ctx.runs += 1;
            out.0.push(*ctx.runs + ctx.score.0);
        }

        let mut w = world();
        w.insert_resource(Score(100));
        let mut schedule = Schedule::new();
        schedule.add_di_system(through_composite.into_config());
        schedule.build();
        schedule.run(&mut w, 0.0);
        schedule.run(&mut w, 0.0);
        schedule.run(&mut w, 0.0);
        assert_eq!(
            w.get_resource::<Reported>().map(|r| r.0.clone()),
            Some(vec![101, 102, 103]),
            "the composite's local reset — its `State` is not the tuple of its fields'",
        );
    }
}
