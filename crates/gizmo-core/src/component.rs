use std::any::Any;

pub trait Component: 'static + Any {}

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
