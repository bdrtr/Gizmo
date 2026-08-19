use super::*;
use crate::world::World;
use std::collections::HashSet;

// ==============================================================
// SCHEDULE — DAG BATCHING & MULTITHREADING
// ==============================================================

/// A group of systems whose declared accesses do not conflict, so that all of them may run
/// concurrently against one shared `&World`. Not *disjoint*: two systems that both read the
/// same type share a batch and share that access.
///
/// A candidate joins only if it conflicts with nothing already in the batch (see
/// [`is_compatible`](Self::is_compatible)). That absence of conflict is the whole safety
/// argument behind the unchecked queries a system receives: a system holds only `&World`, so nothing
/// but the batching stops two co-scheduled systems from mutably aliasing the same component
/// column (see the `SAFETY` note on `SystemParam for Query` in `system/params.rs`).
///
/// Batches are produced by [`Schedule::build`]. Constructing one yourself is of little use:
/// the `systems` vector is crate-private and there is no public accessor, iterator or `run`,
/// so a `SystemBatch` built outside this crate can be pushed into but never read back.
pub struct SystemBatch {
    pub(crate) systems: Vec<Box<dyn System>>,
    /// The metadata each system in `systems` was registered with, same length and same order.
    ///
    /// A batch owns its systems, so without this the metadata would be gone the moment
    /// `build` moved a system in here — which is exactly why a rebuild used to be impossible.
    /// Kept as the caller declared it, BEFORE set-config folding; see
    /// [`SystemConfig::snapshot_meta`].
    pub(crate) metas: Vec<crate::system::config::SystemMeta>,
    /// Union of the accesses of every member — each system's own
    /// [`System::access_info`] merged with the extra access declared on its
    /// [`SystemConfig`] (`reads`/`writes`/`reads_res`/`writes_res`/`exclusive`).
    ///
    /// Grown with `Vec::extend` and never de-duplicated, so a `TypeId` appears once per
    /// system that declared it: this is a multiset, not a set, and its length is only an
    /// upper bound on the number of distinct types touched. `is_exclusive` is the OR over
    /// the members; once it is set the batch rejects every further candidate.
    pub access_info: AccessInfo,
}

impl Default for SystemBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemBatch {
    /// An empty batch: no systems, no declared access, not exclusive.
    ///
    /// Note that an empty batch still answers `false` from
    /// [`is_compatible`](Self::is_compatible) for an *exclusive* system — exclusivity is
    /// rejected from either side. That is why [`Schedule::build`] always ends up giving an
    /// exclusive system a freshly appended batch, where it runs alone.
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
            metas: Vec::new(),
            access_info: AccessInfo::new(),
        }
    }

    /// Appends `system` to the batch and folds its access into
    /// [`access_info`](Self::access_info).
    ///
    /// `config_info` is the access declared by hand on the system's [`SystemConfig`]; it is
    /// merged *on top of* the system's own [`System::access_info`]. The merge is purely
    /// additive — a manual declaration can only widen what the batch believes is touched,
    /// never narrow it — and `is_exclusive` is OR-ed in.
    ///
    /// No compatibility check happens here. The caller must already have had `true` from
    /// [`is_compatible`](Self::is_compatible) for this same `system`/`config_info` pair;
    /// appending a conflicting system creates a data race, because batch members are run in
    /// parallel over a shared world.
    pub(crate) fn add_system_with_meta(
        &mut self,
        system: Box<dyn System>,
        config_info: AccessInfo,
        meta: crate::system::config::SystemMeta,
    ) {
        let mut sys_info = system.access_info();
        sys_info.component_reads.extend(config_info.component_reads);
        sys_info
            .component_writes
            .extend(config_info.component_writes);
        sys_info.resource_reads.extend(config_info.resource_reads);
        sys_info.resource_writes.extend(config_info.resource_writes);
        sys_info.is_exclusive = sys_info.is_exclusive || config_info.is_exclusive;

        self.access_info
            .component_reads
            .extend(sys_info.component_reads);
        self.access_info
            .component_writes
            .extend(sys_info.component_writes);
        self.access_info
            .resource_reads
            .extend(sys_info.resource_reads);
        self.access_info
            .resource_writes
            .extend(sys_info.resource_writes);
        self.access_info.is_exclusive = self.access_info.is_exclusive || sys_info.is_exclusive;

        self.systems.push(system);
        self.metas.push(meta);
    }

    /// Whether `system`, widened by the hand-declared `config_info`, can be added to this
    /// batch without a conflict.
    ///
    /// Conflicting means read/write or write/write on the same component or the same
    /// resource type; two readers of one type are fine, and accesses to different types
    /// never conflict. `false` is returned unconditionally when either side is exclusive,
    /// including against an empty batch.
    ///
    /// Only *declared* access is examined. Anything a system touches without declaring it —
    /// interior mutability, a hand-written [`System::access_info`] that under-reports,
    /// global state — is invisible here and will race if it collides.
    ///
    /// Pure query; pass the identical `config_info` to
    /// [`add_system`](Self::add_system) afterwards, or the answer does not apply.
    pub fn is_compatible(&self, system: &dyn System, config_info: &AccessInfo) -> bool {
        let mut sys_info = system.access_info();
        sys_info
            .component_reads
            .extend(config_info.component_reads.iter().cloned());
        sys_info
            .component_writes
            .extend(config_info.component_writes.iter().cloned());
        sys_info
            .resource_reads
            .extend(config_info.resource_reads.iter().cloned());
        sys_info
            .resource_writes
            .extend(config_info.resource_writes.iter().cloned());
        sys_info.is_exclusive = sys_info.is_exclusive || config_info.is_exclusive;

        self.access_info.is_compatible_with(&sys_info)
    }
}

/// Ordering and phase constraints applied to every system that declared membership of a
/// [`SystemSet`] with `in_set::<S>()`.
///
/// A `SetConfig` does nothing until it is handed to [`Schedule::configure_set`], and it is
/// consumed at [`Schedule::build`] time, so it may be registered before or after its member
/// systems are added. It positions the members relative to other labels, and normally leaves
/// the members free to batch together and run in parallel.
///
/// Normally, not always: [`SystemConfig::in_set`] records the set name as an ordinary LABEL
/// as well as recording membership, and the set's `before`/`after` lists are appended to each
/// member's own without excluding fellow members. So if two systems are both in set `A` and
/// both carry a label that `A`'s own constraints name, they can end up ordered against each
/// other — and a symmetric case (two systems each in `A` and `B`, with `A.after::<B>()`)
/// produces a cycle that makes [`Schedule::build`] panic.
pub struct SetConfig {
    /// Identity of the set — [`SystemSet::set_name`], which defaults to
    /// `std::any::type_name::<S>()`.
    ///
    /// [`Schedule::configure_set`] keys its map on this string, so configuring the same set
    /// twice *replaces* the earlier configuration instead of merging with it.
    /// `SystemConfig::in_set` records the very same string as a label on each member, which
    /// is how the `before`/`after` constraints below resolve to a set's members.
    pub name: &'static str,

    /// Labels the members of this set must run *before*.
    ///
    /// At build time every member gains an ordering edge to every *other* system carrying
    /// one of these labels, which places the member in a strictly earlier batch. Matching is
    /// by exact string against system labels; a name that matches no system logs a warning
    /// and is silently dropped — it is not an error and yields no ordering.
    pub before: Vec<&'static str>,

    /// Labels the members of this set must run *after* — the mirror of the `before` field,
    /// with the same string matching and the same warn-and-ignore treatment of names that
    /// match nothing.
    pub after: Vec<&'static str>,

    /// Phase forced onto every member, or `None` to leave each member's own phase alone.
    ///
    /// `Some(_)` *overrides* a per-system `in_phase`, because set configs are applied after
    /// the system list has been collected. If a system belongs to several configured sets,
    /// the last of its `in_set` declarations that carries a phase wins.
    pub phase: Option<Phase>,
}

impl SetConfig {
    /// An unconstrained configuration for set `S`: no ordering edges, no phase override.
    ///
    /// Registering it is not pointless — [`Schedule::configure_set`] replaces any previous
    /// configuration for `S`, so this is also how a set's constraints are cleared. Chain
    /// `before` / `after` / [`in_phase`](Self::in_phase) to build the constraints up.
    pub fn new<S: SystemSet>() -> Self {
        Self {
            name: S::set_name(),
            before: Vec::new(),
            after: Vec::new(),
            phase: None,
        }
    }
    /// Constrains this set's members to run before the members of `S`.
    ///
    /// Accumulates — call it repeatedly to name several sets. `S` is referenced by
    /// [`SystemSet::set_name`], which resolves against systems carrying that exact label;
    /// `in_set::<S>()` is what attaches it. If no system is in `S`, the constraint is
    /// dropped with a warning at build time.
    pub fn before<S: SystemSet>(mut self) -> Self {
        self.before.push(S::set_name());
        self
    }
    /// Constrains this set's members to run after the members of `S`. Accumulates, resolves
    /// and degrades exactly like the `before` constructor above.
    pub fn after<S: SystemSet>(mut self) -> Self {
        self.after.push(S::set_name());
        self
    }
    /// Forces every member of this set into `phase`, overriding whatever phase the member
    /// declared for itself.
    ///
    /// Naming any phase other than [`Phase::Update`] here is enough to switch the whole
    /// schedule into phase mode — see [`Schedule::build`].
    pub fn in_phase(mut self, phase: Phase) -> Self {
        self.phase = Some(phase);
        self
    }
}

/// A set of systems compiled into a fixed sequence of parallel batches, plus the executor
/// that runs them.
///
/// Systems accumulate as unbuilt configs and are turned into batches by
/// [`build`](Self::build), which [`run`](Self::run) calls lazily. The compiled layout is a
/// pure function of the order the systems were added in and the access each one declares —
/// no hash-map iteration enters into it — so the same construction sequence always yields
/// the same batches. Systems in *different* batches always observe the same relative order;
/// systems inside one batch have
/// no order at all (see [`run`](Self::run)).
///
/// **Add every system, and register every [`SetConfig`], before the first
/// `build`/`run`.** Every mutating method invalidates the compiled batches, and because
/// `build` *moves* the pending configs into those batches, invalidating after a build
/// discards the systems that were already compiled. Adding one system to a built schedule
/// therefore leaves a schedule containing only that system.
pub struct Schedule {
    unbuilt_configs: Vec<SystemConfig>,
    set_configs: std::collections::HashMap<&'static str, SetConfig>,
    /// A separate batch list for each phase. Phases run in order, batches within a phase in parallel.
    pub(crate) phase_batches: Vec<(Phase, Vec<SystemBatch>)>,
    /// Backwards compatibility: the old flat batch list, for when phases are not used.
    pub(crate) legacy_batches: Vec<SystemBatch>,
    pub(crate) uses_phases: bool,
    /// The world tick on which this schedule most recently ran — the reference for change
    /// detection. On every `run` the comparison is made against the previous value.
    last_run_tick: u32,
}

impl Schedule {
    /// An empty, unbuilt schedule with no systems and no set configurations.
    ///
    /// The change-detection reference tick starts at 0. Since a `World` never uses tick 0
    /// and tick filters test *strictly greater than* the reference, the first
    /// [`run`](Self::run) reports every component already in the world as both changed and
    /// added.
    pub fn new() -> Self {
        Self {
            unbuilt_configs: Vec::new(),
            set_configs: std::collections::HashMap::new(),
            phase_batches: Vec::new(),
            legacy_batches: Vec::new(),
            uses_phases: false,
            last_run_tick: 0,
        }
    }

    /// Registers ordering/phase constraints for a system set, replacing any configuration
    /// previously registered under the same [`SetConfig::name`].
    ///
    /// Order relative to the `add_*` calls does not matter: the constraints are applied
    /// when the schedule is built. Configuring a set no system belongs to is silently
    /// ignored.
    ///
    /// Invalidates the compiled batches, so calling it on an already-built schedule throws
    /// the built systems away and leaves the schedule empty.
    pub fn configure_set(&mut self, config: SetConfig) {
        self.set_configs.insert(config.name, config);
        self.invalidate();
    }

    /// Adds a dependency-injected system: a plain function or closure whose arguments are
    /// [`SystemParam`]s (up to 12 of them), or a [`SystemConfig`] already decorated with
    /// `label`/`before`/`after`/`in_set`/`in_phase`/`run_if`.
    ///
    /// This is the only `add_*` entry point that preserves ordering constraints, so it is
    /// the one to use whenever the system is not fully independent. Its component/resource
    /// access is inferred from the parameter types, which is what lets the batcher run it in
    /// parallel with others.
    ///
    /// Systems are appended, and insertion order is the tie-break used by the topological
    /// sort — it decides the batch layout whenever the ordering constraints leave a choice.
    /// Invalidates the compiled batches (see the type-level warning).
    pub fn add_di_system<Params, S: IntoSystemConfig<Params>>(&mut self, system: S) {
        self.unbuilt_configs.push(system.into_config());
        self.invalidate();
    }

    /// Adds a value that already implements [`System`], with no labels, no ordering
    /// constraints and no hand-declared extra access — batching is decided purely by the
    /// system's own [`System::access_info`].
    ///
    /// Anything needing ordering must go through [`add_di_system`](Self::add_di_system)
    /// instead; there is no way to attach constraints afterwards.
    ///
    /// Beware the blanket `System` impl for `FnMut(&World, f32)`: such a closure reports
    /// itself as exclusive, so it becomes a full barrier occupying a batch on its own.
    /// Invalidates the compiled batches (see the type-level warning).
    pub fn add_system<S: System + 'static>(&mut self, system: S) {
        self.unbuilt_configs
            .push(SystemConfig::new(Box::new(system)));
        self.invalidate();
    }

    /// Adds a tuple of up to 8 bare systems (or a single one) in one call.
    ///
    /// Each element is added exactly as [`add_system`](Self::add_system) would add it:
    /// **position in the tuple implies no ordering**, so `add_systems((a, b))` lets `a` and
    /// `b` run in the same batch, in parallel, in an unspecified order. The tuple form also
    /// does not accept already-configured [`SystemConfig`]s; use
    /// [`add_di_system`](Self::add_di_system) one at a time for those.
    pub fn add_systems<T, Configs: IntoSystemConfigs<T>>(&mut self, configs: Configs) {
        configs.into_configs(self);
    }

    /// Adds an already-boxed system — the object-safe entry point, for systems produced at
    /// runtime or by combinators such as `pipe`/`run_if_sys` that hand back a
    /// `Box<dyn System>`.
    ///
    /// Identical in effect to [`add_system`](Self::add_system): no labels, no ordering, and
    /// access taken solely from [`System::access_info`].
    pub fn add_system_boxed(&mut self, system: Box<dyn System>) {
        self.unbuilt_configs.push(SystemConfig::new(system));
        self.invalidate();
    }

    /// Takes a built schedule apart, returning every compiled system to `unbuilt_configs` so the
    /// next [`build`](Self::build) can compile them again alongside whatever was just added.
    ///
    /// This used to DROP them. `build` moves each system out of its config into a batch, and the
    /// metadata went with the dropped config, so there was nothing to rebuild from — clearing the
    /// batches discarded the systems for good. The pattern that bit was ordinary: add systems,
    /// run a frame, then register one more (a runtime plugin, the editor, a script) and only the
    /// newest survived; `configure_set` after a build emptied the schedule outright.
    ///
    /// Batches now carry each system's metadata beside it (`SystemBatch::metas`), which is what
    /// makes the round trip possible.
    ///
    /// **Order caveat, worth knowing:** systems come back in batch order, not in their original
    /// registration order. Every declared `before`/`after`/set constraint is preserved — the DAG
    /// is rebuilt from the same labels — but two systems with NO constraint between them may be
    /// examined in a different order than the first time, so which batch each lands in can
    /// differ. That is within what the scheduler has always promised (a batch's systems run
    /// "concurrently, in an unspecified order"), and it only happens on a rebuild.
    fn invalidate(&mut self) {
        let phase_batches = std::mem::take(&mut self.phase_batches);
        let legacy_batches = std::mem::take(&mut self.legacy_batches);

        let mut recovered = 0usize;
        for batch in phase_batches
            .into_iter()
            .flat_map(|(_, batches)| batches)
            .chain(legacy_batches)
        {
            // `systems` and `metas` are pushed together by `add_system_with_meta` and never
            // touched apart, so `zip` pairs each system with its own metadata.
            debug_assert_eq!(batch.systems.len(), batch.metas.len());
            for (system, meta) in batch.systems.into_iter().zip(batch.metas) {
                self.unbuilt_configs
                    .push(SystemConfig::from_parts(system, meta));
                recovered += 1;
            }
        }

        if recovered > 0 {
            tracing::debug!(
                recovered_systems = recovered,
                "Schedule modified after it was built: {recovered} already-compiled system(s)                  returned to the pending list and will be rebuilt on the next run.",
            );
        }
    }

    fn is_built(&self) -> bool {
        !self.phase_batches.is_empty() || !self.legacy_batches.is_empty()
    }

    /// Compiles the schedule now so that a malformed graph is reported at setup time rather
    /// than on the first frame. Exactly equivalent to [`build`](Self::build).
    ///
    /// # Panics
    ///
    /// Panics on a cyclic ordering constraint. Unmatched `before`/`after` labels are *not*
    /// an error — they are logged as warnings and ignored, so this does not validate that
    /// the constraints you wrote actually took effect.
    pub fn validate(&mut self) {
        self.build();
    }

    /// DAG-batch the configs belonging to a single phase group.
    fn build_batches_for(configs: Vec<SystemConfig>) -> Vec<SystemBatch> {
        let count = configs.len();
        if count == 0 {
            return Vec::new();
        }

        let mut edge_set: HashSet<(usize, usize)> = HashSet::new();
        let mut adj = vec![Vec::new(); count];
        let mut in_degree = vec![0usize; count];

        let add_edge = |from: usize,
                        to: usize,
                        edge_set: &mut HashSet<(usize, usize)>,
                        adj: &mut Vec<Vec<usize>>,
                        in_degree: &mut Vec<usize>| {
            if edge_set.insert((from, to)) {
                adj[from].push(to);
                in_degree[to] += 1;
            }
        };

        for i in 0..count {
            for before_label in &configs[i].before {
                let mut found = false;
                for (j, config_j) in configs.iter().enumerate() {
                    if i != j && config_j.labels.contains(before_label) {
                        add_edge(i, j, &mut edge_set, &mut adj, &mut in_degree);
                        found = true;
                    }
                }
                if !found {
                    crate::gizmo_log!(
                        Warning,
                        "[Schedule] Sistem {}'in before('{}') label'ı eşleşmiyor!",
                        i,
                        before_label
                    );
                }
            }
            for after_label in &configs[i].after {
                let mut found = false;
                for (j, config_j) in configs.iter().enumerate() {
                    if i != j && config_j.labels.contains(after_label) {
                        add_edge(j, i, &mut edge_set, &mut adj, &mut in_degree);
                        found = true;
                    }
                }
                if !found {
                    crate::gizmo_log!(
                        Warning,
                        "[Schedule] Sistem {}'in after('{}') label'ı eşleşmiyor!",
                        i,
                        after_label
                    );
                }
            }
        }

        let mut queue = std::collections::VecDeque::new();
        for (i, deg) in in_degree.iter().enumerate() {
            if *deg == 0 {
                queue.push_back(i);
            }
        }

        let mut sorted_indices = Vec::with_capacity(count);
        while let Some(node) = queue.pop_front() {
            sorted_indices.push(node);
            for &neighbor in &adj[node] {
                in_degree[neighbor] -= 1;
                if in_degree[neighbor] == 0 {
                    queue.push_back(neighbor);
                }
            }
        }

        if sorted_indices.len() != count {
            tracing::error!(
                system_count = count,
                sorted = sorted_indices.len(),
                "[Schedule] cyclic system dependency detected — topological sort incomplete"
            );
            panic!(
                "Cyclic dependency detected! {} sistemin {} tanesi sıralanabildi.",
                count,
                sorted_indices.len()
            );
        }

        // Reverse adjacency
        let mut predecessors = vec![Vec::<usize>::new(); count];
        for (from, neighbors) in adj.iter().enumerate() {
            for &to in neighbors {
                predecessors[to].push(from);
            }
        }

        // DAG Batching (optimal greedy)
        let mut dummy_configs: Vec<Option<SystemConfig>> = configs.into_iter().map(Some).collect();
        let mut batches: Vec<SystemBatch> = Vec::new();
        let mut system_batch = vec![0usize; count];

        for &idx in &sorted_indices {
            let config = dummy_configs[idx].take().unwrap();
            // Snapshotted in `build` before set folding, so a rebuild sees what the caller
            // declared rather than the folded-in copy.
            let meta = config.pristine_meta.clone().unwrap_or_else(|| config.snapshot_meta());

            let earliest = predecessors[idx]
                .iter()
                .map(|&pred| system_batch[pred] + 1)
                .max()
                .unwrap_or(0);

            let placed = (earliest..batches.len())
                .rev()
                .find(|&bidx| batches[bidx].is_compatible(&*config.system, &config.added_info));

            let batch_idx = if let Some(bidx) = placed {
                batches[bidx].add_system_with_meta(config.system, config.added_info, meta);
                bidx
            } else {
                let new_idx = batches.len();
                let mut new_batch = SystemBatch::new();
                new_batch.add_system_with_meta(config.system, config.added_info, meta);
                batches.push(new_batch);
                new_idx
            };

            system_batch[idx] = batch_idx;
        }

        batches
    }

    /// Compiles the pending systems into batches. Idempotent: returns immediately if the
    /// schedule is already built, and leaves the schedule unbuilt (so that later additions
    /// still compile) if there is nothing pending.
    ///
    /// Three steps:
    ///
    /// 1. Each registered [`SetConfig`] is folded into its member systems — the set's
    ///    `before`/`after` labels are appended to the member's own, and a set phase
    ///    overwrites the member's phase. Being in a set that was never passed to
    ///    [`configure_set`](Self::configure_set) contributes nothing here beyond the label
    ///    that `in_set` already attached.
    /// 2. If *any* system now sits in a phase other than [`Phase::Update`], the whole
    ///    schedule switches to phase mode: systems are grouped by phase and each group is
    ///    batched independently, groups running in ascending phase order (`PreUpdate` →
    ///    `Update` → `Physics` → `PostUpdate` → `Render`). Otherwise one flat batch list is
    ///    produced, which is the same thing with a single group. Note there is no ordering
    ///    edge *between* phases — the separation comes from the groups being executed one
    ///    after another.
    /// 3. Within a group, `before`/`after` labels become a DAG which is topologically
    ///    sorted (Kahn, seeded in insertion order), and the systems are packed greedily: a
    ///    system goes into the highest-indexed existing batch that is at or after
    ///    `1 + max(batch index of its predecessors)` and whose access does not conflict with
    ///    it, or into a new batch appended at the end. Insertion order is the only
    ///    tie-break, which is what makes the layout deterministic.
    ///
    /// A `before`/`after` label matching no system logs a warning (naming the system by its
    /// index within the group) and is dropped.
    ///
    /// # Panics
    ///
    /// Panics if the ordering constraints contain a cycle, reporting how many of the
    /// systems could be sorted before the topological sort stalled.
    pub fn build(&mut self) {
        if self.is_built() {
            return;
        }

        let mut configs = std::mem::take(&mut self.unbuilt_configs);
        if configs.is_empty() {
            return;
        }
        let system_count = configs.len();

        // Snapshot each config's metadata BEFORE folding sets in, so `invalidate` can put the
        // configs back exactly as the caller declared them. Folding appends to `before`/`after`
        // and can overwrite `phase`; snapshotting after it would make every rebuild re-append the
        // same constraints.
        for config in &mut configs {
            config.pristine_meta = Some(config.snapshot_meta());
        }

        // Apply SetConfigs to systems
        for config in &mut configs {
            for set_name in &config.in_sets {
                if let Some(set_cfg) = self.set_configs.get(set_name) {
                    config.before.extend(set_cfg.before.iter().copied());
                    config.after.extend(set_cfg.after.iter().copied());
                    if let Some(phase) = set_cfg.phase {
                        config.phase = phase;
                    }
                }
            }
        }

        // Herhangi bir config varsayılan olmayan Phase kullanıyor mu?
        let has_explicit_phase = configs.iter().any(|c| c.phase != Phase::Update);
        self.uses_phases = has_explicit_phase;

        if has_explicit_phase {
            // Fazlara göre grupla
            let mut phase_groups: std::collections::BTreeMap<Phase, Vec<SystemConfig>> =
                std::collections::BTreeMap::new();
            for config in configs {
                phase_groups.entry(config.phase).or_default().push(config);
            }
            // Her faz grubu için bağımsız DAG batch oluştur
            for (phase, group) in phase_groups {
                let batches = Self::build_batches_for(group);
                if !batches.is_empty() {
                    self.phase_batches.push((phase, batches));
                }
            }
        } else {
            // Geriye uyumlu: tek düz batch listesi
            self.legacy_batches = Self::build_batches_for(configs);
        }

        let batch_count: usize = if self.uses_phases {
            self.phase_batches.iter().map(|(_, b)| b.len()).sum()
        } else {
            self.legacy_batches.len()
        };
        tracing::debug!(
            system_count,
            batch_count,
            uses_phases = self.uses_phases,
            "[Schedule] built system DAG into parallel batches"
        );
    }

    /// Runs the batch list (within-phase or legacy).
    fn run_batches(batches: &mut [SystemBatch], world: &mut World, dt: f32) {
        #[cfg(not(target_arch = "wasm32"))]
        use rayon::prelude::*;
        #[cfg(target_arch = "wasm32")]
        use crate::parallel_compat::*;

        for batch in batches.iter_mut() {
            batch.systems.par_iter_mut().for_each(|system| {
                system.run(world, dt);
            });

            // Flush deferred entity mutations between batches.
            let queue_clone = world
                .get_resource::<crate::commands::CommandQueue>()
                .filter(|q| !q.is_empty())
                .map(|q| (*q).clone());
            if let Some(queue) = queue_clone {
                queue.apply(world);
            }
        }
    }

    /// Runs every system once, building the schedule first if that has not happened yet.
    ///
    /// Execution order: phases in ascending order, batches within a phase in index order,
    /// and the systems of one batch **concurrently, in an unspecified order** (rayon on
    /// native targets, a sequential shim on `wasm32`). Co-batched systems are guaranteed to
    /// have no conflicting declared access, so their interleaving is normally unobservable —
    /// but side effects outside the access declaration are not ordered. In particular two
    /// systems taking `Commands` declare only a *read* of the command-queue resource, so
    /// they may share a batch and enqueue in a non-reproducible order.
    ///
    /// The deferred `CommandQueue` is drained and applied **after each batch**, on this
    /// thread. Structural changes requested by a batch are therefore visible to the next
    /// batch (and the next phase), never to the rest of the batch that queued them.
    ///
    /// `dt` is forwarded unchanged to every system; nothing here sub-steps or rescales it,
    /// so a fixed-timestep caller passes its own fixed step.
    ///
    /// Change detection: the call opens a window by pointing the world's reference tick at
    /// the tick of this schedule's *previous* run and advancing the world tick by one, so
    /// `Changed<T>`/`Added<T>` mean "since this schedule last ran". The reference is set
    /// once, before any batch starts, so parallel systems cannot race on it. It is stored
    /// per schedule, not per world: two schedules sharing a world each keep their own
    /// window, and a schedule alternated between two worlds compares against a tick from
    /// the wrong one. The window is opened even when the schedule holds no systems, so an
    /// empty `run` still advances the world tick.
    ///
    /// Finally, if a `FrameProfiler` resource is present its current frame is closed — which
    /// is itself a no-op while the profiler is disabled.
    ///
    /// # Panics
    ///
    /// Panics on a cyclic ordering constraint if the schedule still needs building, and
    /// propagates a panic raised by a system. Such an unwind skips that batch's command
    /// flush, every later batch and phase, and the update of the change-detection
    /// reference — so the next `run` re-opens the same window.
    #[tracing::instrument(skip_all, name = "ecs_update")]
    pub fn run(&mut self, world: &mut World, dt: f32) {
        if !self.is_built() && !self.unbuilt_configs.is_empty() {
            self.build();
        }

        // ── Değişiklik tespiti (change detection) penceresi ───────────────
        // Bu frame'in karşılaştırma referansını bir önceki çalıştırmanın tick'ine
        // ayarla ve dünya tick'ini ilerlet. Böylece `Changed<T>`/`Added<T>` "son
        // çalıştırmadan beri değişenleri" raporlar. Referans tek seferde (paralel
        // batch'lerden ÖNCE) ayarlandığından paralel sistemler arasında yarış yok.
        world.begin_change_frame(self.last_run_tick);

        if self.uses_phases {
            // Fazları sırasıyla çalıştır: PreUpdate → Update → Physics → PostUpdate → Render
            for (_phase, batches) in &mut self.phase_batches {
                let _span = tracing::info_span!("phase", name = _phase.name()).entered();
                Self::run_batches(batches, world, dt);
            }
        } else {
            // Legacy mod: düz batch listesi
            Self::run_batches(&mut self.legacy_batches, world, dt);
        }

        // Bir sonraki frame'in referansı: bu frame'in tick'i.
        self.last_run_tick = world.tick;

        // **No `FrameProfiler::end_frame()` here, deliberately: a schedule run is not a frame.**
        //
        // It used to be, and the arithmetic was wrong for every reader of the result. A windowed
        // frame runs the FIXED schedule 0..N times and the UPDATE schedule once
        // (`gizmo_app::frame::run_fixed_and_update`), and then the loop ends the frame itself —
        // so `end_frame` fired `fixed_steps + 2` times per rendered frame, each one resetting
        // `frame_start` and clearing `active_scopes`. Two consequences, measured 2026-08-19:
        // every `avg_frame_ms` / `estimated_fps` the studio shows was a fragment of the real
        // frame (roughly half the time, roughly double the FPS at 60 Hz with one step), and any
        // scope opened around the whole step — the loop's own `"physics"` scope — was wiped
        // before it could be closed.
        //
        // The frame boundary belongs to whoever owns the frame: the windowed loop, the headless
        // loop, and a hand-written loop like `server/`. Each calls `end_frame` once per frame.
    }

    /// The total batch count (for debug / test purposes)
    #[cfg(test)]
    pub(crate) fn total_batch_count(&self) -> usize {
        if self.uses_phases {
            self.phase_batches.iter().map(|(_, b)| b.len()).sum()
        } else {
            self.legacy_batches.len()
        }
    }
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}



#[cfg(test)]
mod modify_after_build {
    use super::*;
    use crate::world::World;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    /// Adding a system to an already-built schedule silently drops every system already
    /// compiled into it.
    ///
    /// Registering a system after the schedule has run keeps the systems already compiled.
    ///
    /// `build()` moves each system out of its config and into a batch. It used to drop the
    /// config's metadata at that moment, so `invalidate()` had nothing to rebuild from and
    /// clearing the batches lost those systems for good. Because `run()` builds lazily on the
    /// first frame, the shape that bit was ordinary: add systems, run a frame, then register one
    /// more — from a plugin loaded at runtime, the editor, or a script — and the schedule was
    /// left holding only the newcomer.
    ///
    /// This test was written to pin that bug, with a note saying that when it started failing
    /// with `2` the ownership fix had landed and the expectation should be updated rather than
    /// the test deleted. That is what happened: batches now carry each system's metadata
    /// (`SystemBatch::metas`) and `invalidate()` reassembles the configs.
    #[test]
    fn a_system_added_after_the_first_run_does_not_drop_the_earlier_ones() {
        let first = Arc::new(AtomicU32::new(0));
        let second = Arc::new(AtomicU32::new(0));
        let (f, s) = (first.clone(), second.clone());

        let mut schedule = Schedule::new();
        schedule.add_system(move |_w: &World, _dt: f32| {
            f.fetch_add(1, Ordering::Relaxed);
        });

        let mut world = World::new();
        schedule.run(&mut world, 0.016); // builds lazily here
        assert_eq!(first.load(Ordering::Relaxed), 1, "the first system must run once");

        schedule.add_system(move |_w: &World, _dt: f32| {
            s.fetch_add(1, Ordering::Relaxed);
        });
        schedule.run(&mut world, 0.016);

        assert_eq!(
            second.load(Ordering::Relaxed),
            1,
            "the newly added system must run"
        );
        assert_eq!(
            first.load(Ordering::Relaxed),
            2,
            "the system registered BEFORE the first run must survive the rebuild and run again; \
             1 here means `invalidate()` dropped it, which is the bug this test was written for"
        );
    }
    /// Ordering constraints survive a rebuild — the part of the fix that is easy to get wrong.
    ///
    /// Returning the systems is not enough on its own: if the metadata came back empty, every
    /// system would still RUN, so the test above would pass while `before`/`after` silently
    /// stopped being honoured. Here the two systems record the order they ran in, and the
    /// constraint is checked again after a system is added post-build.
    #[test]
    fn ordering_constraints_survive_a_rebuild() {
        let log: Arc<std::sync::Mutex<Vec<&'static str>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let (l1, l2) = (log.clone(), log.clone());

        let mut schedule = Schedule::new();
        schedule.add_di_system(
            (move || { l1.lock().unwrap().push("late"); })
                .label("late")
                .after("early"),
        );
        schedule.add_di_system(
            (move || { l2.lock().unwrap().push("early"); })
                .label("early"),
        );

        let mut world = World::new();
        schedule.run(&mut world, 0.016);
        assert_eq!(*log.lock().unwrap(), vec!["early", "late"], "before the rebuild");

        log.lock().unwrap().clear();
        schedule.add_system(|_w: &World, _dt: f32| {});
        schedule.run(&mut world, 0.016);
        assert_eq!(
            *log.lock().unwrap(),
            vec!["early", "late"],
            "after the rebuild the `after(\"early\")` edge must still hold; an empty or lost \
             metadata round-trip would let these run in either order"
        );
    }

    /// `configure_set` after a build was the worst shape of the same bug: it invalidated the
    /// schedule without adding anything, so every compiled system was dropped and the next
    /// `run()` did nothing at all.
    #[test]
    fn configuring_a_set_after_the_first_run_keeps_the_systems() {
        let ran = Arc::new(AtomicU32::new(0));
        let r = ran.clone();

        let mut schedule = Schedule::new();
        schedule.add_system(move |_w: &World, _dt: f32| {
            r.fetch_add(1, Ordering::Relaxed);
        });

        let mut world = World::new();
        schedule.run(&mut world, 0.016);
        assert_eq!(ran.load(Ordering::Relaxed), 1);

        schedule.configure_set(SetConfig {
            name: "a_set_nothing_belongs_to",
            before: Vec::new(),
            after: Vec::new(),
            phase: None,
        });
        schedule.run(&mut world, 0.016);

        assert_eq!(
            ran.load(Ordering::Relaxed),
            2,
            "configuring a set must not empty the schedule; 1 here means the rebuild dropped \
             every system it had already compiled"
        );
    }
}

#[cfg(test)]
mod frame_boundary {
    use super::*;
    use crate::profiler::FrameProfiler;
    use crate::world::World;

    /// **A schedule run is not a frame.**
    ///
    /// `Schedule::run` used to call `FrameProfiler::end_frame()` at its tail, which made the
    /// profiler's "frame" a schedule run instead. A windowed frame runs the fixed schedule
    /// `0..N` times and the update schedule once, and the loop then ends the frame itself — so
    /// the count was `fixed_steps + 2` per rendered frame, each one resetting the clock. Every
    /// `avg_frame_ms` and `estimated_fps` the studio painted was a fragment of the real frame:
    /// roughly half the time and double the rate at 60 Hz with one fixed step.
    #[test]
    fn running_a_schedule_does_not_end_a_frame() {
        let mut world = World::new();
        world.insert_resource(FrameProfiler::new());
        let mut schedule = Schedule::new();

        for _ in 0..5 {
            schedule.run(&mut world, 1.0 / 60.0);
        }
        assert_eq!(
            world.get_resource::<FrameProfiler>().unwrap().frame_count(),
            0,
            "five schedule runs are not five frames — the loop that owns the frame ends it"
        );

        world
            .get_resource_mut::<FrameProfiler>()
            .unwrap()
            .end_frame();
        assert_eq!(
            world.get_resource::<FrameProfiler>().unwrap().frame_count(),
            1,
            "and one `end_frame` is one frame"
        );
    }

    /// A scope opened around a schedule run survives it.
    ///
    /// The other half of the same defect: `end_frame` clears `active_scopes`, so the implicit
    /// call inside `run` destroyed any scope that spanned it — including the windowed loop's own
    /// `"physics"` scope, which wrapped the whole fixed-step drain and could therefore never be
    /// closed or recorded.
    #[test]
    fn a_scope_around_a_schedule_run_survives_it() {
        let mut world = World::new();
        world.insert_resource(FrameProfiler::new());
        let mut schedule = Schedule::new();

        world
            .get_resource_mut::<FrameProfiler>()
            .unwrap()
            .begin_scope("physics");
        schedule.run(&mut world, 1.0 / 60.0);
        {
            let mut profiler = world.get_resource_mut::<FrameProfiler>().unwrap();
            profiler.end_scope("physics");
            profiler.end_frame();
        }

        let profiler = world.get_resource::<FrameProfiler>().unwrap();
        let frame = profiler.last_frame().expect("one frame was ended");
        assert!(
            frame.scopes.iter().any(|s| s.name == "physics"),
            "the scope spanning the run was wiped by the run itself: {:?}",
            frame.scopes.iter().map(|s| s.name).collect::<Vec<_>>()
        );
    }
}
