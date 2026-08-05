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
            F: FnMut($($P::Item<'_>),+) -> bool + Send + Sync + 'static,
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
        let c = &self.condition_access;
        info.component_reads.extend(c.component_reads.iter().copied());
        info.component_writes.extend(c.component_writes.iter().copied());
        info.resource_reads.extend(c.resource_reads.iter().copied());
        info.resource_writes.extend(c.resource_writes.iter().copied());
        info.is_exclusive |= c.is_exclusive;
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

