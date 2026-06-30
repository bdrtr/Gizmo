use super::*;
use crate::world::World;
use std::collections::HashSet;

// ==============================================================
// SCHEDULE — DAG BATCHING & MULTITHREADING
// ==============================================================

pub struct SystemBatch {
    pub(crate) systems: Vec<Box<dyn System>>,
    pub access_info: AccessInfo,
}

impl Default for SystemBatch {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemBatch {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
            access_info: AccessInfo::new(),
        }
    }

    pub fn add_system(&mut self, system: Box<dyn System>, config_info: AccessInfo) {
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
    }

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

pub struct SetConfig {
    pub name: &'static str,
    pub before: Vec<&'static str>,
    pub after: Vec<&'static str>,
    pub phase: Option<Phase>,
}

impl SetConfig {
    pub fn new<S: SystemSet>() -> Self {
        Self {
            name: S::set_name(),
            before: Vec::new(),
            after: Vec::new(),
            phase: None,
        }
    }
    pub fn before<S: SystemSet>(mut self) -> Self {
        self.before.push(S::set_name());
        self
    }
    pub fn after<S: SystemSet>(mut self) -> Self {
        self.after.push(S::set_name());
        self
    }
    pub fn in_phase(mut self, phase: Phase) -> Self {
        self.phase = Some(phase);
        self
    }
}

pub struct Schedule {
    unbuilt_configs: Vec<SystemConfig>,
    set_configs: std::collections::HashMap<&'static str, SetConfig>,
    /// Her faz için ayrı batch listesi. Fazlar sıralı çalışır, faz içi batch'ler paralel.
    pub(crate) phase_batches: Vec<(Phase, Vec<SystemBatch>)>,
    /// Geriye dönük uyumluluk: faz kullanılmadığında eski düz batch listesi.
    pub(crate) legacy_batches: Vec<SystemBatch>,
    pub(crate) uses_phases: bool,
    /// Bu schedule'ın en son çalıştığı dünya tick'i — değişiklik tespiti (change
    /// detection) için referans. Her `run`'da bir önceki değerle karşılaştırma yapılır.
    last_run_tick: u32,
}

impl Schedule {
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

    pub fn configure_set(&mut self, config: SetConfig) {
        self.set_configs.insert(config.name, config);
        self.invalidate();
    }

    pub fn add_di_system<Params, S: IntoSystemConfig<Params>>(&mut self, system: S) {
        self.unbuilt_configs.push(system.into_config());
        self.invalidate();
    }

    pub fn add_system<S: System + 'static>(&mut self, system: S) {
        self.unbuilt_configs
            .push(SystemConfig::new(Box::new(system)));
        self.invalidate();
    }

    pub fn add_systems<T, Configs: IntoSystemConfigs<T>>(&mut self, configs: Configs) {
        configs.into_configs(self);
    }

    pub fn add_system_boxed(&mut self, system: Box<dyn System>) {
        self.unbuilt_configs.push(SystemConfig::new(system));
        self.invalidate();
    }

    fn invalidate(&mut self) {
        self.phase_batches.clear();
        self.legacy_batches.clear();
    }

    fn is_built(&self) -> bool {
        !self.phase_batches.is_empty() || !self.legacy_batches.is_empty()
    }

    pub fn validate(&mut self) {
        self.build();
    }

    /// Tek bir faz grubuna ait config'leri DAG-batch'le.
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

            let earliest = predecessors[idx]
                .iter()
                .map(|&pred| system_batch[pred] + 1)
                .max()
                .unwrap_or(0);

            let placed = (earliest..batches.len())
                .rev()
                .find(|&bidx| batches[bidx].is_compatible(&*config.system, &config.added_info));

            let batch_idx = if let Some(bidx) = placed {
                batches[bidx].add_system(config.system, config.added_info);
                bidx
            } else {
                let new_idx = batches.len();
                let mut new_batch = SystemBatch::new();
                new_batch.add_system(config.system, config.added_info);
                batches.push(new_batch);
                new_idx
            };

            system_batch[idx] = batch_idx;
        }

        batches
    }

    pub fn build(&mut self) {
        if self.is_built() {
            return;
        }

        let mut configs = std::mem::take(&mut self.unbuilt_configs);
        if configs.is_empty() {
            return;
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
    }

    /// Batch listesini çalıştırır (faz-içi veya legacy).
    fn run_batches(batches: &mut [SystemBatch], world: &mut World, dt: f32) {
        use rayon::prelude::*;

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

        // Frame profiling verisini kaydet (ring buffer'a yaz)
        if let Some(mut profiler) = world.get_resource_mut::<crate::profiler::FrameProfiler>() {
            profiler.end_frame();
        }
    }

    /// Toplam batch sayısı (debug / test amaçlı)
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

