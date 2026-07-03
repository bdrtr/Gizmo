use std::any::Any;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageType {
    Table,
    SparseSet,
}

pub trait Component: 'static + Any + Send + Sync + Clone {
    fn storage_type() -> StorageType {
        StorageType::Table
    }
}

#[macro_export]
macro_rules! impl_component {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::Component for $t {}
        )+
    };
    ($($t:ty),+ ; $storage:expr) => {
        $(
            impl $crate::Component for $t {
                fn storage_type() -> $crate::component::StorageType {
                    $storage
                }
            }
        )+
    };
}

// --- Hiyerarşi (Scene Graph) Bileşenleri ---
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Parent(pub u32);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Children(pub Vec<u32>);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);

impl EntityName {
    pub fn new(name: &str) -> Self {
        Self(name.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IsHidden;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IsDeleted;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PrefabRequest(pub String);

impl PrefabRequest {
    pub fn new(name: &str) -> Self {
        Self(name.to_string())
    }
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EntityName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeshSource(pub String);

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MaterialSource {
    pub albedo: [f32; 4],
    pub roughness: f32,
    pub metallic: f32,
    pub unlit: f32,
    pub texture_source: Option<String>,
}

impl_component!(Parent, Children, EntityName, IsHidden, PrefabRequest, IsDeleted, MeshSource, MaterialSource);

// ============================================================
//  Bundle Trait
// ============================================================

pub trait Bundle {
    fn get_infos() -> Vec<crate::archetype::ComponentInfo>;
    /// # Safety
    /// `arch`, `Self::get_infos()`'un döndürdüğü bileşen sütunlarını içermeli ve `_row`
    /// bu arketipte ayrılmış geçerli bir satır olmalıdır. Veriler ham olarak kopyalanır;
    /// sahiplik arketipe devredilir.
    unsafe fn write_to_archetype(self, arch: &mut crate::archetype::Archetype, _row: usize, tick: u32);
    fn apply(self, _world: &mut crate::world::World, _entity: crate::entity::Entity) where Self: Sized {}
}

pub struct DynamicBundle<B: Bundle, C: Component> {
    pub bundle: B,
    pub component: C,
}

impl<B: Bundle, C: Component> Bundle for DynamicBundle<B, C> {
    fn get_infos() -> Vec<crate::archetype::ComponentInfo> {
        let mut infos = B::get_infos();
        infos.push(crate::archetype::ComponentInfo::of::<C>());
        infos
    }

    unsafe fn write_to_archetype(self, arch: &mut crate::archetype::Archetype, row: usize, tick: u32) {
        self.bundle.write_to_archetype(arch, row, tick);
        let col = arch.get_column_mut(std::any::TypeId::of::<C>()).unwrap();
        if col.len() <= row {
            col.push_raw(&self.component as *const _ as *const u8, tick);
            std::mem::forget(self.component);
        } else {
            let ptr = col.get_mut_ptr(row) as *mut C;
            std::ptr::write(ptr, self.component);
            *col.ticks_ptr_mut().add(row) = crate::archetype::ComponentTicks::new(tick);
        }
    }
}

pub trait BundleExt: Bundle + Sized {
    fn with<C: Component>(self, component: C) -> DynamicBundle<Self, C> {
        DynamicBundle { bundle: self, component }
    }
}

impl<T: Bundle> BundleExt for T {}

impl<T: Component> Bundle for T {
    fn get_infos() -> Vec<crate::archetype::ComponentInfo> {
        vec![crate::archetype::ComponentInfo::of::<T>()]
    }

    fn apply(self, world: &mut crate::world::World, entity: crate::entity::Entity) {
        world.add_component(entity, self);
    }

    unsafe fn write_to_archetype(self, arch: &mut crate::archetype::Archetype, row: usize, tick: u32) {
        let col = arch.get_column_mut(std::any::TypeId::of::<T>()).unwrap_or_else(|| {
            panic!(
                "Component column for `{}` missing in Archetype. The bundle fast-path \
                 (write_to_archetype) only handles Table-storage components; SparseSet \
                 components must be routed via World::add_component. spawn_batch already \
                 falls back for sparse bundles — reaching here means another bundle path \
                 wrote a sparse component into the archetype.",
                std::any::type_name::<T>()
            )
        });
        if col.len() <= row {
            col.push_raw(&self as *const _ as *const u8, tick);
            std::mem::forget(self);
        } else {
            let ptr = col.get_mut_ptr(row) as *mut T;
            std::ptr::write(ptr, self);
            *col.ticks_ptr_mut().add(row) = crate::archetype::ComponentTicks::new(tick);
        }
    }
}

macro_rules! impl_bundle_tuple {
    ($($name:ident),*) => {
        #[allow(non_snake_case)]
        impl<$($name: crate::component::Bundle),*> Bundle for ($($name,)*) {
            fn get_infos() -> Vec<crate::archetype::ComponentInfo> {
                let mut infos = Vec::new();
                $(
                    infos.extend(<$name as crate::component::Bundle>::get_infos());
                )*
                infos
            }

            fn apply(self, world: &mut crate::world::World, entity: crate::entity::Entity) {
                let ($($name,)*) = self;
                $(
                    $name.apply(world, entity);
                )*
            }

            unsafe fn write_to_archetype(self, arch: &mut crate::archetype::Archetype, row: usize, tick: u32) {
                let ($($name,)*) = self;
                $(
                    $name.write_to_archetype(arch, row, tick);
                )*
            }
        }
    };
}

impl_bundle_tuple!(A);
impl_bundle_tuple!(A, B);
impl_bundle_tuple!(A, B, C);
impl_bundle_tuple!(A, B, C, D);
impl_bundle_tuple!(A, B, C, D, E);
impl_bundle_tuple!(A, B, C, D, E, F);
impl_bundle_tuple!(A, B, C, D, E, F, G);
impl_bundle_tuple!(A, B, C, D, E, F, G, H);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O);
impl_bundle_tuple!(A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P);
