use crate::world::World;
use std::any::TypeId;
use std::collections::HashSet;

// ==============================================================
// ACCESS INFO (DAG DEPENDENCY GRAPH)
// ==============================================================

#[derive(Default, Clone)]
pub struct AccessInfo {
    pub component_reads: Vec<TypeId>,
    pub component_writes: Vec<TypeId>,
    pub resource_reads: Vec<TypeId>,
    pub resource_writes: Vec<TypeId>,
    pub is_exclusive: bool,
}

impl AccessInfo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_compatible_with(&self, other: &AccessInfo) -> bool {
        if self.is_exclusive || other.is_exclusive {
            return false;
        }

        for w in &self.component_writes {
            if other.component_writes.contains(w) || other.component_reads.contains(w) {
                return false;
            }
        }
        for r in &self.component_reads {
            if other.component_writes.contains(r) {
                return false;
            }
        }

        for w in &self.resource_writes {
            if other.resource_writes.contains(w) || other.resource_reads.contains(w) {
                return false;
            }
        }
        for r in &self.resource_reads {
            if other.resource_writes.contains(r) {
                return false;
            }
        }

        true
    }
}

// ==============================================================
// PHASE (SYSTEM SET GROUPING)
// ==============================================================

/// Fizik motoru tarzı faz sıralaması.
/// Sistemler bir faza atanır ve fazlar sabit sırada çalışır:
/// `PreUpdate → Update → Physics → PostUpdate → Render`
///
/// Aynı faz içindeki sistemler DAG batching ile paralel çalıştırılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Phase {
    /// Input polling, zaman güncellemesi, olay temizliği
    PreUpdate = 0,
    /// Oyun mantığı, AI, scripting
    #[default]
    Update = 1,
    /// Fizik simülasyonu (fixed timestep ile)
    Physics = 2,
    /// Transform propagation, cleanup
    PostUpdate = 3,
    /// Rendering hazırlığı
    Render = 4,
}

impl Phase {
    /// Tüm fazları sıralı olarak döndürür.
    pub const ALL: [Phase; 5] = [
        Phase::PreUpdate,
        Phase::Update,
        Phase::Physics,
        Phase::PostUpdate,
        Phase::Render,
    ];

    /// Faz adını döndürür (tracing span'ları için).
    pub const fn name(&self) -> &'static str {
        match self {
            Phase::PreUpdate => "pre_update",
            Phase::Update => "update",
            Phase::Physics => "physics",
            Phase::PostUpdate => "post_update",
            Phase::Render => "render",
        }
    }
}

// ==============================================================
// SYSTEM TRAIT
// ==============================================================

/// Bir sistem: her frame'de çalıştırılabilir mantık birimi.
pub trait System: Send + Sync {
    fn run(&mut self, world: &World, dt: f32);
    fn access_info(&self) -> AccessInfo;
}

// ==============================================================
// DEPENDENCY INJECTION SİSTEMİ
// ==============================================================

use crate::world::{ResourceReadGuard, ResourceWriteGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemParamFetchError {
    Resource(crate::world::ResourceFetchError),
    QueryError,
}

impl From<crate::world::ResourceFetchError> for SystemParamFetchError {
    fn from(value: crate::world::ResourceFetchError) -> Self {
        Self::Resource(value)
    }
}

pub trait SystemParam {
    type Item<'w>;
    fn fetch<'w>(world: &'w World, dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError>;
    fn get_access_info(info: &mut AccessInfo);
}

pub struct Res<'w, T: 'static> {
    value: ResourceReadGuard<'w, T>,
}

impl<'w, T: 'static> std::ops::Deref for Res<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: 'static> SystemParam for Res<'static, T> {
    type Item<'w> = Res<'w, T>;
    fn fetch<'w>(world: &'w World, _dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError> {
        let value = world.try_get_resource::<T>()?;
        Ok(Res::<T> { value })
    }
    fn get_access_info(info: &mut AccessInfo) {
        info.resource_reads.push(TypeId::of::<T>());
    }
}

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

impl<T: 'static> SystemParam for ResMut<'static, T> {
    type Item<'w> = ResMut<'w, T>;
    fn fetch<'w>(world: &'w World, _dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError> {
        let value = world.try_get_resource_mut::<T>()?;
        Ok(ResMut::<T> { value })
    }
    fn get_access_info(info: &mut AccessInfo) {
        info.resource_writes.push(TypeId::of::<T>());
    }
}

impl SystemParam for f32 {
    type Item<'w> = f32;
    fn fetch<'w>(_world: &'w World, dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError> {
        Ok(dt)
    }
    fn get_access_info(_info: &mut AccessInfo) {}
}

impl<Q: crate::query::WorldQuery + 'static> SystemParam for crate::query::Query<'static, Q> {
    type Item<'w> = crate::query::Query<'w, Q>;
    fn fetch<'w>(world: &'w World, _dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError> {
        if let Some(query) = world.query::<Q>() {
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

// ==============================================================
// INTO SYSTEM — FONKSİYONLARDAN SİSTEME DÖNÜŞÜM (MAKRO İLE)
// ==============================================================

pub trait IntoSystem<Params> {
    fn into_system(self) -> Box<dyn System>;
}

// 0 Parametre
impl<F> IntoSystem<()> for F
where
    F: FnMut() + Send + Sync + 'static,
{
    fn into_system(self) -> Box<dyn System> {
        struct ZeroParamSystem<F>(F);
        impl<F: FnMut() + Send + Sync + 'static> System for ZeroParamSystem<F> {
            fn run(&mut self, _world: &World, _dt: f32) {
                (self.0)();
            }
            fn access_info(&self) -> AccessInfo {
                AccessInfo::new()
            }
        }
        Box::new(ZeroParamSystem(self))
    }
}

/// 1-8 parametreli IntoSystem implementasyonlarını üretir.
macro_rules! impl_into_system {
    ($($P:ident),+) => {
        #[allow(non_snake_case)]
        impl<F, $($P),+> IntoSystem<($($P,)+)> for F
        where
            F: FnMut($($P::Item<'_>),+) + FnMut($($P),+) + Send + Sync + 'static,
            $($P: SystemParam + 'static,)+
        {
            fn into_system(self) -> Box<dyn System> {
                struct MultiParamSystem<F, $($P),+> {
                    func: F,
                    _marker: std::marker::PhantomData<fn() -> ($($P,)+)>,
                }

                impl<F, $($P),+> System for MultiParamSystem<F, $($P),+>
                where
                    F: FnMut($($P::Item<'_>),+) + FnMut($($P),+) + Send + Sync + 'static,
                    $($P: SystemParam + 'static,)+
                {
                    fn run(&mut self, world: &World, dt: f32) {
                        $(
                            let $P = match $P::fetch(world, dt) {
                                Ok(v) => v,
                                Err(e) => {
                                    crate::gizmo_log!(
                                        Warning,
                                        "[SystemParam] fetch failed for {}: {:?}",
                                        std::any::type_name::<$P>(),
                                        e
                                    );
                                    return;
                                }
                            };
                        )+
                        (self.func)($($P),+);
                    }
                    fn access_info(&self) -> AccessInfo {
                        let mut info = AccessInfo::new();
                        $($P::get_access_info(&mut info);)+
                        info
                    }
                }

                Box::new(MultiParamSystem {
                    func: self,
                    _marker: std::marker::PhantomData,
                })
            }
        }
    };
}

impl_into_system!(P1);
impl_into_system!(P1, P2);
impl_into_system!(P1, P2, P3);
impl_into_system!(P1, P2, P3, P4);
impl_into_system!(P1, P2, P3, P4, P5);
impl_into_system!(P1, P2, P3, P4, P5, P6);
impl_into_system!(P1, P2, P3, P4, P5, P6, P7);
impl_into_system!(P1, P2, P3, P4, P5, P6, P7, P8);
impl_into_system!(P1, P2, P3, P4, P5, P6, P7, P8, P9);
impl_into_system!(P1, P2, P3, P4, P5, P6, P7, P8, P9, P10);
impl_into_system!(P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11);
impl_into_system!(P1, P2, P3, P4, P5, P6, P7, P8, P9, P10, P11, P12);

// Func returning &World and using f32 but acts as an Exclusive Barrier!
impl<F> System for F
where
    F: FnMut(&World, f32) + Send + Sync + 'static,
{
    fn run(&mut self, world: &World, dt: f32) {
        (self)(world, dt);
    }
    // Opaque functions act as a full barrier to prevent unsafe overlaps
    fn access_info(&self) -> AccessInfo {
        let mut info = AccessInfo::new();
        info.is_exclusive = true;
        info
    }
}

// ==============================================================
// RUN CONDITIONS
// ==============================================================

pub struct ConditionalSystem {
    inner: Box<dyn System>,
    condition: Box<dyn FnMut(&World) -> bool + Send + Sync>,
}

impl System for ConditionalSystem {
    fn run(&mut self, world: &World, dt: f32) {
        if (self.condition)(world) {
            self.inner.run(world, dt);
        }
    }
    fn access_info(&self) -> AccessInfo {
        self.inner.access_info()
    }
}

// ==============================================================
// SYSTEM CONFIG — LABEL / BEFORE / AFTER / READS / WRITES
// ==============================================================

pub struct SystemConfig {
    pub(crate) system: Box<dyn System>,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) before: Vec<&'static str>,
    pub(crate) after: Vec<&'static str>,
    pub(crate) added_info: AccessInfo,
    pub(crate) phase: Phase,
}

impl SystemConfig {
    pub fn new(system: Box<dyn System>) -> Self {
        Self {
            system,
            labels: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            added_info: AccessInfo::new(),
            phase: Phase::default(),
        }
    }

    pub fn label(mut self, label: &'static str) -> Self {
        self.labels.push(label);
        self
    }

    pub fn before(mut self, target: &'static str) -> Self {
        self.before.push(target);
        self
    }

    pub fn after(mut self, target: &'static str) -> Self {
        self.after.push(target);
        self
    }

    pub fn reads<T: 'static>(mut self) -> Self {
        self.added_info.component_reads.push(TypeId::of::<T>());
        self
    }
    pub fn writes<T: 'static>(mut self) -> Self {
        self.added_info.component_writes.push(TypeId::of::<T>());
        self
    }
    pub fn reads_res<T: 'static>(mut self) -> Self {
        self.added_info.resource_reads.push(TypeId::of::<T>());
        self
    }
    pub fn writes_res<T: 'static>(mut self) -> Self {
        self.added_info.resource_writes.push(TypeId::of::<T>());
        self
    }
    pub fn exclusive(mut self) -> Self {
        self.added_info.is_exclusive = true;
        self
    }
    pub fn in_phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    pub fn run_if<F>(mut self, condition: F) -> Self
    where
        F: FnMut(&World) -> bool + Send + Sync + 'static,
    {
        self.system = Box::new(ConditionalSystem {
            inner: self.system,
            condition: Box::new(condition),
        });
        self
    }
}

pub trait IntoSystemConfig<Params> {
    fn into_config(self) -> SystemConfig;

    fn label(self, l: &'static str) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().label(l)
    }
    fn before(self, target: &'static str) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().before(target)
    }
    fn after(self, target: &'static str) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().after(target)
    }

    fn reads<C: 'static>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().reads::<C>()
    }
    fn writes<C: 'static>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().writes::<C>()
    }
    fn reads_res<C: 'static>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().reads_res::<C>()
    }
    fn writes_res<C: 'static>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().writes_res::<C>()
    }
    fn exclusive(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().exclusive()
    }
    fn in_phase(self, phase: Phase) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().in_phase(phase)
    }
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

// ==============================================================
// SCHEDULE — DAG BATCHING & MULTITHREADING
// ==============================================================

pub struct SystemBatch {
    systems: Vec<Box<dyn System>>,
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

pub struct Schedule {
    unbuilt_configs: Vec<SystemConfig>,
    /// Her faz için ayrı batch listesi. Fazlar sıralı çalışır, faz içi batch'ler paralel.
    phase_batches: Vec<(Phase, Vec<SystemBatch>)>,
    /// Geriye dönük uyumluluk: faz kullanılmadığında eski düz batch listesi.
    legacy_batches: Vec<SystemBatch>,
    uses_phases: bool,
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            unbuilt_configs: Vec::new(),
            phase_batches: Vec::new(),
            legacy_batches: Vec::new(),
            uses_phases: false,
        }
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

    fn build(&mut self) {
        if self.is_built() {
            return;
        }

        let configs = std::mem::take(&mut self.unbuilt_configs);
        if configs.is_empty() {
            return;
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

        // Frame profiling verisini kaydet (ring buffer'a yaz)
        if let Some(mut profiler) = world.get_resource_mut::<crate::profiler::FrameProfiler>() {
            profiler.end_frame();
        }
    }

    /// Toplam batch sayısı (debug / test amaçlı)
    #[cfg(test)]
    fn total_batch_count(&self) -> usize {
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
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // --- Mock Bileşen ve Kaynaklar ---
    struct CompA;
    struct CompB;

    // Testlerin çalışma sırasını takip etmek için kullanacağımız log
    #[derive(Clone)]
    struct RunLog {
        log: Arc<Mutex<Vec<&'static str>>>,
    }

    impl RunLog {
        fn new() -> Self {
            Self {
                log: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn push(&self, msg: &'static str) {
            self.log.lock().unwrap().push(msg);
        }
        fn get(&self) -> Vec<&'static str> {
            self.log.lock().unwrap().clone()
        }
    }

    // Basit bir test sistemi oluşturucu
    fn create_system(name: &'static str, log: RunLog) -> impl FnMut() + Send + Sync + 'static {
        move || {
            log.push(name);
        }
    }

    #[test]
    fn test_schedule_access_info_compatibility() {
        let mut info1 = AccessInfo::new();
        info1.component_reads.push(TypeId::of::<CompA>());

        let mut info2 = AccessInfo::new();
        info2.component_reads.push(TypeId::of::<CompA>());

        // İki sistem de sadece OKUYOR, birbiriyle uyumlu (parallel çalışabilir)
        assert!(info1.is_compatible_with(&info2));

        let mut info3 = AccessInfo::new();
        info3.component_writes.push(TypeId::of::<CompA>());

        // Biri okuyor diğeri YAZIYOR, uyumsuz (farklı batch'lerde olmalı)
        assert!(!info1.is_compatible_with(&info3));

        // İkisi de YAZIYOR, uyumsuz
        let mut info4 = AccessInfo::new();
        info4.component_writes.push(TypeId::of::<CompA>());
        assert!(!info3.is_compatible_with(&info4));
    }

    #[test]
    fn test_schedule_dag_batching_independent() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // 3 bağımsız sistem, read/write çakışması yok. Tek bir batch içinde çalışmalı.
        schedule.add_di_system(create_system("sys1", log.clone()));
        schedule.add_di_system(create_system("sys2", log.clone()));
        schedule.add_di_system(create_system("sys3", log.clone()));

        schedule.build();

        // Hepsi aynı anda paralel çalışabileceği için 1 adet batch oluşmalı
        assert_eq!(schedule.legacy_batches.len(), 1);
        assert_eq!(schedule.legacy_batches[0].systems.len(), 3);
    }

    #[test]
    fn test_schedule_dag_batching_with_conflicts() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // sys1: CompA yazıyor
        schedule.add_di_system(create_system("sys1", log.clone()).writes::<CompA>());
        // sys2: CompA okuyor (sys1 ile çakışır, ayrı batch'e gitmeli)
        schedule.add_di_system(create_system("sys2", log.clone()).reads::<CompA>());
        // sys3: CompB yazıyor (hiçbiriyle çakışmaz, sys1 ile aynı batch'e girebilir)
        schedule.add_di_system(create_system("sys3", log.clone()).writes::<CompB>());
        // sys4: CompA yazıyor (sys1 ve sys2 ile çakışır, en sona kalmalı)
        schedule.add_di_system(create_system("sys4", log.clone()).writes::<CompA>());

        schedule.build();

        // Beklenen Batch'ler (Greedy Backward Scan):
        // Batch 0: sys1 (writes CompA)
        // Batch 1: sys2 (reads CompA), sys3 (writes CompB)
        // Batch 2: sys4 (writes CompA)
        assert_eq!(schedule.legacy_batches.len(), 3);
        assert_eq!(schedule.legacy_batches[0].systems.len(), 1);
        assert_eq!(schedule.legacy_batches[1].systems.len(), 2);
        assert_eq!(schedule.legacy_batches[2].systems.len(), 1);
    }

    #[test]
    fn test_schedule_explicit_ordering_before_after() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // sys1 "after" sys2 olarak işaretlendi
        schedule.add_di_system(
            create_system("sys1", log.clone())
                .label("System1")
                .after("System2"),
        );

        schedule.add_di_system(create_system("sys2", log.clone()).label("System2"));

        // sys3 "before" sys2 olarak işaretlendi
        schedule.add_di_system(
            create_system("sys3", log.clone())
                .label("System3")
                .before("System2"),
        );

        schedule.build();

        // Bağımsız olsalar bile (okuma/yazma çakışması olmasa dahi) explicit order yüzünden:
        // Sıralama: sys3 -> sys2 -> sys1 olmalı ve farklı batch'lerde olmalılar
        assert_eq!(schedule.legacy_batches.len(), 3);

        let mut world = World::new();
        schedule.run(&mut world, 0.1);

        let result = log.get();
        assert_eq!(result, vec!["sys3", "sys2", "sys1"]);
    }

    #[test]
    #[should_panic(expected = "Cyclic dependency detected!")]
    fn test_schedule_cyclic_dependency_panics() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        schedule.add_di_system(create_system("sysA", log.clone()).label("A").before("B"));

        schedule.add_di_system(create_system("sysB", log.clone()).label("B").before("C"));

        schedule.add_di_system(
            create_system("sysC", log.clone()).label("C").before("A"), // Cycle: A -> B -> C -> A
        );

        // Bu çağrı panic atmalı
        schedule.build();
    }

    #[test]
    fn test_schedule_phase_ordering() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // 3 sistem farklı fazlara atanmış — veri çakışması yok ama
        // faz sıralaması garanti edilmeli: PreUpdate → Physics → Render
        schedule.add_di_system(create_system("render_sys", log.clone()).in_phase(Phase::Render));
        schedule.add_di_system(create_system("physics_sys", log.clone()).in_phase(Phase::Physics));
        schedule
            .add_di_system(create_system("pre_update_sys", log.clone()).in_phase(Phase::PreUpdate));

        schedule.build();

        // Phase modunda olmalı
        assert!(schedule.uses_phases);
        // 3 faz grubu oluşmalı
        assert_eq!(schedule.phase_batches.len(), 3);
        // Sıralama: PreUpdate(0) < Physics(2) < Render(4)
        assert_eq!(schedule.phase_batches[0].0, Phase::PreUpdate);
        assert_eq!(schedule.phase_batches[1].0, Phase::Physics);
        assert_eq!(schedule.phase_batches[2].0, Phase::Render);

        let mut world = World::new();
        schedule.run(&mut world, 0.016);

        // Çalışma sırası deterministik olmalı
        let result = log.get();
        assert_eq!(result, vec!["pre_update_sys", "physics_sys", "render_sys"]);
    }

    #[test]
    fn test_schedule_phase_with_intra_phase_batching() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // Physics fazında 2 çakışan sistem + 1 bağımsız sistem
        schedule.add_di_system(
            create_system("phys1", log.clone())
                .in_phase(Phase::Physics)
                .writes::<CompA>(),
        );
        schedule.add_di_system(
            create_system("phys2", log.clone())
                .in_phase(Phase::Physics)
                .reads::<CompA>(),
        );
        // Update fazında 1 bağımsız sistem
        schedule.add_di_system(create_system("update_sys", log.clone()).in_phase(Phase::Update));

        schedule.build();

        assert!(schedule.uses_phases);
        // 2 faz grubu: Update ve Physics
        assert_eq!(schedule.phase_batches.len(), 2);
        assert_eq!(schedule.phase_batches[0].0, Phase::Update);
        assert_eq!(schedule.phase_batches[1].0, Phase::Physics);

        // Physics fazı 2 batch'e ayrılmalı (writes/reads çakışması)
        assert_eq!(schedule.phase_batches[1].1.len(), 2);

        // Toplam batch sayısı: Update(1) + Physics(2) = 3
        assert_eq!(schedule.total_batch_count(), 3);
    }
}
