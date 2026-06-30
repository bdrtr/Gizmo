use super::*;
use crate::world::World;
use std::any::TypeId;

// ==============================================================
// DEPENDENCY INJECTION SİSTEMİ
// ==============================================================

use crate::world::{ResourceReadGuard, ResourceWriteGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SystemParamFetchError {
    Resource(crate::world::ResourceFetchError),
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
    type Item<'w>;
    fn fetch<'w>(world: &'w World, dt: f32) -> Result<Self::Item<'w>, SystemParamFetchError>;
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
        if let Some(query) = world.query::<Q>() {
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

