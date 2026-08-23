use super::*;
use crate::world::World;

// ==============================================================
// INTO SYSTEM — FONKSİYONLARDAN SİSTEME DÖNÜŞÜM (MAKRO İLE)
// ==============================================================

/// Converts a value (typically a plain function whose arguments implement
/// [`SystemParam`]) into a boxed [`System`] that the scheduler can run.
pub trait IntoSystem<Params> {
    /// Boxes `self` into a runnable system.
    ///
    /// Implemented for `FnMut()` and for functions or closures of 1 to 12 parameters, every
    /// one of which must be a [`SystemParam`], plus an identity impl for an already-boxed
    /// `Box<dyn System>`. `Params` is inferred from the signature and only serves to keep
    /// those impls apart.
    ///
    /// Conversion is where the parameter list becomes an [`AccessInfo`]; nothing is fetched
    /// from the world yet — that happens on every run. The generated runner fetches the
    /// parameters in declaration order and **panics** if any of them cannot be produced
    /// (typically a resource that was never inserted); it deliberately does not skip the
    /// system quietly.
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

/// Two systems fused into one scheduling unit, produced by [`SystemExt::pipe`].
///
/// Despite the name, nothing is piped *between* them: both halves are ordinary systems, run
/// one after the other with the same `&World` and the same `dt`, and no value is handed from
/// the first to the second — they communicate only through the world. Because the pair is a
/// single system, deferred `Commands` are not applied in between (that happens between
/// batches), so the second half does not see entities the first half spawned.
///
/// Access is the union of both halves and exclusive if either half is. Fields are private:
/// build it with [`SystemExt::pipe`].
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

/// Chains one system onto another.
pub trait SystemExt<ParamA> {
    /// Fuses `self` and `other` into a single system that runs `self` first, then `other` —
    /// see [`PipeSystem`] for what that implies (same world and `dt`, no value handed over,
    /// no command flush in between).
    ///
    /// This orders two systems without going through labels: the pair is one node in the
    /// batching graph, so the halves can never be separated or run concurrently with each
    /// other. Other systems may still run in parallel *alongside* the pair.
    ///
    /// The two halves need not be compatible with *each other*: they run sequentially inside
    /// one system, so fusing a writer of `T` with a reader of `T` is sound where registering
    /// them separately would have forced them into different batches. The fused access is the
    /// union of both, so the pair keeps every conflict each half had on its own.
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

/// Generates the IntoSystem implementations for 1-8 parameters.
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

                // Turbofish şart: `MultiParamSystem` bu fonksiyonun İÇİNDE tanımlı, yani tür
                // parametreleri dıştaki `impl`inkilerle bağlı değil — çıkarımla bulunmaları
                // gerekiyor. `F`'nin iki `FnMut` sınırı olduğu için (biri `$P::Item<'_>`, öteki
                // `$P`) aday birden fazla ve nightly bunu belirsiz sayıyor: E0282/E0283, ve
                // `gizmo-core` nightly'de hiç derlenmiyordu — Miri işi de bu yüzden düşüyordu.
                // Türleri açıkça yazmak belirsizliği kaldırıyor; sınırlar değişmiyor.
                Box::new(MultiParamSystem::<F, $($P),+> {
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

