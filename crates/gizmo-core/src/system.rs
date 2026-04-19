use crate::world::World;

pub trait System {
    fn run(&mut self, world: &mut World, dt: f32);
}

// Orijinal System Tanımı (Basit fonksiyonlar için)
impl<F> System for F
where
    F: FnMut(&mut World, f32),
{
    fn run(&mut self, world: &mut World, dt: f32) {
        (self)(world, dt);
    }
}

// ==============================================================
// DEPENDENCY INJECTION SİSTEMİ (BEVY TARZI)
// ==============================================================

use std::cell::{Ref, RefMut};

/// Bir fonksiyonun sistem parametresi olarak alabileceği argümanları tanımlar.
pub trait SystemParam {
    type Item<'w>;
    fn fetch<'w>(world: &'w World, dt: f32) -> Option<Self::Item<'w>>;
}

/// Salt okunur (Immutable) Resource enjeksiyonu
pub struct Res<'w, T: 'static> {
    pub value: Ref<'w, T>,
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

/// Yazılabilir (Mutable) Resource enjeksiyonu
pub struct ResMut<'w, T: 'static> {
    pub value: RefMut<'w, T>,
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

// ==============================================================
// INTO SYSTEM (FONKSİYONLARDAN SİSTEME DÖNÜŞÜM)
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

// dt enjeksiyonu için
impl SystemParam for f32 {
    type Item<'w> = f32;
    fn fetch<'w>(_world: &'w World, dt: f32) -> Option<Self::Item<'w>> {
        Some(dt)
    }
}

// 1 Parametre
impl<F, P1> IntoSystem<(P1,)> for F
where
    F: FnMut(P1::Item<'_>) + 'static,
    P1: SystemParam + 'static,
{
    fn into_system(mut self) -> Box<dyn System> {
        Box::new(move |world: &mut World, dt: f32| {
            if let Some(p1) = P1::fetch(world, dt) {
                (self)(p1);
            }
        })
    }
}

// 2 Parametre
impl<F, P1, P2> IntoSystem<(P1, P2)> for F
where
    F: FnMut(P1::Item<'_>, P2::Item<'_>) + 'static,
    P1: SystemParam + 'static,
    P2: SystemParam + 'static,
{
    fn into_system(mut self) -> Box<dyn System> {
        Box::new(move |world: &mut World, dt: f32| {
            if let (Some(p1), Some(p2)) = (P1::fetch(world, dt), P2::fetch(world, dt)) {
                (self)(p1, p2);
            }
        })
    }
}

// 3 Parametre
impl<F, P1, P2, P3> IntoSystem<(P1, P2, P3)> for F
where
    F: FnMut(P1::Item<'_>, P2::Item<'_>, P3::Item<'_>) + 'static,
    P1: SystemParam + 'static,
    P2: SystemParam + 'static,
    P3: SystemParam + 'static,
{
    fn into_system(mut self) -> Box<dyn System> {
        Box::new(move |world: &mut World, dt: f32| {
            if let (Some(p1), Some(p2), Some(p3)) = (
                P1::fetch(world, dt),
                P2::fetch(world, dt),
                P3::fetch(world, dt),
            ) {
                (self)(p1, p2, p3);
            }
        })
    }
}

// 4 Parametre
impl<F, P1, P2, P3, P4> IntoSystem<(P1, P2, P3, P4)> for F
where
    F: FnMut(P1::Item<'_>, P2::Item<'_>, P3::Item<'_>, P4::Item<'_>) + 'static,
    P1: SystemParam + 'static,
    P2: SystemParam + 'static,
    P3: SystemParam + 'static,
    P4: SystemParam + 'static,
{
    fn into_system(mut self) -> Box<dyn System> {
        Box::new(move |world: &mut World, dt: f32| {
            if let (Some(p1), Some(p2), Some(p3), Some(p4)) = (
                P1::fetch(world, dt),
                P2::fetch(world, dt),
                P3::fetch(world, dt),
                P4::fetch(world, dt),
            ) {
                (self)(p1, p2, p3, p4);
            }
        })
    }
}

// 5 Parametre
impl<F, P1, P2, P3, P4, P5> IntoSystem<(P1, P2, P3, P4, P5)> for F
where
    F: FnMut(P1::Item<'_>, P2::Item<'_>, P3::Item<'_>, P4::Item<'_>, P5::Item<'_>) + 'static,
    P1: SystemParam + 'static,
    P2: SystemParam + 'static,
    P3: SystemParam + 'static,
    P4: SystemParam + 'static,
    P5: SystemParam + 'static,
{
    fn into_system(mut self) -> Box<dyn System> {
        Box::new(move |world: &mut World, dt: f32| {
            if let (Some(p1), Some(p2), Some(p3), Some(p4), Some(p5)) = (
                P1::fetch(world, dt),
                P2::fetch(world, dt),
                P3::fetch(world, dt),
                P4::fetch(world, dt),
                P5::fetch(world, dt),
            ) {
                (self)(p1, p2, p3, p4, p5);
            }
        })
    }
}

// 6 Parametre
impl<F, P1, P2, P3, P4, P5, P6> IntoSystem<(P1, P2, P3, P4, P5, P6)> for F
where
    F: FnMut(P1::Item<'_>, P2::Item<'_>, P3::Item<'_>, P4::Item<'_>, P5::Item<'_>, P6::Item<'_>)
        + 'static,
    P1: SystemParam + 'static,
    P2: SystemParam + 'static,
    P3: SystemParam + 'static,
    P4: SystemParam + 'static,
    P5: SystemParam + 'static,
    P6: SystemParam + 'static,
{
    fn into_system(mut self) -> Box<dyn System> {
        Box::new(move |world: &mut World, dt: f32| {
            if let (Some(p1), Some(p2), Some(p3), Some(p4), Some(p5), Some(p6)) = (
                P1::fetch(world, dt),
                P2::fetch(world, dt),
                P3::fetch(world, dt),
                P4::fetch(world, dt),
                P5::fetch(world, dt),
                P6::fetch(world, dt),
            ) {
                (self)(p1, p2, p3, p4, p5, p6);
            }
        })
    }
}



// ==============================================================
// DEPENDENCY GRAPH (EXECUTION ORDER)
// ==============================================================

pub struct SystemConfig {
    pub system: Box<dyn System>,
    pub labels: Vec<&'static str>,
    pub before: Vec<&'static str>,
    pub after: Vec<&'static str>,
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

impl IntoSystemConfig<()> for SystemConfig {
    fn into_config(self) -> SystemConfig {
        self
    }
}

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

    pub fn add_di_system<Params, S: IntoSystemConfig<Params>>(&mut self, system: S) {
        self.unbuilt_configs.push(system.into_config());
        self.is_built = false;
    }

    pub fn add_system<S: System + 'static>(&mut self, system: S) {
        self.unbuilt_configs.push(SystemConfig::new(Box::new(system)));
        self.is_built = false;
    }

    pub fn build(&mut self) {
        if self.is_built {
            return;
        }
        
        let configs = std::mem::take(&mut self.unbuilt_configs);
        let count = configs.len();
        
        // Adjacency list: edges[A] = [B, C] indicates A must run BEFORE B and C.
        let mut adj = vec![Vec::new(); count];
        let mut in_degree = vec![0; count];
        
        // Resolve relations
        for i in 0..count {
            // "before": config[i] runs before config[j]
            for before_label in &configs[i].before {
                for j in 0..count {
                    if i != j && configs[j].labels.contains(before_label) {
                        adj[i].push(j);
                        in_degree[j] += 1;
                    }
                }
            }
            
            // "after": config[i] runs after config[j]
            for after_label in &configs[i].after {
                for j in 0..count {
                    if i != j && configs[j].labels.contains(after_label) {
                        adj[j].push(i);
                        in_degree[i] += 1;
                    }
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
            panic!("Cyclic dependency detected in System execution graph!");
        }
        
        // We must extract systems by taking ownership.
        // We temporarily replace configs with a dummy to grab the Box<dyn System>
        let mut final_systems = Vec::with_capacity(count);
        let mut dummy_configs: Vec<Option<SystemConfig>> = configs.into_iter().map(Some).collect();
        
        for &idx in &sorted_indices {
            let config = dummy_configs[idx].take().unwrap();
            final_systems.push(config.system);
        }
        
        self.systems = final_systems;
        self.is_built = true;
    }

    pub fn run(&mut self, world: &mut World, dt: f32) {
        if !self.is_built {
            self.build();
        }
        for system in &mut self.systems {
            system.run(world, dt);
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
        // Insert out of order: B, C, A. But specify A before B, B before C.
        schedule.add_di_system(sys_b.label("B").after("A"));
        schedule.add_di_system(sys_c.label("C").after("B"));
        schedule.add_di_system(sys_a.label("A"));

        schedule.run(&mut world, 0.1);

        let final_order = tracker.lock().unwrap().clone();
        assert_eq!(final_order, vec!["A", "B", "C"]);
    }
}
