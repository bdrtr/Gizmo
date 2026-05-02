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
            F: FnMut($($P::Item<'_>),+) + Send + Sync + 'static,
            $($P: SystemParam + 'static,)+
        {
            fn into_system(self) -> Box<dyn System> {
                struct MultiParamSystem<F, $($P),+> {
                    func: F,
                    _marker: std::marker::PhantomData<fn() -> ($($P,)+)>,
                }

                impl<F, $($P),+> System for MultiParamSystem<F, $($P),+>
                where
                    F: FnMut($($P::Item<'_>),+) + Send + Sync + 'static,
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
// SYSTEM CONFIG — LABEL / BEFORE / AFTER / READS / WRITES
// ==============================================================

pub struct SystemConfig {
    pub(crate) system: Box<dyn System>,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) before: Vec<&'static str>,
    pub(crate) after: Vec<&'static str>,
    pub(crate) added_info: AccessInfo,
}

impl SystemConfig {
    pub fn new(system: Box<dyn System>) -> Self {
        Self {
            system,
            labels: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
            added_info: AccessInfo::new(),
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
    batches: Vec<SystemBatch>,
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            unbuilt_configs: Vec::new(),
            batches: Vec::new(),
        }
    }

    pub fn add_di_system<Params, S: IntoSystemConfig<Params>>(&mut self, system: S) {
        self.unbuilt_configs.push(system.into_config());
        self.batches.clear();
    }

    pub fn add_system<S: System + 'static>(&mut self, system: S) {
        self.unbuilt_configs
            .push(SystemConfig::new(Box::new(system)));
        self.batches.clear();
    }

    pub fn add_system_boxed(&mut self, system: Box<dyn System>) {
        self.unbuilt_configs.push(SystemConfig::new(system));
        self.batches.clear();
    }

    pub fn validate(&mut self) {
        self.build();
    }

    fn build(&mut self) {
        if !self.batches.is_empty() {
            return;
        }

        let configs = std::mem::take(&mut self.unbuilt_configs);
        let count = configs.len();
        if count == 0 {
            return;
        }

        let mut edge_set: HashSet<(usize, usize)> = HashSet::new();
        let mut adj = vec![Vec::new(); count];
        let mut in_degree = vec![0usize; count];

        let mut add_edge = |from: usize, to: usize| {
            if edge_set.insert((from, to)) {
                adj[from].push(to);
                in_degree[to] += 1;
            }
        };

        for i in 0..count {
            for before_label in &configs[i].before {
                let mut found = false;
                for j in 0..count {
                    if i != j && configs[j].labels.contains(before_label) {
                        add_edge(i, j);
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
                for j in 0..count {
                    if i != j && configs[j].labels.contains(after_label) {
                        add_edge(j, i);
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
        for i in 0..count {
            if in_degree[i] == 0 {
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

        // Reverse adjacency: predecessors[i] = all systems that must finish before i.
        let mut predecessors = vec![Vec::<usize>::new(); count];
        for from in 0..count {
            for &to in &adj[from] {
                predecessors[to].push(from);
            }
        }

        // --- 2. DAG Batching (optimal greedy) --- //
        // For each system in topological order:
        //   1. earliest_batch = max(batch_of_predecessor) + 1  (respect explicit ordering)
        //   2. scan backwards from latest batch to earliest_batch for a compatible slot
        //   3. if found → add there; otherwise open a new batch
        // This maximises parallelism while preserving all dependency and access constraints.
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

            // Scan backwards from the latest existing batch to `earliest`.
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

        self.batches = batches;
    }

    pub fn run(&mut self, world: &mut World, dt: f32) {
        use rayon::prelude::*;

        if self.batches.is_empty() && !self.unbuilt_configs.is_empty() {
            self.build();
        }

        for batch in &mut self.batches {
            // Paralel çalıştırıyoruz! Bütün sistemler sadece "&World" alır.
            // Resource / Column tabanlı kilitler içeride yönetilir.
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
    struct ResA;

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
        assert_eq!(schedule.batches.len(), 1);
        assert_eq!(schedule.batches[0].systems.len(), 3);
    }

    #[test]
    fn test_schedule_dag_batching_with_conflicts() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // sys1: CompA yazıyor
        schedule.add_di_system(
            create_system("sys1", log.clone()).writes::<CompA>()
        );
        // sys2: CompA okuyor (sys1 ile çakışır, ayrı batch'e gitmeli)
        schedule.add_di_system(
            create_system("sys2", log.clone()).reads::<CompA>()
        );
        // sys3: CompB yazıyor (hiçbiriyle çakışmaz, sys1 ile aynı batch'e girebilir)
        schedule.add_di_system(
            create_system("sys3", log.clone()).writes::<CompB>()
        );
        // sys4: CompA yazıyor (sys1 ve sys2 ile çakışır, en sona kalmalı)
        schedule.add_di_system(
            create_system("sys4", log.clone()).writes::<CompA>()
        );

        schedule.build();

        // Beklenen Batch'ler (Greedy Backward Scan):
        // Batch 0: sys1 (writes CompA)
        // Batch 1: sys2 (reads CompA), sys3 (writes CompB)
        // Batch 2: sys4 (writes CompA)
        assert_eq!(schedule.batches.len(), 3);
        assert_eq!(schedule.batches[0].systems.len(), 1);
        assert_eq!(schedule.batches[1].systems.len(), 2);
        assert_eq!(schedule.batches[2].systems.len(), 1);
    }

    #[test]
    fn test_schedule_explicit_ordering_before_after() {
        let mut schedule = Schedule::new();
        let log = RunLog::new();

        // sys1 "after" sys2 olarak işaretlendi
        schedule.add_di_system(
            create_system("sys1", log.clone())
                .label("System1")
                .after("System2")
        );
        
        schedule.add_di_system(
            create_system("sys2", log.clone())
                .label("System2")
        );

        // sys3 "before" sys2 olarak işaretlendi
        schedule.add_di_system(
            create_system("sys3", log.clone())
                .label("System3")
                .before("System2")
        );

        schedule.build();

        // Bağımsız olsalar bile (okuma/yazma çakışması olmasa dahi) explicit order yüzünden:
        // Sıralama: sys3 -> sys2 -> sys1 olmalı ve farklı batch'lerde olmalılar
        assert_eq!(schedule.batches.len(), 3);

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

        schedule.add_di_system(
            create_system("sysA", log.clone())
                .label("A")
                .before("B")
        );
        
        schedule.add_di_system(
            create_system("sysB", log.clone())
                .label("B")
                .before("C")
        );

        schedule.add_di_system(
            create_system("sysC", log.clone())
                .label("C")
                .before("A") // Cycle: A -> B -> C -> A
        );

        // Bu çağrı panic atmalı
        schedule.build();
    }
}
