use std::any::Any;

pub trait Component: 'static + Any + Send + Sync + Clone {}

#[macro_export]
macro_rules! impl_component {
    ($($t:ty),+ $(,)?) => {
        $(
            impl $crate::Component for $t {}
        )+
    };
}

// --- Hiyerarşi (Scene Graph) Bileşenleri ---
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Parent(pub u32);

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Children(pub Vec<u32>);

/// Entity isim bileşeni — Editor, Lua ve Scene Serialization tarafından kullanılır.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EntityName(pub String);

impl EntityName {
    pub fn new(name: &str) -> Self {
        Self(name.to_string())
    }
}

/// Görünmezlik etiketi: Eğer bu component bir objede varsa render edilmez (veya aktifliği kapatılır).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IsHidden;

/// Prefab spawn talebi. Entity'ye eklendiğinde prefab yükleme sistemi tarafından işlenir
/// ve işlendikten sonra component kaldırılır.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PrefabRequest(pub String);

impl PrefabRequest {
    pub fn new(name: &str) -> Self {
        Self(name.to_string())
    }

    /// Prefab adını döndürür.
    pub fn name(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EntityName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_component!(Parent, Children, EntityName, IsHidden, PrefabRequest);

// ============================================================
//  Bundle Trait — Birden fazla component'i tek seferde ekleme
// ============================================================

/// Bevy tarzı Bundle desteği.
/// Bir struct bu trait'i implemente ederse, `world.spawn_bundle(...)` ile
/// tüm bileşenler tek seferde eklenebilir.
///
/// ```ignore
/// world.spawn_bundle(CameraBundle {
///     position: Vec3::new(0.0, 3.0, 10.0),
///     fov: 60.0_f32.to_radians(),
///     ..default()
/// });
/// ```
pub trait Bundle {
    /// Bundle içindeki tüm bileşenleri verilen entity'ye ekler.
    fn apply(self, world: &mut crate::world::World, entity: crate::entity::Entity);
}

/// Herhangi bir Bundle'a çalışma zamanında veya derleme zamanında dinamik olarak 
/// ekstra component eklemeyi sağlayan zincirlenebilir wrapper.
pub struct DynamicBundle<B: Bundle, C: Component> {
    pub bundle: B,
    pub component: C,
}

impl<B: Bundle, C: Component> Bundle for DynamicBundle<B, C> {
    fn apply(self, world: &mut crate::world::World, entity: crate::entity::Entity) {
        self.bundle.apply(world, entity);
        world.add_component(entity, self.component);
    }
}

/// Tüm Bundle tipleri için otomatik `.with(Component)` zincirleme desteği.
pub trait BundleExt: Bundle + Sized {
    /// Bu bundle'ın üzerine ek bir bileşen daha ekler.
    fn with<C: Component>(self, component: C) -> DynamicBundle<Self, C> {
        DynamicBundle {
            bundle: self,
            component,
        }
    }
}

impl<T: Bundle> BundleExt for T {}
