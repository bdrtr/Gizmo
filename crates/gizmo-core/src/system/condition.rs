use super::*;
use crate::world::World;

// ==============================================================
// RUN CONDITIONS
// ==============================================================

pub trait IntoCondition<Params> {
    fn into_condition(self) -> Box<dyn FnMut(&World) -> bool + Send + Sync>;
    /// World access performed by the condition's own SystemParams. The scheduler must
    /// include this in its disjointness check: a `run_if(|r: Res<Score>| ..)` condition
    /// reads `Score` at run time, so it must conflict with a `ResMut<Score>` writer (and a
    /// `Query<&Pos>` condition with a `Query<Mut<Pos>>` writer) — otherwise they co-batch
    /// and run in parallel over a shared `&World`, racing on the resource/component.
    fn condition_access() -> AccessInfo;
}

impl<F> IntoCondition<()> for F
where
    F: FnMut() -> bool + Send + Sync + 'static,
{
    fn into_condition(mut self) -> Box<dyn FnMut(&World) -> bool + Send + Sync> {
        Box::new(move |_world| self())
    }
    fn condition_access() -> AccessInfo {
        // A parameter-less condition touches no world state.
        AccessInfo::new()
    }
}

macro_rules! impl_into_condition {
    ($($P:ident),+) => {
        #[allow(non_snake_case)]
        impl<F, $($P),+> IntoCondition<($($P,)+)> for F
        where
            F: FnMut($($P::Item<'_>),+) -> bool + Send + Sync + 'static,
            $($P: SystemParam + 'static,)+
        {
            fn into_condition(mut self) -> Box<dyn FnMut(&World) -> bool + Send + Sync> {
                Box::new(move |world| {
                    $(let $P = $P::fetch(world, 0.0).unwrap();)+
                    (self)($($P),+)
                })
            }
            fn condition_access() -> AccessInfo {
                let mut info = AccessInfo::new();
                $($P::get_access_info(&mut info);)+
                info
            }
        }
    };
}

impl_into_condition!(P1);
impl_into_condition!(P1, P2);
impl_into_condition!(P1, P2, P3);
impl_into_condition!(P1, P2, P3, P4);
impl_into_condition!(P1, P2, P3, P4, P5);
impl_into_condition!(P1, P2, P3, P4, P5, P6);

pub trait SystemExtRunIf {
    fn run_if_sys<ParamC, Cond: IntoCondition<ParamC>>(self, cond: Cond) -> Box<dyn System>;
}

impl SystemExtRunIf for Box<dyn System> {
    fn run_if_sys<ParamC, Cond: IntoCondition<ParamC>>(self, cond: Cond) -> Box<dyn System> {
        let condition_access = Cond::condition_access();
        Box::new(ConditionalSystem {
            inner: self,
            condition: cond.into_condition(),
            condition_access,
        })
    }
}

pub struct ConditionalSystem {
    pub(crate) inner: Box<dyn System>,
    pub(crate) condition: Box<dyn FnMut(&World) -> bool + Send + Sync>,
    /// Access the run-condition performs (see [`IntoCondition::condition_access`]).
    pub(crate) condition_access: AccessInfo,
}

impl System for ConditionalSystem {
    fn run(&mut self, world: &World, dt: f32) {
        if (self.condition)(world) {
            self.inner.run(world, dt);
        }
    }
    fn access_info(&self) -> AccessInfo {
        // Union the inner system's access with the CONDITION's access, so the batcher
        // never co-schedules a condition that reads state with a system that writes it.
        let mut info = self.inner.access_info();
        let c = &self.condition_access;
        info.component_reads.extend(c.component_reads.iter().copied());
        info.component_writes.extend(c.component_writes.iter().copied());
        info.resource_reads.extend(c.resource_reads.iter().copied());
        info.resource_writes.extend(c.resource_writes.iter().copied());
        info.is_exclusive |= c.is_exclusive;
        info
    }
}

pub trait DistributiveRunIfExt<Params> {
    fn distributive_run_if<ParamC, Cond: IntoCondition<ParamC> + Clone + Send + Sync + 'static>(self, cond: Cond) -> Box<dyn System>;
}

macro_rules! impl_distributive_run_if {
    ($($P:ident $S:ident $idx:tt),+) => {
        impl<$($P, $S),+> DistributiveRunIfExt<($($P,)+)> for ($($S,)+)
        where
            $($S: IntoSystem<$P> + 'static,)+
        {
            fn distributive_run_if<ParamC, Cond: IntoCondition<ParamC> + Clone + Send + Sync + 'static>(self, cond: Cond) -> Box<dyn System> {
                let systems: Vec<Box<dyn System>> = vec![
                    $(self.$idx.into_system().run_if_sys(cond.clone()),)+
                ];

                struct MacroSystem {
                    systems: Vec<Box<dyn System>>,
                }
                impl System for MacroSystem {
                    fn run(&mut self, world: &World, dt: f32) {
                        for s in &mut self.systems {
                            s.run(world, dt);
                        }
                    }
                    fn access_info(&self) -> AccessInfo {
                        let mut info = AccessInfo::new();
                        for s in &self.systems {
                            let s_info = s.access_info();
                            info.component_reads.extend(s_info.component_reads);
                            info.component_writes.extend(s_info.component_writes);
                            info.resource_reads.extend(s_info.resource_reads);
                            info.resource_writes.extend(s_info.resource_writes);
                        }
                        info
                    }
                }

                Box::new(MacroSystem { systems })
            }
        }
    };
}

impl_distributive_run_if!(P1 S1 0);
impl_distributive_run_if!(P1 S1 0, P2 S2 1);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3, P5 S5 4);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3, P5 S5 4, P6 S6 5);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3, P5 S5 4, P6 S6 5, P7 S7 6);
impl_distributive_run_if!(P1 S1 0, P2 S2 1, P3 S3 2, P4 S4 3, P5 S5 4, P6 S6 5, P7 S7 6, P8 S8 7);

