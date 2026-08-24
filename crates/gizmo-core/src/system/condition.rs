use super::*;
use crate::world::World;

// ==============================================================
// RUN CONDITIONS
// ==============================================================

/// A predicate that gates a system, e.g. `|score: Res<Score>| score.0 > 10`.
///
/// Implemented for `FnMut() -> bool` and for closures taking 1 to 6 [`SystemParam`]s.
/// `Params` exists only to keep those impls from overlapping — it is inferred from the
/// closure's arguments and never spelled out by callers.
///
/// Combine conditions with [`or`], [`and`] and [`not`], which stay on this typed path and so keep
/// the access declaration the opaque `FnMut(&World) -> bool` form throws away.
///
/// # Writing a function that *returns* a condition
///
/// Spell the return type as `impl IntoCondition<(Res<'static, T>,)>`, **not** as
/// `impl FnMut(Res<T>) -> bool`. This trait's impl is bounded on two `FnMut`s — the second is what
/// makes the parameter types inferrable — and an `impl FnMut(..)` return type hides the second one
/// behind the opaque type, so the result is a closure the engine does not recognise as a
/// condition. The parameter list is written with `'static` because that is the `'static` spelling
/// of a [`SystemParam`]; a call site still writes plain `Res<T>`.
pub trait IntoCondition<Params> {
    /// Erases the predicate into a closure the scheduler can store, which fetches the
    /// declared parameters from the world afresh on every evaluation.
    ///
    /// The closure is called once per step, before the gated system's body. Returning
    /// `false` skips that body entirely — the *system's* own parameters are then never
    /// fetched, so gating a system off also suppresses the panic it would raise over a
    /// resource it is missing.
    ///
    /// The condition's own parameters are fetched with `dt = 0.0` (an `f32` condition
    /// parameter is therefore always zero, never the step's delta time) and unwrapped, so a
    /// condition that asks for an absent resource panics.
    fn into_condition(self) -> Box<dyn FnMut(&World) -> bool + Send + Sync>;
    /// World access performed by the condition's own SystemParams. The scheduler must
    /// include this in its disjointness check: a `run_if(|r: Res<Score>| ..)` condition
    /// reads `Score` at run time, so it must conflict with a `ResMut<Score>` writer (and a
    /// `Query<&Pos>` condition with a `Query<Mut<Pos>>` writer) — otherwise they co-batch
    /// and run in parallel over a shared `&World`, racing on the resource/component.
    fn condition_access() -> AccessInfo;
}

impl<F> IntoCondition<()> for F
where
    F: FnMut() -> bool + Send + Sync + 'static,
{
    fn into_condition(mut self) -> Box<dyn FnMut(&World) -> bool + Send + Sync> {
        Box::new(move |_world| self())
    }
    fn condition_access() -> AccessInfo {
        // A parameter-less condition touches no world state.
        AccessInfo::new()
    }
}

macro_rules! impl_into_condition {
    ($($P:ident),+) => {
        #[allow(non_snake_case)]
        impl<F, $($P),+> IntoCondition<($($P,)+)> for F
        where
            // Two `FnMut` bounds, the same pair `IntoSystem` carries and for the same reason. The
            // first is what the closure is actually called with. The SECOND is what makes the
            // parameter types INFERRABLE: `P::Item<'_>` is a projection the compiler cannot run
            // backwards, so given `|s: Res<Score>| …` it could not work out that `P1` is
            // `Res<'static, Score>`. Matching the closure against `FnMut(P1)` gives it directly.
            //
            // Without this line the typed path compiled and could not be *called* with a
            // parameterised closure: every attempt was `E0283 type annotations needed`, and the
            // annotation is unspellable (`Res<'static, Score>` is not what the caller writes).
            // Added 2026-08-24, when the run-condition combinators tried to use it.
            F: FnMut($($P::Item<'_>),+) -> bool + FnMut($($P),+) -> bool + Send + Sync + 'static,
            $($P: SystemParam + 'static,)+
        {
            fn into_condition(mut self) -> Box<dyn FnMut(&World) -> bool + Send + Sync> {
                Box::new(move |world| {
                    $(let $P = $P::fetch(world, 0.0).unwrap();)+
                    (self)($($P),+)
                })
            }
            fn condition_access() -> AccessInfo {
                let mut info = AccessInfo::new();
                $($P::get_access_info(&mut info);)+
                info
            }
        }
    };
}

impl_into_condition!(P1);
impl_into_condition!(P1, P2);
impl_into_condition!(P1, P2, P3);
impl_into_condition!(P1, P2, P3, P4);
impl_into_condition!(P1, P2, P3, P4, P5);
impl_into_condition!(P1, P2, P3, P4, P5, P6);

// ==============================================================
// COMBINATORS — AND, OR, NOT over typed conditions
// ==============================================================

/// Folds `from`'s accesses into `into`. Append-only, like [`SystemParam::get_access_info`].
fn merge_access(into: &mut AccessInfo, from: AccessInfo) {
    into.component_reads.extend(from.component_reads);
    into.component_writes.extend(from.component_writes);
    into.resource_reads.extend(from.resource_reads);
    into.resource_writes.extend(from.resource_writes);
    into.is_exclusive |= from.is_exclusive;
}

/// The `Params` marker a combined condition reports.
///
/// It exists only to keep [`Or`]/[`And`]/[`Not`] from colliding with the blanket
/// [`IntoCondition`] impls for closures, whose markers are tuples. Never named by a caller and
/// never constructed.
pub struct Combined<A, B>(std::marker::PhantomData<fn() -> (A, B)>);

/// Two conditions, true when **either** holds — see [`or`].
pub struct Or<A, B, PA, PB> {
    a: A,
    b: B,
    _params: std::marker::PhantomData<fn() -> (PA, PB)>,
}

/// Two conditions, true when **both** hold — see [`and`].
pub struct And<A, B, PA, PB> {
    a: A,
    b: B,
    _params: std::marker::PhantomData<fn() -> (PA, PB)>,
}

/// A condition inverted — see [`not`].
pub struct Not<C, P> {
    inner: C,
    _params: std::marker::PhantomData<fn() -> P>,
}

/// `a || b`, as one condition the scheduler can still analyse.
///
/// This is the combinator whose absence had a price. Without it an OR had to be written inside a
/// single `SystemConfig::run_if(|world| …)` closure — and that form is opaque, so the wrapped
/// system is marked [`exclusive`](crate::system::SystemConfig::exclusive) and runs alone in its
/// batch. Built this way the OR keeps its access declaration, so the system stays parallelisable.
///
/// **Short-circuits.** `b` is not evaluated on a step where `a` returned true, which matters for a
/// stateful `FnMut` predicate: a right-hand condition counting its own invocations will not count
/// the steps the left-hand one already answered. Put the cheap or the stateless one on the left.
///
/// **Access is the union anyway.** The declaration covers both sides even though only one may run:
/// the scheduler decides batches before the step, so "might read `Score`" has to be treated as
/// "reads `Score`". An OR is therefore never *less* constrained than its halves.
///
/// ```
/// # use gizmo_core::system::{or, SystemExtRunIf, IntoSystem, Res};
/// # #[derive(Default)] struct Score(u32);
/// # #[derive(Default)] struct Paused(bool);
/// # fn hud() {}
/// let system = hud.into_system().run_if_sys(or(
///     |score: Res<Score>| score.0 > 10,
///     |paused: Res<Paused>| paused.0,
/// ));
/// ```
pub fn or<PA, PB, A, B>(a: A, b: B) -> Or<A, B, PA, PB>
where
    A: IntoCondition<PA>,
    B: IntoCondition<PB>,
{
    Or { a, b, _params: std::marker::PhantomData }
}

/// `a && b`, as one condition.
///
/// Stacking `run_if` already ANDs — each call wraps the previous — so this is the *expression*
/// form of what the builder does structurally, and it is what makes an AND usable **inside** an
/// [`or`]. Short-circuits: `b` is not evaluated when `a` is false, and the access is the union
/// regardless, for the reason [`or`] gives.
pub fn and<PA, PB, A, B>(a: A, b: B) -> And<A, B, PA, PB>
where
    A: IntoCondition<PA>,
    B: IntoCondition<PB>,
{
    And { a, b, _params: std::marker::PhantomData }
}

/// `!c`.
///
/// Access is `c`'s: inverting the answer does not change what was read to produce it.
pub fn not<P, C>(c: C) -> Not<C, P>
where
    C: IntoCondition<P>,
{
    Not { inner: c, _params: std::marker::PhantomData }
}

impl<A, B, PA, PB> IntoCondition<Combined<PA, PB>> for Or<A, B, PA, PB>
where
    A: IntoCondition<PA>,
    B: IntoCondition<PB>,
{
    fn into_condition(self) -> Box<dyn FnMut(&World) -> bool + Send + Sync> {
        let mut a = self.a.into_condition();
        let mut b = self.b.into_condition();
        Box::new(move |world| a(world) || b(world))
    }
    fn condition_access() -> AccessInfo {
        let mut info = <A as IntoCondition<PA>>::condition_access();
        merge_access(&mut info, <B as IntoCondition<PB>>::condition_access());
        info
    }
}

impl<A, B, PA, PB> IntoCondition<Combined<PA, PB>> for And<A, B, PA, PB>
where
    A: IntoCondition<PA>,
    B: IntoCondition<PB>,
{
    fn into_condition(self) -> Box<dyn FnMut(&World) -> bool + Send + Sync> {
        let mut a = self.a.into_condition();
        let mut b = self.b.into_condition();
        Box::new(move |world| a(world) && b(world))
    }
    fn condition_access() -> AccessInfo {
        let mut info = <A as IntoCondition<PA>>::condition_access();
        merge_access(&mut info, <B as IntoCondition<PB>>::condition_access());
        info
    }
}

impl<C, P> IntoCondition<Combined<P, ()>> for Not<C, P>
where
    C: IntoCondition<P>,
{
    fn into_condition(self) -> Box<dyn FnMut(&World) -> bool + Send + Sync> {
        let mut inner = self.inner.into_condition();
        Box::new(move |world| !inner(world))
    }
    fn condition_access() -> AccessInfo {
        <C as IntoCondition<P>>::condition_access()
    }
}

/// Attaches a run condition to an already-boxed [`System`].
pub trait SystemExtRunIf {
    /// Wraps `self` so its body only executes on the steps where `cond` holds.
    ///
    /// The wrapper declares the union of the inner system's access and the condition's, so a
    /// condition reading `Score` keeps the system out of every batch containing a
    /// `ResMut<Score>` writer. This is the typed path and it stays parallelizable, unlike the
    /// `SystemConfig::run_if` form, whose opaque `FnMut(&World) -> bool` cannot be inspected
    /// and is therefore marked exclusive.
    ///
    /// Conditions stack: wrapping twice runs the body only when both hold, and the outer
    /// predicate short-circuits — a `false` there means the inner one is not evaluated at all.
    fn run_if_sys<ParamC, Cond: IntoCondition<ParamC>>(self, cond: Cond) -> Box<dyn System>;
}

impl SystemExtRunIf for Box<dyn System> {
    fn run_if_sys<ParamC, Cond: IntoCondition<ParamC>>(self, cond: Cond) -> Box<dyn System> {
        let condition_access = Cond::condition_access();
        Box::new(ConditionalSystem {
            inner: self,
            condition: cond.into_condition(),
            condition_access,
        })
    }
}

/// A [`System`] paired with a run condition, as produced by [`SystemExtRunIf::run_if_sys`]
/// and by `SystemConfig::run_if`.
///
/// The type is public only so it can travel inside `Box<dyn System>`; every field is
/// crate-private, so it is built through those two entry points rather than by hand. The
/// condition is re-evaluated every step — there is no caching and no memory of the previous
/// answer — and its access is folded into [`System::access_info`], so the batcher accounts
/// for what the condition reads as if the system itself read it.
pub struct ConditionalSystem {
    /// The gated system. Its parameters are only fetched on the steps where the condition
    /// returned true.
    pub(crate) inner: Box<dyn System>,
    /// Type-erased to a bare `FnMut(&World) -> bool`, so nothing about what it touches is
    /// recoverable from it. That is why the access it performs is carried separately in
    /// `condition_access`: whatever is missing there can be co-scheduled with a writer of the
    /// same state and raced on. Being `FnMut`, the predicate may also carry state across steps.
    pub(crate) condition: Box<dyn FnMut(&World) -> bool + Send + Sync>,
    /// Access the run-condition performs (see [`IntoCondition::condition_access`]).
    pub(crate) condition_access: AccessInfo,
}

impl System for ConditionalSystem {
    fn run(&mut self, world: &World, dt: f32) {
        if (self.condition)(world) {
            self.inner.run(world, dt);
        }
    }
    fn access_info(&self) -> AccessInfo {
        // Union the inner system's access with the CONDITION's access, so the batcher
        // never co-schedules a condition that reads state with a system that writes it.
        let mut info = self.inner.access_info();
        merge_access(&mut info, self.condition_access.clone());
        info
    }
}

/// Applies one run condition to each member of a tuple of systems (1 to 8 members).
pub trait DistributiveRunIfExt<Params> {
    /// Clones `cond` onto every system of the tuple, then fuses the results into one boxed
    /// system.
    ///
    /// The condition is evaluated once *per member* per step, not once for the group, so a
    /// predicate with side effects or an unstable answer can let some members run and others
    /// not within the same step.
    ///
    /// The fused system is a single scheduling unit: its members run sequentially in tuple
    /// order on one thread, never in parallel with each other, and deferred `Commands` are
    /// not flushed between them (that happens between batches), so a later member does not
    /// observe entities an earlier one spawned.
    ///
    /// Its declared access is the union of the members' component and resource accesses;
    /// `is_exclusive` is *not* carried over, so a tuple that contains an exclusive system is
    /// not itself treated as exclusive.
    ///
    /// `cond` is cloned once per member, so a stateful `FnMut` predicate ends up with one
    /// independent copy of its state per system in the tuple, not one copy shared by all of
    /// them: a condition counting its own invocations counts per member.
    fn distributive_run_if<ParamC, Cond: IntoCondition<ParamC> + Clone + Send + Sync + 'static>(self, cond: Cond) -> Box<dyn System>;
}

macro_rules! impl_distributive_run_if {
    ($($P:ident $S:ident $idx:tt),+) => {
        impl<$($P, $S),+> DistributiveRunIfExt<($($P,)+)> for ($($S,)+)
        where
            $($S: IntoSystem<$P> + 'static,)+
        {
            fn distributive_run_if<ParamC, Cond: IntoCondition<ParamC> + Clone + Send + Sync + 'static>(self, cond: Cond) -> Box<dyn System> {
                let systems: Vec<Box<dyn System>> = vec![
                    $(self.$idx.into_system().run_if_sys(cond.clone()),)+
                ];

                struct MacroSystem {
                    systems: Vec<Box<dyn System>>,
                }
                impl System for MacroSystem {
                    fn run(&mut self, world: &World, dt: f32) {
                        for s in &mut self.systems {
                            s.run(world, dt);
                        }
                    }
                    fn access_info(&self) -> AccessInfo {
                        let mut info = AccessInfo::new();
                        for s in &self.systems {
                            let s_info = s.access_info();
                            info.component_reads.extend(s_info.component_reads);
                            info.component_writes.extend(s_info.component_writes);
                            info.resource_reads.extend(s_info.resource_reads);
                            info.resource_writes.extend(s_info.resource_writes);
                        }
                        info
                    }
                }

                Box::new(MacroSystem { systems })
            }
        }
    };
}

impl_distributive_run_if!(P1 S1 0);
impl_distributive_run_if!(P1 S1 0, P2 S2 1);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3, P5 S5 4);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3, P5 S5 4, P6 S6 5);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3, P5 S5 4, P6 S6 5, P7 S7 6);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3, P5 S5 4, P6 S6 5, P7 S7 6, P8 S8 7);


#[cfg(test)]
mod combinator_tests {
    use super::*;
    use crate::system::{IntoSystemConfig, Res, ResMut, Schedule};
    use crate::world::World;

    #[derive(Default)]
    struct Score(u32);
    #[derive(Default)]
    struct Paused(bool);
    #[derive(Default)]
    struct Runs(u32);
    #[derive(Default)]
    struct RightSideCalls(u32);

    fn world_with(score: u32, paused: bool) -> World {
        let mut world = World::new();
        world.insert_resource(Score(score));
        world.insert_resource(Paused(paused));
        world.insert_resource(Runs(0));
        world.insert_resource(RightSideCalls(0));
        world
    }

    fn count_runs(mut runs: ResMut<Runs>) {
        runs.0 += 1;
    }

    /// Runs `system` once against a world and reports how many times its body executed.
    fn runs_of(system: Box<dyn System>, world: &mut World) -> u32 {
        let mut schedule = Schedule::new();
        schedule.add_di_system(system.into_config());
        schedule.build();
        schedule.run(world, 0.0);
        world.get_resource::<Runs>().map(|r| r.0).unwrap_or(0)
    }

    /// `or` is true when either side is, and false only when neither is.
    ///
    /// Four worlds rather than two: an `or` implemented as `&&` passes a "true when the left is
    /// true" test and fails only on the case where exactly one side holds.
    #[test]
    fn or_is_true_when_either_side_is() {
        for (score, paused, expected) in
            [(0, false, 0), (20, false, 1), (0, true, 1), (20, true, 1)]
        {
            let mut world = world_with(score, paused);
            let system = count_runs.into_system().run_if_sys(or(
                |s: Res<Score>| s.0 > 10,
                |p: Res<Paused>| p.0,
            ));
            assert_eq!(
                runs_of(system, &mut world),
                expected,
                "score {score}, paused {paused}",
            );
        }
    }

    /// `and` is true only when both are, and `not` inverts.
    #[test]
    fn and_is_true_only_when_both_are_and_not_inverts() {
        for (score, paused, expected) in
            [(0, false, 0), (20, false, 0), (0, true, 0), (20, true, 1)]
        {
            let mut world = world_with(score, paused);
            let system = count_runs.into_system().run_if_sys(and(
                |s: Res<Score>| s.0 > 10,
                |p: Res<Paused>| p.0,
            ));
            assert_eq!(runs_of(system, &mut world), expected, "and: {score}/{paused}");
        }
        for (paused, expected) in [(false, 1), (true, 0)] {
            let mut world = world_with(0, paused);
            let system = count_runs.into_system().run_if_sys(not(|p: Res<Paused>| p.0));
            assert_eq!(runs_of(system, &mut world), expected, "not: {paused}");
        }
    }

    /// `or` short-circuits, which is a promise about *state* and not only about cost.
    ///
    /// A right-hand condition that counts its own calls is the only way to see it, and a
    /// stateful predicate is exactly the case the doc warns about.
    #[test]
    fn or_does_not_evaluate_its_right_side_when_the_left_is_true() {
        let mut world = world_with(20, false);
        let system = count_runs.into_system().run_if_sys(or(
            |s: Res<Score>| s.0 > 10,
            |mut calls: ResMut<RightSideCalls>| {
                calls.0 += 1;
                false
            },
        ));
        assert_eq!(runs_of(system, &mut world), 1);
        assert_eq!(
            world.get_resource::<RightSideCalls>().map(|c| c.0),
            Some(0),
            "the right-hand condition ran even though the left already answered true",
        );

        // …and it IS evaluated when the left says no, or the short-circuit would be a silent
        // "the second condition never runs".
        let mut world = world_with(0, false);
        let system = count_runs.into_system().run_if_sys(or(
            |s: Res<Score>| s.0 > 10,
            |mut calls: ResMut<RightSideCalls>| {
                calls.0 += 1;
                false
            },
        ));
        runs_of(system, &mut world);
        assert_eq!(world.get_resource::<RightSideCalls>().map(|c| c.0), Some(1));
    }

    /// The combined access reaches the **scheduler**, from both sides.
    ///
    /// This is the property that makes a combinator worth having rather than a closure: the
    /// condition reads `Score` and `Paused`, so the guarded system must not share a batch with a
    /// writer of either. A union that dropped the right-hand side would put the `Paused` writer in
    /// with it and race on a resource the condition reads.
    #[test]
    fn an_or_declares_both_sides_to_the_scheduler() {
        fn writes_paused(mut p: ResMut<Paused>) {
            p.0 = !p.0;
        }
        fn writes_score(mut s: ResMut<Score>) {
            s.0 += 1;
        }

        for writer in [
            writes_paused.into_config(),
            writes_score.into_config(),
        ] {
            let guarded = count_runs.into_system().run_if_sys(or(
                |s: Res<Score>| s.0 > 10,
                |p: Res<Paused>| p.0,
            ));
            let mut schedule = Schedule::new();
            schedule.add_di_system(guarded.into_config());
            schedule.add_di_system(writer);
            schedule.build();
            assert_eq!(
                schedule.legacy_batches.len(),
                2,
                "a writer of a resource the OR condition reads was co-batched with it",
            );
        }
    }

    /// And the point of the whole thing: the typed OR stays parallelisable.
    ///
    /// The same OR written the only way it could be written before — inside one
    /// `run_if(|world| …)` closure — makes the system **exclusive**, because an opaque predicate
    /// tells the scheduler nothing about what it touches. Two systems that share no state land in
    /// one batch with the combinator and in two without it. That difference is the feature.
    #[test]
    fn the_typed_or_stays_in_one_batch_where_the_closure_form_does_not() {
        #[derive(Default)]
        struct Unrelated(u32);
        fn untouched(mut u: ResMut<Unrelated>) {
            u.0 += 1;
        }

        let typed = count_runs.into_system().run_if_sys(or(
            |s: Res<Score>| s.0 > 10,
            |p: Res<Paused>| p.0,
        ));
        let mut schedule = Schedule::new();
        schedule.add_di_system(typed.into_config());
        schedule.add_di_system(untouched.into_config());
        schedule.build();
        assert_eq!(
            schedule.legacy_batches.len(),
            1,
            "the typed OR forced its system into a batch of its own — it is being treated as \
             opaque, which is the cost it exists to remove",
        );

        // The same condition as an opaque closure, for the contrast the doc claims.
        let mut schedule = Schedule::new();
        schedule.add_di_system(count_runs.into_config().run_if(|world| {
            world.get_resource::<Score>().is_some_and(|s| s.0 > 10)
                || world.get_resource::<Paused>().is_some_and(|p| p.0)
        }));
        schedule.add_di_system(untouched.into_config());
        schedule.build();
        assert_eq!(
            schedule.legacy_batches.len(),
            2,
            "the closure form is documented as exclusive; if it is not, the combinator's whole \
             justification is stale",
        );
    }

    /// Combinators nest, so an OR of two ANDs is one condition with one access declaration.
    #[test]
    fn combinators_nest() {
        // (score > 10 AND NOT paused) OR (score == 0)
        let build = || {
            or(
                and(|s: Res<Score>| s.0 > 10, not(|p: Res<Paused>| p.0)),
                |s: Res<Score>| s.0 == 0,
            )
        };
        for (score, paused, expected) in
            [(0, false, 1), (0, true, 1), (20, false, 1), (20, true, 0), (5, true, 0)]
        {
            let mut world = world_with(score, paused);
            let system = count_runs.into_system().run_if_sys(build());
            assert_eq!(runs_of(system, &mut world), expected, "{score}/{paused}");
        }
    }
    /// The typed path accepts a parameter closure **at all**.
    ///
    /// It did not. `IntoCondition`'s impl was bounded only on `FnMut(P::Item<'_>) -> bool`, and
    /// `P::Item<'_>` is a projection the compiler cannot run backwards — so given
    /// `|s: Res<Score>| …` it could not work out that `P1` is `Res<'static, Score>`, and every
    /// call was `E0283 type annotations needed` with no annotation a caller could write. The
    /// parallelisable half of run conditions existed, was documented, and could not be reached
    /// with anything that took a parameter; the combinators above are what tried to.
    ///
    /// Fixed by the second `FnMut` bound, which `IntoSystem` has carried all along. This test is
    /// the one that goes red if it is removed as redundant — it looks redundant.
    #[test]
    fn the_typed_path_takes_a_parameter_closure_at_all() {
        let mut world = world_with(20, false);
        let system = count_runs.into_system().run_if_sys(|s: Res<Score>| s.0 > 10);
        assert_eq!(runs_of(system, &mut world), 1);
    }
}
