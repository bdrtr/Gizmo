use crate::world::World;
use std::collections::HashSet;

/// Bir sistem: her frame'de çalıştırılabilir mantık birimi.
pub trait System {
    fn run(&mut self, world: &mut World, dt: f32);
}

// Basit fonksiyonlar için blanket impl: fn(&mut World, f32)
impl<F> System for F
where
    F: FnMut(&mut World, f32),
{
    fn run(&mut self, world: &mut World, dt: f32) {
        (self)(world, dt);
    }
}

// ==============================================================
// DEPENDENCY INJECTION SİSTEMİ
// ==============================================================

use crate::world::{ResourceReadGuard, ResourceWriteGuard};

/// Bir fonksiyonun sistem parametresi olarak alabileceği argümanları tanımlar.
pub trait SystemParam {
    type Item<'w>;
    fn fetch<'w>(world: &'w World, dt: f32) -> Option<Self::Item<'w>>;
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
        Some(Res { value })
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
        Some(ResMut { value })
    }
}

/// dt (delta time) enjeksiyonu — f32 parametresi olarak alınabilir.
impl SystemParam for f32 {
    type Item<'w> = f32;
    fn fetch<'w>(_world: &'w World, dt: f32) -> Option<Self::Item<'w>> {
        Some(dt)
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
    F: FnMut() + 'static,
{
    fn into_system(mut self) -> Box<dyn System> {
        Box::new(move |_world: &mut World, _dt: f32| {
            (self)();
        })
    }
}

/// 1-8 parametreli IntoSystem implementasyonlarını üretir.
macro_rules! impl_into_system {
    ($($P:ident),+) => {
        #[allow(non_snake_case)]
        impl<F, $($P),+> IntoSystem<($($P,)+)> for F
        where
            F: FnMut($($P::Item<'_>),+) + 'static,
            $($P: SystemParam + 'static,)+
        {
            fn into_system(mut self) -> Box<dyn System> {
                Box::new(move |world: &mut World, dt: f32| {
                    $(let $P = $P::fetch(world, dt);)+
                    if let ($(Some($P),)+) = ($($P,)+) {
                        (self)($($P),+);
                    }
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

// ==============================================================
// SYSTEM CONFIG — LABEL / BEFORE / AFTER SİSTEMİ
// ==============================================================

pub struct SystemConfig {
    system: Box<dyn System>,
    labels: Vec<&'static str>,
    before: Vec<&'static str>,
    after: Vec<&'static str>,
}

impl SystemConfig {
    pub fn new(system: Box<dyn System>) -> Self {
        Self {
            system,
            labels: Vec::new(),
            before: Vec::new(),
            after: Vec::new(),
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
}

pub trait IntoSystemConfig<Params> {
    fn into_config(self) -> SystemConfig;
    
    fn label(self, l: &'static str) -> SystemConfig where Self: Sized {
        self.into_config().label(l)
    }
    
    fn before(self, target: &'static str) -> SystemConfig where Self: Sized {
        self.into_config().before(target)
    }
    
    fn after(self, target: &'static str) -> SystemConfig where Self: Sized {
        self.into_config().after(target)
    }
}

impl<Params, T: IntoSystem<Params>> IntoSystemConfig<Params> for T {
    fn into_config(self) -> SystemConfig {
        SystemConfig::new(self.into_system())
    }
}

// SystemConfig zaten config — identity dönüşümü. .label().before() zinciri
// SystemConfig ürettiği için add_di_system()'a geçilebilmesi gerekir.
impl IntoSystemConfig<()> for SystemConfig {
    fn into_config(self) -> SystemConfig {
        self
    }
}

// ==============================================================
// SCHEDULE — SİSTEM ÇALIŞMA SIRASI VE DEPENDENCY GRAPH
// ==============================================================

pub struct Schedule {
    unbuilt_configs: Vec<SystemConfig>,
    systems: Vec<Box<dyn System>>,
    is_built: bool,
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            unbuilt_configs: Vec::new(),
            systems: Vec::new(),
            is_built: false,
        }
    }

    /// DI destekli sistem ekler. `.label()`, `.before()`, `.after()` zincirlenebilir.
    pub fn add_di_system<Params, S: IntoSystemConfig<Params>>(&mut self, system: S) {
        self.unbuilt_configs.push(system.into_config());
        self.is_built = false;
    }

    /// Basit sistem ekler: `fn(&mut World, f32)` veya `impl System`.
    pub fn add_system<S: System + 'static>(&mut self, system: S) {
        self.unbuilt_configs.push(SystemConfig::new(Box::new(system)));
        self.is_built = false;
    }

    /// Zaten boxed olarak oluşturulmuş bir sistemi ekler.
    /// `IntoSystem::into_system()` ile elde edilen `Box<dyn System>` için kullanılır.
    pub fn add_system_boxed(&mut self, system: Box<dyn System>) {
        self.unbuilt_configs.push(SystemConfig::new(system));
        self.is_built = false;
    }

    /// Dependency graph'ı doğrular. Döngüsel bağımlılık varsa panic yapar.
    /// `run()` öncesinde erken hata tespiti için kullanılabilir.
    pub fn validate(&mut self) {
        self.build();
    }

    /// Dependency graph'ı oluşturur ve sistemleri topological sort ile sıralar.
    fn build(&mut self) {
        if self.is_built {
            return;
        }
        
        let configs = std::mem::take(&mut self.unbuilt_configs);
        let count = configs.len();
        
        if count == 0 {
            self.is_built = true;
            return;
        }

        // Deduplicated edge set — aynı (i → j) kenarı bir kez eklenir.
        let mut edge_set: HashSet<(usize, usize)> = HashSet::new();
        let mut adj = vec![Vec::new(); count];
        let mut in_degree = vec![0usize; count];
        
        // Yardımcı: kenar ekle (deduplicated)
        let mut add_edge = |from: usize, to: usize| {
            if edge_set.insert((from, to)) {
                adj[from].push(to);
                in_degree[to] += 1;
            }
        };

        // Resolve relations
        for i in 0..count {
            // "before": config[i] runs before config[j]
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
                        "[Schedule] Sistem {}'in before('{}') label'ı hiçbir sistemle eşleşmiyor!",
                        i, before_label
                    );
                }
            }
            
            // "after": config[i] runs after config[j]
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
                        "[Schedule] Sistem {}'in after('{}') label'ı hiçbir sistemle eşleşmiyor!",
                        i, after_label
                    );
                }
            }
        }
        
        // Kahn's Topological Sort
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
                "Cyclic dependency detected in System execution graph! \
                 {} sistem var ama sadece {} tanesi sıralanabildi.",
                count, sorted_indices.len()
            );
        }
        
        // Sıralı sistemleri oluştur
        let mut final_systems = Vec::with_capacity(count);
        let mut dummy_configs: Vec<Option<SystemConfig>> = configs.into_iter().map(Some).collect();
        
        for &idx in &sorted_indices {
            let config = dummy_configs[idx].take().unwrap();
            final_systems.push(config.system);
        }
        
        // Mevcut sistemlerin üzerine yaz — tüm sistemler unbuilt_configs'tan geliyor
        self.systems = final_systems;
        self.is_built = true;
    }

    pub fn run(&mut self, world: &mut World, dt: f32) {
        if !self.is_built {
            self.build();
        }
        for system in &mut self.systems {
            system.run(world, dt);

            // Command Queue Flush - Run deferred operations
            let queue_opt = world.get_resource::<crate::commands::CommandQueue>().map(|q| (*q).clone());
            if let Some(queue) = queue_opt {
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

    // ──── Execution Order ────

    #[test]
    fn test_system_execution_order() {
        let mut world = World::new();
        let tracker = Arc::new(Mutex::new(Vec::new()));
        
        let t1 = tracker.clone();
        let sys_a = move || { t1.lock().unwrap().push("A"); };
        
        let t2 = tracker.clone();
        let sys_b = move || { t2.lock().unwrap().push("B"); };
        
        let t3 = tracker.clone();
        let sys_c = move || { t3.lock().unwrap().push("C"); };

        let mut schedule = Schedule::new();
        // Out of order ekle: B, C, A. A→B→C sırası bekleniyor.
        schedule.add_di_system(sys_b.label("B").after("A"));
        schedule.add_di_system(sys_c.label("C").after("B"));
        schedule.add_di_system(sys_a.label("A"));

        schedule.run(&mut world, 0.1);

        let final_order = tracker.lock().unwrap().clone();
        assert_eq!(final_order, vec!["A", "B", "C"]);
    }

    // ──── Duplicate Edge ────

    #[test]
    fn test_duplicate_edge_dedup() {
        let mut world = World::new();
        let tracker = Arc::new(Mutex::new(Vec::new()));

        let t1 = tracker.clone();
        let sys_a = move || { t1.lock().unwrap().push("A"); };

        let t2 = tracker.clone();
        let sys_b = move || { t2.lock().unwrap().push("B"); };

        let mut schedule = Schedule::new();
        // A.before("B") + B.after("A") → aynı kenar iki kez deneniyor
        schedule.add_di_system(sys_a.label("A").before("B"));
        schedule.add_di_system(sys_b.label("B").after("A"));

        schedule.run(&mut world, 0.1);

        let final_order = tracker.lock().unwrap().clone();
        assert_eq!(final_order, vec!["A", "B"]);
    }

    // ──── Cyclic Dependency ────

    #[test]
    #[should_panic(expected = "Cyclic dependency")]
    fn test_cyclic_dependency_panics() {
        let mut world = World::new();
        let sys_a = move || {};
        let sys_b = move || {};

        let mut schedule = Schedule::new();
        schedule.add_di_system(sys_a.label("A").before("B"));
        schedule.add_di_system(sys_b.label("B").before("A"));

        schedule.run(&mut world, 0.1);
    }

    // ──── Validate ────

    #[test]
    #[should_panic(expected = "Cyclic dependency")]
    fn test_validate_catches_cycle_early() {
        let sys_a = move || {};
        let sys_b = move || {};

        let mut schedule = Schedule::new();
        schedule.add_di_system(sys_a.label("X").after("Y"));
        schedule.add_di_system(sys_b.label("Y").after("X"));

        schedule.validate(); // run() olmadan hata yakalanır
    }

    // ──── DI — Resource Injection ────

    #[test]
    fn test_di_resource_injection() {
        let mut world = World::new();
        world.insert_resource(42_u32);
        world.insert_resource(0_i32);

        fn read_sys(val: Res<u32>, mut out: ResMut<i32>) {
            *out = *val as i32;
        }

        let mut schedule = Schedule::new();
        let sys: Box<dyn System> = IntoSystem::<(Res<'static, u32>, ResMut<'static, i32>)>::into_system(read_sys);
        schedule.add_system_boxed(sys);
        schedule.run(&mut world, 0.0);

        assert_eq!(*world.get_resource::<i32>().unwrap(), 42);
    }

    #[test]
    fn test_di_resource_mutation() {
        let mut world = World::new();
        world.insert_resource(10_u32);

        fn inc_sys(mut val: ResMut<u32>) {
            *val += 5;
        }

        let mut schedule = Schedule::new();
        let sys: Box<dyn System> = IntoSystem::<(ResMut<'static, u32>,)>::into_system(inc_sys);
        schedule.add_system_boxed(sys);
        schedule.run(&mut world, 0.0);

        assert_eq!(*world.get_resource::<u32>().unwrap(), 15);
    }

    #[test]
    fn test_di_dt_injection() {
        let mut world = World::new();
        world.insert_resource(0.0_f64);

        fn dt_sys(dt: f32, mut out: ResMut<f64>) {
            *out = dt as f64;
        }

        let mut schedule = Schedule::new();
        let sys: Box<dyn System> = IntoSystem::<(f32, ResMut<'static, f64>)>::into_system(dt_sys);
        schedule.add_system_boxed(sys);
        schedule.run(&mut world, 0.016);

        let val = *world.get_resource::<f64>().unwrap();
        assert!((0.016_f64 - val).abs() < 0.001);
    }

    #[test]
    fn test_di_multi_param() {
        let mut world = World::new();
        world.insert_resource(100_u32);
        world.insert_resource(String::from("hello"));
        world.insert_resource(Vec::<String>::new());

        fn multi_sys(num: Res<u32>, text: Res<String>, dt: f32, mut out: ResMut<Vec<String>>) {
            out.push(format!("{}-{}-{:.2}", *num, *text, dt));
        }

        let mut schedule = Schedule::new();
        let sys: Box<dyn System> = IntoSystem::<(Res<'static, u32>, Res<'static, String>, f32, ResMut<'static, Vec<String>>)>::into_system(multi_sys);
        schedule.add_system_boxed(sys);
        schedule.run(&mut world, 1.5);

        let out = world.get_resource::<Vec<String>>().unwrap();
        assert_eq!(out[0], "100-hello-1.50");
    }

    // ──── add_system (basit fn) ────

    #[test]
    fn test_add_system_basic() {
        let mut world = World::new();
        world.insert_resource(0u32);

        let mut schedule = Schedule::new();
        schedule.add_system(|world: &mut World, _dt: f32| {
            if let Some(mut val) = world.get_resource_mut::<u32>() {
                *val += 1;
            }
        });

        schedule.run(&mut world, 0.0);
        schedule.run(&mut world, 0.0);

        assert_eq!(*world.get_resource::<u32>().unwrap(), 2);
    }

    // ──── Eşleşmeyen label (warning, panic değil) ────

    #[test]
    fn test_unmatched_label_does_not_panic() {
        let mut world = World::new();
        let sys = move || {};

        let mut schedule = Schedule::new();
        schedule.add_di_system(sys.after("nonexistent_label"));
        
        // Panic olmamalı — sadece warning loglanır
        schedule.run(&mut world, 0.1);
    }
}

