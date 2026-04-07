//! Component Registry — Runtime'da tip adı ↔ TypeId eşlemesi
//!
//! Lua scriptleri ve Editor'ün component'lere isme göre erişmesini sağlar.
//! Örnek: `entity.add_component(id, "PointLight", { color = {1,1,1}, intensity = 2.0 })`

use std::any::TypeId;
use std::collections::HashMap;

/// Component tiplerini isme göre sorgulama ve yönetim kaydı
pub struct ComponentRegistry {
    /// İsim → TypeId eşlemesi
    name_to_type: HashMap<String, TypeId>,
    /// TypeId → İsim eşlemesi (ters lookup)
    type_to_name: HashMap<TypeId, String>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            name_to_type: HashMap::new(),
            type_to_name: HashMap::new(),
        }
    }

    /// Yeni bir component tipini isme göre kaydet
    pub fn register<T: 'static>(&mut self, name: &str) {
        let type_id = TypeId::of::<T>();
        self.name_to_type.insert(name.to_string(), type_id);
        self.type_to_name.insert(type_id, name.to_string());
    }

    /// İsimden TypeId'ye dönüşüm
    pub fn get_type_id(&self, name: &str) -> Option<TypeId> {
        self.name_to_type.get(name).copied()
    }

    /// TypeId'den isime dönüşüm
    pub fn get_name<T: 'static>(&self) -> Option<&str> {
        self.type_to_name.get(&TypeId::of::<T>()).map(|s| s.as_str())
    }

    /// TypeId'den isime dönüşüm (doğrudan TypeId ile)
    pub fn get_name_by_id(&self, type_id: TypeId) -> Option<&str> {
        self.type_to_name.get(&type_id).map(|s| s.as_str())
    }

    /// Kayıtlı tüm component isimlerini döndür
    pub fn all_names(&self) -> Vec<&str> {
        self.name_to_type.keys().map(|s| s.as_str()).collect()
    }

    /// Kayıtlı component sayısı
    pub fn len(&self) -> usize {
        self.name_to_type.len()
    }

    pub fn is_empty(&self) -> bool {
        self.name_to_type.is_empty()
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
