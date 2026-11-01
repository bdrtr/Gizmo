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

/// What a system function may return: `()`, or `Result<(), E>` for any `E: Display` — so a
/// system can use `?`.
///
/// An `Err` does not stop the frame or the system; it is handed to the world's
/// [`SystemErrorHandler`] (by default a `tracing::error!` naming the system), and the system runs
/// again next frame. A panic is still a panic.
pub trait SystemOutput {
    /// Deals with this run's outcome; `system` is the system's type name.
    fn handle(self, world: &World, system: &'static str);
}

impl SystemOutput for () {
    fn handle(self, _world: &World, _system: &'static str) {}
}

impl<E: std::fmt::Display> SystemOutput for Result<(), E> {
    fn handle(self, world: &World, system: &'static str) {
        if let Err(error) = self {
            let handler = world.get_resource::<SystemErrorHandler>().map(|h| h.0).unwrap_or(log_system_error);
            handler(system, &error);
        }
    }
}

/// Where a system's `Err` goes. Insert one as a resource to replace the default — which logs it
/// with `tracing::error!` — with, say, a counter, a panic in tests, or an in-game console line.
///
/// A plain `fn` rather than a closure: it is read, unsynchronised, by systems running in
/// parallel, and a function pointer has no state to race on.
#[derive(Clone, Copy)]
pub struct SystemErrorHandler(pub fn(system: &str, error: &dyn std::fmt::Display));

impl Default for SystemErrorHandler {
    fn default() -> Self {
        Self(log_system_error)
    }
}

/// The default [`SystemErrorHandler`]: one `tracing::error!` line with the system and the error.
pub fn log_system_error(system: &str, error: &dyn std::fmt::Display) {
    tracing::error!(system, %error, "[ECS] system returned an error");
}

// 0 Parametre
impl<F, Out> IntoSystem<()> for F
where
    F: FnMut() -> Out + Send + Sync + 'static,
    Out: SystemOutput + 'static,
{
    fn into_system(self) -> Box<dyn System> {
        struct ZeroParamSystem<F, Out>(F, std::marker::PhantomData<fn() -> Out>);
        impl<F: FnMut() -> Out + Send + Sync + 'static, Out: SystemOutput + 'static> System for ZeroParamSystem<F, Out> {
            fn run(&mut self, world: &World, _dt: f32) {
                (self.0)().handle(world, std::any::type_name::<F>());
            }
            fn access_info(&self) -> AccessInfo {
                AccessInfo::new()
            }
        }
        Box::new(ZeroParamSystem(self, std::marker::PhantomData))
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
    ($($P:ident $s:ident),+) => {
        #[allow(non_snake_case)]
        impl<F, Out, $($P),+> IntoSystem<($($P,)+)> for F
        where
            F: FnMut($($P::Item<'_>),+) -> Out + FnMut($($P),+) -> Out + Send + Sync + 'static,
            Out: SystemOutput + 'static,
            $($P: SystemParam + 'static,)+
        {
            fn into_system(self) -> Box<dyn System> {
                struct MultiParamSystem<F, Out, $($P: SystemParam),+> {
                    func: F,
                    /// The parameters' own state, one tuple per BUILT system — which is what makes
                    /// `Local<T>` per-system rather than per-type. Created with `Default` here,
                    /// never touched again except by the parameters themselves.
                    state: ($($P::State,)+),
                    _marker: std::marker::PhantomData<fn() -> ($($P,)+ Out)>,
                }

                impl<F, Out, $($P),+> System for MultiParamSystem<F, Out, $($P),+>
                where
                    F: FnMut($($P::Item<'_>),+) -> Out + FnMut($($P),+) -> Out + Send + Sync + 'static,
                    Out: SystemOutput + 'static,
                    $($P: SystemParam + 'static,)+
                {
                    fn run(&mut self, world: &World, dt: f32) {
                        // Split the borrow: the fetched parameters hold `&mut` into `state` for the
                        // whole body, and `func` needs `&mut` to be called. Destructuring `self` is
                        // what lets both be live at once.
                        let Self { func, state, .. } = self;
                        let ($($s,)+) = state;
                        $(
                            let $P = match $P::fetch(world, dt, $s) {
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
                        (func)($($P),+).handle(world, std::any::type_name::<F>());
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
                Box::new(MultiParamSystem::<F, Out, $($P),+> {
                    func: self,
                    state: Default::default(),
                    _marker: std::marker::PhantomData,
                })
            }
        }
    };
}

impl_into_system!(P1 s1);
impl_into_system!(P1 s1, P2 s2);
impl_into_system!(P1 s1, P2 s2, P3 s3);
impl_into_system!(P1 s1, P2 s2, P3 s3, P4 s4);
impl_into_system!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5);
impl_into_system!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5, P6 s6);
impl_into_system!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5, P6 s6, P7 s7);
impl_into_system!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5, P6 s6, P7 s7, P8 s8);
impl_into_system!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5, P6 s6, P7 s7, P8 s8, P9 s9);
impl_into_system!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5, P6 s6, P7 s7, P8 s8, P9 s9, P10 s10);
impl_into_system!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5, P6 s6, P7 s7, P8 s8, P9 s9, P10 s10, P11 s11);
impl_into_system!(P1 s1, P2 s2, P3 s3, P4 s4, P5 s5, P6 s6, P7 s7, P8 s8, P9 s9, P10 s10, P11 s11, P12 s12);

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

