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

pub trait SystemParam {
    type Item<'w>;
    fn fetch<'w>(world: &'w World, dt: f32) -> Option<Self::Item<'w>>;
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
    fn fetch<'w>(world: &'w World, _dt: f32) -> Option<Self::Item<'w>> {
        let value = world.get_resource::<T>()?;
        Some(Res::<T> { value })
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
    fn fetch<'w>(world: &'w World, _dt: f32) -> Option<Self::Item<'w>> {
        let value = world.get_resource_mut::<T>()?;
        Some(ResMut::<T> { value })
    }
    fn get_access_info(info: &mut AccessInfo) {
        info.resource_writes.push(TypeId::of::<T>());
    }
}

impl SystemParam for f32 {
    type Item<'w> = f32;
    fn fetch<'w>(_world: &'w World, dt: f32) -> Option<Self::Item<'w>> {
        Some(dt)
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
            fn run(&mut self, _world: &World, _dt: f32) { (self.0)(); }
            fn access_info(&self) -> AccessInfo { AccessInfo::new() }
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
                        $(let $P = $P::fetch(world, dt);)+
                        if let ($(Some($P),)+) = ($($P,)+) {
                            (self.func)($($P),+);
                        }
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
    
    fn label(self, l: &'static str) -> SystemConfig where Self: Sized { self.into_config().label(l) }
    fn before(self, target: &'static str) -> SystemConfig where Self: Sized { self.into_config().before(target) }
    fn after(self, target: &'static str) -> SystemConfig where Self: Sized { self.into_config().after(target) }
    
    fn reads<C: 'static>(self) -> SystemConfig where Self: Sized { self.into_config().reads::<C>() }
    fn writes<C: 'static>(self) -> SystemConfig where Self: Sized { self.into_config().writes::<C>() }
    fn reads_res<C: 'static>(self) -> SystemConfig where Self: Sized { self.into_config().reads_res::<C>() }
    fn writes_res<C: 'static>(self) -> SystemConfig where Self: Sized { self.into_config().writes_res::<C>() }
    fn exclusive(self) -> SystemConfig where Self: Sized { self.into_config().exclusive() }
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
        Self { systems: Vec::new(), access_info: AccessInfo::new() }
    }
    
    pub fn add_system(&mut self, system: Box<dyn System>, config_info: AccessInfo) {
        let mut sys_info = system.access_info();
        sys_info.component_reads.extend(config_info.component_reads);
        sys_info.component_writes.extend(config_info.component_writes);
        sys_info.resource_reads.extend(config_info.resource_reads);
        sys_info.resource_writes.extend(config_info.resource_writes);
        sys_info.is_exclusive = sys_info.is_exclusive || config_info.is_exclusive;

        self.access_info.component_reads.extend(sys_info.component_reads);
        self.access_info.component_writes.extend(sys_info.component_writes);
        self.access_info.resource_reads.extend(sys_info.resource_reads);
        self.access_info.resource_writes.extend(sys_info.resource_writes);
        self.access_info.is_exclusive = self.access_info.is_exclusive || sys_info.is_exclusive;

        self.systems.push(system);
    }
    
    pub fn is_compatible(&self, system: &dyn System, config_info: &AccessInfo) -> bool {
        let mut sys_info = system.access_info();
        sys_info.component_reads.extend(config_info.component_reads.iter().cloned());
        sys_info.component_writes.extend(config_info.component_writes.iter().cloned());
        sys_info.resource_reads.extend(config_info.resource_reads.iter().cloned());
        sys_info.resource_writes.extend(config_info.resource_writes.iter().cloned());
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
        Self { unbuilt_configs: Vec::new(), batches: Vec::new() }
    }

    pub fn add_di_system<Params, S: IntoSystemConfig<Params>>(&mut self, system: S) {
        self.unbuilt_configs.push(system.into_config());
        self.batches.clear();
    }

    pub fn add_system<S: System + 'static>(&mut self, system: S) {
        self.unbuilt_configs.push(SystemConfig::new(Box::new(system)));
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
        if !self.batches.is_empty() { return; }
        
        let configs = std::mem::take(&mut self.unbuilt_configs);
        let count = configs.len();
        if count == 0 { return; }

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
                        add_edge(i, j); found = true;
                    }
                }
                if !found {
                    crate::gizmo_log!(Warning, "[Schedule] Sistem {}'in before('{}') label'ı eşleşmiyor!", i, before_label);
                }
            }
            for after_label in &configs[i].after {
                let mut found = false;
                for j in 0..count {
                    if i != j && configs[j].labels.contains(after_label) {
                        add_edge(j, i); found = true;
                    }
                }
                if !found {
                    crate::gizmo_log!(Warning, "[Schedule] Sistem {}'in after('{}') label'ı eşleşmiyor!", i, after_label);
                }
            }
        }
        
        let mut queue = std::collections::VecDeque::new();
        for i in 0..count {
            if in_degree[i] == 0 { queue.push_back(i); }
        }
        
        let mut sorted_indices = Vec::with_capacity(count);
        while let Some(node) = queue.pop_front() {
            sorted_indices.push(node);
            for &neighbor in &adj[node] {
                in_degree[neighbor] -= 1;
                if in_degree[neighbor] == 0 { queue.push_back(neighbor); }
            }
        }
        
        if sorted_indices.len() != count {
            panic!("Cyclic dependency detected! {} sistemin {} tanesi sıralanabildi.", count, sorted_indices.len());
        }
        
        // --- 2. DAG Batching --- //
        let mut dummy_configs: Vec<Option<SystemConfig>> = configs.into_iter().map(Some).collect();
        let mut batches: Vec<SystemBatch> = Vec::new();
        
        for &idx in &sorted_indices {
            let config = dummy_configs[idx].take().unwrap();
            let mut placed = false;

            if let Some(current_batch) = batches.last_mut() {
                if current_batch.is_compatible(&*config.system, &config.added_info) {
                    placed = true;
                }
            }
            
            if placed {
                batches.last_mut().unwrap().add_system(config.system, config.added_info);
            } else {
                let mut new_batch = SystemBatch::new();
                new_batch.add_system(config.system, config.added_info);
                batches.push(new_batch);
            }
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

            // Her batch bitiminde CommandQueue (eklenen entity'leri) çalıştır
            let queue_opt = world.get_resource::<crate::commands::CommandQueue>().map(|q| (*q).clone());
            if let Some(queue) = queue_opt {
                queue.apply(world);
            }
        }
    }
}

impl Default for Schedule {
    fn default() -> Self { Self::new() }
}
