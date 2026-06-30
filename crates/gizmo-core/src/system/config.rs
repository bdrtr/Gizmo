use super::*;
use crate::world::World;
use std::any::TypeId;

// ==============================================================
// SYSTEM CONFIG — LABEL / BEFORE / AFTER / READS / WRITES
// ==============================================================

pub trait SystemSet: 'static {
    fn set_name() -> &'static str {
        std::any::type_name::<Self>()
    }
}

pub struct SystemConfig {
    pub(crate) system: Box<dyn System>,
    pub(crate) labels: Vec<&'static str>,
    pub(crate) before: Vec<&'static str>,
    pub(crate) after: Vec<&'static str>,
    pub(crate) in_sets: Vec<&'static str>,
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
            in_sets: Vec::new(),
            added_info: AccessInfo::new(),
            phase: Phase::default(),
        }
    }

    pub fn in_set<S: SystemSet>(mut self) -> Self {
        self.in_sets.push(S::set_name());
        self.labels.push(S::set_name());
        self
    }

    pub fn label(mut self, label: &'static str) -> Self {
        self.labels.push(label);
        self
    }

    pub fn before(mut self, target: &'static str) -> Self {
        self.before.push(target);
        self
    }

    pub fn before_set<S: SystemSet>(mut self) -> Self {
        self.before.push(S::set_name());
        self
    }

    pub fn after(mut self, target: &'static str) -> Self {
        self.after.push(target);
        self
    }

    pub fn after_set<S: SystemSet>(mut self) -> Self {
        self.after.push(S::set_name());
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

/// Turns a system into a [`SystemConfig`], the builder used to attach ordering
/// constraints (labels, `before`/`after`, system sets) and a [`Phase`] before
/// adding it to a [`Schedule`].
pub trait IntoSystemConfig<Params> {
    fn into_config(self) -> SystemConfig;

    fn label(self, l: &'static str) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().label(l)
    }
    
    fn in_set<S: SystemSet>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().in_set::<S>()
    }
    
    fn before(self, target: &'static str) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().before(target)
    }
    
    fn before_set<S: SystemSet>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().before_set::<S>()
    }
    
    fn after(self, target: &'static str) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().after(target)
    }

    fn after_set<S: SystemSet>(self) -> SystemConfig
    where
        Self: Sized,
    {
        self.into_config().after_set::<S>()
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


pub trait IntoSystemConfigs<T> {
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

