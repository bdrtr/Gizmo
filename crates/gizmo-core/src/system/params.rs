use super::*;
use crate::world::World;
use std::any::TypeId;

// ==============================================================
// DEPENDENCY INJECTION SİSTEMİ
// ==============================================================

use crate::world::{ResourceReadGuard, ResourceWriteGuard};

/// Why a [`SystemParam`] could not be produced for a run.
///
/// Systems built through [`IntoSystem`] never hand this to user code: the generated runner
/// panics with a diagnostic instead, and the typed [`IntoCondition`] path unwraps it. A
/// parameter that cannot be fetched is treated as a setup mistake, not a recoverable state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SystemParamFetchError {
    /// A [`Res`]/[`ResMut`] — or a parameter built on top of one, such as `Commands` — could
    /// not be borrowed.
    ///
    /// Either no value of that type was inserted into the world, or its lock was taken in an
    /// incompatible way: a live `ResMut<T>` blocks every other borrow of `T`, including a
    /// second `ResMut<T>` in the *same* parameter list, and a lock poisoned by an earlier
    /// panic reports the same way. The borrow is attempted, never waited on, so a
    /// self-conflict surfaces as this error rather than as a deadlock.
    Resource(crate::world::ResourceFetchError),
    /// Query construction returned nothing.
    ///
    /// Unreachable through the built-in `Query` parameter: that constructor never returns
    /// `None`. It is not *infallible*, though — asking for the same component mutably twice
    /// in one query (`Query<(Mut<T>, Mut<T>)>`) panics inside the aliasing check rather than
    /// surfacing here. The variant exists as the failure channel should construction start
    /// reporting failures instead.
    QueryError,
}

impl From<crate::world::ResourceFetchError> for SystemParamFetchError {
    fn from(value: crate::world::ResourceFetchError) -> Self {
        Self::Resource(value)
    }
}

impl std::fmt::Display for SystemParamFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SystemParamFetchError::Resource(e) => {
                write!(f, "system parameter resource fetch failed: {e}")
            }
            SystemParamFetchError::QueryError => {
                write!(f, "system parameter query construction failed")
            }
        }
    }
}

impl std::error::Error for SystemParamFetchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SystemParamFetchError::Resource(e) => Some(e),
            SystemParamFetchError::QueryError => None,
        }
    }
}

// SystemParam tamamen içsel bir DI trait'idir; yanlış bir impl scheduler'ın
// aliasing garantilerini bozar. Tüm impl'ler bu crate içindedir (Res/ResMut/
// f32/Query) ve cross-crate impl yoktur, bu yüzden sealed yapılır.
// `pub(crate)` çünkü SystemParam'ı implemente eden tipler bu crate'in başka
// modüllerinde de var (EventReader/EventWriter @ event.rs, Commands @ commands.rs);
// Sealed'a yalnızca crate içinden erişilebilir, dolayısıyla dış crate'ler hâlâ
// SystemParam impl edemez.
pub(crate) mod sealed {
    pub trait Sealed {}
}

/// A value that a system can request as a parameter (e.g. [`Query`](crate::Query),
/// [`Res`], [`ResMut`]).
///
/// Implementors describe how to fetch their value from the [`World`] and which
/// component/resource accesses they require, allowing the scheduler to run
/// non-conflicting systems in parallel.
pub trait SystemParam: sealed::Sealed {
    /// The value actually handed to the system body, borrowed from the world for `'w`.
    ///
    /// The implementing type is the `'static` spelling of the parameter (`Res<'static, T>`,
    /// `Query<'static, Q>`) and `Item<'w>` is the same type re-borrowed for one run; a system
    /// signature writes plain `Res<Foo>` and lets elision supply `'w`.
    type Item<'w>;

    /// Produces the parameter for a single run.
    ///
    /// Called once per parameter, in declaration order, immediately before the system body,
    /// and whatever it returns (a lock guard, for the resource parameters) stays alive until
    /// that body returns. `dt` is the delta time the schedule was stepped with, passed
    /// straight through — the `f32` parameter is exactly this value, and run conditions are
    /// always evaluated with `dt = 0.0`.
    ///
    /// Fails rather than blocks; see [`SystemParamFetchError`] for what the callers do with
    /// the error (they panic).
    fn fetch<'w>(world: &'w World, dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError>;

    /// Appends this parameter's accesses to `info`.
    ///
    /// Append-only: it must not clear or deduplicate what is already there, since all the
    /// parameters of one system accumulate into a single [`AccessInfo`]. Called while the
    /// schedule is built, not per frame.
    ///
    /// The declaration has to cover everything the fetched value can touch, including memory
    /// only touched incidentally — `Changed<T>`/`Added<T>` declare a *read* of `T` because
    /// they inspect the change ticks a `Mut<T>` writer stamps. Whatever is omitted here may
    /// be co-scheduled with a conflicting writer and raced on.
    fn get_access_info(info: &mut AccessInfo);
}

/// Shared access to a world resource, requested as a system parameter
/// (`fn sys(cfg: Res<Config>)`).
///
/// Holds a read lock on that resource's slot for as long as the value lives, i.e. for the
/// rest of the system body. Any number of `Res<T>` can be live at once; a concurrently live
/// [`ResMut`] of the same type makes the borrow fail, and so does a `T` that was never
/// inserted into the world. Those are not the only failures — see
/// [`SystemParamFetchError::Resource`] for the rest. Every one of them panics the system
/// rather than skipping it.
///
/// Read-only by construction: there is no `DerefMut`.
pub struct Res<'w, T: 'static> {
    value: ResourceReadGuard<'w, T>,
}

impl<'w, T: 'static> std::ops::Deref for Res<'w, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: 'static> sealed::Sealed for Res<'static, T> {}
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

/// Exclusive access to a world resource, requested as a system parameter
/// (`fn sys(mut score: ResMut<Score>)`).
///
/// Holds a write lock on that resource's slot until the system body returns. While it is
/// alive no other borrow of `T` succeeds — not a second `ResMut<T>` in the same parameter
/// list, and not a `world.get_resource::<T>()` call made from inside this very system, which
/// comes back empty instead of blocking.
///
/// Mutation through this guard is invisible to change detection: resources carry no change
/// ticks, so nothing observes a resource write the way `Changed<T>` observes a component
/// write.
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

impl<T: 'static> sealed::Sealed for ResMut<'static, T> {}
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

impl sealed::Sealed for f32 {}
impl SystemParam for f32 {
    type Item<'w> = f32;
    fn fetch<'w>(_world: &'w World, dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError> {
        Ok(dt)
    }
    fn get_access_info(_info: &mut AccessInfo) {}
}

impl<Q: crate::query::WorldQuery + 'static> sealed::Sealed for crate::query::Query<'static, Q> {}
impl<Q: crate::query::WorldQuery + 'static> SystemParam for crate::query::Query<'static, Q> {
    type Item<'w> = crate::query::Query<'w, Q>;
    fn fetch<'w>(world: &'w World, _dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError> {
        // SAFETY: the scheduler validates that co-batched systems have disjoint component
        // access (`AccessInfo`/`is_compatible_with`) before running them in parallel, and
        // runs `is_exclusive` systems alone. So while this system's `Query` is live, no
        // other query mutably aliases the same components. This is the documented contract
        // of `query_unchecked` — the safe `query`/`query_mut` split can't express it because
        // a system only ever holds a shared `&World`.
        if let Some(query) = unsafe { world.query_unchecked::<Q>() } {
            Ok(query)
        } else {
            Err(SystemParamFetchError::QueryError)
        }
    }
    fn get_access_info(info: &mut AccessInfo) {
        let mut types = Vec::new();
        Q::check_aliasing(&mut types);
        for (tid, is_mut) in types {
            if is_mut {
                info.component_writes.push(tid);
            } else {
                info.component_reads.push(tid);
            }
        }
    }
}

