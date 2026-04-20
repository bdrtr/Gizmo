//! Component Registry — Runtime'da tip adı ↔ TypeId eşlemesi
//!
//! Lua scriptleri ve Editor'ün component'lere isme göre erişmesini sağlar.
//!
//! ```rust,ignore
//! let mut registry = ComponentRegistry::new();
//! registry.register::<Transform>("Transform");
//! registry.register::<Camera>("Camera");
//!
//! assert_eq!(registry.get_name::<Transform>(), Some("Transform"));
//! assert_eq!(registry.get_type_id("Camera"), Some(TypeId::of::<Camera>()));
//! ```

use std::any::TypeId;
use std::collections::BTreeMap;

/// Component tiplerini isme göre sorgulama ve yönetim kaydı.
///
/// İki yönlü eşleme tutar: isim → TypeId ve TypeId → isim.
/// `register()` çift kayıt ve desync'i önler.
pub struct ComponentRegistry {
    /// İsim → TypeId eşlemesi (sıralı — deterministic iterasyon)
    name_to_type: BTreeMap<String, TypeId>,
    /// TypeId → İsim eşlemesi (ters lookup)
    type_to_name: BTreeMap<TypeId, String>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            name_to_type: BTreeMap::new(),
            type_to_name: BTreeMap::new(),
        }
    }

    /// Yeni bir component tipini isme göre kaydet.
    ///
    /// # Panics
    /// - Aynı tip farklı bir isimle zaten kayıtlıysa
    /// - Aynı isim farklı bir tipe zaten atanmışsa
    ///
    /// Aynı tip-isim çifti ile tekrar kayıt yapmak güvenlidir (no-op).
    pub fn register<T: 'static>(&mut self, name: &str) {
        let type_id = TypeId::of::<T>();

        // Aynı çiftle tekrar kayıt — no-op
        if let Some(&existing_tid) = self.name_to_type.get(name) {
            if existing_tid == type_id {
                return; // Zaten kayıtlı, aynı çift
            }
            panic!(
                "ComponentRegistry: '{}' ismi zaten farklı bir tipe atanmış!",
                name
            );
        }

        if let Some(existing_name) = self.type_to_name.get(&type_id) {
            panic!(
                "ComponentRegistry: Bu tip zaten '{}' ismiyle kayıtlı, '{}' ile tekrar kayıt edilemez!",
                existing_name, name
            );
        }

        self.name_to_type.insert(name.to_string(), type_id);
        self.type_to_name.insert(type_id, name.to_string());
    }

    /// Bir tipin kaydını siler. İsim ve TypeId eşlemesi birlikte kaldırılır.
    /// Kayıtlı değilse false döner.
    pub fn unregister<T: 'static>(&mut self) -> bool {
        let type_id = TypeId::of::<T>();
        if let Some(name) = self.type_to_name.remove(&type_id) {
            self.name_to_type.remove(&name);
            true
        } else {
            false
        }
    }

    /// İsim ile bir tipin kaydını siler.
    pub fn unregister_by_name(&mut self, name: &str) -> bool {
        if let Some(type_id) = self.name_to_type.remove(name) {
            self.type_to_name.remove(&type_id);
            true
        } else {
            false
        }
    }

    // ──── Sorgulama ────

    /// İsimden TypeId'ye dönüşüm
    pub fn get_type_id(&self, name: &str) -> Option<TypeId> {
        self.name_to_type.get(name).copied()
    }

    /// TypeId'den isime dönüşüm (generic — derleme zamanı tip bilgisiyle)
    pub fn get_name<T: 'static>(&self) -> Option<&str> {
        self.get_name_by_id(TypeId::of::<T>())
    }

    /// TypeId'den isime dönüşüm (runtime TypeId ile)
    pub fn get_name_by_id(&self, type_id: TypeId) -> Option<&str> {
        self.type_to_name.get(&type_id).map(|s| s.as_str())
    }

    /// İsim kayıtlı mı?
    pub fn contains_name(&self, name: &str) -> bool {
        self.name_to_type.contains_key(name)
    }

    /// Tip kayıtlı mı?
    pub fn contains_type<T: 'static>(&self) -> bool {
        self.type_to_name.contains_key(&TypeId::of::<T>())
    }

    /// Kayıtlı tüm component isimlerini sıralı olarak döndürür.
    /// BTreeMap kullanıldığı için sıra her zaman alfabetik ve deterministiktir.
    pub fn all_names(&self) -> Vec<&str> {
        self.name_to_type.keys().map(|s| s.as_str()).collect()
    }

    /// Kayıtlı component sayısı
    #[inline]
    pub fn len(&self) -> usize {
        self.name_to_type.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.name_to_type.is_empty()
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Transform;
    struct Camera;
    struct PointLight;

    #[test]
    fn test_register_and_lookup() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform");
        reg.register::<Camera>("Camera");

        assert_eq!(reg.get_name::<Transform>(), Some("Transform"));
        assert_eq!(reg.get_name::<Camera>(), Some("Camera"));
        assert_eq!(reg.get_type_id("Transform"), Some(TypeId::of::<Transform>()));
        assert_eq!(reg.len(), 2);
    }

    #[test]
    fn test_idempotent_register() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform");
        reg.register::<Transform>("Transform"); // No-op
        assert_eq!(reg.len(), 1);
    }

    #[test]
    #[should_panic(expected = "ismi zaten farklı bir tipe atanmış")]
    fn test_duplicate_name_panics() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Shared");
        reg.register::<Camera>("Shared"); // Farklı tip, aynı isim
    }

    #[test]
    #[should_panic(expected = "zaten")]
    fn test_duplicate_type_panics() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform");
        reg.register::<Transform>("transform"); // Aynı tip, farklı isim
    }

    #[test]
    fn test_unregister() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform");
        assert!(reg.contains_type::<Transform>());

        assert!(reg.unregister::<Transform>());
        assert!(!reg.contains_type::<Transform>());
        assert!(!reg.contains_name("Transform"));
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn test_unregister_by_name() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Camera>("Camera");

        assert!(reg.unregister_by_name("Camera"));
        assert!(!reg.contains_name("Camera"));
        assert!(!reg.contains_type::<Camera>());
    }

    #[test]
    fn test_unregister_nonexistent() {
        let mut reg = ComponentRegistry::new();
        assert!(!reg.unregister::<Transform>());
        assert!(!reg.unregister_by_name("Foo"));
    }

    #[test]
    fn test_contains() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform");

        assert!(reg.contains_name("Transform"));
        assert!(reg.contains_type::<Transform>());
        assert!(!reg.contains_name("Camera"));
        assert!(!reg.contains_type::<Camera>());
    }

    #[test]
    fn test_all_names_sorted() {
        let mut reg = ComponentRegistry::new();
        reg.register::<PointLight>("PointLight");
        reg.register::<Camera>("Camera");
        reg.register::<Transform>("Transform");

        let names = reg.all_names();
        assert_eq!(names, vec!["Camera", "PointLight", "Transform"]);
    }

    #[test]
    fn test_get_name_delegates_to_get_name_by_id() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform");

        let by_generic = reg.get_name::<Transform>();
        let by_id = reg.get_name_by_id(TypeId::of::<Transform>());
        assert_eq!(by_generic, by_id);
    }

    #[test]
    fn test_re_register_after_unregister() {
        let mut reg = ComponentRegistry::new();
        reg.register::<Transform>("Transform");
        reg.unregister::<Transform>();
        reg.register::<Transform>("NewTransform"); // Farklı isimle tekrar kayıt — artık sorunsuz
        assert_eq!(reg.get_name::<Transform>(), Some("NewTransform"));
    }
}
