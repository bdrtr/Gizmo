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

pub struct Schedule {
    systems: Vec<Box<dyn System>>,
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            systems: Vec::new(),
        }
    }

    pub fn add_di_system<Params, S: IntoSystem<Params>>(&mut self, system: S) {
        self.systems.push(system.into_system());
    }

    pub fn add_system<S: System + 'static>(&mut self, system: S) {
        self.systems.push(Box::new(system));
    }

    pub fn run(&mut self, world: &mut World, dt: f32) {
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
