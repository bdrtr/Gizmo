use super::*;
use crate::world::World;
use std::any::TypeId;

// ==============================================================
// SYSTEM CONFIG — LABEL / BEFORE / AFTER / READS / WRITES
// ==============================================================

/// A named group of systems that ordering constraints can address as a unit.
///
/// Implement it on a zero-sized marker type: `struct Physics; impl SystemSet for Physics {}`.
/// A set has no storage and no identity beyond the string returned by
/// [`set_name`](SystemSet::set_name) — [`SystemConfig::in_set`] records that string on the
/// member system twice, once as set membership and once as an ordinary label, and
/// [`SystemConfig::before_set`] / [`after_set`](SystemConfig::after_set) resolve to the same
/// string. Only the membership entry makes a [`SetConfig`] apply, so a hand-written
/// [`label`](SystemConfig::label) spelling the set's name joins no set.
///
/// Constraints that apply to the set as a whole (its own `before`/`after`, or a [`Phase`]
/// override for all members) are declared separately, with a [`SetConfig`] handed to
/// [`Schedule::configure_set`].
pub trait SystemSet: 'static {
    /// The label string this set is identified by.
    ///
    /// Defaults to `std::any::type_name::<Self>()`. Resolution is plain string equality
    /// against system labels, with no type check anywhere: two set types that render to the
    /// same string are one set as far as the scheduler is concerned, and a hand-written
    /// `before("…")` naming that string is equivalent to `before_set::<S>()`. Override this
    /// to pin a short name you control.
    fn set_name() -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// A system bundled with the scheduling metadata [`Schedule`] needs in order to place it:
/// ordering labels, `before`/`after` constraints, set membership, hand-declared world access
/// and a [`Phase`].
///
/// Built by the chaining methods on this type — each consumes and returns `self` — usually
/// starting from a bare function via [`IntoSystemConfig`],
/// and consumed by [`Schedule::add_di_system`]. Nothing here is interpreted until
/// [`Schedule::build`] runs; an unsatisfiable or unmatched constraint therefore surfaces at
/// build time, not at construction time.
pub struct SystemConfig {
    pub(crate) system: Box<dyn System>,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) before: Vec<&'static str>,
    pub(crate) after: Vec<&'static str>,
    pub(crate) in_sets: Vec<&'static str>,
    pub(crate) added_info: AccessInfo,
    pub(crate) phase: Phase,
    /// Metadata as the CALLER declared it, snapshotted by `Schedule::build` before it folds in
    /// any `SetConfig`. `None` until then. See [`SystemConfig::snapshot_meta`].
    pub(crate) pristine_meta: Option<SystemMeta>,
}

/// Everything a [`SystemConfig`] carries except the system itself.
///
/// This exists so a built schedule can be taken apart again. [`Schedule::build`] moves each
/// system out of its config and into a batch; before this type, the metadata went with the
/// dropped config, which is why adding a system after the first `run()` silently discarded every
/// system already compiled — there was nothing left to rebuild them from.
///
/// A batch now stores one of these beside each system, so
/// [`Schedule::invalidate`](crate::system::Schedule) can put the configs back together.
#[derive(Clone)]
pub(crate) struct SystemMeta {
    pub(crate) labels: Vec<&'static str>,
    pub(crate) before: Vec<&'static str>,
    pub(crate) after: Vec<&'static str>,
    pub(crate) in_sets: Vec<&'static str>,
    pub(crate) added_info: AccessInfo,
    pub(crate) phase: Phase,
}

impl SystemConfig {
    /// Snapshots this config's metadata.
    ///
    /// Taken BEFORE `build` folds in any [`SetConfig`], deliberately: set folding appends to
    /// `before`/`after` and can overwrite `phase`, so snapshotting afterwards would make every
    /// rebuild re-append the same constraints and grow those vectors without bound. The DAG
    /// would still come out the same — edges are de-duplicated — but the config would drift
    /// further from what the caller actually declared on each pass.
    pub(crate) fn snapshot_meta(&self) -> SystemMeta {
        SystemMeta {
            labels: self.labels.clone(),
            before: self.before.clone(),
            after: self.after.clone(),
            in_sets: self.in_sets.clone(),
            added_info: self.added_info.clone(),
            phase: self.phase,
        }
    }

    /// Rebuilds a config from a system and the metadata it was registered with.
    pub(crate) fn from_parts(system: Box<dyn System>, meta: SystemMeta) -> Self {
        Self {
            system,
            labels: meta.labels,
            before: meta.before,
            after: meta.after,
            in_sets: meta.in_sets,
            added_info: meta.added_info,
            phase: meta.phase,
            pristine_meta: None,
        }
    }
}

impl SystemConfig {
    /// Wraps an already-boxed system with empty metadata: no labels, no ordering
    /// constraints, no set membership, no extra access declarations, and the default
    /// [`Phase::Update`].
    ///
    /// The system's own [`System::access_info`] is untouched and stays authoritative — the
    /// `reads`/`writes` declarations added afterwards are merged on top of it at build time,
    /// they never replace it.
    pub fn new(system: Box<dyn System>) -> Self {
        Self {
            system,
            labels: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            in_sets: Vec::new(),
            added_info: AccessInfo::new(),
            phase: Phase::default(),
            pristine_meta: None,
        }
    }

    /// Declares membership of system set `S`.
    ///
    /// `S::set_name()` is recorded twice: as set membership, and as an ordinary
    /// [`label`](Self::label). The label is what makes another system's
    /// [`after_set::<S>()`](Self::after_set) order it after *every* member of `S`.
    ///
    /// The membership entry is used at [`Schedule::build`] time to fold in the [`SetConfig`]
    /// registered for `S`, if any: the set's `before`/`after` lists are appended to this
    /// system's, and the set's phase, when it has one, overwrites this system's phase. A set
    /// with no registered `SetConfig` contributes nothing beyond the shared label. Joining
    /// several sets is allowed; if more than one of them carries a phase, the one declared
    /// last wins.
    pub fn in_set<S: SystemSet>(mut self) -> Self {
        self.in_sets.push(S::set_name());
        self.labels.push(S::set_name());
        self
    }

    /// Adds an ordering label other systems can name in `before`/`after`.
    ///
    /// Labels are matched by string content, not pointer identity, and are not required to be
    /// unique: when several systems share a label, one `before`/`after` naming it constrains
    /// all of them at once. A system may carry any number of labels. Labels exist purely for
    /// constraint resolution during [`Schedule::build`] — a system has no access to its own
    /// labels at run time.
    pub fn label(mut self, label: &'static str) -> Self {
        self.labels.push(label);
        self
    }

    /// Requires this system to run before every *other* system labelled `target`.
    ///
    /// Resolution happens once, in [`Schedule::build`], and is scoped to the system's own
    /// [`Phase`] group whenever the schedule is in phase mode (i.e. as soon as any system
    /// asks for a phase other than [`Phase::Update`]). A constraint naming a system in a
    /// different phase therefore matches nothing.
    ///
    /// An unmatched constraint is not an error: it logs a warning and is dropped. This
    /// system's own labels are excluded from the search, so `before` on a label only this
    /// system carries is a no-op that warns.
    ///
    /// Constraints that close a cycle make [`Schedule::build`] panic.
    pub fn before(mut self, target: &'static str) -> Self {
        self.before.push(target);
        self
    }

    /// [`before`](Self::before) applied to `S::set_name()`: run ahead of every system that
    /// declared [`in_set::<S>()`](Self::in_set).
    ///
    /// Identical resolution rules — same phase group only, warns and is dropped when the set
    /// has no members in that group. Declaring this does *not* make the caller a member of
    /// `S`, so a [`SetConfig`] for `S` does not apply to it.
    pub fn before_set<S: SystemSet>(mut self) -> Self {
        self.before.push(S::set_name());
        self
    }

    /// Requires this system to run after every *other* system labelled `target`.
    ///
    /// The mirror of [`before`](Self::before) and subject to the same rules: resolved at
    /// [`Schedule::build`] time within the system's own [`Phase`] group, matching all systems
    /// carrying the label, warning and dropping the constraint when none does, and panicking
    /// if the resulting graph contains a cycle.
    pub fn after(mut self, target: &'static str) -> Self {
        self.after.push(target);
        self
    }

    /// [`after`](Self::after) applied to `S::set_name()`: run once every system that declared
    /// [`in_set::<S>()`](Self::in_set) has finished.
    ///
    /// Same resolution rules as [`before_set`](Self::before_set), and likewise no membership
    /// of `S` is implied.
    pub fn after_set<S: SystemSet>(mut self) -> Self {
        self.after.push(S::set_name());
        self
    }

    /// Declares that this system reads component type `T`, on top of whatever its parameters
    /// already imply.
    ///
    /// Declarations are additive and one-way: at build time they are merged into the
    /// system's own [`System::access_info`], so they can only make a system *more*
    /// constrained. There is no way to retract inferred access. The intended use is a
    /// hand-written [`System`] impl whose `access_info` under-reports what it touches.
    ///
    /// Access influences **batching only, never ordering**: a conflicting pair is merely kept
    /// out of the same parallel batch, and which of the two runs first is left to the
    /// batcher. When the relative order matters, state it with
    /// [`label`](Self::label) + [`before`](Self::before)/[`after`](Self::after).
    pub fn reads<T: 'static>(mut self) -> Self {
        self.added_info.component_reads.push(TypeId::of::<T>());
        self
    }
    /// Declares that this system writes component type `T`, on top of whatever its parameters
    /// already imply.
    ///
    /// A declared write conflicts with any other system's read *or* write of the same type,
    /// so it is the strongest per-type way to force a system out of a shared batch — short of
    /// [`exclusive`](Self::exclusive). Same caveats as [`reads`](Self::reads): additive only,
    /// and it constrains batching, not order.
    pub fn writes<T: 'static>(mut self) -> Self {
        self.added_info.component_writes.push(TypeId::of::<T>());
        self
    }
    /// Declares a read of resource type `T`.
    ///
    /// Resource and component access live in separate lists and are only ever compared
    /// against their own kind: `reads_res::<T>()` conflicts with `writes_res::<T>()` but is
    /// invisible to [`writes::<T>()`](Self::writes). Declaring the wrong kind silently buys
    /// no protection, so declare the one the system actually touches.
    pub fn reads_res<T: 'static>(mut self) -> Self {
        self.added_info.resource_reads.push(TypeId::of::<T>());
        self
    }
    /// Declares a write of resource type `T`.
    ///
    /// Conflicts with every other system's `reads_res::<T>()` and `writes_res::<T>()`, and —
    /// like all access declarations — separates the two into different batches without fixing
    /// which one runs first.
    pub fn writes_res<T: 'static>(mut self) -> Self {
        self.added_info.resource_writes.push(TypeId::of::<T>());
        self
    }
    /// Marks the system exclusive: it is treated as incompatible with every other system
    /// regardless of the access lists, so it ends up alone in its batch and nothing runs
    /// concurrently with it.
    ///
    /// This is a *concurrency* barrier, not a mutability upgrade — [`System::run`] still
    /// receives `&World`, and deferred structural changes still go through the
    /// [`CommandQueue`](crate::commands::CommandQueue), which the schedule flushes after each
    /// batch. It costs a full serialisation point inside the phase, so prefer declaring
    /// precise access where the access is knowable.
    pub fn exclusive(mut self) -> Self {
        self.added_info.is_exclusive = true;
        self
    }
    /// Assigns the system to `phase`. Overwrites any previous call; the default is
    /// [`Phase::Update`], and a [`SetConfig`] phase applied through
    /// [`in_set`](Self::in_set) overrides this at build time.
    ///
    /// Phases run one after another in ascending [`Phase`] order, so this is the coarse
    /// ordering tool: a `PostUpdate` system is guaranteed to run after every `Update` system
    /// without any label plumbing. The price is that `before`/`after` labels are resolved
    /// *within* a phase only — a cross-phase constraint matches nothing and warns.
    ///
    /// The mode switch is schedule-wide, not per system: while every system is left on
    /// [`Phase::Update`] the schedule builds one flat batch list, and a single system asking
    /// for a different phase moves the whole schedule into phase mode.
    pub fn in_phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    /// Gates the system: on each run of the schedule `condition` is evaluated first, against
    /// the same `&World`, and the system runs only if it returns `true`.
    ///
    /// When the condition is `false` the system's body *and* its parameter fetch are skipped
    /// — so a system guarded this way will not panic on a missing `Res<T>` while the guard
    /// stays false. The closure is `FnMut`, so it may keep state between runs; it is called
    /// exactly once per system per schedule run.
    ///
    /// Cost: this closure form is opaque to the scheduler, which cannot tell what the
    /// condition touches. The wrapped system is therefore marked
    /// [`exclusive`](Self::exclusive) and runs alone in its batch — conservative but sound.
    /// For a run condition that stays parallelisable, use the typed [`IntoCondition`] path via
    /// [`SystemExtRunIf::run_if_sys`], which infers the condition's access precisely.
    ///
    /// Repeated calls nest rather than replace: each wraps the previous system, the outermost
    /// condition is evaluated first, and all of them must pass.
    pub fn run_if<F>(mut self, condition: F) -> Self
    where
        F: FnMut(&World) -> bool + Send + Sync + 'static,
    {
        // Opaque world-closure form: its SystemParam access can't be inferred, so mark the
        // conditional system exclusive (its own batch) — conservative but sound. The typed
        // `run_if_sys`/`IntoCondition` path infers precise, parallelizable condition access.
        let mut condition_access = AccessInfo::new();
        condition_access.is_exclusive = true;
        self.system = Box::new(ConditionalSystem {
            inner: self.system,
            condition: Box::new(condition),
            condition_access,
        });
        self
    }
}

/// Turns a system into a [`SystemConfig`], the builder used to attach ordering
/// constraints (labels, `before`/`after`, system sets) and a [`Phase`] before
/// adding it to a [`Schedule`].
pub trait IntoSystemConfig<Params> {
    /// Produces the [`SystemConfig`] with empty metadata (no labels, no constraints,
    /// [`Phase::Update`]).
    ///
    /// Implemented for everything that is [`IntoSystem`] — plain functions whose arguments
    /// are [`SystemParam`]s, and `Box<dyn System>` — and as the identity for a `SystemConfig`
    /// that has already been built, so a finished config can be handed straight to
    /// [`Schedule::add_di_system`].
    ///
    /// The `Params` type parameter only exists to let the compiler pick the right
    /// [`IntoSystem`] impl from the function's argument types; it carries no information at
    /// run time.
    fn into_config(self) -> SystemConfig;

    /// Starts a config and names this system so others can order themselves against it.
    /// See [`SystemConfig::label`] for the matching rules.
    fn label(self, l: &'static str) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().label(l)
    }
    
    /// Starts a config and joins set `S`, which also gives this system `S`'s name as a label.
    /// See [`SystemConfig::in_set`].
    fn in_set<S: SystemSet>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().in_set::<S>()
    }
    
    /// Starts a config ordering this system ahead of every system labelled `target` in the
    /// same phase. See [`SystemConfig::before`] — in particular, an unmatched label only
    /// warns.
    fn before(self, target: &'static str) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().before(target)
    }
    
    /// Starts a config ordering this system ahead of every member of set `S`, without joining
    /// `S`. See [`SystemConfig::before_set`].
    fn before_set<S: SystemSet>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().before_set::<S>()
    }
    
    /// Starts a config ordering this system after every system labelled `target` in the same
    /// phase. See [`SystemConfig::after`].
    fn after(self, target: &'static str) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().after(target)
    }

    /// Starts a config ordering this system after every member of set `S`, without joining
    /// `S`. See [`SystemConfig::after_set`].
    fn after_set<S: SystemSet>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().after_set::<S>()
    }

    /// Starts a config declaring an extra read of component `C`, added to whatever the
    /// system's parameters already imply. Affects batching, not order — see
    /// [`SystemConfig::reads`].
    fn reads<C: 'static>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().reads::<C>()
    }
    /// Starts a config declaring an extra write of component `C`. A write conflicts with
    /// another system's read *or* write of `C`, so it is the strongest separator available —
    /// see [`SystemConfig::writes`].
    fn writes<C: 'static>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().writes::<C>()
    }
    /// Starts a config declaring an extra read of *resource* `C`. Resource access is compared
    /// only against resource access — see [`SystemConfig::reads_res`].
    fn reads_res<C: 'static>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().reads_res::<C>()
    }
    /// Starts a config declaring an extra write of *resource* `C`. Resource access is only
    /// ever compared against resource access — declaring `writes::<C>()` instead buys no
    /// protection here — see [`SystemConfig::writes_res`].
    fn writes_res<C: 'static>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().writes_res::<C>()
    }
    /// Starts a config that makes this system run alone in its batch. Still `&World`, not
    /// `&mut World` — see [`SystemConfig::exclusive`].
    fn exclusive(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().exclusive()
    }
    /// Starts a config placing this system in `phase`. One non-`Update` phase anywhere puts
    /// the whole schedule into phase mode — see [`SystemConfig::in_phase`].
    fn in_phase(self, phase: Phase) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().in_phase(phase)
    }
    /// Starts a config gating this system on `condition`. The closure form forces the system
    /// to run alone in its batch — see [`SystemConfig::run_if`] for why, and for the
    /// parallelisable alternative.
    fn run_if<F>(self, condition: F) -> SystemConfig
    where
        F: FnMut(&World) -> bool + Send + Sync + 'static,
        Self: Sized,
    {
        self.into_config().run_if(condition)
    }
}

impl<Params, T: IntoSystem<Params>> IntoSystemConfig<Params> for T {
    fn into_config(self) -> SystemConfig {
        SystemConfig::new(self.into_system())
    }
}

impl IntoSystemConfig<()> for SystemConfig {
    fn into_config(self) -> SystemConfig {
        self
    }
}


/// Registers one system, or a tuple of 2–8 systems, on a [`Schedule`] in a single call —
/// the trait behind [`Schedule::add_systems`].
///
/// Every element must implement [`IntoSystem`], i.e. be a plain system function or a
/// `Box<dyn System>`. A configured [`SystemConfig`] is *not* accepted (it does not implement
/// [`IntoSystem`]), so labels, phases, access declarations and run conditions cannot be
/// attached through this path; register those systems individually with
/// [`Schedule::add_di_system`].
pub trait IntoSystemConfigs<T> {
    /// Pushes every system in `self` onto `schedule`, left to right, each with empty metadata
    /// ([`Phase::Update`], no labels, no constraints).
    ///
    /// Listing order is insertion order, **not** execution order: grouping systems in one
    /// tuple adds no ordering constraint between them, so the batcher is free to put them all
    /// in the same parallel batch, where their relative order is unspecified. Whether they
    /// end up sharing a batch is a batching decision — do not rely on it either way.
    fn into_configs(self, schedule: &mut Schedule);
}

impl<P1, S1> IntoSystemConfigs<(P1,)> for S1
where
    S1: IntoSystem<P1> + 'static,
{
    fn into_configs(self, schedule: &mut Schedule) {
        schedule.add_system(self.into_system());
    }
}

macro_rules! impl_into_system_configs {
    ($($P:ident $S:ident),+) => {
        impl<$($P, $S),+> IntoSystemConfigs<($($P,)+)> for ($($S,)+)
        where
            $($S: IntoSystem<$P> + 'static,)+
        {
            fn into_configs(self, schedule: &mut Schedule) {
                #[allow(non_snake_case)]
                let ($($S,)+) = self;
                $(schedule.add_system($S.into_system());)+
            }
        }
    };
}

impl_into_system_configs!(P1 S1, P2 S2);
impl_into_system_configs!(P1 S1, P2 S2, P3 S3);
impl_into_system_configs!(P1 S1, P2 S2, P3 S3, P4 S4);
impl_into_system_configs!(P1 S1, P2 S2, P3 S3, P4 S4, P5 S5);
impl_into_system_configs!(P1 S1, P2 S2, P3 S3, P4 S4, P5 S5, P6 S6);
impl_into_system_configs!(P1 S1, P2 S2, P3 S3, P4 S4, P5 S5, P6 S6, P7 S7);
impl_into_system_configs!(P1 S1, P2 S2, P3 S3, P4 S4, P5 S5, P6 S6, P7 S7, P8 S8);

