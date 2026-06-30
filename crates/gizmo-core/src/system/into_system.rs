use super::*;
use crate::world::World;

// ==============================================================
// INTO SYSTEM — FONKSİYONLARDAN SİSTEME DÖNÜŞÜM (MAKRO İLE)
// ==============================================================

/// Converts a value (typically a plain function whose arguments implement
/// [`SystemParam`]) into a boxed [`System`] that the scheduler can run.
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

// Box<dyn System> dönüşümü
impl IntoSystem<()> for Box<dyn System> {
    fn into_system(self) -> Box<dyn System> {
        self
    }
}

impl System for Box<dyn System> {
    fn run(&mut self, world: &World, dt: f32) {
        (**self).run(world, dt);
    }
    fn access_info(&self) -> AccessInfo {
        (**self).access_info()
    }
}

pub struct PipeSystem {
    a: Box<dyn System>,
    b: Box<dyn System>,
}

impl System for PipeSystem {
    fn run(&mut self, world: &World, dt: f32) {
        self.a.run(world, dt);
        self.b.run(world, dt);
    }

    fn access_info(&self) -> AccessInfo {
        let mut info = self.a.access_info();
        let mut b_info = self.b.access_info();
        info.component_reads.append(&mut b_info.component_reads);
        info.component_writes.append(&mut b_info.component_writes);
        info.resource_reads.append(&mut b_info.resource_reads);
        info.resource_writes.append(&mut b_info.resource_writes);
        info.is_exclusive = info.is_exclusive || b_info.is_exclusive;
        info
    }
}

pub trait SystemExt<ParamA> {
    fn pipe<ParamB, SystemB: IntoSystem<ParamB>>(self, other: SystemB) -> Box<dyn System>;
}

impl<ParamA, SystemA: IntoSystem<ParamA>> SystemExt<ParamA> for SystemA {
    fn pipe<ParamB, SystemB: IntoSystem<ParamB>>(self, other: SystemB) -> Box<dyn System> {
        Box::new(PipeSystem {
            a: self.into_system(),
            b: other.into_system(),
        })
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
                                    panic!(
                                        "❌ FATAL ECS ERROR ❌\n\nSistem parametresi '{param_type}' Dünya'da (World) bulunamadı!\n\nHata Detayı: {e:?}\n\nÇözüm: `app.world.insert_resource()` veya `app.add_plugin()` kullanarak eksik kaynağı başlangıçta Dünya'ya eklediğinizden emin olun. Gizmo Engine, hataların sessizce yok sayılmasını önlemek için sistemi durdurdu.\n",
                                        param_type = std::any::type_name::<$P>(),
                                        e = e
                                    );
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

